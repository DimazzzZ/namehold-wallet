//! Secure, Rust-owned wallet lifecycle commands.
//!
//! These commands own all secret ingress/egress. Mnemonics and passphrases are
//! collected/displayed ONLY through the Rust-controlled secure window (see
//! `commands::secure_prompt`); they are never parameters or return values of a
//! React-invoked command. The frontend triggers a flow and receives a
//! secret-free [`WalletProfileSummary`] / [`SignerSessionSummary`] or an error.
//!
//! Tests drive these commands on the mock runtime through the canned answers
//! of `secure_prompt::push_test_answer`. What they cannot reach is a real
//! secure window (a human typing into a native webview) and a `LedgerSigner`
//! (real USB-HID hardware). The pure pieces of wallet lifecycle (key
//! derivation, encryption, vault serialization, session state) live in
//! `noncustodial::{hd,vault,session,derivation,cookie_vault}`.

use std::fmt::Write as _;

use rand::RngCore;
use tauri::{AppHandle, Manager, Runtime};
use zeroize::Zeroizing;

use crate::commands::secure_prompt::{prompt_secure, ImportInput, SecurePromptRequest};
use crate::db;
use crate::error::AppError;
use crate::noncustodial::hd::{self, ExtendedPrivKey, ExtendedPubKey, HARDENED_OFFSET};
use crate::noncustodial::network::Network;
use crate::noncustodial::session::SignerSession;
use crate::noncustodial::types::{SignerSessionSummary, WalletProfileSummary};
use crate::noncustodial::{derivation, vault};
use crate::providers::ledger::LedgerSigner;
use crate::AppState;

// --- small helpers ---------------------------------------------------------

/// 16-byte random hex id for profiles.
pub(crate) fn random_id() -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Validate the requested profile network against the `wallet_profiles.network`
/// CHECK constraint and map it to a [`Network`].
pub(crate) fn validate_network(s: &str) -> Result<(&'static str, Network), AppError> {
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
pub(crate) fn fingerprint(account_xpub: &str) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(account_xpub.as_bytes());
    hex::encode(&d[..8])
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn read_settings<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<std::collections::HashMap<String, String>, AppError> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::get_settings(&conn)
}

/// The largest `address_gap_limit` honoured; a larger value is clamped.
/// The setting is written by the renderer (SECURITY.md §1) and every node
/// sync derives `gap` addresses per branch past the highest used one and
/// asks the node about each (`GET /tx/address`), so an unbounded value
/// would derive billions of keys and flood the node. 1000 is fifty times
/// the BIP44 default and as far as `OWN_FILE_RECEIVE_SEARCH_LIMIT` looks.
pub(crate) const MAX_GAP_LIMIT: u32 = 1000;

/// The BIP44 default gap.
const DEFAULT_GAP_LIMIT: u32 = 20;

/// Set once the first clamp of a stored gap limit has been logged.
static CLAMP_LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Log, once per `flag`, that a stored gap limit above the cap is being
/// clamped. `gap_limit` runs on every sync, so a line per call would flood
/// the log. Returns whether this call logged.
pub(crate) fn log_clamp_once(flag: &std::sync::atomic::AtomicBool, stored: u32) -> bool {
    if flag.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    eprintln!(
        "warning: stored address_gap_limit {stored} exceeds the cap {MAX_GAP_LIMIT}; using {MAX_GAP_LIMIT}"
    );
    true
}

/// `address_gap_limit`, in `1..=MAX_GAP_LIMIT`.
pub(crate) fn gap_limit(settings: &std::collections::HashMap<String, String>) -> u32 {
    gap_limit_logging(settings, &CLAMP_LOGGED)
}

/// `gap_limit` with the once-flag of the clamp log passed in, so a test can
/// see whether a call flagged (logged) a clamp without sharing process state.
pub(crate) fn gap_limit_logging(
    settings: &std::collections::HashMap<String, String>,
    clamp_flag: &std::sync::atomic::AtomicBool,
) -> u32 {
    let stored = settings
        .get("address_gap_limit")
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|n| *n > 0)
        // Why: absent, zero or not a number is no gap the user chose; the
        // default is what a fresh install provisions with.
        .unwrap_or(DEFAULT_GAP_LIMIT);
    if stored > MAX_GAP_LIMIT {
        log_clamp_once(clamp_flag, stored);
    }
    stored.min(MAX_GAP_LIMIT)
}

