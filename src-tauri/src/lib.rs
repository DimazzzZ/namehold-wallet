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
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Manager, WindowEvent};

use crate::commands::secure_prompt::PendingPrompt;
use crate::commands::sync::SyncStatus;
use crate::noncustodial::session::SignerSession;
use tokio::sync::Mutex as AsyncMutex;

/// Bring the main window back to the foreground: unminimize, show (in case it
/// was hidden to tray), and focus. Used by the tray "Open" menu item and by a
/// left-click on the tray icon. Errors are logged, not propagated — a failed
/// focus should never crash the event loop.
fn show_main_window(app: &tauri::AppHandle) {
    // macOS: restore the Dock icon in case we hid it when the window was
    // closed to tray. No-op on other platforms.
    #[cfg(target_os = "macos")]
    let _ = app.set_dock_visibility(true);

    if let Some(win) = app.get_webview_window("main") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
}

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
    /// Whether the hsd RPC endpoint is currently reachable. Updated by the
    /// backend probe loop (every ~5s) and immediately after start/stop actions.
    /// Read by the tray to determine whether to show "Running" or "Stopped",
    /// independent of whether we spawned the child (adoption case).
    pub node_rpc_alive: std::sync::atomic::AtomicBool,
    /// Persistent sync session progress. Survives page navigation.
    pub sync_status: Arc<AsyncMutex<SyncStatus>>,
}

