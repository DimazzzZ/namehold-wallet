use crate::commands::tx::refuse_unsupported_ledger_plan;
use crate::error::AppError;

const PLAIN: &str =
    r#"{"version":0,"locktime":0,"account":0,"network":"main","inputs":[],"outputs":[]}"#;
const FOREIGN: &str = r#"{"version":0,"locktime":2147483700,"account":0,"network":"main",
  "inputs":[{"txid":"0909090909090909090909090909090909090909090909090909090909090909","vout":0,
  "value":0,"branch":0,"child_index":0,"sighash_type":132,"sequence":4294967294,
  "foreign_witness_hex":["aa"]}],"outputs":[]}"#;
/// A cancel's lock coin of ours, signed by its lock key: final sequence, no
/// lock time, nothing foreign.
const LOCK_KEY: &str = r#"{"version":0,"locktime":0,"account":0,"network":"main",
  "inputs":[{"txid":"0909090909090909090909090909090909090909090909090909090909090909","vout":0,
  "value":0,"branch":0,"child_index":11,"sighash_type":131,"sequence":4294967295,
  "lock_key_name":"dexreviews"}],"outputs":[]}"#;
/// A `send_hns` draft stores its build parameters, not a plan; the signer
/// builds the plan from the wallet's own coins.
const SEND_PARAMS: &str = r#"{"network":"main","account":0,"to_address":"hs1q","amount_doos":1}"#;

#[test]
fn ledger_refuses_plan_with_foreign_input() {
    let err = refuse_unsupported_ledger_plan("ledger_hardware", "register", FOREIGN).unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(m) if m.contains("recovery-phrase wallet")));
}

#[test]
fn ledger_refuses_plan_with_lock_key_input() {
    let err = refuse_unsupported_ledger_plan("ledger_hardware", "register", LOCK_KEY).unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(m) if m.contains("recovery-phrase wallet")));
}

#[test]
fn ledger_accepts_ordinary_plan_and_hot_accepts_anything() {
    refuse_unsupported_ledger_plan("ledger_hardware", "register", PLAIN).unwrap();
    refuse_unsupported_ledger_plan("mnemonic_hot", "shakedex_purchase", FOREIGN).unwrap();
}

#[test]
fn ledger_refuses_a_covenant_plan_it_cannot_read() {
    let err =
        refuse_unsupported_ledger_plan("ledger_hardware", "register", "not a plan").unwrap_err();
    assert!(matches!(err, AppError::Other(m) if m.contains("corrupted draft")));
}

#[test]
fn ledger_send_hns_params_are_not_a_plan_and_pass() {
    refuse_unsupported_ledger_plan("ledger_hardware", "send_hns", SEND_PARAMS).unwrap();
}

fn app_with_ledger_draft(plan_json: &str) -> tauri::App<tauri::test::MockRuntime> {
    use tauri::test::{mock_builder, mock_context, noop_assets};

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::migrations::run(&conn).unwrap();
    crate::db::queries::insert_wallet_profile(
        &conn,
        "ledger_profile",
        "Ledger",
        "ledger_hardware",
        "mainnet",
        "xpub-not-read-before-the-guard",
        0,
        false,
    )
    .unwrap();
    crate::db::queries::insert_tx_draft(
        &conn,
        "draft",
        "ledger_profile",
        "register",
        "00",
        plan_json,
        "{}",
    )
    .unwrap();
    mock_builder()
        .manage(crate::AppState {
            db: std::sync::Mutex::new(conn),
            signer: std::sync::Mutex::new(None),
            secure_prompts: std::sync::Mutex::new(std::collections::HashMap::new()),
            hsd_child: std::sync::Mutex::new(None),
            node_rpc_alive: std::sync::atomic::AtomicBool::new(false),
            sync_status: std::sync::Arc::new(tokio::sync::Mutex::new(
                crate::commands::sync::SyncStatus::default(),
            )),
        })
        .build(mock_context(noop_assets()))
        .expect("mock app")
}

/// The guard is wired into the signing path itself, not only callable: a
/// Ledger draft carrying a foreign plan never gets past loading the draft.
#[tokio::test]
async fn signing_a_ledger_draft_with_a_foreign_plan_is_refused() {
    use tauri::Manager;

    let app = app_with_ledger_draft(FOREIGN);
    let err = crate::commands::tx::sign_tx_draft_inner(&app.state(), "draft")
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(m) if m.contains("recovery-phrase wallet")));
}

/// An unreadable covenant plan is refused by the guard with its own message,
/// before the covenant names are resolved from it.
#[tokio::test]
async fn signing_a_ledger_draft_with_an_unreadable_plan_is_refused() {
    use tauri::Manager;

    let app = app_with_ledger_draft("not a plan");
    let err = crate::commands::tx::sign_tx_draft_inner(&app.state(), "draft")
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Other(m) if m.contains("unreadable signing plan")));
}