pub(crate) use crate::noncustodial::session::session_ttl_ms;

/// Derive the BIP44 account-level xpub string `m/44'/coin'/account'`.
pub(crate) fn account_xpub_from_seed(
    network: Network,
    seed: &[u8],
    account: u32,
) -> Result<String, AppError> {
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
pub(crate) fn provision_addresses(
    conn: &rusqlite::Connection,
    profile_id: &str,
    network: Network,
    account_xpub: &str,
    gap: u32,
) -> Result<String, AppError> {
    let xpub = ExtendedPubKey::from_xpub(network, account_xpub)?;
    let receive = derivation::ensure_addresses(
        conn,
        profile_id,
        0,
        network,
        &xpub,
        derivation::BRANCH_RECEIVE,
        gap,
    )?;
    derivation::ensure_addresses(
        conn,
        profile_id,
        0,
        network,
        &xpub,
        derivation::BRANCH_CHANGE,
        gap,
    )?;
    Ok(receive
        .first()
        .map(|d| d.address.clone())
        .unwrap_or_default())
}

/// Device-local key used to encrypt the seed when the user opts out of a
/// passphrase. The seed is still encrypted at rest (not plaintext), but offers
/// NO protection against someone with read access to this device's files:
/// this constant is in the public source. A new mainnet wallet therefore
/// cannot opt out ([`resolve_secret_key`]); a wallet created before that rule
/// keeps opening with it.
const NO_PASSPHRASE_KEY: &str = "namehold::no-passphrase::v1";

/// The shortest passphrase a new mainnet wallet accepts. The passphrase is
/// all that stands between a copy of the database (a backup, another
/// program running as the user) and the seed, and Argon2id slows each guess
/// but cannot make a handful of characters hard to guess.
pub(crate) const MIN_MAINNET_PASSPHRASE_CHARS: usize = 8;

/// Why a new mainnet wallet's passphrase was refused, shown in the secure
/// window when it asks again.
pub(crate) const MAINNET_PASSPHRASE_RULE: &str = "A mainnet wallet needs a passphrase of at \
     least 8 characters: without one, anyone who can read this computer's files could take its \
     funds.";

/// Map an entered passphrase to the (encryption key, `kdf` marker). An empty
/// entry means "no passphrase": encrypt under the device-local key, allowed
/// on test networks only. A mainnet wallet needs a passphrase of at least
/// [`MIN_MAINNET_PASSPHRASE_CHARS`] characters.
pub(crate) fn resolve_secret_key(
    entered: &str,
    network: Network,
) -> Result<(Zeroizing<String>, &'static str), AppError> {
    if network == Network::Main && entered.chars().count() < MIN_MAINNET_PASSPHRASE_CHARS {
        return Err(AppError::InvalidInput(MAINNET_PASSPHRASE_RULE.to_string()));
    }
    if entered.is_empty() {
        Ok((Zeroizing::new(NO_PASSPHRASE_KEY.to_string()), "none"))
    } else {
        Ok((Zeroizing::new(entered.to_string()), "argon2id"))
    }
}

/// The passphrase prompt's message for a new wallet on `network`.
fn new_passphrase_message(network: Network) -> &'static str {
    if network == Network::Main {
        "Choose a passphrase to encrypt this wallet on this device (at least 8 characters). \
         You will type it to unlock the wallet."
    } else {
        "Choose a passphrase to encrypt this wallet on this device, or leave blank for none \
         (anyone who can read this device's files could then use this test wallet)."
    }
}

/// Ask for a new wallet's passphrase until it meets [`resolve_secret_key`]'s
/// rule for `network` or the user cancels.
#[cfg_attr(coverage_nightly, coverage(off))]
async fn ask_new_passphrase<R: Runtime>(
    app: &AppHandle<R>,
    network: Network,
) -> Result<(Zeroizing<String>, &'static str), AppError> {
    let mut message = new_passphrase_message(network).to_string();
    loop {
        let entered = ask_passphrase(app, true, &message).await?;
        match resolve_secret_key(&entered, network) {
            Ok(key) => return Ok(key),
            Err(_) => message = MAINNET_PASSPHRASE_RULE.to_string(),
        }
    }
}

