//! Parse-mode blob builder for the Handshake Ledger app.
//!
//! The "parse" phase streams the entire transaction to the device so it can
//! compute the sighash preimage commitments (hashPrevouts, hashSequences,
//! hashOutputs) and display covenant details for user review. After parse
//! completes, the client sends per-input "sign" requests.
//!
//! # Wire layout (hsd-ledger `lib/ledger/client.js → parseTX()`)
//!
//! ```text
//! u32LE version
//! u32LE locktime
//! u8    inputsCount
//! u8    outputsCount
//! u8    changeFlag (0x00 or 0x01)
//! [if changeFlag] LedgerChange { u8 index, u8 addrVersion, u8 depth, u32BE path[] }
//!
//! for each input:
//!     32 bytes prevout.hash
//!     u32LE    prevout.index
//!     u32LE    sequence
//!     u64LE    coinValue
//!
//! for each output:
//!     u64LE value
//!     u8    addressVersion
//!     u8    addressHashLen
//!     bytes addressHash
//!     u8    covenant.type
//!     varint covenant.items.length
//!     for each item: varbytes(item)
//!     [if name-bearing covenant] u8 nameLen | name[ascii]
//! ```
//!
//! The blob is then split into ≤255-byte APDUs (see [`build_parse_apdus`]).

use crate::error::AppError;
use crate::noncustodial::actions::DraftPlan;
use crate::noncustodial::network::Network;
use crate::noncustodial::tx::output_address_from_string;

use super::apdu::{
    write_var_bytes, write_varint, ApduCommand, CLA_GENERAL, INS_GET_INPUT_SIGNATURE,
    MAX_APDU_DATA,
};
use super::covenant_serializer::{is_supported, requires_name_marker, write_name_marker};

/// Metadata about the change output, so the device can skip prompting for it.
#[derive(Debug, Clone)]
pub struct ChangeInfo {
    /// Zero-based index of the change output in `plan.outputs`.
    pub output_index: u8,
    /// Address witness version (0 for p2wpkh).
    pub address_version: u8,
    /// Full BIP44 derivation path of the change address (5 levels).
    pub path: Vec<u32>,
}

/// The human-readable name associated with a covenant output. Required for the
/// 7 name-bearing types (REVEAL, REDEEM, REGISTER, UPDATE, RENEW, TRANSFER,
/// REVOKE). The caller must supply these for every output whose covenant type
/// requires a name marker.
#[derive(Debug, Clone)]
pub struct OutputName {
    /// Zero-based output index.
    pub output_index: usize,
    /// ASCII name (e.g. "example").
    pub name: String,
}

/// Build the parse-mode APDU sequence for a given draft plan.
///
/// Returns a list of [`ApduCommand`]s ready to send. The first has P1 bit 0x01
/// set (first-packet flag); subsequent ones have P1 = `net_flag` only.
///
/// # Errors
///
/// Returns `AppError::Device` if:
/// - An unsupported covenant type is encountered.
/// - A name-bearing output lacks a corresponding entry in `names`.
/// - An address string fails to decode.
pub fn build_parse_apdus(
    plan: &DraftPlan,
    network: Network,
    change: Option<&ChangeInfo>,
    names: &[OutputName],
    net_flag: u8,
) -> Result<Vec<ApduCommand>, AppError> {
    let blob = build_parse_blob(plan, network, change, names)?;
    Ok(split_into_apdus(&blob, net_flag))
}

