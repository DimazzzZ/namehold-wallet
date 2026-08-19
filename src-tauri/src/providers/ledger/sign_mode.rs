//! Sign-mode APDU builder for the Handshake Ledger app.
//!
//! After the parse phase completes, the client sends one sign request per input.
//! Each request carries the signing key's BIP32 path, the sighash type, the
//! input being signed (prevout + value + sequence), and the script code (the
//! p2wpkh redeem script for standard Namehold inputs).
//!
//! # Wire layout (hsd-ledger `lib/ledger/client.js → getInputSignature()`)
//!
//! ```text
//! u8      pathDepth
//! u32BE   path[0..depth-1]
//! u32LE   sighashType
//! 32      prevout.hash
//! u32LE   prevout.index
//! u64LE   inputValue
//! u32LE   sequence
//! varint  scriptLen
//! bytes   scriptCode
//! u8      0x00   (no single-output commitment; SIGHASH_ALL only)
//! ```
//!
//! The device returns a 65-byte signature: `r(32) || s(32) || sighashType(u8)`.
//! This is a raw compact signature (not DER) that can be directly applied as a
//! witness item.

use crate::error::AppError;
use crate::noncustodial::address::pubkey_to_hash160;
use crate::noncustodial::hd::bip44_path;
use crate::noncustodial::network::Network;
use crate::noncustodial::tx::p2wpkh_script_code;

use super::apdu::{
    chunk_apdus, encode_path, write_var_bytes, ApduCommand, CLA_GENERAL, INS_GET_INPUT_SIGNATURE,
};

/// Signing parameters for a single input (replaces the 10-parameter signature).
#[derive(Debug, Clone)]
pub struct SignInput {
    pub network: Network,
    pub account: u32,
    pub branch: u32,
    pub child_index: u32,
    pub sighash_type: u32,
    pub prevout_hash: [u8; 32],
    pub prevout_index: u32,
    pub value: u64,
    pub sequence: u32,
    pub pubkey: [u8; 33],
}

/// Build the sign-mode APDU sequence for one input.
///
/// Returns the APDU sequence (may be >1 if the blob exceeds 255 bytes, though
/// for standard p2wpkh inputs it's always a single APDU).
pub fn build_sign_apdus(input: &SignInput) -> Result<Vec<ApduCommand>, AppError> {
    let blob = build_sign_blob(input)?;
    Ok(chunk_apdus(
        &blob,
        CLA_GENERAL,
        INS_GET_INPUT_SIGNATURE,
        0x01,
        0x00,
        0x01, // P2 = sign mode
    ))
}

/// Build the raw sign-mode blob.
pub fn build_sign_blob(input: &SignInput) -> Result<Vec<u8>, AppError> {
    let path = bip44_path(input.network, input.account, input.branch, input.child_index);
    let mut buf = Vec::with_capacity(128);

    // Path (depth prefix + BE indices)
    let path_bytes = encode_path(&path)?;
    buf.extend_from_slice(&path_bytes);

    // Sighash type (u32LE)
    buf.extend_from_slice(&input.sighash_type.to_le_bytes());

    // Prevout
    buf.extend_from_slice(&input.prevout_hash);
    buf.extend_from_slice(&input.prevout_index.to_le_bytes());

    // Value + sequence
    buf.extend_from_slice(&input.value.to_le_bytes());
    buf.extend_from_slice(&input.sequence.to_le_bytes());

    // Script code: p2wpkh → hsd's `Script.fromPubkeyhash(hash)`
    // (`OP_DUP OP_BLAKE160 <push20> <hash160> OP_EQUALVERIFY OP_CHECKSIG`).
    // Shared with the non-Ledger signing path so the device signs the same
    // digest consensus verifies — see `noncustodial::tx::p2wpkh_script_code`.
    let hash160 = pubkey_to_hash160(&input.pubkey);
    let script = p2wpkh_script_code(&hash160);
    write_var_bytes(&mut buf, &script);

    // No single-output commitment (SIGHASH_ALL only in Namehold).
    buf.push(0x00);

    Ok(buf)
}

