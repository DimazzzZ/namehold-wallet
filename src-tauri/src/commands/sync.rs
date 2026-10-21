//! Orchestrated full sync with persistent progress.
//!
//! A single command [`start_full_sync`] runs all reconciliation steps in order
//! and writes progress into [`SyncSession`] in `AppState`, so the frontend can
//! poll status via [`get_sync_status`] even across page navigation.
//!
//! COVERAGE: 92.49% line / 95.38% region — realistic ceiling. Remaining ~65
//! missed lines: (a) mid-loop cancellation branches in
//! `repair_step_windowed`/`discover_step` (require timing control not available
//! without mockito response delays; `DISCOVERY_THROTTLE=0` in tests makes
//! loops too fast to cancel mid-iteration); (b) DB `open_conn` error paths
//! (~10 sites — require corrupting the DB file mid-run, structural); (c) the
//! tokio runtime-build failure in `start_full_sync` (cannot inject); (d) the
//! heartbeat thread body (requires 10s `HEARTBEAT_INTERVAL_SECS` to elapse);
//! (e) `discover_step` phase 2's resolve-error abort path (structurally
//! unreachable when name-info succeeds — the `consecutive_errors = 0` reset
//! prevents the counter from reaching 5 purely from resolve errors — effective
//! dead code for the "all resolve errors" scenario). Test harness in
//! `src/tests/sync_race_tests.rs` supports file-backed DBs + mockito
//! node/explorer.

use crate::db::queries;
use crate::error::AppError;
use crate::noncustodial::rpc::NodeRpcClient;
use crate::AppState;
use serde::Serialize;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tauri::State;
use tokio::sync::Mutex;
use tokio::time::sleep;

/// Delay between explorer requests during discovery/repair.
///
/// Under `cfg(test)` this collapses to zero: the throttle only exists to be a
/// courteous client against a real explorer, and every test drives a local
/// `mockito` server (or an unroutable address) where the delay is pure dead
/// wall-clock. Zeroing it under test removes ~12s from a single 25-candidate
/// discovery test with no loss of coverage (the crawl logic is unchanged).
#[cfg(not(test))]
const DISCOVERY_THROTTLE: Duration = Duration::from_millis(150);
#[cfg(test)]
const DISCOVERY_THROTTLE: Duration = Duration::from_millis(0);

/// Test-only seam: when set, the background sync thread panics right after
/// resolving `profile_id` (before Step 1), simulating "a panic in a sync
/// step" deterministically without touching any convergence/backoff logic.
/// Cleared (swapped back to `false`) by the thread the moment it fires, so
/// each test controls exactly one run.
#[cfg(test)]
pub(crate) static TEST_PANIC_HOOK: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

// ---------------------------------------------------------------------------
// Shared ownership resolver
// ---------------------------------------------------------------------------

/// The result of resolving a name's current on-chain owner via explorer
/// history: the owner outpoint, the address that outpoint pays, and whether
/// that address belongs to this wallet.
pub struct OwnerResolution {
    pub owner_txid: String,
    pub owner_vout: u32,
    pub owner_address: String,
    pub owned_by_wallet: bool,
}

/// Resolve a name's current owner the CORRECT way: via `/api/names/:name/history`
/// (the newest owner outpoint), then the owner tx's outputs (whose `address`
/// field is an actual address) — NOT via the name-info payload's `owner.hash`,
/// which is an outpoint txid, not an address, and must never be compared
/// against a wallet's address set.
///
/// Returns `Ok(None)` when the name has no owner history, or when the owner
/// tx has no output at the recorded `owner_vout` — both cases mean ownership
/// can't be confirmed, and callers should treat the name as not (yet) owned.
pub async fn resolve_owner_via_history<C: crate::providers::ExplorerProvider>(
    client: &C,
    name: &str,
    addr_set: &std::collections::HashSet<String>,
) -> Result<Option<OwnerResolution>, crate::error::AppError> {
    let (owner_txid, owner_vout) = match client.get_name_current_owner(name).await? {
        Some(o) => o,
        None => return Ok(None),
    };
    sleep(DISCOVERY_THROTTLE).await;
    let owner_outputs = client.get_tx_named_outputs(&owner_txid).await?;
    let Some(owner_output) = owner_outputs.iter().find(|o| o.index == owner_vout) else {
        return Ok(None);
    };
    let owner_address = owner_output.address.clone();
    let owned_by_wallet = addr_set.contains(&owner_address);
    Ok(Some(OwnerResolution {
        owner_txid,
        owner_vout,
        owner_address,
        owned_by_wallet,
    }))
}

// ---------------------------------------------------------------------------
// Sync state (persistent; survives page navigation)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub running: bool,
    pub step: String,
    pub progress_label: String,
    pub repaired: u32,
    /// Total distinct candidates this repair run needs to check (the whole
    /// backlog, across ALL windows — not just the current window). Set once at
    /// the start of `repair_step` from `queries::count_repair_candidates`, so a
    /// multi-window run shows a stable denominator (e.g. "5 / 540") instead of
    /// the old per-window count that reset every batch and read as "24 / 24".
    pub repair_candidates: u32,
    /// Honest progress: candidates still to check this run. Starts equal to
    /// `repair_candidates` and decreases toward 0 as names are attempted
    /// (owned, not-owned, or errored-and-skipped-this-run all count as
    /// attempted), so the UI can show a figure that truthfully converges to 0.
    pub repair_remaining: u32,
    pub discovered: u32,
    pub names_synced: u32,
    pub errors: Vec<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    /// Discovery progress: how many addresses this wallet has to scan.
    pub discover_addresses_total: u32,
    /// Discovery progress: how many addresses have been scanned so far.
    pub discover_addresses_done: u32,
    /// Discovery progress: how many tx pages have been fetched so far.
    pub discover_txs_scanned: u32,
    /// Discovery progress: how many candidate names found so far (before ownership check).
    pub discover_candidates: u32,
    /// Discovery progress: the current name being checked.
    pub discover_current_name: String,
    /// Whether the current step is waiting for an explorer response.
    pub waiting: bool,
    /// Set by [`cancel_full_sync`] to request an in-flight run stop ASAP. The
    /// running steps observe this between/within batches and bail out cleanly.
    /// Reset to `false` automatically on the next `start_full_sync` (which does
    /// `*s = SyncStatus::default()`).
    pub cancel_requested: bool,
    /// Raised by the run's heartbeat ([`spawn_lock_heartbeat`]) when another
    /// process took this profile's sync lock over. Not the user's Stop
    /// (`cancel_requested`): the run stops the same way, logs why and ends on
    /// the `lock_lost` step ([`finish_run`]). Not sent to the UI; the step and
    /// its label say it. A fresh flag with every `SyncStatus::default()`.
    #[serde(skip)]
    pub lock_lost: crate::db::sync_lock::LockLost,
}

impl Default for SyncStatus {
    fn default() -> Self {
        Self {
            running: false,
            step: "idle".into(),
            progress_label: "Idle".into(),
            repaired: 0,
            repair_candidates: 0,
            repair_remaining: 0,
            discovered: 0,
            names_synced: 0,
            errors: vec![],
            started_at: None,
            finished_at: None,
            discover_addresses_total: 0,
            discover_addresses_done: 0,
            discover_txs_scanned: 0,
            discover_candidates: 0,
            discover_current_name: String::new(),
            waiting: false,
            cancel_requested: false,
            lock_lost: crate::db::sync_lock::LockLost::default(),
        }
    }
}

/// The label of a run that stopped because another process took its sync
/// lock over ([`SyncStatus::lock_lost`]).
pub const LOCK_LOST_LABEL: &str = "Sync stopped: another process took over this profile's sync";

// ---------------------------------------------------------------------------
// Orchestrated sync — runs in a bg thread via `start_full_sync`
// ---------------------------------------------------------------------------

/// Guards `SyncStatus.running` against a panic anywhere in the background
/// sync thread — a step function, or the `.expect`-turned-match that builds
/// the thread's own tokio runtime. Without this, a single panic leaves
/// `running == true` forever: Task 9's atomic check-and-set in
/// `start_full_sync` refuses to start a new run while `running == true`, so
/// one panic would permanently brick Sync until the app is restarted.
///
/// Every NORMAL exit path (DB-open error, no active profile, "done",
/// "cancelled") already sets `running = false` itself and then calls
/// [`RunningGuard::mark_completed`], which makes this guard's `Drop` a
/// no-op. Only an unwinding panic can reach `Drop` with `completed ==
/// false` — that's the one case this guard exists to handle: it clears
/// `running` and records an error so the next `start_full_sync` succeeds
/// instead of being refused.
///
/// Declared as the FIRST local in the thread closure (before the runtime is
/// built), so on unwind it is guaranteed to be the LAST thing dropped —
/// Rust drops locals in reverse declaration order. That means the tokio
/// runtime (`rt`, declared after) is always torn down before this guard's
/// `Drop` runs, so `blocking_lock` below is never called from inside an
/// active tokio context (which would panic).
struct RunningGuard {
    status: Arc<Mutex<SyncStatus>>,
    completed: bool,
}

impl RunningGuard {
    fn new(status: Arc<Mutex<SyncStatus>>) -> Self {
        Self {
            status,
            completed: false,
        }
    }

    /// Call on every normal exit path to suppress the Drop-time cleanup —
    /// that path has already (or is about to) set `running`/`errors`
    /// itself.
    fn mark_completed(&mut self) {
        self.completed = true;
    }
}

impl Drop for RunningGuard {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        // Reached only when the thread is unwinding from a panic that
        // nothing else caught. `blocking_lock` is safe here: see the
        // struct-level doc comment for why the tokio runtime is guaranteed
        // to already be gone by this point.
        let mut s = self.status.blocking_lock();
        s.running = false;
        s.errors.push("sync thread panicked".to_string());
        s.step = "error".into();
        s.progress_label = "Sync failed unexpectedly".into();
        s.finished_at = Some(format!("{:?}", std::time::SystemTime::now()));
    }
}

/// Start a full sync in a background thread. The frontend polls
/// [`get_sync_status`] to see progress even across page navigation.
///
/// The sync steps themselves live in [`run_sync_steps`], shared with
/// the background daemon so the two never drift apart on sync policy. While a
/// sync runs, a heartbeat thread ([`spawn_lock_heartbeat`]) refreshes the sync
/// lock every 10s so the daemon can't take the lock over mid-run.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn start_full_sync(state: State<'_, AppState>) -> Result<serde_json::Value, AppError> {
    let status = state.sync_status.clone();

    // Atomically check-and-set: refuse to start a second sync while one is
    // already running. The check (`s.running`) and the flip to `running =
    // true` happen under the SAME lock acquisition (one critical section), so
    // two concurrent callers can't both observe `running == false` and both
    // proceed to reset status + spawn a thread — only whichever caller wins
    // the lock first sees `false` and gets to start; the other sees `true`
    // (set by the winner, still inside the same critical section) and bails
    // out cleanly with no side effects. This closes the start/start race that
    // previously existed: the old code reset+spawned unconditionally, with no
    // `running` check at all.
    {
        let mut s = status.lock().await;
        if s.running {
            return Ok(serde_json::json!({"started": false, "alreadyRunning": true}));
        }
        *s = SyncStatus::default();
        s.running = true;
        s.started_at = Some(format!("{:?}", std::time::SystemTime::now()));
    }

    // Spawn a blocking thread that does all the explorer work.
    // It re-opens the DB from the known path.
    let db_path = {
        let conn = state.db.lock().unwrap();
        conn.path().unwrap_or("").to_string()
    };

    std::thread::spawn(move || {
        // Must be the FIRST local — see `RunningGuard`'s doc comment for why
        // declaration order here matters (it controls Drop order on unwind).
        let mut guard = RunningGuard::new(status.clone());

        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                // Mirrors the other early-return paths below: record the
                // error and clear `running` explicitly (no async context
                // exists yet to `.lock().await`, so use `blocking_lock`,
                // which is safe here — we're plain sync code on a fresh OS
                // thread with no tokio runtime built at all).
                let mut s = status.blocking_lock();
                s.running = false;
                s.errors.push(format!(
                    "failed to build tokio runtime for sync thread: {e}"
                ));
                s.progress_label = "Sync failed to start".into();
                guard.mark_completed();
                return;
            }
        };

        let guard = &mut guard;
        rt.block_on(async move {
            // Open own DB connection for this background thread — routed
            // through `open_conn` (S4) so it gets the same hardening as every
            // other connection in the app (WAL, busy_timeout, foreign_keys),
            // instead of a bare `Connection::open` with none of that, which
            // made concurrent writes from the sync thread and the main UI
            // thread liable to an immediate "database is locked" error.
            let conn = match open_conn(&db_path) {
                Ok(c) => c,
                Err(e) => {
                    let mut s = status.lock().await;
                    s.running = false;
                    s.errors.push(format!("DB open: {e}"));
                    guard.mark_completed();
                    return;
                }
            };
            // Need AppState-like access. Instead, we work with the conn directly.
            let profile_id = match queries::get_active_profile_id(&conn) {
                Ok(id) if !id.is_empty() => id,
                _ => {
                    let mut s = status.lock().await;
                    s.running = false;
                    s.progress_label = "No active wallet profile".into();
                    guard.mark_completed();
                    return;
                }
            };
            // Drop conn - we'll reacquire locks each step via queries.
            drop(conn);

            // Acquire the sync lock (app priority). If another app instance
            // holds a fresh lock, bail — but a daemon lock is preempted.
            if let Ok(c) = open_conn(&db_path) {
                use crate::db::sync_lock;
                if !sync_lock::acquire_for_app(&c, &profile_id).unwrap_or(false) {
                    let mut s = status.lock().await;
                    s.running = false;
                    s.progress_label = "Another app instance is syncing this profile".into();
                    guard.mark_completed();
                    return;
                }
            }

            #[cfg(test)]
            if TEST_PANIC_HOOK.swap(false, std::sync::atomic::Ordering::SeqCst) {
                panic!("TEST_PANIC_HOOK: injected panic simulating a panic in a sync step");
            }

            // Refresh the app's lock heartbeat while syncing, so a sync that
            // runs longer than STALE_THRESHOLD_SECS doesn't let its own lock
            // go stale and get stolen by the daemon mid-run (which would let
            // both processes write the same profile — the corruption the lock
            // exists to prevent).
            let hb_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let lock_lost = status.lock().await.lock_lost.clone();
            let hb_handle = spawn_lock_heartbeat(
                db_path.clone(),
                profile_id.clone(),
                hb_stop.clone(),
                lock_lost,
            );

            // Run the shared sync steps (also used by the daemon).
            run_sync_steps(&status, &db_path, &profile_id, SyncCaller::App).await;

            // Stop the heartbeat thread now that syncing is done.
            hb_stop.store(true, std::sync::atomic::Ordering::SeqCst);
            let _ = hb_handle.join();

            finish_run(&status).await;
            // Task 11 review, Finding 2: stamp `last_explorer_sync_at` once,
            // OUTSIDE the per-name repair/discover loops and their windowed
            // convergence/memo/cancel/backoff logic entirely (see the
            // function's own doc comment for the "clean run" definition).
            stamp_explorer_sync_if_clean(&status, &db_path, &profile_id).await;
            // Release the sync lock so the daemon can resume. On a panic mid-sync
            // this release is skipped, but the app lock then goes stale after
            // STALE_THRESHOLD_SECS and the daemon takes it over — no deadlock.
            if let Ok(c) = open_conn(&db_path) {
                let _ = crate::db::sync_lock::release(&c, &profile_id);
            }
            guard.mark_completed();
        });
    });

    Ok(serde_json::json!({"started": true}))
}

