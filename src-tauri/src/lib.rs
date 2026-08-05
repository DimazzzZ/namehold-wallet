#![allow(dead_code)]

pub mod commands;
pub mod daemon;
pub mod db;
pub mod error;
mod hsd;
mod models;
mod namebase;
pub mod noncustodial;
mod providers;
mod security;
#[cfg(test)]
mod tests;
mod wallet_delete;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use tauri::Manager;

use crate::commands::secure_prompt::PendingPrompt;
use crate::commands::sync::SyncStatus;
use crate::noncustodial::session::SignerSession;
use tokio::sync::Mutex as AsyncMutex;

pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
    /// The currently-unlocked signer session, if any.
    ///
    /// This holds decrypted key material in memory ONLY; it is never persisted.
    /// The sole on-disk form of the secret is the encrypted vault blob. The
    /// session locks (and zeroizes) on lock/expiry/drop.
    pub signer: Mutex<Option<SignerSession>>,
    /// In-flight secure prompts, keyed by prompt id. Holds secret request
    /// material (e.g. a mnemonic to reveal) in memory only, until answered.
    pub secure_prompts: Mutex<HashMap<String, PendingPrompt>>,
    /// Handle to the hsd node the app started this session, if any. Used to
    /// report running state and to stop the node. Not persisted across restarts.
    pub hsd_child: Mutex<Option<std::process::Child>>,
    /// Persistent sync session progress. Survives page navigation.
    pub sync_status: Arc<AsyncMutex<SyncStatus>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init())
        // Opens external URLs (explorer tx links, etc.) in the system
        // browser. Without this the Tauri webview silently blocks
        // `window.open` / anchor clicks to external hosts.
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Auto-updater (desktop only). The plugin verifies Ed25519
            // signatures against `plugins.updater.pubkey` before installing;
            // `PendingUpdate` holds a checked update between the check and
            // install commands. `process` enables `relaunch()` post-install.
            #[cfg(desktop)]
            {
                use std::sync::Mutex as StdMutex;
                app.handle()
                    .plugin(tauri_plugin_updater::Builder::new().build())?;
                app.handle().plugin(tauri_plugin_process::init())?;
                app.manage(commands::updates::app_updates::PendingUpdate(
                    StdMutex::new(None),
                ));
            }

            // Store everything under ~/.namehold (pairs with the node's ~/.hsd),
            // rather than the OS app-data dir derived from the bundle identifier.
            // The identifier stays reverse-DNS for packaging/signing but no longer
            // dictates where data lives.
            let db_path = app
                .path()
                .home_dir()
                .expect("failed to get home dir")
                .join(".namehold")
                .join("portfolio.db");
            std::fs::create_dir_all(db_path.parent().unwrap()).expect("failed to create data dir");
            let conn = db::connection::open(&db_path).expect("failed to open database");
            db::migrations::run(&conn).expect("failed to run migrations");

            // One-shot backfill for subdomain names (pre-fix imports had just the
            // parent TLD; now we compose {subdomain}.{domain}). Gated by a settings
            // marker so it runs exactly once.
            let needs_backfill = db::queries::get_settings(&conn)
                .ok()
                .map(|s| !s.contains_key("namebase_history_subdomain_name_backfilled_v1"))
                .unwrap_or(true);
            if needs_backfill {
                if let Err(e) = db::namebase_history::backfill_subdomain_names(&conn) {
                    eprintln!("warning: subdomain name backfill failed: {}", e);
                } else {
                    // Mark it done so we don't run again.
                    let _ = db::queries::set_setting(
                        &conn,
                        "namebase_history_subdomain_name_backfilled_v1",
                        "1",
                    );
                }
            }

            app.manage(AppState {
                db: Mutex::new(conn),
                signer: Mutex::new(None),
                secure_prompts: Mutex::new(HashMap::new()),
                hsd_child: Mutex::new(None),
                sync_status: Arc::new(AsyncMutex::new(SyncStatus::default())),
            });

            // Deadline scanner (I1): on start + every ~10 minutes, look for
            // reveal windows / renewals closing soon and fire an OS
            // notification (gated + deduped inside `scan_deadline_notifications`
            // itself — this loop just decides WHEN to ask).
            //
            // Design choice: a single Tauri-managed background task beats a
            // frontend timer here — it runs independently of which page is
            // open (or whether any window is focused at all), and
            // `tokio::time::interval`'s first `tick()` resolves immediately,
            // so "on start" and "every 10 min" are the same loop, not two
            // mechanisms. It does not persist across a full app quit (there is
            // no OS-level background service), but the very next launch
            // re-scans right away, so nothing missed while closed goes
            // unnoticed for more than "until the user reopens the app".
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(600));
                loop {
                    interval.tick().await;
                    let state = handle.state::<AppState>();
                    if let Err(e) =
                        commands::deadlines::scan_deadline_notifications(handle.clone(), state)
                            .await
                    {
                        eprintln!("deadline scan failed: {e}");
                    }
                }
            });

            // Autostart hsd (default ON). Fires once at launch, before any
            // window trigger, so the node comes up without a "Node: Offline"
            // flash while the frontend mounts. Fire-and-forget: any failure
            // (misconfigured paths, version gate, spawn error) is logged and
            // left for the user to resolve via Settings — it never blocks app
            // startup. `start_hsd` is idempotent (it adopts an already-running
            // node via RPC rather than spawning a duplicate), so this is safe
            // even when the user launched hsd manually beforehand.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<AppState>();
                let enabled = {
                    match state.db.lock() {
                        Ok(db) => match crate::db::queries::get_settings(&db) {
                            // Missing key → default ON, matching DEFAULT_SETTINGS.
                            Ok(settings) => {
                                settings.get("autostart_hsd").map(String::as_str) != Some("false")
                            }
                            Err(e) => {
                                eprintln!(
                                    "autostart: could not read settings ({e}); defaulting ON"
                                );
                                true
                            }
                        },
                        Err(e) => {
                            eprintln!("autostart: settings DB lock poisoned ({e}); skipping");
                            return;
                        }
                    }
                };
                if !enabled {
                    return;
                }
                if let Err(e) = commands::node::start_hsd(state).await {
                    eprintln!("hsd autostart skipped: {e}");
                }
            });

            // Chain scanner (Feature 3, Stage 2): a background task that walks
            // blocks from the fully-synced local node and indexes BID/REVEAL
            // covenant outputs per name into `name_bid_outpoints`. This is what
            // lets `read_name_bids` show ALL bidders — not just the wallet's
            // own — without touching the HNSFans explorer once the node is
            // authoritative. The scanner idles (30s poll) when the node is
            // disconnected or still syncing, and sleeps (10s) when it's caught
            // up to the tip. Resumable via `chain_scan_cursor` — the cost of
            // an app restart is one `getblockchaininfo` + one cursor read, not
            // a re-scan.
            let db_path_str = db_path.to_string_lossy().to_string();
            tauri::async_runtime::spawn(async move {
                commands::chain_scan::run_chain_scanner(db_path_str).await;
            });

            // Background sync daemon: if the user has background sync enabled
            // (default ON), ensure the daemon process is alive. If it crashed
            // or the machine rebooted, respawn it.
            {
                let settings = match app.state::<AppState>().db.lock() {
                    Ok(db) => crate::db::queries::get_settings(&db).unwrap_or_default(),
                    Err(_) => Default::default(),
                };
                commands::daemon_ctl::ensure_daemon_if_enabled(&settings);
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::assets::list_assets,
            commands::assets::get_asset,
            commands::assets::update_asset,
            commands::assets::bulk_update_status,
            commands::assets::bulk_update_tags,
            commands::assets::delete_asset,
            commands::assets::get_dashboard_stats,
            commands::batches::list_batches,
            commands::batches::get_batch_with_assets,
            commands::batches::create_batch,
            commands::batches::update_batch,
            commands::batches::delete_batch,
            commands::batches::add_to_batch,
            commands::batches::remove_from_batch,
            commands::settings::get_settings,
            commands::settings::update_setting,
            commands::settings::get_audit_log,
            commands::settings::get_wallet_snapshots,
            commands::csv::import_csv,
            commands::csv::export_csv,
            commands::namebase::connect_namebase,
            commands::namebase::disconnect_namebase,
            commands::namebase::get_namebase_status,
            commands::namebase::fetch_namebase_domains,
            commands::namebase::fetch_namebase_staked,
            commands::namebase::fetch_namebase_renewals,
            commands::namebase::fetch_namebase_withdrawals,
            commands::namebase::import_from_namebase,
            commands::namebase::namebase_transfer_domain,
            commands::namebase::namebase_withdraw_hns,
            commands::namebase::fetch_namebase_domain_withdrawals,
            commands::namebase_history::import_namebase_history_from_file,
            commands::namebase_history::import_namebase_history_live,
            commands::namebase_history::get_namebase_history,
            commands::namebase_history::get_namebase_history_summary,
            commands::namebase_history::clear_namebase_history,
            commands::node::node_status,
            commands::node::resync_hsd_chain,
            commands::node::start_hsd,
            commands::node::stop_hsd,
            commands::read::read_balance,
            commands::read::read_names,
            commands::read::read_auction_position_names,
            commands::read::discover_owned_names,
            commands::read::read_name_info,
            commands::read::read_name_bids,
            commands::read::read_name_records,
            commands::read::read_block_info,
            commands::read::read_tx_info,
            commands::read::read_transactions,
            commands::read::read_renewals,
            commands::history::read_action_history,
            commands::read::repair_owned_names,
            commands::sync::start_full_sync,
            commands::sync::get_sync_status,
            commands::sync::cancel_full_sync,
            commands::daemon_ctl::is_background_sync_enabled,
            commands::daemon_ctl::set_background_sync_enabled,
            commands::daemon_ctl::is_daemon_alive,
            commands::read::compare_inventory_with_provider,
            commands::secure_prompt::secure_prompt_fetch,
            commands::secure_prompt::secure_prompt_submit,
            commands::secure_wallet::secure_create_wallet,
            commands::secure_wallet::secure_import_wallet,
            commands::secure_wallet::secure_reveal_backup_phrase,
            commands::secure_wallet::unlock_local_signer,
            commands::secure_wallet::lock_local_signer,
            commands::secure_wallet::get_signer_session,
            commands::secure_wallet::list_wallet_profiles,
            commands::secure_wallet::set_active_wallet_profile,
            commands::secure_wallet::delete_wallet_profile,
            commands::tx::sync_wallet_state,
            commands::tx::sync_tracked_names,
            commands::tx::build_send_hns_draft,
            commands::tx::estimate_tx_draft_fee,
            commands::tx::sign_tx_draft,
            commands::tx::sign_name_message,
            commands::tx::broadcast_tx_draft,
            commands::tx::refresh_tx_confirmations,
            commands::tx::list_tx_drafts,
            commands::tx::delete_tx_draft,
            commands::tx::release_tx_draft_reservation,
            commands::tx::get_write_capability,
            commands::tx::get_wallet_balances,
            commands::names::build_open_draft,
            commands::names::build_bid_draft,
            commands::names::build_reveal_draft,
            commands::names::build_redeem_draft,
            commands::names::build_register_draft,
            commands::names::build_update_draft,
            commands::names::build_renew_draft,
            commands::names::build_transfer_draft,
            commands::names::build_finalize_draft,
            commands::names::build_cancel_draft,
            commands::names::build_revoke_draft,
            commands::names::build_batch_renew_draft,
            commands::names::build_batch_reveal_draft,
            commands::names::build_batch_redeem_draft,
            commands::names::get_name_action_capabilities,
            commands::names::get_names_action_capabilities,
            commands::bids::recover_bid_commitment,
            commands::bids::export_bid_commitments,
            commands::watchlist::add_to_watchlist,
            commands::watchlist::remove_from_watchlist,
            commands::watchlist::list_watchlist,
            commands::watchlist::is_watched,
            commands::deadlines::scan_deadline_notifications,
            #[cfg(desktop)]
            commands::updates::app_updates::check_for_update,
            #[cfg(desktop)]
            commands::updates::app_updates::install_update,
            #[cfg(desktop)]
            commands::updates::app_updates::current_version,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // `RunEvent::Exit` fires exactly once, right before the event loop
            // stops, regardless of how the app is closing (last window closed,
            // Cmd+Q, `AppHandle::exit`/`restart`, `ExitRequested` left
            // unprevented, …) — so hooking only this one event is enough to
            // reap the hsd child on every exit path. `ExitRequested` fires
            // earlier and can be cancelled by a listener (`api.prevent_exit()`),
            // so it's the wrong place to kill anything: it may not represent an
            // actual exit at all.
            if let tauri::RunEvent::Exit = event {
                let state = app_handle.state::<AppState>();

                // If background sync is enabled, SKIP killing hsd — the daemon
                // needs it alive to continue syncing after the app closes. The
                // hsd process becomes orphaned but alive; next app launch adopts
                // it via RPC probe (see start_hsd's existing adoption path).
                let background_sync_on = state
                    .db
                    .lock()
                    .ok()
                    .and_then(|db| crate::db::queries::get_settings(&db).ok())
                    .map(|s| {
                        s.get(commands::daemon_ctl::SETTING_BACKGROUND_SYNC)
                            .unwrap_or(&commands::daemon_ctl::BACKGROUND_SYNC_DEFAULT.to_string())
                            == "1"
                    })
                    .unwrap_or(false);

                let child = match state.hsd_child.lock() {
                    Ok(mut guard) => guard.take(),
                    Err(poisoned) => poisoned.into_inner().take(),
                };
                if let Some(mut child) = child {
                    if background_sync_on {
                        // Detach: drop the handle without killing. The child
                        // process continues running as an orphan.
                        eprintln!(
                            "hsd shutdown: background sync enabled — leaving hsd alive for daemon"
                        );
                        drop(child);
                    } else {
                        // Best-effort: never let a stuck hsd block the app from
                        // closing. `kill()` + `wait()` on an already-exited child
                        // are harmless no-ops (kill fails silently, wait returns
                        // immediately), so this is safe to run unconditionally.
                        if let Err(e) = child.kill() {
                            eprintln!("hsd shutdown: kill failed (may already be dead): {e}");
                        }
                        if let Err(e) = child.wait() {
                            eprintln!("hsd shutdown: wait failed: {e}");
                        }
                    }
                }
            }
        });
}
