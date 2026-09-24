use rusqlite::Connection;

#[test]
fn test_migration_runs_on_empty_db() {
    let conn = Connection::open_in_memory().unwrap();
    let result = crate::db::migrations::run(&conn);
    assert!(result.is_ok());

    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert!(tables.contains(&"settings".to_string()));
    assert!(tables.contains(&"assets".to_string()));
    assert!(tables.contains(&"batches".to_string()));
    assert!(tables.contains(&"batch_assets".to_string()));
    assert!(tables.contains(&"wallet_snapshots".to_string()));
    assert!(tables.contains(&"audit_log".to_string()));
    assert!(tables.contains(&"schema_version".to_string()));
}

#[test]
fn test_migration_is_idempotent() {
    let conn = Connection::open_in_memory().unwrap();
    let r1 = crate::db::migrations::run(&conn);
    assert!(r1.is_ok());

    let r2 = crate::db::migrations::run(&conn);
    assert!(r2.is_ok());

    let version: String = conn
        .query_row(
            "SELECT version FROM schema_version WHERE version = '001'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, "001");
}

#[test]
fn test_schema_version_tracking() {
    let conn = Connection::open_in_memory().unwrap();
    crate::db::migrations::run(&conn).unwrap();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))
        .unwrap();
    // 001..009, 010 (drop legacy settings), 011 (re-add hsd data dir),
    // 012 (tx-draft confirmation tracking), 013 (owner address column),
    // 014 (bid reveal-end-height estimate), 015 (coin reservation, I3),
    // 016 (last_explorer_sync_at, Task 11 review Finding 2),
    // 017 (backfill bid_commitments.bid_txid/reveal_txid for pre-fix rows),
    // 018 (name_bid_index for chain_scan_cursor for the chain scanner),
    // 020 (namebase_history: imported Namebase account-history events).
    // 021 (sync_locks: cross-process sync coordination for background daemon).
    // 022 (watchlist: track names you don't own).
    // 023 (paid_swap_offers: seller-side tracking for atomic finalizeWithPayment).
    // 024 (watchlist_tags: comma-separated tags per watched name).
    // 025 (watched_name_states: daemon-written cache for watchlist columns + notifications).
    // 026 (ledger_hardware_profiles: add 'ledger_hardware' kind to wallet_profiles CHECK).
    // 027 (profile_settings: per-profile node config overrides, ADR-001),
    // 028 (chain_scan_network_scope: cursor + bid index keyed by network),
    // 029 (bid_auction_scope: bid index keyed by the auction's OPEN height),
    // 030 (bid_commitment_auction: commitments carry their auction too),
    // 031 (rescan_reveal_pairing: reveal values may sit on the wrong bid).
    // 032 (clear_seeded_mainnet_explorer: 009 seeded a mainnet URL that
    //      outranked the network default on every other network).
    assert_eq!(count, 32);
}

#[test]
fn test_default_settings_seeded() {
    let conn = Connection::open_in_memory().unwrap();
    crate::db::migrations::run(&conn).unwrap();

    // Non-custodial settings survive; the node RPC URL is seeded by 009.
    let node_url: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'node_rpc_url'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(node_url, "http://127.0.0.1:12037");

    // Legacy keys are removed by migration 010.
    for key in ["hsd_wallet_api_url", "connection_mode", "write_mode"] {
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM settings WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "legacy setting '{key}' should be deleted");
    }

    // `chain_source` and `allow_remote_broadcast` are seeded by migration 009
    // and were once deleted by migration 010; they've since been reintroduced
    // as live settings (remote-node support), so migration 010 no longer
    // deletes them and they keep their seeded defaults.
    for (key, expected) in [
        ("chain_source", "local_node"),
        ("allow_remote_broadcast", "false"),
    ] {
        let value: String = conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .unwrap_or_else(|_| panic!("'{key}' should still be seeded, not deleted"));
        assert_eq!(value, expected);
    }

    // The hsd data directory is re-added by migration 011 (010 drops it, 011
    // brings it back), so the app can start hsd against a custom prefix.
    let hsd_prefix: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'hsd_prefix'",
            [],
            |row| row.get(0),
        )
        .expect("hsd_prefix should exist after the full migration chain");
    assert_eq!(
        hsd_prefix, "",
        "hsd_prefix defaults to empty (= hsd's own ~/.hsd)"
    );
}

