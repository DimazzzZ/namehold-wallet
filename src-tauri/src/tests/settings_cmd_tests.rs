use crate::db;

#[test]
fn test_get_settings_returns_defaults() {
    let conn = crate::tests::command_helpers::create_test_db();
    let settings = db::queries::get_settings(&conn).unwrap();

    assert_eq!(settings["hsd_wallet_api_url"], "http://127.0.0.1:12039");
    assert_eq!(settings["hsd_node_api_url"], "http://127.0.0.1:12037");
    assert_eq!(settings["hsd_wallet_id"], "primary");
    assert_eq!(settings["hsd_network"], "mainnet");
    assert_eq!(settings["write_mode"], "false");
}

#[test]
fn test_set_setting_new_key() {
    let conn = crate::tests::command_helpers::create_test_db();
    db::queries::set_setting(&conn, "custom_key", "custom_value").unwrap();

    let settings = db::queries::get_settings(&conn).unwrap();
    assert_eq!(settings["custom_key"], "custom_value");
}

#[test]
fn test_set_setting_update_existing() {
    let conn = crate::tests::command_helpers::create_test_db();
    db::queries::set_setting(&conn, "hsd_network", "testnet").unwrap();

    let settings = db::queries::get_settings(&conn).unwrap();
    assert_eq!(settings["hsd_network"], "testnet");
}

#[test]
fn test_set_setting_empty_value() {
    let conn = crate::tests::command_helpers::create_test_db();
    db::queries::set_setting(&conn, "hsd_api_key", "").unwrap();

    let settings = db::queries::get_settings(&conn).unwrap();
    assert_eq!(settings["hsd_api_key"], "");
}

#[test]
fn test_wallet_snapshot_operations() {
    let conn = crate::tests::command_helpers::create_test_db();

    // Insert snapshots
    let id1 = db::queries::insert_wallet_snapshot(&conn, "primary", 1000000, Some("rs1q1"), 5, None).unwrap();
    let id2 = db::queries::insert_wallet_snapshot(&conn, "primary", 2000000, Some("rs1q1"), 10, None).unwrap();
    assert!(id2 > id1);

    // Get latest
    let latest = db::queries::get_latest_wallet_snapshot(&conn).unwrap().unwrap();
    assert_eq!(latest["balance"], 2000000);
    assert_eq!(latest["name_count"], 10);

    // Get list
    let snapshots = db::queries::get_wallet_snapshots(&conn, 5).unwrap();
    assert_eq!(snapshots.len(), 2);
}

#[test]
fn test_audit_log_operations() {
    let conn = crate::tests::command_helpers::create_test_db();

    conn.execute("INSERT INTO audit_log (action, detail) VALUES ('import_csv', '{\"count\":5}')", []).unwrap();
    conn.execute("INSERT INTO audit_log (action, detail) VALUES ('sync', '{\"matched\":3}')", []).unwrap();
    conn.execute("INSERT INTO audit_log (action, detail) VALUES ('import_csv', '{\"count\":10}')", []).unwrap();

    let entries = db::queries::get_recent_audit_log(&conn, 10).unwrap();
    assert_eq!(entries.len(), 3);

    let entries = db::queries::get_recent_audit_log(&conn, 2).unwrap();
    assert_eq!(entries.len(), 2);
}
