//! `compare_inventory_with_provider` reconciles local inventory against the names
//! Namebase still lists — ONE bulk `/api/domains` call (fast), with errors
//! surfaced (not swallowed into a false "everything missing").

use rusqlite::params;
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::read::compare_inventory_with_provider;
use crate::db;
use crate::error::AppError;
use crate::AppState;

fn app_with(conn: rusqlite::Connection) -> tauri::App<tauri::test::MockRuntime> {
    mock_builder()
        .manage(AppState {
            db: std::sync::Mutex::new(conn),
            signer: std::sync::Mutex::new(None),
            secure_prompts: std::sync::Mutex::new(std::collections::HashMap::new()),
            hsd_child: std::sync::Mutex::new(None),
        })
        .build(mock_context(noop_assets()))
        .expect("mock app")
}

/// In-memory DB seeded with the given inventory tlds, plus the Namebase
/// cookie/base-url so the client points at the mock server.
fn seeded_conn(tlds: &[&str], base_url: &str) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    for t in tlds {
        conn.execute("INSERT INTO assets (tld) VALUES (?1)", params![t]).unwrap();
    }
    db::queries::set_setting(&conn, "namebase_cookie", "testcookie").unwrap();
    db::queries::set_setting(&conn, "namebase_base_url", base_url).unwrap();
    conn
}

#[tokio::test]
async fn compare_buckets_inventory_against_namebase() {
    let mut server = mockito::Server::new_async().await;
    // Namebase still lists a, b, d. Inventory is a, b, c.
    let m = server
        .mock("GET", "/api/domains")
        .with_status(200)
        .with_body(r#"{"domains":[{"name":"a"},{"name":"b"},{"name":"d"}]}"#)
        .create_async()
        .await;

    let conn = seeded_conn(&["a", "b", "c"], &server.url());
    let app = app_with(conn);

    let r = compare_inventory_with_provider(app.state()).await.expect("compare ok");
    assert_eq!(r.provider_label, "Namebase");
    assert_eq!(r.matched, vec!["a".to_string(), "b".to_string()]); // still at Namebase
    assert_eq!(r.missing_at_provider, vec!["c".to_string()]); // left Namebase / not there
    assert_eq!(r.extra_at_provider, vec!["d".to_string()]); // on Namebase, not in inventory
    m.assert_async().await;
}

#[tokio::test]
async fn compare_errors_when_namebase_unreachable() {
    // A Namebase failure must surface as an error — NOT a false "all missing".
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", "/api/domains")
        .with_status(500)
        .with_body("{}")
        .create_async()
        .await;

    let conn = seeded_conn(&["a"], &server.url());
    let app = app_with(conn);

    let err = compare_inventory_with_provider(app.state())
        .await
        .expect_err("a Namebase error must propagate, not become empty buckets");
    assert!(matches!(err, AppError::Other(_)), "got {err:?}");
}
