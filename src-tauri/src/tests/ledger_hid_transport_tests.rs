//! Tests for `providers::ledger::hid_transport` helpers that need no device.
//!
//! `status_word_message` turns an APDU status word into the sentence the user
//! reads when a Ledger refuses something. It is the only part of this module
//! that runs without hardware.

use crate::providers::ledger::hid_transport::status_word_message;

#[test]
fn a_known_status_word_explains_itself() {
    let msg = status_word_message(0x6985);
    assert!(
        msg.contains("0x6985"),
        "the raw word stays in the text: {msg}"
    );
    assert!(
        msg.contains("user rejected on device"),
        "the common case must not read as an unexplained code: {msg}"
    );
}

#[test]
fn every_mapped_status_word_adds_a_hint() {
    for (sw, needle) in [
        (0x6985u16, "user rejected"),
        (0x6d00, "instruction not supported"),
        (0x6e00, "class not supported"),
        (0x6a80, "invalid data"),
        (0x5515, "device locked"),
    ] {
        let msg = status_word_message(sw);
        assert!(msg.contains(needle), "0x{sw:04x} lost its hint: {msg}");
    }
}

#[test]
fn an_unknown_status_word_is_reported_without_inventing_a_reason() {
    let msg = status_word_message(0x1234);
    assert_eq!(msg, "APDU failed with status 0x1234");
}
