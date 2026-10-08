//! The resolved, secret-free context every draft-building command starts
//! from: the active profile, its network and account, spendable coins,
//! settings and the profile's own node — plus the two node reads every name
//! draft needs (`getnameinfo`, the renewal block). Shared by the name commands
//! and the Shakedex commands, so neither imports the other.

use rand::RngCore;
use tauri::State;

use crate::db::queries;
use crate::error::AppError;
use crate::noncustodial::hd::ExtendedPubKey;
use crate::noncustodial::network::Network;
use crate::noncustodial::node_rpc::NodeRpc;
use crate::noncustodial::rpc::NodeRpcClient;
use crate::noncustodial::send::{self, SpendableCoin};
use crate::AppState;

pub(crate) fn random_id() -> String {
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

/// Resolved, secret-free build context for a covenant action.
#[derive(Debug)]
pub(crate) struct Ctx {
    pub(crate) profile_id: String,
    /// `wallet_profiles.kind`: `mnemonic_hot`, `ledger_hardware`, ...
    pub(crate) profile_kind: String,
    pub(crate) network: Network,
    pub(crate) account: u32,
    pub(crate) account_xpub: ExtendedPubKey,
    pub(crate) change_address: String,
    pub(crate) funding: Vec<SpendableCoin>,
    pub(crate) settings: std::collections::HashMap<String, String>,
    /// Node RPC client built from the *effective* per-profile config
    /// (per-profile override -> global settings -> default). Resolved once at
    /// `load_ctx` time under the DB lock so every RPC call this command issues
    /// (`getnameinfo`, `getblockchaininfo`, `getblockhash`, ...) targets the
    /// same node the active profile is pinned to — never a stale global URL.
    /// `NodeRpcClient` is `Clone`, so callers `ctx.node.clone()` freely.
    pub(crate) node: NodeRpcClient,
}

/// The active wallet profile, or why there is none.
pub(crate) fn active_profile(
    conn: &rusqlite::Connection,
) -> Result<crate::noncustodial::types::WalletProfileSummary, AppError> {
    let id = queries::get_active_profile_id(conn)?;
    if id.is_empty() {
        return Err(AppError::InvalidInput("no active wallet profile".into()));
    }
    queries::get_wallet_profile(conn, &id)?
        .ok_or_else(|| AppError::NotFound(format!("wallet profile {id}")))
}

pub(crate) fn load_ctx(state: &State<'_, AppState>) -> Result<Ctx, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let profile = active_profile(&conn)?;
    let id = profile.id.clone();
    if profile.watch_only {
        return Err(AppError::InvalidInput(
            "active profile is watch-only".into(),
        ));
    }
    let network = crate::noncustodial::derivation::network_from_profile(&profile.network)?;
    let account_xpub = ExtendedPubKey::from_xpub(network, &profile.account_xpub)?;
    let change = crate::noncustodial::derivation::derive_one(
        network,
        &account_xpub,
        crate::noncustodial::derivation::BRANCH_CHANGE,
        0,
    )?;
    let funding = send::load_spendable_coins(&conn, &id, None, network)?;
    let settings = queries::get_settings(&conn)?;
    // Build the node client from the effective per-profile config under the
    // same lock so a per-profile override always wins over the global URL for
    // every subsequent RPC call this Ctx serves.
    let node = NodeRpcClient::for_profile(&conn, &id)?;
    Ok(Ctx {
        profile_id: id,
        profile_kind: profile.kind,
        network,
        account: profile.account_index as u32,
        account_xpub,
        change_address: change.address,
        funding,
        settings,
        node,
    })
}

/// The fee rate to build with: the caller's, else the user's setting, else
/// the default. An unset or unreadable setting falls back to the default
/// rather than failing the build: the resulting fee is shown in the
/// confirmation before anything is signed.
pub(crate) fn fee_rate(ctx: &Ctx, fee_rate: Option<u64>) -> u64 {
    fee_rate
        .or_else(|| {
            ctx.settings
                .get("fee_rate_doos_per_kvb")
                .and_then(|s| s.parse::<u64>().ok())
                .map(|kvb| (kvb / 1000).max(send::MIN_FEE_RATE_PER_BYTE))
        })
        .unwrap_or(send::DEFAULT_FEE_RATE_PER_BYTE)
}

/// Minimal view of `getnameinfo` we need to build covenants.
#[derive(Debug, Clone)]
pub(crate) struct NameState {
    pub(crate) height: u32,
    pub(crate) value: u64,
    pub(crate) renewals: u32,
    pub(crate) claimed: u32,
    pub(crate) weak: bool,
    /// On-chain auction phase (e.g. "BIDDING", "OPENING", "REVEAL", "CLOSED").
    /// Populated from `getnameinfo.info.state`. Empty string when the node
    /// returns null / no state field. Uppercased for case-insensitive matching
    /// against consensus phase strings.
    pub(crate) phase: String,
}

pub(crate) async fn fetch_name_state(
    client: &dyn NodeRpc,
    name: &str,
) -> Result<NameState, AppError> {
    let v = client.get_name_info(name).await?;
    let info = v.get("info");
    let info = match info {
        Some(i) if !i.is_null() => i,
        _ => {
            return Err(AppError::InvalidInput(format!(
                "name '{name}' has no on-chain state"
            )))
        }
    };
    Ok(name_state_from_info(info))
}

/// [`fetch_name_state`] for a covenant whose fields come from the reply
/// (FINALIZE of a purchase): a reply without the name's height, renewals,
/// claimed count or weak flag is refused, where `fetch_name_state` would
/// default it and build a covenant the node rejects only after the user has
/// confirmed and signed it.
pub(crate) async fn fetch_name_state_strict(
    client: &dyn NodeRpc,
    name: &str,
) -> Result<NameState, AppError> {
    let v = client.get_name_info(name).await?;
    // hsd always sends `info` (`null` for a name with no state): a reply
    // without the key is not its answer.
    let info = match v.get("info") {
        None => {
            return Err(AppError::Rpc(format!(
                "node did not report the name's info for '{name}'"
            )))
        }
        Some(serde_json::Value::Null) => {
            return Err(AppError::InvalidInput(format!(
                "name '{name}' has no on-chain state"
            )))
        }
        Some(i) => i,
    };
    let missing =
        |k: &str| AppError::Rpc(format!("node did not report the name's {k} for '{name}'"));
    let get_u32 = |k: &str| {
        info.get(k)
            .and_then(|x| x.as_u64())
            .and_then(|x| u32::try_from(x).ok())
            .ok_or_else(|| missing(k))
    };
    Ok(NameState {
        height: get_u32("height")?,
        renewals: get_u32("renewals")?,
        claimed: get_u32("claimed")?,
        weak: info
            .get("weak")
            .and_then(|x| x.as_bool())
            .ok_or_else(|| missing("weak"))?,
        // Not part of the FINALIZE covenant: read as leniently as
        // `fetch_name_state` does.
        ..name_state_from_info(info)
    })
}

/// Lenient on purpose: the draft builders that use [`fetch_name_state`]
/// default a field the node leaves out, as they always have. A covenant whose
/// fields must come from the node uses [`fetch_name_state_strict`].
fn name_state_from_info(info: &serde_json::Value) -> NameState {
    let geti = |k: &str| info.get(k).and_then(|x| x.as_i64());
    NameState {
        height: geti("height").unwrap_or(0) as u32,
        value: geti("value").unwrap_or(0) as u64,
        renewals: geti("renewals").unwrap_or(0) as u32,
        claimed: geti("claimed").unwrap_or(0) as u32,
        weak: info.get("weak").and_then(|x| x.as_bool()).unwrap_or(false),
        phase: info
            .get("state")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_uppercase(),
    }
}

/// `getRenewalBlock`: 32-byte block hash at `height - 2*renewalMaturity`.
///
/// Unlike Bitcoin, HSD does NOT reverse block hashes for RPC display — the hex
/// returned by `getblockhash` is already in the raw internal byte order that
/// HSD's `chaindb.getEntryByHash` (and, transitively, the `bad-register-renewal`
/// consensus check in `chain.verifyRenewal`) uses to look the entry up. So we
/// decode the hex as-is; reversing it would produce an unknown hash and the
/// REGISTER / RENEW / FINALIZE covenants would be rejected as invalid on
/// broadcast. Verified against regtest: block `N+1`'s `previousblockhash`
/// equals block `N`'s `getblockhash` output byte-for-byte (see hsd
/// `lib/primitives/headers.js`), whereas Bitcoin-style RPC would reverse it.
pub(crate) async fn renewal_block(
    client: &dyn NodeRpc,
    network: Network,
) -> Result<[u8; 32], AppError> {
    let tip = client.get_blockchain_info().await?.blocks;
    let maturity = network.name_params().renewal_maturity as i64;
    let height = (tip - 2 * maturity).max(0);
    let hash_hex = client.get_block_hash(height).await?;
    let bytes =
        hex::decode(&hash_hex).map_err(|e| AppError::Rpc(format!("bad block hash: {e}")))?;
    if bytes.len() != 32 {
        return Err(AppError::Rpc("block hash not 32 bytes".into()));
    }
    let mut h = [0u8; 32];
    h.copy_from_slice(&bytes);
    Ok(h)
}

// --- consensus guards -------------------------------------------------------
//
// A draft hsd's consensus would refuse is refused here, before it is persisted
// and signed: hsd 8.0.0's `sendrawtransaction` hands back the txid even when
// its mempool turns the transaction away, so a broadcast cannot be trusted to
// say so. Each guard asks the node itself — `getnameinfo` and the tip — what
// `chain.js` will judge the transaction against, at the next block's height
// (`tip + 1`), never the wallet's cached copy of either.

/// The `info` object of hsd's `getnameinfo` reply for `name`. `null` is a name
/// with no state; a reply without the key is not hsd's answer (it always sends
/// one).
async fn node_name_info(client: &dyn NodeRpc, name: &str) -> Result<serde_json::Value, AppError> {
    let mut reply = client.get_name_info(name).await?;
    match reply.get_mut("info").map(serde_json::Value::take) {
        None => Err(AppError::Rpc(format!(
            "node did not report the name's info for '{name}'"
        ))),
        Some(serde_json::Value::Null) => Err(AppError::InvalidInput(format!(
            "name '{name}' has no on-chain state"
        ))),
        Some(info) => Ok(info),
    }
}

/// A block height field of `getnameinfo.info` (`renewal`, `transfer`), read
/// exactly as the node reports it: a reply without it gives no verdict.
fn info_height(info: &serde_json::Value, name: &str, key: &str) -> Result<i64, AppError> {
    info.get(key)
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok())
        .map(i64::from)
        .ok_or_else(|| AppError::Rpc(format!("node did not report the name's {key} for '{name}'")))
}

