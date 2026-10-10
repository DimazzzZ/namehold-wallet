//! Background jobs for Shakedex purchases (R13) and listings (R19).
//!
//! Every sync derives each open purchase's state from the chain — from what
//! the node knows about the purchase transaction, the seller's lock coin and
//! the TRANSFER the purchase creates — rather than from the draft's status:
//!
//! - in the mempool → `unconfirmed`;
//! - mined, and the TRANSFER commits to our name destination →
//!   `awaiting_finalize` with the blocks left in the transfer lockup;
//! - the TRANSFER spent and the name's owner coin at our destination →
//!   `owned`;
//! - refused by hsd at broadcast (its own JSON-RPC error, which hsd 8.0.0
//!   sends only for input it cannot parse), the lock coin spent by someone
//!   else, the TRANSFER
//!   committing elsewhere, the name expiring before it was finalized, or
//!   never mined after one rebroadcast → `lost`, and the purchase's funding
//!   coins are released.
//!
//! No purchase is declared lost while anything on chain still traces to it
//! (see [`trace`]): a node without a transaction index answers "unknown" for a
//! mined transaction. The draft lifecycle (`refresh_tx_confirmations`) never
//! drops or fails a purchase draft; this job does, when it loses the
//! purchase, so a draft `failed` here got hsd's own error at broadcast.
//!
//! A purchase lost while it paid nothing is looked at again for
//! [`REVIVE_WINDOW_DAYS`]: hsd forgets a mempool transaction after 72 hours,
//! but another node can still mine it, and a reorg can undo the purchase
//! that beat it. Once its TRANSFER is mined to us it paid, and it is
//! `awaiting_finalize` again.
//!
//! A purchase whose draft was abandoned before it was sent is deleted with
//! that draft. The job runs from `commands::sync::run_sync_steps` when the
//! profile's node is authoritative, so both the app and the background daemon
//! execute it.
//!
//! A second step, [`refresh_listings_step`], runs two jobs on listing sets
//! both read before either job runs. The before-lock job resolves each
//! listing before the FINALIZE into the lock from chain facts (R19): Aborted
//! once the name has left the lock TRANSFER (its Cancel transfer mined, a
//! REVOKE, a lock TRANSFER that never landed), Expired when hsd reports no
//! live name, Locking again if a reorg undoes the abort, ReadyToFinalize
//! once the transfer lockup is over (and Locking again if a reorg moves the
//! lock TRANSFER back), and a Restored lock when the name's owner coin is a
//! FINALIZE at the listing's own lock address, sent from elsewhere — never
//! Aborted ([`refresh_before_lock`]). The after-lock job
//! follows each listing whose FINALIZE into the lock is built: Listed once it
//! is mined, Finalizing again on a reorg, ReadyToFinalize again if it never
//! landed, settled as before the lock if it never landed and something else
//! spent the lock TRANSFER, and, once the lock coin is spent, SalePending or
//! Sold when a purchase of it is found on chain (R22: a TRANSFER out of our
//! lock committing to an address not ours, in a transaction that pays the
//! listing's payment address) ([`refresh_after_lock`]). It follows a
//! cancel too (R28): CancelAwaitingFinalize once a mined TRANSFER out of the
//! listing's lock coin commits to an address of ours, Sold when a purchase
//! is mined first (the losing cancel's coins released), Listed again when a
//! cancel can no longer land.
//! Both only read the node, and never take the same listing.
//!
//! The market jobs (T6) run after them, on mainnet only:
//! [`publish_listings_step`] announces a published listing on LearnHNS
//! Market as pending once its lock TRANSFER is sent (R23 day 0) and uploads
//! its current price step once the FINALIZE into the lock is mined, every
//! stored step verified over the lock coin our node reports first
//! ([`market_copy`]); [`keep_listed_step`] keeps it there (R25). They read
//! the node and write to the market and the listing's market bookkeeping
//! only: the market's answers never change a listing's state.

use std::collections::HashSet;

use crate::db::queries::{self, PurchaseProgress, PurchaseState, ShakedexPurchase, TxDraftRow};
use crate::error::AppError;
use crate::market::learnhns::{
    LearnHnsClient, ListingKind, MarketReply, PendingListing, ProofCopy,
};
use crate::noncustodial::network::Network;
use crate::noncustodial::node_rpc::NodeRpc;
use crate::noncustodial::rpc::{self, NodeRpcClient};
use crate::noncustodial::send::RESERVATION_TTL_SECS;
use crate::noncustodial::shakedex::cancel;
use crate::noncustodial::shakedex::listing_file;
use crate::noncustodial::shakedex::purchase::{self, transfer_commits_to};
use crate::noncustodial::shakedex::script;
use crate::noncustodial::shakedex::sell;
use crate::noncustodial::shakedex::template;
use crate::noncustodial::shakedex::verify;
use crate::noncustodial::sync::{COV_FINALIZE, COV_TRANSFER};
use crate::noncustodial::tx_evidence;

/// Blocks a sent purchase may be absent from the node's mempool and chain
/// before it is rebroadcast (once) and then given up as lost.
pub(crate) const MISSING_BLOCKS: i64 = 6;

/// hsd's mempool expiry (`policy.MEMPOOL_EXPIRY_TIME`, 72 hours) in blocks
/// at the target spacing (432). A purchase missing from the node this long is in no
/// mempool that keeps hsd's default: one that may not be resent from here is
/// given up as lost rather than kept reserved for ever (a late mining still
/// revives it, [`REVIVE_WINDOW_DAYS`]).
pub(crate) const MEMPOOL_EXPIRY_BLOCKS: i64 =
    72 * 60 * 60 / crate::noncustodial::network::TARGET_SPACING_SECS as i64;

/// How long past the reservation TTL an unsent purchase draft is kept before
/// the sync deletes it with its purchase. A broadcast refuses a purchase draft
/// past the TTL (`commands::tx::broadcast_tx_draft`), but one that started
/// just before it is still talking to the node — a few requests, each bounded
/// by the client's 30-second timeout — and must not lose its purchase record
/// under it.
pub(crate) const SEND_GRACE_SECS: i64 = 600;

/// How long a purchase lost while paying nothing is still checked for a
/// late mining: more than hsd's 72-hour mempool expiry, with room for a
/// reorg.
pub const REVIVE_WINDOW_DAYS: u32 = 7;

/// How long a Sold listing is looked at again for a reorg that takes its
/// purchase out of the chain, the same window as a lost purchase's revival.
pub const SOLD_RECHECK_DAYS: u32 = REVIVE_WINDOW_DAYS;

/// hsd's own JSON-RPC error to the broadcast. hsd 8.0.0 answers
/// `sendrawtransaction` with the txid whatever its mempool does (it relays
/// without awaiting the result), and errors only for input it cannot parse;
/// a purchase someone else beat is lost as [`BOUGHT_BY_OTHER`] instead, once
/// it is found missing with the lock coin spent.
const REFUSED: &str = "the node refused to take the purchase, so nothing was paid";
const BOUGHT_BY_OTHER: &str =
    "someone else bought the name first, or the seller cancelled the listing — nothing was paid";
const NEVER_CONFIRMED: &str = "the purchase never confirmed — nothing was paid";
const NOT_RESENT_PRICE_DROPPED: &str = "the purchase never confirmed, and was not sent again \
    because the listing's price has dropped since — nothing was paid";
const TRANSFERRED_ELSEWHERE: &str = "the purchase transferred the name elsewhere";
const EXPIRED_UNFINALIZED: &str =
    "the name expired before it was finalized — the purchase was paid, but the name is lost";

/// Whether this refresh may rebroadcast a purchase that went missing (R13).
/// The sync daemon never broadcasts (SECURITY.md, "Daemon is read-only"), so
/// it only tracks the purchase and leaves its one rebroadcast to the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rebroadcast {
    Allowed,
    Never,
}

/// Best-effort sync step: refresh the profile's open purchases from its node.
/// Like the other sync steps it returns silently when the database, the
/// profile or its node client cannot be opened; a failed refresh is logged.
/// The caller runs
/// it only when the profile's node is authoritative (connected, synced, on the
/// profile's network, not SPV): anywhere else, a transaction the node does not
/// know proves nothing.
pub async fn refresh_purchases_step(db_path: &str, profile_id: &str, rebroadcast: Rebroadcast) {
    let conn = match crate::db::connection::open_migrated(db_path) {
        Ok(c) => c,
        Err(_) => return,
    };
    // Best-effort like every sync step: a profile that cannot be read (or
    // was deleted meanwhile) has no purchases to refresh this round.
    let network = match queries::get_wallet_profile(&conn, profile_id)
        .ok()
        .flatten()
        .and_then(|p| Network::from_str_opt(&p.network))
    {
        Some(n) => n,
        None => return,
    };
    let client = match NodeRpcClient::for_profile(&conn, profile_id) {
        Ok(c) => c,
        Err(_) => return,
    };
    if let Err(e) =
        refresh_purchases_with_client(&conn, &client, profile_id, network, rebroadcast).await
    {
        eprintln!("shakedex purchases: refresh failed for {profile_id}: {e}");
    }
}

/// Derive every open purchase's state from the chain (see the module docs).
/// A failure on one purchase is logged and leaves that purchase for the next
/// sync; the others are still refreshed.
pub async fn refresh_purchases_with_client(
    conn: &rusqlite::Connection,
    client: &dyn NodeRpc,
    profile_id: &str,
    network: Network,
    rebroadcast: Rebroadcast,
) -> Result<(), AppError> {
    let purchases = queries::list_open_shakedex_purchases(conn, profile_id)?;
    let lost =
        queries::list_recent_unpaid_lost_shakedex_purchases(conn, profile_id, REVIVE_WINDOW_DAYS)?;
    let owned =
        queries::list_recent_owned_shakedex_purchases(conn, profile_id, REVIVE_WINDOW_DAYS)?;
    if purchases.is_empty() && lost.is_empty() && owned.is_empty() {
        return Ok(());
    }
    let tip = client.get_blockchain_info().await?.blocks;
    let job = Job {
        conn,
        client,
        network,
        tip,
        rebroadcast,
    };
    for p in &purchases {
        if let Err(e) = job.refresh(p).await {
            eprintln!("shakedex purchases: {} ({}): {e}", p.id, p.name);
        }
    }
    for p in &lost {
        if let Err(e) = job.revive(p).await {
            eprintln!("shakedex purchases: {} ({}): {e}", p.id, p.name);
        }
    }
    for p in &owned {
        if let Err(e) = job.recheck_owned(p).await {
            eprintln!("shakedex purchases: {} ({}): {e}", p.id, p.name);
        }
    }
    Ok(())
}

/// What the chain says about a purchase whose transaction cannot be read
/// directly (see [`Job::trace`]).
enum Trace {
    /// The TRANSFER coin exists and commits to our destination, mined at the
    /// given height, or `None` while it is only in the mempool.
    Committed(Option<i64>),
    /// The TRANSFER coin exists and commits to another address.
    Elsewhere,
    /// The name's owner coin pays our destination.
    Owned,
    /// The name's owner is still the purchase's TRANSFER output.
    OwnerIsPurchase,
    /// Nothing on chain points at the purchase.
    NotOurs,
}

/// The `info` of a `getnameinfo` reply: `None` when hsd says the name does
/// not exist or has expired (`"info": null`), an error when the reply has no
/// `info` at all — that is not hsd's answer, and proves neither.
fn name_info(reply: &serde_json::Value) -> Result<Option<&serde_json::Value>, AppError> {
    match reply.get("info") {
        None => Err(AppError::Rpc("node did not report the name's state".into())),
        Some(serde_json::Value::Null) => Ok(None),
        Some(info) => Ok(Some(info)),
    }
}

/// The block height of a `GET /tx/:hash` reply, `None` in the mempool. hsd
/// always sends `height`, -1 for a mempool transaction (`TXMeta.getJSON`):
/// a reply without it, or with anything else, is not hsd's answer.
fn tx_height(tx: &serde_json::Value) -> Result<Option<i64>, AppError> {
    rpc::mined_height(tx.get("height").and_then(|h| h.as_i64()), || {
        "the purchase's height".into()
    })
}

/// If a transaction from hsd's `GET /tx/address` spends the TRANSFER at
/// `purchase_txid:0`, the covenant type of the output that input is linked to
/// (the same index: hsd's rule for a TRANSFER spend). hsd lists every
/// transaction with its inputs and outputs, each input with its prevout and
/// each output with its covenant: an entry without them is not hsd's answer.
pub(crate) fn transfer_spent_into(
    tx: &serde_json::Value,
    purchase_txid: &str,
) -> Result<Option<u64>, AppError> {
    let not_hsds = |what: &str| AppError::Rpc(format!("node listed a transaction without {what}"));
    let inputs = tx
        .get("inputs")
        .and_then(|i| i.as_array())
        .ok_or_else(|| not_hsds("its inputs"))?;
    for (i, input) in inputs.iter().enumerate() {
        let prevout = input.get("prevout");
        let hash = prevout.and_then(|o| o.get("hash")).and_then(|h| h.as_str());
        let index = prevout
            .and_then(|o| o.get("index"))
            .and_then(|i| i.as_u64());
        let (Some(hash), Some(index)) = (hash, index) else {
            return Err(not_hsds("an input's prevout"));
        };
        if !(hash.eq_ignore_ascii_case(purchase_txid) && index == 0) {
            continue;
        }
        let covenant = tx
            .get("outputs")
            .and_then(|o| o.as_array())
            .and_then(|o| o.get(i))
            .and_then(|o| o.get("covenant"))
            .and_then(|c| c.get("type"))
            .and_then(|t| t.as_u64())
            .ok_or_else(|| not_hsds("the output a TRANSFER spend is linked to"))?;
        return Ok(Some(covenant));
    }
    Ok(None)
}

struct Job<'a> {
    conn: &'a rusqlite::Connection,
    client: &'a dyn NodeRpc,
    network: Network,
    tip: i64,
    rebroadcast: Rebroadcast,
}

