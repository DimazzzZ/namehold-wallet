//! Covenant / name action commands: build OPEN/BID/REVEAL/REDEEM/REGISTER/
//! UPDATE/RENEW/TRANSFER/FINALIZE/CANCEL/REVOKE drafts.
//!
//! Each command resolves the active profile, fetches the current name state from
//! the node (`getnameinfo`), constructs the covenant + funded plan via
//! `noncustodial::actions`, and persists a `wallet_tx_drafts` row. Signing and
//! broadcast reuse `commands::tx::{sign_tx_draft, broadcast_tx_draft}` — covenant
//! draft plans are signed by `actions::sign_plan` (dispatched there by action).
//!
//! NOTE: on-chain validity (value math, renewal-block selection, bid matching)
//! must be validated against a regtest node before mainnet use; the default
//! network is regtest and writes are gated by the unlocked signer + broadcaster.

use rand::RngCore;
use serde::Serialize;
use tauri::State;

use crate::db::{self, queries};
use crate::error::AppError;
use crate::noncustodial::actions::{self, NameInputSpec, PrimaryOutput};
use crate::noncustodial::hd::ExtendedPubKey;
use crate::noncustodial::network::Network;
use crate::noncustodial::rpc::NodeRpcClient;
use crate::noncustodial::send::{self, SpendableCoin};
use crate::noncustodial::tx::sighash;
use crate::noncustodial::types::TxDraftSummary;
use crate::noncustodial::{address, bids, covenants, names, resource};
use crate::AppState;