fn blocks_word(n: i64) -> &'static str {
    if n == 1 {
        "block"
    } else {
        "blocks"
    }
}

/// Refuses a RENEW of `name` that hsd would reject as `bad-renewal-premature`:
/// a name may be renewed only once `tree_interval` blocks have passed since it
/// was last registered, renewed or finalized (`ns.renewal`).
pub(crate) async fn ensure_renew_not_premature(
    client: &dyn NodeRpc,
    network: Network,
    name: &str,
) -> Result<(), AppError> {
    let info = node_name_info(client, name).await?;
    let renewal = info_height(&info, name, "renewal")?;
    let tip = client.get_blockchain_info().await?.blocks;
    let blocks = network.name_params().blocks_until_renew(renewal, tip);
    if blocks > 0 {
        return Err(AppError::InvalidInput(format!(
            "'{name}' was renewed too recently: it can be renewed in {blocks} {}",
            blocks_word(blocks)
        )));
    }
    Ok(())
}

/// Refuses a FINALIZE of `name` that hsd would reject: with no transfer
/// recorded on the name, or as `bad-finalize-maturity` while the transfer
/// lockup is not over (the same [`NameParams::blocks_until_finalize`] the
/// Finalize button is offered by).
///
/// [`NameParams::blocks_until_finalize`]: crate::noncustodial::network::NameParams::blocks_until_finalize
pub(crate) async fn ensure_finalize_matured(
    client: &dyn NodeRpc,
    network: Network,
    name: &str,
) -> Result<(), AppError> {
    let info = node_name_info(client, name).await?;
    let transfer = info_height(&info, name, "transfer")?;
    if transfer == 0 {
        return Err(AppError::InvalidInput(format!(
            "the node reports no transfer of '{name}': nothing to finalize"
        )));
    }
    let tip = client.get_blockchain_info().await?.blocks;
    let blocks = network.name_params().blocks_until_finalize(transfer, tip);
    if blocks > 0 {
        return Err(AppError::InvalidInput(format!(
            "the transfer of '{name}' is still locked for {blocks} more {}",
            blocks_word(blocks)
        )));
    }
    Ok(())
}

