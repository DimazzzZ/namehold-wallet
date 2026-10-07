//! The resolved, secret-free context every draft-building command starts
//! from: the active profile, its network and account, spendable coins,
//! settings and the profile's own node — plus the two node reads every name
//! draft needs (`getnameinfo`, the renewal block). Kept apart from the name
//! commands so other draft-building commands can share it without importing
//! them.

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

/// Lenient on purpose: the draft builders that use [`fetch_name_state`]
/// default a field the node leaves out, as they always have.
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
