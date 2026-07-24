//! Orchestrated full sync with persistent progress.
//!
//! A single command [`start_full_sync`] runs all reconciliation steps in order
//! and writes progress into [`SyncSession`] in `AppState`, so the frontend can
//! poll status via [`get_sync_status`] even across page navigation.

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
const DISCOVERY_THROTTLE: Duration = Duration::from_millis(150);

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
        }
    }
}

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
#[tauri::command]
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

            #[cfg(test)]
            if TEST_PANIC_HOOK.swap(false, std::sync::atomic::Ordering::SeqCst) {
                panic!("TEST_PANIC_HOOK: injected panic simulating a panic in a sync step");
            }

            // Step 1: Node sync (best-effort)
            {
                let mut s = status.lock().await;
                s.step = "node".into();
                s.progress_label = "Syncing with local node…".into();
            }
            let _node_ok = sync_node_step(&db_path, &profile_id).await;

            // Is the node authoritative (connected AND fully synced)? When it is,
            // the explorer-backed repair/discover steps below are redundant: Step 1
            // already refreshed the wallet's coins from the node, and owned-name
            // state is kept current by the node coin scan plus the wallet's own
            // covenant writes. HNSFans is only a PRE-SYNC workaround — once the
            // node is caught up we stop calling it. (Re-checked here, per run, so a
            // node that later falls out of sync transparently re-enables the
            // explorer path on the next Sync.)
            let node_authoritative = {
                let settings = open_conn(&db_path)
                    .ok()
                    .and_then(|c| queries::get_settings(&c).ok());
                match settings {
                    Some(s) => crate::commands::read::node_ready_from_settings(&s).await,
                    None => false,
                }
            };

            // Step 2: Repair owned names from inventory + tracked (explorer only).
            {
                let mut s = status.lock().await;
                s.step = "repair".into();
                s.progress_label = if node_authoritative {
                    "Repairing owned names via node…".into()
                } else {
                    "Repairing owned names…".into()
                };
            }
            if !node_authoritative {
                repair_step(&status, &db_path, &profile_id).await;
            }

            // Step 3: Minimal explorer discovery (skipped when the node is synced —
            // node-backed discovery runs in Step 1 / node_discover_step).
            {
                let mut s = status.lock().await;
                s.step = "discover".into();
                s.progress_label = if node_authoritative {
                    "Scanning node for names…".into()
                } else {
                    "Scanning explorer for names…".into()
                };
            }
            if node_authoritative {
                node_discover_step(&db_path, &profile_id).await;
            } else {
                discover_step(&status, &db_path, &profile_id).await;
            }

            // Done — but if either step observed a cancellation request, report
            // the run as cancelled rather than complete.
            {
                let mut s = status.lock().await;
                if s.cancel_requested {
                    s.step = "cancelled".into();
                    s.progress_label = "Sync stopped".into();
                } else {
                    s.step = "done".into();
                    s.progress_label = "Sync complete".into();
                }
                s.running = false;
                s.finished_at = Some(format!("{:?}", std::time::SystemTime::now()));
            }
            // Task 11 review, Finding 2: stamp `last_explorer_sync_at` once,
            // OUTSIDE the per-name repair/discover loops and their windowed
            // convergence/memo/cancel/backoff logic entirely (see the
            // function's own doc comment for the "clean run" definition).
            stamp_explorer_sync_if_clean(&status, &db_path, &profile_id).await;
            guard.mark_completed();
        });
    });

    Ok(serde_json::json!({"started": true}))
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
pub(crate) fn open_conn(db_path: &str) -> Result<rusqlite::Connection, AppError> {
    let conn = crate::db::connection::open(std::path::Path::new(db_path))?;
    crate::db::migrations::run(&conn)?;
    Ok(conn)
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

async fn sync_node_step(db_path: &str, profile_id: &str) -> bool {
    let conn = match open_conn(db_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let settings = match queries::get_settings(&conn) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let addresses = match queries::get_profile_addresses(&conn, profile_id) {
        Ok(a) => a,
        Err(_) => return false,
    };
    drop(conn);

    let client = NodeRpcClient::from_settings(&settings);
    let height = match client.get_blockchain_info().await {
        Ok(info) => info.blocks,
        Err(_) => return false,
    };

    let mut all_coins = Vec::new();
    for addr in &addresses {
        if let Ok(mut coins) = client.get_coins_by_address(addr).await {
            all_coins.append(&mut coins);
        }
    }

    let mut conn = match open_conn(db_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    apply_node_sync_batch(&mut conn, profile_id, &all_coins, height).is_ok()
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
async fn node_discover_step(db_path: &str, profile_id: &str) {
    let (settings, hashes) = {
        let conn = match open_conn(db_path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let settings = match queries::get_settings(&conn) {
            Ok(s) => s,
            Err(_) => return,
        };
        let hashes =
            queries::list_unspent_wallet_name_hashes(&conn, profile_id).unwrap_or_default();
        (settings, hashes)
    };
    if hashes.is_empty() {
        return;
    }

    let client = NodeRpcClient::from_settings(&settings);

    // Resolve each hash → name (prefer node's `getnamebyhash`; fall back to the
    // coin's own rawName when the node can't resolve it). De-dup so we call
    // getnameinfo at most once per name.
    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for h in &hashes {
        let resolved = match client.get_name_by_hash(&h.name_hash_hex).await {
            Ok(Some(n)) => Some(n),
            Ok(None) | Err(_) => h
                .raw_name_hex
                .as_deref()
                .and_then(|hex| hex::decode(hex).ok())
                .and_then(|bytes| String::from_utf8(bytes).ok()),
        };
        if let Some(name) = resolved {
            let trimmed = name.trim().to_ascii_lowercase();
            if !trimmed.is_empty() {
                names.insert(trimmed);
            }
        }
    }
    if names.is_empty() {
        return;
    }

    // Fetch each name's authoritative state from the node.
    let mut fetched: Vec<(String, serde_json::Value)> = Vec::new();
    for name in &names {
        if let Ok(info) = client.get_name_info(name).await {
            fetched.push((name.clone(), info));
        }
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
const SYNC_ERROR_BACKOFF: Duration = Duration::from_secs(2);
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

/// Read the cancellation flag without holding the lock across the caller's work.
async fn cancel_requested(status: &Arc<Mutex<SyncStatus>>) -> bool {
    status.lock().await.cancel_requested
}

/// Repair owned names, looping over successive candidate windows until the whole
/// backlog is checked (true convergence) — so ONE Sync click finishes the job in
/// the background instead of the user re-clicking once per window.
async fn repair_step(status: &Arc<Mutex<SyncStatus>>, db_path: &str, profile_id: &str) {
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
/// * Cancellation — `cancel_requested` is checked at the top of each window AND
///   before each per-name check, so Stop is responsive even mid-window.
pub(crate) async fn repair_step_windowed(
    status: &Arc<Mutex<SyncStatus>>,
    db_path: &str,
    profile_id: &str,
    window: u32,
) {
    let (explorer, all_addresses, total_backlog) = {
        let conn = match open_conn(db_path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let settings = match queries::get_settings(&conn) {
            Ok(s) => s,
            Err(_) => return,
        };
        let explorer = crate::providers::explorer_client_from_settings(&settings);
        let addrs = queries::get_profile_addresses(&conn, profile_id).unwrap_or_default();
        // Total backlog counted once: the stable "/ N" denominator for progress.
        let total =
            queries::count_repair_candidates(&conn, profile_id, REPAIR_MIN_AGE_HOURS).unwrap_or(0);
        (explorer, addrs, total)
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
        if cancel_requested(status).await {
            let mut s = status.lock().await;
            s.waiting = false;
            s.progress_label = "Sync cancelled".into();
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
            if cancel_requested(status).await {
                let mut s = status.lock().await;
                s.waiting = false;
                s.progress_label = "Sync cancelled".into();
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
/// A run is "clean" iff it reached here with no cancellation and no
/// aborting error: `SyncStatus.errors` is ONLY ever pushed to by
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
pub(crate) async fn stamp_explorer_sync_if_clean(
    status: &Arc<Mutex<SyncStatus>>,
    db_path: &str,
    profile_id: &str,
) {
    let clean = {
        let s = status.lock().await;
        !s.cancel_requested && s.errors.is_empty()
    };
    if !clean {
        return;
    }
    if let Ok(conn) = open_conn(db_path) {
        let _ = queries::stamp_explorer_sync(&conn, profile_id);
    }
}

pub(crate) async fn discover_step(
    status: &Arc<Mutex<SyncStatus>>,
    db_path: &str,
    profile_id: &str,
) {
    let (explorer, addrs) = {
        let conn = match open_conn(db_path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let settings = match queries::get_settings(&conn) {
            Ok(s) => s,
            Err(_) => return,
        };
        let explorer = crate::providers::explorer_client_from_settings(&settings);
        let addrs = queries::get_profile_addresses(&conn, profile_id).unwrap_or_default();
        (explorer, addrs)
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
        if cancel_requested(status).await {
            let mut s = status.lock().await;
            s.waiting = false;
            s.progress_label = "Sync cancelled".into();
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
            if cancel_requested(status).await {
                let mut s = status.lock().await;
                s.waiting = false;
                s.progress_label = "Sync cancelled".into();
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
                if cancel_requested(status).await {
                    let mut s = status.lock().await;
                    s.waiting = false;
                    s.progress_label = "Sync cancelled".into();
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
        if cancel_requested(status).await {
            let mut s = status.lock().await;
            s.waiting = false;
            s.progress_label = "Sync cancelled".into();
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
}
