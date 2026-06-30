use crate::db;
use std::io::Write;

fn setup_db() -> rusqlite::Connection {
    crate::tests::command_helpers::create_test_db()
}

fn write_csv(content: &str, name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("namehold_csv_cmd_test_{}", name));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.csv");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    path.to_str().unwrap().to_string()
}

#[test]
fn test_csv_import_with_tags() {
    let conn = setup_db();
    let csv = "Name,Staked,Category,Tags,Notes\ncrypto,true,Premium,high_value;test,High value\nwallet,false,Finance,medium_value,Finance TLD\n";
    let path = write_csv(csv, "tags");

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

        // Convert comma-separated tags to JSON array
        let tags_json = row.tags.as_deref().map(|t| {
            let items: Vec<&str> = t.split(';').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
            serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string())
        });

        conn.execute(
            "INSERT INTO assets (tld, is_staked, status, category, tags, notes) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![tld, if is_staked { 1 } else { 0 }, status, row.category, tags_json, row.notes],
        )
        .unwrap();
        imported += 1;
    }

    assert_eq!(imported, 2);

    let assets = db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    assert_eq!(assets.len(), 2);

    let crypto = assets.iter().find(|a| a.tld == "crypto").unwrap();
    assert!(crypto.is_staked);
    assert_eq!(crypto.status.as_str(), "do_not_touch_staked");
    assert_eq!(crypto.tags, vec!["high_value", "test"]);
}

#[test]
fn test_csv_import_with_status_hint() {
    let conn = setup_db();
    let csv = "Name,Status\nexample,finalized_owned\n";
    let path = write_csv(csv, "status");

    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .trim(csv::Trim::All)
        .from_path(&path)
        .unwrap();

    for result in rdr.deserialize() {
        let row: crate::commands::csv::CsvRow = result.unwrap();
        let tld = row.tld.as_deref().unwrap_or("").trim_start_matches('.').trim().to_lowercase();
        let is_staked = row.is_staked.as_deref().map(crate::commands::csv::parse_boolish).unwrap_or(false);
        let status = crate::commands::csv::infer_status(is_staked, row.status.as_deref());

        conn.execute(
            "INSERT INTO assets (tld, is_staked, status) VALUES (?1, ?2, ?3)",
            rusqlite::params![tld, 0, status],
        )
        .unwrap();
    }

    let assets = db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    assert_eq!(assets[0].status.as_str(), "finalized_owned");
}

#[test]
fn test_csv_import_empty_name_skipped() {
    let conn = setup_db();
    let csv = "Name,Category\n,skipped\nvalid,cat\n";
    let path = write_csv(csv, "empty");

    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .trim(csv::Trim::All)
        .from_path(&path)
        .unwrap();

    let mut imported = 0;
    for result in rdr.deserialize() {
        let row: crate::commands::csv::CsvRow = result.unwrap();
        let tld = row.tld.as_deref().unwrap_or("").trim_start_matches('.').trim().to_lowercase();
        if tld.is_empty() { continue; }
        conn.execute("INSERT INTO assets (tld, status) VALUES (?1, 'not_started')", [tld]).unwrap();
        imported += 1;
    }

    assert_eq!(imported, 1);
    let assets = db::queries::list_assets(&conn, None, None, None, None, None).unwrap();
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0].tld, "valid");
}