fn random_id() -> String {
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

/// Resolved, secret-free build context for a covenant action.
struct Ctx {
    profile_id: String,
    network: Network,
    account: u32,
    account_xpub: ExtendedPubKey,
    change_address: String,
    funding: Vec<SpendableCoin>,
    settings: std::collections::HashMap<String, String>,
}

fn load_ctx(state: &State<'_, AppState>) -> Result<Ctx, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let id = queries::get_active_profile_id(&conn)?;
    if id.is_empty() {
        return Err(AppError::InvalidInput("no active wallet profile".into()));
    }
    let profile = queries::get_wallet_profile(&conn, &id)?
        .ok_or_else(|| AppError::NotFound(format!("wallet profile {id}")))?;
    if profile.watch_only {
        return Err(AppError::InvalidInput("active profile is watch-only".into()));
    }
    let network = crate::noncustodial::derivation::network_from_profile(&profile.network)?;
    let account_xpub = ExtendedPubKey::from_xpub(network, &profile.account_xpub)?;
    let change = crate::noncustodial::derivation::derive_one(
        network,
        &account_xpub,
        crate::noncustodial::derivation::BRANCH_CHANGE,
        0,
    )?;
    let funding = send::load_spendable_coins(&conn, &id)?;
    let settings = queries::get_settings(&conn)?;
    Ok(Ctx {
        profile_id: id,
        network,
        account: profile.account_index as u32,
        account_xpub,
        change_address: change.address,
        funding,
        settings,
    })
}

fn fee_rate(ctx: &Ctx, fee_rate: Option<u64>) -> u64 {
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
struct NameState {
    height: u32,
    value: u64,
    renewals: u32,
    claimed: u32,
    weak: bool,
}

async fn fetch_name_state(client: &NodeRpcClient, name: &str) -> Result<NameState, AppError> {
    let v = client.get_name_info(name).await?;
    let info = v.get("info");
    let info = match info {
        Some(i) if !i.is_null() => i,
        _ => return Err(AppError::InvalidInput(format!("name '{name}' has no on-chain state"))),
    };
    let geti = |k: &str| info.get(k).and_then(|x| x.as_i64());
    Ok(NameState {
        height: geti("height").unwrap_or(0) as u32,
        value: geti("value").unwrap_or(0) as u64,
        renewals: geti("renewals").unwrap_or(0) as u32,
        claimed: geti("claimed").unwrap_or(0) as u32,
        weak: info.get("weak").and_then(|x| x.as_bool()).unwrap_or(false),
    })
}

/// `getRenewalBlock`: internal-order 32-byte hash at `height - 2*renewalMaturity`.
async fn renewal_block(client: &NodeRpcClient, network: Network) -> Result<[u8; 32], AppError> {
    let tip = client.get_blockchain_info().await?.blocks;
    let maturity = network.name_params().renewal_maturity as i64;
    let height = (tip - 2 * maturity).max(0);
    let hash_hex = client.get_block_hash(height).await?;
    let bytes = hex::decode(&hash_hex)
        .map_err(|e| AppError::Rpc(format!("bad block hash: {e}")))?;
    if bytes.len() != 32 {
        return Err(AppError::Rpc("block hash not 32 bytes".into()));
    }
    let mut h = [0u8; 32];
    h.copy_from_slice(&bytes);
    h.reverse(); // display -> internal
    Ok(h)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ActionSummary<'a> {
    action: &'a str,
    name: &'a str,
    send_total_doos: i64,
    fee_doos: i64,
    change_doos: i64,
    input_total_doos: i64,
    num_inputs: i64,
    recipient_address: Option<&'a str>,
    txid: Option<&'a str>,
}

/// Persist a planned covenant draft and return its summary.
fn persist(
    state: &State<'_, AppState>,
    profile_id: &str,
    action: &str,
    name: &str,
    recipient: Option<&str>,
    res: &actions::PlanResult,
) -> Result<TxDraftSummary, AppError> {
    let summary = ActionSummary {
        action,
        name,
        send_total_doos: res.plan.outputs[0].value as i64,
        fee_doos: res.fee as i64,
        change_doos: res.change as i64,
        input_total_doos: res.input_total as i64,
        num_inputs: res.plan.inputs.len() as i64,
        recipient_address: recipient,
        txid: Some(&res.txid),
    };
    let id = random_id();
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::insert_tx_draft(
        &conn,
        &id,
        profile_id,
        action,
        &res.unsigned_tx_hex,
        &serde_json::to_string(&res.plan)?,
        &serde_json::to_string(&summary)?,
    )?;
    db::queries::get_tx_draft(&conn, &id)?
        .map(|d| d.to_summary())
        .ok_or_else(|| AppError::Other("draft vanished after insert".into()))
}

fn name_input_from(coin: queries::NameCoin) -> NameInputSpec {
    NameInputSpec {
        txid: coin.txid,
        vout: coin.vout,
        value: coin.value,
        branch: coin.branch,
        child_index: coin.child_index,
        sighash_type: sighash::ALL,
    }
}

// --- OPEN ------------------------------------------------------------------

#[tauri::command]
pub async fn build_open_draft(
    state: State<'_, AppState>,
    name: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let nh = names::hash_name(&name)?;
    let raw = names::raw_name(&name)?;
    // OPEN output goes to a wallet receive address (value 0).
    let recv = crate::noncustodial::derivation::derive_one(
        ctx.network,
        &ctx.account_xpub,
        crate::noncustodial::derivation::BRANCH_RECEIVE,
        0,
    )?;
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        None,
        PrimaryOutput { value: 0, address: recv.address, covenant: covenants::open(&nh, &raw) },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist(&state, &ctx.profile_id, "open", &name, None, &res)
}

// --- BID -------------------------------------------------------------------

#[tauri::command]
pub async fn build_bid_draft(
    state: State<'_, AppState>,
    name: String,
    bid_value: i64,
    lockup: i64,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    if bid_value <= 0 || lockup < bid_value {
        return Err(AppError::InvalidInput("lockup must be >= bid value > 0".into()));
    }
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let nh = names::hash_name(&name)?;
    let raw = names::raw_name(&name)?;
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let ns = fetch_name_state(&client, &name).await?;

    // Bid output goes to a wallet receive address (branch 0, index 0).
    let bid_addr = crate::noncustodial::derivation::derive_one(
        ctx.network,
        &ctx.account_xpub,
        crate::noncustodial::derivation::BRANCH_RECEIVE,
        0,
    )?;
    let (_v, program) = address::decode(ctx.network, &bid_addr.address)?;
    let mut addr_hash = [0u8; 20];
    if program.len() != 20 {
        return Err(AppError::InvalidInput("bid address is not p2wpkh".into()));
    }
    addr_hash.copy_from_slice(&program);

    let nonce = bids::compute_nonce(&ctx.account_xpub, &nh, &addr_hash, bid_value as u64)?;
    let blind = bids::compute_blind(bid_value as u64, &nonce);
    let cov = covenants::bid(&nh, ns.height, &raw, &blind);

    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        None,
        PrimaryOutput { value: lockup as u64, address: bid_addr.address.clone(), covenant: cov },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;

    // Persist the bid commitment (secret nonce/blind) before returning.
    {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        queries::insert_bid_commitment(
            &conn,
            &ctx.profile_id,
            &name,
            &hex::encode(nh),
            &bid_addr.address,
            crate::noncustodial::derivation::BRANCH_RECEIVE as i64,
            0,
            bid_value,
            lockup,
            &hex::encode(nonce),
            &hex::encode(blind),
        )?;
    }
    persist(&state, &ctx.profile_id, "bid", &name, None, &res)
}

// --- REVEAL ----------------------------------------------------------------

#[tauri::command]
pub async fn build_reveal_draft(
    state: State<'_, AppState>,
    name: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let nh = names::hash_name(&name)?;
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let ns = fetch_name_state(&client, &name).await?;

    // Look up our bid commitment + the unspent BID coin at that address.
    let (bid, bid_coin) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let bid = queries::get_bid_commitment(&conn, &ctx.profile_id, &name)?
            .ok_or_else(|| AppError::NotFound(format!("no bid commitment for '{name}'")))?;
        let coin = queries::find_unspent_covenant_utxo(
            &conn,
            &ctx.profile_id,
            &bid.address,
            crate::noncustodial::sync::COV_BID as i64,
        )?
        .ok_or_else(|| AppError::NotFound("no unspent bid coin (sync first?)".into()))?;
        (bid, coin)
    };
    let mut nonce = [0u8; 32];
    let nb = hex::decode(&bid.nonce_hex).map_err(|e| AppError::Crypto(format!("nonce: {e}")))?;
    if nb.len() != 32 {
        return Err(AppError::Crypto("stored nonce not 32 bytes".into()));
    }
    nonce.copy_from_slice(&nb);

    let cov = covenants::reveal(&nh, ns.height, &nonce);
    // Reveal output value = the true bid value; output address = the bid coin's
    // address. The lockup − bid difference returns as change automatically.
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(bid_coin.clone())),
        PrimaryOutput {
            value: bid.bid_value_doos as u64,
            address: bid_coin.address.clone(),
            covenant: cov,
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist(&state, &ctx.profile_id, "reveal", &name, None, &res)
}

// --- REDEEM ----------------------------------------------------------------

#[tauri::command]
pub async fn build_redeem_draft(
    state: State<'_, AppState>,
    name: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let nh = names::hash_name(&name)?;
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let ns = fetch_name_state(&client, &name).await?;

    let coin = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let bid = queries::get_bid_commitment(&conn, &ctx.profile_id, &name)?
            .ok_or_else(|| AppError::NotFound(format!("no bid for '{name}'")))?;
        queries::find_unspent_covenant_utxo(
            &conn,
            &ctx.profile_id,
            &bid.address,
            crate::noncustodial::sync::COV_REVEAL as i64,
        )?
        .ok_or_else(|| AppError::NotFound("no unspent losing reveal coin".into()))?
    };
    // REDEEM reclaims the reveal output value back to the wallet.
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(coin.clone())),
        PrimaryOutput {
            value: coin.value,
            address: coin.address.clone(),
            covenant: covenants::redeem(&nh, ns.height),
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist(&state, &ctx.profile_id, "redeem", &name, None, &res)
}

