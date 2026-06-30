//! Secure, Rust-owned wallet lifecycle commands.
//!
//! These commands own all secret ingress/egress. Mnemonics and passphrases are
//! collected/displayed ONLY through the Rust-controlled secure window (see
//! `commands::secure_prompt`); they are never parameters or return values of a
//! React-invoked command. The frontend triggers a flow and receives a
//! secret-free [`WalletProfileSummary`] / [`SignerSessionSummary`] or an error.

use rand::RngCore;
use tauri::{AppHandle, Manager};
use zeroize::Zeroize;

use crate::commands::secure_prompt::{prompt_secure, SecurePromptRequest};
use crate::db;
use crate::error::AppError;
use crate::noncustodial::hd::{self, ExtendedPrivKey, ExtendedPubKey, HARDENED_OFFSET};
use crate::noncustodial::network::Network;
use crate::noncustodial::session::SignerSession;
use crate::noncustodial::types::{SignerSessionSummary, WalletProfileSummary};
use crate::noncustodial::{derivation, vault};
use crate::AppState;

// --- small helpers ---------------------------------------------------------

/// 16-byte random hex id for profiles.
fn random_id() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Validate the requested profile network against the `wallet_profiles.network`
/// CHECK constraint and map it to a [`Network`].
fn validate_network(s: &str) -> Result<(&'static str, Network), AppError> {
    match s {
        "mainnet" => Ok(("mainnet", Network::Main)),
        "testnet" => Ok(("testnet", Network::Testnet)),
        "regtest" => Ok(("regtest", Network::Regtest)),
        other => Err(AppError::InvalidInput(format!(
            "unsupported network '{other}' (expected mainnet|testnet|regtest)"
        ))),
    }
}

/// Non-secret fingerprint of an account xpub (first 8 bytes of its SHA-256).
fn fingerprint(account_xpub: &str) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(account_xpub.as_bytes());
    hex::encode(&d[..8])
}

fn read_settings(app: &AppHandle) -> Result<std::collections::HashMap<String, String>, AppError> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::get_settings(&conn)
}

fn gap_limit(settings: &std::collections::HashMap<String, String>) -> u32 {
    settings
        .get("address_gap_limit")
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(20)
}

fn session_ttl_ms(settings: &std::collections::HashMap<String, String>) -> u128 {
    let secs = settings
        .get("signer_session_timeout_seconds")
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(900);
    (secs as u128) * 1000
}

/// Derive the BIP44 account-level xpub string `m/44'/coin'/account'`.
fn account_xpub_from_seed(network: Network, seed: &[u8], account: u32) -> Result<String, AppError> {
    let master = ExtendedPrivKey::from_seed(seed)?;
    let path = [
        HARDENED_OFFSET + 44,
        HARDENED_OFFSET + network.coin_type(),
        HARDENED_OFFSET + account,
    ];
    let node = master.derive_path(&path)?;
    Ok(ExtendedPubKey::from_priv(&node).to_base58check(network))
}

/// Derive + persist the initial receive/change address windows for a profile.
/// Returns the first receive address (the profile's default receive address).
fn provision_addresses(
    conn: &rusqlite::Connection,
    profile_id: &str,
    network: Network,
    account_xpub: &str,
    gap: u32,
) -> Result<String, AppError> {
    let xpub = ExtendedPubKey::from_xpub(network, account_xpub)?;
    let receive =
        derivation::ensure_addresses(conn, profile_id, 0, network, &xpub, derivation::BRANCH_RECEIVE, gap)?;
    derivation::ensure_addresses(conn, profile_id, 0, network, &xpub, derivation::BRANCH_CHANGE, gap)?;
    Ok(receive
        .first()
        .map(|d| d.address.clone())
        .unwrap_or_default())
}

/// Device-local key used to encrypt the seed when the user opts out of a
/// passphrase. The seed is still encrypted at rest (not plaintext), but offers
/// NO protection against someone with read access to this device's files.
const NO_PASSPHRASE_KEY: &str = "namehold::no-passphrase::v1";

/// Map an entered passphrase to the (encryption key, `kdf` marker). An empty
/// entry means "no passphrase": encrypt under the device-local key.
fn resolve_secret_key(entered: &str) -> (String, &'static str) {
    if entered.is_empty() {
        (NO_PASSPHRASE_KEY.to_string(), "none")
    } else {
        (entered.to_string(), "argon2id")
    }
}