/// Parse the 65-byte signature returned by the device in sign mode.
///
/// Layout: `r(32) || s(32) || sighashType(u8)`. Returns the full 65-byte blob
/// (the caller applies it as a witness item — Handshake uses raw compact sigs,
/// not DER).
pub fn parse_signature(body: &[u8]) -> Result<[u8; 65], AppError> {
    if body.len() != 65 {
        return Err(AppError::Device(format!(
            "expected 65-byte signature, got {} bytes",
            body.len()
        )));
    }
    let mut sig = [0u8; 65];
    sig.copy_from_slice(body);
    Ok(sig)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noncustodial::network::Network;

    #[test]
    fn sign_blob_layout() {
        // Use a dummy pubkey (compressed, starts with 0x02)
        let mut pubkey = [0x02u8; 33];
        pubkey[1] = 0xAA;
        let prevout = [0xBB; 32];

        let blob = build_sign_blob(&SignInput {
            network: Network::Main,
            account: 0,
            branch: 0,
            child_index: 7,
            sighash_type: 1, // SIGHASH_ALL
            prevout_hash: prevout,
            prevout_index: 0,
            value: 50_000_000,
            sequence: 0xFFFF_FFFF,
            pubkey,
        })
        .unwrap();

        // Path: depth=5, then 5 * u32BE
        assert_eq!(blob[0], 5);
        // 44' = 0x8000002C
        assert_eq!(&blob[1..5], &[0x80, 0x00, 0x00, 0x2C]);
        // 5353' = 0x800014E9
        assert_eq!(&blob[5..9], &[0x80, 0x00, 0x14, 0xE9]);
        // 0' = 0x80000000
        assert_eq!(&blob[9..13], &[0x80, 0x00, 0x00, 0x00]);
        // branch=0
        assert_eq!(&blob[13..17], &[0x00, 0x00, 0x00, 0x00]);
        // index=7
        assert_eq!(&blob[17..21], &[0x00, 0x00, 0x00, 0x07]);

        // sighash type = 1 (u32LE)
        assert_eq!(&blob[21..25], &[0x01, 0x00, 0x00, 0x00]);

        // prevout hash
        assert_eq!(&blob[25..57], &[0xBB; 32]);
        // prevout index = 0
        assert_eq!(&blob[57..61], &[0x00, 0x00, 0x00, 0x00]);
        // value = 50_000_000
        assert_eq!(&blob[61..69], &50_000_000u64.to_le_bytes());
        // sequence = 0xFFFFFFFF
        assert_eq!(&blob[69..73], &[0xFF, 0xFF, 0xFF, 0xFF]);

        // script code: varint(25) then 25 bytes starting with OP_DUP
        assert_eq!(blob[73], 25); // varint for 25
        assert_eq!(blob[74], 0x76); // OP_DUP
        assert_eq!(blob[75], 0xc0); // OP_BLAKE160 (not Bitcoin's OP_HASH160)
        assert_eq!(blob[76], 0x14); // push 20

        // Trailing: 0x00 (no single-output)
        let last = blob.len() - 1;
        assert_eq!(blob[last], 0x00);
    }

    #[test]
    fn sign_apdus_p1_p2_flags() {
        let blob = vec![0x42u8; 300]; // > 255, forces 2 APDUs
        let apdus = chunk_apdus(&blob, CLA_GENERAL, INS_GET_INPUT_SIGNATURE, 0x01, 0x00, 0x01);
        assert_eq!(apdus.len(), 2);
        assert_eq!(apdus[0].p1, 0x01);
        assert_eq!(apdus[0].p2, 0x01);
        assert_eq!(apdus[1].p1, 0x00);
        assert_eq!(apdus[1].p2, 0x01);
    }

    #[test]
    fn p2wpkh_script_code_layout() {
        let hash = [0x42u8; 20];
        let script = p2wpkh_script_code(&hash);
        assert_eq!(script.len(), 25);
        assert_eq!(script[0], 0x76); // OP_DUP
        assert_eq!(script[1], 0xc0); // OP_BLAKE160 (not Bitcoin's OP_HASH160)
        assert_eq!(script[2], 0x14);
        assert_eq!(&script[3..23], &[0x42; 20]);
        assert_eq!(script[23], 0x88);
        assert_eq!(script[24], 0xac);
    }

    #[test]
    fn parse_signature_ok() {
        let mut raw = [0u8; 65];
        raw[64] = 0x01; // sighash type
        let sig = parse_signature(&raw).unwrap();
        assert_eq!(sig[64], 0x01);
    }

    #[test]
    fn parse_signature_wrong_len() {
        assert!(parse_signature(&[0u8; 64]).is_err());
        assert!(parse_signature(&[0u8; 66]).is_err());
    }
}