/// Make `id` the active wallet profile and drop a signer session that belongs
/// to another profile, so stale key material never lingers under a different
/// wallet (the UI would show "Unlocked" while every use of the key is refused).
/// The one activation path: create, every import kind, Ledger and the profile
/// switcher all go through it. The caller must not hold the database lock.
pub(crate) fn activate_profile(state: &AppState, id: &str) -> Result<(), AppError> {
    {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        db::queries::set_active_profile(&conn, id)?;
    }
    drop_signer_if(state, |owner| owner != id)
}

/// Drop the signer session when it belongs to a profile `drop_for` picks
/// (by its profile id); no session, nothing to drop. Shared by the profile
/// activation (another profile's signer) and deletion (the deleted one's).
fn drop_signer_if(state: &AppState, drop_for: impl FnOnce(&str) -> bool) -> Result<(), AppError> {
    let mut slot = state
        .signer
        .lock()
        .map_err(|e| AppError::Lock(e.to_string()))?;
    if slot
        .as_ref()
        .is_some_and(|s| drop_for(s.wallet_profile_id()))
    {
        *slot = None;
    }
    Ok(())
}

/// Room for the longest 12-word phrase: BIP39 English words are at most 8
/// letters, plus the spaces.
const PHRASE_CAPACITY: usize = 12 * 9;

/// A decrypted vault's recovery phrase, read in place so no second copy of it
/// exists to wipe.
fn stored_phrase(plaintext: &[u8]) -> Result<&str, AppError> {
    std::str::from_utf8(plaintext)
        .map_err(|_| AppError::Crypto("stored secret is not valid UTF-8".to_string()))
}

/// The secure prompt that asks for a recovery phrase to import.
pub(crate) fn phrase_import_request() -> SecurePromptRequest {
    SecurePromptRequest {
        mode: "import".to_string(),
        title: "Import recovery phrase".to_string(),
        message: "Enter your 12 or 24 word recovery phrase.".to_string(),
        input: Some(ImportInput::RecoveryPhrase),
        ..Default::default()
    }
}

/// The secure prompt that asks for an account xpub to import. It names the
/// input kind so the window shows an xpub hint and error, not the phrase ones.
pub(crate) fn xpub_import_request() -> SecurePromptRequest {
    SecurePromptRequest {
        mode: "import".to_string(),
        title: "Import account xpub".to_string(),
        message: "Enter the account-level extended public key (xpub).".to_string(),
        input: Some(ImportInput::Xpub),
        ..Default::default()
    }
}

/// Prompt the user for a passphrase via the secure window. `new` shows a confirm
/// field and ALLOWS an empty value (opt out of a passphrase); the unlock prompt
/// (`new = false`) requires a non-empty value. Errors only on cancel.
#[cfg_attr(coverage_nightly, coverage(off))]
async fn ask_passphrase<R: Runtime>(
    app: &AppHandle<R>,
    new: bool,
    message: &str,
) -> Result<Zeroizing<String>, AppError> {
    let mode = if new { "passphrase_new" } else { "passphrase" };
    let res = prompt_secure(
        app,
        SecurePromptRequest {
            mode: mode.to_string(),
            title: "Wallet passphrase".to_string(),
            message: message.to_string(),
            payload: None,
            ..Default::default()
        },
    )
    .await?;
    if !res.confirmed {
        return Err(AppError::InvalidInput(
            "passphrase entry cancelled".to_string(),
        ));
    }
    Ok(res.value.unwrap_or_default())
}

/// The key a stored recovery phrase was encrypted under: the fixed device key
/// for a `kdf = "none"` wallet, otherwise what the user types when asked
/// with `message`.
#[cfg_attr(coverage_nightly, coverage(off))]
async fn stored_passphrase<R: Runtime>(
    app: &AppHandle<R>,
    kdf: &str,
    message: &str,
) -> Result<Zeroizing<String>, AppError> {
    if kdf == "none" {
        Ok(Zeroizing::new(NO_PASSPHRASE_KEY.to_string()))
    } else {
        ask_passphrase(app, false, message).await
    }
}

