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

use super::apdu::{
    encode_path, write_var_bytes, ApduCommand, CLA_GENERAL, INS_GET_INPUT_SIGNATURE, MAX_APDU_DATA,
};

/// Build the sign-mode APDU sequence for one input.
///
/// * `network` / `account` / `branch` / `child_index` — locate the signing key.
/// * `sighash_type` — typically `SIGHASH_ALL` (0x01).
/// * `prevout_hash` — 32-byte txid of the coin being spent (no byte-reversal).
/// * `prevout_index` — output index of the coin.
/// * `value` — coin value in doos.
/// * `sequence` — input sequence (0xFFFFFFFF for final).
/// * `pubkey` — the compressed public key (33 bytes) of the signing key. Used
///   to derive the HASH160 for the p2wpkh script code.
///
/// Returns the APDU sequence (may be >1 if the blob exceeds 255 bytes, though
/// for standard p2wpkh inputs it's always a single APDU).
#[allow(clippy::too_many_arguments)]
pub fn build_sign_apdus(
    network: Network,
    account: u32,
    branch: u32,
    child_index: u32,
    sighash_type: u32,
    prevout_hash: &[u8; 32],
    prevout_index: u32,
    value: u64,
    sequence: u32,
    pubkey: &[u8; 33],
) -> Result<Vec<ApduCommand>, AppError> {
    let blob = build_sign_blob(
        network,
        account,
        branch,
        child_index,
        sighash_type,
        prevout_hash,
        prevout_index,
        value,
        sequence,
        pubkey,
    )?;
    Ok(split_sign_apdus(&blob))
}

/// Build the raw sign-mode blob.
#[allow(clippy::too_many_arguments)]
pub fn build_sign_blob(
    network: Network,
    account: u32,
    branch: u32,
    child_index: u32,
    sighash_type: u32,
    prevout_hash: &[u8; 32],
    prevout_index: u32,
    value: u64,
    sequence: u32,
    pubkey: &[u8; 33],
) -> Result<Vec<u8>, AppError> {
    let path = bip44_path(network, account, branch, child_index);
    let mut buf = Vec::with_capacity(128);

    // Path (depth prefix + BE indices)
    let path_bytes = encode_path(&path)?;
    buf.extend_from_slice(&path_bytes);

    // Sighash type (u32LE)
    buf.extend_from_slice(&sighash_type.to_le_bytes());

    // Prevout
    buf.extend_from_slice(prevout_hash);
    buf.extend_from_slice(&prevout_index.to_le_bytes());

    // Value + sequence
    buf.extend_from_slice(&value.to_le_bytes());
    buf.extend_from_slice(&sequence.to_le_bytes());

    // Script code: p2wpkh → the classic 25-byte OP_DUP OP_HASH160 <hash160>
    // OP_EQUALVERIFY OP_CHECKSIG (same as BIP143 for Bitcoin).
    let hash160 = pubkey_to_hash160(pubkey);
    let script = p2wpkh_script_code(&hash160);
    write_var_bytes(&mut buf, &script);

    // No single-output commitment (SIGHASH_ALL only in Namehold).
    buf.push(0x00);

    Ok(buf)
}

/// The 25-byte p2pkh script used as the "script code" in BIP143-style sighash
/// for p2wpkh inputs:
/// `OP_DUP OP_HASH160 OP_PUSH20 <hash160> OP_EQUALVERIFY OP_CHECKSIG`
pub fn p2wpkh_script_code(hash160: &[u8; 20]) -> [u8; 25] {
    let mut s = [0u8; 25];
    s[0] = 0x76; // OP_DUP
    s[1] = 0xA9; // OP_HASH160
    s[2] = 0x14; // push 20 bytes
    s[3..23].copy_from_slice(hash160);
    s[23] = 0x88; // OP_EQUALVERIFY
    s[24] = 0xAC; // OP_CHECKSIG
    s
}

/// Split the sign blob into APDUs.
///
/// * P2 = 0x01 (sign mode).
/// * First APDU: P1 = 0x01.
/// * Subsequent: P1 = 0x00.
/// * **No network flag in sign-mode P1** (per hsd-ledger reference).
fn split_sign_apdus(blob: &[u8]) -> Vec<ApduCommand> {
    let chunks: Vec<&[u8]> = blob.chunks(MAX_APDU_DATA).collect();
    let mut cmds = Vec::with_capacity(chunks.len());
    for (i, chunk) in chunks.iter().enumerate() {
        cmds.push(ApduCommand {
            cla: CLA_GENERAL,
            ins: INS_GET_INPUT_SIGNATURE,
            p1: if i == 0 { 0x01 } else { 0x00 },
            p2: 0x01,
            data: chunk.to_vec(),
        });
    }
    cmds
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

        let blob = build_sign_blob(
            Network::Main,
            0,
            0,
            7,
            1, // SIGHASH_ALL
            &prevout,
            0,
            50_000_000,
            0xFFFF_FFFF,
            &pubkey,
        )
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
        assert_eq!(blob[75], 0xA9); // OP_HASH160
        assert_eq!(blob[76], 0x14); // push 20

        // Trailing: 0x00 (no single-output)
        let last = blob.len() - 1;
        assert_eq!(blob[last], 0x00);
    }

    #[test]
    fn sign_apdus_p1_p2_flags() {
        let blob = vec![0x42u8; 300]; // > 255, forces 2 APDUs
        let apdus = split_sign_apdus(&blob);
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
        assert_eq!(script[0], 0x76);
        assert_eq!(script[1], 0xA9);
        assert_eq!(script[2], 0x14);
        assert_eq!(&script[3..23], &[0x42; 20]);
        assert_eq!(script[23], 0x88);
        assert_eq!(script[24], 0xAC);
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
