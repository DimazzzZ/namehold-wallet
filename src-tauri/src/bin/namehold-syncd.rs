//! `namehold-syncd` — background sync daemon.
//!
//! Runs continuously (60s interval), syncing all wallet profiles from the
//! HSD node into the local SQLite database at `~/.namehold/portfolio.db`.
//!
//! Spawned by the Tauri app as a detached process when the user enables the
//! "Sync in background" setting. Coordinates with the app via a database-backed
//! sync lock (see `db::sync_lock`) so both never write the same profile
//! concurrently.
//!
//! Read-only: never signs or broadcasts transactions.

use namehold_wallet_lib::daemon;

fn main() {
    // Resolve the DB path from the user's home directory.
    // Matches the path resolution used by the Tauri app (see `lib.rs`).
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            eprintln!("namehold-syncd: could not resolve home directory; exiting");
            std::process::exit(1);
        }
    };

    let db_path = home.join(".namehold").join("portfolio.db");
    let db_path_str = db_path.to_string_lossy().to_string();

    // Ensure the DB directory exists (in case the user has never run the app).
    if let Some(parent) = db_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!(
                "namehold-syncd: could not create data directory {}: {}",
                parent.display(),
                e
            );
            std::process::exit(1);
        }
    }

    // Install a Ctrl+C / SIGTERM handler for graceful shutdown.
    let db_for_cleanup = db_path_str.clone();
    ctrlc::set_handler(move || {
        eprintln!("namehold-syncd: received shutdown signal");
        let _ = daemon::cleanup(&db_for_cleanup);
        std::process::exit(0);
    })
    .ok(); // If we can't install the handler, keep running anyway.

    if let Err(e) = daemon::run(&db_path_str) {
        eprintln!("namehold-syncd: fatal error: {e}");
        let _ = daemon::cleanup(&db_path_str);
        std::process::exit(1);
    }
}