/// What the reveal window says when it shows a stored recovery phrase.
const REVEAL_MESSAGE: &str =
    "Write these words down in order and keep them offline. Anyone with this phrase can spend \
     your funds.";

/// What the reveal window says when it shows a new wallet's recovery phrase.
const CREATE_REVEAL_MESSAGE: &str =
    "Write these words down in order and keep them offline. Anyone with this phrase can spend \
     your funds. The wallet is created when you press Done; closing this window creates nothing.";

/// Said when a new wallet's reveal window was closed without the user
/// confirming that the phrase is written down.
pub(crate) const CREATE_NOT_CONFIRMED: &str = "The wallet was not created: you did not confirm \
     that the recovery phrase is written down. Nothing was saved.";

/// Display a mnemonic in the secure window for backup. Returns whether the user
/// confirmed they backed it up.
#[cfg_attr(coverage_nightly, coverage(off))]
async fn reveal_mnemonic<R: Runtime>(
    app: &AppHandle<R>,
    phrase: &str,
    message: &str,
) -> Result<bool, AppError> {
    let res = prompt_secure(
        app,
        SecurePromptRequest {
            mode: "reveal".to_string(),
            title: "Back up your recovery phrase".to_string(),
            message: message.to_string(),
            payload: Some(Zeroizing::new(phrase.to_string())),
            ..Default::default()
        },
    )
    .await?;
    Ok(res.confirmed)
}

// --- commands --------------------------------------------------------------

/// Create a brand-new hot wallet. The mnemonic is generated, used, and revealed
/// entirely in the backend / secure window — React never sees it. Nothing is
/// stored until the user confirmed in the reveal window that the phrase is
/// written down; a reveal closed without that creates nothing and says so.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn secure_create_wallet<R: Runtime>(
    app: AppHandle<R>,
    label: String,
    network: String,
) -> Result<WalletProfileSummary, AppError> {
    let (network_str, net) = validate_network(&network)?;
    let settings = read_settings(&app)?;
    let gap = gap_limit(&settings);

    // 1. Collect a new passphrase (with confirmation) in the secure window.
    //    On a test network, leaving it blank opts out of a passphrase (seed
    //    encrypted under a device-local key; no prompt to unlock); a mainnet
    //    wallet must have one.
    let (enc_pass, kdf) = ask_new_passphrase(&app, net).await?;

    // 2. Generate a 12-word mnemonic from fresh entropy.
    let mut entropy = Zeroizing::new([0u8; 16]);
    rand::rng().fill_bytes(&mut *entropy);
    let mnemonic = bip39::Mnemonic::from_entropy(&*entropy)
        .map_err(|e| AppError::Crypto(format!("mnemonic generation: {e}")))?;
    drop(entropy);
    // Sized up front: a String that grew would leave its smaller buffers,
    // with the first words in them, unwiped.
    let mut phrase = Zeroizing::new(String::with_capacity(PHRASE_CAPACITY));
    write!(phrase, "{mnemonic}").map_err(|e| AppError::Crypto(format!("mnemonic text: {e}")))?;
    drop(mnemonic);

    // 3. Derive the account xpub.
    let seed = Zeroizing::new(hd::seed_from_mnemonic(&phrase, "")?);
    let account_xpub = account_xpub_from_seed(net, &seed[..], 0)?;
    drop(seed);

    // 4. Reveal for backup. Only the user's confirmation lets the wallet exist.
    if !reveal_mnemonic(&app, &phrase, CREATE_REVEAL_MESSAGE).await? {
        return Err(AppError::Other(CREATE_NOT_CONFIRMED.to_string()));
    }

    // 5. Encrypt the mnemonic at rest; persist profile + secret + initial
    //    addresses, all or none, and set active.
    let fp = fingerprint(&account_xpub);
    let vault_blob = vault::encrypt(phrase.as_bytes(), &enc_pass)?;
    drop(phrase);
    let id = random_id();
    {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let tx = conn.unchecked_transaction()?;
        db::queries::insert_wallet_profile(
            &tx,
            &id,
            &label,
            "mnemonic_hot",
            network_str,
            &account_xpub,
            0,
            false,
        )?;
        db::queries::insert_wallet_secret(&tx, &id, &vault_blob, kdf, &fp)?;
        let receive_addr = provision_addresses(&tx, &id, net, &account_xpub, gap)?;
        db::queries::update_profile_receive(&tx, &id, &receive_addr, gap as i64)?;
        db::queries::update_profile_change_depth(&tx, &id, gap as i64)?;
        tx.commit()?;
    }
    activate_profile(&app.state::<AppState>(), &id)?;

    load_profile(&app, &id)
}

