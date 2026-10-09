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
//! Aborted ([`refresh_listings_before_lock_with_client`]). The after-lock job
//! follows each listing whose FINALIZE into the lock is built: Listed once it
//! is mined, Finalizing again on a reorg, ReadyToFinalize again if it never
//! landed, settled as before the lock if it never landed and something else
//! spent the lock TRANSFER, and, once the lock coin is spent, SalePending or
//! Sold when a purchase of it is found on chain (R22: a TRANSFER out of our
//! lock committing to an address not ours, in a transaction that pays the
//! listing's payment address) ([`refresh_listings_after_lock_with_client`]).
//! Both only read the node, and never take the same listing.

use std::collections::HashSet;

use crate::db::queries::{self, PurchaseProgress, PurchaseState, ShakedexPurchase, TxDraftRow};
use crate::error::AppError;
use crate::noncustodial::network::Network;
use crate::noncustodial::node_rpc::NodeRpc;
use crate::noncustodial::rpc::{self, NodeRpcClient};
use crate::noncustodial::send::RESERVATION_TTL_SECS;
use crate::noncustodial::shakedex::purchase::{self, transfer_commits_to};
use crate::noncustodial::shakedex::script;
use crate::noncustodial::shakedex::sell;
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
    /// existed or has expired.
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
    /// or in the mempool, or never mined).
    Gone { owner: (String, u32), revoked: bool },
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
        // says otherwise is not consistent, so no verdict.
        match coin.mined_height()? {
            None => Ok(LockOnChain::Pending),
            Some(_) => Err(AppError::Rpc(format!(
                "node reports lock TRANSFER {lock_transfer_txid}:0 mined but not the name's owner"
            ))),
        }
    } else {
        Ok(LockOnChain::Gone {
            owner: (hash.to_string(), index),
            revoked: revoked != 0,
        })
    }
}

/// Where a name's owner coin sits, from hsd's `GET /coin`.
#[derive(Debug, PartialEq, Eq)]
enum InOurLock {
    /// A FINALIZE at the listing's lock address: the name is in our lock.
    Finalize,
    /// At the listing's lock address, but another covenant (a purchase's
    /// TRANSFER or a cancel already): the name went through our lock.
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
/// (as a FINALIZE, or another covenant) or elsewhere; hsd's 404 is
/// [`InOurLock::SpentInMempool`]. Only the coin's `address` and
/// `covenant.type` are read; a coin without address or covenant is not
/// hsd's whole answer: an error.
async fn owner_in_our_lock(
    client: &dyn NodeRpc,
    network: Network,
    l: &queries::ShakedexListing,
    owner: &(String, u32),
) -> Result<InOurLock, AppError> {
    let Some(coin) = client.get_coin(&owner.0, owner.1).await? else {
        return Ok(InOurLock::SpentInMempool);
    };
    let (Some(address), Some(covenant)) = (coin.address.as_deref(), coin.covenant.as_ref()) else {
        return Err(AppError::Rpc(format!(
            "node did not report the address or covenant of coin {}:{}",
            owner.0, owner.1
        )));
    };
    if address != listing_lock_address(network, l)? {
        Ok(InOurLock::No)
    } else if covenant.kind == COV_FINALIZE {
        Ok(InOurLock::Finalize)
    } else {
        Ok(InOurLock::Other)
    }
}

/// The lock address of listing `l`, from its stored lock public key.
fn listing_lock_address(
    network: Network,
    l: &queries::ShakedexListing,
) -> Result<String, AppError> {
    let pubkey: [u8; 33] = hex::decode(&l.lock_pubkey_hex)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| AppError::Other(format!("corrupted listing {}: bad lock key", l.id)))?;
    script::lock_address(network, &pubkey)
}

