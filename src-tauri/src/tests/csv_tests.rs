use crate::commands::csv::{normalize_tld, parse_boolish, infer_status};

#[test]
fn test_normalize_tld() {
    assert_eq!(normalize_tld("crypto"), "crypto");
    assert_eq!(normalize_tld(".crypto"), "crypto");
    assert_eq!(normalize_tld("  .CRYPTO  "), "crypto");
    assert_eq!(normalize_tld("  test  "), "test");
    assert_eq!(normalize_tld(".wallet."), "wallet.");
}

#[test]
fn test_parse_boolish() {
    assert!(parse_boolish("true"));
    assert!(parse_boolish("1"));
    assert!(parse_boolish("yes"));
    assert!(parse_boolish("y"));
    assert!(parse_boolish("TRUE"));
    assert!(parse_boolish("staked"));
    assert!(!parse_boolish("false"));
    assert!(!parse_boolish("0"));
    assert!(!parse_boolish("no"));
    assert!(!parse_boolish(""));
}

#[test]
fn test_infer_status_staked() {
    assert_eq!(infer_status(true, None), "do_not_touch_staked");
    assert_eq!(infer_status(true, Some("not_started")), "do_not_touch_staked");
}

#[test]
fn test_infer_status_unstaked_no_hint() {
    assert_eq!(infer_status(false, None), "not_started");
}

#[test]
fn test_infer_status_with_hint() {
    assert_eq!(infer_status(false, Some("finalized_owned")), "finalized_owned");
    assert_eq!(infer_status(false, Some("waiting_transfer_tx")), "waiting_transfer_tx");
    assert_eq!(infer_status(false, Some("namebase_transfer_requested")), "namebase_transfer_requested");
    assert_eq!(infer_status(false, Some("unknown_status")), "not_started");
}