impl Job<'_> {
    async fn refresh(&self, p: &ShakedexPurchase) -> Result<(), AppError> {
        let Some(draft) = queries::get_tx_draft(self.conn, &p.purchase_draft_id)? else {
            // The draft was discarded, which is only possible before broadcast.
            return queries::delete_shakedex_purchase(self.conn, &p.id);
        };
        match draft.status.as_str() {
            "failed" => self.lose_unless_traced(p, REFUSED, None).await,
            "draft" | "signed" => {
                // Deletes only if the draft is still unsent and past the TTL
                // and the send grace.
                queries::delete_abandoned_shakedex_purchase(
                    self.conn,
                    &p.id,
                    &p.purchase_draft_id,
                    RESERVATION_TTL_SECS + SEND_GRACE_SECS,
                )?;
                Ok(())
            }
            // Sent, or possibly sent: only the chain can tell what happened.
            _ => self.refresh_from_chain(p, &draft).await,
        }
    }

    async fn refresh_from_chain(
        &self,
        p: &ShakedexPurchase,
        draft: &TxDraftRow,
    ) -> Result<(), AppError> {
        let tx = self.client.get_tx_by_hash(&p.purchase_txid).await?;
        if tx.is_null() {
            return self.refresh_missing(p, draft).await;
        }
        match tx_height(&tx)? {
            // In the mempool (also after a reorg took its block away).
            None => self.unconfirmed(p, None, p.rebroadcast_count),
            Some(height) => match self.trace(p).await? {
                Trace::Committed(_) if self.name_moved_on(p).await? => {
                    self.mark_lost_paid(p, EXPIRED_UNFINALIZED)
                }
                Trace::Committed(_) | Trace::OwnerIsPurchase => {
                    self.awaiting_finalize(p, Some(height))
                }
                Trace::NotOurs => self.after_transfer_spent(p, height).await,
                Trace::Elsewhere => self.mark_lost_paid(p, TRANSFERRED_ELSEWHERE),
                Trace::Owned => self.owned(p, Some(height)),
            },
        }
    }

    /// The purchase is unknown to the node: lost if the lock coin went
    /// elsewhere, otherwise counted down, rebroadcast once, then given up.
    async fn refresh_missing(
        &self,
        p: &ShakedexPurchase,
        draft: &TxDraftRow,
    ) -> Result<(), AppError> {
        let lock_vout = u32::try_from(p.lock_vout).map_err(|_| {
            AppError::Other(format!("lock output index {} is invalid", p.lock_vout))
        })?;
        if self
            .client
            .get_coin(&p.lock_txid, lock_vout)
            .await?
            .is_none()
        {
            // Spent — by our purchase on a node without a tx index, or by
            // someone else's.
            return self.lose_unless_traced(p, BOUGHT_BY_OTHER, None).await;
        }
        // Not seen missing before: the count of missing blocks starts now.
        let missing_since = p.missing_since_height.unwrap_or(self.tip);
        if self.tip - missing_since < MISSING_BLOCKS {
            return self.unconfirmed(p, Some(missing_since), p.rebroadcast_count);
        }
        // A dropped draft's coins were already released and may be spent
        // elsewhere since: never resend it.
        if p.rebroadcast_count > 0 || draft.status == "dropped" {
            return self.lose_unless_traced(p, NEVER_CONFIRMED, None).await;
        }
        let Some(signed) = draft.signed_tx_hex.as_deref() else {
            return self.lose_unless_traced(p, NEVER_CONFIRMED, None).await;
        };
        // Not sent from here: keep waiting for a sync that may send it —
        // until no mempool holds it any more.
        let wait = || async {
            if self.tip - missing_since >= MEMPOOL_EXPIRY_BLOCKS {
                return self.lose_unless_traced(p, NEVER_CONFIRMED, None).await;
            }
            self.unconfirmed(p, Some(missing_since), p.rebroadcast_count)
        };
        if !self.may_broadcast().await? {
            // Not allowed to send from here (the daemon, or a node without
            // the opt-in): the app's next sync, the user or a changed opt-in
            // can still send it.
            return wait().await;
        }
        // A rebroadcast is a broadcast (R10): only the paid step, and only
        // while it is the current one. A cheaper step replaced it: lost. The
        // paid step not valid at the median time (it went back in a reorg):
        // hsd would take it as non-final and still answer with the txid,
        // spending the one rebroadcast — wait instead. A price we cannot
        // re-check is an error, and the next sync tries again.
        let paid = p.paid_doos()?;
        let paid_lock_time = purchase::plan_lock_time(&draft.signing_inputs_json)?;
        match verify::paid_step(
            self.client,
            self.network,
            &p.listing_json,
            paid,
            paid_lock_time,
        )
        .await?
        {
            verify::PaidStep::Current => {}
            verify::PaidStep::Cheaper => {
                return self
                    .lose_unless_traced(p, NOT_RESENT_PRICE_DROPPED, None)
                    .await;
            }
            verify::PaidStep::NotValid => return wait().await,
        }
        match self.client.send_raw_transaction(signed).await {
            // hsd answers with the txid whatever its mempool does with the
            // purchase: only a look-up says whether the node took it.
            Ok(_) => {
                match tx_evidence::taken_by_node_with_client(self.client, signed, &p.purchase_txid)
                    .await
                {
                    tx_evidence::Taken::No => self.lose_unless_traced(p, REFUSED, Some(1)).await,
                    // Taken, mined meanwhile (the next sync traces it), or no
                    // answer: the one rebroadcast is spent either way.
                    _ => self.unconfirmed(p, None, 1),
                }
            }
            // hsd's own error: it did not take the transaction (in hsd 8.0.0
            // only for input it cannot parse; a mempool refusal still
            // answers with the txid).
            Err(e) if rpc::is_node_rejection(&e) => {
                self.lose_unless_traced(p, REFUSED, Some(1)).await
            }
            // Transport failure or a reply that is not hsd's: nothing is
            // known, try again next sync.
            Err(e) => Err(e),
        }
    }

    /// A purchase lost while paying nothing, looked at again: if its
    /// TRANSFER has since been mined to us, it paid after all — and if the
    /// TRANSFER is already spent with the name at our destination, someone
    /// finalized it since (anyone may, once the lockup is over), and it is
    /// owned. The same trace as [`Job::lose_unless_traced`]; a TRANSFER only
    /// in the mempool waits until it is mined, so the purchase never reopens
    /// beside a new purchase of the same listing.
    async fn revive(&self, p: &ShakedexPurchase) -> Result<(), AppError> {
        let (height, owned) = match self.trace(p).await? {
            Trace::Committed(Some(height)) => (height, false),
            // The name at our destination is ours only through this purchase
            // if its own transaction is mined: a later purchase of the same
            // listing may have been given the same unused address, and only
            // one purchase of a lock coin can be mined.
            Trace::Owned => match self.mined_height(&p.purchase_txid).await? {
                Some(height) => (height, true),
                None => return Ok(()),
            },
            Trace::Committed(None) | Trace::OwnerIsPurchase | Trace::Elsewhere | Trace::NotOurs => {
                return Ok(())
            }
        };
        let p = &ShakedexPurchase {
            lost_reason: None,
            ..p.clone()
        };
        let expired = !owned && self.name_moved_on(p).await?;
        let tx = self.conn.unchecked_transaction()?;
        let job = Job { conn: &tx, ..*self };
        if owned {
            job.owned(p, Some(height))?;
        } else if expired {
            // Its coins were released when it was first lost.
            job.save(
                p,
                PurchaseProgress {
                    state: PurchaseState::Lost,
                    purchase_height: Some(height),
                    blocks_remaining: None,
                    missing_since_height: None,
                    rebroadcast_count: p.rebroadcast_count,
                    lost_reason: Some(EXPIRED_UNFINALIZED.to_owned()),
                },
            )?;
        } else {
            job.awaiting_finalize(p, Some(height))?;
        }
        // It reached the chain: the draft's "nothing was paid" is wrong now,
        // and the confirmation sync takes it from here.
        if queries::get_tx_draft(&tx, &p.purchase_draft_id)?.is_some() {
            queries::update_tx_draft_status(
                &tx,
                &p.purchase_draft_id,
                "broadcasted",
                None,
                Some(&p.purchase_txid),
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The height `txid` is mined at, or `None` while it is unknown to the
    /// node or only in its mempool.
    async fn mined_height(&self, txid: &str) -> Result<Option<i64>, AppError> {
        let tx = self.client.get_tx_by_hash(txid).await?;
        if tx.is_null() {
            return Ok(None);
        }
        tx_height(&tx)
    }

    /// An owned purchase, looked at again: the FINALIZE spends the
    /// purchase's TRANSFER coin, so that coin unspent again means a reorg
    /// took the FINALIZE away, and the purchase awaits finalize again (or is
    /// unconfirmed, if the reorg took the purchase back to the mempool). One
    /// coin lookup; while the coin stays spent, nothing else is read — what
    /// the owner did with the name since is not this purchase's business.
    async fn recheck_owned(&self, p: &ShakedexPurchase) -> Result<(), AppError> {
        let Some(transfer) = self.client.get_coin(&p.purchase_txid, 0).await? else {
            return Ok(());
        };
        if !transfer_commits_to(&transfer, self.network, &p.destination_address)? {
            return Ok(());
        }
        match transfer.mined_height()? {
            Some(height) => self.awaiting_finalize(p, Some(height)),
            None => self.unconfirmed(p, None, p.rebroadcast_count),
        }
    }

    /// Whether this refresh may send the purchase: never from the daemon,
    /// otherwise the same gates as every other broadcast
    /// ([`rpc::broadcast_gates`]).
    /// The network compared is the job's own, read from the purchase's
    /// profile when the job started.
    async fn may_broadcast(&self) -> Result<bool, AppError> {
        if self.rebroadcast == Rebroadcast::Never {
            return Ok(false);
        }
        let settings = queries::get_settings(self.conn)?;
        Ok(
            rpc::broadcast_gates(self.client, &settings, Some(self.network.as_str()))
                .await
                .is_ok(),
        )
    }

    /// Mark the purchase lost for `reason` — unless the chain still traces
    /// to it, in which case it takes the state the trace shows.
    async fn lose_unless_traced(
        &self,
        p: &ShakedexPurchase,
        reason: &str,
        rebroadcast_count: Option<i64>,
    ) -> Result<(), AppError> {
        let p = &ShakedexPurchase {
            // `None`: this refresh sent nothing, so the count stays.
            rebroadcast_count: rebroadcast_count.unwrap_or(p.rebroadcast_count),
            ..p.clone()
        };
        match self.trace(p).await? {
            Trace::Committed(Some(_)) if self.name_moved_on(p).await? => {
                self.mark_lost_paid(p, EXPIRED_UNFINALIZED)
            }
            Trace::Committed(Some(height)) => self.awaiting_finalize(p, Some(height)),
            // Our TRANSFER, but only in the mempool: the lockup has not begun.
            Trace::Committed(None) => self.unconfirmed(p, None, p.rebroadcast_count),
            Trace::OwnerIsPurchase => self.awaiting_finalize(p, None),
            Trace::Owned => self.owned(p, None),
            Trace::Elsewhere => self.mark_lost_paid(p, TRANSFERRED_ELSEWHERE),
            Trace::NotOurs => self.mark_lost(p, reason),
        }
    }

    /// Follow the purchase on chain without its transaction: the TRANSFER
    /// coin at `purchase_txid:0`, then the name's current owner.
    async fn trace(&self, p: &ShakedexPurchase) -> Result<Trace, AppError> {
        if let Some(transfer) = self.client.get_coin(&p.purchase_txid, 0).await? {
            return Ok(
                if transfer_commits_to(&transfer, self.network, &p.destination_address)? {
                    Trace::Committed(transfer.mined_height()?)
                } else {
                    Trace::Elsewhere
                },
            );
        }
        let reply = self.client.get_name_info(&p.name).await?;
        // An expired (or never opened) name has no owner to trace to.
        let Some(info) = name_info(&reply)? else {
            return Ok(Trace::NotOurs);
        };
        // `NotOurs` ends in `lost` and releases the purchase's coins: an
        // owner the node leaves out is an error (this purchase waits for the
        // next sync), not a proof that the name is someone else's.
        let owner = info.get("owner");
        let txid = owner.and_then(|o| o.get("hash")).and_then(|v| v.as_str());
        let index = owner
            .and_then(|o| o.get("index"))
            .and_then(|v| v.as_u64())
            .and_then(|v| u32::try_from(v).ok());
        let (Some(txid), Some(index)) = (txid, index) else {
            return Err(AppError::Rpc("node did not report the name's owner".into()));
        };
        if txid.eq_ignore_ascii_case(&p.purchase_txid) {
            return Ok(Trace::OwnerIsPurchase);
        }
        let at_destination = match self.client.get_coin(txid, index).await? {
            // The chain's owner coin, gone from the node's view: hsd reports a
            // coin spent in its mempool as missing (`getCoin`), and the spend
            // may be ours (an UPDATE after our FINALIZE). Whose it is shows
            // once the spend is mined.
            None => {
                return Err(AppError::Rpc(
                    "the name's owner coin is spent in the node's mempool".into(),
                ))
            }
            Some(c) => match c.address {
                Some(a) => a == p.destination_address,
                None => {
                    return Err(AppError::Rpc(
                        "node did not report the owner coin's address".into(),
                    ))
                }
            },
        };
        Ok(if at_destination {
            Trace::Owned
        } else {
            Trace::NotOurs
        })
    }

    /// A mined purchase whose TRANSFER is spent on chain while the name's
    /// owner coin is not at our destination. Only our FINALIZE can have spent
    /// it: the TRANSFER sits at the listing's lock address, whose script lets
    /// a coin be spent only into a TRANSFER signed by the seller or into a
    /// FINALIZE (`type TRANSFER equal IF <key> CHECKSIG ELSE type FINALIZE
    /// equal`), and hsd never lets a TRANSFER be spent into another TRANSFER
    /// (rules.js). A FINALIZE pays the address the TRANSFER committed to, ours,
    /// so the name arrived and has moved on since: owned. That FINALIZE is in
    /// our destination's history; a history without it (cut short, or not
    /// indexed yet), or with a spender that is not a FINALIZE, gives no
    /// verdict.
    async fn after_transfer_spent(
        &self,
        p: &ShakedexPurchase,
        purchase_height: i64,
    ) -> Result<(), AppError> {
        for tx in self
            .client
            .get_txs_by_address(&p.destination_address)
            .await?
        {
            let Some(covenant) = transfer_spent_into(&tx, &p.purchase_txid)? else {
                continue;
            };
            if covenant != u64::from(crate::noncustodial::sync::COV_FINALIZE) {
                return Err(AppError::Rpc(format!(
                    "the purchase's transfer was spent into covenant {covenant}, \
                     which its lock script does not allow"
                )));
            }
            return match tx_height(&tx)? {
                Some(_) => self.owned(p, Some(purchase_height)),
                // Our FINALIZE, only in the mempool: the name is on its way.
                None => self.awaiting_finalize(p, Some(purchase_height)),
            };
        }
        Err(AppError::Rpc(
            "the node lists no FINALIZE of the purchase's transfer".into(),
        ))
    }

    /// Whether a name whose mined TRANSFER is still unspent has moved on
    /// without it: expired (hsd's `getnameinfo` then reports no `info`), or
    /// opened and won again after expiring (the owner is another coin). Such
    /// a purchase can never be finalized. An owner the node leaves out
    /// proves neither, so the purchase keeps waiting.
    ///
    /// The caller saw the TRANSFER unspent before this reads the name, and our
    /// own FINALIZE may land in between: the owner is then the FINALIZE, and
    /// the name arrived rather than moved on. So the TRANSFER is read again
    /// after the name: still unspent, the name moved on without it; spent
    /// since, the two reads disagree and this sync gives no verdict.
    async fn name_moved_on(&self, p: &ShakedexPurchase) -> Result<bool, AppError> {
        let v = self.client.get_name_info(&p.name).await?;
        let moved_on = match name_info(&v)? {
            None => true,
            Some(info) => info
                .get("owner")
                .and_then(|o| o.get("hash"))
                .and_then(|h| h.as_str())
                .is_some_and(|h| !h.eq_ignore_ascii_case(&p.purchase_txid)),
        };
        if moved_on && self.client.get_coin(&p.purchase_txid, 0).await?.is_none() {
            return Err(AppError::Rpc(
                "the purchase's transfer was spent while the name was being read".into(),
            ));
        }
        Ok(moved_on)
    }

    /// `unconfirmed`, absent from the node since `missing_since` (if it is),
    /// after `rebroadcast_count` rebroadcasts.
    fn unconfirmed(
        &self,
        p: &ShakedexPurchase,
        missing_since: Option<i64>,
        rebroadcast_count: i64,
    ) -> Result<(), AppError> {
        self.save(
            p,
            PurchaseProgress {
                state: PurchaseState::Unconfirmed,
                purchase_height: None,
                blocks_remaining: None,
                missing_since_height: missing_since,
                rebroadcast_count,
                lost_reason: None,
            },
        )
    }

    /// `awaiting_finalize` at `height`, or at the height already recorded
    /// when the chain did not report one.
    fn awaiting_finalize(&self, p: &ShakedexPurchase, height: Option<i64>) -> Result<(), AppError> {
        let height = height.or(p.purchase_height);
        let params = self.network.name_params();
        self.save(
            p,
            PurchaseProgress {
                state: PurchaseState::AwaitingFinalize,
                purchase_height: height,
                blocks_remaining: height.map(|h| params.blocks_until_finalize(h, self.tip)),
                missing_since_height: None,
                rebroadcast_count: p.rebroadcast_count,
                lost_reason: None,
            },
        )
    }

    fn owned(&self, p: &ShakedexPurchase, height: Option<i64>) -> Result<(), AppError> {
        self.save(
            p,
            PurchaseProgress {
                state: PurchaseState::Owned,
                purchase_height: height.or(p.purchase_height),
                blocks_remaining: Some(0),
                missing_since_height: None,
                rebroadcast_count: p.rebroadcast_count,
                lost_reason: None,
            },
        )
    }

    /// `lost` for `reason` when the purchase never paid. Activity lists the
    /// purchase through its draft (R13): a draft that never mined is marked
    /// dropped (a refused one stays failed) and carries `reason`; a confirmed
    /// one keeps its status.
    fn mark_lost(&self, p: &ShakedexPurchase, reason: &str) -> Result<(), AppError> {
        self.save_lost(p, reason, true)
    }

    /// `lost` for `reason` when the purchase is on chain, so it did pay: its
    /// draft is left to the confirmation sync, never marked dropped.
    fn mark_lost_paid(&self, p: &ShakedexPurchase, reason: &str) -> Result<(), AppError> {
        self.save_lost(p, reason, false)
    }

    /// Marks the purchase lost and releases its funding coins in the same
    /// transaction; with `unpaid`, also ends its draft (see [`Self::mark_lost`]).
    fn save_lost(&self, p: &ShakedexPurchase, reason: &str, unpaid: bool) -> Result<(), AppError> {
        let tx = self.conn.unchecked_transaction()?;
        queries::update_shakedex_purchase_state(
            &tx,
            &p.id,
            &PurchaseProgress {
                state: PurchaseState::Lost,
                purchase_height: p.purchase_height,
                blocks_remaining: None,
                missing_since_height: None,
                rebroadcast_count: p.rebroadcast_count,
                lost_reason: Some(reason.to_owned()),
            },
        )?;
        queries::release_reserved_utxos_for_draft(&tx, &p.purchase_draft_id)?;
        let draft = if unpaid {
            queries::get_tx_draft(&tx, &p.purchase_draft_id)?
        } else {
            None
        };
        if let Some(draft) = draft {
            let status = match draft.status.as_str() {
                "confirmed" => None,
                "failed" => Some("failed"),
                _ => Some("dropped"),
            };
            if let Some(status) = status {
                queries::update_tx_draft_status(
                    &tx,
                    &p.purchase_draft_id,
                    status,
                    Some(reason),
                    None,
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn save(&self, p: &ShakedexPurchase, progress: PurchaseProgress) -> Result<(), AppError> {
        queries::update_shakedex_purchase_state(self.conn, &p.id, &progress)
    }
}

// ---------------------------------------------------------------------------
// Listings before the FINALIZE into the lock (R19)
// ---------------------------------------------------------------------------

/// How long an Aborted listing is looked at again for a reorg that takes its
/// Cancel transfer out of the chain, the same window as a lost purchase's
/// revival.
pub const ABORT_RECHECK_DAYS: u32 = REVIVE_WINDOW_DAYS;

/// What the chain shows about a listing's Cancel transfer. Every coin lookup
/// is hsd's `GET /coin/:hash/:index`, which needs no transaction index and
/// answers 404 for a coin spent in a block or in the mempool
/// (`node.getCoin`).
#[derive(Debug, PartialEq, Eq)]
enum CancelOnChain {
    /// The cancel's UPDATE is a coin mined in a block.
    Mined,
    /// The cancel is in no block: its UPDATE is a coin only in the mempool,
    /// or the lock TRANSFER it spends is still a coin.
    NotMined,
    /// No answer that decides it: a transport error, a coin without hsd's
    /// height, or both coins gone (the cancel mined and its UPDATE spent
    /// since, or the lock TRANSFER spent by another transaction).
    Unknown,
}

/// Read [`CancelOnChain`] for the cancel `cancel_txid`, which spends the
/// lock TRANSFER at `lock_transfer_txid:0`. The cancel's UPDATE is its
/// output 0 (`actions::build_plan` puts the covenant output first).
async fn cancel_on_chain(
    client: &dyn NodeRpc,
    cancel_txid: &str,
    lock_transfer_txid: &str,
) -> CancelOnChain {
    match client.get_coin(cancel_txid, 0).await {
        Ok(Some(update)) => match update.mined_height() {
            Ok(Some(_)) => CancelOnChain::Mined,
            Ok(None) => CancelOnChain::NotMined,
            Err(_) => CancelOnChain::Unknown,
        },
        Ok(None) => match client.get_coin(lock_transfer_txid, 0).await {
            Ok(Some(_)) => CancelOnChain::NotMined,
            Ok(None) | Err(_) => CancelOnChain::Unknown,
        },
        Err(_) => CancelOnChain::Unknown,
    }
}

/// What the chain shows about a listing's lock TRANSFER at
/// `lock_transfer_txid:0`, from hsd's `getnameinfo` and `GET /coin` alone.
#[derive(Debug, PartialEq, Eq)]
enum LockOnChain {
    /// hsd reports no live state for the name (`info: null`): it never
    /// existed or has expired; or the lock TRANSFER belongs to a
    /// registration that is gone: mined but not the owner, its covenant's
    /// name height is not hsd's `info.height` (the name expired and was
    /// opened again, `ns.reset`).
    NoName,
    /// The lock TRANSFER owns a name that is not revoked
    /// ([`sell::lock_transfer_owns_name`]). `transfer` is
    /// [`sell::transfer_height`]: `None` when hsd's `info.transfer` is
    /// missing, out of range or 0 (not hsd's whole answer).
    Owner { transfer: Option<i64> },
    /// The lock TRANSFER is a coin in the mempool (`height: -1`), not the
    /// owner yet (hsd moves `owner` only when a block is connected).
    Pending,
    /// Neither: `owner` is another outpoint, or the name is revoked
    /// (a REVOKE leaves `owner` at the coin it spent and sets `revoked`,
    /// hsd `chain.js`), and `GET /coin` is hsd's empty 404 (spent in a block
    /// or in the mempool, or never mined). `renewal` is hsd's
    /// `info.renewal`, the block of the name's last FINALIZE or renewal (a
    /// TRANSFER leaves it, `chain.js`): only a hint where to read a
    /// FINALIZE without the transaction index ([`spend_view`]).
    Gone {
        owner: (String, u32),
        revoked: bool,
        renewal: Option<i64>,
    },
}

/// Read [`LockOnChain`]. Any read error, and a reply without `info`, the
/// owner's `hash` and `index`, or `revoked`, is an error: not hsd's whole
/// answer, so no verdict.
async fn lock_on_chain(
    client: &dyn NodeRpc,
    name: &str,
    lock_transfer_txid: &str,
) -> Result<LockOnChain, AppError> {
    let reply = client.get_name_info(name).await?;
    let Some(info) = name_info(&reply)? else {
        return Ok(LockOnChain::NoName);
    };
    let owner = info.get("owner");
    let hash = owner.and_then(|o| o.get("hash")).and_then(|h| h.as_str());
    let index = owner
        .and_then(|o| o.get("index"))
        .and_then(|i| i.as_u64())
        .and_then(|i| u32::try_from(i).ok());
    let revoked = info.get("revoked").and_then(|r| r.as_u64());
    let (Some(hash), Some(index), Some(revoked)) = (hash, index, revoked) else {
        return Err(AppError::Rpc(
            "node did not report the name's owner or whether it was revoked".into(),
        ));
    };
    if sell::lock_transfer_owns_name(hash, u64::from(index), revoked, lock_transfer_txid) {
        Ok(LockOnChain::Owner {
            transfer: sell::transfer_height(info),
        })
    } else if let Some(coin) = client.get_coin(lock_transfer_txid, 0).await? {
        // Mined in a block, it would be the owner (or revoked): a node that
        // says otherwise is not consistent, so no verdict — unless the
        // registration it belongs to is gone: the name expired and was
        // opened again, so hsd's name height is not the one the TRANSFER
        // commits to.
        match coin.mined_height()? {
            None => Ok(LockOnChain::Pending),
            Some(_) => {
                let ours = coin
                    .covenant
                    .as_ref()
                    .and_then(purchase::covenant_name_height);
                let theirs = info.get("height").and_then(|h| h.as_u64());
                match (ours, theirs) {
                    (Some(o), Some(t)) if u64::from(o) != t => Ok(LockOnChain::NoName),
                    _ => Err(AppError::Rpc(format!(
                        "node reports lock TRANSFER {lock_transfer_txid}:0 mined but not the \
                         name's owner"
                    ))),
                }
            }
        }
    } else {
        Ok(LockOnChain::Gone {
            owner: (hash.to_string(), index),
            revoked: revoked != 0,
            renewal: info.get("renewal").and_then(|r| r.as_i64()),
        })
    }
}

/// Where a name's owner coin sits, from hsd's `GET /coin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InOurLock {
    /// A FINALIZE of the name at the listing's lock address: the name is in
    /// our lock.
    Finalize,
    /// At the listing's lock address, but not a FINALIZE of the name (a
    /// purchase's TRANSFER, a cancel already, or a covenant of another
    /// name, which hsd should never show as this name's owner): never
    /// adopted.
    Other,
    /// Elsewhere.
    No,
    /// hsd's 404: `info.owner` is the owner at the tip, so its coin is
    /// missing only while a mempool transaction spends it (`fullnode.js`
    /// `getCoin`). Where the name goes is not known yet: no verdict until
    /// that spend is mined and the owner moves to a coin hsd can show.
    SpentInMempool,
}

/// Where the name's owner coin `owner` sits: at this listing's lock address
/// (as a FINALIZE of the name, [`sell::ListingLock::holds`], or anything
/// else) or elsewhere; hsd's 404 is [`InOurLock::SpentInMempool`]. A coin
/// without address or covenant is not hsd's whole answer: an error.
async fn owner_in_our_lock(
    client: &dyn NodeRpc,
    network: Network,
    l: &queries::ShakedexListing,
    owner: &(String, u32),
) -> Result<InOurLock, AppError> {
    let Some(coin) = client.get_coin(&owner.0, owner.1).await? else {
        return Ok(InOurLock::SpentInMempool);
    };
    let Some(at) = sell::CoinAt::of_coin(&coin) else {
        return Err(AppError::Rpc(format!(
            "node did not report the address or covenant of coin {}:{}",
            owner.0, owner.1
        )));
    };
    let lock = listing_lock(network, l)?;
    if lock.holds(at, COV_FINALIZE, None) {
        Ok(InOurLock::Finalize)
    } else if lock.is_at(at) {
        Ok(InOurLock::Other)
    } else {
        Ok(InOurLock::No)
    }
}

// ---------------------------------------------------------------------------
// A listing row's stored fields, read in one place for the jobs and the
// commands (Cancel, Lower price, the cancel's FINALIZE): a field that does not
// read is a corrupted row.
// ---------------------------------------------------------------------------

/// Listing `l`'s stored lock public key.
pub(crate) fn lock_pubkey(l: &queries::ShakedexListing) -> Result<[u8; 33], AppError> {
    hex::decode(&l.lock_pubkey_hex)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| AppError::Other(format!("corrupted listing {}: bad lock key", l.id)))
}

/// Listing `l`'s lock: the lock address of its stored lock public key, and
/// its name.
pub(crate) fn listing_lock(
    network: Network,
    l: &queries::ShakedexListing,
) -> Result<sell::ListingLock, AppError> {
    sell::ListingLock::new(script::lock_address(network, &lock_pubkey(l)?)?, &l.name)
}

/// Listing `l`'s stored price steps (`steps_json`).
pub(crate) fn stored_steps(
    l: &queries::ShakedexListing,
) -> Result<Vec<sell::StoredStep>, AppError> {
    serde_json::from_str(&l.steps_json)
        .map_err(|e| AppError::Other(format!("corrupted listing {}: unreadable steps: {e}", l.id)))
}

/// The before-lock job of [`refresh_listings_with_client`] (R19), for one
/// listing still Locking or ReadyToFinalize, or Aborted within
/// [`ABORT_RECHECK_DAYS`]. Every verdict rests on a found fact, never on the
/// broadcast (hsd answers `sendrawtransaction` with the txid even when it
/// refuses):
///
/// - the linked Cancel transfer's UPDATE mined in a block → Aborted; back in
///   the mempool, or the lock TRANSFER a coin again → Locking;
/// - otherwise, from the name and the lock TRANSFER alone ([`LockOnChain`]):
///   hsd reports no live name → Expired; the lock TRANSFER neither the owner
///   nor a coin while its draft can no longer land (mined, dropped, failed,
///   or deleted) → Aborted — a Cancel transfer sent from anywhere, a REVOKE,
///   a replaced cancel whose older one was mined, a lock TRANSFER that never
///   landed; an Aborted listing whose lock TRANSFER is a coin or the owner
///   again → Locking;
/// - but the owner coin a FINALIZE at this listing's lock address (a
///   FINALIZE into our lock this device did not build) → Restored with that
///   outpoint, never Aborted; another covenant there → unchanged; the
///   owner coin hsd's 404 (spent in the mempool) → unchanged until that
///   spend is mined;
/// - and before either of those last two, a mined purchase of a FINALIZE of
///   this listing into the lock paying its payment address → Sold (R22,
///   [`sale_of_left_lock`]);
/// - the lock TRANSFER the owner and `blocks_until_finalize` of hsd's
///   `info.transfer` 0 at the tip → ReadyToFinalize, not 0 → Locking; the
///   lock TRANSFER a coin in the mempool (`height: -1`), not the owner →
///   Locking; a lock TRANSFER mined in a block but not the owner is not a
///   consistent answer → unchanged, unless its covenant's name height is not
///   hsd's (the name expired and was opened again) → Expired.
///
/// Locking again is refused while another listing of the name is open
/// ([`queries::unabort_shakedex_listing`]). Anything the node does not
/// answer leaves the listing as it is. Sends nothing. An error leaves the
/// listing for the next sync (the caller logs it).
async fn refresh_before_lock(
    conn: &rusqlite::Connection,
    client: &dyn NodeRpc,
    network: Network,
    l: &queries::ShakedexListing,
) -> Result<(), AppError> {
    let Some(lock_transfer_txid) = l.lock_transfer_txid.as_deref() else {
        return Ok(());
    };
    let abortable = l.state.aborts_by_cancel_transfer();
    let aborted = l.state == queries::ListingState::Aborted;
    // The linked cancel first. Its txid is kept on the listing, not read
    // from its draft: a cancel broadcast, dropped and deleted may still be
    // mined.
    if let Some(cancel_txid) = l.abort_txid.as_deref() {
        match cancel_on_chain(client, cancel_txid, lock_transfer_txid).await {
            CancelOnChain::Mined if abortable => {
                queries::abort_shakedex_listing(conn, &l.id)?;
                return Ok(());
            }
            CancelOnChain::NotMined if aborted => {
                relock(conn, l)?;
                return Ok(());
            }
            _ => {}
        }
    }
    match lock_on_chain(client, &l.name, lock_transfer_txid).await? {
        lock @ (LockOnChain::NoName | LockOnChain::Gone { .. }) if abortable => {
            settle_left_lock(conn, client, network, l, lock).await?;
        }
        LockOnChain::Owner { .. } | LockOnChain::Pending if aborted => relock(conn, l)?,
        LockOnChain::Owner {
            transfer: Some(height),
        } if abortable => {
            // hsd judges the FINALIZE at tip + 1 (`chain.js`,
            // `height < ns.transfer + transferLockup`).
            let tip = client.get_blockchain_info().await?.blocks;
            let ready = network.name_params().blocks_until_finalize(height, tip) == 0;
            match (l.state, ready) {
                (queries::ListingState::Locking, true) => {
                    queries::mark_listing_ready(conn, &l.id)?;
                }
                (queries::ListingState::ReadyToFinalize, false) => {
                    queries::mark_listing_locking_again(conn, &l.id)?;
                }
                _ => {}
            }
        }
        LockOnChain::Pending if l.state == queries::ListingState::ReadyToFinalize => {
            queries::mark_listing_locking_again(conn, &l.id)?;
        }
        _ => {}
    }
    Ok(())
}

/// R19: the name has left the lock TRANSFER (`lock` is
/// [`LockOnChain::NoName`] or [`LockOnChain::Gone`]); settle listing `l`
/// from where it went. The one resolution for a listing before the lock and
/// for a Finalizing one whose FINALIZE is dead (the SQL writes accept no
/// other source, [`queries::abort_shakedex_listing`]):
///
/// - hsd reports no live name, or the lock TRANSFER belongs to a
///   registration that is gone → Expired;
/// - the name revoked, or its owner coin readable elsewhere while the lock
///   draft can no longer land → Aborted;
/// - the owner coin a FINALIZE at this listing's lock address → Restored
///   with that outpoint, never Aborted (coordinator (b): only that positive
///   evidence says the name is in our lock, whoever sent the FINALIZE);
/// - but a mined purchase of a FINALIZE of this listing into the lock
///   paying its payment address (R22, [`sale_of_left_lock`]) → Sold, never
///   Aborted; a purchase found whose lock coin hsd does not show to be this
///   listing's → unchanged;
/// - the owner coin at our lock under another covenant, or hsd's 404 for
///   it (spent in the mempool) → unchanged.
async fn settle_left_lock(
    conn: &rusqlite::Connection,
    client: &dyn NodeRpc,
    network: Network,
    l: &queries::ShakedexListing,
    lock: LockOnChain,
) -> Result<(), AppError> {
    let (owner, revoked, renewal) = match lock {
        LockOnChain::NoName => {
            queries::expire_shakedex_listing(conn, &l.id)?;
            return Ok(());
        }
        LockOnChain::Gone {
            owner,
            revoked,
            renewal,
        } => (owner, revoked, renewal),
        LockOnChain::Owner { .. } | LockOnChain::Pending => return Ok(()),
    };
    if !revoked {
        let where_ = owner_in_our_lock(client, network, l, &owner).await?;
        match where_ {
            InOurLock::Finalize => {
                queries::adopt_lock_finalized_elsewhere(conn, &l.id, &owner.0, owner.1)?;
                return Ok(());
            }
            InOurLock::SpentInMempool => return Ok(()),
            InOurLock::Other | InOurLock::No => {
                // R22: the name may have left through our lock — a FINALIZE
                // of ours mined after all, its lock coin bought.
                match sale_of_left_lock(conn, client, network, l, &owner, renewal).await? {
                    LeftSale::Sold { txid, lock, proven } => {
                        let lock = (lock.0.as_str(), lock.1);
                        let n = if proven {
                            queries::sell_listing_through_proven_lock(conn, &l.id, &txid, lock)?
                        } else {
                            queries::sell_shakedex_listing(conn, &l.id, &txid, lock)?
                        };
                        if n == 0 {
                            eprintln!(
                                "shakedex listings: {} ({}): purchase {txid} of lock coin {}:{} \
                                 found, but the listing was not moved to sold from {}",
                                l.id,
                                l.name,
                                lock.0,
                                lock.1,
                                l.state.as_str()
                            );
                        }
                        return Ok(());
                    }
                    LeftSale::Unproven => return Ok(()),
                    LeftSale::None if where_ == InOurLock::Other => return Ok(()),
                    LeftSale::None => {}
                }
            }
        }
    }
    let status = queries::lock_draft_status(conn, l)?;
    if !status.is_some_and(|s| queries::draft_may_still_land(&s)) {
        queries::abort_shakedex_listing(conn, &l.id)?;
    }
    Ok(())
}

/// What [`sale_of_left_lock`] found.
#[derive(Debug, PartialEq, Eq)]
enum LeftSale {
    /// A mined purchase of this listing's lock coin `lock`; `proven` when
    /// the listing had no lock outpoint and `lock` was tied to it by
    /// [`finalize_into_lock`] (written by
    /// [`queries::sell_listing_through_proven_lock`]).
    Sold {
        txid: String,
        lock: (String, u32),
        proven: bool,
    },
    /// A mined purchase paying the payment address out of our lock address,
    /// but hsd does not show the coin it spends as this listing's lock coin
    /// (not readable, or not a FINALIZE spending its lock TRANSFER): no
    /// verdict either way.
    Unproven,
    /// No mined purchase.
    None,
}

/// R22 for a listing whose name left the lock TRANSFER
/// ([`settle_left_lock`]), the name's owner being `owner`. A listing with a
/// stored lock outpoint is judged by [`find_sale`]. One without (a dead
/// FINALIZE's listing back at ReadyToFinalize, or still Locking) has no
/// outpoint for [`find_sale`] to tie a purchase to: a purchase found out of
/// our lock address with any input ([`find_purchases`], every mined one) is
/// only a lead to the coin it spends, tried in turn: it is the lock only
/// when hsd shows it as a
/// FINALIZE at this listing's lock address spending this listing's lock
/// TRANSFER ([`finalize_into_lock`]); [`find_sale`] then judges the
/// listing with that outpoint.
async fn sale_of_left_lock(
    conn: &rusqlite::Connection,
    client: &dyn NodeRpc,
    network: Network,
    l: &queries::ShakedexListing,
    owner: &(String, u32),
    renewal: Option<i64>,
) -> Result<LeftSale, AppError> {
    if l.lock_txid.is_some() && l.lock_vout.is_some() {
        return Ok(match find_sale(conn, client, network, l, owner).await? {
            Sale::Mined { txid, lock } => LeftSale::Sold {
                txid,
                lock,
                proven: false,
            },
            _ => LeftSale::None,
        });
    }
    let leads = find_purchases(conn, client, network, l, owner, None).await?;
    if leads.is_empty() {
        return Ok(LeftSale::None);
    }
    for lead in leads {
        let Sale::Mined { lock, .. } = lead else {
            continue;
        };
        let proven = match finalize_into_lock(client, network, l, &lock, renewal).await {
            Ok(p) => p,
            // Skipping withholds a verdict, never makes one.
            Err(e) => {
                eprintln!(
                    "shakedex listings: {} ({}): transaction {} could not be read, skipped: {e}",
                    l.id, l.name, lock.0
                );
                continue;
            }
        };
        if !proven {
            eprintln!(
                "shakedex listings: {} ({}): a purchase out of our lock spends {}:{}, which the \
                 node does not show as this listing's FINALIZE: not this listing's sale",
                l.id, l.name, lock.0, lock.1
            );
            continue;
        }
        let tied = queries::ShakedexListing {
            lock_txid: Some(lock.0),
            lock_vout: Some(i64::from(lock.1)),
            ..l.clone()
        };
        if let Sale::Mined { txid, lock } = find_sale(conn, client, network, &tied, owner).await? {
            return Ok(LeftSale::Sold {
                txid,
                lock,
                proven: true,
            });
        }
    }
    Ok(LeftSale::Unproven)
}

/// Whether hsd shows `lock` as listing `l`'s lock coin: its transaction,
/// read with [`spend_view`] (`renewal` the block to read it at without the
/// index), is in a block, output `lock.1` is a FINALIZE of the name at the
/// listing's lock address, and input `lock.1` (hsd links a FINALIZE to the
/// input at its index) spends the listing's lock TRANSFER. Not found, or
/// anything else: `false`.
async fn finalize_into_lock(
    client: &dyn NodeRpc,
    network: Network,
    l: &queries::ShakedexListing,
    lock: &(String, u32),
    renewal: Option<i64>,
) -> Result<bool, AppError> {
    let Some(lock_transfer_txid) = l.lock_transfer_txid.as_deref() else {
        return Ok(false);
    };
    let Some(tx) = spend_view(client, &lock.0, renewal).await? else {
        return Ok(false);
    };
    let k = lock.1 as usize;
    let at = listing_lock(network, l)?;
    let spends_our_transfer = tx
        .inputs
        .get(k)
        .is_some_and(|(t, v)| t == lock_transfer_txid && *v == 0);
    let is_our_lock = tx
        .outputs
        .get(k)
        .is_some_and(|o| at.holds(sell::CoinAt::of_output(o), COV_FINALIZE, None));
    Ok(tx.height.is_some() && spends_our_transfer && is_our_lock)
}

fn relock(conn: &rusqlite::Connection, l: &queries::ShakedexListing) -> Result<(), AppError> {
    if queries::unabort_shakedex_listing(conn, &l.id)? == 0 {
        eprintln!(
            "shakedex listings: {} ({}): its abort is no longer on chain, but it stays \
             aborted: another listing of the name is open",
            l.id, l.name
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Listings after the FINALIZE into the lock is built (R19, R22)
// ---------------------------------------------------------------------------

/// Best-effort sync step for every listing (R19, R22): the before-lock job,
/// then the after-lock job, on listing sets both read before either runs
/// ([`refresh_listings_with_client`]). Like the other sync steps it returns
/// silently when the database or the profile's node client cannot be
/// opened; a failed refresh is logged. It only reads the node and the
/// database, so the daemon runs it as the app does; the caller runs it only
/// when the node is authoritative, since a node that is behind would show a
/// mined cancel's lock TRANSFER as unspent and a mined purchase as missing.
pub async fn refresh_listings_step(db_path: &str, profile_id: &str) {
    let conn = match crate::db::connection::open_migrated(db_path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let client = match NodeRpcClient::for_profile(&conn, profile_id) {
        Ok(c) => c,
        Err(_) => return,
    };
    if let Err(e) = refresh_listings_with_client(&conn, &client, profile_id).await {
        eprintln!("shakedex listings: refresh failed for {profile_id}: {e}");
    }
}

/// Both listing jobs, each on the set it takes
/// ([`queries::ListingState::BEFORE_LOCK_JOB`] for [`refresh_before_lock`]'s
/// rules, [`queries::ListingState::AFTER_LOCK_JOB`] for
/// [`refresh_after_lock`]'s), both sets read first: a
/// listing one job moves into the other's set is judged once per sync. Sends
/// nothing. A failure on one listing is logged and leaves it for the next
/// sync.
pub async fn refresh_listings_with_client(
    conn: &rusqlite::Connection,
    client: &dyn NodeRpc,
    profile_id: &str,
) -> Result<(), AppError> {
    let before = queries::list_shakedex_listings_before_lock(conn, profile_id, ABORT_RECHECK_DAYS)?;
    let after = queries::list_shakedex_listings_after_lock(conn, profile_id, SOLD_RECHECK_DAYS)?;
    if before.is_empty() && after.is_empty() {
        return Ok(());
    }
    let network = queries::profile_network(conn, profile_id)?;
    for l in &before {
        if let Err(e) = refresh_before_lock(conn, client, network, l).await {
            eprintln!("shakedex listings: {} ({}): {e}", l.id, l.name);
        }
    }
    for l in &after {
        if let Err(e) = refresh_after_lock(conn, client, network, l).await {
            eprintln!("shakedex listings: {} ({}): {e}", l.id, l.name);
        }
    }
    Ok(())
}

/// The after-lock job of [`refresh_listings_with_client`] (R19, R22, R28),
/// for one listing of [`queries::ListingState::AFTER_LOCK_JOB`] (a Sold one
/// within [`SOLD_RECHECK_DAYS`]), from hsd's `GET /coin` of its lock outpoint
/// `(lock_txid, lock_vout)`:
///
/// - a FINALIZE of the name at the listing's lock address mined in a block → a
///   Finalizing listing Listed; in the mempool (`height: -1`) → a Listed one
///   Finalizing (a reorg took it back); a coin at all → a SalePending or
///   Sold listing Listed (Finalizing while that coin is in the mempool and
///   our FINALIZE draft exists; Restored without a listing file), its
///   purchase forgotten, unless another listing of the name is open by then;
///   mined,
///   with the name's live state gone or its height not the lock coin's → a
///   Listed, Restored or Cancelling listing Expired ([`registration_ended`]);
/// - a Cancelling listing whose cancel can no longer land ([`cancel_dead`]),
///   the lock coin mined in a block → Expired when its registration is gone,
///   otherwise Listed (Restored without a file), the cancel forgotten; the
///   lock coin in the mempool → unchanged;
/// - hsd's 404 for the lock coin while the lock TRANSFER
///   `(lock_transfer_txid, 0)` is a coin again (the FINALIZE into the lock
///   in no block and no mempool): a Finalizing listing whose FINALIZE draft
///   is `failed`, `dropped` or gone ([`finalize_dead`]) → ReadyToFinalize,
///   its lock outpoint, steps and file dropped (they were signed over a
///   coin that does not exist); a Listed one, and a SalePending or Sold one
///   with our FINALIZE draft → Finalizing; a SalePending or Sold one without
///   it (adopted from another device's FINALIZE) → Restored; a Restored one
///   → Locking without its outpoint ([`queries::unadopt_restored_lock`]); a
///   Cancelling one whose cancel is dead → where a Listed one (a Restored
///   one without a file) goes, the cancel forgotten
///   ([`uncancel_over_a_dead_lock_finalize`]);
/// - hsd's 404 for both, for a Finalizing listing whose FINALIZE is dead:
///   something else spent the lock TRANSFER → settled from the name as
///   before the lock ([`settle_left_lock`]: Expired, Aborted, Restored, or
///   unchanged);
/// - any other 404 (the lock coin spent in a block or in the mempool) is no
///   verdict alone: the name's owner is read; no live name → a Listed,
///   SalePending or Restored listing Expired; the owner a mined TRANSFER
///   out of the lock coin committing to an address of ours, linked from it,
///   or a mined FINALIZE home at an address of ours spending such a TRANSFER
///   ([`cancel_of_lock`]) → CancelAwaitingFinalize, and a cancel of ours
///   that lost is released ([`cancel_mined`]); otherwise a purchase is looked
///   for ([`find_sale`]); one in the mempool while the owner is still the
///   lock coin → SalePending, one mined while the owner has moved → Sold, a
///   cancel of ours released ([`sell_releasing_cancel`]); a
///   lock restored by name (no payment address) → Sold only by a mined price
///   step (sighash `0x84`) out of its lock coin ([`sale_of_restored_lock`]);
///   a Sold listing moves only by [`queries::resell_sold_listing`]: back to
///   SalePending when its purchase is in the mempool again, or to the txid
///   of another purchase of its lock coin mined instead;
/// - a coin at another address or of another covenant, a reply missing the
///   coin's address, covenant or height, a name reply missing `info` or the
///   owner, or a read error → unchanged.
///
/// A listing whose cancel TRANSFER is mined
/// ([`queries::ListingState::CANCEL_MINED`]) is followed from that TRANSFER
/// instead ([`cancel_on_its_way_home`]).
///
/// Sends nothing. An error leaves the listing for the next sync (the caller
/// logs it).
async fn refresh_after_lock(
    conn: &rusqlite::Connection,
    client: &dyn NodeRpc,
    network: Network,
    l: &queries::ShakedexListing,
) -> Result<(), AppError> {
    if queries::ListingState::CANCEL_MINED.contains(&l.state) {
        return cancel_on_its_way_home(conn, client, network, l).await;
    }
    let Some((lock_txid, lock_vout)) = stored_lock(l)? else {
        return Ok(());
    };
    match client.get_coin(lock_txid, lock_vout).await? {
        Some(coin) => lock_coin_held(conn, client, network, l, &coin, (lock_txid, lock_vout)).await,
        None => lock_coin_spent(conn, client, network, l, (lock_txid, lock_vout)).await,
    }
}

/// Whether the registration a locked listing's lock coin belongs to is gone:
/// hsd reports no live state for the name (`info: null`), or the name's
/// height is not the one the lock coin's covenant commits to (it expired and
/// was opened again, `ns.reset`). `lock_height` `None` (a covenant without a
/// readable height item) decides only the first. A live name without its
/// height is not hsd's whole answer: an error.
fn registration_ended(
    reply: &serde_json::Value,
    lock_height: Option<u32>,
) -> Result<bool, AppError> {
    let Some(info) = name_info(reply)? else {
        return Ok(true);
    };
    let height = info
        .get("height")
        .and_then(|h| h.as_u64())
        .ok_or_else(|| AppError::Rpc("node did not report the name's height".into()))?;
    Ok(lock_height.is_some_and(|h| u64::from(h) != height))
}

/// The lock coin is a coin: a FINALIZE at the listing's lock address. Mined,
/// a Finalizing listing is Listed; in the mempool (`height: -1`), a Listed
/// one is Finalizing (a reorg took it back). A SalePending or Sold listing
/// goes back to Listed (Restored without a listing file): hsd answers 404 for
/// a coin any mempool transaction spends, so its purchase is in no block and
/// no mempool of this node. A Listed or Restored listing whose name has no
/// live state, or whose name height is not the lock coin's, is Expired
/// ([`registration_ended`]). Anything else at that outpoint is an error, and
/// the listing stays as it is.
async fn lock_coin_held(
    conn: &rusqlite::Connection,
    client: &dyn NodeRpc,
    network: Network,
    l: &queries::ShakedexListing,
    coin: &crate::noncustodial::rpc::NodeCoin,
    lock: (&str, u32),
) -> Result<(), AppError> {
    let (Some(at), Some(covenant)) = (sell::CoinAt::of_coin(coin), coin.covenant.as_ref()) else {
        return Err(AppError::Rpc(format!(
            "node did not report the address or covenant of lock coin {}:{}",
            lock.0, lock.1
        )));
    };
    if !listing_lock(network, l)?.holds(at, COV_FINALIZE, None) {
        return Err(AppError::Other(format!(
            "lock coin {}:{} is not a FINALIZE of the name at the listing's lock address",
            lock.0, lock.1
        )));
    }
    // Each write moves only the state it names; any other state is left as
    // it is.
    match (l.state, coin.mined_height()?) {
        (queries::ListingState::Finalizing, Some(_)) => {
            queries::mark_listing_listed(conn, &l.id)?;
        }
        (queries::ListingState::Listed, None) => {
            queries::mark_listing_finalizing_again(conn, &l.id)?;
        }
        (queries::ListingState::SalePending | queries::ListingState::Sold, mined) => {
            let seen = match mined {
                Some(_) => LockFinalizeSeen::Mined,
                None => LockFinalizeSeen::Mempool,
            };
            if queries::unsell_shakedex_listing(conn, &l.id, unsell_target(l, seen))? == 0 {
                eprintln!(
                    "shakedex listings: {} ({}): its purchase is no longer on chain, but it \
                     stays sold: another listing of the name is open",
                    l.id, l.name
                );
            }
        }
        // R28: the lock coin a mined coin (no transaction of this node
        // spends it) and our cancel can no longer land: Listed again
        // (Restored without a file), the cancel forgotten, unless the name's
        // registration is gone (Expired, never Listed first). In the mempool
        // (a reorg of the FINALIZE into the lock) it stays Cancelling: a
        // listing is not put back on the market over an unmined FINALIZE.
        (queries::ListingState::Cancelling, Some(_)) if cancel_dead(conn, l)? => {
            let reply = client.get_name_info(&l.name).await?;
            if registration_ended(&reply, purchase::covenant_name_height(covenant))? {
                queries::expire_locked_listing(conn, &l.id)?;
            } else {
                let to = queries::ListingState::uncancel_target(l.listing_file_json.is_some());
                queries::uncancel_listing(conn, &l.id, to)?;
            }
        }
        (
            queries::ListingState::Listed
            | queries::ListingState::Restored
            | queries::ListingState::Cancelling,
            Some(_),
        ) => {
            let reply = client.get_name_info(&l.name).await?;
            if registration_ended(&reply, purchase::covenant_name_height(covenant))? {
                queries::expire_locked_listing(conn, &l.id)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Where hsd shows the FINALIZE into the lock of a listing whose purchase
/// is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LockFinalizeSeen {
    /// The lock coin is a coin mined in a block.
    Mined,
    /// The lock coin is a coin in the mempool (`height: -1`).
    Mempool,
    /// In no block and no mempool: the lock TRANSFER is a coin again.
    Nowhere,
}

/// Where a SalePending or Sold listing goes back to when its purchase is
/// gone (R22, [`queries::unsell_shakedex_listing`]): Finalizing while the
/// FINALIZE into the lock is not mined and the listing has our FINALIZE
/// draft (its file is not exported over an unmined FINALIZE); Listed when
/// the lock coin is a coin and the listing has its file; Restored otherwise
/// (no file, or a FINALIZE from another device that is in no block and no
/// mempool: the next sync takes a Restored lock adopted from our lock
/// TRANSFER to Locking).
fn unsell_target(l: &queries::ShakedexListing, seen: LockFinalizeSeen) -> queries::ListingState {
    use queries::ListingState as S;
    let draft = l.lock_finalize_draft_id.is_some();
    let file = l.listing_file_json.is_some();
    match seen {
        LockFinalizeSeen::Mempool | LockFinalizeSeen::Nowhere if draft => S::Finalizing,
        LockFinalizeSeen::Mined | LockFinalizeSeen::Mempool if file => S::Listed,
        _ => S::Restored,
    }
}

/// hsd's 404 for the lock coin: spent in a block or in the mempool
/// (`fullnode.js` `getCoin`, `mempool.isSpent`). Never a verdict alone.
async fn lock_coin_spent(
    conn: &rusqlite::Connection,
    client: &dyn NodeRpc,
    network: Network,
    l: &queries::ShakedexListing,
    lock: (&str, u32),
) -> Result<(), AppError> {
    // The lock TRANSFER a coin again: hsd answers 404 for a coin any mempool
    // transaction spends, so the FINALIZE into the lock is in no block and
    // no mempool of this node.
    if let Some(lock_transfer_txid) = l.lock_transfer_txid.as_deref() {
        if client.get_coin(lock_transfer_txid, 0).await?.is_some() {
            match l.state {
                // Our FINALIZE never landed: Finalize & sign may run again.
                queries::ListingState::Finalizing if finalize_dead(conn, l)? => {
                    queries::revert_listing_to_ready(conn, &l.id)?;
                }
                queries::ListingState::Listed => {
                    queries::mark_listing_finalizing_again(conn, &l.id)?;
                }
                queries::ListingState::SalePending | queries::ListingState::Sold => {
                    let to = unsell_target(l, LockFinalizeSeen::Nowhere);
                    if queries::unsell_shakedex_listing(conn, &l.id, to)? == 0 {
                        eprintln!(
                            "shakedex listings: {} ({}): its FINALIZE is no longer on chain, \
                             but it stays sold: another listing of the name is open",
                            l.id, l.name
                        );
                    }
                }
                queries::ListingState::Restored => {
                    queries::unadopt_restored_lock(conn, &l.id)?;
                }
                // R28: our cancel can no longer land, and the lock coin it
                // spent is in no block and no mempool: the cancel is
                // forgotten and the listing goes where a Listed one (a
                // Restored one without a file) goes.
                queries::ListingState::Cancelling if cancel_dead(conn, l)? => {
                    uncancel_over_a_dead_lock_finalize(conn, l)?;
                }
                _ => {}
            }
            return Ok(());
        }
    }
    if l.state == queries::ListingState::Finalizing && finalize_dead(conn, l)? {
        // Something else spent the lock TRANSFER (an older cancel mined
        // later, another device's FINALIZE, a REVOKE): settle it as before
        // the lock. A 404 that only means "spent in the mempool" leaves it,
        // as there.
        let Some(lock_transfer_txid) = l.lock_transfer_txid.as_deref() else {
            return Ok(());
        };
        let gone = lock_on_chain(client, &l.name, lock_transfer_txid).await?;
        settle_left_lock(conn, client, network, l, gone).await?;
        return Ok(());
    }
    let reply = client.get_name_info(&l.name).await?;
    let Some(info) = name_info(&reply)? else {
        // No live name: the listing ended with it (R31's lockup risk, or a
        // listing left to expire). Sold stays Sold, and a Finalizing
        // listing is not moved ([`queries::expire_locked_listing`]).
        queries::expire_locked_listing(conn, &l.id)?;
        return Ok(());
    };
    let owner = owner_of(info)?;
    let owner_is_lock = owner.0 == lock.0 && owner.1 == lock.1;
    // R28: a mined TRANSFER out of this lock coin to an address of ours is
    // a cancel (ours, another device's, or our own purchase), judged before
    // any sale: the sale rule says "not ours" of it.
    if queries::ListingWrite::CancelMined.from().contains(&l.state) {
        if let Some(c) = cancel_of_lock(conn, client, network, l, &owner, lock).await? {
            return cancel_mined(conn, l, (&c.0, c.1), lock, queries::release_losing_cancel);
        }
    }
    if l.payment_address.is_none() {
        // A lock restored by name: no payment address to find a purchase
        // by, only the owner. While the owner is still the lock coin, hsd's
        // `GET /coin` of it is the 404 that brought us here: no verdict.
        if let Some(txid) = sale_of_restored_lock(conn, client, network, l, &owner, lock).await? {
            sell_releasing_cancel(conn, l, &txid, lock, queries::release_losing_cancel)?;
        }
        return Ok(());
    }
    // hsd moves the owner only when a block is connected: a purchase is
    // Pending while the owner is still the lock coin, and Mined only once it
    // is not. Two facts that disagree are no verdict.
    match (
        find_sale(conn, client, network, l, &owner).await?,
        owner_is_lock,
    ) {
        // A Sold listing whose purchase is back in the mempool (a reorg),
        // or whose lock coin another purchase bought instead: only Sold's
        // own guarded write moves it.
        (Sale::Pending { txid, lock }, true) if l.state == queries::ListingState::Sold => {
            let to = queries::ListingState::SalePending;
            let lock = (lock.0.as_str(), lock.1);
            if queries::resell_sold_listing(conn, &l.id, to, &txid, lock)? == 0 {
                eprintln!(
                    "shakedex listings: {} ({}): its purchase is back in the mempool, but it \
                     stays sold: another listing of the name is open",
                    l.id, l.name
                );
            }
        }
        // The purchase the listing was sold by: nothing to write (a write
        // would move `updated_at`, and the re-check window would never
        // close).
        (Sale::Mined { txid, .. }, false)
            if l.state == queries::ListingState::Sold
                && l.sold_txid.as_deref().is_some_and(|s| s == txid) => {}
        (Sale::Mined { txid, lock }, false) if l.state == queries::ListingState::Sold => {
            let to = queries::ListingState::Sold;
            queries::resell_sold_listing(conn, &l.id, to, &txid, (lock.0.as_str(), lock.1))?;
        }
        (Sale::Pending { txid, lock }, true) => {
            queries::mark_listing_sale_pending(conn, &l.id, &txid, (lock.0.as_str(), lock.1))?;
        }
        (Sale::Mined { txid, lock }, false) => {
            let lock = (lock.0.as_str(), lock.1);
            sell_releasing_cancel(conn, l, &txid, lock, queries::release_losing_cancel)?;
        }
        _ => {}
    }
    Ok(())
}

/// R22 for a Restored lock without a payment address (restored by name,
/// R32), the name's owner being `owner` and its lock coin `lock` spent: the
/// owner coin's transaction, read with `GET /tx` or, without the index, in
/// the block at the owner coin's height (`GET /coin`), is
/// [`sell::sale_out_of_restored_lock`] at the owner's index: its txid then.
/// Anything else is `None`, no verdict: the owner coin hsd's 404 (spent in
/// the mempool; or the lock coin itself, still the owner while a purchase
/// of it is in the mempool), its transaction not found or not in a block,
/// or not a price step's purchase of this lock coin (our cancel, `0x83`
/// committing to an address of ours, is judged first as a cancel,
/// [`cancel_of_lock`]). A lock input without its witness is an error. When
/// the buyer's FINALIZE is mined before a sync sees the TRANSFER as the
/// owner, the owner's transaction is that FINALIZE: not followed back, no
/// verdict. Only the write's source states move
/// ([`queries::ListingWrite::Sell`]).
async fn sale_of_restored_lock(
    conn: &rusqlite::Connection,
    client: &dyn NodeRpc,
    network: Network,
    l: &queries::ShakedexListing,
    owner: &(String, u32),
    lock: (&str, u32),
) -> Result<Option<String>, AppError> {
    let Some(coin) = client.get_coin(&owner.0, owner.1).await? else {
        return Ok(None);
    };
    let Some(tx) = spend_view(client, &owner.0, coin.mined_height()?).await? else {
        return Ok(None);
    };
    let own = own_addresses(conn, &l.wallet_profile_id)?;
    let at = listing_lock(network, l)?;
    let sold = sell::sale_out_of_restored_lock(&tx, owner.1, lock, &at, network, &own)?;
    Ok(sold.then(|| owner.0.clone()))
}

/// What the chain shows about a purchase of a listing's lock coin (R22).
/// `lock` is the coin the purchase spends, the input [`sell::purchase_in`]
/// found spending the listing's stored lock outpoint (a FINALIZE at its lock
/// address, checked when the listing got it), which the database write
/// compares again ([`queries::sell_shakedex_listing`]).
#[derive(Debug, PartialEq, Eq)]
enum Sale {
    /// Mined in a block.
    Mined { txid: String, lock: (String, u32) },
    /// In the node's mempool.
    Pending { txid: String, lock: (String, u32) },
    /// No purchase found: no verdict.
    None,
}

/// The name's owner outpoint from `info`; a reply without `owner.hash` or
/// `owner.index` is not hsd's whole answer.
fn owner_of(info: &serde_json::Value) -> Result<(String, u32), AppError> {
    let owner = info.get("owner");
    let hash = owner.and_then(|o| o.get("hash")).and_then(|h| h.as_str());
    let index = owner
        .and_then(|o| o.get("index"))
        .and_then(|i| i.as_u64())
        .and_then(|i| u32::try_from(i).ok());
    match (hash, index) {
        (Some(h), Some(i)) => Ok((h.to_string(), i)),
        _ => Err(AppError::Rpc("node did not report the name's owner".into())),
    }
}

/// hsd's view of transaction `txid`: `GET /tx` (the mempool always, a block
/// only with `--index-tx`); on hsd's not-found, the block at `seen_at`, the
/// height our coin of it was last seen mined at (`getblockhash` +
/// `getblock`, no index needed). `None`: not found where it was looked for
/// (a coin row of a transaction that left the mempool, or a block that a
/// reorg replaced), so no verdict.
async fn spend_view(
    client: &dyn NodeRpc,
    txid: &str,
    seen_at: Option<i64>,
) -> Result<Option<sell::SpendView>, AppError> {
    let tx = client.get_tx_by_hash(txid).await?;
    if !tx.is_null() {
        return sell::spend_view_from_rest(&tx).map(Some);
    }
    let Some(height) = seen_at.filter(|h| *h >= 0) else {
        return Ok(None);
    };
    let hash = client.get_block_hash(height).await?;
    let block = client.get_block(&hash).await?;
    sell::spend_view_from_block(&block, txid)
}

/// Transaction `txid` found on chain ([`spend_view`], `seen_at` the height
/// to read its block at without the index) and judged by
/// [`sell::purchase_in`]: its block height (`None` in the mempool) and the
/// lock coin it spends, or `None` when it is not found or is not a purchase
/// of `p`'s lock coin.
async fn purchase_found(
    client: &dyn NodeRpc,
    txid: &str,
    seen_at: Option<i64>,
    p: &sell::PurchaseOf<'_>,
) -> Result<Option<(Option<i64>, (String, u32))>, AppError> {
    let Some(tx) = spend_view(client, txid, seen_at).await? else {
        return Ok(None);
    };
    Ok(sell::purchase_in(&tx, p)?.map(|spent| (tx.height, spent)))
}

/// R22: look for a purchase of listing `l`'s lock coin, the name's owner
/// being `owner`. Nothing without a payment address (a lock restored by
/// name, until its file is imported) or without a lock outpoint. Every
/// verdict rests on the purchase transaction found on chain and judged by
/// [`sell::purchase_in`] against the listing's STORED lock outpoint (every
/// lock coin of a name sits at the same lock address, ADR 0004, so the
/// address alone does not say which listing was bought); the lock coin
/// returned is the input `purchase_in` found. A coin of ours in
/// `tracked_utxos` is only a lead to a transaction. Two ways, neither needing
/// hsd's transaction index:
///
/// - the owner first: hsd moves it to the purchase's TRANSFER (its output 0)
///   when its block is connected. When the wallet has a coin of that
///   transaction at the payment address and hsd shows the owner as a coin
///   that is a TRANSFER of the name at our lock address: committing to an
///   address of ours it is a cancel, judged first (R28, [`cancel_of_lock`])
///   → no verdict here; in the mempool it
///   is not a mined owner → no verdict; mined, its transaction is read
///   (`GET /tx`, or the block at the owner coin's own height) and is Mined
///   when `purchase_in` accepts it and it is in a block;
/// - otherwise (the buyer finalized since, the purchase is in the mempool,
///   or the owner is another lock coin's purchase) each transaction that
///   paid our payment address, read the same way at the height our coin of
///   it was seen mined: accepted in the mempool → Pending, in a block →
///   Mined.
///
/// A lead (or the owner's transaction) that cannot be read is logged and
/// skipped: skipping can only withhold a verdict, never make one, since any
/// verdict still needs its own transaction found and accepted, so one
/// unreadable transaction does not hold the listing for ever.
async fn find_sale(
    conn: &rusqlite::Connection,
    client: &dyn NodeRpc,
    network: Network,
    l: &queries::ShakedexListing,
    owner: &(String, u32),
) -> Result<Sale, AppError> {
    let Some(lock) = stored_lock(l)? else {
        return Ok(Sale::None);
    };
    let found = find_purchases(conn, client, network, l, owner, Some(lock)).await?;
    // `find_purchases` with a lock returns at most the first purchase found;
    // none found is no verdict, as `Sale::None` says.
    Ok(found.into_iter().next().unwrap_or(Sale::None))
}

/// Listing `l`'s stored lock outpoint, `None` while it has none; a stored
/// output index that is not a `u32` is a corrupted row.
pub(crate) fn stored_lock(l: &queries::ShakedexListing) -> Result<Option<(&str, u32)>, AppError> {
    let (Some(txid), Some(vout)) = (l.lock_txid.as_deref(), l.lock_vout) else {
        return Ok(None);
    };
    let vout = u32::try_from(vout)
        .map_err(|_| AppError::Other(format!("corrupted listing {}: bad lock output", l.id)))?;
    Ok(Some((txid, vout)))
}

/// [`find_sale`]'s search with the lock coin `lock` the purchase must spend:
/// the first purchase found, mined or in the mempool. `None` accepts a
/// purchase of any coin at the listing's lock address and returns every
/// mined one found (in the mempool they are skipped); the coin each spends
/// is only a lead, never a listing's lock outpoint until
/// [`finalize_into_lock`] ties it to the listing ([`sale_of_left_lock`]).
async fn find_purchases(
    conn: &rusqlite::Connection,
    client: &dyn NodeRpc,
    network: Network,
    l: &queries::ShakedexListing,
    owner: &(String, u32),
    lock: Option<(&str, u32)>,
) -> Result<Vec<Sale>, AppError> {
    let Some(payment) = l.payment_address.as_deref() else {
        return Ok(vec![]);
    };
    let first_only = lock.is_some();
    let mut found: Vec<Sale> = Vec::new();
    let profile = &l.wallet_profile_id;
    let at = listing_lock(network, l)?;
    let own = own_addresses(conn, profile)?;
    let p = sell::PurchaseOf {
        network,
        lock,
        at: &at,
        payment_address: payment,
        own: &own,
    };
    let unreadable = |txid: &str, e: AppError| {
        eprintln!(
            "shakedex listings: {} ({}): transaction {txid} could not be read, skipped: {e}",
            l.id, l.name
        );
    };
    if queries::own_coin_in_tx(conn, profile, &owner.0, payment)? {
        if let Some(coin) = client.get_coin(&owner.0, owner.1).await? {
            let Some(owner_at) = sell::CoinAt::of_coin(&coin) else {
                return Err(AppError::Rpc(format!(
                    "node did not report the address or covenant of coin {}:{}",
                    owner.0, owner.1
                )));
            };
            if at.holds(owner_at, COV_TRANSFER, None) {
                if sell::commitment_is_ours(owner_at.items, network, &own)? {
                    return Ok(vec![]);
                }
                // hsd names a coin the owner only once its block is
                // connected: an owner coin shown in the mempool is not a
                // mined purchase, so no verdict.
                let Some(height) = coin.mined_height()? else {
                    return Ok(vec![]);
                };
                match purchase_found(client, &owner.0, Some(height), &p).await {
                    Ok(Some((Some(_), spent))) => {
                        found.push(Sale::Mined {
                            txid: owner.0.clone(),
                            lock: spent,
                        });
                        if first_only {
                            return Ok(found);
                        }
                    }
                    // Not a purchase of our lock coin (or a view that
                    // disagrees with the mined owner): look at the leads.
                    Ok(_) => {}
                    Err(e) => unreadable(&owner.0, e),
                }
            }
        }
    }
    for (txid, seen_at) in queries::own_coins_at(conn, profile, payment)? {
        let sale = match purchase_found(client, &txid, seen_at, &p).await {
            Ok(Some((Some(_), spent))) => Sale::Mined { txid, lock: spent },
            Ok(Some((None, spent))) if first_only => Sale::Pending { txid, lock: spent },
            Ok(_) => continue,
            Err(e) => {
                unreadable(&txid, e);
                continue;
            }
        };
        if first_only {
            return Ok(vec![sale]);
        }
        if !found.contains(&sale) {
            found.push(sale);
        }
    }
    Ok(found)
}

/// Whether a Finalizing listing's FINALIZE draft is dead: it can no longer
/// be mined (`failed`, `dropped`, or its row gone; not
/// [`queries::draft_alive`]). The SQL writes ask the same of the database
/// ([`queries::abort_shakedex_listing`]).
fn finalize_dead(
    conn: &rusqlite::Connection,
    l: &queries::ShakedexListing,
) -> Result<bool, AppError> {
    draft_dead(conn, l.lock_finalize_draft_id.as_deref())
}

/// Whether a Cancelling listing's cancel can no longer land from this
/// device: no cancel draft (another device's cancel, taken back by a
/// reorg), its row gone, or `failed`/`dropped`.
fn cancel_dead(
    conn: &rusqlite::Connection,
    l: &queries::ShakedexListing,
) -> Result<bool, AppError> {
    draft_dead(conn, l.cancel_draft_id.as_deref())
}

/// Whether the draft `id` can no longer be mined: none, its row gone, or
/// `failed`/`dropped` (not [`queries::draft_alive`]). The SQL writes ask the
/// same of the database.
fn draft_dead(conn: &rusqlite::Connection, id: Option<&str>) -> Result<bool, AppError> {
    let status = match id {
        Some(id) => queries::get_tx_draft(conn, id)?.map(|d| d.status),
        None => None,
    };
    Ok(!status.is_some_and(|s| queries::draft_alive(&s)))
}

/// Every derived address of the profile: what "ours" means for a TRANSFER's
/// commitment (R22, R28).
fn own_addresses(conn: &rusqlite::Connection, profile: &str) -> Result<HashSet<String>, AppError> {
    Ok(queries::get_profile_addresses(conn, profile)?
        .into_iter()
        .collect())
}

/// R28: the mined cancel out of listing `l`'s lock coin `lock`, found from
/// the name's owner `owner`: its outpoint, the TRANSFER of the name at our
/// lock committing to an address of ours whose transaction spends `lock` at
/// its index ([`sell::cancel_out_of_lock`]). Two owners lead to it:
///
/// - the owner is that TRANSFER, mined;
/// - the owner is a mined FINALIZE of the name at an address of ours whose
///   input k spends a TRANSFER ([`sell::cancel_finalized_home`]): the cancel
///   was mined and finalized home (from another same-seed device) before
///   this device synced. That TRANSFER's transaction is read with `GET /tx`
///   (its block is not known, so without the transaction index it is not
///   found: no verdict) and must be the cancel above.
///
/// `None` (no verdict here) for the owner still the lock coin (a spend of it
/// is at most in the mempool), the owner coin hsd's 404 (spent in the
/// mempool), anything else as the owner (a TRANSFER committing elsewhere is
/// a sale's, judged next), in the mempool, or a transaction not found or not
/// linked from `lock`. A coin without its address or covenant is not hsd's
/// answer.
async fn cancel_of_lock(
    conn: &rusqlite::Connection,
    client: &dyn NodeRpc,
    network: Network,
    l: &queries::ShakedexListing,
    owner: &(String, u32),
    lock: (&str, u32),
) -> Result<Option<(String, u32)>, AppError> {
    if owner.0 == lock.0 && owner.1 == lock.1 {
        return Ok(None);
    }
    let Some(coin) = client.get_coin(&owner.0, owner.1).await? else {
        return Ok(None);
    };
    let Some(owner_at) = sell::CoinAt::of_coin(&coin) else {
        return Err(AppError::Rpc(format!(
            "node did not report the address or covenant of coin {}:{}",
            owner.0, owner.1
        )));
    };
    let at = listing_lock(network, l)?;
    let own = own_addresses(conn, &l.wallet_profile_id)?;
    let transfer = at.holds(owner_at, COV_TRANSFER, None);
    let home = owner_at.covenant_type == COV_FINALIZE && own.contains(owner_at.address);
    if transfer && !sell::commitment_is_ours(owner_at.items, network, &own)? || !transfer && !home {
        return Ok(None);
    }
    // hsd names a coin the owner only once its block is connected; a coin
    // shown in the mempool is no mined cancel.
    let Some(height) = coin.mined_height()? else {
        return Ok(None);
    };
    let Some(tx) = spend_view(client, &owner.0, Some(height)).await? else {
        return Ok(None);
    };
    let cancel = if transfer {
        (owner.clone(), tx)
    } else {
        let Some(spent) = tx.inputs.get(owner.1 as usize).cloned() else {
            return Ok(None);
        };
        if !sell::cancel_finalized_home(&tx, owner.1, (&spent.0, spent.1), &at, &own) {
            return Ok(None);
        }
        let Some(transfer_tx) = spend_view(client, &spent.0, None).await? else {
            return Ok(None);
        };
        (spent, transfer_tx)
    };
    let (outpoint, tx) = cancel;
    let found = sell::cancel_out_of_lock(&tx, outpoint.1, lock, &at, network, &own)?;
    Ok(found.then_some(outpoint))
}

/// How a cancel of ours that lost is released: [`queries::release_losing_cancel`],
/// or [`queries::release_replaced_cancel`] where the spender replaced our
/// mined cancel in a reorg.
type ReleaseCancel = fn(&rusqlite::Connection, &str, &str, &str) -> Result<bool, AppError>;

/// R28: CancelAwaitingFinalize with the mined cancel `cancel`, linked from
/// `lock`; when it is not our own cancel draft's transaction (another
/// device's cancel, or a purchase of our own), our cancel can never land
/// and its coins are released (`release`, which compares `cancel`'s txid,
/// the spender read from hsd, with our draft's), in the same database
/// transaction.
fn cancel_mined(
    conn: &rusqlite::Connection,
    l: &queries::ShakedexListing,
    cancel: (&str, u32),
    lock: (&str, u32),
    release: ReleaseCancel,
) -> Result<(), AppError> {
    let tx = conn.unchecked_transaction()?;
    if queries::mark_listing_cancel_mined(&tx, &l.id, cancel, lock)? == 1 {
        release(&tx, &l.id, cancel.0, cancel::CANCEL_LOST_TO_ANOTHER)?;
    }
    tx.commit()?;
    Ok(())
}

/// R22, R28: Sold by the mined purchase `purchase_txid` of `lock`; a cancel
/// this device built for the listing can never land now, so its coins are
/// released in the same database transaction (`release`, the purchase the
/// spender read from hsd). Returns how many rows the sale moved.
fn sell_releasing_cancel(
    conn: &rusqlite::Connection,
    l: &queries::ShakedexListing,
    purchase_txid: &str,
    lock: (&str, u32),
    release: ReleaseCancel,
) -> Result<usize, AppError> {
    let tx = conn.unchecked_transaction()?;
    let n = queries::sell_shakedex_listing(&tx, &l.id, purchase_txid, lock)?;
    if n == 1 {
        release(&tx, &l.id, purchase_txid, cancel::CANCEL_LOST_TO_PURCHASE)?;
    }
    tx.commit()?;
    Ok(n)
}

/// R28 with R22's reorg rule: a Cancelling listing whose cancel is dead
/// ([`cancel_dead`]) while the lock TRANSFER is a coin again (the FINALIZE
/// into the lock in no block and no mempool). The cancel is forgotten
/// ([`queries::uncancel_listing`] to
/// [`queries::ListingState::uncancel_target`]) and, in the same database
/// transaction, the listing takes the move a Listed one (Finalizing) or a
/// Restored one (Locking without its outpoint) takes there. Nothing is
/// written unless both moves apply.
fn uncancel_over_a_dead_lock_finalize(
    conn: &rusqlite::Connection,
    l: &queries::ShakedexListing,
) -> Result<(), AppError> {
    let to = queries::ListingState::uncancel_target(l.listing_file_json.is_some());
    let tx = conn.unchecked_transaction()?;
    if queries::uncancel_listing(&tx, &l.id, to)? != 1 {
        return Ok(());
    }
    let moved = match to {
        queries::ListingState::Listed => queries::mark_listing_finalizing_again(&tx, &l.id)?,
        _ => queries::unadopt_restored_lock(&tx, &l.id)?,
    };
    if moved == 1 {
        tx.commit()?;
    }
    Ok(())
}

/// A listing's mined cancel outpoint `(cancel_txid, cancel_vout)`; a row in
/// [`queries::ListingState::CANCEL_MINED`] without it, or with an output
/// index that is not a `u32`, is corrupted.
pub(crate) fn stored_cancel(l: &queries::ShakedexListing) -> Result<(&str, u32), AppError> {
    let corrupted = || AppError::Other(format!("corrupted listing {}: no mined cancel", l.id));
    let txid = l.cancel_txid.as_deref().ok_or_else(corrupted)?;
    let vout = l
        .cancel_vout
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(corrupted)?;
    Ok((txid, vout))
}

/// R28, a listing whose cancel TRANSFER is mined (CancelAwaitingFinalize,
/// CancelFinalizing), from hsd's `GET /coin` of that TRANSFER:
///
/// - a coin mined in a block: no live name, or hsd's name height not the
///   one it commits to (the name expired and was opened again,
///   [`registration_ended`]) → Expired; the name's owner: the blocks left
///   until its FINALIZE is valid at tip + 1
///   (`NameParams::blocks_until_finalize` of hsd's `info.transfer`) are
///   stored for the reminder, and a CancelFinalizing listing whose FINALIZE
///   draft is dead ([`draft_dead`]) → CancelAwaitingFinalize; a mined coin
///   that is not the owner is no consistent answer → unchanged;
/// - a coin in the mempool (`height: -1`), or hsd's 404 while the lock coin
///   is a coin again (the cancel in no block and no mempool): a reorg took
///   the cancel back → Cancelling ([`queries::mark_listing_cancel_unmined`],
///   for the stored cancel txid);
/// - hsd's 404 otherwise (the TRANSFER spent in a block or the mempool): the
///   name's owner is read; no live name → Expired; the owner a mined
///   FINALIZE of the name at an address of ours spending the cancel's
///   TRANSFER ([`sell::cancel_finalized_home`], its transaction read with
///   `GET /tx` or in the block at the owner coin's height) → Cancelled,
///   whoever sent it (a cancel [`cancel_of_lock`] found from its FINALIZE
///   home ends here on the next sync); otherwise a reorg may have replaced
///   the cancel, and the lock coin's spender is judged on mined facts
///   ([`cancel_replaced`]): another mined cancel out of it →
///   CancelAwaitingFinalize with that outpoint, a mined purchase of it →
///   Sold, our cancel released either way; anything else (the owner coin's
///   404: the FINALIZE in the mempool; a spender in the mempool) →
///   unchanged.
///
/// A reply missing a field the verdict reads, or a coin there that is not
/// a TRANSFER of the name at the lock address, is an error: unchanged.
/// Sends nothing.
async fn cancel_on_its_way_home(
    conn: &rusqlite::Connection,
    client: &dyn NodeRpc,
    network: Network,
    l: &queries::ShakedexListing,
) -> Result<(), AppError> {
    let cancel = stored_cancel(l)?;
    let at = listing_lock(network, l)?;
    if let Some(coin) = client.get_coin(cancel.0, cancel.1).await? {
        match at.stored_coin(&coin, cancel, COV_TRANSFER) {
            Ok(_) => {}
            Err(sell::StoredCoinRefusal::Unreadable) => {
                return Err(AppError::Rpc(format!(
                    "node did not report the address or covenant of cancel {}:{}",
                    cancel.0, cancel.1
                )));
            }
            Err(sell::StoredCoinRefusal::SomethingElse) => {
                return Err(AppError::Other(format!(
                    "cancel {}:{} is not a TRANSFER of the name at the listing's lock address",
                    cancel.0, cancel.1
                )));
            }
        }
        if coin.mined_height()?.is_none() {
            queries::mark_listing_cancel_unmined(conn, &l.id, cancel.0)?;
            return Ok(());
        }
        let reply = client.get_name_info(&l.name).await?;
        let cov_height = coin
            .covenant
            .as_ref()
            .and_then(purchase::covenant_name_height);
        if registration_ended(&reply, cov_height)? {
            queries::expire_locked_listing(conn, &l.id)?;
            return Ok(());
        }
        // `registration_ended` took hsd's `info: null`.
        let Some(info) = name_info(&reply)? else {
            return Ok(());
        };
        let owner = owner_of(info)?;
        if !(owner.0.eq_ignore_ascii_case(cancel.0) && owner.1 == cancel.1) {
            // hsd names a coin the owner once its block is connected: a
            // mined cancel that is not the owner is no consistent answer.
            return Ok(());
        }
        let transfer = sell::transfer_height(info).ok_or_else(|| {
            AppError::Rpc("node did not report the block of the cancel's TRANSFER".into())
        })?;
        let tip = client.get_blockchain_info().await?.blocks;
        let left = network.name_params().blocks_until_finalize(transfer, tip);
        queries::set_cancel_blocks_remaining(conn, &l.id, left)?;
        if l.state == queries::ListingState::CancelFinalizing {
            if let Some(draft) = l.cancel_finalize_draft_id.as_deref() {
                if draft_dead(conn, Some(draft))? {
                    queries::revert_listing_cancel_finalize(conn, &l.id, draft)?;
                }
            }
        }
        return Ok(());
    }
    if let Some((lock_txid, lock_vout)) = stored_lock(l)? {
        if client.get_coin(lock_txid, lock_vout).await?.is_some() {
            queries::mark_listing_cancel_unmined(conn, &l.id, cancel.0)?;
            return Ok(());
        }
    }
    let reply = client.get_name_info(&l.name).await?;
    let Some(info) = name_info(&reply)? else {
        queries::expire_locked_listing(conn, &l.id)?;
        return Ok(());
    };
    let owner = owner_of(info)?;
    if let Some(coin) = client.get_coin(&owner.0, owner.1).await? {
        if let Some(height) = coin.mined_height()? {
            if let Some(tx) = spend_view(client, &owner.0, Some(height)).await? {
                let own = own_addresses(conn, &l.wallet_profile_id)?;
                if sell::cancel_finalized_home(&tx, owner.1, cancel, &at, &own) {
                    queries::mark_listing_cancelled(conn, &l.id, cancel)?;
                    return Ok(());
                }
            }
        }
    }
    let Some(lock) = stored_lock(l)? else {
        return Ok(());
    };
    cancel_replaced(conn, client, network, l, &owner, lock, cancel).await
}

/// R22, R28: hsd's 404 for both the mined cancel `cancel`'s TRANSFER and
/// the stored lock coin `lock`, the name's owner `owner` not a FINALIZE
/// home through `cancel`: a reorg may have replaced our cancel. The lock
/// coin's spender is judged as for a Listed listing ([`lock_coin_spent`]),
/// on mined facts tied to `lock` only: another mined cancel out of it
/// ([`cancel_of_lock`], an outpoint that is not `cancel`) →
/// CancelAwaitingFinalize with that outpoint ([`cancel_mined`]); otherwise
/// a mined purchase of it while the owner has moved ([`find_sale`];
/// [`sale_of_restored_lock`] for a lock restored by name) → Sold
/// ([`sell_releasing_cancel`]). Either way a cancel of ours that can no
/// longer land is released with that mined spender as evidence, even while
/// the draft tracker still calls it `confirmed`, once hsd, read again, does
/// not show our cancel mined ([`replaced_cancel_release`]). A spender in the mempool, no
/// spender found, or `cancel` itself found again is no verdict.
async fn cancel_replaced(
    conn: &rusqlite::Connection,
    client: &dyn NodeRpc,
    network: Network,
    l: &queries::ShakedexListing,
    owner: &(String, u32),
    lock: (&str, u32),
    cancel: (&str, u32),
) -> Result<(), AppError> {
    if let Some(c) = cancel_of_lock(conn, client, network, l, owner, lock).await? {
        if !(c.0.eq_ignore_ascii_case(cancel.0) && c.1 == cancel.1) {
            let release = replaced_cancel_release(conn, client, l).await?;
            cancel_mined(conn, l, (&c.0, c.1), lock, release)?;
        }
        return Ok(());
    }
    if l.payment_address.is_none() {
        if let Some(txid) = sale_of_restored_lock(conn, client, network, l, owner, lock).await? {
            let release = replaced_cancel_release(conn, client, l).await?;
            sell_releasing_cancel(conn, l, &txid, lock, release)?;
        }
        return Ok(());
    }
    let owner_is_lock = owner.0 == lock.0 && owner.1 == lock.1;
    if let (Sale::Mined { txid, lock }, false) = (
        find_sale(conn, client, network, l, owner).await?,
        owner_is_lock,
    ) {
        let release = replaced_cancel_release(conn, client, l).await?;
        let lock = (lock.0.as_str(), lock.1);
        sell_releasing_cancel(conn, l, &txid, lock, release)?;
    }
    Ok(())
}

/// How [`cancel_replaced`] releases our cancel draft. Only a `confirmed`
/// draft differs between the two releases, and it is released
/// ([`queries::release_replaced_cancel`]) only when hsd, read again now,
/// does not show the draft's own transaction mined: [`spend_view`] of its
/// txid (`GET /tx`, or without the index the block at its recorded
/// confirmation height) not found, or in the mempool. Shown mined (a reorg
/// between the job's reads put it back), it is kept
/// ([`queries::release_losing_cancel`]); a reply that is not hsd's whole
/// answer is an error, and nothing is written this sync.
async fn replaced_cancel_release(
    conn: &rusqlite::Connection,
    client: &dyn NodeRpc,
    l: &queries::ShakedexListing,
) -> Result<ReleaseCancel, AppError> {
    let draft = match l.cancel_draft_id.as_deref() {
        Some(id) => queries::get_tx_draft(conn, id)?,
        None => None,
    };
    let Some(d) = draft.filter(|d| d.status == queries::CONFIRMED_STATUS) else {
        return Ok(queries::release_replaced_cancel);
    };
    let Some(txid) = d.txid.as_deref() else {
        return Ok(queries::release_losing_cancel);
    };
    let mined = spend_view(client, txid, d.confirmation_height)
        .await?
        .is_some_and(|tx| tx.height.is_some());
    Ok(if mined {
        queries::release_losing_cancel
    } else {
        queries::release_replaced_cancel
    })
}

// ---------------------------------------------------------------------------
// The market jobs (T6): R23's publishing on LearnHNS Market, R25's keeping
// listed. Mainnet only; they read the node and write to the market and the
// listing's market bookkeeping only: never a key, a signature or a send.
// ---------------------------------------------------------------------------

/// R25: the market copy is checked about hourly.
pub const KEEP_LISTED_INTERVAL_SECS: i64 = 3_600;
/// R25: the first retry after a failure, doubled per failure up to
/// [`RETRY_MAX_SECS`].
pub const RETRY_BASE_SECS: i64 = 300;
pub const RETRY_MAX_SECS: i64 = 6 * 3_600;
/// R23: `expiresAt` is moved ahead once it is this close.
pub const EXPIRY_REFRESH_MARGIN_SECS: i64 = 30 * 86_400;

/// R25: the wait before try `attempts + 1` after `attempts` failures in a row.
pub(crate) fn retry_delay_secs(attempts: i64) -> i64 {
    let doublings = u32::try_from(attempts.saturating_sub(1).clamp(0, 16)).unwrap_or(16);
    RETRY_BASE_SECS
        .saturating_mul(1 << doublings)
        .min(RETRY_MAX_SECS)
}

/// `secs` (Unix) as `market_retry_at` stores it: RFC 3339, UTC, seconds.
pub(crate) fn rfc3339(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Whether a market action on `l` is due at `now`. A `market_retry_at` this
/// code did not write (unreadable) is due: retrying early only costs a
/// request, waiting forever loses the listing.
fn due(l: &queries::ShakedexListing, now: i64) -> bool {
    match l.market_retry_at.as_deref() {
        None => true,
        Some(at) => chrono::DateTime::parse_from_rfc3339(at)
            .map(|t| t.timestamp() <= now)
            .unwrap_or(true),
    }
}

/// What a market job writes after a reply ([`after_reply`]).
struct MarketResult {
    status: queries::MarketStatus,
    retry_at: Option<String>,
    attempts: i64,
    error: Option<String>,
}

/// What a market job writes after an answer, given the listing's attempts so
/// far: success → `ok` (Listed/ReplacedReuploaded due again in an hour,
/// Pending and Reported not due), the count back to 0; the market's own
/// refusal → Refused with its words and **no** `retry_at` (never retried
/// automatically: only a write that changes what is sent starts it over);
/// no answer, or the market's "not seen yet" (never a verdict on the
/// listing: its state is the after-lock job's, from our own node) →
/// Retrying with the reason, due again after [`retry_delay_secs`] (5 min
/// doubling to the 6 h cap).
fn after_reply<T>(
    reply: &MarketReply<T>,
    ok: queries::MarketStatus,
    attempts: i64,
    now: i64,
) -> MarketResult {
    use queries::MarketStatus as S;
    let retrying = |why: String| {
        let attempts = attempts.saturating_add(1);
        MarketResult {
            status: S::Retrying,
            retry_at: Some(rfc3339(now.saturating_add(retry_delay_secs(attempts)))),
            attempts,
            error: Some(why),
        }
    };
    match reply {
        MarketReply::Accepted(_) => MarketResult {
            status: ok,
            retry_at: matches!(ok, S::Listed | S::ReplacedReuploaded)
                .then(|| rfc3339(now.saturating_add(KEEP_LISTED_INTERVAL_SECS))),
            attempts: 0,
            error: None,
        },
        MarketReply::Refused { status, error } => MarketResult {
            status: S::Refused,
            retry_at: None,
            attempts,
            error: Some(format!("{error} (HTTP {status})")),
        },
        MarketReply::NotSeenYet { status, error } => retrying(format!(
            "the market has not seen it on chain yet: {error} (HTTP {status})"
        )),
        MarketReply::NoAnswer(why) => retrying(format!("no answer from the market: {why}")),
    }
}

/// Write `r` over `l`, the row the job read ([`queries::record_market_result`]:
/// nothing is written once the listing changed meanwhile).
fn record(
    conn: &rusqlite::Connection,
    l: &queries::ShakedexListing,
    r: &MarketResult,
) -> Result<(), AppError> {
    queries::record_market_result(
        conn,
        &l.id,
        &queries::MarketSeen {
            state: l.state,
            steps_json: &l.steps_json,
            listing_file_json: l.listing_file_json.as_deref(),
        },
        &queries::MarketUpdate {
            status: r.status,
            retry_at: r.retry_at.as_deref(),
            attempts: r.attempts,
            error: r.error.as_deref(),
        },
    )?;
    Ok(())
}

/// The market's view of a listing the jobs may upload (R23, R25): every
/// read from the node, none from the market.
pub(crate) enum MarketCopy {
    /// The one-step file to upload (the current step at the node's median
    /// time), and the listing as it now stands (after an expiry refresh).
    Ready {
        file: String,
        listing: Box<queries::ShakedexListing>,
    },
    /// A step of the stored file does not verify over the lock coin hsd
    /// reports, the file is not this listing's, or the row does not read
    /// (its steps, lock key, lock outpoint, file): nothing is uploaded.
    StepsUnverified(String),
    /// The node gives no verdict this sync (no median time, the lock coin
    /// spent (404) or not this listing's FINALIZE, unmined, no step valid
    /// yet): nothing is done, nothing written. A node reply that does not
    /// read (an error) is no verdict either: logged, nothing written.
    NotNow(String),
}

/// R23, R25, carried from T4: the copy of Listed `l` the market would get.
/// Reads hsd's median time and `GET /coin` of the stored lock outpoint; the
/// coin must be this listing's FINALIZE at its lock address
/// ([`sell::ListingLock::stored_coin`]), mined; the stored file must be this
/// listing's (name, lock outpoint, key, payment address, and the steps the
/// row stores) and every one of its steps must verify over that coin at the
/// value hsd reports ([`sell::lock_coin_value`],
/// [`sell::verify_file_steps`]). A Listed listing whose `expiresAt` is
/// within [`EXPIRY_REFRESH_MARGIN_SECS`] of `now` gets MTP +
/// [`sell::LISTING_LIFETIME_SECS`] first, only when that is later than the
/// stored one ([`queries::refresh_listing_expiry`]). Network-agnostic and
/// sends nothing: the live test runs it on regtest.
pub(crate) async fn market_copy(
    conn: &rusqlite::Connection,
    node: &dyn NodeRpc,
    network: Network,
    l: &queries::ShakedexListing,
    now: i64,
) -> Result<MarketCopy, AppError> {
    let not_now = |why: &str| Ok(MarketCopy::NotNow(why.into()));
    let unverified = |why: String| Ok(MarketCopy::StepsUnverified(why));
    // The row first: what it stores either reads or is recorded once as
    // StepsUnverified (a corrupted row is no node question, and logging it
    // every sync tells the user nothing).
    let row = match row_for_market(l, network) {
        Ok(row) => row,
        Err(e) => return unverified(e.to_string()),
    };
    let RowForMarket {
        lock,
        stored_file,
        at,
        file,
    } = row;
    let Some(mtp) = node.get_blockchain_info().await?.mediantime else {
        return not_now("the node reported no median time");
    };
    let Some(coin) = node.get_coin(lock.0, lock.1).await? else {
        return not_now("the node reports the lock coin spent");
    };
    if at.stored_coin(&coin, lock, COV_FINALIZE).is_err() {
        return not_now("the node reports something else than this listing's lock coin");
    }
    if coin.mined_height()?.is_none() {
        return not_now("the FINALIZE into the lock is not mined");
    }
    let value = sell::lock_coin_value(&coin, &file, &at.address)?;
    if let Err(e) = sell::verify_file_steps(&file, value, network) {
        return unverified(e.to_string());
    }
    let encoded = match file.encoded_steps() {
        Ok(e) => e,
        Err(e) => return unverified(e.to_string()),
    };
    let Some(index) = template::current_step_index(&encoded, mtp) else {
        return not_now("no price step is valid yet");
    };
    let refreshed = mtp.saturating_add(sell::LISTING_LIFETIME_SECS);
    let near_end = |exp: u64| {
        i64::try_from(exp).is_ok_and(|e| e.saturating_sub(now) <= EXPIRY_REFRESH_MARGIN_SECS)
    };
    let (file_json, listing) = match file.expires_at {
        Some(exp)
            if l.state == queries::ListingState::Listed && near_end(exp) && refreshed > exp =>
        {
            let json = listing_file::with_expiry(stored_file, refreshed, network)?;
            let expires_at = i64::try_from(refreshed)
                .map_err(|_| AppError::Other("listing expiry out of range".into()))?;
            let moved = queries::refresh_listing_expiry(
                conn,
                &l.id,
                &queries::RefreshedExpiry {
                    lock,
                    old_file: stored_file,
                    listing_file_json: &json,
                    expires_at,
                },
            )?;
            if moved == 0 {
                return not_now("the listing changed meanwhile");
            }
            let Some(listing) = queries::get_shakedex_listing(conn, &l.id)? else {
                return not_now("the listing is gone");
            };
            (json, listing)
        }
        _ => (stored_file.to_string(), l.clone()),
    };
    Ok(MarketCopy::Ready {
        file: listing_file::market_copy(&file_json, index, network)?,
        listing: Box::new(listing),
    })
}

/// What [`market_copy`] reads from the row alone, before asking the node.
struct RowForMarket<'a> {
    lock: (&'a str, u32),
    stored_file: &'a str,
    at: sell::ListingLock,
    file: listing_file::ListingFile,
}

/// Listing `l`'s stored lock outpoint, lock, and listing file, the file
/// this listing's own (name, lock outpoint, key, payment address, and the
/// steps the row stores). An `Err` is the row's, never the node's: the
/// caller records it as StepsUnverified.
fn row_for_market(
    l: &queries::ShakedexListing,
    network: Network,
) -> Result<RowForMarket<'_>, AppError> {
    let corrupted = |why: &str| AppError::Other(format!("corrupted listing {}: {why}", l.id));
    let lock = stored_lock(l)?.ok_or_else(|| corrupted("no lock coin stored"))?;
    let stored_file = l
        .listing_file_json
        .as_deref()
        .ok_or_else(|| corrupted("no listing file stored"))?;
    let at = listing_lock(network, l)?;
    let file = listing_file::ListingFile::parse(stored_file, network)
        .map_err(|e| corrupted(&format!("the stored listing file does not read: {e}")))?;
    let steps: Vec<(u64, u64, String)> = file
        .steps
        .iter()
        .map(|s| (s.price, s.lock_time, hex::encode(s.signature)))
        .collect();
    let stored: Vec<(u64, u64, String)> = stored_steps(l)?
        .into_iter()
        .map(|s| (s.price, s.lock_time, s.signature.to_ascii_lowercase()))
        .collect();
    if !(file.name == l.name
        && hex::encode(file.lock_txid).eq_ignore_ascii_case(lock.0)
        && file.lock_vout == lock.1
        && file.public_key == lock_pubkey(l)?
        && l.payment_address.as_deref() == Some(file.payment_addr.as_str())
        && steps == stored)
    {
        return Err(corrupted("the stored listing file is not this listing's"));
    }
    Ok(RowForMarket {
        lock,
        stored_file,
        at,
        file,
    })
}

/// R23 (T6): announce and publish the profile's published listings from a
/// sync. Mainnet only: returns before any read off mainnet, and the client
/// is told the profile's network, so it refuses every write off mainnet as
/// well (`LearnHnsClient::from_settings`, the one client of the app's and
/// the daemon's sync). Silent when the database, the profile or a client
/// cannot be opened; a failed run is logged.
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn publish_listings_step(db_path: &str, profile_id: &str) {
    let Some((conn, node, market)) = market_job_clients(db_path, profile_id) else {
        return;
    };
    let now = chrono::Utc::now().timestamp();
    if let Err(e) = publish_listings_with_client(&conn, &node, &market, profile_id, now).await {
        eprintln!("shakedex market: publish failed for {profile_id}: {e}");
    }
}

/// R25 (T6): keep the profile's published listings on the market from a
/// sync; the IO shell of [`keep_listed_with_client`], like
/// [`publish_listings_step`].
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn keep_listed_step(db_path: &str, profile_id: &str) {
    let Some((conn, node, market)) = market_job_clients(db_path, profile_id) else {
        return;
    };
    let now = chrono::Utc::now().timestamp();
    if let Err(e) = keep_listed_with_client(&conn, &node, &market, profile_id, now).await {
        eprintln!("shakedex market: keep listed failed for {profile_id}: {e}");
    }
}

/// The database, node client and market client of a market job, on a
/// mainnet profile only (R23).
#[cfg_attr(coverage_nightly, coverage(off))]
fn market_job_clients(
    db_path: &str,
    profile_id: &str,
) -> Option<(rusqlite::Connection, NodeRpcClient, LearnHnsClient)> {
    let conn = crate::db::connection::open_migrated(db_path).ok()?;
    let network = queries::profile_network(&conn, profile_id).ok()?;
    if network != Network::Main {
        return None;
    }
    let node = NodeRpcClient::for_profile(&conn, profile_id).ok()?;
    let settings = queries::get_settings(&conn).ok()?;
    let market = LearnHnsClient::from_settings(&settings)
        .ok()?
        .for_network(network);
    Some((conn, node, market))
}

/// R23 (T6): announce and publish the profile's published listings. Mainnet
/// only (returns before any read off mainnet). Day 0: each listing of
/// [`queries::list_listings_to_announce`] that is due gets its pending
/// listing (no node read). Once Listed (the FINALIZE into the lock mined,
/// one confirmation): each Listed Buy Now listing of
/// [`queries::list_listings_kept_on_market`] (published) the market has not taken yet
/// (`market_status` unset) gets its current step uploaded
/// ([`market_copy`]); reverse auctions are T8's. Reads the node, writes only
/// to the market and to the listing's market bookkeeping (and an expiry
/// refresh): never signs, never broadcasts (SECURITY.md). A failure on one
/// listing is logged and leaves it for the next sync.
pub async fn publish_listings_with_client(
    conn: &rusqlite::Connection,
    node: &dyn NodeRpc,
    market: &LearnHnsClient,
    profile_id: &str,
    now: i64,
) -> Result<(), AppError> {
    if queries::profile_network(conn, profile_id)? != Network::Main {
        return Ok(());
    }
    for l in queries::list_listings_to_announce(conn, profile_id)? {
        if due(&l, now) {
            if let Err(e) = announce(conn, market, &l, now).await {
                eprintln!("shakedex market: {} ({}): {e}", l.id, l.name);
            }
        }
    }
    for l in queries::list_listings_kept_on_market(conn, profile_id)? {
        // Not taken by the market yet: unset. A Listed row is never Pending
        // (every move to Listed starts the bookkeeping over,
        // `ListingWrite::market_reset_sql`, and the day-0 announce never
        // takes a Listed one).
        let first = l.market_status.is_none();
        // Only a Listed one: a Cancelling listing of that set (its cancel
        // not sent yet) is on its way off the market, not onto it.
        let listed = l.state == queries::ListingState::Listed;
        if first && listed && l.mode == queries::ListingMode::BuyNow && due(&l, now) {
            if let Err(e) = upload_current(conn, node, market, &l, now).await {
                eprintln!("shakedex market: {} ({}): {e}", l.id, l.name);
            }
        }
    }
    Ok(())
}

/// R23 day 0: post `l`'s pending listing — the lock TRANSFER's outpoint, the
/// lock address, the mode — and record the market's answer.
async fn announce(
    conn: &rusqlite::Connection,
    market: &LearnHnsClient,
    l: &queries::ShakedexListing,
    now: i64,
) -> Result<(), AppError> {
    let transfer_txid = l.lock_transfer_txid.as_deref().ok_or_else(|| {
        AppError::Other(format!("corrupted listing {}: no lock TRANSFER txid", l.id))
    })?;
    let lock = listing_lock(Network::Main, l)?;
    let reply = market
        .post_pending_listing(&PendingListing {
            name: &l.name,
            transfer_txid,
            transfer_vout: 0,
            lock_address: &lock.address,
            kind: match l.mode {
                queries::ListingMode::BuyNow => ListingKind::FixedPrice,
                queries::ListingMode::ReverseAuction => ListingKind::ReverseAuction,
            },
        })
        .await?;
    record(
        conn,
        l,
        &after_reply(
            &reply,
            queries::MarketStatus::Pending,
            l.market_attempts,
            now,
        ),
    )
}

/// R23: upload `l`'s current step ([`market_copy`], every stored step
/// verified on our node first) and record the market's answer.
async fn upload_current(
    conn: &rusqlite::Connection,
    node: &dyn NodeRpc,
    market: &LearnHnsClient,
    l: &queries::ShakedexListing,
    now: i64,
) -> Result<(), AppError> {
    match market_copy(conn, node, Network::Main, l, now).await? {
        MarketCopy::NotNow(why) => {
            not_now(l, &why);
            Ok(())
        }
        MarketCopy::StepsUnverified(why) => record(conn, l, &steps_unverified(l, why, now)),
        MarketCopy::Ready { file, listing } => {
            upload(
                conn,
                market,
                &file,
                &listing,
                queries::MarketStatus::Listed,
                now,
            )
            .await
        }
    }
}

/// A [`MarketCopy::NotNow`]: no verdict this sync, nothing written.
fn not_now(l: &queries::ShakedexListing, why: &str) {
    eprintln!(
        "shakedex market: {} ({}): not uploaded now: {why}",
        l.id, l.name
    );
}

/// What a [`MarketCopy::StepsUnverified`] writes: nothing is uploaded, and
/// the steps are verified again on our node after the backoff.
fn steps_unverified(l: &queries::ShakedexListing, why: String, now: i64) -> MarketResult {
    let attempts = l.market_attempts.saturating_add(1);
    MarketResult {
        status: queries::MarketStatus::StepsUnverified,
        retry_at: Some(rfc3339(now.saturating_add(retry_delay_secs(attempts)))),
        attempts,
        error: Some(why),
    }
}

/// Upload `file` (`listing`'s [`MarketCopy::Ready`] copy) and record the
/// market's answer, `ok` on acceptance. An acceptance naming another name
/// than ours is no answer about ours.
async fn upload(
    conn: &rusqlite::Connection,
    market: &LearnHnsClient,
    file: &str,
    listing: &queries::ShakedexListing,
    ok: queries::MarketStatus,
    now: i64,
) -> Result<(), AppError> {
    let reply = match market.upload_proof(file).await? {
        MarketReply::Accepted(a) if a.name != listing.name => MarketReply::NoAnswer(format!(
            "the market answered for '{}', not '{}'",
            a.name, listing.name
        )),
        other => other,
    };
    record(
        conn,
        listing,
        &after_reply(&reply, ok, listing.market_attempts, now),
    )
}

/// R25 (T6): about hourly, keep the profile's published Buy Now listings on
/// LearnHNS Market. Mainnet only: returns before any read off mainnet.
/// Takes the listings of [`queries::list_listings_kept_on_market`] — Listed,
/// and Cancelling while the cancel is not sent (still buyable on chain;
/// R24, R28: the jobs stop once it is sent) — that the market has taken or
/// failed to take for want of an answer or of verified steps
/// (`market_status` Listed, ReplacedReuploaded, Retrying, StepsUnverified)
/// and that are due ([`due`]). The first upload is Listed only and
/// [`publish_listings_with_client`]'s. Never a Refused one: it waits for a
/// write that changes what is sent; only a Refused Listed listing's expiry
/// is looked at, without a market call, and only once the stored
/// `expires_at` is within [`EXPIRY_REFRESH_MARGIN_SECS`]: the refresh
/// ([`market_copy`]) starts its bookkeeping over, and the next run's first
/// upload sends the new file. The expiry is refreshed for Listed listings
/// only ([`market_copy`], `ListingWrite::RefreshExpiry` is Listed to
/// Listed). Reverse auctions are T8's.
///
/// Per listing: [`market_copy`] first (our node: every step verified
/// again, the expiry refreshed when near). StepsUnverified → recorded,
/// backed off, nothing asked of the market. A refresh just started the
/// bookkeeping over → uploaded at once (Listed). Otherwise the market's
/// copy (`GET /listing/<name>/proof.json`): the same offer
/// ([`listing_file::same_market_listing`]) → nothing uploaded, checked again
/// in an hour (Retrying/StepsUnverified → Listed, ReplacedReuploaded
/// stays); a copy that does not read as a listing file, or another offer →
/// ours uploaded over it (ReplacedReuploaded); the market's own "not
/// listed" → uploaded (Listed); no answer → Retrying, backed off
/// ([`retry_delay_secs`]), nothing uploaded on it. The upload's answer is
/// recorded by [`after_reply`] (a refusal → Refused, not retried). Reads the
/// node, writes only to the market and the listing's market bookkeeping
/// (and an expiry refresh): never signs, never broadcasts (SECURITY.md). A
/// failure on one listing is logged and leaves it for the next sync.
pub async fn keep_listed_with_client(
    conn: &rusqlite::Connection,
    node: &dyn NodeRpc,
    market: &LearnHnsClient,
    profile_id: &str,
    now: i64,
) -> Result<(), AppError> {
    use queries::MarketStatus as S;
    if queries::profile_network(conn, profile_id)? != Network::Main {
        return Ok(());
    }
    for l in queries::list_listings_kept_on_market(conn, profile_id)? {
        if l.mode != queries::ListingMode::BuyNow {
            continue;
        }
        let listed = l.state == queries::ListingState::Listed;
        let run = match l.market_status {
            Some(S::Listed | S::ReplacedReuploaded | S::Retrying | S::StepsUnverified) => {
                if !due(&l, now) {
                    continue;
                }
                keep_listed(conn, node, market, &l, now).await
            }
            Some(S::Refused) if listed && expiry_near(&l, now) => {
                refresh_refused(conn, node, &l, now).await
            }
            _ => continue,
        };
        if let Err(e) = run {
            eprintln!("shakedex market: {} ({}): {e}", l.id, l.name);
        }
    }
    Ok(())
}

/// Whether Listed `l`'s stored `expires_at` is within
/// [`EXPIRY_REFRESH_MARGIN_SECS`] of `now` (or not stored: [`market_copy`]
/// reads the file's own). The refresh rule itself is [`market_copy`]'s.
fn expiry_near(l: &queries::ShakedexListing, now: i64) -> bool {
    l.expires_at
        .is_none_or(|e| e.saturating_sub(now) <= EXPIRY_REFRESH_MARGIN_SECS)
}

/// R23 for a Refused Listed listing: [`market_copy`]'s expiry refresh, and
/// nothing else — no market call, and nothing recorded (a refusal stays
/// until what is sent changes; the refresh is such a change and starts the
/// bookkeeping over itself).
async fn refresh_refused(
    conn: &rusqlite::Connection,
    node: &dyn NodeRpc,
    l: &queries::ShakedexListing,
    now: i64,
) -> Result<(), AppError> {
    match market_copy(conn, node, Network::Main, l, now).await? {
        MarketCopy::Ready { .. } => {}
        MarketCopy::NotNow(why) | MarketCopy::StepsUnverified(why) => not_now(l, &why),
    }
    Ok(())
}

/// R25: one due check of Listed `l` (see [`keep_listed_with_client`]).
async fn keep_listed(
    conn: &rusqlite::Connection,
    node: &dyn NodeRpc,
    market: &LearnHnsClient,
    l: &queries::ShakedexListing,
    now: i64,
) -> Result<(), AppError> {
    use queries::MarketStatus as S;
    let (file, listing) = match market_copy(conn, node, Network::Main, l, now).await? {
        MarketCopy::NotNow(why) => {
            not_now(l, &why);
            return Ok(());
        }
        MarketCopy::StepsUnverified(why) => {
            return record(conn, l, &steps_unverified(l, why, now));
        }
        MarketCopy::Ready { file, listing } => (file, listing),
    };
    if listing.market_status.is_none() {
        // The expiry was just refreshed: the market holds the old file.
        return upload(conn, market, &file, &listing, S::Listed, now).await;
    }
    match market.proof_copy(&listing.name).await? {
        ProofCopy::Copy(text) => {
            let ours = listing_file::ListingFile::parse(&file, Network::Main)?;
            let same = listing_file::ListingFile::parse(&text, Network::Main)
                .is_ok_and(|theirs| listing_file::same_market_listing(&theirs, &ours));
            if same {
                let ok = match listing.market_status {
                    Some(S::ReplacedReuploaded) => S::ReplacedReuploaded,
                    _ => S::Listed,
                };
                let matched: MarketReply<()> = MarketReply::Accepted(());
                record(
                    conn,
                    &listing,
                    &after_reply(&matched, ok, listing.market_attempts, now),
                )
            } else {
                upload(conn, market, &file, &listing, S::ReplacedReuploaded, now).await
            }
        }
        ProofCopy::NotListed => upload(conn, market, &file, &listing, S::Listed, now).await,
        ProofCopy::NoAnswer(why) => {
            let reply: MarketReply<()> = MarketReply::NoAnswer(why);
            record(
                conn,
                &listing,
                &after_reply(&reply, S::Listed, listing.market_attempts, now),
            )
        }
    }
}