/// macOS notification sender bundle identifier. Must match `identifier` in
/// `tauri.conf.json` so the OS attributes notifications to Namehold (name +
/// icon) instead of the launching process (e.g. Terminal in dev).
#[cfg(target_os = "macos")]
const NOTIFY_BUNDLE_ID: &str = "org.zhavoronkov.nameholdwallet";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // macOS: claim the notification sender identity as Namehold BEFORE the
    // notification plugin initializes. `notify_rust::set_application` is
    // `Once`-guarded, so the first caller wins for the whole process. This
    // pre-empts `tauri-plugin-notification`'s dev-mode fallback, which
    // otherwise calls `set_application("com.apple.Terminal")` when
    // `tauri::is_dev()` is true (see the plugin's `desktop.rs`), making both
    // this app AND the standalone `notify-rust` calls show "Terminal".
    //
    // The sender NAME always resolves; the ICON resolves only when the
    // bundled `.app` is registered with Launch Services (i.e. a built `.app`
    // has been opened at least once). Release installs register on first
    // open, so end users always see the Namehold name + icon. Harmless
    // no-op otherwise — the notification text is always correct.
    #[cfg(target_os = "macos")]
    let _ = notify_rust::set_application(NOTIFY_BUNDLE_ID);

    // `mut` is only used in release builds (the autostart plugin is added
    // conditionally below); allow the unused-mut warning in debug builds.
    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init())
        // Opens external URLs (explorer tx links, etc.) in the system
        // browser. Without this the Tauri webview silently blocks
        // `window.open` / anchor clicks to external hosts.
        .plugin(tauri_plugin_opener::init());

    // "Launch at login" toggle in Settings. Uses each OS's native
    // mechanism: LaunchAgent plist on macOS, HKCU\...\Run on Windows,
    // .desktop autostart entry on Linux. `None` = no extra args on
    // autostart (the app starts normally).
    //
    // Disabled in debug builds: dev binaries live under target/debug/ and
    // writing a LaunchAgent pointing there breaks login-launch when the
    // user also has a release .app installed.
    #[cfg(not(debug_assertions))]
    {
        builder = builder.plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ));
    }

    builder
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
                node_rpc_alive: std::sync::atomic::AtomicBool::new(false),
                sync_status: Arc::new(AsyncMutex::new(SyncStatus::default())),
            });

            // -----------------------------------------------------------------
            // System tray (menu bar on macOS). Lets the user close the window
            // while the app keeps running (hsd + background daemon stay alive),
            // and reopen it, control the node, and quit — all from the tray.
            // -----------------------------------------------------------------
            {
                let open = MenuItem::with_id(app, "open", "Open Namehold", true, None::<&str>)?;
                // Disabled status label — informational only.
                let status = MenuItem::with_id(app, "status", "Node: …", false, None::<&str>)?;
                let toggle =
                    MenuItem::with_id(app, "toggle_node", "Start Node", true, None::<&str>)?;
                let bgsync = CheckMenuItem::with_id(
                    app,
                    "bgsync",
                    "Background sync",
                    true,
                    false,
                    None::<&str>,
                )?;
                let sep = PredefinedMenuItem::separator(app)?;
                let quit = MenuItem::with_id(app, "quit", "Quit Namehold", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&open, &status, &toggle, &bgsync, &sep, &quit])?;

                let tray = TrayIconBuilder::with_id("main-tray")
                    .icon(tauri::include_image!("icons/tray-normal.png"))
                    // On macOS, treat the icon as a template image so it
                    // adapts to light/dark menu bar. On other platforms this
                    // is a no-op. State is conveyed via glyph variants
                    // (filled / outline / badge) rather than color.
                    .icon_as_template(true)
                    .menu(&menu)
                    // Show the menu on BOTH left and right click. This
                    // matches every other macOS menu-bar app (Slack, Docker,
                    // 1Password, etc.) and avoids relying on AppKit to
                    // deliver a MouseButton::Left / Up event to our handler
                    // — that event does not reliably fire when a menu is
                    // attached to the NSStatusItem. "Open Namehold" is the
                    // first menu item so activating the window is one click
                    // away.
                    .show_menu_on_left_click(true)
                    .on_menu_event(|app, event| {
                        let app = app.clone();
                        match event.id().as_ref() {
                            "open" => show_main_window(&app),
                            "toggle_node" => {
                                // Start or stop hsd depending on current state,
                                // then refresh the tray. Runs on the async
                                // runtime so we don't block the menu callback.
                                tauri::async_runtime::spawn(async move {
                                    let running = {
                                        let state = app.state::<AppState>();
                                        // Match tray snapshot: reachable RPC OR
                                        // a child we spawned counts as running.
                                        // Fixes the adopted-node case where hsd
                                        // is up but hsd_child is None.
                                        state
                                            .node_rpc_alive
                                            .load(std::sync::atomic::Ordering::Relaxed)
                                            || state
                                                .hsd_child
                                                .lock()
                                                .ok()
                                                .map(|g| g.is_some())
                                                .unwrap_or(false)
                                    };
                                    let result = if running {
                                        commands::node::stop_hsd(app.state::<AppState>())
                                            .await
                                            .map(|_| ())
                                    } else {
                                        commands::node::start_hsd(app.state::<AppState>())
                                            .await
                                            .map(|_| ())
                                    };
                                    if let Err(e) = result {
                                        eprintln!("tray: toggle node failed: {e}");
                                    }
                                    commands::tray::refresh_tray(&app);
                                });
                            }
                            "bgsync" => {
                                tauri::async_runtime::spawn(async move {
                                    // Flip based on the current persisted value.
                                    let currently_on = {
                                        let state = app.state::<AppState>();
                                        commands::daemon_ctl::is_background_sync_enabled(state)
                                            .await
                                            .unwrap_or(true)
                                    };
                                    if let Err(e) =
                                        commands::daemon_ctl::set_background_sync_enabled(
                                            app.state::<AppState>(),
                                            !currently_on,
                                        )
                                        .await
                                    {
                                        eprintln!("tray: toggle background sync failed: {e}");
                                    }
                                    commands::tray::refresh_tray(&app);
                                });
                            }
                            "quit" => {
                                commands::tray::REALLY_QUITTING
                                    .store(true, std::sync::atomic::Ordering::SeqCst);
                                app.exit(0);
                            }
                            _ => {}
                        }
                    })
                    .build(app)?;

                app.manage(commands::tray::TrayState {
                    tray,
                    status,
                    toggle,
                    bgsync,
                });

                // Intercept the MAIN window's close button. When "close to
                // tray" is on (default) and we're not in a real quit, hide the
                // window instead of closing it — hsd and the daemon keep
                // running, and the tray icon brings the window back. When the
                // setting is off, exit explicitly so `RunEvent::Exit` fires and
                // the hsd-reap path runs (on macOS, closing the last window
                // does NOT terminate the process on its own).
                if let Some(win) = app.get_webview_window("main") {
                    let handle = app.handle().clone();
                    win.on_window_event(move |event| {
                        if let WindowEvent::CloseRequested { api, .. } = event {
                            if commands::tray::REALLY_QUITTING
                                .load(std::sync::atomic::Ordering::SeqCst)
                            {
                                return; // Real quit in progress — let it close.
                            }
                            if commands::tray::read_close_to_tray_setting(&handle) {
                                api.prevent_close();
                                if let Some(w) = handle.get_webview_window("main") {
                                    let _ = w.hide();

                                    // macOS: hide from Dock when closing to tray.
                                    #[cfg(target_os = "macos")]
                                    let _ = handle.set_dock_visibility(false);

                                    // Fire first-time tray notification (async, fire-and-forget).
                                    tauri::async_runtime::spawn({
                                        let handle = handle.clone();
                                        async move {
                                            if let Err(e) =
                                                commands::tray::fire_tray_hint_notification(&handle)
                                                    .await
                                            {
                                                eprintln!("tray hint notification failed: {e}");
                                            }
                                        }
                                    });
                                }
                            } else {
                                handle.exit(0);
                            }
                        }
                    });
                }

                // Initial paint, then a light ticker. The ticker is the
                // reconciliation path for tray state that changes WITHOUT going
                // through a tray menu action: the frontend calling
                // `start_hsd`/`stop_hsd`/`set_background_sync_enabled` via
                // invoke, hsd autostart at launch, and passive sync-progress
                // transitions. Tray-initiated actions refresh eagerly in their
                // own handlers, so this only needs to be "reasonably fresh".
                let handle = app.handle().clone();
                commands::tray::refresh_tray(&handle);
                tauri::async_runtime::spawn(async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3));
                    loop {
                        interval.tick().await;
                        commands::tray::refresh_tray(&handle);
                    }
                });

                // Backend RPC probe loop: every ~5s, check if hsd is reachable
                // and update the `node_rpc_alive` flag. This keeps the tray
                // accurate even when the main window is closed to tray (the
                // frontend's polling pauses when unfocused). The first tick
                // fires immediately so the flag is fresh within a few hundred ms
                // of app launch.
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
                    loop {
                        interval.tick().await;
                        let state = handle.state::<AppState>();
                        let _ = commands::node::probe_and_update(&state).await;
                    }
                });
            }

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
            commands::read::get_resource,
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
            commands::tray::is_close_to_tray_enabled,
            commands::tray::set_close_to_tray_enabled,
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
            commands::names::build_batch_bid_draft,
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
            commands::names::build_batch_finalize_draft,
            commands::names::build_finalize_with_payment_draft,
            commands::names::get_name_action_capabilities,
            commands::names::get_names_action_capabilities,
            commands::bids::recover_bid_commitment,
            commands::bids::brute_force_recover_bid,
            commands::bids::export_bid_commitments,
            commands::watchlist::add_to_watchlist,
            commands::watchlist::remove_from_watchlist,
            commands::watchlist::list_watchlist,
            commands::watchlist::is_watched,
            commands::watchlist::get_watchlist_status,
            commands::watchlist::update_watchlist_tags,
            commands::watchlist::export_watchlist_csv,
            commands::watchlist::import_watchlist_csv,
            commands::watched_states::get_watched_states,
            commands::paid_swaps::create_paid_swap_offer,
            commands::paid_swaps::get_paid_swap_offer,
            commands::paid_swaps::claim_paid_transfer,
            commands::paid_swaps::remove_paid_swap_offer,
            commands::deadlines::scan_deadline_notifications,
            #[cfg(all(debug_assertions, not(test)))]
            commands::debug_notify::simulate_notification,
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
            match event {
                // macOS: clicking the Dock icon of a running-but-hidden app
                // fires `Reopen`. Bring the (possibly tray-hidden) window back
                // and restore the Dock icon. Without this, a close-to-tray'd
                // window can't be reopened from the Dock.
                #[cfg(target_os = "macos")]
                tauri::RunEvent::Reopen { .. } => {
                    show_main_window(app_handle);
                }
                // `RunEvent::Exit` fires exactly once, right before the event
                // loop stops, regardless of how the app is closing (last window
                // closed, Cmd+Q, `AppHandle::exit`/`restart`, `ExitRequested`
                // left unprevented, …) — so hooking only this one event is
                // enough to reap the hsd child on every exit path.
                // `ExitRequested` fires earlier and can be cancelled by a
                // listener (`api.prevent_exit()`), so it's the wrong place to
                // kill anything: it may not represent an actual exit at all.
                tauri::RunEvent::Exit => {
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
                                .unwrap_or(
                                    &commands::daemon_ctl::BACKGROUND_SYNC_DEFAULT.to_string(),
                                )
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
                _ => {}
            }
        });
}
