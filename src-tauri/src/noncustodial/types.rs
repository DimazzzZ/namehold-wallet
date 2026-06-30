//! Frontend-facing, secret-free types for the non-custodial wallet.
//!
//! Everything here is safe to serialize across the Tauri IPC boundary to React.
//! NONE of these structs may carry mnemonics, private keys, seeds, decrypted
//! key material, or bid nonce/blind secrets — those stay backend-only.

use serde::{Deserialize, Serialize};

/// A wallet profile as shown to the frontend. Mirrors `wallet_profiles` minus
/// nothing secret (the table itself holds no secret columns — those live in
/// `wallet_secrets`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletProfileSummary {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub network: String,
    pub account_xpub: String,
    pub account_index: i64,
    pub receive_depth: i64,
    pub change_depth: i64,
    pub receive_address: Option<String>,
    pub last_synced_height: Option<i64>,
    pub last_synced_at: Option<String>,
    pub watch_only: bool,
    /// Whether unlocking this profile requires a passphrase. False when the vault
    /// was created without one (`kdf='none'`, device-local key) — the UI uses
    /// this to drop the "enter your passphrase" copy and unlock in one click.
    pub has_passphrase: bool,
    /// Whether this profile is the active one (matches `active_wallet_profile_id`).
    pub active: bool,
}

/// Non-secret view of the in-memory signer session.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignerSessionSummary {
    /// Active unlocked profile id, or `None` when locked.
    pub wallet_profile_id: Option<String>,
    pub unlocked: bool,
    /// Absolute expiry in epoch milliseconds (0 when locked).
    pub unlocked_until_epoch_ms: i64,
}

impl SignerSessionSummary {
    pub fn locked() -> Self {
        Self {
            wallet_profile_id: None,
            unlocked: false,
            unlocked_until_epoch_ms: 0,
        }
    }
}

/// Human-readable, confirm-before-broadcast summary of a transaction draft.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TxSummary {
    pub action: String,
    pub send_total_doos: i64,
    pub fee_doos: i64,
    pub change_doos: i64,
    pub input_total_doos: i64,
    pub num_inputs: i64,
    pub recipient_address: Option<String>,
    pub txid: Option<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// A persisted transaction draft as shown to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TxDraftSummary {
    pub id: String,
    pub wallet_profile_id: String,
    pub action: String,
    pub status: String,
    /// Parsed `summary_json` (a [`TxSummary`]); kept as a value so a legacy or
    /// partial summary never breaks listing.
    pub summary: serde_json::Value,
    pub error_message: Option<String>,
    pub txid: Option<String>,
    pub created_at: String,
}

/// Result of broadcasting a signed draft.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastResult {
    pub draft_id: String,
    pub txid: String,
    pub status: String,
}
