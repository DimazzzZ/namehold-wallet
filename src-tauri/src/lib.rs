mod commands;
mod db;
mod error;
mod hsd;
mod models;
mod namebase;
mod noncustodial;
mod providers;
mod wallet_delete;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::Mutex;
use tauri::Manager;

use crate::commands::secure_prompt::PendingPrompt;
use crate::noncustodial::session::SignerSession;

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
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let db_path = app
                .path()
                .home_dir().expect("failed to get home dir").join(".namehold")
                .join("portfolio.db");
            std::fs::create_dir_all(db_path.parent().unwrap()).expect("failed to create data dir");
            let conn = db::connection::open(&db_path).expect("failed to open database");
            db::migrations::run(&conn).expect("failed to run migrations");
            app.manage(AppState {
                db: Mutex::new(conn),
                signer: Mutex::new(None),
                secure_prompts: Mutex::new(HashMap::new()),
                hsd_child: Mutex::new(None),
            });
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
            commands::node::node_status,
            commands::node::start_hsd,
            commands::node::stop_hsd,
            commands::read::read_balance,
            commands::read::read_names,
            commands::read::discover_owned_names,
            commands::read::read_name_info,
            commands::read::read_transactions,
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
            commands::tx::broadcast_tx_draft,
            commands::tx::list_tx_drafts,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
