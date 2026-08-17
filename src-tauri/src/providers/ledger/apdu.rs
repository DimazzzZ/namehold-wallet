//! APDU command builders and response parsers for the Handshake Ledger app
//! (`handshake-org/ledger-app-hns`).
//!
//! Byte layouts are transcribed from the reference client `hsd-ledger`
//! (Node.js, v2.0.2) and cross-checked against the on-device parser in
//! `ledger-app-hns/src/apdu-signature.c`. See the module-level doc in
//! `mod.rs` for the full serialization contract.
//!
//! Endianness (critical — do not "fix"):
//!   * APDU header bytes (CLA/INS/P1/P2/Lc): u8 each.
//!   * BIP32 path elements: **big-endian** u32, preceded by a u8 depth.
//!   * tx version/locktime/sequence/index and sighash type: **little-endian**.
//!   * coin/output values: **little-endian** u64.
//!   * All lengths (covenant items, script): hsd varint (compact-size-like).

use crate::error::AppError;

// --- Instruction class + opcodes (hsd-ledger `lib/apdu/common.js`) ---

/// All Handshake instructions use CLA `0xE0` (`CLA.GENERAL`).
pub const CLA_GENERAL: u8 = 0xE0;

/// `INS.GET_APP_VERSION` — returns the running app's semantic version.
pub const INS_GET_APP_VERSION: u8 = 0x40;
/// `INS.GET_PUBLIC_KEY` — derives a pubkey/xpub/address at a BIP32 path.
pub const INS_GET_PUBLIC_KEY: u8 = 0x42;
/// `INS.GET_INPUT_SIGNATURE` — drives both tx "parse" and per-input "sign".
pub const INS_GET_INPUT_SIGNATURE: u8 = 0x44;

// --- P1 network flags (low bits of P1 in parse mode / get-public-key) ---
pub const NET_FLAG_MAIN: u8 = 0x00;
pub const NET_FLAG_TESTNET: u8 = 0x02;
pub const NET_FLAG_REGTEST: u8 = 0x04;
pub const NET_FLAG_SIMNET: u8 = 0x06;

/// Max payload bytes per APDU (`MAX_TX_PACKET`). Longer blobs are split.
pub const MAX_APDU_DATA: usize = 255;

/// Success status word.
pub const SW_OK: u16 = 0x9000;
/// User rejected the on-device prompt (`CONDITIONS_OF_USE_NOT_SATISFIED`).
pub const SW_USER_REJECTED: u16 = 0x6985;

/// Map a network name to the Ledger P1 network flag. Simnet is folded to the
/// testnet flag for on-device address rendering (the app knows only the four
/// coin types; simnet shares testnet's display treatment).
pub fn network_flag(network: crate::noncustodial::network::Network) -> u8 {
    use crate::noncustodial::network::Network;
    match network {
        Network::Main => NET_FLAG_MAIN,
        Network::Testnet => NET_FLAG_TESTNET,
        Network::Regtest => NET_FLAG_REGTEST,
        Network::Simnet => NET_FLAG_SIMNET,
    }
}

/// A raw APDU command (pre-HID-framing).
#[derive(Debug, Clone)]
pub struct ApduCommand {
    pub cla: u8,
    pub ins: u8,
    pub p1: u8,
    pub p2: u8,
    pub data: Vec<u8>,
}

impl ApduCommand {
    /// Serialize to the ISO-7816 short-APDU wire form (no `Le`):
    /// `CLA | INS | P1 | P2 | Lc | data`. `data` must be <= 255 bytes.
    pub fn to_raw(&self) -> Result<Vec<u8>, AppError> {
        if self.data.len() > MAX_APDU_DATA {
            return Err(AppError::Device(format!(
                "APDU data too long: {} > {}",
                self.data.len(),
                MAX_APDU_DATA
            )));
        }
        let mut out = Vec::with_capacity(5 + self.data.len());
        out.push(self.cla);
        out.push(self.ins);
        out.push(self.p1);
        out.push(self.p2);
        out.push(self.data.len() as u8);
        out.extend_from_slice(&self.data);
        Ok(out)
    }
}

