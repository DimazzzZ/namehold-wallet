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
use crate::noncustodial::exchange::engine::{BtcLockSeen, BtcSeen, Holdings, Role, SpendSeen};
use crate::noncustodial::exchange::script;

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

/// The view of `h`'s BTC lock. `Ok(BtcSeen { lock: None })` when no
/// source knows the lock transaction yet.
pub async fn observe(sources: &[Box<dyn BtcSource>], h: &Holdings) -> Result<BtcSeen, AppError> {
    let Some(first) = sources.first() else {
        return Err(AppError::InvalidInput(
            "no Bitcoin source is configured".into(),
        ));
    };
    let t = &h.terms;
    let network = t.network()?;
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

    let Some(tx) = first.raw_tx(&txid).await? else {
        return Ok(BtcSeen { mtp, lock: None });
    };
    if tx.compute_txid() != txid {
        return Err(AppError::Rpc(
            "a Bitcoin source returned another transaction".into(),
        ));
    }
    let matches = tx
        .output
        .get(lock_ref.vout as usize)
        .is_some_and(|o| btc_htlc::is_lock_output(o, &bp, t.btc_amount));

    // Proven confirmations, the same block from every source.
    let mut confs = u32::MAX;
    let mut block = None;
    for src in sources {
        let (c, b) =
            proven_confirmations(src.as_ref(), &tx, t.btc_confirmations, tip, network).await?;
        match (&block, b) {
            (None, b) => block = Some(b),
            (Some(prev), b) if *prev != b => {
                confs = 0; // the sources disagree: nothing is proven
            }
            _ => {}
        }
        confs = confs.min(c);
    }
    if confs == u32::MAX {
        confs = 0;
    }

    // The spend.
    let spend = match first.outspend(&outpoint).await? {
        None | Some(Outspend::Unspent) => None,
        Some(Outspend::Spent { txid: spender }) => {
            Some(classify_spend(first.as_ref(), h, spender).await?)
        }
    };

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

async fn classify_spend(
    src: &dyn BtcSource,
    h: &Holdings,
    spender: Option<Txid>,
) -> Result<SpendSeen, AppError> {
    let bp = h.terms.btc_params()?;
    let own = own_spend_txids(h);
    // Which known transaction spent it, if the source cannot say.
    let spender = match spender {
        Some(s) => Some(s),
        None => {
            let mut found = None;
            for id in &own {
                if src.raw_tx(id).await?.is_some() {
                    found = Some(*id);
                    break;
                }
            }
            found
        }
    };
    let confirmations = match &spender {
        Some(id) => match src.tx_block(id).await? {
            Some((height, _)) => {
                let (tip, _) = src.tip().await?;
                u32::try_from(tip.saturating_sub(height) + 1).unwrap_or(u32::MAX)
            }
            None => 0,
        },
        None => 0,
    };
    let txid = spender.map(|s| s.to_string()).unwrap_or_default();
    // A claim carries the secret; read it from the spender's witness when we
    // can, else infer it from who we are: a spend that is not ours is the
    // other path.
    let witness_secret = match &spender {
        Some(id) => src.raw_tx(id).await?.and_then(|tx| {
            tx.input
                .iter()
                .find_map(|i| script::secret_from_claim_witness(&bp, &i.witness.to_vec()))
        }),
        None => None,
    };
    let is_ours = spender.is_some_and(|s| own.contains(&s));
    let is_claim = match h.role {
        // The Buyer's own spends are refunds; anything else is the Seller's claim.
        Role::Buyer => !is_ours,
        // The Seller's own spends are claims; anything else is the Buyer's refund.
        Role::Seller => is_ours,
    };
    let secret = if is_claim {
        // The Buyer knows its secret; the Seller read it from the HNS chain
        // and put it in its claim. Either way a claim is marked by `Some`.
        witness_secret.or(h.secret).or(Some([0u8; 32]))
    } else {
        None
    };
    Ok(SpendSeen {
        txid,
        confirmations,
        secret,
    })
}
