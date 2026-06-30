use crate::hsd::types::*;

#[test]
fn test_deserialize_hsd_balance() {
    let json = r#"{"confirmed": 1000000, "unconfirmed": 500000, "lockedUnconfirmed": 0, "lockedConfirmed": 0}"#;
    let balance: HsdBalance = serde_json::from_str(json).unwrap();
    assert_eq!(balance.confirmed, 1000000);
    assert_eq!(balance.unconfirmed, 500000);
    assert_eq!(balance.locked_unconfirmed, Some(0));
    assert_eq!(balance.locked_confirmed, Some(0));
}

#[test]
fn test_deserialize_hsd_name() {
    let json = r#"{
        "name": "crypto",
        "nameHash": "abc123",
        "state": "CLOSED",
        "height": 7203,
        "renewal": 14636,
        "owner": {"hash": "deadbeef", "index": 0},
        "value": 1000000,
        "highest": 2000000,
        "registered": true,
        "expired": false,
        "stats": {
            "renewalPeriodStart": 14636,
            "renewalPeriodEnd": 23276,
            "blocksUntilExpire": 6154,
            "daysUntilExpire": 21.37
        }
    }"#;
    let name: HsdName = serde_json::from_str(json).unwrap();
    assert_eq!(name.name, "crypto");
    assert_eq!(name.state.as_deref(), Some("CLOSED"));
    assert_eq!(name.height, Some(7203));
    assert_eq!(name.renewal, Some(14636));
    assert!(name.owner.is_some());
    assert_eq!(name.owner.as_ref().unwrap().hash, "deadbeef");
    assert!(name.stats.is_some());
    let stats = name.stats.as_ref().unwrap();
    assert_eq!(stats.days_until_expire, Some(21.37));
}

#[test]
fn test_deserialize_hsd_address() {
    let json = r#"{"address": "rs1qtest123", "name": "default", "account": 0, "branch": 0, "index": 4}"#;
    let addr: HsdAddress = serde_json::from_str(json).unwrap();
    assert_eq!(addr.address, "rs1qtest123");
    assert_eq!(addr.name.as_deref(), Some("default"));
}

#[test]
fn test_deserialize_hsd_wallet_info() {
    let json = r#"{"wid": 0, "id": "primary", "network": "main", "accountDepth": 5, "watchOnly": false}"#;
    let info: HsdWalletInfo = serde_json::from_str(json).unwrap();
    assert!(info.wid.is_some());
    assert_eq!(info.id.as_deref(), Some("primary"));
    assert_eq!(info.watch_only, Some(false));
}
