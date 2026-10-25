//! Tests for `commands::paid_swaps`: the read-only list of offers recorded
//! before paid swaps were withdrawn. Nothing can create, claim or remove one
//! any more; Activity shows them so a seller can still see what was agreed.

use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::paid_swaps::list_paid_swap_offers;
use crate::db;
use crate::AppState;

fn app_with(setup: impl FnOnce(&rusqlite::Connection)) -> tauri::App<tauri::test::MockRuntime> {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    setup(&conn);
    mock_builder()
        .manage(AppState {
            db: std::sync::Mutex::new(conn),
            signer: std::sync::Mutex::new(None),
            secure_prompts: std::sync::Mutex::new(std::collections::HashMap::new()),
            hsd_children: std::sync::Mutex::new(std::collections::HashMap::new()),
            hsd_starting: std::sync::Mutex::new(std::collections::HashSet::new()),
            node_rpc_alive: std::sync::atomic::AtomicBool::new(false),
            sync_status: std::sync::Arc::new(tokio::sync::Mutex::new(
                crate::commands::sync::SyncStatus::default(),
            )),
        })
        .build(mock_context(noop_assets()))
        .expect("mock app")
}

#[test]
fn lists_no_offers_when_none_were_recorded() {
    let app = app_with(|_| {});
    assert!(list_paid_swap_offers(app.state()).unwrap().is_empty());
}

#[test]
fn lists_every_recorded_offer_newest_first_with_its_claim() {
    let app = app_with(|c| {
        c.execute_batch(
            "INSERT INTO paid_swap_offers (name, buyer_address, price_doos, created_at)
               VALUES ('older', 'hs1qolder', 1000000, '2026-09-01 10:00:00');
             INSERT INTO paid_swap_offers
               (name, buyer_address, price_doos, transfer_txid, claimed, created_at)
               VALUES ('newer', 'hs1qnewer', 2500000, 'ab12', 1, '2026-09-10 10:00:00');",
        )
        .unwrap();
    });
    let offers = list_paid_swap_offers(app.state()).unwrap();
    let json = serde_json::to_value(&offers).unwrap();
    assert_eq!(
        json,
        serde_json::json!([
            {
                "name": "newer",
                "buyerAddress": "hs1qnewer",
                "priceDoos": 2500000,
                "transferTxid": "ab12",
                "claimed": true,
                "createdAt": "2026-09-10 10:00:00"
            },
            {
                "name": "older",
                "buyerAddress": "hs1qolder",
                "priceDoos": 1000000,
                "transferTxid": null,
                "claimed": false,
                "createdAt": "2026-09-01 10:00:00"
            }
        ])
    );
}
