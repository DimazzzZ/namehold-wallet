//! The BTC lock as the configured Bitcoin sources show it (spec §3.6, R9).
//!
//! Confirmations are counted only as far as they are proven: a Merkle proof
//! into a header with valid proof of work and the following headers linked
//! onto it (`btc_spv::verify`), and every source must name the same block
//! for the lock. Anything less is 0 confirmations, so the Seller never
//! commits on a source's word. A source that cannot be read makes the whole
//! view an error ("could not check").
//!
//! A spend of the lock is classified by what the party knows: its own
//! presigned spends' txids, or the spender's witness when the source names
//! the spender. There are only two paths, so a spend that is not ours is
//! the other party's.

use bitcoin::{OutPoint, Transaction, Txid};

use crate::error::AppError;
use crate::exchange_io::btc_source::{BtcSource, Outspend};
use crate::noncustodial::exchange::btc_htlc;
use crate::noncustodial::exchange::btc_spv::{self, InclusionProof};
use crate::noncustodial::exchange::engine::{BtcLockSeen, BtcSeen, Holdings, SpendSeen};
use crate::noncustodial::exchange::policy::Policy;
use crate::noncustodial::exchange::script;
use crate::noncustodial::network::Network;

/// The txids of `h`'s own presigned spends of the BTC lock (the witness is
/// not part of a txid, so a claim's txid is known before its secret).
pub fn own_spend_txids(h: &Holdings) -> Vec<Txid> {
    h.btc_refunds
        .iter()
        .chain(h.btc_claims.iter())
        .filter_map(|p| btc_htlc::decode(&p.tx_hex).ok())
        .map(|t| t.compute_txid())
        .collect()
}

/// Confirmations of `txid` proven through `src`, up to `want`.
async fn proven_confirmations(
    src: &dyn BtcSource,
    tx: &Transaction,
    want: u32,
    tip: u64,
    network: crate::noncustodial::network::Network,
) -> Result<(u32, Option<bitcoin::BlockHash>), AppError> {
    let txid = tx.compute_txid();
    let Some((height, block)) = src.tx_block(&txid).await? else {
        return Ok((0, None));
    };
    let header = src.header(&block).await?;
    let (merkle, pos) = src.merkle(&txid, &block).await?;
    let mut later = Vec::new();
    let extra = u64::from(want.saturating_sub(1));
    let mut h = height + 1;
    while h <= tip && h <= height + extra {
        later.push(src.header_at(h).await?);
        h += 1;
    }
    let v = btc_spv::verify(
        &InclusionProof {
            tx: tx.clone(),
            header,
            merkle,
            pos,
            later,
        },
        network,
    )?;
    if v.txid != txid || v.block_hash != block {
        return Err(AppError::Rpc(
            "a Bitcoin source's proof is for something else".into(),
        ));
    }
    Ok((v.confirmations, Some(block)))
}

/// Spec §1 and §3.6: a lock is proven from at least the policy's number of
/// sources (two on mainnet and testnet), every one agreeing. The one rule
/// for the settings and for every observation.
pub fn enough_sources(network: Network, configured: usize) -> Result<(), AppError> {
    let need = Policy::for_network(network).min_btc_sources;
    if configured < need {
        return Err(AppError::InvalidInput(format!(
            "the exchange needs at least {need} independent Bitcoin sources on {}; {configured} configured",
            network.display_name()
        )));
    }
    Ok(())
}

