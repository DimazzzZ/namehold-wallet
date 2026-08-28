//! Tests for `commands::paid_swaps` — the CRUD commands (create/get/remove)
//! plus the async `claim_paid_transfer` command. `claim_paid_transfer` is
//! covered here via a mockito-mocked hsd node (the REST `/tx/:hash` endpoint);
//! `find_payment_output` and `verify_paid_transfer_with_client` are tested
//! elsewhere (inline in the source file and in `node_rpc_injected_tests.rs`).

use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::paid_swaps::{
    claim_paid_transfer, create_paid_swap_offer, get_paid_swap_offer, remove_paid_swap_offer,
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

/// Build a mock app whose connection is pre-seeded (settings, offers, etc.)
/// by the supplied closure. Used by the `claim_paid_transfer` tests that need
/// a `node_rpc_url` setting pointing at a mockito server.
fn app_with(setup: impl FnOnce(&rusqlite::Connection)) -> tauri::App<tauri::test::MockRuntime> {
    let conn = migrated_conn();
    setup(&conn);
    mock_builder()
        .manage(AppState {
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

// ---------------------------------------------------------------------------
// claim_paid_transfer — validation branches (no node needed)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn claim_rejects_empty_name() {
    let app = app();
    let err = claim_paid_transfer(app.state(), "   ".into(), "txid".into())
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(m) if m == "name cannot be empty"));
}

#[tokio::test]
async fn claim_rejects_empty_txid() {
    let app = app();
    let err = claim_paid_transfer(app.state(), "example".into(), "   ".into())
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(m) if m == "txid cannot be empty"));
}

#[tokio::test]
async fn claim_returns_not_found_for_missing_offer() {
    let app = app();
    let err = claim_paid_transfer(app.state(), "ghost".into(), "txid".into())
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound(m) if m.contains("ghost")));
}

#[tokio::test]
async fn claim_rejects_already_claimed_offer() {
    let app = app_with(|conn| {
        create_offer(conn, "claimed-name", "hs1qbuyer", 1_000_000);
        conn.execute(
            "UPDATE paid_swap_offers SET claimed = 1 WHERE name = 'claimed-name'",
            [],
        )
        .unwrap();
    });
    let err = claim_paid_transfer(app.state(), "claimed-name".into(), "txid".into())
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(m) if m == "offer already claimed"));
}

// ---------------------------------------------------------------------------
// claim_paid_transfer — full path with a mockito-mocked hsd node
// ---------------------------------------------------------------------------

/// Helper: insert a fresh (unclaimed) offer directly into the DB.
fn create_offer(conn: &rusqlite::Connection, name: &str, buyer: &str, price_doos: i64) {
    conn.execute(
        "INSERT INTO paid_swap_offers (name, buyer_address, price_doos)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![name, buyer, price_doos],
    )
    .unwrap();
}

#[tokio::test]
async fn claim_verifies_and_marks_claimed_on_valid_payment() {
    let mut server = mockito::Server::new_async().await;
    let _tx = server
        .mock("GET", "/tx/paytx")
        .with_status(200)
        .with_body(
            r#"{"hash":"paytx","confirmations":7,
                "outputs":[
                  {"address":"hs1qbuyer","value":5000000},
                  {"address":"hs1qseller","value":1000000}
                ]}"#,
        )
        .create_async()
        .await;

    let url = server.url();
    let app = app_with(|conn| {
        create_offer(conn, "sale", "hs1qbuyer", 1_000_000);
        db::queries::set_setting(conn, "node_rpc_url", &url).unwrap();
    });

    let result = claim_paid_transfer(app.state(), "sale".into(), "paytx".into())
        .await
        .unwrap();
    assert!(result.verified);
    assert_eq!(result.paid_doos, 1_000_000);
    assert_eq!(result.confirmations, 7);

    // The offer must now be marked claimed with the transfer txid recorded.
    let offer = get_paid_swap_offer(app.state(), "sale".into())
        .unwrap()
        .unwrap();
    assert!(offer.claimed);
    assert_eq!(offer.transfer_txid.as_deref(), Some("paytx"));
}

#[tokio::test]
async fn claim_accepts_overpayment() {
    let mut server = mockito::Server::new_async().await;
    let _tx = server
        .mock("GET", "/tx/overpay")
        .with_status(200)
        .with_body(
            r#"{"hash":"overpay","confirmations":3,
                "outputs":[
                  {"address":"hs1qbuyer","value":5000000},
                  {"address":"hs1qseller","value":2500000}
                ]}"#,
        )
        .create_async()
        .await;

    let url = server.url();
    let app = app_with(|conn| {
        create_offer(conn, "sale", "hs1qbuyer", 1_000_000);
        db::queries::set_setting(conn, "node_rpc_url", &url).unwrap();
    });

    let result = claim_paid_transfer(app.state(), "sale".into(), "overpay".into())
        .await
        .unwrap();
    assert!(result.verified);
    assert_eq!(result.paid_doos, 2_500_000);
}

