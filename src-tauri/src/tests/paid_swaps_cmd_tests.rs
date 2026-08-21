//! Tests for `commands::paid_swaps` — the CRUD commands (create/get/remove).
//! The async `claim_paid_transfer` needs a live node so it is not covered here;
//! `find_payment_output` is already tested inline in the source file.

use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::paid_swaps::{
    create_paid_swap_offer, get_paid_swap_offer, remove_paid_swap_offer,
};
use crate::db;
use crate::error::AppError;
use crate::AppState;

fn migrated_conn() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    conn
}

fn app() -> tauri::App<tauri::test::MockRuntime> {
    mock_builder()
        .manage(AppState {
            db: std::sync::Mutex::new(migrated_conn()),
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

#[test]
fn get_returns_none_when_missing() {
    let app = app();
    let result = get_paid_swap_offer(app.state(), "nope".into()).unwrap();
    assert!(result.is_none());
}

#[test]
fn create_then_get_returns_offer() {
    let app = app();
    create_paid_swap_offer(app.state(), "example".into(), "hs1qbuyer".into(), 5_000_000).unwrap();

    let offer = get_paid_swap_offer(app.state(), "example".into())
        .unwrap()
        .expect("offer should exist");
    assert_eq!(offer.name, "example");
    assert_eq!(offer.buyer_address, "hs1qbuyer");
    assert_eq!(offer.price_doos, 5_000_000);
    assert!(!offer.claimed);
    assert!(offer.transfer_txid.is_none());
}

#[test]
fn create_rejects_empty_name() {
    let app = app();
    let err =
        create_paid_swap_offer(app.state(), "   ".into(), "hs1qbuyer".into(), 100).unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(_)));
}

#[test]
fn create_rejects_empty_buyer_address() {
    let app = app();
    let err = create_paid_swap_offer(app.state(), "n".into(), "   ".into(), 100).unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(_)));
}

#[test]
fn create_rejects_non_positive_price() {
    let app = app();
    let err = create_paid_swap_offer(app.state(), "n".into(), "hs1qb".into(), 0).unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(_)));
    let err = create_paid_swap_offer(app.state(), "n".into(), "hs1qb".into(), -1).unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(_)));
}

#[test]
fn create_upserts_on_conflict_and_resets_claim_state() {
    let app = app();
    create_paid_swap_offer(app.state(), "n".into(), "hs1qb1".into(), 1_000).unwrap();
    // Simulate that the offer was claimed and had a transfer txid recorded.
    {
        let state = app.state::<AppState>();
        let conn = state.db.lock().unwrap();
        conn.execute(
            "UPDATE paid_swap_offers SET claimed = 1, transfer_txid = 'oldtxid' WHERE name = 'n'",
            [],
        )
        .unwrap();
    }
    // Re-creating should upsert AND reset claim state per the ON CONFLICT clause.
    create_paid_swap_offer(app.state(), "n".into(), "hs1qb2".into(), 2_000).unwrap();
    let offer = get_paid_swap_offer(app.state(), "n".into())
        .unwrap()
        .unwrap();
    assert_eq!(offer.buyer_address, "hs1qb2");
    assert_eq!(offer.price_doos, 2_000);
    assert!(!offer.claimed);
    assert!(offer.transfer_txid.is_none());
}

#[test]
fn remove_deletes_and_is_idempotent() {
    let app = app();
    create_paid_swap_offer(app.state(), "n".into(), "hs1qb".into(), 1_000).unwrap();
    remove_paid_swap_offer(app.state(), "n".into()).unwrap();
    assert!(get_paid_swap_offer(app.state(), "n".into())
        .unwrap()
        .is_none());
    // Removing again does not error.
    remove_paid_swap_offer(app.state(), "n".into()).unwrap();
}

#[test]
fn get_trims_name_on_lookup() {
    let app = app();
    create_paid_swap_offer(app.state(), "n".into(), "hs1qb".into(), 1_000).unwrap();
    let offer = get_paid_swap_offer(app.state(), "  n  ".into())
        .unwrap()
        .expect("trimmed lookup should hit");
    assert_eq!(offer.name, "n");
}
