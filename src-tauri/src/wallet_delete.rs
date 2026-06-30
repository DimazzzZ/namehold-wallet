use crate::error::AppError;
use std::path::Path;

/// Delete a wallet by renaming the wallet database folder.
/// This is the safest approach since hsd doesn't support wallet deletion via API.
/// Returns the backup path.
pub fn delete_wallet_folder(wallet_db_path: &Path) -> Result<std::path::PathBuf, AppError> {
    if !wallet_db_path.exists() {
        return Err(AppError::Other(format!(
            "Wallet database not found at: {}",
            wallet_db_path.display()
        )));
    }

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let backup_path = wallet_db_path.with_extension(format!("bak.{}", timestamp));

    std::fs::rename(wallet_db_path, &backup_path)
        .map_err(|e| AppError::Other(format!("Failed to rename wallet folder: {}", e)))?;

    Ok(backup_path)
}

/// Get the wallet database path based on hsd prefix
pub fn get_wallet_db_path(hsd_prefix: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(hsd_prefix).join("wallet")
}
