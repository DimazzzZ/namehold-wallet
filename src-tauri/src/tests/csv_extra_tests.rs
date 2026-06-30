use crate::commands::csv::{normalize_tld, parse_boolish, infer_status};

#[test]
fn test_normalize_tld_strips_dot_prefix() {
    assert_eq!(normalize_tld(".example"), "example");
    // trim_start_matches('.') only strips consecutive dots
    assert_eq!(normalize_tld("..example"), "example");
}

#[test]
fn test_normalize_tld_strips_whitespace() {
    assert_eq!(normalize_tld("  example  "), "example");
    assert_eq!(normalize_tld("\texample\t"), "example");
}

#[test]
fn test_normalize_tld_lowercases() {
    assert_eq!(normalize_tld("EXAMPLE"), "example");
    assert_eq!(normalize_tld("Crypto"), "crypto");
}

#[test]
fn test_normalize_tld_empty() {
    assert_eq!(normalize_tld(""), "");
}

#[test]
fn test_parse_boolish_various_true() {
    for val in &["true", "TRUE", "True", "1", "yes", "YES", "y", "Y", "staked", "STAKED"] {
        assert!(parse_boolish(val), "Expected true for '{}'", val);
    }
}

#[test]
fn test_parse_boolish_various_false() {
    for val in &["false", "FALSE", "0", "no", "NO", "", "n", "unstaked", "maybe"] {
        assert!(!parse_boolish(val), "Expected false for '{}'", val);
    }
}

#[test]
fn test_infer_status_staked_overrides_hint() {
    assert_eq!(infer_status(true, Some("finalized_owned")), "do_not_touch_staked");
    assert_eq!(infer_status(true, Some("not_started")), "do_not_touch_staked");
    assert_eq!(infer_status(true, None), "do_not_touch_staked");
}

#[test]
fn test_infer_status_unstaked_with_valid_hints() {
    assert_eq!(infer_status(false, Some("not_started")), "not_started");
    assert_eq!(infer_status(false, Some("namebase_transfer_requested")), "namebase_transfer_requested");
    assert_eq!(infer_status(false, Some("waiting_transfer_tx")), "waiting_transfer_tx");
    assert_eq!(infer_status(false, Some("transfer_seen_on_chain")), "transfer_seen_on_chain");
    assert_eq!(infer_status(false, Some("waiting_finalize")), "waiting_finalize");
    assert_eq!(infer_status(false, Some("finalized_owned")), "finalized_owned");
    assert_eq!(infer_status(false, Some("failed_or_stuck")), "failed_or_stuck");
}

#[test]
fn test_infer_status_unknown_hint_defaults_to_not_started() {
    assert_eq!(infer_status(false, Some("unknown_status")), "not_started");
    assert_eq!(infer_status(false, Some("random")), "not_started");
    assert_eq!(infer_status(false, None), "not_started");
}