// --- owner actions (spend the name's owner UTXO) ---------------------------

/// Common loader: fetch our owner coin + current name state.
async fn owner_coin_and_state(
    state: &State<'_, AppState>,
    ctx: &Ctx,
    name: &str,
) -> Result<(queries::NameCoin, NameState), AppError> {
    let coin = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        queries::get_name_coin(&conn, &ctx.profile_id, name)?
            .ok_or_else(|| AppError::NotFound(format!("wallet does not hold '{name}' (sync?)")))?
    };
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let ns = fetch_name_state(&client, name).await?;
    Ok((coin, ns))
}

#[tauri::command]
pub async fn build_register_draft(
    state: State<'_, AppState>,
    name: String,
    records: Option<Vec<serde_json::Value>>,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let nh = names::hash_name(&name)?;
    let (coin, ns) = owner_coin_and_state(&state, &ctx, &name).await?;
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let rblock = renewal_block(&client, ctx.network).await?;
    let res_bytes = match &records {
        Some(r) if !r.is_empty() => resource::encode(r)?,
        _ => Vec::new(), // EMPTY resource
    };
    // REGISTER locks `ns.value` (the price); the rest returns as change.
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(coin.clone())),
        PrimaryOutput {
            value: ns.value,
            address: coin.address.clone(),
            covenant: covenants::register(&nh, ns.height, &res_bytes, &rblock),
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist(&state, &ctx.profile_id, "register", &name, None, &res)
}

#[tauri::command]
pub async fn build_update_draft(
    state: State<'_, AppState>,
    name: String,
    records: Vec<serde_json::Value>,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let nh = names::hash_name(&name)?;
    let (coin, ns) = owner_coin_and_state(&state, &ctx, &name).await?;
    let res_bytes = resource::encode(&records)?;
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(coin.clone())),
        PrimaryOutput {
            value: coin.value,
            address: coin.address.clone(),
            covenant: covenants::update(&nh, ns.height, &res_bytes),
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist(&state, &ctx.profile_id, "update", &name, None, &res)
}

