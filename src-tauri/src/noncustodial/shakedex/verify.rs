//! Everything a buyer relies on, checked against the profile's node. The
//! market's own availability is never consulted.

use serde::Serialize;

use crate::error::AppError;

use crate::noncustodial::network::{NameParams, Network, BLOCKS_PER_DAY};
use crate::noncustodial::node_rpc::NodeRpc;
use crate::noncustodial::shakedex::listing_file::ListingFile;
use crate::noncustodial::shakedex::script::lock_address;
use crate::noncustodial::shakedex::template::{self, StepTemplate};
use crate::noncustodial::sync::COV_FINALIZE;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Hidden {
    SoldOrCancelled,
    FailedVerification {
        reason: String,
    },
    ExpiresBeforeFinalize,
    #[serde(rename_all = "camelCase")]
    NotYetValid {
        first_valid_in_secs: u64,
    },
    CouldNotCheck {
        reason: String,
    },
    /// Not checked at all: the profile's chain source (SPV, Explorer) cannot
    /// run the checks. Distinct from `CouldNotCheck`, where a node that can
    /// run them failed to answer.
    Unverified,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Buyable {
    pub current_step: usize,
    pub next_step: Option<usize>,
    pub lock_value: u64,
    pub name_height: u32,
    pub expiry_end: u32,
    pub warn_expiry: bool,
    pub mtp: u64,
    pub tip: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "verdict", rename_all = "camelCase")]
pub enum Verdict {
    Buyable(Buyable),
    Hidden(Hidden),
}

/// `(buyable, warn)`: whether the name outlives the transfer lockup plus a
/// day's margin after `tip`, and whether it expires within a month beyond that.
pub fn finalize_deadline_ok(expiry_end: u32, tip: u32, p: &NameParams) -> (bool, bool) {
    // "One day" in blocks; on regtest/simnet, whose lockup is shorter than a
    // day, the lockup itself.
    let day = u64::from(p.transfer_lockup).min(BLOCKS_PER_DAY as u64);
    let margin = tip as u64 + 1 + p.transfer_lockup as u64 + day;
    let ok = (expiry_end as u64) > margin;
    let warn = (expiry_end as u64) <= margin + 30 * day;
    (ok, warn)
}

/// The node's tip height and median time past, which every verification is
/// measured against.
#[derive(Debug, Clone, Copy)]
pub struct ChainPoint {
    pub tip: u32,
    pub mtp: u64,
}

/// Read the tip and MTP from the node, or the reason a listing could not be
/// checked.
pub async fn chain_point(client: &dyn NodeRpc) -> Result<ChainPoint, Hidden> {
    let info = client
        .get_blockchain_info()
        .await
        .map_err(|e| Hidden::CouldNotCheck {
            reason: e.to_string(),
        })?;
    let mtp = info.mediantime.ok_or_else(|| Hidden::CouldNotCheck {
        reason: "node did not report median time".into(),
    })?;
    let tip = u32::try_from(info.blocks).map_err(|_| Hidden::CouldNotCheck {
        reason: format!("node reported block height {}", info.blocks),
    })?;
    Ok(ChainPoint { tip, mtp })
}

pub async fn verify_listing(client: &dyn NodeRpc, network: Network, l: &ListingFile) -> Verdict {
    match chain_point(client).await {
        Ok(at) => verify_listing_at(client, network, l, at).await,
        Err(h) => Verdict::Hidden(h),
    }
}