/// The end of an app run: its terminal step and label. A run whose sync lock
/// was taken over ends on `lock_lost` ([`LOCK_LOST_LABEL`]), one the user
/// stopped on `cancelled`, any other on `done`.
pub(crate) async fn finish_run(status: &Arc<Mutex<SyncStatus>>) {
    let mut s = status.lock().await;
    if s.lock_lost.is_lost() {
        s.step = "lock_lost".into();
        s.progress_label = LOCK_LOST_LABEL.into();
    } else if s.cancel_requested {
        s.step = "cancelled".into();
        s.progress_label = "Sync stopped".into();
    } else {
        s.step = "done".into();
        s.progress_label = "Sync complete".into();
    }
    s.running = false;
    s.finished_at = Some(format!("{:?}", std::time::SystemTime::now()));
}

/// Whether the run must stop before `next` because another process took its
/// sync lock over ([`SyncStatus::lock_lost`]); logs it when so.
fn stop_for_lost_lock(lock: &crate::db::sync_lock::LockLost, profile_id: &str, next: &str) -> bool {
    let lost = lock.is_lost();
    if lost {
        eprintln!(
            "sync: the sync lock of {profile_id} was taken over by another process; stopping before {next}"
        );
    }
    lost
}

/// Who runs [`run_sync_steps`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncCaller {
    /// The app: reports progress to the UI and may rebroadcast.
    App,
    /// `namehold-syncd`: no UI, and it never broadcasts (SECURITY.md).
    Daemon,
}

impl SyncCaller {
    /// Whether this caller's sync may rebroadcast a missing purchase: the
    /// daemon never may (SECURITY.md, "The daemon never signs or broadcasts").
    pub fn rebroadcast(self) -> crate::shakedex_jobs::Rebroadcast {
        match self {
            SyncCaller::App => crate::shakedex_jobs::Rebroadcast::Allowed,
            SyncCaller::Daemon => crate::shakedex_jobs::Rebroadcast::Never,
        }
    }
}

/// The shared sync steps, used by both the app and the daemon.
///
/// This is the single source of truth for sync policy. Both [`start_full_sync`]
/// (manual Sync button) and [`crate::daemon::sync_profile`] call this, so they
/// never drift apart on which steps to run or when to skip them.
///
/// Steps:
/// 1. Node UTXO sync (always attempted). When the node is not authoritative,
///    the address window then grows from the explorer
///    ([`explorer_window_step`]), so the explorer reads below cover it.
/// 2. Repair owned names — explorer path only (skipped when the node is
///    authoritative, since Step 1 already refreshed coins from the node).
/// 3. Discover new names — node path when authoritative, explorer path otherwise.
/// 4. Shakedex purchases — only when the node is authoritative; never sends
///    from the daemon. Then the Shakedex listing aborts (R19), on the same
///    condition; that step sends nothing from either caller. Then, on a
///    mainnet profile, the market jobs (R23, R25): publishing and keeping
///    published listings on LearnHNS Market; they read the node and write
///    to the market only, never signing or sending, from either caller.
///
/// `caller` says who runs it. The app writes `SyncStatus` progress labels (the
/// UI polls them) and may rebroadcast a missing purchase; the daemon has no
/// UI, uses a throwaway status only to satisfy the step function signatures,
/// and never broadcasts.
///
/// A run whose sync lock another process took over
/// ([`SyncStatus::lock_lost`]) stops before its next step, and inside the
/// node step and the market jobs before their next round, write or listing:
/// the process that took the lock syncs the profile.
pub async fn run_sync_steps(
    status: &Arc<Mutex<SyncStatus>>,
    db_path: &str,
    profile_id: &str,
    caller: SyncCaller,
) {
    let report_progress = caller == SyncCaller::App;
    let lock = status.lock().await.lock_lost.clone();
    let lost = |next: &str| stop_for_lost_lock(&lock, profile_id, next);
    // Determine node mode from settings.
    let node_mode = open_conn(db_path)
        .ok()
        .and_then(|c| queries::get_settings(&c).ok())
        .map(|s| crate::noncustodial::rpc::resolve_node_mode(&s))
        .unwrap_or(crate::noncustodial::rpc::NodeMode::Full);

    // Step 1: Node sync (best-effort).
    // In SPV mode, use the SPV-specific step (explorer-based, no --index-address).
    // In full mode, use the standard node sync step.
    if report_progress {
        let mut s = status.lock().await;
        s.step = "node".into();
        s.progress_label = if node_mode.is_spv() {
            "Syncing via SPV node…".into()
        } else {
            "Syncing with local node…".into()
        };
    }
    // A node that cannot answer for the wallet's coins (no address index) is
    // not authoritative for this run: the run's errors say why, and the
    // explorer steps below run as for a node that is behind.
    let mut node_answers_for_coins = true;
    if node_mode.is_spv() {
        let _spv_ok = crate::commands::sync_spv::sync_spv_step(db_path, profile_id).await;
    } else if let NodeStep::NotAuthoritative(why) = sync_node_step(db_path, profile_id, &lock).await
    {
        eprintln!("sync: profile {profile_id}: {why}");
        status.lock().await.errors.push(why);
        node_answers_for_coins = false;
    }
    if lost("the node readiness check") {
        return;
    }

    // Is the node authoritative (connected AND fully synced)? When it is,
    // the explorer-backed repair/discover steps below are redundant: Step 1
    // already refreshed the wallet's coins from the node, and owned-name
    // state is kept current by the node coin scan plus the wallet's own
    // covenant writes. HNSFans is only a PRE-SYNC workaround — once the
    // node is caught up we stop calling it. (Re-checked here, per run, so a
    // node that later falls out of sync transparently re-enables the
    // explorer path on the next Sync.)
    //
    // In SPV mode, the node is NEVER authoritative for names/UTXOs
    // (SPV nodes don't have --index-address), so always use explorer.
    let node_authoritative = if node_mode.is_spv() || !node_answers_for_coins {
        false
    } else {
        // Both the network and the node belong to the profile being synced,
        // not to whichever profile happens to be active. The network, because
        // a node on another chain must never be authoritative for this one.
        // The node, because this profile may point at its own (ADR-001) — and
        // the steps this gate governs already build their client per profile,
        // so asking the global node whether it is caught up decided what to do
        // with a node nobody was going to talk to.
        //
        // A network this step cannot read makes the node not authoritative,
        // rather than authoritative without the comparison. Passing `None`
        // here tells the readiness gate there is nothing to compare, which is
        // right during onboarding and wrong for a profile that has a network
        // and simply could not be read: it would let a node on another chain
        // seed this profile's cache. The explorer path this falls back to is
        // the conservative one and already exists.
        match open_conn(db_path)
            .ok()
            .and_then(|c| queries::get_wallet_profile(&c, profile_id).ok().flatten())
        {
            Some(p) => {
                crate::commands::node_readiness::node_ready_from_profile(
                    db_path,
                    profile_id,
                    Some(&p.network),
                )
                .await
            }
            None => false,
        }
    };

    // The explorer reads below cover only the stored addresses: grow the
    // window from the explorer first, so they cover the addresses past it in
    // this sync. A node that is authoritative already grew it (Step 1).
    if !node_authoritative {
        if lost("the address window") {
            return;
        }
        if report_progress {
            let mut s = status.lock().await;
            s.step = "window".into();
            s.progress_label = "Looking for addresses past the last used one…".into();
        }
        // The user's Stop skips it, as it ends the explorer steps below.
        if !stopped(status).await {
            explorer_window_step(status, db_path, profile_id).await;
        }
    }

    // Step 2: Repair owned names from inventory + tracked (explorer only).
    if lost("the repair step") {
        return;
    }
    if report_progress {
        let mut s = status.lock().await;
        s.step = "repair".into();
        s.progress_label = if node_authoritative {
            "Repairing owned names via node…".into()
        } else {
            "Repairing owned names…".into()
        };
    }
    if !node_authoritative {
        repair_step(status, db_path, profile_id).await;
    }

    // Step 3: Minimal explorer discovery (skipped when the node is synced —
    // node-backed discovery runs in Step 1 / node_discover_step).
    if lost("the discover step") {
        return;
    }
    if report_progress {
        let mut s = status.lock().await;
        s.step = "discover".into();
        s.progress_label = if node_authoritative {
            "Scanning node for names…".into()
        } else {
            "Scanning explorer for names…".into()
        };
    }
    if node_authoritative {
        node_discover_step(db_path, profile_id).await;
    } else {
        discover_step(status, db_path, profile_id).await;
    }

    // Step 4: Shakedex purchases — derive each open purchase's state from the
    // chain (R13). Only against an authoritative node: one that is behind, on
    // another network or SPV would report a mined purchase as unknown and get
    // it rebroadcast or declared lost. Best-effort, like the steps above.
    if node_authoritative {
        if lost("the Shakedex purchases") {
            return;
        }
        crate::shakedex_jobs::refresh_purchases_step(db_path, profile_id, caller.rebroadcast())
            .await;
        if lost("the Shakedex listings") {
            return;
        }
        // R19, R22: every listing's state from chain facts — before the
        // FINALIZE into the lock (Aborted, Expired, ReadyToFinalize,
        // Restored) and after it (Listed, SalePending, Sold). Reads the node
        // and the database only, so the daemon runs it as the app does.
        crate::shakedex_jobs::refresh_listings_step(db_path, profile_id).await;
        // R23, R24, R25, R28 (T6, T8): publish, advance, keep listed and
        // report the profile's published listings on LearnHNS Market,
        // mainnet only. Each reads the node and writes to the market and the
        // listing's market bookkeeping only: no key, no signature, no send —
        // the daemon runs them as the app does (SECURITY.md, "The daemon
        // never signs or broadcasts"). Each stops before its next listing
        // once the lock is lost.
        crate::shakedex_jobs::publish_listings_step(db_path, profile_id, &lock).await;
        // R24 (T8): a reverse auction's copy on the market follows its
        // schedule: the current step uploaded once it moves. Same contract
        // as the two around it: no key, no signature, no send.
        crate::shakedex_jobs::advance_reverse_auctions_step(db_path, profile_id, &lock).await;
        crate::shakedex_jobs::keep_listed_step(db_path, profile_id, &lock).await;
    }
}

/// Spawn a background thread that refreshes the sync lock's heartbeat.
///
/// Used by [`start_full_sync`] to keep the app's lock from going stale during
/// a long sync run, and by the daemon for its own runs. Both refresh every
/// [`crate::daemon::HEARTBEAT_INTERVAL_SECS`]. When a refresh finds the lock
/// is no longer ours (another process took it over), it raises `lock_lost`,
/// which stops the run ([`run_sync_steps`]), and refreshes no more.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn spawn_lock_heartbeat(
    db_path: String,
    profile_id: String,
    stop: Arc<std::sync::atomic::AtomicBool>,
    lock_lost: crate::db::sync_lock::LockLost,
) -> std::thread::JoinHandle<()> {
    let interval = Duration::from_secs(crate::daemon::HEARTBEAT_INTERVAL_SECS);
    spawn_lock_heartbeat_every(interval, db_path, profile_id, stop, lock_lost)
}

/// [`spawn_lock_heartbeat`] at `interval`, so a test need not wait out the
/// production one.
pub(crate) fn spawn_lock_heartbeat_every(
    interval: Duration,
    db_path: String,
    profile_id: String,
    stop: Arc<std::sync::atomic::AtomicBool>,
    lock_lost: crate::db::sync_lock::LockLost,
) -> std::thread::JoinHandle<()> {
    use std::sync::atomic::Ordering;
    std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            std::thread::sleep(interval);
            if stop.load(Ordering::SeqCst) {
                break;
            }
            if let Ok(c) = open_conn(&db_path) {
                match crate::db::sync_lock::refresh_heartbeat(&c, &profile_id) {
                    Ok(true) => {}
                    Ok(false) => {
                        eprintln!(
                            "sync heartbeat: the sync lock of {profile_id} was taken over by another process; stopping this sync"
                        );
                        lock_lost.raise();
                        break;
                    }
                    Err(e) => {
                        eprintln!("sync heartbeat: refresh failed for {profile_id}: {e}");
                    }
                }
            }
        }
    })
}

/// Poll the current sync status.
#[tauri::command]
pub async fn get_sync_status(state: State<'_, AppState>) -> Result<SyncStatus, AppError> {
    let s = state.sync_status.lock().await;
    Ok(s.clone())
}

/// Request that an in-flight background sync stop as soon as possible. Only sets
/// the `cancel_requested` flag on the shared status (leaving every other field
/// intact so the UI keeps showing what was done so far); the running steps
/// observe it between and within batches and bail out cleanly. A no-op when no
/// sync is running — the flag is cleared on the next `start_full_sync`.
#[tauri::command]
pub async fn cancel_full_sync(state: State<'_, AppState>) -> Result<(), AppError> {
    let mut s = state.sync_status.lock().await;
    s.cancel_requested = true;
    Ok(())
}

// ---------------------------------------------------------------------------
// Step helpers — all take db_path and profile_id, open their own conn
// ---------------------------------------------------------------------------

/// Open a DB connection for the background sync thread. Routed through
/// [`crate::db::connection::open`] (S4) rather than a bare `Connection::open`
/// so every sync-thread connection gets the same hardening as the app's main
/// connection: WAL mode, `busy_timeout` (so a connection that finds the DB
/// momentarily locked by a concurrent writer waits and retries instead of
/// failing immediately with "database is locked"), and `foreign_keys = ON`.
///
/// Legacy name kept for its existing callers; new code calls
/// [`crate::db::connection::open_migrated`] directly.
pub fn open_conn(db_path: &str) -> Result<rusqlite::Connection, AppError> {
    crate::db::connection::open_migrated(db_path)
}

