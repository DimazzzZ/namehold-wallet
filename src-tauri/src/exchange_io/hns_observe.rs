//! The HNS lock as the profile's node shows it (spec §3.6): every
//! transaction touching the lock's address, decoded from its own bytes (the
//! txid is computed here, outputs and witnesses are read from the
//! transaction, not from the node's JSON summary).
//!
//! The node must have the address and transaction indexes: without them hsd
//! answers `GET /tx/address` with an empty list, which would read as "the
//! Seller never locked". [`observe`] refuses such a node.

use crate::error::AppError;
use crate::noncustodial::exchange::engine::{HnsLockSeen, HnsSeen, LockRef, SpendSeen};
use crate::noncustodial::exchange::hns_htlc;
use crate::noncustodial::exchange::terms::Terms;
use crate::noncustodial::node_rpc::NodeRpc;
use crate::noncustodial::tx::Transaction;

/// One transaction as the node lists it: its bytes and its height (`None`
/// in the mempool).
pub struct NodeTx {
    pub tx: Transaction,
    pub height: Option<i64>,
}

/// Read a node's `GET /tx/address` entry. Fails closed: an entry without
/// `hex` or with a height that is not a number is an error, never skipped.
pub fn node_tx(v: &serde_json::Value) -> Result<NodeTx, AppError> {
    let Some(hex_tx) = v.get("hex").and_then(|h| h.as_str()) else {
        return Err(AppError::Rpc("a node transaction has no hex".into()));
    };
    let raw = hex::decode(hex_tx)
        .map_err(|_| AppError::Rpc("a node transaction's hex is malformed".into()))?;
    let tx = Transaction::decode(&raw)?;
    let Some(h) = v.get("height").and_then(|h| h.as_i64()) else {
        return Err(AppError::Rpc("a node transaction has no height".into()));
    };
    Ok(NodeTx {
        tx,
        height: (h >= 0).then_some(h),
    })
}

fn confirmations(height: Option<i64>, tip: i64) -> u32 {
    match height {
        Some(h) if h <= tip => u32::try_from(tip - h + 1).unwrap_or(u32::MAX),
        _ => 0,
    }
}

/// A coinbase transaction: its first input spends nothing (hsd
/// `TX.isCoinbase`).
fn is_coinbase(tx: &Transaction) -> bool {
    tx.inputs
        .first()
        .is_some_and(|i| i.prevout.hash == [0u8; 32] && i.prevout.index == u32::MAX)
}

