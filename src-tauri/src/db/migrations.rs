use rusqlite::Connection;

const MIGRATIONS: &[(&str, &str)] = &[
    ("001", include_str!("../sql/001_initial.sql")),
    ("002", include_str!("../sql/002_hsd_prefix.sql")),
    ("003", include_str!("../sql/003_provider_modes.sql")),
    ("004", include_str!("../sql/004_wallet_addresses.sql")),
    ("005", include_str!("../sql/005_fix_hnsfans_api_url.sql")),
    (
        "006",
        include_str!("../sql/006_noncustodial_wallet_profiles.sql"),
    ),
    (
        "007",
        include_str!("../sql/007_noncustodial_chain_cache.sql"),
    ),
    (
        "008",
        include_str!("../sql/008_noncustodial_name_state.sql"),
    ),
    ("009", include_str!("../sql/009_node_rpc_settings.sql")),
    ("010", include_str!("../sql/010_drop_legacy_settings.sql")),
    ("011", include_str!("../sql/011_hsd_data_dir.sql")),
    ("012", include_str!("../sql/012_tx_draft_confirmations.sql")),
    ("013", include_str!("../sql/013_owner_address.sql")),
    ("014", include_str!("../sql/014_reveal_end_height.sql")),
    ("015", include_str!("../sql/015_coin_reservation.sql")),
    ("016", include_str!("../sql/016_last_explorer_sync_at.sql")),
    ("017", include_str!("../sql/017_backfill_bid_txids.sql")),
    ("018", include_str!("../sql/018_name_bid_index.sql")),
    (
        "019",
        include_str!("../sql/019_namebase_cookie_at_rest.sql"),
    ),
    ("020", include_str!("../sql/020_namebase_history.sql")),
    ("021", include_str!("../sql/021_sync_locks.sql")),
    ("022", include_str!("../sql/022_watchlist.sql")),
    ("023", include_str!("../sql/023_paid_swap_offers.sql")),
    ("024", include_str!("../sql/024_watchlist_tags.sql")),
    ("025", include_str!("../sql/025_watched_name_states.sql")),
    (
        "026",
        include_str!("../sql/026_ledger_hardware_profiles.sql"),
    ),
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
        // All migrations should be present
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 26, "expected 26 migrations, got {count}");
    }

    #[test]
    fn run_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();
        run(&conn).unwrap(); // second run should not error
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 26);
    }

    #[test]
    fn run_creates_tables() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();
        // Spot-check that key tables exist
        for table in &[
            "assets",
            "batches",
            "settings",
            "wallet_profiles",
            "wallet_tx_drafts",
        ] {
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

    // -- 017 backfill ---------------------------------------------------

    fn seed_profile(conn: &Connection, id: &str) {
        conn.execute(
            "INSERT INTO wallet_profiles
                (id, label, kind, network, account_xpub, account_index, watch_only)
             VALUES (?1, ?2, 'watch_only_xpub', 'regtest', 'xpub-fake', 0, 1)",
            rusqlite::params![id, format!("profile {id}")],
        )
        .unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    fn seed_commitment(
        conn: &Connection,
        profile_id: &str,
        name: &str,
        blind_hex: &str,
        bid_txid: Option<&str>,
        reveal_txid: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO bid_commitments
                (wallet_profile_id, name, name_hash_hex, address, branch, child_index,
                 bid_value_doos, lockup_value_doos, nonce_hex, blind_hex, bid_txid, reveal_txid)
             VALUES (?1, ?2, 'deadbeef', 'hs1qfake', 0, 0, 100000000, 200000000, 'nonce', ?3, ?4, ?5)",
            rusqlite::params![profile_id, name, blind_hex, bid_txid, reveal_txid],
        )
        .unwrap();
    }

    fn seed_draft(
        conn: &Connection,
        id: &str,
        profile_id: &str,
        action: &str,
        status: &str,
        name: &str,
        txid: &str,
    ) {
        let summary_json = format!(r#"{{"name":"{name}","txid":"{txid}"}}"#);
        conn.execute(
            "INSERT INTO wallet_tx_drafts
                (id, wallet_profile_id, action, unsigned_tx_hex, signing_inputs_json,
                 summary_json, status)
             VALUES (?1, ?2, ?3, 'deadbeef', '[]', ?4, ?5)",
            rusqlite::params![id, profile_id, action, summary_json, status],
        )
        .unwrap();
    }

    fn run_backfill(conn: &Connection) {
        conn.execute_batch(include_str!("../sql/017_backfill_bid_txids.sql"))
            .unwrap();
    }

    fn get_bid_txid(conn: &Connection, profile_id: &str, name: &str) -> Option<String> {
        conn.query_row(
            "SELECT bid_txid FROM bid_commitments WHERE wallet_profile_id = ?1 AND name = ?2",
            rusqlite::params![profile_id, name],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn get_reveal_txid(conn: &Connection, profile_id: &str, name: &str) -> Option<String> {
        conn.query_row(
            "SELECT reveal_txid FROM bid_commitments WHERE wallet_profile_id = ?1 AND name = ?2",
            rusqlite::params![profile_id, name],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn backfill_fills_bid_txid_from_confirmed_bid_draft() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();

        seed_profile(&conn, "profile-a");
        seed_commitment(&conn, "profile-a", "namehold", "blind-1", None, None);
        seed_draft(
            &conn,
            "draft-1",
            "profile-a",
            "bid",
            "confirmed",
            "namehold",
            "d0788cec550272e5631c",
        );

        run_backfill(&conn);

        assert_eq!(
            get_bid_txid(&conn, "profile-a", "namehold"),
            Some("d0788cec550272e5631c".to_string())
        );
    }

    #[test]
    fn backfill_fills_reveal_txid_from_confirmed_reveal_draft() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();

        seed_profile(&conn, "profile-a");
        seed_commitment(
            &conn,
            "profile-a",
            "namehold",
            "blind-1",
            Some("bid-txid-already-set"),
            None,
        );
        seed_draft(
            &conn,
            "draft-reveal-1",
            "profile-a",
            "reveal",
            "confirmed",
            "namehold",
            "reveal-txid-abc",
        );

        run_backfill(&conn);

        assert_eq!(
            get_reveal_txid(&conn, "profile-a", "namehold"),
            Some("reveal-txid-abc".to_string())
        );
        // bid_txid untouched by the reveal backfill.
        assert_eq!(
            get_bid_txid(&conn, "profile-a", "namehold"),
            Some("bid-txid-already-set".to_string())
        );
    }

    #[test]
    fn backfill_is_idempotent_and_never_overwrites_an_existing_txid() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();

        seed_profile(&conn, "profile-a");
        // Already backfilled/forward-written row: must not be touched even
        // though a *different* txid is sitting in a matching draft.
        seed_commitment(
            &conn,
            "profile-a",
            "already-set",
            "blind-1",
            Some("original-txid"),
            None,
        );
        seed_draft(
            &conn,
            "draft-1",
            "profile-a",
            "bid",
            "confirmed",
            "already-set",
            "different-txid-should-not-apply",
        );

        // NULL row that should get backfilled.
        seed_commitment(&conn, "profile-a", "null-name", "blind-2", None, None);
        seed_draft(
            &conn,
            "draft-2",
            "profile-a",
            "bid",
            "confirmed",
            "null-name",
            "fresh-txid",
        );

        run_backfill(&conn);
        run_backfill(&conn); // second run must be a no-op

        assert_eq!(
            get_bid_txid(&conn, "profile-a", "already-set"),
            Some("original-txid".to_string()),
            "pre-existing txid must never be overwritten"
        );
        assert_eq!(
            get_bid_txid(&conn, "profile-a", "null-name"),
            Some("fresh-txid".to_string())
        );

        // Run once more for good measure — values must be stable.
        run_backfill(&conn);
        assert_eq!(
            get_bid_txid(&conn, "profile-a", "already-set"),
            Some("original-txid".to_string())
        );
        assert_eq!(
            get_bid_txid(&conn, "profile-a", "null-name"),
            Some("fresh-txid".to_string())
        );
    }

    #[test]
    fn backfill_respects_per_wallet_isolation() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();

        seed_profile(&conn, "profile-a");
        seed_profile(&conn, "profile-b");

        // profile-a has a bare commitment with no matching draft of its own.
        seed_commitment(&conn, "profile-a", "shared-name", "blind-a", None, None);
        // profile-b has a confirmed draft for the SAME name.
        seed_draft(
            &conn,
            "draft-b",
            "profile-b",
            "bid",
            "confirmed",
            "shared-name",
            "profile-b-txid",
        );

        run_backfill(&conn);

        assert_eq!(
            get_bid_txid(&conn, "profile-a", "shared-name"),
            None,
            "a draft from profile B must never backfill profile A's commitment"
        );
    }

    #[test]
    fn backfill_skips_dropped_and_failed_drafts() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();

        seed_profile(&conn, "profile-a");
        seed_commitment(&conn, "profile-a", "namehold", "blind-1", None, None);
        seed_draft(
            &conn,
            "draft-dropped",
            "profile-a",
            "bid",
            "dropped",
            "namehold",
            "dropped-txid",
        );
        seed_draft(
            &conn,
            "draft-failed",
            "profile-a",
            "bid",
            "failed",
            "namehold",
            "failed-txid",
        );

        run_backfill(&conn);

        assert_eq!(
            get_bid_txid(&conn, "profile-a", "namehold"),
            None,
            "dropped/failed drafts must not be used as a backfill source"
        );
    }

    // -- 026 FK safety (ledger hardware profiles) ---------------------------

    #[test]
    fn migration_026_preserves_child_rows_during_wallet_profiles_rebuild() {
        let conn = Connection::open_in_memory().unwrap();
        // Run migrations 001-025 (stop before 026).
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS schema_version (
                version TEXT PRIMARY KEY,
                applied_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            ",
        )
        .unwrap();
        for (version, sql) in MIGRATIONS.iter().take(25) {
            conn.execute_batch(sql).unwrap();
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                [*version],
            )
            .unwrap();
        }

        // Seed a profile with child rows (addresses, UTXOs, secrets, name states).
        conn.execute(
            "INSERT INTO wallet_profiles
                (id, label, kind, network, account_xpub, account_index, watch_only)
             VALUES (?, ?, 'watch_only_xpub', 'mainnet', 'xpub-fake', 0, 1)",
            rusqlite::params!["profile-1", "Test Profile"],
        )
        .unwrap();

        // Seed derived addresses.
        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index, address,
                 script_pubkey_hex, public_key_hex)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params!["profile-1", 0, 0, 0, "hs1q1234", "0014aa", "02aa"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO derived_addresses
                (wallet_profile_id, account_index, branch, child_index, address,
                 script_pubkey_hex, public_key_hex)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params!["profile-1", 0, 0, 1, "hs1q5678", "0014bb", "02bb"],
        )
        .unwrap();

        // Seed tracked UTXOs.
        conn.execute(
            "INSERT INTO tracked_utxos
                (wallet_profile_id, txid, vout, address, script_pubkey_hex, value_doos)
             VALUES (?, ?, ?, ?, ?, ?)",
            rusqlite::params!["profile-1", "aabbcc", 0, "hs1q1234", "0014aa", 100000],
        )
        .unwrap();

        // Seed tracked name states.
        conn.execute(
            "INSERT INTO tracked_name_states
                (wallet_profile_id, name, name_hash_hex, state, height)
             VALUES (?, ?, ?, ?, ?)",
            rusqlite::params!["profile-1", "myname", "deadbeef", "CLOSED", 100],
        )
        .unwrap();

        // Seed wallet_secrets (encrypted vault).
        conn.execute(
            "INSERT INTO wallet_secrets
                (wallet_profile_id, kdf_salt_hex, nonce_hex, ciphertext_hex, public_fingerprint)
             VALUES (?, ?, ?, ?, ?)",
            rusqlite::params!["profile-1", "salt00", "nonce00", "encrypted_blob", "fp00"],
        )
        .unwrap();

        // Verify child rows exist before migration 026.
        let addr_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM derived_addresses WHERE wallet_profile_id = ?",
                ["profile-1"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(addr_count, 2, "should have 2 addresses before migration 026");

        let utxo_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tracked_utxos WHERE wallet_profile_id = ?",
                ["profile-1"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(utxo_count, 1, "should have 1 UTXO before migration 026");

        let name_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tracked_name_states WHERE wallet_profile_id = ?",
                ["profile-1"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(name_count, 1, "should have 1 name state before migration 026");

        let secret_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM wallet_secrets WHERE wallet_profile_id = ?",
                ["profile-1"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(secret_count, 1, "should have 1 secret before migration 026");

        // Run migration 026 (the one being tested).
        conn.execute_batch(MIGRATIONS[25].1).unwrap();
        conn.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            ["026"],
        )
        .unwrap();

        // Verify child rows SURVIVED the table rebuild.
        let addr_count_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM derived_addresses WHERE wallet_profile_id = ?",
                ["profile-1"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            addr_count_after, 2,
            "migration 026 must NOT cascade-delete derived_addresses"
        );

        let utxo_count_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tracked_utxos WHERE wallet_profile_id = ?",
                ["profile-1"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            utxo_count_after, 1,
            "migration 026 must NOT cascade-delete tracked_utxos"
        );

        let name_count_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tracked_name_states WHERE wallet_profile_id = ?",
                ["profile-1"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            name_count_after, 1,
            "migration 026 must NOT cascade-delete tracked_name_states"
        );

        let secret_count_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM wallet_secrets WHERE wallet_profile_id = ?",
                ["profile-1"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            secret_count_after, 1,
            "migration 026 must NOT cascade-delete wallet_secrets"
        );

        // Verify FK constraints are still enforced after the migration.
        let fk_enabled: i64 = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert_eq!(fk_enabled, 1, "foreign_keys pragma must be ON after migration");

        // Verify no FK violations exist.
        let violations: Vec<String> = conn
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query_map([], |_row| Ok(String::new()))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            violations.is_empty(),
            "migration 026 must not introduce FK violations"
        );
    }

    fn apply_migrations_through_025(conn: &Connection) {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS schema_version (
                version TEXT PRIMARY KEY,
                applied_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            ",
        )
        .unwrap();
        for (version, sql) in MIGRATIONS.iter().take(25) {
            conn.execute_batch(sql).unwrap();
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                [*version],
            )
            .unwrap();
        }
    }

    /// A retry after a crash mid-migration re-runs the same SQL from scratch
    /// (schema_version's "026" row is only inserted once execute_batch
    /// returns). The rename dance must tolerate being applied to a
    /// wallet_profiles it already produced, without losing existing rows.
    #[test]
    fn migration_026_can_be_reapplied_directly_without_losing_data() {
        let conn = Connection::open_in_memory().unwrap();
        apply_migrations_through_025(&conn);

        conn.execute_batch(MIGRATIONS[25].1).unwrap();

        conn.execute(
            "INSERT INTO wallet_profiles
                (id, label, kind, network, account_xpub, account_index, watch_only)
             VALUES (?, ?, 'ledger_hardware', 'mainnet', 'xpub-fake', 0, 1)",
            rusqlite::params!["profile-1", "Ledger Profile"],
        )
        .unwrap();

        // Re-run the exact same migration SQL a second time, as a retry
        // would after a crash left schema_version without a "026" row.
        conn.execute_batch(MIGRATIONS[25].1)
            .expect("migration 026 must be safe to re-run directly");

        let label: String = conn
            .query_row(
                "SELECT label FROM wallet_profiles WHERE id = ?",
                ["profile-1"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            label, "Ledger Profile",
            "re-running migration 026 must not lose existing wallet_profiles rows"
        );

        let kind_allows_ledger: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM wallet_profiles WHERE kind = 'ledger_hardware'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kind_allows_ledger, 1);
    }

    /// If the process is killed partway through migration 026 — after the
    /// scratch table's `DROP TABLE wallet_profiles` but before `COMMIT` —
    /// the wrapping transaction must roll the whole batch back, not leave
    /// the database without a `wallet_profiles` table.
    #[test]
    fn migration_026_aborted_transaction_leaves_wallet_profiles_intact() {
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!(
            "namehold_migration_026_abort_test_{pid}_{}.db",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&path);

        {
            let conn = crate::db::connection::open(&path).unwrap();
            apply_migrations_through_025(&conn);
            conn.execute(
                "INSERT INTO wallet_profiles
                    (id, label, kind, network, account_xpub, account_index, watch_only)
                 VALUES (?, ?, 'watch_only_xpub', 'mainnet', 'xpub-fake', 0, 1)",
                rusqlite::params!["profile-1", "Pre-existing Profile"],
            )
            .unwrap();

            // Replay migration 026's statements up to (but not including)
            // COMMIT, simulating a crash right after the DROP — the moment
            // the old, non-transactional SQL would have left no
            // wallet_profiles table at all.
            conn.execute_batch(
                "
                PRAGMA foreign_keys = OFF;
                BEGIN;
                DROP TABLE IF EXISTS wallet_profiles_new;
                CREATE TABLE wallet_profiles_new (
                    id TEXT PRIMARY KEY, label TEXT NOT NULL, kind TEXT NOT NULL,
                    network TEXT NOT NULL, account_xpub TEXT NOT NULL,
                    account_index INTEGER NOT NULL DEFAULT 0,
                    receive_depth INTEGER NOT NULL DEFAULT 0,
                    change_depth INTEGER NOT NULL DEFAULT 0,
                    receive_address TEXT, last_synced_height INTEGER,
                    last_synced_at TEXT, watch_only INTEGER NOT NULL DEFAULT 0,
                    last_explorer_sync_at TEXT,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                );
                INSERT INTO wallet_profiles_new
                    (id, label, kind, network, account_xpub, account_index, receive_depth,
                     change_depth, receive_address, last_synced_height, last_synced_at,
                     watch_only, last_explorer_sync_at, created_at, updated_at)
                SELECT
                    id, label, kind, network, account_xpub, account_index, receive_depth,
                    change_depth, receive_address, last_synced_height, last_synced_at,
                    watch_only, last_explorer_sync_at, created_at, updated_at
                FROM wallet_profiles;
                DROP TABLE wallet_profiles;
                ",
            )
            .unwrap();
            // Connection dropped here without COMMIT — the uncommitted
            // transaction (including the DROP) must be discarded, not
            // persisted, when the connection closes.
        }

        let conn = crate::db::connection::open(&path).unwrap();
        let label: String = conn
            .query_row(
                "SELECT label FROM wallet_profiles WHERE id = ?",
                ["profile-1"],
                |row| row.get(0),
            )
            .expect(
                "wallet_profiles must survive an aborted mid-migration transaction \
                 (an uncommitted DROP TABLE must roll back, not persist)",
            );
        assert_eq!(label, "Pre-existing Profile");

        let _ = std::fs::remove_file(&path);
    }
}
