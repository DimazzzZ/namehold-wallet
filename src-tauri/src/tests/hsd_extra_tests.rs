use crate::hsd::types::*;

#[test]
fn test_hsd_balance_deserialization_full() {
    let json = r#"{"confirmed": 1000000, "unconfirmed": 500000, "lockedUnconfirmed": 100000, "lockedConfirmed": 200000}"#;
    let balance: HsdBalance = serde_json::from_str(json).unwrap();
    assert_eq!(balance.confirmed, 1000000);
    assert_eq!(balance.unconfirmed, 500000);
    assert_eq!(balance.locked_unconfirmed, Some(100000));
    assert_eq!(balance.locked_confirmed, Some(200000));
}

#[test]
fn test_hsd_balance_deserialization_minimal() {
    let json = r#"{"confirmed": 0, "unconfirmed": 0}"#;
    let balance: HsdBalance = serde_json::from_str(json).unwrap();
    assert_eq!(balance.confirmed, 0);
    assert_eq!(balance.unconfirmed, 0);
    assert_eq!(balance.locked_unconfirmed, None);
    assert_eq!(balance.locked_confirmed, None);
}

#[test]
fn test_hsd_name_deserialization_full() {
    let json = r#"{
        "name": "example",
        "nameHash": "abc123",
        "state": "CLOSED",
        "height": 1000,
        "renewal": 5000,
        "owner": {"hash": "deadbeef", "index": 0},
        "value": 500000,
        "highest": 1000000,
        "registered": true,
        "expired": false,
        "revoked": false,
        "transfer": null,
        "stats": {
            "renewalPeriodStart": 5000,
            "renewalPeriodEnd": 10000,
            "blocksUntilExpire": 5000,
            "daysUntilExpire": 34.7
        }
    }"#;
    let name: HsdName = serde_json::from_str(json).unwrap();
    assert_eq!(name.name, "example");
    assert_eq!(name.state.as_deref(), Some("CLOSED"));
    assert_eq!(name.height, Some(1000));
    assert_eq!(name.renewal, Some(5000));
    assert_eq!(name.registered, Some(true));
    assert_eq!(name.expired, Some(false));
    assert_eq!(name.revoked, Some(false));
    assert!(name.owner.is_some());
    assert!(name.stats.is_some());
    let stats = name.stats.unwrap();
    assert_eq!(stats.days_until_expire, Some(34.7));
    assert_eq!(stats.blocks_until_expire, Some(5000));
}

#[test]
fn test_hsd_name_deserialization_minimal() {
    let json = r#"{"name": "test"}"#;
    let name: HsdName = serde_json::from_str(json).unwrap();
    assert_eq!(name.name, "test");
    assert_eq!(name.state, None);
    assert_eq!(name.height, None);
    assert!(name.owner.is_none());
    assert!(name.stats.is_none());
}

#[test]
fn test_hsd_wallet_info_deserialization() {
    let json = r#"{
        "wid": 0,
        "id": "primary",
        "network": "main",
        "accountDepth": 5,
        "token": "abc123",
        "watchOnly": false
    }"#;
    let info: HsdWalletInfo = serde_json::from_str(json).unwrap();
    assert_eq!(info.id.as_deref(), Some("primary"));
    assert_eq!(info.network.as_deref(), Some("main"));
    assert_eq!(info.account_depth, Some(5));
    assert_eq!(info.watch_only, Some(false));
}

#[test]
fn test_hsd_address_deserialization() {
    let json = r#"{
        "name": "default",
        "account": 0,
        "branch": 0,
        "index": 5,
        "publicKey": "03abc123",
        "address": "hs1qtest123"
    }"#;
    let addr: HsdAddress = serde_json::from_str(json).unwrap();
    assert_eq!(addr.address, "hs1qtest123");
    assert_eq!(addr.name.as_deref(), Some("default"));
    assert_eq!(addr.index, Some(5));
}

#[test]
fn test_hsd_owner_deserialization() {
    let json = r#"{"hash": "deadbeef01234567890", "index": 42}"#;
    let owner: HsdOwner = serde_json::from_str(json).unwrap();
    assert_eq!(owner.hash, "deadbeef01234567890");
    assert_eq!(owner.index, 42);
}

#[test]
fn test_hsd_name_stats_deserialization() {
    let json = r#"{
        "renewalPeriodStart": 1000,
        "renewalPeriodEnd": 5000,
        "blocksUntilExpire": 4000,
        "daysUntilExpire": 27.8
    }"#;
    let stats: HsdNameStats = serde_json::from_str(json).unwrap();
    assert_eq!(stats.renewal_period_start, Some(1000));
    assert_eq!(stats.renewal_period_end, Some(5000));
    assert_eq!(stats.blocks_until_expire, Some(4000));
    assert_eq!(stats.days_until_expire, Some(27.8));
}

#[test]
fn test_hsd_node_info_deserialization() {
    let json = r#"{"version": "7.0.0", "network": "main", "chain": {"height": 100000}}"#;
    let info: HsdNodeInfo = serde_json::from_str(json).unwrap();
    assert_eq!(info.version.as_deref(), Some("7.0.0"));
    assert_eq!(info.network.as_deref(), Some("main"));
}

#[test]
fn test_hsd_name_with_all_none_fields() {
    let json = r#"{"name": "empty"}"#;
    let name: HsdName = serde_json::from_str(json).unwrap();
    assert_eq!(name.name, "empty");
    assert!(name.name_hash.is_none());
    assert!(name.state.is_none());
    assert!(name.height.is_none());
    assert!(name.renewal.is_none());
    assert!(name.owner.is_none());
    assert!(name.value.is_none());
    assert!(name.highest.is_none());
    assert!(name.registered.is_none());
    assert!(name.expired.is_none());
    assert!(name.stats.is_none());
    assert!(name.transfer.is_none());
    assert!(name.revoked.is_none());
}