/// Encode a BIP32 path for `GET_PUBLIC_KEY` / signing messages:
/// `u8 depth | u32BE index * depth`. Hardened indices carry the high bit.
pub fn encode_path(path: &[u32]) -> Result<Vec<u8>, AppError> {
    if path.len() > 10 {
        return Err(AppError::Device(format!(
            "derivation path too deep: {} levels (max 10)",
            path.len()
        )));
    }
    let mut out = Vec::with_capacity(1 + path.len() * 4);
    out.push(path.len() as u8);
    for &idx in path {
        out.extend_from_slice(&idx.to_be_bytes());
    }
    Ok(out)
}

/// Build the `GET_APP_VERSION` command (empty payload).
pub fn get_app_version() -> ApduCommand {
    ApduCommand {
        cla: CLA_GENERAL,
        ins: INS_GET_APP_VERSION,
        p1: 0x00,
        p2: 0x00,
        data: Vec::new(),
    }
}

/// Build the `GET_PUBLIC_KEY` command.
///
/// * `confirm` — require the user to confirm on-device (P1 bit 0x01).
/// * `net_flag` — network low bits ORed into P1.
/// * `with_xpub` — request chain code + parent fingerprint (P2 bit 0x01).
/// * `with_address` — request the bech32 address string (P2 bit 0x02).
pub fn get_public_key(
    path: &[u32],
    confirm: bool,
    net_flag: u8,
    with_xpub: bool,
    with_address: bool,
) -> Result<ApduCommand, AppError> {
    let p1 = (if confirm { 0x01 } else { 0x00 }) | net_flag;
    let p2 = (if with_address { 0x02 } else { 0x00 }) | (if with_xpub { 0x01 } else { 0x00 });
    Ok(ApduCommand {
        cla: CLA_GENERAL,
        ins: INS_GET_PUBLIC_KEY,
        p1,
        p2,
        data: encode_path(path)?,
    })
}

/// Parsed `GET_PUBLIC_KEY` response.
#[derive(Debug, Clone)]
pub struct PublicKeyResponse {
    pub public_key: [u8; 33],
    pub chain_code: Option<[u8; 32]>,
    pub parent_fingerprint: Option<u32>,
    pub address: Option<String>,
}

/// Parse a `GET_PUBLIC_KEY` response body (status word already stripped).
///
/// Layout: `pubkey[33] | ccLen u8 | cc[ccLen] | fpLen u8 | fp(u32BE if fpLen) |
/// addrLen u8 | addr[addrLen]`.
pub fn parse_public_key(body: &[u8]) -> Result<PublicKeyResponse, AppError> {
    let mut cur = Cursor::new(body);
    let pk = cur.take(33)?;
    let mut public_key = [0u8; 33];
    public_key.copy_from_slice(pk);

    let cc_len = cur.u8()? as usize;
    let chain_code = if cc_len == 32 {
        let cc = cur.take(32)?;
        let mut buf = [0u8; 32];
        buf.copy_from_slice(cc);
        Some(buf)
    } else {
        if cc_len != 0 {
            cur.take(cc_len)?; // skip unexpected length
        }
        None
    };

    let fp_len = cur.u8()? as usize;
    let parent_fingerprint = if fp_len == 4 {
        Some(u32::from_be_bytes([cur.u8()?, cur.u8()?, cur.u8()?, cur.u8()?]))
    } else {
        if fp_len != 0 {
            cur.take(fp_len)?;
        }
        None
    };

    let address = if cur.remaining() > 0 {
        let addr_len = cur.u8()? as usize;
        if addr_len > 0 {
            let bytes = cur.take(addr_len)?;
            Some(
                String::from_utf8(bytes.to_vec())
                    .map_err(|e| AppError::Device(format!("bad address utf8: {e}")))?,
            )
        } else {
            None
        }
    } else {
        None
    };

    Ok(PublicKeyResponse {
        public_key,
        chain_code,
        parent_fingerprint,
        address,
    })
}

