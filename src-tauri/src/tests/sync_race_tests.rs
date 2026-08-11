//! Regression tests for the `start_full_sync` start/start race (Task 9, S6)
//! and for the "sync captures its profile once, and never re-reads the
//! active profile mid-run" invariant that keeps a background run from
//! writing into a DIFFERENT profile's rows after the user switches the
//! active profile mid-sync.
//!
//! Uses the same `tauri::test::mock_builder` + file-backed-DB pattern as
//! `discover_names_tests.rs` / `repair_convergence_tests.rs`.

use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::sync::start_full_sync;
use crate::db;
use crate::AppState;

const PROFILE_A: &str = "syncrace_a";
const PROFILE_B: &str = "syncrace_b";

/// Serializes the tests below that spawn a REAL `start_full_sync` background
/// OS thread. Every such thread — regardless of which test spawned it —
/// unconditionally does `TEST_PANIC_HOOK.swap(false, ...)` (see that
/// static's doc comment): it's a single process-global flag, not scoped to
/// a test. Without this gate, `concurrent_start_full_sync_exactly_one_wins`'s
/// background thread can run concurrently with
/// `panic_in_sync_thread_clears_running_and_records_error` and consume the
/// flag the latter just set, before that test's own thread ever checks it —
/// making the panic never fire and the test fail with "an error describing
/// the panic must be recorded, got []". Observed intermittently under
/// `cargo test`'s parallel runner once enough other test modules shifted
/// scheduling timing; this lock removes the race outright rather than
/// tuning it away.
static REAL_SYNC_THREAD_TEST_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn app_with(conn: rusqlite::Connection) -> tauri::App<tauri::test::MockRuntime> {
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

/// A migrated, file-backed DB (needed because the sync thread re-opens the
/// path independently of the `AppState` connection) with two profiles, one
/// active. Node/explorer settings point at unroutable local addresses so any
/// background HTTP call this test's sync run makes fails almost immediately
/// instead of hitting a real node/explorer or hanging.
fn seeded_db() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    // PID keeps the path unique across nextest's per-test processes (the
    // COUNTER alone resets to 0 in each process → collisions → readonly DB).
    let pid = std::process::id();
    let path = std::env::temp_dir().join(format!("namehold_sync_race_test_{pid}_{n}.db"));
    let _ = std::fs::remove_file(&path);

    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    db::queries::insert_wallet_profile(
        &conn,
        PROFILE_A,
        "A",
        "mnemonic_hot",
        "mainnet",
        "xpubFAKEA",
        0,
        false,
    )
    .unwrap();
    db::queries::insert_wallet_profile(
        &conn,
        PROFILE_B,
        "B",
        "mnemonic_hot",
        "mainnet",
        "xpubFAKEB",
        0,
        false,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, PROFILE_A).unwrap();
    // Deliberately unroutable: no derived addresses/assets are seeded either,
    // so repair/discover short-circuit without any HTTP call, and the one
    // node RPC call (`get_blockchain_info`) fails fast (connection refused)
    // rather than reaching a real node.
    db::queries::set_setting(&conn, "node_rpc_url", "http://127.0.0.1:1").unwrap();
    db::queries::set_setting(&conn, "explorer_api_url", "http://127.0.0.1:1").unwrap();
    drop(conn);
    path
}

struct TempDbGuard(std::path::PathBuf);
impl Drop for TempDbGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(self.0.with_extension("db-wal"));
        let _ = std::fs::remove_file(self.0.with_extension("db-shm"));
    }
}

// ---------------------------------------------------------------------------
// 1. Deterministic unit-level proof of the atomic check-and-set: simulate an
//    in-flight run by flipping `running = true` directly (no thread timing
//    involved), then call the real command and assert it refuses AND leaves
//    the existing status untouched.
//
//    Before the fix, `start_full_sync` had NO `running` check at all — it
//    unconditionally reset status (`*s = SyncStatus::default()`) and spawned
//    a second background thread even while one was already in flight. This
//    test fails against that old code: it would see `started: true` (no
//    `alreadyRunning` key) and the sentinel `started_at` would be clobbered.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn start_full_sync_refuses_second_start_while_running() {
    let path = seeded_db();
    let _guard = TempDbGuard(path.clone());
    let conn = rusqlite::Connection::open(&path).unwrap();
    let app = app_with(conn);

    {
        let state = app.state::<AppState>();
        let mut s = state.sync_status.lock().await;
        s.running = true;
        s.started_at = Some("SENTINEL".to_string());
        s.repaired = 7; // arbitrary in-progress state that must survive
    }

    let result = start_full_sync(app.state()).await.expect("command ok");
    assert_eq!(
        result["started"],
        serde_json::json!(false),
        "must refuse to start a second run"
    );
    assert_eq!(result["alreadyRunning"], serde_json::json!(true));

    let state = app.state::<AppState>();
    let s = state.sync_status.lock().await;
    assert_eq!(
        s.started_at.as_deref(),
        Some("SENTINEL"),
        "the in-flight run's status must be untouched"
    );
    assert_eq!(
        s.repaired, 7,
        "no reset to SyncStatus::default() must have happened"
    );
    assert!(s.running, "still reports running");
}