/// Build the raw parse-mode blob (before APDU splitting).
pub fn build_parse_blob(
    plan: &DraftPlan,
    network: Network,
    change: Option<&ChangeInfo>,
    names: &[OutputName],
) -> Result<Vec<u8>, AppError> {
    let mut buf = Vec::with_capacity(512);

    // Header
    buf.extend_from_slice(&plan.version.to_le_bytes());
    buf.extend_from_slice(&plan.locktime.to_le_bytes());
    buf.push(plan.inputs.len() as u8);
    buf.push(plan.outputs.len() as u8);

    // Change flag + optional LedgerChange
    match change {
        Some(c) => {
            buf.push(0x01);
            buf.push(c.output_index);
            buf.push(c.address_version);
            buf.push(c.path.len() as u8);
            for &idx in &c.path {
                buf.extend_from_slice(&idx.to_be_bytes());
            }
        }
        None => buf.push(0x00),
    }

    // Inputs
    for inp in &plan.inputs {
        let hash = hex::decode(&inp.txid)
            .map_err(|e| AppError::Device(format!("bad input txid hex: {e}")))?;
        if hash.len() != 32 {
            return Err(AppError::Device(format!(
                "input txid must be 32 bytes, got {}",
                hash.len()
            )));
        }
        buf.extend_from_slice(&hash);
        buf.extend_from_slice(&inp.vout.to_le_bytes());
        buf.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // sequence (always final)
        buf.extend_from_slice(&inp.value.to_le_bytes());
    }

    // Outputs
    for (i, out) in plan.outputs.iter().enumerate() {
        if !is_supported(out.covenant_type) {
            return Err(AppError::Device(format!(
                "unsupported covenant type {} on output #{i}",
                out.covenant_type
            )));
        }

        // value
        buf.extend_from_slice(&out.value.to_le_bytes());

        // address (version | hashLen | hash)
        let addr = output_address_from_string(network, &out.address)?;
        buf.push(addr.version);
        buf.push(addr.hash.len() as u8);
        buf.extend_from_slice(&addr.hash);

        // covenant (type | varint(count) | varbytes(item)*)
        let items = decode_covenant_items(&out.covenant_items_hex)?;
        buf.push(out.covenant_type);
        write_varint(&mut buf, items.len() as u64);
        for item in &items {
            write_var_bytes(&mut buf, item);
        }

        // LedgerCovenant name marker (for the 7 name-bearing types)
        if requires_name_marker(out.covenant_type) {
            let name = names
                .iter()
                .find(|n| n.output_index == i)
                .ok_or_else(|| {
                    AppError::Device(format!(
                        "output #{i} has covenant type {} but no name was provided",
                        out.covenant_type
                    ))
                })?;
            write_name_marker(&mut buf, &name.name)?;
        }
    }

    Ok(buf)
}

/// Decode hex-encoded covenant items back to raw bytes.
fn decode_covenant_items(hex_items: &[String]) -> Result<Vec<Vec<u8>>, AppError> {
    hex_items
        .iter()
        .map(|h| hex::decode(h).map_err(|e| AppError::Device(format!("bad covenant item hex: {e}"))))
        .collect()
}