#[tokio::test]
async fn claim_returns_unverified_when_no_payment_output() {
    let mut server = mockito::Server::new_async().await;
    // Only outputs are to the buyer → no qualifying payment output.
    let _tx = server
        .mock("GET", "/tx/nopay")
        .with_status(200)
        .with_body(
            r#"{"hash":"nopay","confirmations":4,
                "outputs":[
                  {"address":"hs1qbuyer","value":5000000},
                  {"address":"hs1qbuyer","value":500000}
                ]}"#,
        )
        .create_async()
        .await;

    let url = server.url();
    let app = app_with(|conn| {
        create_offer(conn, "sale", "hs1qbuyer", 1_000_000);
        db::queries::set_setting(conn, "node_rpc_url", &url).unwrap();
    });

    let result = claim_paid_transfer(app.state(), "sale".into(), "nopay".into())
        .await
        .unwrap();
    assert!(!result.verified);
    assert_eq!(result.paid_doos, 0);
    assert_eq!(result.confirmations, 4);

    // Offer must NOT have been marked claimed.
    let offer = get_paid_swap_offer(app.state(), "sale".into())
        .unwrap()
        .unwrap();
    assert!(!offer.claimed);
    assert!(offer.transfer_txid.is_none());
}

#[tokio::test]
async fn claim_returns_unverified_when_payment_below_price() {
    let mut server = mockito::Server::new_async().await;
    // Payment output is 1 doo short of the asking price.
    let _tx = server
        .mock("GET", "/tx/short")
        .with_status(200)
        .with_body(
            r#"{"hash":"short","confirmations":1,
                "outputs":[
                  {"address":"hs1qbuyer","value":5000000},
                  {"address":"hs1qseller","value":999999}
                ]}"#,
        )
        .create_async()
        .await;

    let url = server.url();
    let app = app_with(|conn| {
        create_offer(conn, "sale", "hs1qbuyer", 1_000_000);
        db::queries::set_setting(conn, "node_rpc_url", &url).unwrap();
    });

    let result = claim_paid_transfer(app.state(), "sale".into(), "short".into())
        .await
        .unwrap();
    assert!(!result.verified);
    assert_eq!(result.paid_doos, 0);
}

#[tokio::test]
async fn claim_returns_not_found_when_node_lacks_tx() {
    let mut server = mockito::Server::new_async().await;
    // hsd returns 404 for an unknown tx → get_tx_by_hash yields Value::Null
    // → the helper maps that to NotFound.
    let _tx = server
        .mock("GET", "/tx/missing")
        .with_status(404)
        .with_body("")
        .create_async()
        .await;

    let url = server.url();
    let app = app_with(|conn| {
        create_offer(conn, "sale", "hs1qbuyer", 1_000_000);
        db::queries::set_setting(conn, "node_rpc_url", &url).unwrap();
    });

    let err = claim_paid_transfer(app.state(), "sale".into(), "missing".into())
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound(m) if m.contains("missing")));

    // Offer must remain unclaimed after a failed lookup.
    let offer = get_paid_swap_offer(app.state(), "sale".into())
        .unwrap()
        .unwrap();
    assert!(!offer.claimed);
}

#[tokio::test]
async fn claim_propagates_node_rpc_error() {
    let mut server = mockito::Server::new_async().await;
    // Non-JSON body with a 500 status → transport/RPC-style error propagates.
    let _tx = server
        .mock("GET", "/tx/boom")
        .with_status(500)
        .with_body("not json")
        .create_async()
        .await;

    let url = server.url();
    let app = app_with(|conn| {
        create_offer(conn, "sale", "hs1qbuyer", 1_000_000);
        db::queries::set_setting(conn, "node_rpc_url", &url).unwrap();
    });

    let err = claim_paid_transfer(app.state(), "sale".into(), "boom".into())
        .await
        .unwrap_err();
    // The exact variant depends on how the node client classifies the failure;
    // it must NOT be a successful claim and must NOT be NotFound.
    assert!(!matches!(err, AppError::NotFound(_)));

    let offer = get_paid_swap_offer(app.state(), "sale".into())
        .unwrap()
        .unwrap();
    assert!(!offer.claimed);
}