/// Prompt the user for a passphrase via the secure window. `new` shows a confirm
/// field and ALLOWS an empty value (opt out of a passphrase); the unlock prompt
/// (`new = false`) requires a non-empty value. Errors only on cancel.
async fn ask_passphrase(app: &AppHandle, new: bool, message: &str) -> Result<String, AppError> {
    let mode = if new { "passphrase_new" } else { "passphrase" };
    let res = prompt_secure(
        app,
        SecurePromptRequest {
            mode: mode.to_string(),
            title: "Wallet passphrase".to_string(),
            message: message.to_string(),
            payload: None,
        },
    )
    .await?;
    if !res.confirmed {
        return Err(AppError::InvalidInput("passphrase entry cancelled".to_string()));
    }
    Ok(res.value.unwrap_or_default())
}

/// Display a mnemonic in the secure window for backup. Returns whether the user
/// confirmed they backed it up.
async fn reveal_mnemonic(app: &AppHandle, phrase: &str) -> Result<bool, AppError> {
    let res = prompt_secure(
        app,
        SecurePromptRequest {
            mode: "reveal".to_string(),
            title: "Back up your recovery phrase".to_string(),
            message: "Write these words down in order and keep them offline. \
                      Anyone with this phrase can spend your funds."
                .to_string(),
            payload: Some(phrase.to_string()),
        },
    )
    .await?;
    Ok(res.confirmed)
}

// --- commands --------------------------------------------------------------

/// Create a brand-new hot wallet. The mnemonic is generated, used, and revealed
/// entirely in the backend / secure window — React never sees it.
#[tauri::command]
pub async fn secure_create_wallet(
    app: AppHandle,
    label: String,
    network: String,
) -> Result<WalletProfileSummary, AppError> {
    let (network_str, net) = validate_network(&network)?;
    let settings = read_settings(&app)?;
    let gap = gap_limit(&settings);

    // 1. Collect a new passphrase (with confirmation) in the secure window.
    //    Leaving it blank opts out of a passphrase (seed encrypted under a
    //    device-local key; no prompt to unlock).
    let entered = ask_passphrase(
        &app,
        true,
        "Choose a passphrase to encrypt this wallet on this device, or leave blank for none.",
    )
    .await?;
    let (enc_pass, kdf) = resolve_secret_key(&entered);

    // 2. Generate a 12-word mnemonic from fresh entropy.
    let mut entropy = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut entropy);
    let mnemonic = bip39::Mnemonic::from_entropy(&entropy)
        .map_err(|e| AppError::Crypto(format!("mnemonic generation: {e}")))?;
    entropy.zeroize();
    let mut phrase = mnemonic.to_string();

    // 3. Derive the account xpub and encrypt the mnemonic at rest.
    let mut seed = hd::seed_from_mnemonic(&phrase, "")?;
    let account_xpub = account_xpub_from_seed(net, &seed, 0)?;
    seed.zeroize();
    let fp = fingerprint(&account_xpub);
    let vault_blob = vault::encrypt(phrase.as_bytes(), &enc_pass)?;

    // 4. Persist profile + secret + initial addresses, set active.
    let id = random_id();
    {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        db::queries::insert_wallet_profile(
            &conn, &id, &label, "mnemonic_hot", network_str, &account_xpub, 0, false,
        )?;
        db::queries::insert_wallet_secret(&conn, &id, &vault_blob, kdf, &fp)?;
        let receive_addr = provision_addresses(&conn, &id, net, &account_xpub, gap)?;
        db::queries::update_profile_receive(&conn, &id, &receive_addr, gap as i64)?;
        db::queries::update_profile_change_depth(&conn, &id, gap as i64)?;
        db::queries::set_active_profile(&conn, &id)?;
    }

    // 5. Reveal for backup (display only), then wipe the plaintext.
    let _ = reveal_mnemonic(&app, &phrase).await;
    phrase.zeroize();

    load_profile(&app, &id)
}