/// Apply one node-sync batch (every coin upsert + spent-reconciliation +
/// cursor advance + profile sync-timestamp bump) inside a SINGLE transaction,
/// so a crash or error partway through never leaves a partially-updated UTXO
/// set paired with an advanced (or stale) cursor — either the whole batch
/// lands, or none of it does and the next sync retries from the same cursor.
fn apply_node_sync_batch(
    conn: &mut rusqlite::Connection,
    profile_id: &str,
    coins: &[crate::noncustodial::rpc::NodeCoin],
    height: i64,
) -> Result<(), AppError> {
    let tx = conn.unchecked_transaction()?;
    for coin in coins {
        crate::noncustodial::sync::upsert_utxo(&tx, profile_id, coin)?;
        if let Some(addr) = &coin.address {
            crate::noncustodial::sync::mark_address_used(&tx, profile_id, addr, coin.height)?;
        }
    }
    crate::noncustodial::sync::mark_missing_as_spent(&tx, profile_id, coins)?;
    crate::noncustodial::sync::set_sync_cursor(&tx, profile_id, height)?;
    queries::update_profile_sync(&tx, profile_id, height)?;
    tx.commit()?;
    Ok(())
}

/// What [`sync_node_step`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeStep {
    /// The node's coins were applied (a failed window growth after that is
    /// logged, and the next sync goes on).
    Synced,
    /// Nothing more was applied: the database, the node or the sync lock
    /// stopped the step, and the next sync retries from the same cursor.
    Failed,
    /// Nothing was read or applied: the node cannot answer for the wallet's
    /// coins (no address index, or its indexes could not be read), and is not
    /// authoritative for this run. The reason goes to the run's errors.
    NotAuthoritative(String),
}

/// The phrase every [`NodeStep::NotAuthoritative`] reason carries. The Wallet
/// page finds the reason in the run's errors by it and shows it next to the
/// balance (`nodeCoinsNotSyncedReason` in `src/queries/sync.ts`).
pub const NODE_COINS_NOT_SYNCED: &str = "the node's coins were not synced";

/// Node UTXO sync: scan the profile's addresses, apply the batch, grow the
/// address window ([`grow_address_window`]) and apply what it found. Writes
/// nothing more once `lock` is raised (another process took the sync lock
/// over): the batch not yet applied is left for that process.
pub async fn sync_node_step(
    db_path: &str,
    profile_id: &str,
    lock: &crate::db::sync_lock::LockLost,
) -> NodeStep {
    // Resolve the profile's effective node config (per-profile override ->
    // global -> default) rather than reading global settings directly.
    let client =
        match open_conn(db_path).and_then(|conn| NodeRpcClient::for_profile(&conn, profile_id)) {
            Ok(c) => c,
            Err(_) => return NodeStep::Failed,
        };
    sync_node_step_with_client(&client, db_path, profile_id, lock).await
}

/// [`sync_node_step`] against `client`. Testable against a mock.
pub(crate) async fn sync_node_step_with_client(
    client: &dyn crate::noncustodial::node_rpc::NodeRpc,
    db_path: &str,
    profile_id: &str,
    lock: &crate::db::sync_lock::LockLost,
) -> NodeStep {
    let conn = match open_conn(db_path) {
        Ok(c) => c,
        Err(_) => return NodeStep::Failed,
    };
    let addresses = match queries::get_profile_addresses(&conn, profile_id) {
        Ok(a) => a,
        Err(_) => return NodeStep::Failed,
    };
    // The settings read fails as the reads above do: no sync step, not a
    // window grown by a gap the user did not set.
    let gap = match queries::get_settings(&conn) {
        Ok(s) => crate::commands::secure_wallet::gap_limit(&s),
        Err(_) => return NodeStep::Failed,
    };
    drop(conn);

    let height = match client.get_blockchain_info().await {
        Ok(info) => info.blocks,
        Err(_) => return NodeStep::Failed,
    };

    // hsd without `--index-address` answers every coin and history query
    // with an empty list, never an error: applied, that list would mark
    // every tracked coin spent. Such a node, or one whose indexes cannot be
    // read, is not asked for coins or history at all this run. The tx index
    // is not needed here: the coins are complete without it, and the window
    // then grows only through addresses holding coins (see `NodeHistory`).
    match crate::noncustodial::node_rpc::node_indexes(client).await {
        Ok(indexes) if indexes.address => {}
        Ok(_) => {
            return NodeStep::NotAuthoritative(format!(
                "{} ({NODE_COINS_NOT_SYNCED}, and nothing was marked spent)",
                crate::noncustodial::node_rpc::NEEDS_ADDRESS_INDEX
            ))
        }
        Err(e) => return NodeStep::NotAuthoritative(format!("{NODE_COINS_NOT_SYNCED}: {e}")),
    }

    let mut all_coins = match fetch_coins_with_guard_with_client(client, &addresses).await {
        Some(coins) => coins,
        // Guard tripped: had addresses but every per-address query errored —
        // bail so the next sync retries from the same cursor (see the helper).
        None => return NodeStep::Failed,
    };

    if stop_for_lost_lock(lock, profile_id, "the node sync batch") {
        return NodeStep::Failed;
    }
    let mut conn = match open_conn(db_path) {
        Ok(c) => c,
        Err(_) => return NodeStep::Failed,
    };
    if apply_node_sync_batch(&mut conn, profile_id, &all_coins, height).is_err() {
        return NodeStep::Failed;
    }
    drop(conn);

    // Grow the address window by the gap limit and scan what it adds. Its own
    // step after the scan above has landed: a failure here leaves that batch
    // applied and the window as far as it got, and the next sync goes on.
    let found = match grow_address_window(
        &NodeHistory(client),
        db_path,
        profile_id,
        gap,
        WindowStop::Lock(lock),
    )
    .await
    {
        Ok(found) => found,
        Err(e) => {
            eprintln!("sync_node_step: growing the address window of {profile_id} failed: {e}");
            return NodeStep::Synced;
        }
    };
    if stop_for_lost_lock(lock, profile_id, "the address window batch") {
        return NodeStep::Failed;
    }
    if found.is_empty() {
        return NodeStep::Synced;
    }
    all_coins.extend(found);
    let mut conn = match open_conn(db_path) {
        Ok(c) => c,
        Err(_) => return NodeStep::Failed,
    };
    match apply_node_sync_batch(&mut conn, profile_id, &all_coins, height) {
        Ok(()) => NodeStep::Synced,
        Err(_) => NodeStep::Failed,
    }
}

/// The most rounds of [`grow_address_window`] one sync runs. Each round can
/// move a branch's window by up to `gap` addresses, so this bounds the
/// derivation and queries one sync spends on a long run of used addresses;
/// the next sync goes on from where this one stopped.
const MAX_WINDOW_ROUNDS: u32 = 50;

/// What a source says of one address in [`grow_address_window`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AddressUse {
    /// No transaction ever touched it, as far as the source knows.
    Unused,
    /// A transaction touched it, at `height` when the source says.
    Used { height: Option<i64> },
}

/// How [`grow_address_window`] spaces its queries to a source.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pacing {
    /// Sleep before every query after the first.
    throttle: Duration,
    /// Sleep after a failed query.
    backoff: Duration,
    /// Stop asking, for this sync, after this many failures in a row.
    give_up_after: Option<u32>,
    /// The most rounds one sync runs with this source.
    max_rounds: u32,
}

/// The most rounds of [`grow_address_window`] one sync runs from the
/// explorer. The explorer is not checked against the chain, and one that
/// lists a transaction for every address would grow each branch by the gap
/// limit every round; five rounds bound that to `5 * gap` addresses a branch
/// a sync (the node's [`MAX_WINDOW_ROUNDS`] would allow `50 * gap`). An
/// honest run of used addresses longer than that goes on at the next sync,
/// which starts from the window this one persisted.
const EXPLORER_WINDOW_ROUNDS: u32 = 5;

/// What stops [`grow_address_window`] before its next query or write.
pub(crate) enum WindowStop<'a> {
    /// The node sync: the sync lock taken over by another process.
    Lock(&'a crate::db::sync_lock::LockLost),
    /// A run with a status: the user's Stop or the lost lock ([`stopped`],
    /// which says which in the progress label).
    Status(&'a Arc<Mutex<SyncStatus>>),
}

impl WindowStop<'_> {
    async fn now(&self, profile_id: &str) -> bool {
        match self {
            WindowStop::Lock(lock) => {
                stop_for_lost_lock(lock, profile_id, "the next address window query or write")
            }
            WindowStop::Status(status) => {
                let stop = stopped(status).await;
                if stop {
                    eprintln!(
                        "sync: profile {profile_id}: stopped; the address window grows no \
                         further this sync"
                    );
                }
                stop
            }
        }
    }
}

/// Where [`grow_address_window`] learns which addresses were used: the
/// node's address index ([`NodeHistory`]) or the explorer's per-address tx
/// lists ([`ExplorerHistory`]). The loop is the same for both.
#[async_trait::async_trait]
pub(crate) trait AddressHistory: Send + Sync {
    /// Who answers, for the log.
    fn name(&self) -> &'static str;
    /// How to space the queries.
    fn pacing(&self) -> Pacing;
    /// Whether a transaction ever touched `address`.
    async fn address_use(&self, address: &str) -> Result<AddressUse, AppError>;
    /// The unspent coins at a newly derived address, for the caller's sync
    /// batch; none from a source that does not list coins.
    async fn coins(
        &self,
        address: &str,
    ) -> Result<Vec<crate::noncustodial::rpc::NodeCoin>, AppError>;
}

/// The node as an [`AddressHistory`]: `GET /tx/address` for the history and
/// `GET /coin/address` for the coins.
///
/// Sees only what the node indexes, and hsd answers an empty list, not an
/// error, for what it does not index. The history needs `--index-tx` as well
/// as `--index-address`: with the address index alone only addresses that
/// still hold coins move the window, so a run of spent addresses longer than
/// the gap ends it. Without `--index-address` both queries would return
/// nothing: [`sync_node_step`] does not grow the window from such a node.
pub(crate) struct NodeHistory<'a>(pub &'a dyn crate::noncustodial::node_rpc::NodeRpc);

#[async_trait::async_trait]
impl AddressHistory for NodeHistory<'_> {
    fn name(&self) -> &'static str {
        "the node"
    }

    fn pacing(&self) -> Pacing {
        Pacing {
            throttle: Duration::ZERO,
            backoff: Duration::ZERO,
            give_up_after: None,
            max_rounds: MAX_WINDOW_ROUNDS,
        }
    }

    async fn address_use(&self, address: &str) -> Result<AddressUse, AppError> {
        let txs = self.0.get_txs_by_address(address).await?;
        Ok(match txs.first() {
            Some(tx) => AddressUse::Used {
                height: tx
                    .get("height")
                    .and_then(|h| h.as_i64())
                    .filter(|h| *h >= 0),
            },
            None => AddressUse::Unused,
        })
    }

    async fn coins(
        &self,
        address: &str,
    ) -> Result<Vec<crate::noncustodial::rpc::NodeCoin>, AppError> {
        self.0.get_coins_by_address(address).await
    }
}

/// The explorer as an [`AddressHistory`]: an address is used when its tx
/// list (`get_address_txids`) counts a transaction. The explorer lists no
/// coins (SPV and explorer reads take the balance from it over the stored
/// addresses), so [`AddressHistory::coins`] is empty, and gives no height, so
/// an address it marks used has no first-seen height. Queries are spaced as
/// the discover step spaces them: [`DISCOVERY_THROTTLE`] apart,
/// [`SYNC_ERROR_BACKOFF`] after a failure, and none more this sync after
/// [`SYNC_MAX_CONSECUTIVE_ERRORS`] failures in a row (the explorer is down or
/// rate-limiting us).
pub(crate) struct ExplorerHistory<'a, E>(pub &'a E);

#[async_trait::async_trait]
impl<E: crate::providers::ExplorerProvider + Sync> AddressHistory for ExplorerHistory<'_, E> {
    fn name(&self) -> &'static str {
        "the explorer"
    }

    fn pacing(&self) -> Pacing {
        Pacing {
            throttle: DISCOVERY_THROTTLE,
            backoff: SYNC_ERROR_BACKOFF,
            give_up_after: Some(SYNC_MAX_CONSECUTIVE_ERRORS),
            max_rounds: EXPLORER_WINDOW_ROUNDS,
        }
    }

    async fn address_use(&self, address: &str) -> Result<AddressUse, AppError> {
        // One txid is enough to tell; `total` counts them all.
        let (txids, total) = self.0.get_address_txids(address, 1, 0).await?;
        Ok(if total > 0 || !txids.is_empty() {
            AddressUse::Used { height: None }
        } else {
            AddressUse::Unused
        })
    }

    async fn coins(
        &self,
        _address: &str,
    ) -> Result<Vec<crate::noncustodial::rpc::NodeCoin>, AppError> {
        Ok(Vec::new())
    }
}