/// [`verify_listing`] against a tip and MTP already read, so a page of
/// listings costs one `getblockchaininfo`.
pub async fn verify_listing_at(
    client: &dyn NodeRpc,
    network: Network,
    l: &ListingFile,
    at: ChainPoint,
) -> Verdict {
    let hide = Verdict::Hidden;
    let fail = |m: &str| {
        Verdict::Hidden(Hidden::FailedVerification {
            reason: m.to_owned(),
        })
    };
    let ChainPoint { tip, mtp } = at;
    let txid = hex::encode(l.lock_txid);
    let coin = match client.get_coin(&txid, l.lock_vout).await {
        Ok(Some(c)) => c,
        Ok(None) => return hide(Hidden::SoldOrCancelled),
        Err(e) => {
            return hide(Hidden::CouldNotCheck {
                reason: e.to_string(),
            })
        }
    };
    // hsd always reports both: a reply without one says nothing about the
    // listing either way.
    let Some(coin_address) = coin.address.as_deref() else {
        return hide(Hidden::CouldNotCheck {
            reason: "node did not report the lock coin's address".into(),
        });
    };
    // The whole address — network, witness version and program — must be
    // this listing's lock: comparing the program alone would accept the
    // same bytes under another witness version.
    match lock_address(network, &l.public_key) {
        Ok(lock) if lock == coin_address => {}
        _ => return fail("lock coin is not paid to this listing's lock"),
    }
    let Some(cov) = coin.covenant.as_ref() else {
        return hide(Hidden::CouldNotCheck {
            reason: "node did not report the lock coin's covenant".into(),
        });
    };
    if cov.kind != COV_FINALIZE {
        return fail("lock coin does not hold this name");
    }
    // hsd's FINALIZE covenant always carries the name: a reply without it
    // says nothing about which name the coin holds.
    let Some(cov_name) = cov.items.get(2) else {
        return hide(Hidden::CouldNotCheck {
            reason: "node did not report the name in the lock coin's covenant".into(),
        });
    };
    if !cov_name.eq_ignore_ascii_case(&hex::encode(l.name.as_bytes())) {
        return fail("lock coin does not hold this name");
    }
    // Zero is a real value: a name won with a single bid pays nothing at
    // reveal, so its coin (and the lock coin after it) is worth 0.
    if coin.value < 0 {
        return fail("lock coin has a negative value");
    }
    let name_info = match client.get_name_info(&l.name).await {
        Ok(v) => v,
        Err(e) => {
            return hide(Hidden::CouldNotCheck {
                reason: e.to_string(),
            })
        }
    };
    // hsd always sends `info`, and reports an expired name as `"info": null`.
    // A reply without the key is not hsd saying the name expired.
    let Some(ni) = name_info.get("info") else {
        return hide(Hidden::CouldNotCheck {
            reason: "node did not report the name's info".into(),
        });
    };
    if ni.is_null() {
        return hide(Hidden::ExpiresBeforeFinalize);
    }
    let (Some(owner_hash), Some(owner_index)) =
        (ni["owner"]["hash"].as_str(), ni["owner"]["index"].as_u64())
    else {
        return hide(Hidden::CouldNotCheck {
            reason: "node did not report the name's owner".into(),
        });
    };
    if owner_hash != txid || owner_index != u64::from(l.lock_vout) {
        return fail("the name has changed hands since this listing was made");
    }
    let Some(name_height) = ni["height"].as_u64().and_then(|h| u32::try_from(h).ok()) else {
        return hide(Hidden::CouldNotCheck {
            reason: "node did not report the name's height".into(),
        });
    };
    // hsd writes the height as 4 little-endian bytes in hex: anything else
    // is not its reply, and proves nothing about who holds the name.
    let Some(cov_height) = cov
        .items
        .get(1)
        .and_then(|h| hex::decode(h).ok())
        .and_then(|b| <[u8; 4]>::try_from(b).ok())
        .map(u32::from_le_bytes)
    else {
        return hide(Hidden::CouldNotCheck {
            reason: "node did not report a readable height in the lock coin's covenant".into(),
        });
    };
    if cov_height != name_height {
        return fail("the name has changed hands since this listing was made");
    }
    let payment = match l.payment_output_address(network) {
        Ok(a) => a,
        Err(_) => return fail("payment address is invalid"),
    };
    if l.steps.is_empty() {
        return fail("listing has no price steps");
    }
    for step in &l.steps {
        let t = StepTemplate {
            lock_outpoint: (l.lock_txid, l.lock_vout),
            lock_value: coin.value as u64,
            lock_pubkey: &l.public_key,
            payment: payment.clone(),
            price: step.price,
            lock_time_secs: step.lock_time,
        };
        if template::verify_step_signature(&t, &step.signature).is_err() {
            return fail("a price step's signature does not verify");
        }
    }
    let encoded = match l.encoded_steps() {
        Ok(v) => v,
        // Unreachable: `ListingFile::parse` refuses a lock time it cannot
        // encode; kept so a listing built another way still fails closed.
        Err(_) => return fail("a price step's lock time is out of range"),
    };
    let Some(current) = template::current_step_index(&encoded, mtp) else {
        let first_valid_in_secs = l
            .steps
            .iter()
            .map(|s| template::secs_until_valid(s.lock_time, mtp))
            .min()
            // Unreachable: `ListingFile::parse` refuses a listing without steps.
            .unwrap_or(0);
        return hide(Hidden::NotYetValid {
            first_valid_in_secs,
        });
    };
    let next = encoded
        .iter()
        .enumerate()
        .filter(|(_, (price, enc))| {
            !template::is_valid_at(*enc, mtp) && *price < encoded[current].0
        })
        .min_by_key(|(_, (_, enc))| *enc)
        .map(|(i, _)| i);
    let params = network.name_params();
    let Some(renewal) = ni["renewal"].as_u64().and_then(|r| u32::try_from(r).ok()) else {
        return hide(Hidden::CouldNotCheck {
            reason: "node did not report the name's renewal height".into(),
        });
    };
    let Some(claimed) = ni["claimed"].as_u64().map(|c| c > 0) else {
        return hide(Hidden::CouldNotCheck {
            reason: "node did not report whether the name was claimed".into(),
        });
    };
    // A height past u32 is beyond any chain's reach: saturate, as before.
    let end = u32::try_from(params.expiry_end(i64::from(renewal), claimed)).unwrap_or(u32::MAX);
    let (ok, warn) = finalize_deadline_ok(end, tip, &params);
    if !ok {
        return hide(Hidden::ExpiresBeforeFinalize);
    }
    Verdict::Buyable(Buyable {
        current_step: current,
        next_step: next,
        lock_value: coin.value as u64,
        name_height,
        expiry_end: end,
        warn_expiry: warn,
        mtp,
        tip,
    })
}