#[tokio::test]
async fn claim_finds_payment_output_in_the_middle() {
    let mut server = mockito::Server::new_async().await;
    // Payment output is neither first nor last; find_payment_output must locate it.
    let _tx = server
        .mock("GET", "/tx/midpay")
        .with_status(200)
        .with_body(
            r#"{"hash":"midpay","confirmations":9,
                "outputs":[
                  {"address":"hs1qbuyer","value":5000000},
                  {"address":"hs1qseller","value":1200000},
                  {"address":"hs1qbuyer","value":300000}
                ]}"#,
        )
        .create_async()
        .await;

    let url = server.url();
    let app = app_with(|conn| {
        create_offer(conn, "sale", "hs1qbuyer", 1_000_000);
        db::queries::set_setting(conn, "node_rpc_url", &url).unwrap();
    });

    let result = claim_paid_transfer(app.state(), "sale".into(), "midpay".into())
        .await
        .unwrap();
    assert!(result.verified);
    assert_eq!(result.paid_doos, 1_200_000);
}

#[tokio::test]
async fn claim_two_offers_are_marked_independently() {
    let mut server = mockito::Server::new_async().await;
    let _tx_a = server
        .mock("GET", "/tx/txA")
        .with_status(200)
        .with_body(
            r#"{"hash":"txA","confirmations":2,
                "outputs":[{"address":"sellerA","value":1000000}]}"#,
        )
        .create_async()
        .await;
    let _tx_b = server
        .mock("GET", "/tx/txB")
        .with_status(200)
        .with_body(
            r#"{"hash":"txB","confirmations":2,
                "outputs":[{"address":"sellerB","value":2000000}]}"#,
        )
        .create_async()
        .await;

    let url = server.url();
    let app = app_with(|conn| {
        create_offer(conn, "alpha", "buyerA", 1_000_000);
        create_offer(conn, "beta", "buyerB", 2_000_000);
        db::queries::set_setting(conn, "node_rpc_url", &url).unwrap();
    });

    claim_paid_transfer(app.state(), "alpha".into(), "txA".into())
        .await
        .unwrap();
    let alpha = get_paid_swap_offer(app.state(), "alpha".into())
        .unwrap()
        .unwrap();
    let beta_before = get_paid_swap_offer(app.state(), "beta".into())
        .unwrap()
        .unwrap();
    assert!(alpha.claimed);
    assert!(
        !beta_before.claimed,
        "beta must be untouched by alpha's claim"
    );

    claim_paid_transfer(app.state(), "beta".into(), "txB".into())
        .await
        .unwrap();
    let beta = get_paid_swap_offer(app.state(), "beta".into())
        .unwrap()
        .unwrap();
    assert!(beta.claimed);
    assert_eq!(beta.transfer_txid.as_deref(), Some("txB"));
}

#[tokio::test]
async fn claim_trims_name_and_txid() {
    let mut server = mockito::Server::new_async().await;
    let _tx = server
        .mock("GET", "/tx/trimtx")
        .with_status(200)
        .with_body(
            r#"{"hash":"trimtx","confirmations":6,
                "outputs":[{"address":"hs1qseller","value":1000000}]}"#,
        )
        .create_async()
        .await;

    let url = server.url();
    let app = app_with(|conn| {
        create_offer(conn, "sale", "hs1qbuyer", 1_000_000);
        db::queries::set_setting(conn, "node_rpc_url", &url).unwrap();
    });

    let result = claim_paid_transfer(app.state(), "  sale  ".into(), "  trimtx  ".into())
        .await
        .unwrap();
    assert!(result.verified);
    let offer = get_paid_swap_offer(app.state(), "sale".into())
        .unwrap()
        .unwrap();
    assert_eq!(offer.transfer_txid.as_deref(), Some("trimtx"));
}

// ---------------------------------------------------------------------------
// get_paid_swap_offer — edge cases (claimed flag + transfer_txid round-trip)
// ---------------------------------------------------------------------------

#[test]
fn get_deserializes_null_transfer_txid_as_none() {
    let app = app();
    create_paid_swap_offer(app.state(), "n".into(), "hs1qb".into(), 1_000).unwrap();
    let offer = get_paid_swap_offer(app.state(), "n".into())
        .unwrap()
        .unwrap();
    assert!(offer.transfer_txid.is_none());
    assert!(!offer.claimed);
}

#[test]
fn get_deserializes_claimed_flag_and_txid() {
    let app = app_with(|conn| {
        create_offer(conn, "n", "hs1qb", 1_000);
        conn.execute(
            "UPDATE paid_swap_offers SET claimed = 1, transfer_txid = 'set-txid' WHERE name = 'n'",
            [],
        )
        .unwrap();
    });
    let offer = get_paid_swap_offer(app.state(), "n".into())
        .unwrap()
        .unwrap();
    assert!(offer.claimed);
    assert_eq!(offer.transfer_txid.as_deref(), Some("set-txid"));
}
