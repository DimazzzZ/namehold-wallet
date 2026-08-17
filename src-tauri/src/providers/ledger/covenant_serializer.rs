//! Covenant handling for the Ledger parse-mode blob.
//!
//! # The actual contract (verified against hsd-ledger `lib/ledger/client.js`)
//!
//! Contrary to an early assumption, the client does **not** reorder covenant
//! items. In `parseTX()` it calls hsd's own `output.write(buf)`, which emits
//! the covenant in standard hsd wire form:
//!
//! ```text
//! covenant.type (u8) | varint(itemCount) | (varint(len) | bytes)*   // per item
//! ```
//!
//! This is byte-for-byte what Namehold already stores in
//! [`PlanOutput::covenant_items_hex`](crate::noncustodial::actions::PlanOutput).
//! So the only covenant-specific work is: for the seven **name-bearing**
//! covenant types the client appends a `LedgerCovenant` marker immediately
//! after the output — a plain `u8 nameLen | name[ascii]`. The device uses it to
//! show the human-readable name on-screen while reviewing the covenant.
//!
//! Marker required (name appended): REVEAL, REDEEM, REGISTER, UPDATE, RENEW,
//! TRANSFER, REVOKE.
//! No marker: NONE, OPEN, BID, FINALIZE (the app derives/echoes the name from
//! the covenant items directly, or the output has no name to show).

use crate::error::AppError;
use crate::noncustodial::sync::{
    COV_BID, COV_FINALIZE, COV_NONE, COV_OPEN, COV_REDEEM, COV_REGISTER, COV_RENEW, COV_REVEAL,
    COV_REVOKE, COV_TRANSFER, COV_UPDATE,
};

use super::apdu::{write_var_bytes, write_varint};

/// Whether a covenant type requires an appended `LedgerCovenant` name marker in
/// parse mode.
pub fn requires_name_marker(covenant_type: u8) -> bool {
    matches!(
        covenant_type,
        COV_REVEAL | COV_REDEEM | COV_REGISTER | COV_RENEW | COV_TRANSFER | COV_REVOKE | COV_UPDATE
    )
}

/// Covenant types the Handshake Ledger app accepts. CLAIM (1) is intentionally
/// excluded — Namehold never builds airdrop claims, and the device app does not
/// review them.
pub fn is_supported(covenant_type: u8) -> bool {
    matches!(
        covenant_type,
        COV_NONE
            | COV_OPEN
            | COV_BID
            | COV_REVEAL
            | COV_REDEEM
            | COV_REGISTER
            | COV_UPDATE
            | COV_RENEW
            | COV_TRANSFER
            | COV_FINALIZE
            | COV_REVOKE
    )
}

/// Serialize a covenant into hsd wire form: `type | varint(count) |
/// varbytes(item)*`. `items` are the raw (already-decoded) covenant items in
/// hsd order, exactly as stored in a plan's `covenant_items_hex`.
pub fn write_covenant(out: &mut Vec<u8>, covenant_type: u8, items: &[Vec<u8>]) {
    out.push(covenant_type);
    write_varint(out, items.len() as u64);
    for item in items {
        write_var_bytes(out, item);
    }
}

/// Append the `LedgerCovenant` name marker (`u8 nameLen | name[ascii]`) for a
/// name-bearing covenant. `name` must be ASCII and <= 255 bytes (Handshake
/// names are; `Rules.verifyString` guarantees ASCII printable, max 63 bytes).
pub fn write_name_marker(out: &mut Vec<u8>, name: &str) -> Result<(), AppError> {
    let bytes = name.as_bytes();
    if bytes.len() > 255 {
        return Err(AppError::Device(format!(
            "name too long for Ledger marker: {} bytes",
            bytes.len()
        )));
    }
    if !bytes.iter().all(|b| b.is_ascii()) {
        return Err(AppError::Device(format!(
            "name is not ASCII, cannot render on Ledger: {name:?}"
        )));
    }
    out.push(bytes.len() as u8);
    out.extend_from_slice(bytes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_marker_only_for_the_seven_types() {
        assert!(requires_name_marker(COV_REVEAL));
        assert!(requires_name_marker(COV_REDEEM));
        assert!(requires_name_marker(COV_REGISTER));
        assert!(requires_name_marker(COV_UPDATE));
        assert!(requires_name_marker(COV_RENEW));
        assert!(requires_name_marker(COV_TRANSFER));
        assert!(requires_name_marker(COV_REVOKE));

        assert!(!requires_name_marker(COV_NONE));
        assert!(!requires_name_marker(COV_OPEN));
        assert!(!requires_name_marker(COV_BID));
        assert!(!requires_name_marker(COV_FINALIZE));
    }

    #[test]
    fn covenant_wire_form_matches_hsd() {
        // BID: [nameHash(32), u32le(start), name, blind(32)]
        let name_hash = vec![0xAAu8; 32];
        let start = 0x0000_1234u32.to_le_bytes().to_vec();
        let name = b"example".to_vec();
        let blind = vec![0xBBu8; 32];
        let items = vec![name_hash, start, name, blind];

        let mut out = Vec::new();
        write_covenant(&mut out, COV_BID, &items);

        // type
        assert_eq!(out[0], COV_BID);
        // count = 4 (varint < 0xFD → single byte)
        assert_eq!(out[1], 4);
        // first item: varint(32) then 32 bytes
        assert_eq!(out[2], 32);
        assert_eq!(&out[3..35], &[0xAA; 32]);
        // second item: varint(4) then 4 bytes LE
        assert_eq!(out[35], 4);
        assert_eq!(&out[36..40], &[0x34, 0x12, 0x00, 0x00]);
    }

    #[test]
    fn name_marker_layout() {
        let mut out = Vec::new();
        write_name_marker(&mut out, "example").unwrap();
        assert_eq!(out[0], 7);
        assert_eq!(&out[1..], b"example");
    }

    #[test]
    fn name_marker_rejects_non_ascii() {
        let mut out = Vec::new();
        assert!(write_name_marker(&mut out, "exãmple").is_err());
    }

    #[test]
    fn claim_is_unsupported() {
        assert!(!is_supported(1)); // COV_CLAIM
        assert!(is_supported(COV_OPEN));
    }
}
