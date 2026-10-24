//! Process-lifecycle **integration** tests for `start_hsd` / `stop_hsd` /
//! `resync_hsd_chain` — the paths the unit-test suite deliberately leaves
//! uncovered because they spawn a real OS process, wait on it, kill it, and
//! move chain files on disk.
//!
//! The seam: `node.rs` resolves the hsd binary from the `hsd_path` setting
//! (verbatim, via `configured_hsd_path` → `find_hsd_binary`) and the data dir
//! from `hsd_prefix`. We point `hsd_path` at a **fake hsd** shell script we
//! generate into a tempdir and `hsd_prefix` at that same tempdir. The fake
//! answers `--version` (so the minimum-version gate passes) and, when spawned
//! as the node, behaves per a baked-in mode:
//!   - `stay-alive`: sleeps, so the child stays running (exercises the spawn →
//!     wait-loop → RPC-probe path; RPC is mocked with mockito).
//!   - `die-lock`: prints a data-dir-LOCK line to stdout and exits non-zero
//!     (hsd's stdout/stderr are redirected to `namehold-hsd.log` by `start_hsd`,
//!     so this drives the "already running on this data dir" branch).
//!   - `die-error`: prints a generic error and exits non-zero (the
//!     "hsd exited on startup" branch + log-tail surfacing).
//!
//! These tests are `#[cfg(unix)]`: the fake binary is a `/bin/sh` script and we
//! rely on POSIX kill/wait semantics. The production code they exercise is the
//! same on macOS and Linux CI.
#![cfg(unix)]

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serial_test::serial;
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::node::{resync_hsd_chain, start_hsd, stop_hsd};
use crate::db;
use crate::noncustodial::network::Network;
use crate::AppState;

/// Behavior of the fake hsd when spawned as the node (not `--version`).
enum FakeMode {
    /// Sleep so the child stays alive; the node "runs".
    StayAlive,
    /// Print a data-dir LOCK line and exit non-zero.
    DieLock,
    /// Print a generic error line and exit non-zero.
    DieError,
}

/// A tempdir that cleans itself up on drop. Holds the fake binary + data dir.
struct Harness {
    dir: PathBuf,
    binary: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Harness {
    /// Create a unique tempdir and write a fake-hsd `/bin/sh` script into it
    /// with the given spawn behavior. `version` controls the `--version` reply
    /// (so we can also exercise the too-old-version gate).
    fn new(mode: FakeMode, version: &str) -> Self {
        // Uniqueness must not depend on clock resolution: two threads entering
        // this function within the same coarse `SystemTime` tick would otherwise
        // compute the SAME dir, and the last writer's script/drop would clobber
        // the other test's binary — silently swapping version + spawn behavior.
        // A process-global atomic counter guarantees a distinct dir per harness.
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let unique = format!("namehold-fakehsd-{}-{seq}", std::process::id());
        let dir = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&dir).unwrap();

        let spawn_body = match mode {
            // Sleep long enough that the test finishes and kills us; if the test
            // forgets to kill, the tempdir drop + process exit still bound it.
            FakeMode::StayAlive => "sleep 60\n",
            FakeMode::DieLock => {
                "echo 'Error: could not obtain lock: Resource temporarily unavailable (LOCK)'\nexit 1\n"
            }
            FakeMode::DieError => "echo 'Error: something went wrong during startup'\nexit 1\n",
        };

        let script = format!(
            "#!/bin/sh\n\
             # Fake hsd for integration tests.\n\
             if [ \"$1\" = \"--version\" ]; then\n\
             \techo '{version}'\n\
             \texit 0\n\
             fi\n\
             # Record the full argv of a node spawn so tests can assert which\n\
             # network flag (--regtest/--testnet/...) and --prefix hsd was told\n\
             # to use. Written next to the binary; one line, space-joined.\n\
             echo \"$@\" > \"$(dirname \"$0\")/fake-hsd-argv.log\"\n\
             {spawn_body}",
        );

        let binary = dir.join("fake-hsd");
        {
            let mut f = std::fs::File::create(&binary).unwrap();
            f.write_all(script.as_bytes()).unwrap();
        }
        let mut perms = std::fs::metadata(&binary).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&binary, perms).unwrap();