// ---------------------------------------------------------------------------
// 2. Genuine concurrency proof: fire two `start_full_sync` calls concurrently
//    via `tokio::join!` against the SAME AppState. Because the check-and-set
//    is one critical section with no `.await` inside it other than the lock
//    acquisition itself, exactly one of the two must see `alreadyRunning`.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn concurrent_start_full_sync_exactly_one_wins() {
    let _gate = REAL_SYNC_THREAD_TEST_GATE.lock().await;
    let path = seeded_db();
    let _guard = TempDbGuard(path.clone());
    let conn = rusqlite::Connection::open(&path).unwrap();
    let app = app_with(conn);

    let (r1, r2) = tokio::join!(start_full_sync(app.state()), start_full_sync(app.state()),);
    let r1 = r1.expect("command ok");
    let r2 = r2.expect("command ok");

    let started_count = [&r1, &r2]
        .iter()
        .filter(|r| r["started"] == serde_json::json!(true))
        .count();
    let already_running_count = [&r1, &r2]
        .iter()
        .filter(|r| r["alreadyRunning"] == serde_json::json!(true))
        .count();

    assert_eq!(started_count, 1, "exactly one of the two concurrent calls must actually start a run, got r1={r1:?} r2={r2:?}");
    assert_eq!(
        already_running_count, 1,
        "the other must observe alreadyRunning, got r1={r1:?} r2={r2:?}"
    );

    // Let the winning background thread finish (fast: no addresses/assets
    // seeded, node RPC points at an unroutable address) so it doesn't
    // outlive the test process.
    for _ in 0..100 {
        let running = {
            let state = app.state::<AppState>();
            let s = state.sync_status.lock().await;
            s.running
        };
        if !running {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

// ---------------------------------------------------------------------------
// 3. Profile-generation guard: `repair_step_windowed` (and its siblings
//    `sync_node_step` / `discover_step`) take `profile_id` as an explicit
//    parameter captured ONCE by `start_full_sync` before the background
//    thread starts — none of them ever re-reads "the active profile" mid-run.
//    Prove this directly: run `repair_step_windowed` for profile A against a
//    mocked explorer that resolves a name as owned, and RACE an active-profile
//    switch to B against it (via a separate connection, simulating the user
//    switching wallets while sync runs in the background). The resulting
//    `tracked_name_states` write must land under A, never B, regardless of
//    which profile is "active" by the time the write happens.
// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// 4. Panic-in-a-sync-step must not brick Sync forever. Injects a
//    deterministic panic partway through the background run (right after
//    `profile_id` is resolved, before Step 1 — see `TEST_PANIC_HOOK`) and
//    proves the `RunningGuard` Drop cleanup runs: `running` is cleared, an
//    error naming the panic is recorded, and — critically, given Task 9's
//    atomic check-and-set — a SUBSEQUENT `start_full_sync` call actually
//    starts rather than being refused as `alreadyRunning`.
//
//    Negative control (not automated — restructuring the fix to prove a
//    hang isn't worth the churn): reverting the `RunningGuard`
//    struct/Drop-impl and the `guard.mark_completed()` calls back to the
//    pre-fix code (bare `.expect(...)` + no guard) makes this test hang —
//    the panicked thread never clears `running`, so the polling loop below
//    spins until it times out and the final assertions fail on `running ==
//    true` with `errors` still empty.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn panic_in_sync_thread_clears_running_and_records_error() {
    let _gate = REAL_SYNC_THREAD_TEST_GATE.lock().await;
    let path = seeded_db();
    let _guard = TempDbGuard(path.clone());
    let conn = rusqlite::Connection::open(&path).unwrap();
    let app = app_with(conn);

    crate::commands::sync::TEST_PANIC_HOOK.store(true, std::sync::atomic::Ordering::SeqCst);

    let result = start_full_sync(app.state()).await.expect("command ok");
    assert_eq!(
        result["started"],
        serde_json::json!(true),
        "first start must succeed"
    );

    // Wait for the background thread to panic, unwind, and let the
    // RunningGuard clear `running`.
    let mut running = true;
    let mut errors = Vec::new();
    for _ in 0..100 {
        let state = app.state::<AppState>();
        let s = state.sync_status.lock().await;
        running = s.running;
        errors = s.errors.clone();
        drop(s);
        if !running {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    assert!(
        !running,
        "running must be cleared after the sync thread panics — otherwise, given Task 9's \
         atomic check-and-set, Sync is permanently bricked until app restart"
    );
    assert!(
        errors.iter().any(|e| e.contains("panicked")),
        "an error describing the panic must be recorded, got {errors:?}"
    );

    // The real proof this finding cared about: a SUBSEQUENT start must
    // succeed, not be refused as `alreadyRunning`.
    let result2 = start_full_sync(app.state()).await.expect("command ok");
    assert_eq!(
        result2["started"],
        serde_json::json!(true),
        "a start after a panicked run must succeed, not be refused, got {result2:?}"
    );

    // Let the second run finish so it doesn't outlive the test process.
    for _ in 0..100 {
        let state = app.state::<AppState>();
        let s = state.sync_status.lock().await;
        let still_running = s.running;
        drop(s);
        if !still_running {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

mod profile_scope_guard {
    use crate::commands::sync::{repair_step_windowed, SyncStatus};
    use crate::db;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    const PROFILE_A: &str = "scoperepair_a";
    const PROFILE_B: &str = "scoperepair_b";
    const MINE: &str = "hs1qmineaddr0000000000000000000000000001";

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct TempDb {
        path: std::path::PathBuf,
    }
    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
            let _ = std::fs::remove_file(self.path.with_extension("db-wal"));
            let _ = std::fs::remove_file(self.path.with_extension("db-shm"));
        }
    }

    fn seeded_db(explorer_url: &str) -> TempDb {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let path =
            std::env::temp_dir().join(format!("namehold_sync_scope_repair_test_{pid}_{n}.db"));
        let _ = std::fs::remove_file(&path);
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        db::migrations::run(&conn).unwrap();
        db::queries::insert_wallet_profile(
            &conn,
            PROFILE_A,
            "A",
            "mnemonic_hot",
            "mainnet",
            "xpubFAKEA",
            0,
            false,
        )
        .unwrap();
        db::queries::insert_wallet_profile(
            &conn,
            PROFILE_B,
            "B",
            "mnemonic_hot",
            "mainnet",
            "xpubFAKEB",
            0,
            false,
        )
        .unwrap();
        db::queries::set_active_profile(&conn, PROFILE_A).unwrap();
        db::queries::set_setting(&conn, "explorer_api_url", explorer_url).unwrap();
        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index,
                 address, script_pubkey_hex, public_key_hex)
             VALUES (?1, 0, 0, 0, ?2, '0014aa', '02aa')",
            rusqlite::params![PROFILE_A, MINE],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO assets (tld, status) VALUES ('ownedname', 'not_started')",
            [],
        )
        .unwrap();
        drop(conn);
        TempDb { path }
    }

    #[tokio::test]
    async fn repair_step_write_stays_on_captured_profile_despite_mid_run_active_profile_switch() {
        let mut server = mockito::Server::new_async().await;
        let _name = server
            .mock("GET", "/api/names/ownedname")
            .with_body(r#"{"name":"ownedname","hash":"deadbeef","state":"CLOSED","height":100,"renewal":200}"#)
            .create_async()
            .await;
        let _hist = server
            .mock("GET", "/api/names/ownedname/history")
            .with_body(r#"{"result":[{"action":"Finalize","txid":"txA","index":0}]}"#)
            .create_async()
            .await;
        let _owner_tx = server
            .mock("GET", "/api/txs/txA")
            .with_body(format!(
                r#"{{"outputs":[{{"action":"FINALIZE","name":"ownedname","address":"{MINE}","value":400000}}]}}"#
            ))
            .create_async()
            .await;

        let db = seeded_db(&server.url());
        let db_path = db.path.to_str().unwrap().to_string();
        let status = Arc::new(tokio::sync::Mutex::new(SyncStatus::default()));

        // Run repair for PROFILE_A explicitly — exactly the `profile_id` arg
        // `start_full_sync` would have captured before spawning — spawned so
        // we can race a profile switch against it.
        let db_path2 = db_path.clone();
        let status2 = status.clone();
        let handle = tokio::spawn(async move {
            repair_step_windowed(&status2, &db_path2, PROFILE_A, 150).await;
        });

        // Switch the active profile to B WHILE the repair run (for A) is
        // still in flight. The run needs at least two explorer round trips
        // before it writes (each a mock HTTP round trip), so this has a real
        // window to land before that write. (Under `cfg(test)`
        // DISCOVERY_THROTTLE is 0, so the window comes from the round trips
        // themselves, not the throttle — still ample for the 30ms switch.)
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        {
            let switch_conn = rusqlite::Connection::open(&db.path).unwrap();
            db::queries::set_active_profile(&switch_conn, PROFILE_B).unwrap();
        }

        handle.await.unwrap();

        let verify_conn = rusqlite::Connection::open(&db.path).unwrap();
        let profile_of_write: String = verify_conn
            .query_row(
                "SELECT wallet_profile_id FROM tracked_name_states WHERE name = 'ownedname'",
                [],
                |r| r.get(0),
            )
            .expect("owned name was recorded");
        assert_eq!(
            profile_of_write, PROFILE_A,
            "repair_step_windowed must keep writing under the profile_id it was called with (A), \
             even though the active profile setting changed to B mid-run"
        );

        // Sanity: profile B, only made active mid-run, received nothing.
        let b_count: i64 = verify_conn
            .query_row(
                "SELECT COUNT(*) FROM tracked_name_states WHERE wallet_profile_id = ?1",
                rusqlite::params![PROFILE_B],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            b_count, 0,
            "profile B must not have received any writes from A's run"
        );
    }
}
