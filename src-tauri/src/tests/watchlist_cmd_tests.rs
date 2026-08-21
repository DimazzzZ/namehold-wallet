//! Tests for `commands::watchlist` — watchlist CRUD + CSV import/export, driven
//! through the real `#[tauri::command]` functions over a fully-migrated
//! in-memory DB.

use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::watchlist::{
    add_to_watchlist, export_watchlist_csv, get_watchlist_status, import_watchlist_csv, is_watched,
    list_watchlist, remove_from_watchlist, update_watchlist_tags,
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

fn tmp_csv_path(label: &str) -> String {
    let mut path = std::env::temp_dir();
    let unique = format!(
        "namehold_watchlist_{}_{}_{}.csv",
        label,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    path.push(unique);
    path.to_string_lossy().into_owned()
}

#[test]
fn add_then_list_and_is_watched() {
    let app = app();
    add_to_watchlist(
        app.state(),
        "example".into(),
        Some("my note".into()),
        Some("tag1,tag2".into()),
    )
    .unwrap();

    let list = list_watchlist(app.state()).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "example");
    assert_eq!(list[0].notes, "my note");
    assert_eq!(list[0].tags, "tag1,tag2");

    assert!(is_watched(app.state(), "example".into()).unwrap());
    assert!(!is_watched(app.state(), "missing".into()).unwrap());
}

#[test]
fn add_trims_name_and_rejects_empty() {
    let app = app();
    add_to_watchlist(app.state(), "  spaced  ".into(), None, None).unwrap();
    assert!(is_watched(app.state(), "spaced".into()).unwrap());

    let err = add_to_watchlist(app.state(), "   ".into(), None, None).unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(_)));
}

#[test]
fn add_is_idempotent_and_updates_notes_tags() {
    let app = app();
    add_to_watchlist(
        app.state(),
        "dup".into(),
        Some("first".into()),
        Some("a".into()),
    )
    .unwrap();
    add_to_watchlist(
        app.state(),
        "dup".into(),
        Some("second".into()),
        Some("b".into()),
    )
    .unwrap();

    let list = list_watchlist(app.state()).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].notes, "second");
    assert_eq!(list[0].tags, "b");
}

#[test]
fn remove_is_idempotent() {
    let app = app();
    add_to_watchlist(app.state(), "gone".into(), None, None).unwrap();
    remove_from_watchlist(app.state(), "gone".into()).unwrap();
    assert!(!is_watched(app.state(), "gone".into()).unwrap());
    // Removing again is a no-op, not an error.
    remove_from_watchlist(app.state(), "gone".into()).unwrap();
}

#[test]
fn list_is_ordered_newest_first() {
    let app = app();
    // Insert with explicit added_at to make ordering deterministic.
    {
        let db = app.state::<AppState>();
        let conn = db.db.lock().unwrap();
        conn.execute(
            "INSERT INTO watched_names (name, notes, tags, added_at) VALUES ('old', '', '', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO watched_names (name, notes, tags, added_at) VALUES ('new', '', '', '2024-12-31T00:00:00Z')",
            [],
        )
        .unwrap();
    }
    let list = list_watchlist(app.state()).unwrap();
    assert_eq!(list[0].name, "new");
    assert_eq!(list[1].name, "old");
}

#[test]
fn update_tags_succeeds_and_errors_when_absent() {
    let app = app();
    add_to_watchlist(app.state(), "tagme".into(), None, Some("old".into())).unwrap();
    update_watchlist_tags(app.state(), "tagme".into(), "  new,tags  ".into()).unwrap();

    let list = list_watchlist(app.state()).unwrap();
    assert_eq!(list[0].tags, "new,tags");

    let err = update_watchlist_tags(app.state(), "nonexistent".into(), "x".into()).unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
}

#[test]
fn get_watchlist_status_reports_membership_and_tags() {
    let app = app();
    add_to_watchlist(app.state(), "watched".into(), None, Some("mytag".into())).unwrap();

    let statuses =
        get_watchlist_status(app.state(), vec!["watched".into(), "unwatched".into()]).unwrap();
    assert_eq!(statuses.len(), 2);

    let w = statuses.iter().find(|s| s.name == "watched").unwrap();
    assert!(w.watched);
    assert_eq!(w.tags, "mytag");
    // No cached tracked_name_states row → state/expiry None.
    assert!(w.state.is_none());
    assert!(w.expiry.is_none());

    let u = statuses.iter().find(|s| s.name == "unwatched").unwrap();
    assert!(!u.watched);
    assert_eq!(u.tags, "");
}

#[test]
fn csv_export_then_import_round_trips() {
    let path_str = tmp_csv_path("roundtrip");
    struct Cleanup(String);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _cleanup = Cleanup(path_str.clone());

    // Export from a populated source app.
    let src = app();
    add_to_watchlist(
        src.state(),
        "alpha".into(),
        Some("note, with comma".into()),
        Some("t1".into()),
    )
    .unwrap();
    add_to_watchlist(src.state(), "beta".into(), None, Some("t2".into())).unwrap();
    let exported = export_watchlist_csv(src.state(), path_str.clone()).unwrap();
    assert_eq!(exported, 2);

    // Import into a fresh, empty destination app.
    let dest = app();
    let result = import_watchlist_csv(dest.state(), path_str.clone()).unwrap();
    assert_eq!(result.imported, 2);
    assert_eq!(result.skipped, 0);
    assert!(result.errors.is_empty());

    let list = list_watchlist(dest.state()).unwrap();
    assert_eq!(list.len(), 2);
    let alpha = list.iter().find(|w| w.name == "alpha").unwrap();
    // The comma-containing note must survive the CSV quoting round-trip.
    assert_eq!(alpha.notes, "note, with comma");
    assert_eq!(alpha.tags, "t1");
}

#[test]
fn csv_import_skips_duplicates() {
    let path_str = tmp_csv_path("dupes");
    struct Cleanup(String);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _cleanup = Cleanup(path_str.clone());

    let src = app();
    add_to_watchlist(src.state(), "dup".into(), None, None).unwrap();
    export_watchlist_csv(src.state(), path_str.clone()).unwrap();

    // Destination already has the name → import counts it as skipped.
    let dest = app();
    add_to_watchlist(dest.state(), "dup".into(), None, None).unwrap();
    let result = import_watchlist_csv(dest.state(), path_str).unwrap();
    assert_eq!(result.imported, 0);
    assert_eq!(result.skipped, 1);
}
