use crate::commands::csv;

#[test]
fn test_parse_boolish_true_variants() {
    assert!(csv::parse_boolish("true"));
    assert!(csv::parse_boolish("1"));
    assert!(csv::parse_boolish("yes"));
    assert!(csv::parse_boolish("y"));
    assert!(csv::parse_boolish("staked"));
    assert!(csv::parse_boolish("TRUE"));
    assert!(csv::parse_boolish("Yes"));
    assert!(csv::parse_boolish("Y"));
    assert!(csv::parse_boolish("Staked"));
}

#[test]
fn test_parse_boolish_false_variants() {
    assert!(!csv::parse_boolish("false"));
    assert!(!csv::parse_boolish("0"));
    assert!(!csv::parse_boolish("no"));
    assert!(!csv::parse_boolish("n"));
    assert!(!csv::parse_boolish(""));
    assert!(!csv::parse_boolish("anything_else"));
}

#[test]
fn test_normalize_tld_removes_dot_and_trims() {
    assert_eq!(csv::normalize_tld(".example"), "example");
    assert_eq!(csv::normalize_tld("  .example  "), "example");
    assert_eq!(csv::normalize_tld("EXAMPLE"), "example");
    assert_eq!(csv::normalize_tld("..example"), "example");
    assert_eq!(csv::normalize_tld("...example"), "example");
}

#[test]
fn test_normalize_tld_no_dot() {
    assert_eq!(csv::normalize_tld("example"), "example");
    assert_eq!(csv::normalize_tld("  example  "), "example");
}

#[test]
fn test_normalize_tld_empty() {
    assert_eq!(csv::normalize_tld(""), "");
    assert_eq!(csv::normalize_tld("."), "");
    assert_eq!(csv::normalize_tld("  "), "");
}

#[test]
fn test_infer_status_staked() {
    assert_eq!(csv::infer_status(true, None), "do_not_touch_staked");
    assert_eq!(csv::infer_status(true, Some("not_started")), "do_not_touch_staked");
}

#[test]
fn test_infer_status_no_hint() {
    assert_eq!(csv::infer_status(false, None), "not_started");
}

#[test]
fn test_infer_status_known_hints() {
    assert_eq!(csv::infer_status(false, Some("namebase_transfer_requested")), "namebase_transfer_requested");
    assert_eq!(csv::infer_status(false, Some("waiting_transfer_tx")), "waiting_transfer_tx");
    assert_eq!(csv::infer_status(false, Some("transfer_seen_on_chain")), "transfer_seen_on_chain");
    assert_eq!(csv::infer_status(false, Some("waiting_finalize")), "waiting_finalize");
    assert_eq!(csv::infer_status(false, Some("finalized_owned")), "finalized_owned");
    assert_eq!(csv::infer_status(false, Some("failed_or_stuck")), "failed_or_stuck");
    assert_eq!(csv::infer_status(false, Some("do_not_touch_staked")), "do_not_touch_staked");
}

#[test]
fn test_infer_status_unknown_hint() {
    assert_eq!(csv::infer_status(false, Some("unknown_status")), "not_started");
}

#[test]
fn test_infer_status_normalizes_spaces_and_dashes() {
    assert_eq!(csv::infer_status(false, Some("Namebase Transfer Requested")), "namebase_transfer_requested");
    assert_eq!(csv::infer_status(false, Some("waiting-transfer-tx")), "waiting_transfer_tx");
}