#[test]
fn test_tx_draft_confirmation_schema() {
    // Migration 012 adds the confirmation_height column and the 'confirmed' /
    // 'dropped' terminal statuses (recreating the table to change the CHECK).
    let conn = Connection::open_in_memory().unwrap();
    crate::db::migrations::run(&conn).unwrap();

    let cols: Vec<String> = conn
        .prepare("PRAGMA table_info(wallet_tx_drafts)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        cols.iter().any(|c| c == "confirmation_height"),
        "confirmation_height column should exist"
    );

    // Exercise the status CHECK, not the FK to wallet_profiles — drop FK
    // enforcement so an arbitrary wallet_profile_id is allowed. id == status
    // keeps PKs unique across the calls below.
    conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
    let insert = |status: &str| {
        conn.execute(
            "INSERT INTO wallet_tx_drafts
                (id, wallet_profile_id, action, unsigned_tx_hex, signing_inputs_json, summary_json, status)
             VALUES (?1, 'p', 'send_hns', '00', '{}', '{}', ?2)",
            rusqlite::params![status, status],
        )
    };
    assert!(insert("confirmed").is_ok(), "'confirmed' must be accepted");
    assert!(insert("dropped").is_ok(), "'dropped' must be accepted");
    assert!(
        insert("broadcasted").is_ok(),
        "existing statuses still accepted"
    );
    assert!(
        insert("bogus").is_err(),
        "an unknown status must be rejected by the CHECK"
    );
}

#[test]
fn test_connection_open() {
    let dir = std::env::temp_dir().join("namehold_test_db");
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("test.db");

    let result = crate::db::connection::open(&db_path);
    assert!(result.is_ok());

    let conn = result.unwrap();
    let journal: String = conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal, "wal");

    let fk: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert_eq!(fk, 1);

    let _ = std::fs::remove_dir_all(&dir);
}

// --- 032: the seeded mainnet explorer is cleared, a chosen one is kept ---

/// Migration 009 seeded this value; 032 removes exactly it.
const SEEDED_MAINNET_EXPLORER: &str = "https://e.hnsfans.com";

#[test]
fn migration_032_clears_the_explorer_url_009_seeded() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::migrations::run(&conn).unwrap();
    // Re-seed the way an installation that ran the original 009 looks, then
    // replay 032 over it: migrations run once, so this is the state such a
    // database is already in when the new build starts.
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES ('explorer_api_url', ?1)",
        [SEEDED_MAINNET_EXPLORER],
    )
    .unwrap();
    conn.execute_batch(include_str!("../sql/032_clear_seeded_mainnet_explorer.sql"))
        .unwrap();

    let remaining: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM settings WHERE key = 'explorer_api_url'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        remaining, 0,
        "the seeded mainnet URL must be gone so the network default applies"
    );
}

#[test]
fn migration_032_keeps_an_explorer_url_the_user_chose() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::migrations::run(&conn).unwrap();
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES ('explorer_api_url', ?1)",
        ["https://explorer.example.test"],
    )
    .unwrap();
    conn.execute_batch(include_str!("../sql/032_clear_seeded_mainnet_explorer.sql"))
        .unwrap();

    let value: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'explorer_api_url'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(value, "https://explorer.example.test");
}

// --- 030's backfill offsets must still describe the consensus they encode ---

/// Migration 030 backfills `name_start_height` from `reveal_end_height` using a
/// per-network offset spelled out as a literal: 2197, 469, 21.
///
/// The literals are deliberate. A migration is a one-shot transformation of
/// rows written under the rules of its own time, so deriving the offset from
/// live constants would let a later consensus change silently rewrite history
/// differently. But nothing then tells anyone editing `NameParams` that a
/// migration encodes the old values — which is what this test is for. If it
/// fails, 030 is not wrong; it is a record of what was true, and the failure
/// says the rules have moved since.
#[test]
fn migration_030_offsets_match_the_name_params_they_were_derived_from() {
    use crate::noncustodial::network::Network;

    // reveal_end = start + (tree_interval + 1) + bidding_period + reveal_period
    let offset = |n: Network| {
        let p = n.name_params();
        (p.tree_interval + 1 + p.bidding_period + p.reveal_period) as i64
    };

    assert_eq!(offset(Network::Main), 2197, "mainnet offset in 030");
    assert_eq!(offset(Network::Testnet), 469, "testnet offset in 030");
    assert_eq!(offset(Network::Regtest), 21, "regtest offset in 030");

    // The migration's CASE has no arm for simnet. That is safe only while the
    // schema refuses to store one.
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::migrations::run(&conn).unwrap();
    let rejected = conn.execute(
        "INSERT INTO wallet_profiles (id, label, kind, network, account_xpub)
         VALUES ('sim', 'Sim', 'watch_only_xpub', 'simnet', 'xpubSIM')",
        [],
    );
    assert!(
        rejected.is_err(),
        "030 assumes simnet cannot be stored; the CHECK must keep it out"
    );
}
