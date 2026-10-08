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

const CHANGE: &str = "hs1qdhtaj7ws7chd2z2tulrmakqww428myx08d6w3v";
const PAY: &str = "hs1qd42hrldu5yqee58se4uj6xctm7nk28r70e84vx";

fn coin(txid_byte: u8, child: u32) -> crate::noncustodial::send::SpendableCoin {
    crate::noncustodial::send::SpendableCoin {
        txid: hex::encode([txid_byte; 32]),
        vout: 0,
        value: 50_000_000,
        branch: 0,
        child_index: child,
    }
}

fn refused_with_the_r16_reason(action: &str, plan_json: &str) {
    let err = refuse_unsupported_ledger_plan("ledger_hardware", action, plan_json).unwrap_err();
    assert!(
        matches!(&err, AppError::InvalidInput(m) if m == crate::noncustodial::shakedex::RECOVERY_PHRASE_ONLY),
        "{action}: {err:?}"
    );
}

/// R16: the cancel and both FINALIZE plans never reach a Ledger. The cancel
/// (a lock-key input) and its FINALIZE (a fixed witness) are refused by
/// their shape under any action; the FINALIZE into the lock is an ordinary
/// plan the device could sign, refused because it is a Shakedex draft.
#[test]
fn ledger_refuses_cancel_plan() {
    use crate::noncustodial::network::Network;
    use crate::noncustodial::shakedex::cancel::{
        build_cancel_finalize_plan, build_cancel_plan, CancelFinalizeInput, CancelInput,
        CANCEL_ACTION, CANCEL_FINALIZE_ACTION,
    };
    use crate::noncustodial::shakedex::sell::{
        build_lock_finalize_plan, LockFinalizeInput, LOCK_FINALIZE_ACTION,
    };
    let pubkey = [2u8; 33];
    let transfer = coin(0x31, 0);
    let funding = [coin(1, 1)];
    let lock_finalize = build_lock_finalize_plan(&LockFinalizeInput {
        network: Network::Main,
        account: 0,
        transfer: &transfer,
        lock_pubkey: pubkey,
        name: "dexreviews",
        name_height: 120,
        weak: false,
        claimed: 0,
        renewals: 0,
        renewal_block: [0x77; 32],
        funding: &funding,
        change_address: CHANGE,
        rate: 5,
        fixed_fee: None,
    })
    .unwrap()
    .plan;
    let cancel = build_cancel_plan(&CancelInput {
        network: Network::Main,
        account: 0,
        name: "dexreviews",
        name_height: 120,
        lock_outpoint: ([0x2c; 32], 0),
        lock_value: 1_000_000,
        lock_pubkey: pubkey,
        cancel_address: PAY,
        cancel_branch: 0,
        cancel_index: 11,
        funding: &funding,
        change_address: CHANGE,
        rate: 5,
        fixed_fee: None,
    })
    .unwrap()
    .plan;
    let cancel_finalize = build_cancel_finalize_plan(&CancelFinalizeInput {
        network: Network::Main,
        account: 0,
        transfer_outpoint: ([0xba; 32], 0),
        transfer_value: 1_000_000,
        lock_pubkey: pubkey,
        name: "dexreviews",
        name_height: 120,
        weak: false,
        claimed: 0,
        renewals: 0,
        renewal_block: [0x88; 32],
        dest_address: PAY,
        funding: &funding,
        change_address: CHANGE,
        rate: 5,
        fixed_fee: None,
    })
    .unwrap()
    .plan;

    for (action, plan) in [
        (LOCK_FINALIZE_ACTION, &lock_finalize),
        (CANCEL_ACTION, &cancel),
        (CANCEL_FINALIZE_ACTION, &cancel_finalize),
    ] {
        let json = serde_json::to_string(plan).unwrap();
        refused_with_the_r16_reason(action, &json);
        refuse_unsupported_ledger_plan("mnemonic_hot", action, &json).unwrap();
    }
    for plan in [&cancel, &cancel_finalize] {
        refused_with_the_r16_reason("transfer", &serde_json::to_string(plan).unwrap());
    }
    assert!(
        !lock_finalize.has_foreign_or_custom_inputs(),
        "only its action tells the lock FINALIZE apart"
    );
}

/// R16/R29: every Shakedex draft action is in the refused class, so a new
/// one cannot reach a Ledger by being missing from a list.
#[test]
fn every_shakedex_action_is_refused_for_the_ledger() {
    use crate::noncustodial::shakedex::{cancel, is_shakedex_action, purchase, sell};
    for action in [
        purchase::PURCHASE_ACTION,
        purchase::PURCHASE_FINALIZE_ACTION,
        sell::LOCK_FINALIZE_ACTION,
        cancel::CANCEL_ACTION,
        cancel::CANCEL_FINALIZE_ACTION,
    ] {
        assert!(is_shakedex_action(action), "{action}");
        refused_with_the_r16_reason(action, PLAIN);
    }
    assert!(!is_shakedex_action("transfer"));
}