/// Import an existing wallet. `kind` is `mnemonic_hot` or `watch_only_xpub`.
/// The mnemonic / xpub is entered in the secure window.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn secure_import_wallet<R: Runtime>(
    app: AppHandle<R>,
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
            let res = prompt_secure(&app, phrase_import_request()).await?;
            let phrase = match res.value {
                Some(v) if res.confirmed && !v.is_empty() => v,
                _ => return Err(AppError::InvalidInput("import cancelled".to_string())),
            };
            // Validate by deriving the seed (parses the mnemonic).
            let seed = Zeroizing::new(hd::seed_from_mnemonic(&phrase, "")?);
            let account_xpub = account_xpub_from_seed(net, &seed[..], 0)?;
            drop(seed);

            let (enc_pass, kdf) = ask_new_passphrase(&app, net).await?;
            let fp = fingerprint(&account_xpub);
            let vault_blob = vault::encrypt(phrase.as_bytes(), &enc_pass)?;
            drop(phrase);

            let state = app.state::<AppState>();
            let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
            db::queries::insert_wallet_profile(
                &conn,
                &id,
                &label,
                "mnemonic_hot",
                network_str,
                &account_xpub,
                0,
                false,
            )?;
            db::queries::insert_wallet_secret(&conn, &id, &vault_blob, kdf, &fp)?;
            let receive_addr = provision_addresses(&conn, &id, net, &account_xpub, gap)?;
            db::queries::update_profile_receive(&conn, &id, &receive_addr, gap as i64)?;
            db::queries::update_profile_change_depth(&conn, &id, gap as i64)?;
            drop(conn);
            activate_profile(&state, &id)?;
        }
        "watch_only_xpub" => {
            // The xpub is public; still entered via the secure surface for a
            // consistent UX, but no passphrase / secret is stored.
            let res = prompt_secure(&app, xpub_import_request()).await?;
            let xpub = match res.value {
                Some(v) if res.confirmed && !v.is_empty() => v.trim().to_string(),
                _ => return Err(AppError::InvalidInput("import cancelled".to_string())),
            };
            // Validate it parses for this network.
            ExtendedPubKey::from_xpub(net, &xpub)?;

            let state = app.state::<AppState>();
            let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
            db::queries::insert_wallet_profile(
                &conn,
                &id,
                &label,
                "watch_only_xpub",
                network_str,
                &xpub,
                0,
                true,
            )?;
            let receive_addr = provision_addresses(&conn, &id, net, &xpub, gap)?;
            db::queries::update_profile_receive(&conn, &id, &receive_addr, gap as i64)?;
            db::queries::update_profile_change_depth(&conn, &id, gap as i64)?;
            drop(conn);
            activate_profile(&state, &id)?;
        }
        other => {
            return Err(AppError::InvalidInput(format!(
                "unsupported import kind '{other}'"
            )))
        }
    }

    load_profile(&app, &id)
}