/// The view of `h`'s BTC lock. `Ok(BtcSeen { lock: None })` when no
/// source knows the lock transaction yet.
pub async fn observe(sources: &[Box<dyn BtcSource>], h: &Holdings) -> Result<BtcSeen, AppError> {
    let t = &h.terms;
    let network = t.network()?;
    enough_sources(network, sources.len())?;
    let Some(first) = sources.first() else {
        return Err(AppError::InvalidInput(
            "no Bitcoin source is configured".into(),
        ));
    };
    for src in sources {
        src.ready().await?;
    }
    let bp = t.btc_params()?;
    let (tip, mtp) = first.tip().await?;
    let Some(lock_ref) = &h.btc_lock else {
        return Ok(BtcSeen { mtp, lock: None });
    };
    let txid = btc_htlc::parse_txid(&lock_ref.txid)?;
    let outpoint = OutPoint {
        txid,
        vout: lock_ref.vout,
    };

    // The lock transaction from whichever source has it (review H2: one
    // source that does not know it is not "it does not exist").
    let mut found = None;
    for src in sources {
        if let Some(tx) = src.raw_tx(&txid).await? {
            if tx.compute_txid() != txid {
                return Err(AppError::Rpc(
                    "a Bitcoin source returned another transaction".into(),
                ));
            }
            found = Some(tx);
            break;
        }
    }
    let Some(tx) = found else {
        return Ok(BtcSeen { mtp, lock: None });
    };
    let matches = tx
        .output
        .get(lock_ref.vout as usize)
        .is_some_and(|o| btc_htlc::is_lock_output(o, &bp, t.btc_amount));
    let confs = proven_everywhere(sources, &tx, t.btc_confirmations, tip, network).await?;

    let spend = observe_spend(sources, h, &outpoint, tip, network).await?;
    Ok(BtcSeen {
        mtp,
        lock: Some(BtcLockSeen {
            outpoint: lock_ref.clone(),
            confirmations: confs,
            matches,
            spend,
        }),
    })
}

/// Confirmations of `tx` proven through every source, the same block from
/// each; 0 when any disagrees.
async fn proven_everywhere(
    sources: &[Box<dyn BtcSource>],
    tx: &Transaction,
    want: u32,
    tip: u64,
    network: crate::noncustodial::network::Network,
) -> Result<u32, AppError> {
    let mut confs = u32::MAX;
    let mut block: Option<Option<bitcoin::BlockHash>> = None;
    for src in sources {
        let (c, b) = proven_confirmations(src.as_ref(), tx, want, tip, network).await?;
        match &block {
            None => block = Some(b),
            Some(prev) if *prev != b => return Ok(0),
            _ => {}
        }
        confs = confs.min(c);
    }
    Ok(if confs == u32::MAX { 0 } else { confs })
}

/// The spend of the BTC lock, read from the spending transaction itself
/// (review H1): a claim only when its witness carries a secret that hashes
/// to `H`, a refund only when its witness is the lock's refund, and its
/// confirmations proven like the lock's. A spend no source can show is
/// `None`: nothing is decided on a source's word that the lock is spent.
async fn observe_spend(
    sources: &[Box<dyn BtcSource>],
    h: &Holdings,
    outpoint: &OutPoint,
    tip: u64,
    network: crate::noncustodial::network::Network,
) -> Result<Option<SpendSeen>, AppError> {
    let bp = h.terms.btc_params()?;
    // The spender, named by a source or one of our own presigned spends.
    let mut candidates: Vec<Txid> = Vec::new();
    for src in sources {
        if let Some(Outspend::Spent { txid: Some(id) }) = src.outspend(outpoint).await? {
            if !candidates.contains(&id) {
                candidates.push(id);
            }
        }
    }
    for id in own_spend_txids(h) {
        if !candidates.contains(&id) {
            candidates.push(id);
        }
    }
    for id in candidates {
        let mut spender = None;
        for src in sources {
            if let Some(tx) = src.raw_tx(&id).await? {
                if tx.compute_txid() == id {
                    spender = Some(tx);
                    break;
                }
            }
        }
        let Some(tx) = spender else { continue };
        let Some(input) = tx.input.iter().find(|i| i.previous_output == *outpoint) else {
            continue;
        };
        let items = input.witness.to_vec();
        let secret = script::secret_from_claim_witness(&bp, &items);
        let is_refund = items.len() == 3 && items[1].is_empty() && bp.matches(&items[2]);
        if secret.is_none() && !is_refund {
            continue; // not a spend of this lock
        }
        // Proven confirmations of the spend; 1 is enough to call it mined.
        let confirmations = proven_everywhere(sources, &tx, 1, tip, network).await?;
        return Ok(Some(SpendSeen {
            txid: id.to_string(),
            confirmations,
            secret,
        }));
    }
    Ok(None)
}
