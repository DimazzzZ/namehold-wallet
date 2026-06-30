use rusqlite::Connection;
use std::io::Write;

fn setup_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;").unwrap();
    let sql = include_str!("../sql/001_initial.sql");
    conn.execute_batch(sql).unwrap();
    conn
}

fn write_temp_csv(content: &str, name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("namehold_csv_test_{}", name));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.csv");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    path.to_str().unwrap().to_string()
}

#[test]
fn test_csv_import_basic() {
    let conn = setup_db();
    let csv = "Name,Staked,Category,Notes\ncrypto,true,Premium,High value\nwallet,false,Finance,Finance TLD\ntest,false,Test,Test note\n";
    let path = write_temp_csv(csv, "basic");

    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_path(&path)
        .unwrap();

    let mut imported = 0;
    for result in rdr.deserialize() {
        let row: crate::commands::csv::CsvRow = result.unwrap();
        let tld = row.tld.as_deref().unwrap_or("").trim_start_matches('.').trim().to_lowercase();
        if tld.is_empty() { continue; }

        let is_staked = row.is_staked.as_deref().map(crate::commands::csv::parse_boolish).unwrap_or(false);
        let status = crate::commands::csv::infer_status(is_staked, row.status.as_deref());

        conn.execute(
            "INSERT INTO assets (tld, is_staked, status, category, notes) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![tld, if is_staked { 1 } else { 0 }, status, row.category, row.notes],
        )
        .unwrap();
        imported += 1;
    }

    assert_eq!(imported, 3);

    let assets = crate::db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    assert_eq!(assets.len(), 3);

    let staked = assets.iter().find(|a| a.tld == "crypto").unwrap();
    assert!(staked.is_staked);
    assert_eq!(staked.status.as_str(), "do_not_touch_staked");

    let unstaked = assets.iter().find(|a| a.tld == "wallet").unwrap();
    assert!(!unstaked.is_staked);
    assert_eq!(unstaked.status.as_str(), "not_started");

    let _ = std::fs::remove_dir_all(std::env::temp_dir().join("namehold_csv_test_basic"));
}

#[test]
fn test_csv_import_duplicate_handling() {
    let conn = setup_db();
    let csv = "Name,Staked,Category\ncrypto,true,Premium\n";
    let path = write_temp_csv(csv, "dup");

    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .trim(csv::Trim::All)
        .from_path(&path)
        .unwrap();

    for result in rdr.deserialize() {
        let row: crate::commands::csv::CsvRow = result.unwrap();
        let tld = row.tld.as_deref().unwrap_or("").trim_start_matches('.').trim().to_lowercase();
        conn.execute(
            "INSERT INTO assets (tld, is_staked, status) VALUES (?1, ?2, ?3)
             ON CONFLICT(tld) DO UPDATE SET is_staked = excluded.is_staked, updated_at = datetime('now')",
            rusqlite::params![tld, 1, "do_not_touch_staked"],
        )
        .unwrap();
    }

    let mut rdr2 = csv::ReaderBuilder::new()
        .has_headers(true)
        .trim(csv::Trim::All)
        .from_path(&path)
        .unwrap();

    for result in rdr2.deserialize() {
        let row: crate::commands::csv::CsvRow = result.unwrap();
        let tld = row.tld.as_deref().unwrap_or("").trim_start_matches('.').trim().to_lowercase();
        conn.execute(
            "INSERT INTO assets (tld, is_staked, status) VALUES (?1, ?2, ?3)
             ON CONFLICT(tld) DO UPDATE SET is_staked = excluded.is_staked, updated_at = datetime('now')",
            rusqlite::params![tld, 0, "not_started"],
        )
        .unwrap();
    }

    let assets = crate::db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    assert_eq!(assets.len(), 1);
    assert!(!assets[0].is_staked);

    let _ = std::fs::remove_dir_all(std::env::temp_dir().join("namehold_csv_test_dup"));
}