        Harness { dir, binary }
    }

    fn data_dir(&self) -> &Path {
        &self.dir
    }

    /// Read back the argv the fake hsd recorded on its last node spawn.
    /// Returns `None` if the fake was only asked for `--version` (no spawn),
    /// or if the script hasn't been invoked as a node yet.
    fn recorded_argv(&self) -> Option<String> {
        // An EMPTY read is not a recording. The fake hsd records with
        // `echo "$@" > log`, and the shell creates and truncates that file
        // before `echo` writes a byte into it — so between the two syscalls
        // the file exists and is empty. Returning `Some("")` there ended the
        // caller's poll loop on the first tick and left it asserting against
        // an empty argv, which is the flake this helper caused: about one run
        // in twenty, with a message claiming hsd was never told its network.
        let s = std::fs::read_to_string(self.dir.join("fake-hsd-argv.log")).ok()?;
        if s.trim().is_empty() {
            return None;
        }
        Some(s)
    }
}

/// In-memory DB wired to the harness: `hsd_path` → fake binary, `hsd_prefix` →
/// data dir, `node_rpc_url` → the given URL (mockito or an unroutable address).
fn conn_for(h: &Harness, node_rpc_url: &str) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    db::queries::set_setting(&conn, "hsd_path", h.binary.to_str().unwrap()).unwrap();
    db::queries::set_setting(&conn, "hsd_prefix", h.data_dir().to_str().unwrap()).unwrap();
    db::queries::set_setting(&conn, "node_rpc_url", node_rpc_url).unwrap();
    // `start_hsd` needs an active profile to know which network to launch: it
    // refuses rather than guessing mainnet, since guessing would begin a full
    // mainnet chain sync in the user's data dir. Seed a mainnet one so these
    // process-lifecycle tests exercise the spawn paths, not that guard. Tests
    // that care about another network insert their own and re-point
    // `set_active_profile`.
    db::queries::insert_wallet_profile(
        &conn,
        "lifecycle-default",
        "Lifecycle",
        "watch_only_xpub",
        "mainnet",
        "xpub_placeholder",
        0,
        true,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, "lifecycle-default").unwrap();
    conn
}

fn app_with(conn: rusqlite::Connection) -> tauri::App<tauri::test::MockRuntime> {
    mock_builder()
        .manage(AppState {
            db: std::sync::Mutex::new(conn),
            signer: std::sync::Mutex::new(None),
            secure_prompts: std::sync::Mutex::new(std::collections::HashMap::new()),
            hsd_children: std::sync::Mutex::new(std::collections::HashMap::new()),
            node_rpc_alive: std::sync::atomic::AtomicBool::new(false),
            sync_status: std::sync::Arc::new(tokio::sync::Mutex::new(
                crate::commands::sync::SyncStatus::default(),
            )),
        })
        .build(mock_context(noop_assets()))
        .expect("mock app")
}

/// An unroutable RPC URL so `probe_node` fails fast and deterministically
/// (port 1 is never listening).
const NO_RPC: &str = "http://127.0.0.1:1";

/// Set up a mockito server whose RPC **fails the first probe, then succeeds**.
///
/// `start_hsd` probes RPC once up front to adopt an already-running node; if
/// that first probe answered we'd never spawn our fake binary (the adoption
/// path returns early). So the first `getblockchaininfo` hit returns HTTP 503
/// (→ `probe_node` yields `None` → we fall through to spawn), and every
/// subsequent hit — the wait-loop probes after the child is up — returns a
/// healthy blockchain-info with the given height.
///
/// Returns the guards (kept alive for the test's duration) and the URL.
async fn spawn_then_up_server(height: i64) -> (mockito::ServerGuard, mockito::Mock, mockito::Mock) {
    let mut server = mockito::Server::new_async().await;
    let fail_once = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_status(503)
        .expect(1)
        .create_async()
        .await;
    let then_up = server
        .mock("POST", "/")
        .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
        .with_body(format!(
            r#"{{"result":{{"blocks":{height},"headers":{height},"verificationprogress":1.0}},"error":null,"id":1}}"#,
        ))
        .expect_at_least(1)
        .create_async()
        .await;
    (server, fail_once, then_up)
}