/// Import an existing wallet. `kind` is `mnemonic_hot` or `watch_only_xpub`.
/// The mnemonic / xpub is entered in the secure window.
#[tauri::command]
pub async fn secure_import_wallet(
    app: AppHandle,
    label: String,
    network: String,
    kind: String,
) -> Result<WalletProfileSummary, AppError> {
    let (network_str, net) = validate_network(&network)?;
    let settings = read_settings(&app)?;
    let gap = gap_limit(&settings);
    let id = random_id();

    match kind.as_str() {
        "mnemonic_hot" => {
            // Enter the recovery phrase in the secure window.
            let res = prompt_secure(
                &app,
                SecurePromptRequest {
                    mode: "import".to_string(),
                    title: "Import recovery phrase".to_string(),
                    message: "Enter your 12 or 24 word recovery phrase.".to_string(),
                    payload: None,
                },
            )
            .await?;
            let mut phrase = match res.value {
                Some(v) if res.confirmed && !v.is_empty() => v,
                _ => return Err(AppError::InvalidInput("import cancelled".to_string())),
            };
            // Validate by deriving the seed (parses the mnemonic).
            let mut seed = hd::seed_from_mnemonic(&phrase, "")?;
            let account_xpub = account_xpub_from_seed(net, &seed, 0)?;
            seed.zeroize();

            let entered = ask_passphrase(
                &app,
                true,
                "Choose a passphrase to encrypt this wallet on this device, or leave blank for none.",
            )
            .await?;
            let (enc_pass, kdf) = resolve_secret_key(&entered);
            let fp = fingerprint(&account_xpub);
            let vault_blob = vault::encrypt(phrase.as_bytes(), &enc_pass)?;
            phrase.zeroize();

            let state = app.state::<AppState>();
            let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
            db::queries::insert_wallet_profile(
                &conn, &id, &label, "mnemonic_hot", network_str, &account_xpub, 0, false,
            )?;
            db::queries::insert_wallet_secret(&conn, &id, &vault_blob, kdf, &fp)?;
            let receive_addr = provision_addresses(&conn, &id, net, &account_xpub, gap)?;
            db::queries::update_profile_receive(&conn, &id, &receive_addr, gap as i64)?;
            db::queries::update_profile_change_depth(&conn, &id, gap as i64)?;
            db::queries::set_active_profile(&conn, &id)?;
        }
        "watch_only_xpub" => {
            // The xpub is public; still entered via the secure surface for a
            // consistent UX, but no passphrase / secret is stored.
            let res = prompt_secure(
                &app,
                SecurePromptRequest {
                    mode: "import".to_string(),
                    title: "Import account xpub".to_string(),
                    message: "Enter the account-level extended public key (xpub).".to_string(),
                    payload: None,
                },
            )
            .await?;
            let xpub = match res.value {
                Some(v) if res.confirmed && !v.is_empty() => v.trim().to_string(),
                _ => return Err(AppError::InvalidInput("import cancelled".to_string())),
            };
            // Validate it parses for this network.
            ExtendedPubKey::from_xpub(net, &xpub)?;

            let state = app.state::<AppState>();
            let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
            db::queries::insert_wallet_profile(
                &conn, &id, &label, "watch_only_xpub", network_str, &xpub, 0, true,
            )?;
            let receive_addr = provision_addresses(&conn, &id, net, &xpub, gap)?;
            db::queries::update_profile_receive(&conn, &id, &receive_addr, gap as i64)?;
            db::queries::update_profile_change_depth(&conn, &id, gap as i64)?;
            db::queries::set_active_profile(&conn, &id)?;
        }
        other => {
            return Err(AppError::InvalidInput(format!(
                "unsupported import kind '{other}'"
            )))
        }
    }

    load_profile(&app, &id)
}

/// Re-display an existing wallet's recovery phrase after passphrase entry.
/// Returns nothing — the phrase is shown only in the secure window.
#[tauri::command]
pub async fn secure_reveal_backup_phrase(
    app: AppHandle,
    wallet_profile_id: String,
) -> Result<(), AppError> {
    // Load the encrypted blob + kdf marker (drop the lock before any prompt).
    let secret = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        db::queries::get_wallet_secret_meta(&conn, &wallet_profile_id)?
    };
    let (blob, kdf) = secret.ok_or_else(|| {
        AppError::InvalidInput("this profile has no recovery phrase (watch-only)".to_string())
    })?;

    let passphrase = if kdf == "none" {
        NO_PASSPHRASE_KEY.to_string()
    } else {
        ask_passphrase(&app, false, "Enter your wallet passphrase to reveal the phrase.").await?
    };
    let mut plaintext = vault::decrypt(&blob, &passphrase)?;
    let mut phrase = String::from_utf8(plaintext.clone())
        .map_err(|_| AppError::Crypto("stored secret is not valid UTF-8".to_string()))?;
    plaintext.zeroize();

    let _ = reveal_mnemonic(&app, &phrase).await;
    phrase.zeroize();
    Ok(())
}

