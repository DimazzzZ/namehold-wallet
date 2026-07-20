use rusqlite::Connection;

const MIGRATIONS: &[(&str, &str)] = &[
    ("001", include_str!("../sql/001_initial.sql")),
    ("002", include_str!("../sql/002_hsd_prefix.sql")),
    ("003", include_str!("../sql/003_provider_modes.sql")),
    ("004", include_str!("../sql/004_wallet_addresses.sql")),
    ("005", include_str!("../sql/005_fix_hnsfans_api_url.sql")),
    ("006", include_str!("../sql/006_noncustodial_wallet_profiles.sql")),
    ("007", include_str!("../sql/007_noncustodial_chain_cache.sql")),
    ("008", include_str!("../sql/008_noncustodial_name_state.sql")),
    ("009", include_str!("../sql/009_node_rpc_settings.sql")),
    ("010", include_str!("../sql/010_drop_legacy_settings.sql")),
    ("011", include_str!("../sql/011_hsd_data_dir.sql")),
    ("012", include_str!("../sql/012_tx_draft_confirmations.sql")),
    ("013", include_str!("../sql/013_owner_address.sql")),
    ("014", include_str!("../sql/014_reveal_end_height.sql")),
    ("015", include_str!("../sql/015_coin_reservation.sql")),
    ("016", include_str!("../sql/016_last_explorer_sync_at.sql")),
];

pub fn run(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS schema_version (
            version TEXT PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        ",
    )?;

    for (version, sql) in MIGRATIONS {
        let exists: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM schema_version WHERE version = ?1",
            [version],
            |row| row.get(0),
        )?;
        if !exists {
            conn.execute_batch(sql)?;
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                [version],
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_applies_all_migrations() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();
        // All 16 migrations should be present
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 16, "expected 16 migrations, got {count}");
    }

    #[test]
    fn run_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();
        run(&conn).unwrap(); // second run should not error
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 16);
    }

    #[test]
    fn run_creates_tables() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();
        // Spot-check that key tables exist
        for table in &["assets", "batches", "settings", "wallet_profiles", "wallet_tx_drafts"] {
            let exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(exists, "table '{table}' should exist");
        }
    }
}
