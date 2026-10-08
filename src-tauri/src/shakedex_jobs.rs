//! Background jobs for Shakedex purchases (R13).
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

use crate::db::queries::{self, PurchaseProgress, PurchaseState, ShakedexPurchase, TxDraftRow};
use crate::error::AppError;
use crate::noncustodial::network::Network;
use crate::noncustodial::node_rpc::NodeRpc;
use crate::noncustodial::rpc::{self, NodeRpcClient};
use crate::noncustodial::send::RESERVATION_TTL_SECS;
use crate::noncustodial::shakedex::purchase::{self, transfer_commits_to};
use crate::noncustodial::shakedex::verify;
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