/// Unlock the local signer for a profile by decrypting its seed into memory.
#[tauri::command]
pub async fn unlock_local_signer(
    app: AppHandle,
    wallet_profile_id: String,
) -> Result<SignerSessionSummary, AppError> {
    let settings = read_settings(&app)?;
    let ttl_ms = session_ttl_ms(&settings);

    // Load profile network + encrypted blob + kdf marker (lock dropped before prompt).
    let (network, secret) = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let profile = db::queries::get_wallet_profile(&conn, &wallet_profile_id)?
            .ok_or_else(|| AppError::NotFound(format!("wallet profile {wallet_profile_id}")))?;
        let net = derivation::network_from_profile(&profile.network)?;
        let secret = db::queries::get_wallet_secret_meta(&conn, &wallet_profile_id)?;
        (net, secret)
    };
    let (blob, kdf) = secret.ok_or_else(|| {
        AppError::InvalidInput("cannot unlock a watch-only profile".to_string())
    })?;

    let passphrase = if kdf == "none" {
        NO_PASSPHRASE_KEY.to_string()
    } else {
        ask_passphrase(&app, false, "Enter your wallet passphrase to unlock.").await?
    };
    let mut plaintext = vault::decrypt(&blob, &passphrase)?;
    let mut phrase = String::from_utf8(plaintext.clone())
        .map_err(|_| AppError::Crypto("stored secret is not valid UTF-8".to_string()))?;
    plaintext.zeroize();
    let mut seed = hd::seed_from_mnemonic(&phrase, "")?;
    phrase.zeroize();
    let master = ExtendedPrivKey::from_seed(&seed)?;
    seed.zeroize();

    let session = SignerSession::unlock(wallet_profile_id.clone(), network, master, ttl_ms);
    let summary = SignerSessionSummary {
        wallet_profile_id: Some(wallet_profile_id),
        unlocked: session.is_unlocked(),
        unlocked_until_epoch_ms: session.unlocked_until_ms() as i64,
    };
    {
        let state = app.state::<AppState>();
        let mut slot = state.signer.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        *slot = Some(session);
    }
    Ok(summary)
}

/// Lock the local signer, zeroizing in-memory key material.
#[tauri::command]
pub async fn lock_local_signer(state: tauri::State<'_, AppState>) -> Result<(), AppError> {
    let mut slot = state.signer.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    *slot = None; // Drop zeroizes the key material.
    Ok(())
}

/// Current signer session state (secret-free).
#[tauri::command]
pub async fn get_signer_session(
    state: tauri::State<'_, AppState>,
) -> Result<SignerSessionSummary, AppError> {
    let slot = state.signer.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    match slot.as_ref() {
        Some(s) if s.is_unlocked() => Ok(SignerSessionSummary {
            wallet_profile_id: Some(s.wallet_profile_id().to_string()),
            unlocked: true,
            unlocked_until_epoch_ms: s.unlocked_until_ms() as i64,
        }),
        _ => Ok(SignerSessionSummary::locked()),
    }
}

/// List all wallet profiles.
#[tauri::command]
pub async fn list_wallet_profiles(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<WalletProfileSummary>, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::list_wallet_profiles(&conn)
}

/// Set the active wallet profile. Switching away from the unlocked profile locks
/// the signer so stale key material is never used for a different wallet.
#[tauri::command]
pub async fn set_active_wallet_profile(
    state: tauri::State<'_, AppState>,
    wallet_profile_id: String,
) -> Result<WalletProfileSummary, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let profile = db::queries::get_wallet_profile(&conn, &wallet_profile_id)?
        .ok_or_else(|| AppError::NotFound(format!("wallet profile {wallet_profile_id}")))?;
    db::queries::set_active_profile(&conn, &wallet_profile_id)?;
    drop(conn);

    let mut slot = state.signer.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    if slot
        .as_ref()
        .map(|s| s.wallet_profile_id() != wallet_profile_id)
        .unwrap_or(false)
    {
        *slot = None;
    }
    drop(slot);

    Ok(WalletProfileSummary {
        active: true,
        ..profile
    })
}

/// Delete a wallet profile and all its data. If it was the active profile, the
/// active selection is cleared and the signer is locked.
#[tauri::command]
pub async fn delete_wallet_profile(
    state: tauri::State<'_, AppState>,
    wallet_profile_id: String,
) -> Result<(), AppError> {
    {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let active = db::queries::get_active_profile_id(&conn)?;
        db::queries::delete_wallet_profile(&conn, &wallet_profile_id)?;
        if active == wallet_profile_id {
            db::queries::set_active_profile(&conn, "")?;
        }
    }
    let mut slot = state.signer.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    if slot
        .as_ref()
        .map(|s| s.wallet_profile_id() == wallet_profile_id)
        .unwrap_or(false)
    {
        *slot = None;
    }
    Ok(())
}

/// Read a profile summary by id (helper for command returns).
fn load_profile(app: &AppHandle, id: &str) -> Result<WalletProfileSummary, AppError> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::get_wallet_profile(&conn, id)?
        .ok_or_else(|| AppError::NotFound(format!("wallet profile {id}")))
}