/// The HNS view of `terms` from the transactions at the lock's address.
/// `own` is the Seller's own lock coin: when given, only that coin is the
/// lock (spec 2026-11-07, review C1: anyone can pay the lock address, and a
/// decoy coin mined earlier must not blind the Seller to the claim of its
/// own). Whatever coin it watches, a claim of **any** coin with the lock's
/// script reveals the secret, and `revealed` carries it.
///
/// `strict`: only `own` is the lock (the Seller's view). Otherwise `own` is
/// only preferred (the Buyer's view of the Seller's announced coin), and the
/// earliest confirmed coin is taken when it is not found.
///
/// An output of a coinbase transaction is never the lock, in either view:
/// hsd refuses to spend it until it matures (100 blocks on mainnet, longer
/// than the HNS refund time) and still sends the refused claim to its peers
/// (`FullNode.sendTX`, "Attempting to broadcast anyway"), secret included.
/// The Seller could then take the BTC with the secret and refund the HNS
/// the moment the coin matures. It counts as a mismatched payment.
pub fn seen_from(
    terms: &Terms,
    txs: &[NodeTx],
    tip: i64,
    mtp: u64,
    own: Option<&LockRef>,
    strict: bool,
) -> Result<HnsSeen, AppError> {
    let hp = terms.hns_params()?;
    let program = hns_htlc::program(&hp.script());
    let mut mismatched = false;
    // (height or MAX for mempool, txid, vout, confirmations)
    let mut candidates: Vec<(i64, String, u32, u32)> = Vec::new();
    for nt in txs {
        let txid = nt.tx.txid();
        let mined = is_coinbase(&nt.tx);
        for (vout, out) in nt.tx.outputs.iter().enumerate() {
            if out.address.version != 0 || out.address.hash != program {
                continue;
            }
            if !mined && hns_htlc::is_lock_output(out, &hp, terms.hns_amount) {
                candidates.push((
                    nt.height.unwrap_or(i64::MAX),
                    txid.clone(),
                    vout as u32,
                    confirmations(nt.height, tip),
                ));
            } else {
                mismatched = true;
            }
        }
    }
    // A secret revealed by a claim of any coin with the lock's script.
    let revealed = txs.iter().find_map(|nt| {
        (0..nt.tx.inputs.len()).find_map(|i| hns_htlc::secret_from_spend(&nt.tx, i, &hp))
    });
    // The Seller's own coin, or else the earliest confirmed one: for the
    // Buyer any coin of exactly the lock's script and value is claimable.
    candidates.sort();
    let preferred = own.and_then(|o| {
        candidates
            .iter()
            .find(|(_, t, v, _)| *t == o.txid && *v == o.vout)
            .cloned()
    });
    let chosen = match preferred {
        Some(c) => Some(c),
        None if strict && own.is_some() => None,
        None => candidates.into_iter().next(),
    };
    let Some((_, txid, vout, confs)) = chosen else {
        return Ok(HnsSeen {
            mtp,
            lock: None,
            mismatched_payment: mismatched,
            revealed,
        });
    };
    let mut hash = [0u8; 32];
    hex::decode_to_slice(&txid, &mut hash).map_err(|_| AppError::Rpc("bad txid".into()))?;
    // Spends of that coin; a confirmed one wins over a mempool one.
    let mut spend: Option<SpendSeen> = None;
    for nt in txs {
        for (i, input) in nt.tx.inputs.iter().enumerate() {
            if input.prevout.hash != hash || input.prevout.index != vout {
                continue;
            }
            let s = SpendSeen {
                txid: nt.tx.txid(),
                confirmations: confirmations(nt.height, tip),
                secret: hns_htlc::secret_from_spend(&nt.tx, i, &hp),
            };
            let better = match &spend {
                None => true,
                Some(old) => s.confirmations > old.confirmations,
            };
            if better {
                spend = Some(s);
            }
        }
    }
    Ok(HnsSeen {
        mtp,
        lock: Some(HnsLockSeen {
            outpoint: LockRef { txid, vout },
            confirmations: confs,
            spend,
        }),
        mismatched_payment: mismatched,
        revealed,
    })
}

/// Why the exchange refuses a node that cannot list a lock's transactions.
pub const NEEDS_INDEXED_NODE: &str =
    "the exchange needs a node with the address and transaction indexes (hsd --index-address --index-tx)";

/// The HNS view of `terms` from `node`. An `Err` is "could not check":
/// the engine then decides nothing on the HNS chain.
pub async fn observe(
    node: &dyn NodeRpc,
    terms: &Terms,
    own: Option<&LockRef>,
    strict: bool,
) -> Result<HnsSeen, AppError> {
    let idx = crate::noncustodial::node_rpc::node_indexes(node).await?;
    if !idx.tx || !idx.address {
        return Err(AppError::InvalidInput(NEEDS_INDEXED_NODE.into()));
    }
    let info = node.get_blockchain_info().await?;
    let Some(mtp) = info.mediantime else {
        return Err(AppError::Rpc(
            "the node did not report its median time".into(),
        ));
    };
    let network = terms.network()?;
    let addr = hns_htlc::lock_address(network, &terms.hns_params()?)?;
    let raw = node.get_txs_by_address(&addr).await?;
    let txs = raw.iter().map(node_tx).collect::<Result<Vec<_>, _>>()?;
    seen_from(terms, &txs, info.blocks, mtp, own, strict)
}