/// The reveal coins in `coins` that a REDEEM may spend: every one but the
/// name's owner coin as the node reports it (`getnameinfo.info.owner`). Until
/// REGISTER spends it, the winning reveal IS the owner coin, and hsd rejects
/// redeeming it (`bad-redeem-owner`), taking the whole transaction down with
/// it. Refused when `coins` held nothing but the owner coin; an empty `coins`
/// is returned as it is, for the caller to say there is nothing to redeem.
pub(crate) async fn exclude_owner_reveal(
    client: &dyn NodeRpc,
    name: &str,
    coins: Vec<queries::NameCoin>,
) -> Result<Vec<queries::NameCoin>, AppError> {
    if coins.is_empty() {
        return Ok(coins);
    }
    let info = node_name_info(client, name).await?;
    let owner = info.get("owner");
    let hash = owner.and_then(|o| o.get("hash")).and_then(|h| h.as_str());
    let index = owner.and_then(|o| o.get("index")).and_then(|i| i.as_u64());
    let (Some(hash), Some(index)) = (hash, index) else {
        return Err(AppError::Rpc(format!(
            "node did not report the name's owner for '{name}'"
        )));
    };
    let losing: Vec<_> = coins
        .into_iter()
        .filter(|c| !(c.txid.eq_ignore_ascii_case(hash) && u64::from(c.vout) == index))
        .collect();
    if losing.is_empty() {
        return Err(AppError::InvalidInput(format!(
            "your reveal for '{name}' won the auction: it is the name's owner coin and cannot \
             be redeemed (register the name instead)"
        )));
    }
    Ok(losing)
}