/// Import a Ledger Nano S/X hardware wallet profile.
///
/// Connects to the first plugged-in Ledger device, verifies the Handshake app
/// is open, fetches the account-level extended public key from the device, and
/// creates a `ledger_hardware` profile. The device's private keys never leave
/// the device; this wallet can only sign via on-device APDU requests.
///
/// The device must have the official `handshake-org/ledger-app-hns` app
/// installed and open (dev-mode sideload; not yet in Ledger Live).
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn import_ledger_profile(
    app: AppHandle,
    label: String,
    network: String,
) -> Result<WalletProfileSummary, AppError> {
    let (network_str, net) = validate_network(&network)?;
    let settings = read_settings(&app)?;
    let gap = gap_limit(&settings);
    let id = random_id();

    // Connect to device and fetch account xpub. This is blocking HID I/O, so
    // we run it in a blocking task to avoid stalling the async runtime.
    let (pubkey, chain_code) = tokio::task::spawn_blocking(move || {
        let mut signer = LedgerSigner::connect()?;
        // Verify the app is running and reachable.
        let version = signer.get_app_version()?;
        if version.major != 1 || version.minor < 6 {
            return Err(AppError::Device(format!(
                "unsupported Ledger app version {version} (need 1.6+)"
            )));
        }
        // Fetch the account-level pubkey + chain code (with user confirmation).
        signer.get_account_pubkey(net, 0, /* confirm */ true)
    })
    .await
    .map_err(|e| AppError::Other(format!("ledger task failed: {e}")))??;

    // Build the account xpub from the device's pubkey + chain code.
    let account_xpub = ExtendedPubKey::from_parts(&pubkey, &chain_code)?.to_base58check(net);

    // Create the profile (no secret, like watch-only).
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::insert_wallet_profile(
        &conn,
        &id,
        &label,
        "ledger_hardware",
        network_str,
        &account_xpub,
        0,
        false, // watch_only=false (can spend via device)
    )?;
    let receive_addr = provision_addresses(&conn, &id, net, &account_xpub, gap)?;
    db::queries::update_profile_receive(&conn, &id, &receive_addr, gap as i64)?;
    db::queries::update_profile_change_depth(&conn, &id, gap as i64)?;
    drop(conn);
    activate_profile(&state, &id)?;

    load_profile(&app, &id)
}

/// Re-display an existing wallet's recovery phrase after passphrase entry.
/// Returns nothing — the phrase is shown only in the secure window.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn secure_reveal_backup_phrase<R: Runtime>(
    app: AppHandle<R>,
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

    let passphrase = stored_passphrase(
        &app,
        &kdf,
        "Enter your wallet passphrase to reveal the phrase.",
    )
    .await?;
    let plaintext = Zeroizing::new(vault::decrypt(&blob, &passphrase)?);
    let phrase = stored_phrase(&plaintext)?;

    reveal_mnemonic(&app, phrase, REVEAL_MESSAGE).await?;
    Ok(())
}

/// Unlock the local signer for a profile by decrypting its seed into memory.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn unlock_local_signer<R: Runtime>(
    app: AppHandle<R>,
    wallet_profile_id: String,
) -> Result<SignerSessionSummary, AppError> {
    let settings = read_settings(&app)?;
    let ttl_ms = session_ttl_ms(&settings);

    // Load profile network + encrypted blob + kdf marker (lock dropped before prompt).
    let (network, secret, profile_kind) = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let profile = db::queries::get_wallet_profile(&conn, &wallet_profile_id)?
            .ok_or_else(|| AppError::NotFound(format!("wallet profile {wallet_profile_id}")))?;
        let net = derivation::network_from_profile(&profile.network)?;
        let secret = db::queries::get_wallet_secret_meta(&conn, &wallet_profile_id)?;
        (net, secret, profile.kind.clone())
    };
    let (blob, kdf) = secret.ok_or_else(|| {
        let msg = if profile_kind == "ledger_hardware" {
            "Ledger profiles are always watch-only and do not require unlocking"
        } else {
            "cannot unlock a watch-only profile"
        };
        AppError::InvalidInput(msg.to_string())
    })?;

    let passphrase =
        stored_passphrase(&app, &kdf, "Enter your wallet passphrase to unlock.").await?;
    let plaintext = Zeroizing::new(vault::decrypt(&blob, &passphrase)?);
    let seed = Zeroizing::new(hd::seed_from_mnemonic(stored_phrase(&plaintext)?, "")?);
    let master = ExtendedPrivKey::from_seed(&seed[..])?;
    drop(seed);

    let mut session = SignerSession::unlock(wallet_profile_id.clone(), network, master, ttl_ms);
    let summary = SignerSessionSummary {
        wallet_profile_id: Some(wallet_profile_id),
        unlocked: session.is_unlocked(),
        unlocked_until_epoch_ms: session.unlocked_until_ms() as i64,
    };
    {
        let state = app.state::<AppState>();
        let mut slot = state
            .signer
            .lock()
            .map_err(|e| AppError::Lock(e.to_string()))?;
        *slot = Some(session);
    }
    Ok(summary)
}