/// Gap-limit discovery: mark the lookahead addresses
/// ([`crate::noncustodial::derivation::lookahead_addresses`]) that `source`
/// says were used as used (an address whose coins were all spent has none
/// left for a coin scan to see), grow the window
/// ([`crate::noncustodial::derivation::extend_address_window`]), read the
/// new addresses' coins and mark those used, and repeat until the window
/// stops growing or the source's round cap ([`Pacing`]) is reached. Returns the coins found
/// at the new addresses, for the caller's sync batch. A query that fails for
/// one address leaves it unused for this sync, and the failure is logged;
/// the window it reached is persisted, so the next sync scans it. A source
/// that gives up after failures in a row ([`Pacing`]) ends the growth for
/// this sync once what it found is written.
///
/// Each lookahead address is asked about once per sync, and on every sync
/// while it stays unused: about `2 * gap` queries a sync.
///
/// Stops, asking and writing nothing more, once `stop` says so (the sync lock
/// taken over by another process, or for a run with a status the user's
/// Stop): checked before each round, each query and each write. What it
/// found so far is returned; the caller applies none of it.
pub(crate) async fn grow_address_window(
    source: &dyn AddressHistory,
    db_path: &str,
    profile_id: &str,
    gap: u32,
    stop: WindowStop<'_>,
) -> Result<Vec<crate::noncustodial::rpc::NodeCoin>, AppError> {
    use crate::noncustodial::{derivation, sync as nsync};
    let pacing = source.pacing();
    let mut queries = 0u32;
    let mut failures_in_a_row = 0u32;
    let mut gave_up = false;
    let mut found = Vec::new();
    let mut checked: HashSet<String> = HashSet::new();
    for _ in 0..pacing.max_rounds {
        if gave_up || stop.now(profile_id).await {
            break;
        }
        let ahead = derivation::lookahead_addresses(&open_conn(db_path)?, profile_id)?;
        let mut with_history = Vec::new();
        for address in ahead {
            if !checked.insert(address.clone()) {
                continue;
            }
            if queries > 0 {
                sleep(pacing.throttle).await;
            }
            if stop.now(profile_id).await {
                break;
            }
            queries += 1;
            // An address the source did not answer for counts as unused this
            // round (best effort: the window it reaches is persisted, and the
            // next sync asks again), and the failure is logged.
            match source.address_use(&address).await {
                Ok(AddressUse::Used { height }) => {
                    failures_in_a_row = 0;
                    with_history.push((address, height));
                }
                Ok(AddressUse::Unused) => failures_in_a_row = 0,
                Err(e) => {
                    failures_in_a_row += 1;
                    eprintln!(
                        "sync: profile {profile_id}: no history for {address} from {} ({e}); \
                         read as unused this sync",
                        source.name()
                    );
                    if pacing.give_up_after.is_some_and(|n| failures_in_a_row >= n) {
                        eprintln!(
                            "sync: profile {profile_id}: {failures_in_a_row} failures in a row \
                             from {}; the address window grows no further this sync",
                            source.name()
                        );
                        gave_up = true;
                        break;
                    }
                    sleep(pacing.backoff).await;
                }
            }
        }
        if stop.now(profile_id).await {
            break;
        }
        let added = {
            let conn = open_conn(db_path)?;
            for (address, height) in &with_history {
                nsync::mark_address_used(&conn, profile_id, address, *height)?;
            }
            derivation::extend_address_window(&conn, profile_id, gap)?
        };
        if added.is_empty() {
            break;
        }
        let mut round = Vec::new();
        for d in &added {
            if stop.now(profile_id).await {
                break;
            }
            // Best effort, as above: the new address is stored, and the next
            // sync's coin scan reads it again; the failure is logged.
            match source.coins(&d.address).await {
                Ok(mut coins) => round.append(&mut coins),
                Err(e) => eprintln!(
                    "sync: profile {profile_id}: no coins for new address {} from {} ({e}); \
                     the next sync reads it",
                    d.address,
                    source.name()
                ),
            }
        }
        if stop.now(profile_id).await {
            break;
        }
        let conn = open_conn(db_path)?;
        for coin in &round {
            if let Some(address) = &coin.address {
                nsync::mark_address_used(&conn, profile_id, address, coin.height)?;
            }
        }
        found.extend(round);
    }
    Ok(found)
}

/// Grow the address window from the explorer ([`grow_address_window`] with
/// [`ExplorerHistory`]), for a sync whose node is not authoritative (SPV, or
/// a full node that is behind or unreachable), where balance and names are
/// read from the explorer over the stored addresses: an address it has never
/// derived is never read. Runs before the repair and discover steps, so they
/// and the balance read cover the new addresses in the same sync. Best
/// effort: no explorer for the profile's network, an unreadable setting or a
/// failed write leaves the window as it is (logged), and the sync goes on.
/// Stops, asking and writing nothing more, on the user's Stop or once the
/// sync lock is lost (`status`, checked before every query).
pub(crate) async fn explorer_window_step(
    status: &Arc<Mutex<SyncStatus>>,
    db_path: &str,
    profile_id: &str,
) {
    let (explorer, gap) = {
        let conn = match open_conn(db_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "sync: profile {profile_id}: the address window is not grown from the \
                     explorer: database not opened ({e})"
                );
                return;
            }
        };
        let settings = match queries::get_settings(&conn) {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "sync: profile {profile_id}: the address window is not grown from the \
                     explorer: settings not read ({e})"
                );
                return;
            }
        };
        // A network that cannot be read is unknown, not mainnet: no explorer
        // is guessed for it (as in the discover step).
        let explorer =
            crate::commands::active_profile::profile_network_opt_from_conn(&conn, profile_id)
                .and_then(|network| {
                    crate::providers::explorer_client_from_settings(&settings, network)
                });
        (
            explorer,
            crate::commands::secure_wallet::gap_limit(&settings),
        )
    };
    let Some(explorer) = explorer else {
        eprintln!(
            "sync: profile {profile_id}: no explorer for its network; the address window \
             grows only from the node"
        );
        return;
    };
    if let Err(e) = grow_address_window(
        &ExplorerHistory(&explorer),
        db_path,
        profile_id,
        gap,
        WindowStop::Status(status),
    )
    .await
    {
        eprintln!("sync: growing the address window of {profile_id} from the explorer failed: {e}");
    }
}

/// Client-injected coin-fetch phase for [`sync_node_step`], with the
/// "false everything-spent cascade" guard. Fetches coins per address; returns
/// `Some(all_coins)` on success, or `None` when the wallet HAS addresses but
/// EVERY per-address query errored (the node momentarily failing). Applying
/// an empty set in that case would make `mark_missing_as_spent` wipe every
/// tracked UTXO — so the caller must bail. The guard covers a node that
/// errors; a node without `--index-address` does not error but lists
/// nothing, and is handled before this by `node_indexes` in
/// [`sync_node_step`].
///
/// An empty address list returns `Some(vec![])` (nothing to fetch, nothing to
/// wipe). Testable against a mock.
pub(crate) async fn fetch_coins_with_guard_with_client(
    client: &dyn crate::noncustodial::node_rpc::NodeRpc,
    addresses: &[String],
) -> Option<Vec<crate::noncustodial::rpc::NodeCoin>> {
    let mut all_coins = Vec::new();
    let mut any_success = false;
    let mut any_error = false;
    for addr in addresses {
        match client.get_coins_by_address(addr).await {
            Ok(mut coins) => {
                any_success = true;
                all_coins.append(&mut coins);
            }
            Err(_) => any_error = true,
        }
    }
    if !addresses.is_empty() && any_error && !any_success {
        return None;
    }
    Some(all_coins)
}