/// R19, for each listing still Locking or ReadyToFinalize, or Aborted within
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
///   outpoint, never Aborted; another covenant there → unchanged (T4's);
///   the owner coin hsd's 404 (spent in the mempool) → unchanged until that
///   spend is mined;
/// - the lock TRANSFER the owner and `blocks_until_finalize` of hsd's
///   `info.transfer` 0 at the tip → ReadyToFinalize, not 0 → Locking; the
///   lock TRANSFER a coin in the mempool (`height: -1`), not the owner →
///   Locking; a lock TRANSFER mined in a block but not the owner is not a
///   consistent answer → unchanged.
///
/// Locking again is refused while another listing of the name is open
/// ([`queries::unabort_shakedex_listing`]). Anything the node does not
/// answer leaves the listing as it is. Sends nothing. A failure on one
/// listing is logged and leaves it for the next sync.
pub async fn refresh_listings_before_lock_with_client(
    conn: &rusqlite::Connection,
    client: &dyn NodeRpc,
    profile_id: &str,
) -> Result<(), AppError> {
    let listings =
        queries::list_shakedex_listings_before_lock(conn, profile_id, ABORT_RECHECK_DAYS)?;
    if listings.is_empty() {
        return Ok(());
    }
    let network = queries::profile_network(conn, profile_id)?;
    for l in listings {
        if let Err(e) = refresh_before_lock(conn, client, network, &l).await {
            eprintln!("shakedex listings: {} ({}): {e}", l.id, l.name);
        }
    }
    Ok(())
}

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
/// - hsd reports no live name → Expired;
/// - the name revoked, or its owner coin readable elsewhere while the lock
///   draft can no longer land → Aborted;
/// - the owner coin a FINALIZE at this listing's lock address → Restored
///   with that outpoint, never Aborted (coordinator (b): only that positive
///   evidence says the name is in our lock, whoever sent the FINALIZE);
/// - the owner coin at our lock under another covenant (T4 decides), or
///   hsd's 404 for it (spent in the mempool) → unchanged.
async fn settle_left_lock(
    conn: &rusqlite::Connection,
    client: &dyn NodeRpc,
    network: Network,
    l: &queries::ShakedexListing,
    lock: LockOnChain,
) -> Result<(), AppError> {
    let (owner, revoked) = match lock {
        LockOnChain::NoName => {
            queries::expire_shakedex_listing(conn, &l.id)?;
            return Ok(());
        }
        LockOnChain::Gone { owner, revoked } => (owner, revoked),
        LockOnChain::Owner { .. } | LockOnChain::Pending => return Ok(()),
    };
    if !revoked {
        match owner_in_our_lock(client, network, l, &owner).await? {
            InOurLock::Finalize => {
                queries::adopt_lock_finalized_elsewhere(conn, &l.id, &owner.0, owner.1)?;
                return Ok(());
            }
            InOurLock::Other | InOurLock::SpentInMempool => return Ok(()),
            InOurLock::No => {}
        }
    }
    let status = queries::lock_draft_status(conn, l)?;
    if !status.is_some_and(|s| queries::draft_may_still_land(&s)) {
        queries::abort_shakedex_listing(conn, &l.id)?;
    }
    Ok(())
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
/// ([`queries::ListingState::BEFORE_LOCK_JOB`] for
/// [`refresh_listings_before_lock_with_client`]'s rules,
/// [`queries::ListingState::AFTER_LOCK_JOB`] for
/// [`refresh_listings_after_lock_with_client`]'s), both sets read first: a
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

/// R19 and R22, for every Finalizing, Listed, SalePending and Restored
/// listing, and every Sold one within [`SOLD_RECHECK_DAYS`], from hsd's
/// `GET /coin` of its lock outpoint `(lock_txid, lock_vout)`:
///
/// - a FINALIZE at the listing's lock address mined in a block → a
///   Finalizing listing Listed; in the mempool (`height: -1`) → a Listed one
///   Finalizing (a reorg took it back); a coin at all → a SalePending or
///   Sold listing Listed (Restored without a listing file), its purchase
///   forgotten, unless another listing of the name is open by then; mined,
///   with the name's live state gone or its height not the lock coin's → a
///   Listed or Restored listing Expired ([`registration_ended`]);
/// - hsd's 404 for the lock coin while the lock TRANSFER
///   `(lock_transfer_txid, 0)` is a coin again (the FINALIZE into the lock
///   in no block and no mempool): a Finalizing listing whose FINALIZE draft
///   is `failed`, `dropped` or gone ([`finalize_dead`]) → ReadyToFinalize,
///   its lock outpoint, steps and file dropped (they were signed over a
///   coin that does not exist); a Listed one, and a SalePending or Sold one
///   with our FINALIZE draft → Finalizing; a Restored one → Locking without
///   its outpoint ([`queries::unadopt_restored_lock`]);
/// - hsd's 404 for both, for a Finalizing listing whose FINALIZE is dead:
///   something else spent the lock TRANSFER → settled from the name as
///   before the lock ([`settle_left_lock`]: Expired, Aborted, Restored, or
///   unchanged);
/// - any other 404 (the lock coin spent in a block or in the mempool) is no
///   verdict alone: the name's owner is read; no live name → a Listed,
///   SalePending or Restored listing Expired; otherwise a purchase is looked
///   for ([`find_sale`]); one in the mempool while the owner is still the
///   lock coin → SalePending, one mined while the owner has moved → Sold;
/// - a coin at another address or of another covenant, a reply missing the
///   coin's address, covenant or height, a name reply missing `info` or the
///   owner, or a read error → unchanged.
///
/// Sends nothing. A failure on one listing is logged and leaves it for the
/// next sync.
pub async fn refresh_listings_after_lock_with_client(
    conn: &rusqlite::Connection,
    client: &dyn NodeRpc,
    profile_id: &str,
) -> Result<(), AppError> {
    let listings = queries::list_shakedex_listings_after_lock(conn, profile_id, SOLD_RECHECK_DAYS)?;
    if listings.is_empty() {
        return Ok(());
    }
    let network = queries::profile_network(conn, profile_id)?;
    for l in listings {
        if let Err(e) = refresh_after_lock(conn, client, network, &l).await {
            eprintln!("shakedex listings: {} ({}): {e}", l.id, l.name);
        }
    }
    Ok(())
}

async fn refresh_after_lock(
    conn: &rusqlite::Connection,
    client: &dyn NodeRpc,
    network: Network,
    l: &queries::ShakedexListing,
) -> Result<(), AppError> {
    let (Some(lock_txid), Some(lock_vout)) = (l.lock_txid.as_deref(), l.lock_vout) else {
        return Ok(());
    };
    let lock_vout = u32::try_from(lock_vout)
        .map_err(|_| AppError::Other(format!("corrupted listing {}: bad lock output", l.id)))?;
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
    let (Some(address), Some(covenant)) = (coin.address.as_deref(), coin.covenant.as_ref()) else {
        return Err(AppError::Rpc(format!(
            "node did not report the address or covenant of lock coin {}:{}",
            lock.0, lock.1
        )));
    };
    if address != listing_lock_address(network, l)? || covenant.kind != COV_FINALIZE {
        return Err(AppError::Other(format!(
            "lock coin {}:{} is not a FINALIZE at the listing's lock address",
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
        (queries::ListingState::SalePending | queries::ListingState::Sold, _) => {
            let to = if l.listing_file_json.is_some() {
                queries::ListingState::Listed
            } else {
                queries::ListingState::Restored
            };
            if queries::unsell_shakedex_listing(conn, &l.id, to)? == 0 {
                eprintln!(
                    "shakedex listings: {} ({}): its purchase is no longer on chain, but it \
                     stays sold: another listing of the name is open",
                    l.id, l.name
                );
            }
        }
        (queries::ListingState::Listed | queries::ListingState::Restored, Some(_)) => {
            let reply = client.get_name_info(&l.name).await?;
            if registration_ended(&reply, purchase::covenant_name_height(covenant))? {
                queries::expire_locked_listing(conn, &l.id)?;
            }
        }
        _ => {}
    }
    Ok(())
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
    // no mempool of this node (plan deviation 4).
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
                queries::ListingState::SalePending | queries::ListingState::Sold
                    if l.lock_finalize_draft_id.is_some() =>
                {
                    let to = queries::ListingState::Finalizing;
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
    let owner_is_lock = owner.0.eq_ignore_ascii_case(lock.0) && owner.1 == lock.1;
    // hsd moves the owner only when a block is connected: a purchase is
    // Pending while the owner is still the lock coin, and Mined only once it
    // is not. Two facts that disagree are no verdict.
    match (
        find_sale(conn, client, network, l, &owner).await?,
        owner_is_lock,
    ) {
        (Sale::Pending { txid, lock }, true) => {
            queries::mark_listing_sale_pending(
                conn,
                &l.id,
                &txid,
                Some((lock.0.as_str(), lock.1)),
            )?;
        }
        (Sale::Mined { txid, lock }, false) => {
            queries::sell_shakedex_listing(conn, &l.id, &txid, Some((lock.0.as_str(), lock.1)))?;
        }
        _ => {}
    }
    Ok(())
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
///   address of ours it is our cancel → no verdict (T5); in the mempool it
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
    let Some(payment) = l.payment_address.as_deref() else {
        return Ok(Sale::None);
    };
    let (Some(lock_txid), Some(lock_vout)) = (l.lock_txid.as_deref(), l.lock_vout) else {
        return Ok(Sale::None);
    };
    let lock_vout = u32::try_from(lock_vout)
        .map_err(|_| AppError::Other(format!("corrupted listing {}: bad lock output", l.id)))?;
    let profile = &l.wallet_profile_id;
    let lock_address = listing_lock_address(network, l)?;
    let name_hash = hex::encode(crate::noncustodial::names::hash_name(&l.name)?);
    let own: HashSet<String> = queries::get_profile_addresses(conn, profile)?
        .into_iter()
        .collect();
    let p = sell::PurchaseOf {
        network,
        lock: Some((lock_txid, lock_vout)),
        lock_address: &lock_address,
        name_hash: &name_hash,
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
            let (Some(address), Some(cov)) = (coin.address.as_deref(), coin.covenant.as_ref())
            else {
                return Err(AppError::Rpc(format!(
                    "node did not report the address or covenant of coin {}:{}",
                    owner.0, owner.1
                )));
            };
            if address == lock_address
                && cov.kind == COV_TRANSFER
                && cov
                    .items
                    .first()
                    .is_some_and(|h| h.eq_ignore_ascii_case(&name_hash))
            {
                if sell::commitment_is_ours(&cov.items, network, &own)? {
                    return Ok(Sale::None);
                }
                // hsd names a coin the owner only once its block is
                // connected: an owner coin shown in the mempool is not a
                // mined purchase, so no verdict.
                let Some(height) = coin.mined_height()? else {
                    return Ok(Sale::None);
                };
                match purchase_found(client, &owner.0, Some(height), &p).await {
                    Ok(Some((Some(_), spent))) => {
                        return Ok(Sale::Mined {
                            txid: owner.0.clone(),
                            lock: spent,
                        });
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
        match purchase_found(client, &txid, seen_at, &p).await {
            Ok(Some((Some(_), spent))) => return Ok(Sale::Mined { txid, lock: spent }),
            Ok(Some((None, spent))) => return Ok(Sale::Pending { txid, lock: spent }),
            Ok(None) => {}
            Err(e) => unreadable(&txid, e),
        }
    }
    Ok(Sale::None)
}

/// Whether a Finalizing listing's FINALIZE draft is dead: it can no longer
/// be mined (`failed`, `dropped`, or its row gone; not
/// [`queries::draft_alive`]). The SQL writes ask the same of the database
/// ([`queries::abort_shakedex_listing`]).
fn finalize_dead(
    conn: &rusqlite::Connection,
    l: &queries::ShakedexListing,
) -> Result<bool, AppError> {
    let status = match l.lock_finalize_draft_id.as_deref() {
        Some(id) => queries::get_tx_draft(conn, id)?.map(|d| d.status),
        None => None,
    };
    Ok(!status.is_some_and(|s| queries::draft_alive(&s)))
}