/// Lock the local signer. Dropping the session drops the key in place,
/// which overwrites its secret key and chain code.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn lock_local_signer(state: tauri::State<'_, AppState>) -> Result<(), AppError> {
    let mut slot = state
        .signer
        .lock()
        .map_err(|e| AppError::Lock(e.to_string()))?;
    *slot = None;
    Ok(())
}

/// Drop the signer session once it has expired, so its key does not stay in
/// memory until the next command looks at it. `lib.rs` runs this every
/// second. It never waits: while a command holds the signer (signing), this
/// tick is skipped. Returns whether a session was dropped.
pub(crate) fn sweep_expired_signer(state: &AppState) -> bool {
    match state.signer.try_lock() {
        Ok(mut slot) => crate::noncustodial::session::expire_slot(&mut slot),
        // Why: a command is using the signer and checks the expiry itself.
        Err(std::sync::TryLockError::WouldBlock) => false,
        // Why: a poisoned lock stays poisoned, so no command can use or lock
        // this signer again; only this drops the key before the app exits.
        Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner().take().is_some(),
    }
}

/// Current signer session state (secret-free). An expired session is
/// dropped here, key and all.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn get_signer_session(
    state: tauri::State<'_, AppState>,
) -> Result<SignerSessionSummary, AppError> {
    let mut slot = state
        .signer
        .lock()
        .map_err(|e| AppError::Lock(e.to_string()))?;
    // A session `expire_slot` leaves in the slot is an unlocked one.
    crate::noncustodial::session::expire_slot(&mut slot);
    match slot.as_ref() {
        Some(s) => Ok(SignerSessionSummary {
            wallet_profile_id: Some(s.wallet_profile_id().to_string()),
            unlocked: true,
            unlocked_until_epoch_ms: s.unlocked_until_ms() as i64,
        }),
        None => Ok(SignerSessionSummary::locked()),
    }
}

/// List all wallet profiles.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn list_wallet_profiles(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<WalletProfileSummary>, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::list_wallet_profiles(&conn)
}

/// Set the active wallet profile. Switching away from the unlocked profile locks
/// the signer so stale key material is never used for a different wallet.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn set_active_wallet_profile(
    state: tauri::State<'_, AppState>,
    wallet_profile_id: String,
) -> Result<WalletProfileSummary, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let profile = db::queries::get_wallet_profile(&conn, &wallet_profile_id)?
        .ok_or_else(|| AppError::NotFound(format!("wallet profile {wallet_profile_id}")))?;
    drop(conn);
    activate_profile(&state, &wallet_profile_id)?;

    Ok(WalletProfileSummary {
        active: true,
        ..profile
    })
}

/// The Warning row of a delete that erases a stored recovery phrase.
pub(crate) const DELETE_SEED_WARNING: &str = "This erases the encrypted recovery phrase stored on \
     this device. Without the recovery phrase written down, this wallet's funds and names are lost \
     for good.";

/// Said when a profile gained or lost its stored recovery phrase while the
/// delete was being confirmed: what the user confirmed is no longer what
/// would be deleted.
pub(crate) const DELETE_PROFILE_CHANGED: &str =
    "This wallet changed while you were asked to confirm. Nothing was deleted; try again.";

/// A profile's kind in the words the wallet list uses (`walletKindLabel`).
fn kind_label(profile: &WalletProfileSummary) -> &'static str {
    if profile.kind == "ledger_hardware" {
        "Ledger"
    } else if profile.watch_only {
        "Watch-only"
    } else {
        "Hot"
    }
}