/// A minimal forward-only byte cursor with checked reads.
pub struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    pub fn u8(&mut self) -> Result<u8, AppError> {
        let b = self.take(1)?;
        Ok(b[0])
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8], AppError> {
        if self.pos + n > self.buf.len() {
            return Err(AppError::Device(format!(
                "APDU response truncated: wanted {n} bytes at offset {}, have {}",
                self.pos,
                self.buf.len()
            )));
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
}

/// Encode an hsd varint (Bitcoin compact-size-like, LE tail):
///   n < 0xFD          -> [n as u8]
///   n <= 0xFFFF       -> [0xFD, u16le]
///   n <= 0xFFFF_FFFF  -> [0xFE, u32le]
///   else              -> [0xFF, u64le]
pub fn write_varint(out: &mut Vec<u8>, n: u64) {
    if n < 0xFD {
        out.push(n as u8);
    } else if n <= 0xFFFF {
        out.push(0xFD);
        out.extend_from_slice(&(n as u16).to_le_bytes());
    } else if n <= 0xFFFF_FFFF {
        out.push(0xFE);
        out.extend_from_slice(&(n as u32).to_le_bytes());
    } else {
        out.push(0xFF);
        out.extend_from_slice(&n.to_le_bytes());
    }
}

/// Encode length-prefixed bytes (hsd `writeVarBytes`): `varint(len) | bytes`.
pub fn write_var_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    write_varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_boundaries() {
        let mut v = Vec::new();
        write_varint(&mut v, 0x10);
        assert_eq!(v, vec![0x10]);

        v.clear();
        write_varint(&mut v, 0xFD);
        assert_eq!(v, vec![0xFD, 0xFD, 0x00]);

        v.clear();
        write_varint(&mut v, 0x1234);
        assert_eq!(v, vec![0xFD, 0x34, 0x12]);

        v.clear();
        write_varint(&mut v, 0x0001_0000);
        assert_eq!(v, vec![0xFE, 0x00, 0x00, 0x01, 0x00]);

        v.clear();
        write_varint(&mut v, 0x1_0000_0000);
        assert_eq!(v, vec![0xFF, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn path_encoding_is_big_endian_with_depth_prefix() {
        // m/44'/5353'/0'/0/0
        let path = [
            44 + 0x8000_0000,
            5353 + 0x8000_0000,
            0x8000_0000,
            0,
            0,
        ];
        let enc = encode_path(&path).unwrap();
        assert_eq!(enc[0], 5, "depth prefix");
        // 44' = 0x8000002C, big-endian
        assert_eq!(&enc[1..5], &[0x80, 0x00, 0x00, 0x2C]);
        // 5353' = 0x800014E9
        assert_eq!(&enc[5..9], &[0x80, 0x00, 0x14, 0xE9]);
    }

    #[test]
    fn apdu_raw_layout() {
        let cmd = get_app_version();
        let raw = cmd.to_raw().unwrap();
        assert_eq!(raw, vec![0xE0, 0x40, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn get_public_key_p1_p2_bits() {
        let path = [44 + 0x8000_0000, 5353 + 0x8000_0000, 0x8000_0000];
        let cmd = get_public_key(&path, true, NET_FLAG_TESTNET, true, true).unwrap();
        assert_eq!(cmd.p1, 0x01 | NET_FLAG_TESTNET);
        assert_eq!(cmd.p2, 0x02 | 0x01);
        assert_eq!(cmd.ins, INS_GET_PUBLIC_KEY);
    }

    #[test]
    fn parse_public_key_full() {
        // pubkey(33) | ccLen=32 | cc(32) | fpLen=4 | fp(4) | addrLen=5 | "hs1qx"
        let mut body = Vec::new();
        body.extend_from_slice(&[0x02; 33]);
        body.push(32);
        body.extend_from_slice(&[0xAB; 32]);
        body.push(4);
        body.extend_from_slice(&0x1122_3344u32.to_be_bytes());
        body.push(5);
        body.extend_from_slice(b"hs1qx");
        let parsed = parse_public_key(&body).unwrap();
        assert_eq!(parsed.public_key[0], 0x02);
        assert_eq!(parsed.chain_code.unwrap()[0], 0xAB);
        assert_eq!(parsed.parent_fingerprint, Some(0x1122_3344));
        assert_eq!(parsed.address.as_deref(), Some("hs1qx"));
    }

    #[test]
    fn parse_public_key_minimal() {
        // pubkey(33) | ccLen=0 | fpLen=0  (no address)
        let mut body = Vec::new();
        body.extend_from_slice(&[0x03; 33]);
        body.push(0);
        body.push(0);
        let parsed = parse_public_key(&body).unwrap();
        assert!(parsed.chain_code.is_none());
        assert!(parsed.parent_fingerprint.is_none());
        assert!(parsed.address.is_none());
    }
}