/// Split a blob into ≤255-byte APDUs with the correct P1/P2 flags for parse
/// mode.
///
/// * P2 = 0x00 (parse mode).
/// * First APDU: P1 = `net_flag | 0x01`.
/// * Subsequent: P1 = `net_flag`.
fn split_into_apdus(blob: &[u8], net_flag: u8) -> Vec<ApduCommand> {
    let chunks: Vec<&[u8]> = blob.chunks(MAX_APDU_DATA).collect();
    let mut cmds = Vec::with_capacity(chunks.len());
    for (i, chunk) in chunks.iter().enumerate() {
        cmds.push(ApduCommand {
            cla: CLA_GENERAL,
            ins: INS_GET_INPUT_SIGNATURE,
            p1: if i == 0 { net_flag | 0x01 } else { net_flag },
            p2: 0x00,
            data: chunk.to_vec(),
        });
    }
    cmds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noncustodial::actions::{DraftPlan, PlanInput, PlanOutput};
    use crate::noncustodial::network::Network;
    use crate::noncustodial::sync::COV_NONE;

    fn simple_send_plan() -> DraftPlan {
        // 1 input, 1 output (plain send, covenant NONE)
        DraftPlan {
            version: 0,
            locktime: 0,
            account: 0,
            network: "main".into(),
            inputs: vec![PlanInput {
                txid: "aa".repeat(32),
                vout: 0,
                value: 100_000_000,
                branch: 0,
                child_index: 0,
                sighash_type: 1,
            }],
            outputs: vec![PlanOutput {
                value: 99_000_000,
                // Valid Handshake mainnet address
                address: "hs1qd42hrldu5yqee58se4uj6xctm7nk28r70e84vx".into(),
                covenant_type: COV_NONE,
                covenant_items_hex: vec![],
            }],
            change_output_index: None,
        }
    }

    #[test]
    fn parse_blob_header_layout() {
        let plan = simple_send_plan();
        let blob = build_parse_blob(&plan, Network::Main, None, &[]).unwrap();

        // version u32LE = 0
        assert_eq!(&blob[0..4], &[0, 0, 0, 0]);
        // locktime u32LE = 0
        assert_eq!(&blob[4..8], &[0, 0, 0, 0]);
        // inputsCount = 1
        assert_eq!(blob[8], 1);
        // outputsCount = 1
        assert_eq!(blob[9], 1);
        // changeFlag = 0
        assert_eq!(blob[10], 0);
    }

    #[test]
    fn parse_blob_input_section() {
        let plan = simple_send_plan();
        let blob = build_parse_blob(&plan, Network::Main, None, &[]).unwrap();

        // After header (11 bytes): input starts at offset 11
        let input_start = 11;
        // prevout.hash = 0xAA * 32
        assert_eq!(&blob[input_start..input_start + 32], &[0xAA; 32]);
        // prevout.index = 0 (u32LE)
        assert_eq!(
            &blob[input_start + 32..input_start + 36],
            &[0, 0, 0, 0]
        );
        // sequence = 0xFFFFFFFF
        assert_eq!(
            &blob[input_start + 36..input_start + 40],
            &[0xFF, 0xFF, 0xFF, 0xFF]
        );
        // value = 100_000_000 (u64LE)
        assert_eq!(
            &blob[input_start + 40..input_start + 48],
            &100_000_000u64.to_le_bytes()
        );
    }

    #[test]
    fn parse_blob_output_covenant_none() {
        let plan = simple_send_plan();
        let blob = build_parse_blob(&plan, Network::Main, None, &[]).unwrap();

        // After header(11) + input(48) = 59: output starts
        let out_start = 59;
        // value = 99_000_000 (u64LE)
        assert_eq!(
            &blob[out_start..out_start + 8],
            &99_000_000u64.to_le_bytes()
        );
        // address: version=0, hashLen=20, hash=0x00*20
        assert_eq!(blob[out_start + 8], 0); // version
        assert_eq!(blob[out_start + 9], 20); // hashLen
        // covenant: type=0 (NONE), count=0
        let cov_start = out_start + 10 + 20;
        assert_eq!(blob[cov_start], 0); // type NONE
        assert_eq!(blob[cov_start + 1], 0); // 0 items
    }

    #[test]
    fn split_respects_max_apdu_data() {
        let blob = vec![0x42u8; 600]; // > 255
        let apdus = split_into_apdus(&blob, 0x00);
        assert_eq!(apdus.len(), 3);
        assert_eq!(apdus[0].data.len(), 255);
        assert_eq!(apdus[1].data.len(), 255);
        assert_eq!(apdus[2].data.len(), 90);
        // First APDU has P1 bit 0x01
        assert_eq!(apdus[0].p1, 0x01);
        assert_eq!(apdus[1].p1, 0x00);
        assert_eq!(apdus[2].p1, 0x00);
    }

    #[test]
    fn change_info_serialized_correctly() {
        let plan = simple_send_plan();
        let change = ChangeInfo {
            output_index: 1,
            address_version: 0,
            path: vec![
                44 + 0x8000_0000,
                5353 + 0x8000_0000,
                0x8000_0000,
                1,
                7,
            ],
        };
        let blob = build_parse_blob(&plan, Network::Main, Some(&change), &[]).unwrap();

        // changeFlag at offset 10
        assert_eq!(blob[10], 0x01);
        // outputIndex
        assert_eq!(blob[11], 1);
        // addressVersion
        assert_eq!(blob[12], 0);
        // pathDepth
        assert_eq!(blob[13], 5);
        // first path element: 44' = 0x8000002C, big-endian
        assert_eq!(&blob[14..18], &[0x80, 0x00, 0x00, 0x2C]);
    }
}
