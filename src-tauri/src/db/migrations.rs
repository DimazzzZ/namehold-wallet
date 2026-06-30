use rusqlite::Connection;

const MIGRATIONS: &[(&str, &str)] = &[
    ("001", include_str!("../sql/001_initial.sql")),
    ("002", include_str!("../sql/002_hsd_prefix.sql")),
    ("003", include_str!("../sql/003_provider_modes.sql")),
    ("004", include_str!("../sql/004_wallet_addresses.sql")),
    ("005", include_str!("../sql/005_fix_hnsfans_api_url.sql")),
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