// ===========================================================================
// start_hsd: spawn succeeds, RPC comes up → connected, child alive.
// ===========================================================================
#[tokio::test]
#[serial(hsd_home)]
async fn start_hsd_spawns_binary_and_reports_connected_when_rpc_answers() {
    // Fail the adoption probe so we actually spawn, then answer the loop probe.
    let (server, _fail, _up) = spawn_then_up_server(42).await;
    let h = Harness::new(FakeMode::StayAlive, "8.5.0");
    let app = app_with(conn_for(&h, &server.url()));
    let state = app.state::<AppState>();

    let v = start_hsd(app.state()).await.expect("start_hsd ok");

    assert_eq!(v["connected"], serde_json::json!(true));
    assert_eq!(v["process_alive"], serde_json::json!(true));
    assert_eq!(v["height"], serde_json::json!(42));
    // A child WAS spawned this time (unlike the pure-adoption path).
    assert!(!state.hsd_children.lock().unwrap().is_empty());
    assert!(state
        .node_rpc_alive
        .load(std::sync::atomic::Ordering::Relaxed));
    // An audit-log row was written for the spawn.
    {
        let db = state.db.lock().unwrap();
        let n: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM audit_log WHERE action = 'start_hsd'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    // Clean up the spawned child so it doesn't linger.
    stop_hsd(app.state()).await.expect("stop ok");
}

// ===========================================================================
// start_hsd: a regtest active profile MUST spawn hsd with `--network=regtest`
// (not the silent-mainnet bare `--regtest`). This is the tight red-capable loop
// for the "network selection" bug: if a future refactor drops the network flag,
// or resolves the wrong profile, this assertion fails. We also assert `--prefix`
// is the *un-scoped base* (the `hsd_prefix` setting) — hsd itself appends the
// `regtest` subdir from `--network`, so passing the already-scoped
// `<base>/regtest` here would double-nest to `<base>/regtest/regtest`.
// ===========================================================================
#[tokio::test]
#[serial(hsd_home)]
async fn start_hsd_passes_regtest_flag_and_scoped_prefix_for_regtest_profile() {
    let (server, _fail, _up) = spawn_then_up_server(1).await;
    let h = Harness::new(FakeMode::StayAlive, "8.5.0");
    let conn = conn_for(&h, &server.url());
    // Swap the seeded mainnet profile out for a regtest one.
    db::queries::insert_wallet_profile(
        &conn,
        "p-regtest",
        "Regtest",
        "watch_only_xpub",
        "regtest",
        "xpub_placeholder",
        0,
        true,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, "p-regtest").unwrap();

    let app = app_with(conn);
    let state = app.state::<AppState>();
    // The un-scoped base we configured as `hsd_prefix`; hsd must receive exactly
    // this as `--prefix`, then scope itself via `--network`.
    let base_prefix = h.data_dir().to_str().unwrap().to_string();

    let v = start_hsd(app.state()).await.expect("start_hsd ok");
    assert_eq!(v["network"], serde_json::json!("regtest"));
    assert!(!state.hsd_children.lock().unwrap().is_empty());

    // The child had a moment to run its `echo` before it started sleeping;
    // the RPC probe loop in start_hsd waits until mockito answers, which is
    // more than enough time for our one-line echo. Poll briefly just in case.
    let mut argv: Option<String> = None;
    for _ in 0..50 {
        if let Some(s) = h.recorded_argv() {
            argv = Some(s);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let argv = argv.expect("fake hsd should have recorded its argv");

    // The two must-have facts for a regtest launch:
    // hsd 8.x only honors the `--network=<name>` form; a bare `--regtest`
    // is ignored and silently falls back to mainnet (the contamination bug).
    assert!(
        argv.contains("--network=regtest"),
        "hsd was NOT told to run on regtest via --network=; argv was: {argv}"
    );
    // The bare positional form must NOT be used — it's the silent-mainnet trap.
    assert!(
        !argv.contains("--regtest"),
        "hsd was spawned with the bare --regtest flag (silent mainnet fallback); argv was: {argv}"
    );
    assert!(
        argv.contains("--prefix="),
        "hsd was spawned without --prefix; argv was: {argv}"
    );
    // The prefix must be the un-scoped *base*, not a hand-scoped
    // `<base>/regtest`: hsd appends the `regtest` subdir itself from
    // `--network=regtest`. Passing `<base>/regtest` here would double-nest the
    // chain to `<base>/regtest/regtest`. Assert the exact base is present and
    // that no `<base>/regtest` prefix token leaked in.
    assert!(
        argv.contains(&format!("--prefix={base_prefix}")),
        "hsd --prefix was not the un-scoped base ({base_prefix}); argv was: {argv}"
    );
    let scoped = std::path::Path::new(&base_prefix).join("regtest");
    assert!(
        !argv.contains(&format!("--prefix={}", scoped.to_str().unwrap())),
        "hsd --prefix was hand-scoped to <base>/regtest (double-nest risk); argv was: {argv}"
    );
    // No cross-network flags leaked in.
    assert!(
        !argv.contains("--network=testnet"),
        "unexpected testnet: {argv}"
    );
    assert!(
        !argv.contains("--network=simnet"),
        "unexpected simnet: {argv}"
    );
    assert!(
        !argv.contains("--network=main"),
        "unexpected mainnet: {argv}"
    );

    stop_hsd(app.state()).await.expect("stop ok");
}

// ===========================================================================
// start_hsd: SPV mode + a testnet profile drive the `--spv` arg branch and the
// `Network::Testnet` match arm (the default full-index/mainnet path is covered
// by the test above). We can't observe the child's argv directly, but running
// these settings through the spawn path executes those otherwise-uncovered
// branches; success (connected) confirms the spawn still works.
// ===========================================================================
#[tokio::test]
#[serial(hsd_home)]
async fn start_hsd_spawns_in_spv_mode_on_testnet() {
    let (server, _fail, _up) = spawn_then_up_server(7).await;
    let h = Harness::new(FakeMode::StayAlive, "8.5.0");
    let conn = conn_for(&h, &server.url());
    // SPV node mode → the `--spv` branch instead of --index-address/--index-tx.
    db::queries::set_setting(&conn, "node_mode", "spv").unwrap();
    // A testnet active profile → the Network::Testnet arm.
    db::queries::insert_wallet_profile(
        &conn,
        "p1",
        "test",
        "watch_only_xpub",
        "testnet",
        "xpub_placeholder",
        0,
        true,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, "p1").unwrap();

    let app = app_with(conn);
    let state = app.state::<AppState>();

    let v = start_hsd(app.state()).await.expect("start_hsd ok");
    assert_eq!(v["connected"], serde_json::json!(true));
    assert_eq!(v["network"], serde_json::json!("testnet"));
    assert!(!state.hsd_children.lock().unwrap().is_empty());

    stop_hsd(app.state()).await.expect("stop ok");
}

// ===========================================================================
// start_hsd: an *unparseable* `--version` output takes the `None` arm of the
// version gate — it warns and proceeds to spawn rather than blocking.
// ===========================================================================
#[tokio::test]
#[serial(hsd_home)]
async fn start_hsd_proceeds_when_version_unparseable() {
    let (server, _fail, _up) = spawn_then_up_server(3).await;
    // A version string with no numeric semver prefix → parse_hsd_version None.
    let h = Harness::new(FakeMode::StayAlive, "hsd-nightly-unknown");
    let app = app_with(conn_for(&h, &server.url()));
    let state = app.state::<AppState>();

    let v = start_hsd(app.state())
        .await
        .expect("start_hsd ok despite bad version");
    assert_eq!(v["connected"], serde_json::json!(true));
    assert!(!state.hsd_children.lock().unwrap().is_empty());

    stop_hsd(app.state()).await.expect("stop ok");
}

// ===========================================================================
// start_hsd: the spawned child dies immediately with a LOCK message → the
// "already running on this data directory" branch, log tail surfaced.
// ===========================================================================
#[tokio::test]
#[serial(hsd_home)]
async fn start_hsd_surfaces_data_dir_lock_when_child_dies_with_lock() {
    let h = Harness::new(FakeMode::DieLock, "8.5.0");
    // No RPC ever answers → the wait-loop notices the child exited.
    let app = app_with(conn_for(&h, NO_RPC));
    let state = app.state::<AppState>();

    let err = start_hsd(app.state())
        .await
        .expect_err("should fail on LOCK");
    let msg = err.to_string();
    assert!(
        msg.contains("already running on this data directory"),
        "expected data-dir-lock message, got: {msg}"
    );
    // The log tail is included in the error.
    assert!(
        msg.contains("LOCK"),
        "expected log tail with LOCK, got: {msg}"
    );
    // The dead child was reaped from the handle.
    assert!(state.hsd_children.lock().unwrap().is_empty());
}

// ===========================================================================
// start_hsd: the spawned child dies with a generic error → the
// "hsd exited on startup" branch with the log tail.
// ===========================================================================
#[tokio::test]
#[serial(hsd_home)]
async fn start_hsd_surfaces_generic_exit_when_child_dies() {
    let h = Harness::new(FakeMode::DieError, "8.5.0");
    let app = app_with(conn_for(&h, NO_RPC));
    let state = app.state::<AppState>();

    let err = start_hsd(app.state())
        .await
        .expect_err("should fail on exit");
    let msg = err.to_string();
    assert!(
        msg.contains("hsd exited on startup"),
        "expected startup-exit message, got: {msg}"
    );
    assert!(
        msg.contains("something went wrong"),
        "expected log tail in error, got: {msg}"
    );
    assert!(state.hsd_children.lock().unwrap().is_empty());
}

// ===========================================================================
// start_hsd: a too-old hsd version is refused BEFORE any spawn.
// ===========================================================================
#[tokio::test]
#[serial(hsd_home)]
async fn start_hsd_refuses_too_old_version_before_spawn() {
    // Version 7.x < the 8.0.0 minimum. `die-lock` mode would fail loudly if we
    // ever reached spawn — but we must not.
    let h = Harness::new(FakeMode::DieLock, "7.99.0");
    let app = app_with(conn_for(&h, NO_RPC));
    let state = app.state::<AppState>();

    let err = start_hsd(app.state())
        .await
        .expect_err("should refuse old version");
    let msg = err.to_string();
    assert!(
        msg.contains("older than the minimum supported"),
        "expected version-gate message, got: {msg}"
    );
    // No child spawned: we bailed before the spawn.
    assert!(state.hsd_children.lock().unwrap().is_empty());
}

// ===========================================================================
// stop_hsd: kills the child we spawned and clears the handle + alive flag.
// ===========================================================================
#[tokio::test]
#[serial(hsd_home)]
async fn stop_hsd_kills_spawned_child_and_clears_handle() {
    let (server, _fail, _up) = spawn_then_up_server(9).await;
    let h = Harness::new(FakeMode::StayAlive, "8.5.0");
    let app = app_with(conn_for(&h, &server.url()));
    let state = app.state::<AppState>();

    start_hsd(app.state()).await.expect("start ok");
    assert!(!state.hsd_children.lock().unwrap().is_empty());
    state
        .node_rpc_alive
        .store(true, std::sync::atomic::Ordering::Relaxed);

    stop_hsd(app.state()).await.expect("stop ok");

    // The child handle was taken + killed.
    assert!(state.hsd_children.lock().unwrap().is_empty());
    // The alive flag was cleared.
    assert!(!state
        .node_rpc_alive
        .load(std::sync::atomic::Ordering::Relaxed));
    // A stop_hsd audit row was written.
    {
        let db = state.db.lock().unwrap();
        let n: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM audit_log WHERE action = 'stop_hsd'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }
}

// ===========================================================================
// resync_hsd_chain: moves existing chain artifacts into a timestamped backup
// dir, then respawns hsd. We seed a mainnet chain layout (blocks/chain/tree)
// and assert they end up under a `_noindex-backup-*` dir.
// ===========================================================================
#[tokio::test]
#[serial(hsd_home)]
async fn resync_hsd_chain_backs_up_chain_data_and_respawns() {
    let (server, _fail, _up) = spawn_then_up_server(1).await;
    let h = Harness::new(FakeMode::StayAlive, "8.5.0");
    // Mainnet layout: blocks/, chain/, tree/ directly under the prefix — the
    // harness seeds a mainnet profile, and mainnet is the one network whose
    // data dir is the prefix root rather than a subdirectory of it.
    for sub in ["blocks", "chain", "tree"] {
        let p = h.data_dir().join(sub);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("marker"), b"data").unwrap();
    }

    let app = app_with(conn_for(&h, &server.url()));
    let state = app.state::<AppState>();

    let v = resync_hsd_chain(app.state()).await.expect("resync ok");
    // resync delegates to start_hsd, which reports connected via mocked RPC.
    assert_eq!(v["connected"], serde_json::json!(true));

    // The original chain dirs were moved aside.
    assert!(!h.data_dir().join("blocks").exists());
    assert!(!h.data_dir().join("chain").exists());
    assert!(!h.data_dir().join("tree").exists());

    // A backup dir exists containing the moved artifacts.
    let backup = std::fs::read_dir(h.data_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("_noindex-backup-"))
                    .unwrap_or(false)
        })
        .expect("a _noindex-backup-* dir should exist");
    assert!(backup.join("blocks").join("marker").exists());
    assert!(backup.join("chain").exists());
    assert!(backup.join("tree").exists());

    // An audit row records the resync.
    {
        let db = state.db.lock().unwrap();
        let n: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM audit_log WHERE action = 'resync_hsd_chain'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    stop_hsd(app.state()).await.expect("stop ok");
}

/// `resync_hsd_chain` moves chain data out of the way, so it must know which
/// network's data that is. Without an active profile it refuses, for the same
/// reason `start_hsd` does: guessing mainnet would back up — and then re-sync
/// over — a directory belonging to a network the user is not on.
#[tokio::test]
#[serial(hsd_home)]
async fn resync_hsd_chain_refuses_without_an_active_profile() {
    let (server, _fail, _up) = spawn_then_up_server(1).await;
    let h = Harness::new(FakeMode::StayAlive, "8.5.0");
    let conn = conn_for(&h, &server.url());
    // Drop the profile the harness seeds, leaving the wallet with none.
    db::queries::set_setting(&conn, "active_wallet_profile_id", "").unwrap();
    conn.execute("DELETE FROM wallet_profiles", []).unwrap();

    let app = app_with(conn);
    let err = resync_hsd_chain(app.state())
        .await
        .expect_err("no profile means no chain to re-sync");
    assert!(
        format!("{err}").contains("no active wallet profile"),
        "the refusal should say what is missing, got: {err}"
    );
}

// ===========================================================================
// Spec 2026-10-24 R2: starting a node never rewrites the global URL. The
// global URL is read by every other profile, so a start for regtest must not
// turn the mainnet wallet's default into 14037. A local-node profile already
// reaches its own port (R1) and never asks for a rewrite; a remote-node
// profile whose global URL happens to be another network's loopback default
// is the case the old realign still wrote.
// ===========================================================================
#[tokio::test]
#[serial(hsd_home)]
async fn starting_a_node_leaves_the_global_url_as_it_was() {
    let h = Harness::new(FakeMode::DieError, "8.5.0");
    let conn = conn_for(&h, "http://127.0.0.1:12037");
    db::queries::set_setting(&conn, "chain_source", "remote_node").unwrap();
    db::queries::insert_wallet_profile(
        &conn,
        "p-regtest",
        "Regtest",
        "watch_only_xpub",
        "regtest",
        "xpub_placeholder",
        0,
        true,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, "p-regtest").unwrap();
    let app = app_with(conn);
    let state = app.state::<AppState>();

    // The fake exits at once; whatever the outcome, the setting is the point.
    let _ = start_hsd(app.state()).await;

    let db = state.db.lock().unwrap();
    let url = db::queries::get_settings(&db).unwrap()["node_rpc_url"].clone();
    assert_eq!(url, "http://127.0.0.1:12037");
}

// ===========================================================================
// Spec 2026-10-24 R10: the spawn flags come from the started network's own
// mode. Regtest set to SPV starts `--spv` even while the app-wide mode is
// full, and a network with no mode of its own follows the app-wide one.
// ===========================================================================
#[tokio::test]
#[serial(hsd_home)]
async fn the_networks_mode_picks_the_spawn_flags() {
    let (server, _fail, _up) = spawn_then_up_server(3).await;
    let h = Harness::new(FakeMode::StayAlive, "8.5.0");
    let conn = conn_for(&h, &server.url());
    db::queries::set_setting(&conn, "node_mode", "full").unwrap();
    db::queries::set_setting(&conn, "node_mode.regtest", "spv").unwrap();
    db::queries::insert_wallet_profile(
        &conn,
        "p-regtest",
        "Regtest",
        "watch_only_xpub",
        "regtest",
        "xpub_placeholder",
        0,
        true,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, "p-regtest").unwrap();
    let app = app_with(conn);

    start_hsd(app.state()).await.expect("start_hsd ok");
    let mut argv = None;
    for _ in 0..50 {
        if let Some(s) = h.recorded_argv() {
            argv = Some(s);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    stop_hsd(app.state()).await.expect("stop ok");

    let argv = argv.expect("fake hsd recorded its argv");
    assert!(argv.contains("--spv"), "{argv}");
    assert!(!argv.contains("--index-address"), "{argv}");
}

// ===========================================================================
// Spec 2026-10-24 R6: the node the app spawns is recorded in the network's
// data directory as its own, and a stop forgets it.
// ===========================================================================
#[tokio::test]
#[serial(hsd_home)]
async fn a_spawned_node_is_recorded_as_own_until_it_is_stopped() {
    let (server, _fail, _up) = spawn_then_up_server(5).await;
    let h = Harness::new(FakeMode::StayAlive, "8.5.0");
    let app = app_with(conn_for(&h, &server.url())); // mainnet: data dir = base
    let state = app.state::<AppState>();

    start_hsd(app.state()).await.expect("start_hsd ok");
    let child_pid = state
        .hsd_children
        .lock()
        .unwrap()
        .values()
        .next()
        .map(|c| c.id());
    assert!(child_pid.is_some());
    assert_eq!(
        crate::commands::hsd_pid::own_node_pid(h.data_dir()),
        child_pid,
        "the child is the network's own node"
    );

    stop_hsd(app.state()).await.expect("stop ok");
    assert!(!h
        .data_dir()
        .join(crate::commands::hsd_pid::PID_FILE)
        .exists());
}

// ===========================================================================
// Spec 2026-10-24 R3/R5: one own node per network. Starting testnet's node
// leaves regtest's running, and stopping testnet's leaves regtest's alone.
// ===========================================================================

fn add_profile_on(conn: &rusqlite::Connection, id: &str, network: &str, url: &str) {
    db::queries::insert_wallet_profile(
        conn,
        id,
        id,
        "watch_only_xpub",
        network,
        "xpub_placeholder",
        0,
        true,
    )
    .unwrap();
    crate::tests::command_helpers::set_profile_override(conn, id, "node_rpc_url", url);
}

fn activate(app: &tauri::App<tauri::test::MockRuntime>, id: &str) {
    let state = app.state::<AppState>();
    let db = state.db.lock().unwrap();
    db::queries::set_active_profile(&db, id).unwrap();
}

fn child_alive(app: &tauri::App<tauri::test::MockRuntime>, network: Network) -> bool {
    let state = app.state::<AppState>();
    let mut children = state.hsd_children.lock().unwrap();
    children
        .get_mut(&network)
        .is_some_and(|c| matches!(c.try_wait(), Ok(None)))
}

#[tokio::test]
#[serial(hsd_home)]
async fn two_networks_run_side_by_side() {
    let (regtest_node, _f1, _u1) = spawn_then_up_server(1).await;
    let (testnet_node, _f2, _u2) = spawn_then_up_server(1).await;
    let h = Harness::new(FakeMode::StayAlive, "8.5.0");
    let conn = conn_for(&h, NO_RPC);
    add_profile_on(&conn, "p-regtest", "regtest", &regtest_node.url());
    add_profile_on(&conn, "p-testnet", "testnet", &testnet_node.url());
    let app = app_with(conn);

    activate(&app, "p-regtest");
    start_hsd(app.state()).await.expect("regtest starts");
    activate(&app, "p-testnet");
    start_hsd(app.state())
        .await
        .expect("testnet starts beside regtest");
    assert!(
        child_alive(&app, Network::Regtest),
        "starting testnet stopped regtest"
    );
    assert!(child_alive(&app, Network::Testnet));

    stop_hsd(app.state()).await.expect("stop testnet");
    assert!(!child_alive(&app, Network::Testnet));
    assert!(
        child_alive(&app, Network::Regtest),
        "stopping testnet stopped regtest"
    );

    activate(&app, "p-regtest");
    stop_hsd(app.state()).await.expect("stop regtest");
}

/// Only this network's own node already running stops a start.
#[tokio::test]
#[serial(hsd_home)]
async fn a_second_start_of_the_same_network_is_refused() {
    let (node, _f, _u) = spawn_then_up_server(1).await;
    let h = Harness::new(FakeMode::StayAlive, "8.5.0");
    let conn = conn_for(&h, NO_RPC);
    add_profile_on(&conn, "p-regtest", "regtest", &node.url());
    let app = app_with(conn);
    activate(&app, "p-regtest");

    start_hsd(app.state()).await.expect("regtest starts");
    let err = start_hsd(app.state()).await.expect_err("already running");
    assert_eq!(err.to_string(), "The regtest node is already running.");
    stop_hsd(app.state()).await.expect("stop");
}

/// Spec 2026-10-24 R6/R7: a node an earlier session started and left for the
/// daemon is still the app's own, so Stop ends it and forgets its record.
#[tokio::test]
#[serial(hsd_home)]
async fn stop_ends_an_own_node_left_by_an_earlier_session() {
    let h = Harness::new(FakeMode::StayAlive, "8.5.0");
    let conn = conn_for(&h, NO_RPC);
    add_profile_on(&conn, "p-regtest", "regtest", NO_RPC);
    let regtest_dir = h.data_dir().join("regtest");
    std::fs::create_dir_all(&regtest_dir).unwrap();
    // The "earlier session's" node: a process this test started itself.
    let mut orphan = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
    crate::commands::hsd_pid::write_pid_file(&regtest_dir, orphan.id()).unwrap();
    let app = app_with(conn);
    activate(&app, "p-regtest");

    stop_hsd(app.state()).await.expect("stop the own orphan");

    let status = orphan.wait().unwrap();
    assert!(
        !status.success(),
        "the orphan was terminated, not left to finish"
    );
    assert!(!regtest_dir
        .join(crate::commands::hsd_pid::PID_FILE)
        .exists());
}

// ===========================================================================
// Spec 2026-10-24 R4/R5: a change of the active wallet names the network
// whose node to start, once per change, and only for a local-node wallet
// whose network starts on its own. The first look (app start) counts as a
// change. Starting it is `start_hsd`'s job, which never stops another
// network's node.
// ===========================================================================
#[test]
fn a_change_of_the_active_wallet_names_the_node_to_start() {
    use crate::commands::node::autostart_on_profile_change;
    let h = Harness::new(FakeMode::StayAlive, "8.5.0");
    let conn = conn_for(&h, NO_RPC); // active: a mainnet local-node wallet
    add_profile_on(&conn, "p-regtest", "regtest", NO_RPC);
    add_profile_on(&conn, "p-remote", "testnet", NO_RPC);
    crate::tests::command_helpers::set_profile_override(
        &conn,
        "p-remote",
        "chain_source",
        "remote_node",
    );
    let mut last = None;

    assert_eq!(
        autostart_on_profile_change(&conn, &mut last),
        Some(Network::Main),
        "app start"
    );
    assert_eq!(
        autostart_on_profile_change(&conn, &mut last),
        None,
        "no change, no start"
    );

    db::queries::set_active_profile(&conn, "p-regtest").unwrap();
    assert_eq!(
        autostart_on_profile_change(&conn, &mut last),
        Some(Network::Regtest)
    );
    assert_eq!(autostart_on_profile_change(&conn, &mut last), None);

    db::queries::set_active_profile(&conn, "p-remote").unwrap();
    assert_eq!(
        autostart_on_profile_change(&conn, &mut last),
        None,
        "a remote node starts nothing"
    );

    db::queries::set_setting(&conn, "autostart_hsd.regtest", "false").unwrap();
    db::queries::set_active_profile(&conn, "p-regtest").unwrap();
    assert_eq!(
        autostart_on_profile_change(&conn, &mut last),
        None,
        "autostart off"
    );
}

// ===========================================================================
// Spec 2026-10-24 R9: quit treats every own node by one rule. With background
// sync on, every own node is left for the daemon with its record; with it
// off, every own node is stopped, an earlier session's included, and its
// record removed.
// ===========================================================================

/// Two own children (regtest, testnet) and an earlier session's own node on
/// mainnet, recorded by its pid file.
async fn app_with_three_own_nodes(
    h: &Harness,
) -> (
    tauri::App<tauri::test::MockRuntime>,
    std::process::Child,
    [mockito::ServerGuard; 2],
) {
    let (regtest_node, _f1, _u1) = spawn_then_up_server(1).await;
    let (testnet_node, _f2, _u2) = spawn_then_up_server(1).await;
    let conn = conn_for(h, NO_RPC);
    add_profile_on(&conn, "p-regtest", "regtest", &regtest_node.url());
    add_profile_on(&conn, "p-testnet", "testnet", &testnet_node.url());
    let orphan = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
    crate::commands::hsd_pid::write_pid_file(h.data_dir(), orphan.id()).unwrap();
    let app = app_with(conn);
    activate(&app, "p-regtest");
    start_hsd(app.state()).await.expect("regtest starts");
    activate(&app, "p-testnet");
    start_hsd(app.state()).await.expect("testnet starts");
    (app, orphan, [regtest_node, testnet_node])
}

fn pid_files(h: &Harness) -> [bool; 3] {
    let f = crate::commands::hsd_pid::PID_FILE;
    [
        h.data_dir().join(f).exists(),
        h.data_dir().join("regtest").join(f).exists(),
        h.data_dir().join("testnet").join(f).exists(),
    ]
}

#[tokio::test]
#[serial(hsd_home)]
async fn quit_with_background_sync_leaves_every_own_node() {
    let h = Harness::new(FakeMode::StayAlive, "8.5.0");
    let (app, mut orphan, _servers) = app_with_three_own_nodes(&h).await;
    let pids: Vec<u32> = app
        .state::<AppState>()
        .hsd_children
        .lock()
        .unwrap()
        .values()
        .map(|c| c.id())
        .collect();

    crate::commands::node::release_own_nodes_on_quit(&app.state::<AppState>(), true);

    assert!(
        app.state::<AppState>()
            .hsd_children
            .lock()
            .unwrap()
            .is_empty(),
        "handles let go"
    );
    for pid in &pids {
        assert!(
            crate::commands::hsd_pid::process_start_time(*pid).is_some(),
            "{pid} left running"
        );
    }
    assert!(
        orphan.try_wait().unwrap().is_none(),
        "the earlier session's node left running"
    );
    assert_eq!(
        pid_files(&h),
        [true, true, true],
        "records kept for the next session"
    );
    for pid in pids {
        unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    }
    let _ = orphan.kill();
}

#[tokio::test]
#[serial(hsd_home)]
async fn quit_without_background_sync_stops_every_own_node() {
    let h = Harness::new(FakeMode::StayAlive, "8.5.0");
    let (app, mut orphan, _servers) = app_with_three_own_nodes(&h).await;

    crate::commands::node::release_own_nodes_on_quit(&app.state::<AppState>(), false);

    assert!(app
        .state::<AppState>()
        .hsd_children
        .lock()
        .unwrap()
        .is_empty());
    assert!(
        !orphan.wait().unwrap().success(),
        "the earlier session's node was stopped"
    );
    assert_eq!(pid_files(&h), [false, false, false], "records removed");
}
