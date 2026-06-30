mod commands;
mod db;
mod error;
mod hsd;
mod models;
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
            commands::wallet::send_hns,
            commands::wallet::transfer_name,
            commands::sync::sync_names,
            commands::sync::get_sync_report,
            commands::settings::get_settings,
            commands::settings::update_setting,
            commands::settings::get_audit_log,
            commands::settings::get_wallet_snapshots,
            commands::csv::import_csv,
            commands::csv::export_csv,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
