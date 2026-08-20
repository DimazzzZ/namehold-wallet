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

#[cfg(test)]
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
///
/// Currently test-only: production callers inline the equivalent bytes in
/// [`build_parse_blob`](crate::providers::ledger::parse_mode::build_parse_blob).
/// Kept as a reference implementation for the wire format and to guard against
/// future drift; promote to `pub` when a real caller lands.
#[cfg(test)]
pub(crate) fn write_covenant(out: &mut Vec<u8>, covenant_type: u8, items: &[Vec<u8>]) {
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
        return Err(AppError::Protocol(format!(
            "name too long for Ledger marker: {} bytes",
            bytes.len()
        )));
    }
    if !bytes.iter().all(|b| b.is_ascii()) {
        return Err(AppError::Protocol(format!(
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

    /// Table-driven wire-form test: verify the exact serialized bytes for every
    /// supported covenant type against hsd's `output.write(buf)` format:
    /// `type(u8) | varint(itemCount) | (varint(len) | bytes)*`
    #[test]
    fn wire_form_all_covenant_types() {
        // Each entry: (covenant_type, items, expected_prefix_bytes)
        // We verify: type byte, item count, first item's varint+length.
        let name_hash = vec![0xAAu8; 32];
        let height = 0x0000_0064u32.to_le_bytes().to_vec(); // 4 bytes
        let name_bytes = b"example".to_vec();
        let nonce = vec![0xCCu8; 32];
        let blind = vec![0xBBu8; 32];
        let data = vec![0xFFu8; 20];
        let block_hash = vec![0xDDu8; 32];
        let version = vec![0x00u8];
        let addr = vec![0xEEu8; 20];
        let flags = vec![0x00u8];
        let claim_hash = vec![0x11u8; 32];
        let renewal_count = 0x0000_0001u32.to_le_bytes().to_vec();

        let cases: Vec<(u8, Vec<Vec<u8>>, usize)> = vec![
            // (type, items, expected_item_count)
            (
                COV_OPEN,
                vec![
                    name_hash.clone(),
                    height.clone(),
                    name_bytes.clone(),
                    nonce.clone(),
                ],
                4,
            ),
            (
                COV_BID,
                vec![
                    name_hash.clone(),
                    height.clone(),
                    name_bytes.clone(),
                    blind.clone(),
                ],
                4,
            ),
            (
                COV_REVEAL,
                vec![name_hash.clone(), height.clone(), nonce.clone()],
                3,
            ),
            (COV_REDEEM, vec![name_hash.clone(), height.clone()], 2),
            (
                COV_REGISTER,
                vec![name_hash.clone(), height.clone(), data.clone()],
                3,
            ),
            (
                COV_UPDATE,
                vec![name_hash.clone(), height.clone(), data.clone()],
                3,
            ),
            (
                COV_RENEW,
                vec![name_hash.clone(), height.clone(), block_hash.clone()],
                3,
            ),
            (
                COV_TRANSFER,
                vec![
                    name_hash.clone(),
                    height.clone(),
                    version.clone(),
                    addr.clone(),
                ],
                4,
            ),
            (
                COV_FINALIZE,
                vec![
                    name_hash.clone(),
                    height.clone(),
                    name_bytes.clone(),
                    flags.clone(),
                    claim_hash.clone(),
                    renewal_count.clone(),
                    block_hash.clone(),
                ],
                7,
            ),
            (COV_REVOKE, vec![name_hash.clone(), height.clone()], 2),
        ];

        for (cov_type, items, expected_count) in &cases {
            let mut out = Vec::new();
            write_covenant(&mut out, *cov_type, items);

            // Byte 0: covenant type
            assert_eq!(out[0], *cov_type, "type mismatch for covenant {cov_type}");
            // Byte 1: varint item count (all ≤ 252 so single byte)
            assert_eq!(
                out[1], *expected_count as u8,
                "item count mismatch for covenant {cov_type}"
            );
            // Byte 2: first item length varint (nameHash is always 32 → 0x20)
            assert_eq!(
                out[2], 32,
                "first item length mismatch for covenant {cov_type}"
            );
            // Bytes 3..35: first item body (nameHash)
            assert_eq!(
                &out[3..35],
                &name_hash[..],
                "first item body mismatch for covenant {cov_type}"
            );

            // Verify total serialized length matches the sum of all items + their varints.
            let expected_len: usize = 1 // type
                + 1 // varint(count)
                + items.iter().map(|item| {
                    let mut tmp = Vec::new();
                    write_var_bytes(&mut tmp, item);
                    tmp.len()
                }).sum::<usize>();
            assert_eq!(
                out.len(),
                expected_len,
                "total length mismatch for covenant {cov_type}"
            );
        }
    }
}
