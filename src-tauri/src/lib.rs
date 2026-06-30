mod commands;
mod db;
mod error;
mod hsd;
mod models;
mod namebase;
mod providers;
mod wallet_delete;
#[cfg(test)]
mod tests;

use std::sync::Mutex;
use tauri::Manager;

pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
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
            commands::wallet::check_connection,
            commands::wallet::get_wallet_info,
            commands::wallet::get_balance,
            commands::wallet::get_address,
            commands::wallet::get_names,
            commands::wallet::get_name_info,
            commands::wallet::get_resource,
            commands::wallet::get_transactions,
            commands::wallet::list_wallets,
            commands::wallet::create_wallet,
            commands::wallet::delete_wallet,
            commands::wallet::get_mnemonic,
            commands::wallet::send_hns,
            commands::wallet::transfer_name,
            commands::wallet::cancel_transfer,
            commands::wallet::get_pending_transactions,
            commands::wallet::get_transaction,
            commands::wallet::get_coins,
            commands::wallet::lock_wallet,
            commands::wallet::unlock_wallet,
            commands::wallet::change_passphrase,
            commands::wallet::get_account,
            commands::wallet::validate_address,
            commands::wallet::estimate_fee,
            commands::sync::sync_names,
            commands::sync::get_sync_report,
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
            commands::node::get_node_status,
            commands::node::stop_hsd,
            commands::node::start_hsd,
            commands::node::get_hsd_log,
            commands::read::get_read_context,
            commands::read::read_balance,
            commands::read::read_names,
            commands::read::read_name_info,
            commands::read::read_transactions,
            commands::read::get_wallet_read_model,
            commands::read::compare_inventory_with_provider,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