#[tauri::command]
pub async fn build_renew_draft(
    state: State<'_, AppState>,
    name: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let nh = names::hash_name(&name)?;
    let (coin, ns) = owner_coin_and_state(&state, &ctx, &name).await?;
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let rblock = renewal_block(&client, ctx.network).await?;
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(coin.clone())),
        PrimaryOutput {
            value: coin.value,
            address: coin.address.clone(),
            covenant: covenants::renew(&nh, ns.height, &rblock),
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist(&state, &ctx.profile_id, "renew", &name, None, &res)
}

#[tauri::command]
pub async fn build_transfer_draft(
    state: State<'_, AppState>,
    name: String,
    recipient: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let nh = names::hash_name(&name)?;
    let (coin, ns) = owner_coin_and_state(&state, &ctx, &name).await?;
    let (version, program) = address::decode(ctx.network, &recipient)?;
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(coin.clone())),
        PrimaryOutput {
            value: coin.value,
            address: coin.address.clone(),
            covenant: covenants::transfer(&nh, ns.height, version, &program),
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist(&state, &ctx.profile_id, "transfer", &name, Some(&recipient), &res)
}

#[tauri::command]
pub async fn build_finalize_draft(
    state: State<'_, AppState>,
    name: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let nh = names::hash_name(&name)?;
    let raw = names::raw_name(&name)?;
    let (coin, ns) = owner_coin_and_state(&state, &ctx, &name).await?;
    let client = NodeRpcClient::from_settings(&ctx.settings);
    let rblock = renewal_block(&client, ctx.network).await?;

    // The finalize output goes to the TRANSFER target recorded on the owner
    // coin's covenant: items = [nameHash, height, version(u8), addrHash].
    let cov_json = coin
        .covenant_json
        .as_deref()
        .ok_or_else(|| AppError::InvalidInput("name is not in transfer; nothing to finalize".into()))?;
    let cov: serde_json::Value = serde_json::from_str(cov_json)?;
    let items = cov.get("items").and_then(|i| i.as_array())
        .ok_or_else(|| AppError::InvalidInput("owner coin has no covenant items".into()))?;
    if items.len() < 4 {
        return Err(AppError::InvalidInput("owner coin is not a TRANSFER".into()));
    }
    let ver_hex = items[2].as_str().unwrap_or("00");
    let hash_hex = items[3].as_str().unwrap_or("");
    let version = u8::from_str_radix(ver_hex, 16).unwrap_or(0);
    let target_hash = hex::decode(hash_hex)
        .map_err(|e| AppError::InvalidInput(format!("bad transfer target: {e}")))?;
    if version != 0 || target_hash.len() != 20 {
        return Err(AppError::InvalidInput("finalize target must be p2wpkh".into()));
    }
    let mut h160 = [0u8; 20];
    h160.copy_from_slice(&target_hash);
    let target_address = address::encode_p2wpkh(ctx.network, &h160)?;

    let flags: u8 = if ns.weak { 1 } else { 0 };
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(coin.clone())),
        PrimaryOutput {
            value: coin.value,
            address: target_address.clone(),
            covenant: covenants::finalize(&nh, ns.height, &raw, flags, ns.claimed, ns.renewals, &rblock),
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist(&state, &ctx.profile_id, "finalize", &name, Some(&target_address), &res)
}

#[tauri::command]
pub async fn build_cancel_draft(
    state: State<'_, AppState>,
    name: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let nh = names::hash_name(&name)?;
    let (coin, ns) = owner_coin_and_state(&state, &ctx, &name).await?;
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(coin.clone())),
        PrimaryOutput {
            value: coin.value,
            address: coin.address.clone(),
            covenant: covenants::cancel(&nh, ns.height),
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist(&state, &ctx.profile_id, "cancel", &name, None, &res)
}

#[tauri::command]
pub async fn build_revoke_draft(
    state: State<'_, AppState>,
    name: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = load_ctx(&state)?;
    let rate = self::fee_rate(&ctx, fee_rate);
    let nh = names::hash_name(&name)?;
    let (coin, ns) = owner_coin_and_state(&state, &ctx, &name).await?;
    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(name_input_from(coin.clone())),
        PrimaryOutput {
            value: coin.value,
            address: coin.address.clone(),
            covenant: covenants::revoke(&nh, ns.height),
        },
        &ctx.funding,
        &ctx.change_address,
        rate,
    )?;
    persist(&state, &ctx.profile_id, "revoke", &name, None, &res)
}