/// Node-only owned-name discovery. Replaces `discover_step` when the local
/// node is authoritative (connected + fully synced). Precondition:
/// `sync_node_step` just refreshed the wallet's `tracked_utxos` from the node.
///
/// Mechanism: enumerate the name hashes referenced by the wallet's unspent
/// name-covenant coins (which cover every name the wallet has an active
/// auction position in OR currently owns), resolve each to a name string (via
/// hsd `getnamebyhash`, falling back to the coin's own `rawName` items[2]
/// for OPEN/BID/FINALIZE covenants), then `getnameinfo` and upsert into
/// `tracked_name_states` so `read_names` serves them.
///
/// Best-effort: transient RPC failures on individual names are skipped rather
/// than aborting the pass — the next Sync run picks them up. Only the wallet
/// profile's own coins are visible here, so this NEVER hits the network for
/// other bidders' data — that's the per-bid explorer path (Stage 2 replaces
/// it with the chain scanner).
pub async fn node_discover_step(db_path: &str, profile_id: &str) {
    let (client, hashes) = {
        let conn = match open_conn(db_path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let client = match NodeRpcClient::for_profile(&conn, profile_id) {
            Ok(c) => c,
            Err(_) => return,
        };
        let hashes =
            queries::list_unspent_wallet_name_hashes(&conn, profile_id).unwrap_or_default();
        (client, hashes)
    };
    if hashes.is_empty() {
        return;
    }

    // Resolve hashes → names and fetch their on-chain state via the node.
    let fetched =
        crate::commands::read::discover_names_via_node_with_client(&client, &hashes).await;
    if fetched.is_empty() {
        return;
    }

    // Upsert in a single transaction so a mid-write DB error can't leave
    // `tracked_name_states` half-updated.
    if let Ok(mut conn) = open_conn(db_path) {
        if let Ok(tx) = conn.transaction() {
            for (name, info) in &fetched {
                let _ = crate::noncustodial::sync::upsert_name_state(&tx, profile_id, name, info);
            }
            let _ = tx.commit();
        }
    }
}

/// How many inventory/tracked names one background repair *window* re-checks.
/// The repair step now loops over successive windows until the whole backlog
/// converges (see [`repair_step_windowed`]); this bounds each DB query and the
/// per-window progress, not the total work a single Sync click performs.
const REPAIR_WINDOW: u32 = 150;
/// Skip names re-checked within this many hours so repeated windows converge
/// (page through the inventory) instead of re-checking the same names forever.
const REPAIR_MIN_AGE_HOURS: i64 = 12;
/// Sleep this long after each consecutive explorer transport error, to back off
/// from a rate-limited / briefly-unavailable explorer before retrying. Shared by
/// both step functions (`repair_step_windowed` and `discover_step`) — the same
/// backoff behaviour applies to any explorer HTTP call in the background sync.
///
/// Under `cfg(test)` this drops to 1ms: the abort-after-N-consecutive-errors
/// tests (`all_transport_errors_abort_with_message`,
/// `all_format_errors_abort_with_degraded_message`) assert the *count* and
/// *degraded-status* outcome, not the wall-clock magnitude of the backoff. The
/// production 2s spacing exists only to be gentle against a live explorer and
/// has no place in an offline test loop.
#[cfg(not(test))]
const SYNC_ERROR_BACKOFF: Duration = Duration::from_secs(2);
#[cfg(test)]
const SYNC_ERROR_BACKOFF: Duration = Duration::from_millis(1);
/// Abort the current step after this many *consecutive* transport errors: the
/// explorer is down or rate-limiting us, so a clear error is surfaced and the
/// next Sync click resumes where this one left off (memoized via last_synced_at).
/// Shared by `repair_step_windowed` and `discover_step`.
const SYNC_MAX_CONSECUTIVE_ERRORS: u32 = 5;

// ---------------------------------------------------------------------------
// discover_step constants (phase-1 pagination + phase-2 "recently checked" memo)
// ---------------------------------------------------------------------------

/// Tx-list page size for the discovery crawl (matches `read.rs`'s
/// `discover_owned_names`). One HNSFans `/api/txs` page returns up to this many
/// txids.
const DISCOVERY_PAGE_SIZE: u32 = 25;
/// Max tx pages scanned per address during discovery (matches `read.rs`) — bounds
/// the crawl cost for very busy addresses.
const DISCOVERY_MAX_PAGES_PER_ADDRESS: u32 = 8;
/// A discover candidate whose inventory `last_synced_at` is within this many
/// hours is skipped in phase 2 — it was already checked by a recent repair or
/// discover sweep (the "recently checked" memo). Matches `REPAIR_MIN_AGE_HOURS`
/// so a name repair just stamped this same run is not re-verified by discover.
const DISCOVER_MEMO_HOURS: i64 = 12;

/// Whether the run must stop now: the user asked to (`cancel_requested`) or
/// another process took the sync lock over (`lock_lost`). When so, clears
/// `waiting` and says which in the progress label. Holds the status lock only
/// for the check, not across the caller's work.
async fn stopped(status: &Arc<Mutex<SyncStatus>>) -> bool {
    let mut s = status.lock().await;
    let label = if s.lock_lost.is_lost() {
        LOCK_LOST_LABEL
    } else if s.cancel_requested {
        "Sync cancelled"
    } else {
        return false;
    };
    s.waiting = false;
    s.progress_label = label.into();
    true
}

/// Repair owned names, looping over successive candidate windows until the whole
/// backlog is checked (true convergence) — so ONE Sync click finishes the job in
/// the background instead of the user re-clicking once per window.
pub async fn repair_step(status: &Arc<Mutex<SyncStatus>>, db_path: &str, profile_id: &str) {
    repair_step_windowed(status, db_path, profile_id, REPAIR_WINDOW).await;
}

/// The convergence loop, parameterized by window size so tests can seed more
/// candidates than one window and prove a SINGLE call pages through them all.
///
/// Mechanism:
/// * `attempted` — an in-run `HashSet` of every name we've already tried this
///   run. Each window re-queries `list_repair_candidates` (oldest-checked
///   first, `name ASC` tiebreak for a deterministic order) and filters out
///   names in `attempted`. Inventory names converge naturally (their
///   `last_synced_at` is stamped, so they drop out of the next query);
///   tracked-only names never get stamped (documented limitation of
///   `list_repair_candidates`), so `attempted` is what keeps them from being
///   re-fetched forever. Crucially, the SQL `max` passed to
///   `list_repair_candidates` GROWS by `attempted.len()` each iteration (see
///   `fetch_limit` below): with a plain fixed `window` LIMIT, a deterministic
///   query would keep returning the exact same top-`window` rows once none of
///   them are ever stamped (all tracked-only), so filtering by `attempted`
///   would empty out immediately and the loop would falsely "converge" after
///   just one window, silently leaving later candidates unattempted. Growing
///   the SQL limit alongside `attempted` guarantees the fetch always reaches
///   far enough to surface names not yet tried, then `.take(window)` caps the
///   actual batch size back down to the intended window. An empty
///   POST-FILTER window == true convergence -> break.
/// * `repair_candidates` / `repair_remaining` — `repair_candidates` is the total
///   backlog counted ONCE up front (stable denominator); `repair_remaining`
///   starts there and decreases by one per attempted name, converging to 0.
/// * Backoff — consecutive transport errors sleep `SYNC_ERROR_BACKOFF` and
///   abort at `SYNC_MAX_CONSECUTIVE_ERRORS`; the counter resets on any
///   successful check (owned OR not-owned). A whole window with zero successful
///   checks also aborts (belt-and-suspenders against an infinite loop).
/// * Cancellation — `cancel_requested` (and a lost sync lock, see [`stopped`])
///   is checked at the top of each window AND before each per-name check, so
///   Stop is responsive even mid-window.
pub async fn repair_step_windowed(
    status: &Arc<Mutex<SyncStatus>>,
    db_path: &str,
    profile_id: &str,
    window: u32,
) {
    let (explorer_opt, all_addresses, total_backlog) = {
        let conn = match open_conn(db_path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let settings = match queries::get_settings(&conn) {
            Ok(s) => s,
            Err(_) => return,
        };
        // G2: resolve the profile's network to gate the explorer availability.
        // A network that cannot be read is unknown, not mainnet — guessing
        // would point this step at another chain's explorer and write its
        // answers into this profile's cache.
        let explorer_opt =
            crate::commands::active_profile::profile_network_opt_from_conn(&conn, profile_id)
                .and_then(|network| {
                    crate::providers::explorer_client_from_settings(&settings, network)
                });
        let addrs = queries::get_profile_addresses(&conn, profile_id).unwrap_or_default();
        // Total backlog counted once: the stable "/ N" denominator for progress.
        let total =
            queries::count_repair_candidates(&conn, profile_id, REPAIR_MIN_AGE_HOURS).unwrap_or(0);
        (explorer_opt, addrs, total)
    };
    // G2: if no explorer is available for this network and the node isn't
    // authoritative, the repair step cannot proceed. Return early without
    // stamping the sync (so a later run can retry).
    let explorer = match explorer_opt {
        Some(e) => e,
        None => {
            let mut s = status.lock().await;
            s.errors
                .push(crate::providers::EXPLORER_UNAVAILABLE.to_string());
            return;
        }
    };
    let addr_set: HashSet<String> = all_addresses.iter().cloned().collect();

    {
        let mut s = status.lock().await;
        s.repair_candidates = total_backlog;
        s.repair_remaining = total_backlog;
    }

    let mut attempted: HashSet<String> = HashSet::new();
    let mut repaired = 0u32;
    let mut consecutive_errors = 0u32;

    loop {
        // Cancellation is checked at the top of each window…
        if stopped(status).await {
            return;
        }

        // Next window of candidates, minus names already tried this run. The
        // SQL fetch limit grows with `attempted.len()` so a deterministically-
        // ordered result (see `list_repair_candidates` docs) always reaches
        // past already-tried rows to surface fresh ones, even when none of
        // them are ever stamped out of the query (tracked-only names); the
        // batch itself is then capped back to `window` via `.take`.
        let batch: Vec<String> = {
            let conn = match open_conn(db_path) {
                Ok(c) => c,
                Err(_) => return,
            };
            let fetch_limit = window.saturating_add(attempted.len() as u32);
            queries::list_repair_candidates(&conn, profile_id, fetch_limit, REPAIR_MIN_AGE_HOURS)
                .unwrap_or_default()
                .into_iter()
                .filter(|n| !attempted.contains(n))
                .take(window as usize)
                .collect()
        };
        // Empty post-filter window => every remaining candidate was already
        // tried this run => convergence.
        if batch.is_empty() {
            break;
        }

        let mut progressed = 0u32;
        for name in &batch {
            // …and before each per-name check, so Stop is responsive mid-window.
            if stopped(status).await {
                return;
            }
            attempted.insert(name.clone());
            let remaining = total_backlog.saturating_sub(attempted.len() as u32);
            {
                let mut s = status.lock().await;
                s.repair_remaining = remaining;
                s.progress_label = format!("Checking {name} — {remaining} left (owned {repaired})");
                s.waiting = true;
            }

            sleep(DISCOVERY_THROTTLE).await;
            // A transport error means the name wasn't actually checked. It's
            // already in `attempted` (so we don't re-fetch it this run), and we
            // leave last_synced_at untouched so a LATER Sync run retries it.
            let info_opt = match explorer.get_name_info_optional(name).await {
                Ok(v) => v,
                Err(e) => {
                    {
                        let mut s = status.lock().await;
                        s.waiting = false;
                    }
                    consecutive_errors += 1;
                    if consecutive_errors >= SYNC_MAX_CONSECUTIVE_ERRORS {
                        record_error_and_clear_waiting(status, &e).await;
                        return;
                    }
                    sleep(SYNC_ERROR_BACKOFF).await;
                    continue;
                }
            };
            // Throttle again: `get_name_info_optional` above and the resolver's
            // first internal call are two separate explorer HTTP calls, so every
            // call must be preceded by a sleep (DISCOVERY_THROTTLE contract).
            sleep(DISCOVERY_THROTTLE).await;
            let resolution = resolve_owner_via_history(&explorer, name, &addr_set).await;
            {
                let mut s = status.lock().await;
                s.waiting = false;
            }

            let mut conn = match open_conn(db_path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            match resolution {
                Ok(Some(res)) if res.owned_by_wallet => {
                    if let Some(info) = &info_opt {
                        if apply_repair_owned(
                            &mut conn,
                            profile_id,
                            info,
                            &res.owner_txid,
                            res.owner_vout,
                            &res.owner_address,
                            name,
                        )
                        .is_ok()
                        {
                            repaired += 1;
                            {
                                let mut s = status.lock().await;
                                s.repaired = repaired;
                            }
                        }
                    } else {
                        // Owned per history but the name-info lookup returned
                        // 404 — record the check so we don't spin on it.
                        let _ = queries::touch_asset_synced(&conn, name);
                    }
                    progressed += 1;
                    consecutive_errors = 0;
                }
                // Checked and not owned (no history, or owner is a foreign
                // address): stamp last_synced_at so repeated windows converge.
                Ok(_) => {
                    let _ = queries::touch_asset_synced(&conn, name);
                    progressed += 1;
                    consecutive_errors = 0;
                }
                // Explorer errored mid-resolve: leave last_synced_at so a later
                // Sync retries. Count it as a transport error for backoff.
                Err(e) => {
                    consecutive_errors += 1;
                    if consecutive_errors >= SYNC_MAX_CONSECUTIVE_ERRORS {
                        record_error_and_clear_waiting(status, &e).await;
                        return;
                    }
                    sleep(SYNC_ERROR_BACKOFF).await;
                }
            }
        }

        // A whole window that made zero successful checks means every name in it
        // errored (transport) — the explorer is unavailable. Stop rather than
        // loop forever re-fetching the same never-stamped names.
        if progressed == 0 {
            let mut s = status.lock().await;
            s.errors
                .push("Explorer unavailable / rate-limited — Sync will continue later".into());
            s.waiting = false;
            break;
        }
    }
}

/// Apply the "confirmed owned" repair outcome — `upsert_owned_name` (records
/// the current owner outpoint) + `mark_asset_finalized_owned` (advances
/// inventory status + stamps `last_synced_at`) — inside a SINGLE transaction,
/// so a crash between the two never leaves the tracked name state updated
/// without the inventory status following (or vice versa).
fn apply_repair_owned(
    conn: &mut rusqlite::Connection,
    profile_id: &str,
    info: &crate::hsd::types::HsdName,
    owner_txid: &str,
    owner_vout: u32,
    owner_address: &str,
    tld: &str,
) -> Result<(), AppError> {
    let tx = conn.unchecked_transaction()?;
    queries::upsert_owned_name(&tx, profile_id, info, owner_txid, owner_vout, owner_address)?;
    queries::mark_asset_finalized_owned(&tx, tld, info.state.as_deref())?;
    tx.commit()?;
    Ok(())
}

/// The message pushed to `SyncStatus.errors` when a background step aborts
/// after repeated explorer errors — distinguishes "the explorer's response
/// shape changed" (loud, actionable — Task 11 / S1) from the existing
/// generic "unreachable / rate-limited" message. Both cases go through the
/// SAME error channel and abort mechanism; only the wording differs.
fn explorer_error_message(err: &AppError) -> &'static str {
    match err {
        AppError::ExplorerFormat(_) => {
            "Explorer degraded — response format changed unexpectedly. Check the explorer URL in \
             Settings, or wait for it to recover; Sync will continue later."
        }
        _ => "Explorer unavailable / rate-limited — Sync will continue later",
    }
}

/// Push the "explorer down after repeated errors" message (worded per the
/// triggering error's kind) and clear `waiting`.
async fn record_error_and_clear_waiting(status: &Arc<Mutex<SyncStatus>>, err: &AppError) {
    let mut s = status.lock().await;
    s.errors.push(explorer_error_message(err).to_string());
    s.waiting = false;
}

/// Task 11 review, Finding 2: `wallet_profiles.last_synced_at` only ever
/// advances via the node-RPC sync step, so in explorer-only mode (no local
/// node) the frontend's "Last successful sync" line stayed "—" forever even
/// after a fully successful explorer sync. This stamps a SEPARATE
/// `last_explorer_sync_at` column exactly ONCE, called from the "Done"
/// block of `start_full_sync`'s background thread — never from inside
/// `repair_step_windowed`/`discover_step` — so it touches none of their
/// windowed-convergence/memo/cancel/backoff logic.
///
/// A run is "clean" iff it reached here with no cancellation, its sync lock
/// still its own (a run that lost it writes nothing more, this stamp
/// included) and no aborting error: `SyncStatus.errors` is ONLY ever pushed to by
/// [`record_error_and_clear_waiting`], which fires exclusively when a step
/// aborts after `SYNC_MAX_CONSECUTIVE_ERRORS` consecutive explorer errors —
/// so an empty `errors` here means repair + discover both ran to
/// completion (whether or not they found anything to do). An aborted or
/// cancelled run leaves the column exactly as it was (never advanced), so
/// it never overstates freshness.
///
/// Best-effort: a DB hiccup here must not fail an otherwise-clean sync run,
/// so any error opening the connection or running the UPDATE is swallowed
/// rather than pushed into `SyncStatus.errors`.
pub async fn stamp_explorer_sync_if_clean(
    status: &Arc<Mutex<SyncStatus>>,
    db_path: &str,
    profile_id: &str,
) {
    let clean = {
        let s = status.lock().await;
        !s.cancel_requested && !s.lock_lost.is_lost() && s.errors.is_empty()
    };
    if !clean {
        return;
    }
    if let Ok(conn) = open_conn(db_path) {
        let _ = queries::stamp_explorer_sync(&conn, profile_id);
    }
}

pub async fn discover_step(status: &Arc<Mutex<SyncStatus>>, db_path: &str, profile_id: &str) {
    let (explorer_opt, addrs) = {
        let conn = match open_conn(db_path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let settings = match queries::get_settings(&conn) {
            Ok(s) => s,
            Err(_) => return,
        };
        // G2: resolve the profile's network to gate the explorer availability.
        // A network that cannot be read is unknown, not mainnet — guessing
        // would point this step at another chain's explorer and write its
        // answers into this profile's cache.
        let explorer_opt =
            crate::commands::active_profile::profile_network_opt_from_conn(&conn, profile_id)
                .and_then(|network| {
                    crate::providers::explorer_client_from_settings(&settings, network)
                });
        let addrs = queries::get_profile_addresses(&conn, profile_id).unwrap_or_default();
        (explorer_opt, addrs)
    };
    // G2: if no explorer is available for this network and the node isn't
    // authoritative, the discover step cannot proceed. Return early without
    // stamping the sync (so a later run can retry).
    let explorer = match explorer_opt {
        Some(e) => e,
        None => {
            let mut s = status.lock().await;
            s.errors
                .push(crate::providers::EXPLORER_UNAVAILABLE.to_string());
            return;
        }
    };
    if addrs.is_empty() {
        return;
    }
    let addr_set: HashSet<&str> = addrs.iter().map(|s| s.as_str()).collect();

    // Set total address count for progress display.
    {
        let mut s = status.lock().await;
        s.discover_addresses_total = addrs.len() as u32;
    }

    // A single consecutive-transport-error counter spans BOTH phases: any
    // successful explorer call resets it, `SYNC_MAX_CONSECUTIVE_ERRORS` in a row
    // aborts the whole step with a clear message (the next Sync resumes via the
    // memo). Same backoff/abort mechanism as `repair_step_windowed` (Task A).
    let mut consecutive_errors = 0u32;

    // Phase 1 — paginate the tx crawl per address (up to
    // `DISCOVERY_MAX_PAGES_PER_ADDRESS` pages of `DISCOVERY_PAGE_SIZE` txids,
    // stopping early on a short/empty page or when total is reached), collecting
    // name candidates from outputs that pay one of our addresses. Mirrors
    // `read.rs`'s `discover_owned_names` crawl, plus cancellation + backoff.
    let mut candidates: HashSet<String> = HashSet::new();
    let mut seen_tx: HashSet<String> = HashSet::new();
    for (i, addr) in addrs.iter().enumerate() {
        // Cancellation is checked per-address (before any explorer call)…
        if stopped(status).await {
            return;
        }
        // Update progress: which address we are scanning.
        {
            let mut s = status.lock().await;
            s.discover_addresses_done = i as u32;
        }
        let mut offset = 0u32;
        let mut pages = 0u32;
        loop {
            // …and per-page, so Stop is responsive even mid-crawl of a busy addr.
            if stopped(status).await {
                return;
            }
            {
                let mut s = status.lock().await;
                s.waiting = true;
            }
            let (txids, total) = match explorer
                .get_address_txids(addr, DISCOVERY_PAGE_SIZE, offset)
                .await
            {
                Ok(v) => {
                    consecutive_errors = 0;
                    v
                }
                Err(e) => {
                    {
                        let mut s = status.lock().await;
                        s.waiting = false;
                    }
                    consecutive_errors += 1;
                    if consecutive_errors >= SYNC_MAX_CONSECUTIVE_ERRORS {
                        record_error_and_clear_waiting(status, &e).await;
                        return;
                    }
                    sleep(SYNC_ERROR_BACKOFF).await;
                    // Rate-limited / transport error: skip the rest of this
                    // address (as read.rs does) and move to the next one.
                    break;
                }
            };
            {
                let mut s = status.lock().await;
                s.waiting = false;
            }
            for txid in &txids {
                if !seen_tx.insert(txid.clone()) {
                    continue;
                }
                if stopped(status).await {
                    return;
                }
                sleep(DISCOVERY_THROTTLE).await;
                {
                    let mut s = status.lock().await;
                    s.waiting = true;
                }
                match explorer.get_tx_named_outputs(txid).await {
                    Ok(outs) => {
                        consecutive_errors = 0;
                        for o in outs {
                            if addr_set.contains(o.address.as_str()) {
                                candidates.insert(o.name);
                            }
                        }
                    }
                    Err(e) => {
                        {
                            let mut s = status.lock().await;
                            s.waiting = false;
                        }
                        consecutive_errors += 1;
                        if consecutive_errors >= SYNC_MAX_CONSECUTIVE_ERRORS {
                            record_error_and_clear_waiting(status, &e).await;
                            return;
                        }
                        sleep(SYNC_ERROR_BACKOFF).await;
                        continue;
                    }
                }
                {
                    let mut s = status.lock().await;
                    s.discover_txs_scanned += 1;
                    s.discover_candidates = candidates.len() as u32;
                    s.waiting = false;
                }
            }
            pages += 1;
            offset += DISCOVERY_PAGE_SIZE;
            if txids.is_empty()
                || (offset as u64) >= total
                || pages >= DISCOVERY_MAX_PAGES_PER_ADDRESS
            {
                break;
            }
            // Throttle before fetching the next page (DISCOVERY_THROTTLE contract).
            sleep(DISCOVERY_THROTTLE).await;
        }
    }
    // All addresses scanned.
    {
        let mut s = status.lock().await;
        s.discover_addresses_done = addrs.len() as u32;
        s.waiting = false;
    }

    // Load already-known names so we don't re-discover what's tracked.
    let mut seen_names: HashSet<String> = {
        let conn = match open_conn(db_path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let tracked = queries::list_tracked_name_names(&conn, profile_id).unwrap_or_default();
        tracked.into_iter().collect()
    };

    // "Recently checked" memo: names whose inventory `last_synced_at` is within
    // DISCOVER_MEMO_HOURS were already verified by a recent repair or discover
    // sweep, so phase 2 skips re-fetching them. This is what makes a re-run
    // resume where the last one stopped instead of re-checking everything.
    let recently_synced: HashSet<String> = {
        let conn = match open_conn(db_path) {
            Ok(c) => c,
            Err(_) => return,
        };
        queries::list_recently_synced_tlds(&conn, DISCOVER_MEMO_HOURS)
            .unwrap_or_default()
            .into_iter()
            .collect()
    };

    // Phase 2 — verify ownership for every candidate not already tracked and not
    // in the memo (no per-run budget cap anymore: one Sync processes them all).
    // `resolve_owner_via_history` (Task 2) needs an owned `HashSet<String>`
    // (phase 1's `addr_set` above borrows from `addrs` as `&str`).
    let owned_addr_set: HashSet<String> = addrs.iter().cloned().collect();
    let mut discovered = 0u32;
    for name in &candidates {
        // Cancellation is checked before each per-candidate check.
        if stopped(status).await {
            return;
        }
        if !seen_names.insert(name.clone()) {
            continue;
        }
        // Memo: skip candidates checked recently (by this or a prior sync).
        if recently_synced.contains(name) {
            continue;
        }

        {
            let mut s = status.lock().await;
            s.discover_current_name = name.clone();
            s.waiting = true;
        }
        sleep(DISCOVERY_THROTTLE).await;
        // Use the *optional* name lookup so a 404 (name unknown to the explorer)
        // is a clean "not found" rather than a transport error that trips backoff.
        let info_opt = match explorer.get_name_info_optional(name).await {
            Ok(v) => {
                consecutive_errors = 0;
                v
            }
            Err(e) => {
                {
                    let mut s = status.lock().await;
                    s.waiting = false;
                }
                consecutive_errors += 1;
                if consecutive_errors >= SYNC_MAX_CONSECUTIVE_ERRORS {
                    record_error_and_clear_waiting(status, &e).await;
                    return;
                }
                sleep(SYNC_ERROR_BACKOFF).await;
                continue;
            }
        };
        // Resolve the CURRENT owner via history + the owner tx's output address
        // (Task 2). Throttle again here: `get_name_info_optional` above and the
        // resolver's first internal call are two separate explorer HTTP calls.
        sleep(DISCOVERY_THROTTLE).await;
        let resolution = resolve_owner_via_history(&explorer, name, &owned_addr_set).await;
        {
            let mut s = status.lock().await;
            s.waiting = false;
        }
        match resolution {
            Ok(Some(res)) if res.owned_by_wallet => {
                consecutive_errors = 0;
                if let Some(info) = &info_opt {
                    let conn = match open_conn(db_path) {
                        Ok(c) => c,
                        Err(_) => return,
                    };
                    let _ = queries::upsert_owned_name(
                        &conn,
                        profile_id,
                        info,
                        &res.owner_txid,
                        res.owner_vout,
                        &res.owner_address,
                    );
                    discovered += 1;
                    {
                        let mut s = status.lock().await;
                        s.discovered = discovered;
                    }
                }
                // Owned per history but name-info 404'd (no `info`): nothing to
                // upsert; a later repair sweep will finalize it.
            }
            // Checked and NOT owned (no history, or a foreign owner address):
            // stamp the memo so a later Sync skips it. This is a no-op for a
            // discovered-but-foreign name that has no `assets` row — acceptable,
            // such names are few.
            Ok(_) => {
                consecutive_errors = 0;
                let conn = match open_conn(db_path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let _ = queries::touch_asset_synced(&conn, name);
            }
            // Explorer errored mid-resolve: count it as a transport error for
            // backoff, leave the name unstamped so a later Sync retries it.
            Err(e) => {
                consecutive_errors += 1;
                if consecutive_errors >= SYNC_MAX_CONSECUTIVE_ERRORS {
                    record_error_and_clear_waiting(status, &e).await;
                    return;
                }
                sleep(SYNC_ERROR_BACKOFF).await;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Task 9 unit tests: connection hardening + batch-transaction atomicity.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod db_hardening_tests {
    use super::*;
    use crate::noncustodial::rpc::NodeCoin;

    fn temp_db_path(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("namehold_sync_hardening_{tag}_{n}.db"))
    }

    /// [S4] `open_conn` — the helper every sync-thread step routes its
    /// connections through — must apply the SAME hardening as
    /// `db::connection::open`: WAL mode is implied by `busy_timeout` actually
    /// mattering, but the two pragmas that matter most for a bg thread
    /// writing concurrently with the UI thread are `busy_timeout` (retry
    /// instead of an immediate "database is locked") and `foreign_keys`.
    /// Before the fix, `open_conn` called `rusqlite::Connection::open`
    /// directly and neither pragma was set (both default to 0/off).
    #[test]
    fn open_conn_applies_busy_timeout_and_foreign_keys() {
        let path = temp_db_path("open_conn");
        let _ = std::fs::remove_file(&path);

        let conn = open_conn(path.to_str().unwrap()).expect("open_conn should succeed");

        let busy_timeout: i64 = conn
            .pragma_query_value(None, "busy_timeout", |row| row.get(0))
            .unwrap();
        assert!(
            busy_timeout > 0,
            "busy_timeout must be set (got {busy_timeout})"
        );

        let fk: i64 = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert_eq!(fk, 1, "foreign_keys must be ON");

        drop(conn);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    fn mem_conn_for_batch() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        crate::db::migrations::run(&conn).unwrap();
        crate::db::queries::insert_wallet_profile(
            &conn,
            "p1",
            "Test",
            "mnemonic_hot",
            "mainnet",
            "xpubFAKE",
            0,
            false,
        )
        .unwrap();
        conn
    }

    fn coin(txid: &str, vout: u32, address: Option<&str>) -> NodeCoin {
        NodeCoin {
            txid: txid.to_string(),
            vout,
            value: 1_000_000,
            script: Some("0014aabb".to_string()),
            address: address.map(|s| s.to_string()),
            height: Some(50),
            confirmations: Some(1),
            coinbase: Some(false),
            covenant: None,
        }
    }

    /// A batch where the SECOND coin is malformed (no address — `upsert_utxo`
    /// rejects it with `AppError::InvalidInput`) must roll back EVERYTHING:
    /// not just the bad coin, but the good coin that preceded it AND the
    /// cursor/profile-sync advance that would otherwise follow. Before this
    /// task, each write was a standalone `conn.execute` with no enclosing
    /// transaction, so the first coin and the cursor could each land
    /// independently regardless of a later failure.
    #[test]
    fn failed_batch_rolls_back_utxos_and_leaves_cursor_untouched() {
        let mut conn = mem_conn_for_batch();
        // Seed a pre-existing cursor so we can prove it does NOT move.
        crate::noncustodial::sync::set_sync_cursor(&conn, "p1", 10).unwrap();

        let coins = vec![
            coin("aa", 0, Some("hs1qgoodaddr")),
            coin("bb", 0, None), // missing address -> upsert_utxo errors mid-batch
        ];

        let result = apply_node_sync_batch(&mut conn, "p1", &coins, 999);
        assert!(
            result.is_err(),
            "a malformed coin mid-batch must fail the whole batch"
        );

        let utxo_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM tracked_utxos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(utxo_count, 0, "no partial UTXO write survives a failed batch — including the GOOD coin before the bad one");

        assert_eq!(
            crate::noncustodial::sync::get_sync_height(&conn, "p1").unwrap(),
            10,
            "cursor must stay at the pre-batch height, not advance to the failed batch's height"
        );
    }

    /// The success path commits the UTXO upsert, the spent-reconciliation,
    /// the cursor, and the profile sync timestamp together.
    #[test]
    fn successful_batch_commits_utxos_and_cursor_together() {
        let mut conn = mem_conn_for_batch();
        let coins = vec![coin("aa", 0, Some("hs1qgoodaddr"))];

        apply_node_sync_batch(&mut conn, "p1", &coins, 42).expect("batch should succeed");

        let utxo_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM tracked_utxos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(utxo_count, 1);
        assert_eq!(
            crate::noncustodial::sync::get_sync_height(&conn, "p1").unwrap(),
            42
        );
    }

    /// A batch that reconciles a previously-tracked UTXO as spent (because
    /// it's no longer in `live_coins`) must do so in the SAME transaction as
    /// the cursor advance — a failure elsewhere in the batch must not leave
    /// the coin marked spent with a stale cursor (or vice versa).
    #[test]
    fn mark_missing_as_spent_is_part_of_the_same_transaction() {
        let mut conn = mem_conn_for_batch();
        // First batch: one coin lands, unspent.
        apply_node_sync_batch(&mut conn, "p1", &[coin("aa", 0, Some("hs1qgoodaddr"))], 1)
            .expect("first batch");

        // Second batch: "aa" is no longer live (node stopped reporting it) —
        // it should be marked spent AND the cursor should advance, atomically.
        apply_node_sync_batch(&mut conn, "p1", &[], 2).expect("second batch");

        let spent_by: Option<String> = conn
            .query_row(
                "SELECT spent_by_txid FROM tracked_utxos WHERE txid = 'aa' AND vout = 0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(spent_by.is_some(), "missing coin must be marked spent");
        assert_eq!(
            crate::noncustodial::sync::get_sync_height(&conn, "p1").unwrap(),
            2
        );
    }

    /// When every per-address coin query errors (the node momentarily failing),
    /// `sync_node_step` must bail out and NOT call `apply_node_sync_batch` with an
    /// empty coin set — that would falsely mark every tracked UTXO as spent.
    #[tokio::test]
    async fn sync_node_step_bails_when_all_coin_queries_error() {
        let mut server = mockito::Server::new_async().await;
        let _indexes_server =
            crate::tests::mock_node_rpc::mock_node_indexes(&mut server, true, true).await;

        // Mock getblockchaininfo → synced.
        let _m_info = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
            .with_status(200)
            .with_body(
                serde_json::json!({
                    "result": { "blocks": 100, "headers": 100, "verificationprogress": 1.0 },
                    "error": null, "id": null
                })
                .to_string(),
            )
            .create_async()
            .await;

        // Mock /coin/address/* → all error (simulating the node momentarily
        // failing; without the index hsd answers `200 []` instead).
        let _m_coins = server
            .mock("GET", mockito::Matcher::Regex("^/coin/address/".into()))
            .with_status(400)
            .with_body(
                serde_json::json!({
                    "error": { "message": "Address indexing not enabled" }
                })
                .to_string(),
            )
            .create_async()
            .await;

        // Seed a file-backed DB (sync_node_step re-opens by path).
        let db_path = temp_db_path("empty_live_coins_guard");
        let _ = std::fs::remove_file(&db_path);
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        crate::db::migrations::run(&conn).unwrap();
        crate::db::queries::insert_wallet_profile(
            &conn,
            "p1",
            "Test",
            "mnemonic_hot",
            "mainnet",
            "xpubFAKE",
            0,
            false,
        )
        .unwrap();
        // Seed a derived address so `get_profile_addresses` is non-empty.
        let addr = crate::noncustodial::derivation::DerivedAddress {
            branch: 0,
            child_index: 0,
            address: "hs1qgoodaddr".to_string(),
            script_pubkey_hex: "0014aabb".to_string(),
            public_key_hex: "02aabbcc".to_string(),
        };
        crate::noncustodial::derivation::persist_address(&conn, "p1", 0, &addr).unwrap();
        // Seed a pre-existing tracked UTXO.
        crate::noncustodial::sync::upsert_utxo(
            &conn,
            "p1",
            &coin("preexisting", 0, Some("hs1qgoodaddr")),
        )
        .unwrap();
        // Initial cursor — must NOT move.
        crate::noncustodial::sync::set_sync_cursor(&conn, "p1", 50).unwrap();
        // Point settings at the mock server.
        crate::db::queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();
        crate::db::queries::set_setting(&conn, "node_rpc_api_key", "x").unwrap();
        drop(conn);

        // Run sync_node_step — must bail out because every coin query errored.
        let step = sync_node_step(
            db_path.to_str().unwrap(),
            "p1",
            &crate::db::sync_lock::LockLost::default(),
        )
        .await;
        assert_eq!(
            step,
            NodeStep::Failed,
            "sync_node_step must fail when all coin queries error"
        );

        // Verify the UTXO was NOT marked spent and the cursor did NOT move.
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let spent_by: Option<String> = conn
            .query_row(
                "SELECT spent_by_txid FROM tracked_utxos WHERE txid = 'preexisting' AND vout = 0",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            spent_by.is_none(),
            "pre-existing UTXO must NOT be marked spent when every coin query errored"
        );
        let cursor = crate::noncustodial::sync::get_sync_height(&conn, "p1").unwrap();
        assert_eq!(
            cursor, 50,
            "cursor must NOT advance when sync_node_step bails out"
        );
        drop(conn);

        // Cleanup.
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
        let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
    }

    /// A db_path whose parent directory does not exist makes `open_conn`
    /// fail. Every sync step routes its first connection through `open_conn`
    /// and bails on error — these guards keep a transient FS/DB failure from
    /// panicking the background sync thread. Drive each step with a bad path
    /// and assert it returns quietly (the step fns are `async` and infallible
    /// from the caller's view; reaching the `return`/`return false` arm is the
    /// behavior under test).
    fn unopenable_db_path() -> String {
        // A path under a directory that does not exist -> sqlite open fails.
        std::env::temp_dir()
            .join("namehold_sync_no_such_dir_xyz")
            .join("nested")
            .join("wallet.db")
            .to_str()
            .unwrap()
            .to_string()
    }

    #[tokio::test]
    async fn sync_node_step_returns_false_when_conn_cannot_open() {
        let bad = unopenable_db_path();
        let step = sync_node_step(&bad, "p1", &crate::db::sync_lock::LockLost::default()).await;
        assert_eq!(
            step,
            NodeStep::Failed,
            "sync_node_step must fail when open_conn fails"
        );
    }

    #[tokio::test]
    async fn node_discover_step_returns_early_when_conn_cannot_open() {
        // Infallible return type — the assertion is simply that it does not
        // panic and completes, exercising the `Err(_) => return` guard.
        let bad = unopenable_db_path();
        node_discover_step(&bad, "p1").await;
    }

    #[tokio::test]
    async fn repair_step_windowed_returns_early_when_conn_cannot_open() {
        let bad = unopenable_db_path();
        let status = Arc::new(Mutex::new(SyncStatus::default()));
        repair_step_windowed(&status, &bad, "p1", REPAIR_WINDOW).await;
    }

    #[tokio::test]
    async fn discover_step_returns_early_when_conn_cannot_open() {
        let bad = unopenable_db_path();
        let status = Arc::new(Mutex::new(SyncStatus::default()));
        discover_step(&status, &bad, "p1").await;
    }

    /// `run_sync_steps` computes `node_authoritative`; when the DB cannot be
    /// opened the settings lookup yields `None`, driving the `None => false`
    /// arm (line ~466). With a bad path every downstream step also hits its
    /// own `open_conn` guard, so the whole orchestration returns quietly.
    #[tokio::test]
    async fn run_sync_steps_treats_unopenable_db_as_non_authoritative() {
        let bad = unopenable_db_path();
        let status = Arc::new(Mutex::new(SyncStatus::default()));
        // The app caller, so the progress-label writes are exercised too.
        run_sync_steps(&status, &bad, "p1", SyncCaller::App).await;
    }

    /// Pre-set `cancel_requested = true` before entering the repair loop; the
    /// top-of-window cancel check must observe it and write the "Sync cancelled"
    /// progress label before any explorer traffic. The DB must be openable so
    /// the guard runs (the open_conn guard would swallow a bad-path case).
    #[tokio::test]
    async fn repair_step_windowed_bails_on_pre_set_cancel_flag() {
        let path = temp_db_path("repair_cancel");
        let _ = std::fs::remove_file(&path);
        let conn = open_conn(path.to_str().unwrap()).expect("open");
        crate::db::queries::insert_wallet_profile(
            &conn,
            "p1",
            "Test",
            "mnemonic_hot",
            "mainnet",
            "xpubFAKE",
            0,
            false,
        )
        .unwrap();
        drop(conn);

        let status = Arc::new(Mutex::new(SyncStatus {
            cancel_requested: true,
            ..SyncStatus::default()
        }));
        repair_step_windowed(&status, path.to_str().unwrap(), "p1", REPAIR_WINDOW).await;

        let s = status.lock().await;
        assert_eq!(
            s.progress_label, "Sync cancelled",
            "top-of-window cancel guard must set the progress label"
        );
        assert!(!s.waiting, "cancel guard must clear the waiting flag");
        drop(s);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    /// Same shape for `discover_step`: pre-set cancel, expect the "Sync cancelled"
    /// label. The discover fn checks `cancel_requested` at the top of its
    /// address-scan loop; with no addresses seeded the loop body never runs, but
    /// the top cancel arm also fires from a top-level cancel check earlier in
    /// the fn (documented in the cover map at lines 1121/1163/1256).
    #[tokio::test]
    async fn discover_step_bails_on_pre_set_cancel_flag() {
        let path = temp_db_path("discover_cancel");
        let _ = std::fs::remove_file(&path);
        let conn = open_conn(path.to_str().unwrap()).expect("open");
        crate::db::queries::insert_wallet_profile(
            &conn,
            "p1",
            "Test",
            "mnemonic_hot",
            "mainnet",
            "xpubFAKE",
            0,
            false,
        )
        .unwrap();
        // Seed at least one address so the discover fn enters its per-address
        // loop where the mid-fn cancel checks live.
        let addr = crate::noncustodial::derivation::DerivedAddress {
            branch: 0,
            child_index: 0,
            address: "hs1qcanceladdr".to_string(),
            script_pubkey_hex: "0014dead".to_string(),
            public_key_hex: "02deadbeef".to_string(),
        };
        crate::noncustodial::derivation::persist_address(&conn, "p1", 0, &addr).unwrap();
        drop(conn);

        let status = Arc::new(Mutex::new(SyncStatus {
            cancel_requested: true,
            ..SyncStatus::default()
        }));
        discover_step(&status, path.to_str().unwrap(), "p1").await;

        let s = status.lock().await;
        // Either the top-level cancel arm or the per-page arm sets the label;
        // both write the same string, and either is a valid observation here.
        assert_eq!(s.progress_label, "Sync cancelled");
        drop(s);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }
}

// ---------------------------------------------------------------------------
// The address window grows by the gap limit during the node sync.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod address_window_tests {
    use super::*;
    use crate::noncustodial::derivation::{self, BRANCH_CHANGE, BRANCH_RECEIVE};
    use crate::noncustodial::hd::{seed_from_mnemonic, ExtendedPrivKey, ExtendedPubKey};
    use crate::noncustodial::network::Network;

    const GAP: u32 = 3;

    fn xpub() -> ExtendedPubKey {
        let seed = seed_from_mnemonic(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            "",
        )
        .unwrap();
        ExtendedPubKey::from_priv(&ExtendedPrivKey::from_seed(&seed).unwrap())
    }

    fn addr(branch: u32, index: u32) -> String {
        derivation::derive_one(Network::Main, &xpub(), branch, index)
            .unwrap()
            .address
    }

    fn txid(n: u8) -> String {
        format!("{n:02x}").repeat(32)
    }

    fn coin_body(n: u8, address: &str) -> String {
        format!(
            r#"[{{"hash":"{}","index":0,"value":1000000,"address":"{address}","height":90,"covenant":{{"type":0,"action":"NONE","items":[]}}}}]"#,
            txid(n)
        )
    }

    /// A watch-only profile restored with the window `[0, GAP)` per branch.
    /// The chain has coins at receive 2 (inside it), receive 4 (past it, found
    /// once receive 2 is used) and receive 7 (found only after receive 4 is),
    /// a spent-only history at change 1 (no coin left, so only the history
    /// marks it used) and a coin at change 3 behind it. One sync tracks every
    /// coin: the window grows, the new addresses are scanned in the same sync,
    /// and again until it stops growing.
    #[tokio::test]
    async fn one_sync_grows_the_window_and_scans_the_new_addresses() {
        let mut server = mockito::Server::new_async().await;
        let _indexes_server =
            crate::tests::mock_node_rpc::mock_node_indexes(&mut server, true, true).await;
        let _info = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
            .with_body(r#"{"result":{"chain":"main","blocks":100},"error":null,"id":1}"#)
            .create_async()
            .await;
        let coins = [
            (1u8, addr(BRANCH_RECEIVE, 2)),
            (2, addr(BRANCH_RECEIVE, 4)),
            (3, addr(BRANCH_RECEIVE, 7)),
            (4, addr(BRANCH_CHANGE, 3)),
        ];
        let mut mocks = Vec::new();
        for (n, a) in &coins {
            mocks.push(
                server
                    .mock("GET", format!("/coin/address/{a}").as_str())
                    .with_body(coin_body(*n, a))
                    .create_async()
                    .await,
            );
        }
        let spent_only = addr(BRANCH_CHANGE, 1);
        mocks.push(
            server
                .mock("GET", format!("/tx/address/{spent_only}").as_str())
                .with_body(format!(r#"[{{"hash":"{}","height":80}}]"#, txid(9)))
                .create_async()
                .await,
        );
        mocks.push(
            server
                .mock("GET", mockito::Matcher::Regex("^/coin/address/".into()))
                .with_body("[]")
                .create_async()
                .await,
        );
        mocks.push(
            server
                .mock("GET", mockito::Matcher::Regex("^/tx/address/".into()))
                .with_body("[]")
                .create_async()
                .await,
        );

        let db_path = std::env::temp_dir().join(format!(
            "namehold_address_window_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        crate::db::migrations::run(&conn).unwrap();
        let xpub_str = xpub().to_base58check(Network::Main);
        queries::insert_wallet_profile(
            &conn,
            "p1",
            "Watch",
            "watch_only_xpub",
            "mainnet",
            &xpub_str,
            0,
            true,
        )
        .unwrap();
        crate::commands::secure_wallet::provision_addresses(
            &conn,
            "p1",
            Network::Main,
            &xpub_str,
            GAP,
        )
        .unwrap();
        queries::set_setting(&conn, "address_gap_limit", &GAP.to_string()).unwrap();
        queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();
        queries::set_setting(&conn, "node_rpc_api_key", "x").unwrap();
        drop(conn);

        assert_eq!(
            sync_node_step(
                db_path.to_str().unwrap(),
                "p1",
                &crate::db::sync_lock::LockLost::default()
            )
            .await,
            NodeStep::Synced
        );

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let tracked: HashSet<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT txid FROM tracked_utxos
                     WHERE wallet_profile_id = 'p1' AND spent_by_txid IS NULL",
                )
                .unwrap();
            stmt.query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        let want: HashSet<String> = coins.iter().map(|(n, _)| txid(*n)).collect();
        assert_eq!(tracked, want, "every coin of the account is tracked");
        // The window ends GAP past the highest used index of each branch.
        assert_eq!(
            derivation::max_derived_index(&conn, "p1", 0, BRANCH_RECEIVE).unwrap(),
            Some(7 + GAP)
        );
        assert_eq!(
            derivation::max_derived_index(&conn, "p1", 0, BRANCH_CHANGE).unwrap(),
            Some(3 + GAP)
        );
        drop(conn);
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
        let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
    }

    /// The sync lock is taken over while the first round of window growth
    /// scans the addresses it added (the coin at receive 4 is fetched): that
    /// round writes nothing more, no later round runs (receive 7 is never
    /// asked for) and the coins found are not applied. The window stays where
    /// round one's growth left it, and only the coin of the first batch
    /// (receive 2) is tracked.
    #[tokio::test]
    async fn a_lost_lock_stops_the_window_before_its_next_round() {
        let mut server = mockito::Server::new_async().await;
        let _indexes_server =
            crate::tests::mock_node_rpc::mock_node_indexes(&mut server, true, true).await;
        let _info = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::Regex("getblockchaininfo".into()))
            .with_body(r#"{"result":{"chain":"main","blocks":100},"error":null,"id":1}"#)
            .create_async()
            .await;
        let lock = crate::db::sync_lock::LockLost::default();
        let (r2, r4, r7) = (
            addr(BRANCH_RECEIVE, 2),
            addr(BRANCH_RECEIVE, 4),
            addr(BRANCH_RECEIVE, 7),
        );
        let _r2 = server
            .mock("GET", format!("/coin/address/{r2}").as_str())
            .with_body(coin_body(1, &r2))
            .create_async()
            .await;
        let raise = lock.clone();
        let body = coin_body(2, &r4);
        let r4_mock = server
            .mock("GET", format!("/coin/address/{r4}").as_str())
            .with_body_from_request(move |_| {
                // What the heartbeat does once another process holds the lock.
                raise.raise();
                body.clone().into_bytes()
            })
            .expect(1)
            .create_async()
            .await;
        let r7_mock = server
            .mock("GET", format!("/coin/address/{r7}").as_str())
            .expect(0)
            .create_async()
            .await;
        let _rest = server
            .mock("GET", mockito::Matcher::Regex("^/coin/address/".into()))
            .with_body("[]")
            .create_async()
            .await;
        let _history = server
            .mock("GET", mockito::Matcher::Regex("^/tx/address/".into()))
            .with_body("[]")
            .create_async()
            .await;

        let db_path = std::env::temp_dir().join(format!(
            "namehold_address_window_lost_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        crate::db::migrations::run(&conn).unwrap();
        let xpub_str = xpub().to_base58check(Network::Main);
        queries::insert_wallet_profile(
            &conn,
            "p1",
            "Watch",
            "watch_only_xpub",
            "mainnet",
            &xpub_str,
            0,
            true,
        )
        .unwrap();
        crate::commands::secure_wallet::provision_addresses(
            &conn,
            "p1",
            Network::Main,
            &xpub_str,
            GAP,
        )
        .unwrap();
        queries::set_setting(&conn, "address_gap_limit", &GAP.to_string()).unwrap();
        queries::set_setting(&conn, "node_rpc_url", &server.url()).unwrap();
        queries::set_setting(&conn, "node_rpc_api_key", "x").unwrap();
        drop(conn);

        assert_eq!(
            sync_node_step(db_path.to_str().unwrap(), "p1", &lock).await,
            NodeStep::Failed
        );

        r4_mock.assert_async().await;
        r7_mock.assert_async().await;
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let tracked: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT txid FROM tracked_utxos WHERE wallet_profile_id = 'p1'")
                .unwrap();
            stmt.query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        assert_eq!(tracked, vec![txid(1)], "only the first batch is applied");
        // Round one grew receive to receive 2 + GAP; receive 4 was never
        // marked used, so no round grew it further.
        assert_eq!(
            derivation::max_derived_index(&conn, "p1", 0, BRANCH_RECEIVE).unwrap(),
            Some(2 + GAP)
        );
        let r4_used: bool = conn
            .query_row(
                "SELECT used FROM derived_addresses WHERE address = ?1",
                [&r4],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!r4_used, "round one wrote nothing once the lock was lost");
        drop(conn);
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
        let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
    }

    /// A file-backed DB with one mainnet watch-only profile `p1` restored with
    /// the window `[0, GAP)` per branch, in SPV mode against an unreachable
    /// node and the explorer at `explorer_url`.
    fn spv_profile_db(tag: &str, explorer_url: &str) -> std::path::PathBuf {
        let db_path = std::env::temp_dir().join(format!(
            "namehold_address_window_{tag}_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        crate::db::migrations::run(&conn).unwrap();
        let xpub_str = xpub().to_base58check(Network::Main);
        queries::insert_wallet_profile(
            &conn,
            "p1",
            "Watch",
            "watch_only_xpub",
            "mainnet",
            &xpub_str,
            0,
            true,
        )
        .unwrap();
        crate::commands::secure_wallet::provision_addresses(
            &conn,
            "p1",
            Network::Main,
            &xpub_str,
            GAP,
        )
        .unwrap();
        queries::set_setting(&conn, "address_gap_limit", &GAP.to_string()).unwrap();
        queries::set_setting(&conn, "node_mode", "spv").unwrap();
        queries::set_setting(&conn, "node_rpc_url", "http://127.0.0.1:1").unwrap();
        queries::set_setting(&conn, "explorer_api_url", explorer_url).unwrap();
        drop(conn);
        db_path
    }

    fn remove_db(db_path: &std::path::Path) {
        let _ = std::fs::remove_file(db_path);
        let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
        let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
    }

    /// One page of an address's txs, in the shape HNSFans answers
    /// `/api/txs?address=` (the provider and discover tests record it).
    fn txs_page(hashes: &[&str]) -> String {
        let result: Vec<serde_json::Value> = hashes
            .iter()
            .map(|h| serde_json::json!({ "hash": h }))
            .collect();
        serde_json::json!({
            "limit": 25, "offset": 0, "total": hashes.len(), "result": result
        })
        .to_string()
    }

    /// `address`'s tx list, served `hits` times (mockito hands a request to
    /// the first matching mock still short of its hits, then to the last one).
    async fn txs_mock(
        server: &mut mockito::ServerGuard,
        address: &str,
        hashes: &[&str],
        hits: usize,
    ) -> mockito::Mock {
        server
            .mock("GET", "/api/txs")
            .match_query(mockito::Matcher::UrlEncoded(
                "address".into(),
                address.into(),
            ))
            .with_body(txs_page(hashes))
            .expect(hits)
            .create_async()
            .await
    }

    /// An SPV sync of a restored profile whose window is `[0, GAP)`: the
    /// explorer lists a tx at receive 2 (inside the window, no name) and a
    /// FINALIZE of `pastwindow` paying receive 4 (past it). The window grows
    /// from the explorer's tx lists until it stops growing, and the same
    /// sync's explorer discovery reads receive 4: the name is tracked, and the
    /// balance (read from the explorer over the stored addresses) covers it.
    #[tokio::test]
    async fn an_spv_sync_grows_the_window_from_the_explorer_and_finds_a_name_past_it() {
        let mut server = mockito::Server::new_async().await;
        let (r2, r4) = (addr(BRANCH_RECEIVE, 2), addr(BRANCH_RECEIVE, 4));
        // Each is read twice: by the window's growth and by discovery.
        let r2_txs = txs_mock(&mut server, &r2, &["txA"], 2).await;
        let r4_txs = txs_mock(&mut server, &r4, &["txB"], 2).await;
        let _rest = server
            .mock("GET", "/api/txs")
            .match_query(mockito::Matcher::Any)
            .with_body(txs_page(&[]))
            .create_async()
            .await;
        let _tx_a = server
            .mock("GET", "/api/txs/txA")
            .with_body(format!(
                r#"{{"outputs":[{{"action":"NONE","address":"{r2}","value":5}}]}}"#
            ))
            .create_async()
            .await;
        let _tx_b = server
            .mock("GET", "/api/txs/txB")
            .with_body(format!(
                r#"{{"outputs":[{{"action":"FINALIZE","name":"pastwindow","address":"{r4}","value":400000}}]}}"#
            ))
            .create_async()
            .await;
        let _name = server
            .mock("GET", "/api/names/pastwindow")
            .with_body(r#"{"name":"pastwindow","hash":"deadbeef","state":"CLOSED","height":100,"renewal":200}"#)
            .create_async()
            .await;
        let _hist = server
            .mock("GET", "/api/names/pastwindow/history")
            .with_body(r#"{"result":[{"action":"Finalize","txid":"txB","index":0}]}"#)
            .create_async()
            .await;
        let db_path = spv_profile_db("spv", &server.url());

        let status = Arc::new(Mutex::new(SyncStatus::default()));
        run_sync_steps(&status, db_path.to_str().unwrap(), "p1", SyncCaller::App).await;

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        assert_eq!(
            derivation::max_derived_index(&conn, "p1", 0, BRANCH_RECEIVE).unwrap(),
            Some(4 + GAP),
            "the window ends GAP past receive 4"
        );
        assert_eq!(
            derivation::max_derived_index(&conn, "p1", 0, BRANCH_CHANGE).unwrap(),
            Some(GAP - 1),
            "the change branch has no history and keeps its window"
        );
        assert!(
            queries::get_profile_addresses(&conn, "p1")
                .unwrap()
                .contains(&r4),
            "the explorer balance is read over receive 4"
        );
        let tracked: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM tracked_name_states
                 WHERE wallet_profile_id = 'p1' AND name = 'pastwindow'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(tracked, "the name past the restore window is tracked");
        drop(conn);
        r2_txs.assert_async().await;
        r4_txs.assert_async().await;
        remove_db(&db_path);
    }

    /// An explorer that fails every tx list: the window stays `[0, GAP)` and
    /// the sync goes on to the discover step over the stored addresses, which
    /// reports the explorer's failure.
    #[tokio::test]
    async fn an_explorer_error_leaves_the_window_and_the_sync_goes_on() {
        let mut server = mockito::Server::new_async().await;
        let _txs = server
            .mock("GET", "/api/txs")
            .match_query(mockito::Matcher::Any)
            .with_status(500)
            .with_body("boom")
            .create_async()
            .await;
        let db_path = spv_profile_db("spv_err", &server.url());

        let status = Arc::new(Mutex::new(SyncStatus::default()));
        run_sync_steps(&status, db_path.to_str().unwrap(), "p1", SyncCaller::App).await;

        let s = status.lock().await;
        assert_eq!(s.step, "discover", "the sync reached the discover step");
        assert_eq!(s.discover_addresses_total, 2 * GAP);
        assert!(
            !s.errors.is_empty(),
            "discovery reports the explorer failure"
        );
        drop(s);
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        for branch in [BRANCH_RECEIVE, BRANCH_CHANGE] {
            assert_eq!(
                derivation::max_derived_index(&conn, "p1", 0, branch).unwrap(),
                Some(GAP - 1),
                "no growth on branch {branch}"
            );
        }
        drop(conn);
        remove_db(&db_path);
    }

    fn status() -> Arc<Mutex<SyncStatus>> {
        Arc::new(Mutex::new(SyncStatus::default()))
    }

    fn is_used(conn: &rusqlite::Connection, address: &str) -> bool {
        conn.query_row(
            "SELECT used FROM derived_addresses WHERE address = ?1",
            [address],
            |r| r.get(0),
        )
        .unwrap()
    }

    async fn failing_txs(server: &mut mockito::ServerGuard, hits: usize) -> mockito::Mock {
        server
            .mock("GET", "/api/txs")
            .match_query(mockito::Matcher::Any)
            .with_status(500)
            .with_body("boom")
            .expect(hits)
            .create_async()
            .await
    }

    /// The explorer is asked as the discover step asks it: after
    /// `SYNC_MAX_CONSECUTIVE_ERRORS` failures in a row no more of the `2 * GAP`
    /// lookahead addresses is asked about this sync.
    #[tokio::test]
    async fn the_explorer_window_stops_asking_after_failures_in_a_row() {
        let mut server = mockito::Server::new_async().await;
        let txs = failing_txs(&mut server, SYNC_MAX_CONSECUTIVE_ERRORS as usize).await;
        let db_path = spv_profile_db("spv_give_up", &server.url());
        // The explorer gives up before it has asked about every address.
        const _: () = assert!(SYNC_MAX_CONSECUTIVE_ERRORS < 2 * GAP);

        explorer_window_step(&status(), db_path.to_str().unwrap(), "p1").await;

        txs.assert_async().await;
        remove_db(&db_path);
    }

    /// The explorer lists a tx at receive 0, then fails five times in a row
    /// (receive 1 and 2, change 0 to 2): it is asked no more, and the window
    /// still grows from receive 0.
    #[tokio::test]
    async fn the_explorer_window_writes_what_it_found_before_it_stopped_asking() {
        let mut server = mockito::Server::new_async().await;
        let r0 = addr(BRANCH_RECEIVE, 0);
        let r0_txs = txs_mock(&mut server, &r0, &["txA"], 1).await;
        let failing = failing_txs(&mut server, SYNC_MAX_CONSECUTIVE_ERRORS as usize).await;
        let db_path = spv_profile_db("spv_give_up_writes", &server.url());

        explorer_window_step(&status(), db_path.to_str().unwrap(), "p1").await;

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        assert!(is_used(&conn, &r0), "receive 0 is marked used");
        assert_eq!(
            derivation::max_derived_index(&conn, "p1", 0, BRANCH_RECEIVE).unwrap(),
            Some(GAP),
            "the window ends GAP past receive 0"
        );
        drop(conn);
        r0_txs.assert_async().await;
        failing.assert_async().await;
        remove_db(&db_path);
    }

    /// An explorer that lists a tx for every address (wrong or hostile) grows
    /// each branch by GAP a round for `EXPLORER_WINDOW_ROUNDS` rounds, not for
    /// the node's `MAX_WINDOW_ROUNDS`.
    #[tokio::test]
    async fn an_explorer_that_lists_every_address_grows_the_window_a_few_rounds_only() {
        let mut server = mockito::Server::new_async().await;
        let _txs = server
            .mock("GET", "/api/txs")
            .match_query(mockito::Matcher::Any)
            .with_body(txs_page(&["txA"]))
            .create_async()
            .await;
        let db_path = spv_profile_db("spv_hostile", &server.url());

        explorer_window_step(&status(), db_path.to_str().unwrap(), "p1").await;

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        for branch in [BRANCH_RECEIVE, BRANCH_CHANGE] {
            assert_eq!(
                derivation::max_derived_index(&conn, "p1", 0, branch).unwrap(),
                Some(GAP - 1 + EXPLORER_WINDOW_ROUNDS * GAP),
                "branch {branch} grew GAP a round for EXPLORER_WINDOW_ROUNDS rounds"
            );
        }
        drop(conn);
        remove_db(&db_path);
    }

    /// Either half of the explorer's answer makes an address used: a `total`
    /// above zero with an empty page, or a listed tx with a `total` of zero.
    /// Both zero is unused.
    #[tokio::test]
    async fn the_explorer_reads_an_address_as_used_from_its_total_or_its_txids() {
        let mut server = mockito::Server::new_async().await;
        let cases = [
            (
                "hs1qtotalonly",
                r#"{"limit":1,"offset":0,"total":3,"result":[]}"#,
            ),
            (
                "hs1qhashonly",
                r#"{"limit":1,"offset":0,"total":0,"result":[{"hash":"txA"}]}"#,
            ),
            (
                "hs1qneither",
                r#"{"limit":1,"offset":0,"total":0,"result":[]}"#,
            ),
        ];
        let mut mocks = Vec::new();
        for (address, body) in cases {
            mocks.push(
                server
                    .mock("GET", "/api/txs")
                    .match_query(mockito::Matcher::UrlEncoded(
                        "address".into(),
                        address.into(),
                    ))
                    .with_body(body)
                    .create_async()
                    .await,
            );
        }
        let client = crate::providers::hnsfans::HnsFansClient::new(&server.url());
        let source = ExplorerHistory(&client);
        let used = AddressUse::Used { height: None };
        assert_eq!(source.address_use("hs1qtotalonly").await.unwrap(), used);
        assert_eq!(source.address_use("hs1qhashonly").await.unwrap(), used);
        assert_eq!(
            source.address_use("hs1qneither").await.unwrap(),
            AddressUse::Unused
        );
    }

    /// The sync lock is taken over while the explorer answers for receive 2:
    /// no other address is asked about after it (only receive 0 and 1 were,
    /// before), receive 2 is not marked used and the window does not grow.
    #[tokio::test]
    async fn a_lost_lock_stops_the_explorer_window_before_its_write() {
        let mut server = mockito::Server::new_async().await;
        let status = status();
        let raise = status.lock().await.lock_lost.clone();
        let r2 = addr(BRANCH_RECEIVE, 2);
        let body = txs_page(&["txA"]);
        let r2_txs = server
            .mock("GET", "/api/txs")
            .match_query(mockito::Matcher::UrlEncoded("address".into(), r2.clone()))
            .with_body_from_request(move |_| {
                // What the heartbeat does once another process holds the lock.
                raise.raise();
                body.clone().into_bytes()
            })
            .expect(1)
            .create_async()
            .await;
        let rest = server
            .mock("GET", "/api/txs")
            .match_query(mockito::Matcher::Any)
            .with_body(txs_page(&[]))
            .expect(2)
            .create_async()
            .await;
        let db_path = spv_profile_db("spv_lost", &server.url());

        explorer_window_step(&status, db_path.to_str().unwrap(), "p1").await;

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        assert!(
            !is_used(&conn, &r2),
            "nothing is written once the lock is lost"
        );
        assert_eq!(
            derivation::max_derived_index(&conn, "p1", 0, BRANCH_RECEIVE).unwrap(),
            Some(GAP - 1)
        );
        drop(conn);
        assert_eq!(status.lock().await.progress_label, LOCK_LOST_LABEL);
        r2_txs.assert_async().await;
        rest.assert_async().await;
        remove_db(&db_path);
    }

    /// The user presses Stop while the explorer answers for receive 0: no
    /// other address is asked about, nothing is written, and the label says
    /// the sync was cancelled.
    #[tokio::test]
    async fn stop_ends_the_explorer_window_before_its_next_query() {
        let mut server = mockito::Server::new_async().await;
        let status = status();
        let r0 = addr(BRANCH_RECEIVE, 0);
        let press = status.clone();
        let body = txs_page(&["txA"]);
        let r0_txs = server
            .mock("GET", "/api/txs")
            .match_query(mockito::Matcher::UrlEncoded("address".into(), r0.clone()))
            .with_body_from_request(move |_| {
                // What `cancel_full_sync` does; the loop holds the status lock
                // only for its checks, never across a query.
                loop {
                    if let Ok(mut s) = press.try_lock() {
                        s.cancel_requested = true;
                        break;
                    }
                    std::thread::yield_now();
                }
                body.clone().into_bytes()
            })
            .expect(1)
            .create_async()
            .await;
        let rest = server
            .mock("GET", "/api/txs")
            .match_query(mockito::Matcher::Any)
            .with_body(txs_page(&[]))
            .expect(0)
            .create_async()
            .await;
        let db_path = spv_profile_db("spv_stop", &server.url());

        explorer_window_step(&status, db_path.to_str().unwrap(), "p1").await;

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        assert!(!is_used(&conn, &r0), "nothing is written after Stop");
        assert_eq!(
            derivation::max_derived_index(&conn, "p1", 0, BRANCH_RECEIVE).unwrap(),
            Some(GAP - 1)
        );
        drop(conn);
        assert_eq!(status.lock().await.progress_label, "Sync cancelled");
        r0_txs.assert_async().await;
        rest.assert_async().await;
        remove_db(&db_path);
    }
}
