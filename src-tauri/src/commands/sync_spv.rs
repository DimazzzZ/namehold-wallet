//! SPV-mode sync step.
//!
//! In SPV mode, hsd runs with `--spv` (no `--index-address`), so the
//! `GET /coin/address/:addr` endpoint is unavailable. SPV mode is **read-only**:
//! balance, names, and DNS records come from the explorer; sending is blocked
//! by the write-capability check (see `get_write_capability` in `tx.rs`).
//!
//! **Important limitation**: The explorer does NOT expose individual UTXOs —
//! only aggregate balance. Therefore, this step does NOT populate the UTXO
//! cache. This is by design: since sending is blocked in SPV mode, individual
//! UTXOs are not needed. The explorer provides balance display directly.
//!
//! Steps 2+3 (repair + discover) use the explorer path, which is the default
//! when `node_authoritative == false` in `run_sync_steps`.
//!
//! This is the SPV counterpart to `sync_node_step` in `sync.rs`.

use crate::db::queries;
use crate::noncustodial::rpc::NodeRpcClient;
use crate::providers::explorer_client_from_settings;

use super::sync::open_conn;

/// SPV-specific sync step.
///
/// In SPV mode, the explorer is the primary data source for balance and names.
/// This step confirms the SPV node is reachable, checks explorer health,
/// and updates the sync cursor. Does NOT populate the UTXO cache because
/// the explorer doesn't expose individual UTXOs (only aggregate balance).
///
/// Since sending is blocked in SPV mode (write-capability check), individual
/// UTXOs are not needed — the explorer provides balance display directly.
///
/// Steps 2+3 (repair + discover) use the explorer path, which is the default
/// when `node_authoritative == false` in `run_sync_steps`.
///
/// Returns `true` if the sync completed successfully.
pub async fn sync_spv_step(db_path: &str, profile_id: &str) -> bool {
    let conn = match open_conn(db_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let settings = match queries::get_settings(&conn) {
        Ok(s) => s,
        Err(_) => return false,
    };
    drop(conn);

    // Verify the SPV node is reachable.
    // Note: SPV mode is read-only. Sending is blocked by write-capability check.
    // Individual UTXOs are not tracked (explorer provides balance display).
    let client = NodeRpcClient::from_settings(&settings);
    let height = match client.get_blockchain_info().await {
        Ok(info) => info.blocks,
        Err(e) => {
            eprintln!("sync_spv_step: SPV node not reachable: {e}");
            return false;
        }
    };

    // Check explorer health (non-blocking, just logs warnings).
    // This helps diagnose explorer connectivity issues early.
    let explorer = explorer_client_from_settings(&settings);
    if let Err(e) = explorer.health().await {
        eprintln!("sync_spv_step: explorer health check failed: {e}");
        // Continue anyway — explorer might be temporarily down, and
        // the repair/discover steps will handle individual request failures.
    }

    // Update the sync cursor to the current height.
    // We don't populate UTXOs (explorer handles balance display),
    // but advancing the cursor keeps the sync state consistent.
    let conn = match open_conn(db_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    if let Err(e) = crate::noncustodial::sync::set_sync_cursor(&conn, profile_id, height) {
        eprintln!("sync_spv_step: failed to update sync cursor: {e}");
    }

    eprintln!(
        "sync_spv_step: SPV sync complete for profile {} at height {}",
        profile_id, height
    );
    true
}