/// What the secure window shows before `profile` is deleted, as `details`
/// for a `confirm` prompt: its label, network, kind and fingerprint, and
/// "Delete wallet" on the button. A profile whose encrypted recovery phrase
/// is stored on this device (`has_stored_seed`) also gets a Warning and asks
/// for its label to be typed (`confirmText`; its fingerprint when the label
/// is blank), because deleting it erases the only copy the device holds. A
/// watch-only or Ledger profile holds no secret and is confirmed with a click.
pub(crate) fn delete_profile_details(
    profile: &WalletProfileSummary,
    has_stored_seed: bool,
) -> serde_json::Value {
    let fp = fingerprint(&profile.account_xpub);
    let mut rows = vec![
        serde_json::json!({ "label": "Action", "value": "Delete this wallet from this device" }),
        serde_json::json!({ "label": "Wallet", "value": profile.label }),
        serde_json::json!({ "label": "Network", "value": profile.network }),
        serde_json::json!({ "label": "Kind", "value": kind_label(profile) }),
        serde_json::json!({ "label": "Fingerprint", "value": fp }),
    ];
    if has_stored_seed {
        rows.push(serde_json::json!({ "label": "Warning", "value": DELETE_SEED_WARNING }));
    }
    let mut details = serde_json::json!({ "rows": rows, "confirmLabel": "Delete wallet" });
    if has_stored_seed {
        let label = profile.label.trim();
        let typed = if label.is_empty() {
            fp
        } else {
            label.to_string()
        };
        details["confirmText"] = serde_json::json!(typed);
    }
    details
}

/// Delete a wallet profile and all its data, once the user has confirmed it
/// in the secure window ([`delete_profile_details`]); a compromised webview
/// cannot erase a stored recovery phrase with a call. Declined, closed, or
/// confirmed without the typed label it asks for: `UserRejected`, nothing
/// deleted. If it was the active profile, the active selection is cleared
/// and the signer is locked.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn delete_wallet_profile<R: Runtime>(
    app: AppHandle<R>,
    wallet_profile_id: String,
) -> Result<(), AppError> {
    let state = app.state::<AppState>();
    // Read what the window shows; the lock is dropped before the prompt.
    let (profile, has_stored_seed) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let profile = db::queries::get_wallet_profile(&conn, &wallet_profile_id)?
            .ok_or_else(|| AppError::NotFound(format!("wallet profile {wallet_profile_id}")))?;
        let has_stored_seed =
            db::queries::get_wallet_secret_meta(&conn, &wallet_profile_id)?.is_some();
        (profile, has_stored_seed)
    };
    let details = delete_profile_details(&profile, has_stored_seed);
    let typed = details
        .get("confirmText")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let answer = prompt_secure(
        &app,
        SecurePromptRequest {
            mode: "confirm".to_string(),
            title: "Delete wallet".to_string(),
            message: "Check that this is the wallet to delete. This cannot be undone.".to_string(),
            details: Some(details),
            ..Default::default()
        },
    )
    .await?;
    if !answer.confirmed {
        return Err(AppError::UserRejected);
    }
    // The backend checks what was typed, not only the window's own check.
    if let Some(expected) = typed {
        if answer.value.as_ref().map(|v| v.trim()) != Some(expected.as_str()) {
            return Err(AppError::UserRejected);
        }
    }
    {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        if db::queries::get_wallet_secret_meta(&conn, &wallet_profile_id)?.is_some()
            != has_stored_seed
        {
            return Err(AppError::Other(DELETE_PROFILE_CHANGED.to_string()));
        }
        let active = db::queries::get_active_profile_id(&conn)?;
        db::queries::delete_wallet_profile(&conn, &wallet_profile_id)?;
        if active == wallet_profile_id {
            db::queries::set_active_profile(&conn, "")?;
        }
    }
    drop_signer_if(&state, |owner| owner == wallet_profile_id)
}

/// Read a profile summary by id (helper for command returns).
#[cfg_attr(coverage_nightly, coverage(off))]
fn load_profile<R: Runtime>(
    app: &AppHandle<R>,
    id: &str,
) -> Result<WalletProfileSummary, AppError> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    db::queries::get_wallet_profile(&conn, id)?
        .ok_or_else(|| AppError::NotFound(format!("wallet profile {id}")))
}