/// Refusal when a cheaper step became valid between review and broadcast.
pub const PRICE_CHANGED: &str = "the price changed — review the purchase again";

/// Just before a purchase is broadcast: refuse if a step cheaper than the one
/// it pays (`paid`) has become valid on the node since it was reviewed. Fails
/// closed — without the node's median time nothing is sent.
pub async fn recheck_price(
    client: &dyn NodeRpc,
    network: Network,
    listing_json: &str,
    paid: u64,
) -> Result<(), AppError> {
    if cheaper_step_valid(client, network, listing_json, paid).await? {
        return Err(AppError::InvalidInput(PRICE_CHANGED.into()));
    }
    Ok(())
}

/// Whether a step of the listing cheaper than `paid` is valid at the node's
/// median time now. An error when the node does not report its median time:
/// that proves neither.
pub async fn cheaper_step_valid(
    client: &dyn NodeRpc,
    network: Network,
    listing_json: &str,
    paid: u64,
) -> Result<bool, AppError> {
    let listing = ListingFile::parse(listing_json, network)?;
    let mtp = client
        .get_blockchain_info()
        .await?
        .mediantime
        .ok_or_else(|| {
            AppError::InvalidInput(
                "node did not report median time, so the price could not be re-checked; \
                 the purchase was not sent"
                    .into(),
            )
        })?;
    let encoded = listing.encoded_steps()?;
    Ok(matches!(
        template::current_step_index(&encoded, mtp),
        Some(i) if encoded[i].0 < paid
    ))
}
