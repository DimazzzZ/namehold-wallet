//! Raw Handshake transaction construction, signing, and serialization.
//!
//! Every byte-level detail in this module is verified against canonical hsd
//! 8.0.0 source:
//!   - `lib/primitives/tx.js` `signatureHash(index, prev, value, type)` — the
//!     sighash preimage and the use of a SINGLE Blake2b-256 digest (NOT
//!     double-SHA256 like Bitcoin BIP143).
//!   - `lib/primitives/tx.js` `write(bw)` — the witness serialization layout
//!     (`version || varint(nin) || inputs || varint(nout) || outputs ||
//!     locktime || witnesses`). Handshake has no segwit marker/flag byte; the
//!     witness is always appended after locktime.
//!   - `lib/primitives/input.js` `write` — `prevout || sequence(u32)` (no
//!     scriptSig; Handshake is witness-only).
//!   - `lib/primitives/outpoint.js` `write` — `hash(32) || index(u32)`.
//!   - `lib/primitives/output.js` `write` — `value(u64) || address || covenant`.
//!   - `lib/primitives/address.js` `write` — `version(u8) || len(u8) || hash`.
//!   - `lib/primitives/covenant.js` `write` — `type(u8) || varint(count) ||
//!     varbytes(item)*`. An empty covenant for a plain send is `00 00`.
//!   - `lib/script/witness.js` `write` — `varint(nitems) || varbytes(item)*`.
//!   - `lib/script/script.js` `fromPubkeyhash(hash)` — the script code used as
//!     `prev` when signing a P2WPKH input:
//!     `OP_DUP(0x76) OP_BLAKE160(0xc0) <push20:0x14> <hash160>
//!      OP_EQUALVERIFY(0x88) OP_CHECKSIG(0xac)` (25 bytes).
//!   - hsd signs with bcrypto `secp256k1.sign` which returns a 64-byte COMPACT
//!     (R||S, low-S) ECDSA signature; `signature()` appends a 1-byte sighash
//!     type → 65-byte witness signature.
//!   - `lib/script/common.js` hashType: ALL=1, NONE=2, SINGLE=3,
//!     SINGLEREVERSE=4, NOINPUT=0x40, ANYONECANPAY=0x80.

use crate::error::AppError;
use crate::noncustodial::address;
use crate::noncustodial::network::Network;
use blake2::digest::consts::U32;
use blake2::{Blake2b, Digest};
use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};

/// Blake2b with a 32-byte (256-bit) output, matching hsd's `blake2b.digest`
/// default (output length 32, no key, no personalization).
type Blake2b256 = Blake2b<U32>;

/// Compute the 32-byte Blake2b-256 digest used throughout Handshake's
/// transaction hashing (sighash sub-digests and the final sighash).
pub fn blake2b256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Blake2b256::new();
    hasher.update(data);
    let out = hasher.finalize();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&out);
    hash
}

/// A 32-byte all-zero hash (hsd `consensus.ZERO_HASH`), used when a sighash
/// flag suppresses a sub-digest.
const ZERO_HASH: [u8; 32] = [0u8; 32];

/// Handshake sighash types (`lib/script/common.js` hashType).
pub mod sighash {
    pub const ALL: u32 = 1;
    pub const NONE: u32 = 2;
    pub const SINGLE: u32 = 3;
    pub const SINGLEREVERSE: u32 = 4;
    pub const NOINPUT: u32 = 0x40;
    pub const ANYONECANPAY: u32 = 0x80;
    /// Mask for the base type (low 5 bits), matching hsd's `type & 0x1f`.
    pub const MASK: u32 = 0x1f;
}

// --- opcodes used in the P2WPKH script code (lib/script/common.js) ---
const OP_DUP: u8 = 0x76;
const OP_BLAKE160: u8 = 0xc0;
const OP_EQUALVERIFY: u8 = 0x88;
const OP_CHECKSIG: u8 = 0xac;

/// Build the P2WPKH script code (`prev`) used when signing a P2WPKH input.
///
/// hsd `Script.fromPubkeyhash(hash)`:
/// `OP_DUP OP_BLAKE160 <push20> <hash160> OP_EQUALVERIFY OP_CHECKSIG` (25 bytes).
pub fn p2wpkh_script_code(hash160: &[u8; 20]) -> Vec<u8> {
    let mut code = Vec::with_capacity(25);
    code.push(OP_DUP);
    code.push(OP_BLAKE160);
    code.push(0x14); // push 20 bytes
    code.extend_from_slice(hash160);
    code.push(OP_EQUALVERIFY);
    code.push(OP_CHECKSIG);
    code
}

/// A minimal little-endian byte writer mirroring hsd's `bufio` semantics for
/// the subset of operations transaction serialization needs.
#[derive(Default)]
struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    fn new() -> Self {
        Writer { buf: Vec::new() }
    }

    fn write_u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    fn write_u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn write_u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn write_bytes(&mut self, v: &[u8]) {
        self.buf.extend_from_slice(v);
    }

    /// Bitcoin/Handshake CompactSize varint (hsd `writeVarint`).
    fn write_varint(&mut self, n: u64) {
        if n < 0xfd {
            self.buf.push(n as u8);
        } else if n <= 0xffff {
            self.buf.push(0xfd);
            self.buf.extend_from_slice(&(n as u16).to_le_bytes());
        } else if n <= 0xffff_ffff {
            self.buf.push(0xfe);
            self.buf.extend_from_slice(&(n as u32).to_le_bytes());
        } else {
            self.buf.push(0xff);
            self.buf.extend_from_slice(&n.to_le_bytes());
        }
    }

    /// varint length prefix followed by the bytes (hsd `writeVarBytes`).
    fn write_var_bytes(&mut self, v: &[u8]) {
        self.write_varint(v.len() as u64);
        self.write_bytes(v);
    }

    fn into_bytes(self) -> Vec<u8> {
        self.buf
    }
}

/// A transaction outpoint (previous output reference).
#[derive(Clone, Debug)]
pub struct Outpoint {
    /// 32-byte transaction hash of the funding tx, in the same natural byte
    /// order hsd uses everywhere (no Bitcoin-style reversal). This is exactly
    /// the hex string the node reports for the coin.
    pub hash: [u8; 32],
    pub index: u32,
}

impl Outpoint {
    fn write(&self, w: &mut Writer) {
        w.write_bytes(&self.hash);
        w.write_u32(self.index);
    }
}

/// A transaction input. Handshake inputs carry no scriptSig (witness-only); the
/// witness is serialized separately at the end of the tx.
#[derive(Clone, Debug)]
pub struct Input {
    pub prevout: Outpoint,
    pub sequence: u32,
    /// Witness stack items. For P2WPKH this is `[signature(65), pubkey(33)]`.
    /// Empty until the input is signed.
    pub witness: Vec<Vec<u8>>,
}

impl Input {
    /// New input with the default final sequence (0xffffffff).
    pub fn new(prevout: Outpoint) -> Self {
        Input {
            prevout,
            sequence: 0xffff_ffff,
            witness: Vec::new(),
        }
    }

    /// Serialize the non-witness portion: `prevout || sequence`.
    fn write(&self, w: &mut Writer) {
        self.prevout.write(w);
        w.write_u32(self.sequence);
    }

    /// Serialize the witness stack: `varint(nitems) || varbytes(item)*`.
    fn write_witness(&self, w: &mut Writer) {
        w.write_varint(self.witness.len() as u64);
        for item in &self.witness {
            w.write_var_bytes(item);
        }
    }
}

/// A covenant attached to an output. A plain HNS send carries an empty covenant
/// (`type 0`, no items), which serializes as `00 00`.
#[derive(Clone, Debug, Default)]
pub struct Covenant {
    pub covenant_type: u8,
    pub items: Vec<Vec<u8>>,
}

impl Covenant {
    fn write(&self, w: &mut Writer) {
        w.write_u8(self.covenant_type);
        w.write_varint(self.items.len() as u64);
        for item in &self.items {
            w.write_var_bytes(item);
        }
    }

    /// Serialize this covenant on its own: `type(u8) || varint(count) ||
    /// varbytes(item)*`. Matches hsd `Covenant.encode()` byte-for-byte and is
    /// used by the hsd-parity tests to compare covenant encodings.
    pub fn to_raw(&self) -> Vec<u8> {
        let mut w = Writer::new();
        self.write(&mut w);
        w.into_bytes()
    }
}

/// A transaction output address: witness version + program (hash).
#[derive(Clone, Debug)]
pub struct OutputAddress {
    pub version: u8,
    pub hash: Vec<u8>,
}

impl OutputAddress {
    fn write(&self, w: &mut Writer) {
        w.write_u8(self.version);
        w.write_u8(self.hash.len() as u8);
        w.write_bytes(&self.hash);
    }
}

/// A transaction output: `value(u64) || address || covenant`.
#[derive(Clone, Debug)]
pub struct Output {
    pub value: u64,
    pub address: OutputAddress,
    pub covenant: Covenant,
}

impl Output {
    fn write(&self, w: &mut Writer) {
        w.write_u64(self.value);
        self.address.write(w);
        self.covenant.write(w);
    }

    fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        self.write(&mut w);
        w.into_bytes()
    }
}

/// A Handshake transaction.
#[derive(Clone, Debug)]
pub struct Transaction {
    pub version: u32,
    pub inputs: Vec<Input>,
    pub outputs: Vec<Output>,
    pub locktime: u32,
}

impl Default for Transaction {
    fn default() -> Self {
        Self::new()
    }
}

impl Transaction {
    /// New empty transaction (version 0, locktime 0 — hsd defaults).
    pub fn new() -> Self {
        Transaction {
            version: 0,
            inputs: Vec::new(),
            outputs: Vec::new(),
            locktime: 0,
        }
    }

    /// Blake2b-256 of all serialized prevouts (`hashPrevouts`).
    fn hash_prevouts(&self) -> [u8; 32] {
        let mut w = Writer::new();
        for input in &self.inputs {
            input.prevout.write(&mut w);
        }
        blake2b256(&w.into_bytes())
    }

    /// Blake2b-256 of all input sequences (`hashSequence`).
    fn hash_sequence(&self) -> [u8; 32] {
        let mut w = Writer::new();
        for input in &self.inputs {
            w.write_u32(input.sequence);
        }
        blake2b256(&w.into_bytes())
    }

    /// Blake2b-256 of all serialized outputs (`hashOutputs`).
    fn hash_outputs(&self) -> [u8; 32] {
        let mut w = Writer::new();
        for output in &self.outputs {
            output.write(&mut w);
        }
        blake2b256(&w.into_bytes())
    }

    /// Compute the Handshake signature hash for input `index`, spending an
    /// output with script code `prev` and amount `value`, under sighash `type`.
    ///
    /// Mirrors hsd `tx.js` `signatureHash` exactly. The result is a single
    /// Blake2b-256 digest of the preimage:
    /// `version || hashPrevouts || hashSequence || prevHash || prevIndex ||
    ///  varbytes(prev) || value || sequence || hashOutputs || locktime || type`.
    pub fn signature_hash(
        &self,
        index: usize,
        prev: &[u8],
        value: u64,
        hash_type: u32,
    ) -> Result<[u8; 32], AppError> {
        if index >= self.inputs.len() {
            return Err(AppError::InvalidInput(format!(
                "signature_hash: input index {index} out of range ({})",
                self.inputs.len()
            )));
        }

        // hsd: NOINPUT replaces `input` with a fresh empty Input for the
        // per-input fields (prevout + sequence) only.
        let use_empty_input = (hash_type & sighash::NOINPUT) != 0;
        let input = &self.inputs[index];

        let base = hash_type & sighash::MASK;

        let prevouts = if (hash_type & sighash::ANYONECANPAY) == 0 {
            self.hash_prevouts()
        } else {
            ZERO_HASH
        };

        let sequences = if (hash_type & sighash::ANYONECANPAY) == 0
            && base != sighash::SINGLE
            && base != sighash::SINGLEREVERSE
            && base != sighash::NONE
        {
            self.hash_sequence()
        } else {
            ZERO_HASH
        };

        let outputs = if base != sighash::SINGLE
            && base != sighash::SINGLEREVERSE
            && base != sighash::NONE
        {
            self.hash_outputs()
        } else if base == sighash::SINGLE {
            if index < self.outputs.len() {
                blake2b256(&self.outputs[index].encode())
            } else {
                ZERO_HASH
            }
        } else if base == sighash::SINGLEREVERSE {
            if index < self.outputs.len() {
                let i = self.outputs.len() - 1 - index;
                blake2b256(&self.outputs[i].encode())
            } else {
                ZERO_HASH
            }
        } else {
            // NONE
            ZERO_HASH
        };

        // Per-input fields: zeroed under NOINPUT (empty Input has
        // hash=ZERO_HASH, index=0, sequence=0).
        let (prev_hash, prev_index, sequence): ([u8; 32], u32, u32) = if use_empty_input {
            (ZERO_HASH, 0, 0)
        } else {
            (input.prevout.hash, input.prevout.index, input.sequence)
        };

        let mut w = Writer::new();
        w.write_u32(self.version);
        w.write_bytes(&prevouts);
        w.write_bytes(&sequences);
        w.write_bytes(&prev_hash);
        w.write_u32(prev_index);
        w.write_var_bytes(prev);
        w.write_u64(value);
        w.write_u32(sequence);
        w.write_bytes(&outputs);
        w.write_u32(self.locktime);
        w.write_u32(hash_type);

        Ok(blake2b256(&w.into_bytes()))
    }

    /// Sign input `index` as a P2WPKH spend with `key`, setting the witness to
    /// `[signature(65), compressed_pubkey(33)]`.
    ///
    /// `prev_hash160` is the Blake2b-160 of the compressed public key (the
    /// program of the P2WPKH output being spent). `value` is that output's
    /// amount in dollarydoos. `hash_type` is normally `sighash::ALL`.
    ///
    /// hsd produces a 64-byte compact (low-S) ECDSA signature and appends the
    /// 1-byte sighash type. `secp256k1`'s `serialize_compact` returns R||S with
    /// low-S already enforced (the crate normalizes on signing).
    pub fn sign_p2wpkh_input(
        &mut self,
        index: usize,
        key: &SecretKey,
        prev_hash160: &[u8; 20],
        value: u64,
        hash_type: u32,
    ) -> Result<(), AppError> {
        let secp = Secp256k1::new();
        let pubkey = PublicKey::from_secret_key(&secp, key);
        let compressed = pubkey.serialize(); // 33-byte SEC1 compressed

        let script_code = p2wpkh_script_code(prev_hash160);
        let sighash = self.signature_hash(index, &script_code, value, hash_type)?;

        let msg = Message::from_digest(sighash);
        let sig = secp.sign_ecdsa(&msg, key);
        // 64-byte compact (R||S), low-S normalized by the crate on signing.
        let compact = sig.serialize_compact();

        // hsd witness signature: 64-byte compact + 1-byte sighash type.
        // The sighash type byte is the low byte of the (u32) type, matching
        // hsd `signature()` which does `bw.writeU8(type)`.
        let mut sig_bytes = Vec::with_capacity(65);
        sig_bytes.extend_from_slice(&compact);
        sig_bytes.push((hash_type & 0xff) as u8);

        let witness = vec![sig_bytes, compressed.to_vec()];
        self.inputs[index].witness = witness;
        Ok(())
    }

    /// Serialize the base (no-witness) form used for txid computation:
    /// `version || varint(nin) || inputs || varint(nout) || outputs || locktime`.
    fn write_base(&self, w: &mut Writer) {
        w.write_u32(self.version);
        w.write_varint(self.inputs.len() as u64);
        for input in &self.inputs {
            input.write(w);
        }
        w.write_varint(self.outputs.len() as u64);
        for output in &self.outputs {
            output.write(w);
        }
        w.write_u32(self.locktime);
    }

    /// The Handshake txid: Blake2b-256 of the NON-witness serialization, hex
    /// -encoded in natural (internal) byte order.
    ///
    /// Unlike Bitcoin, Handshake does NOT byte-reverse hashes for display: hsd
    /// `tx.txid()` is `this.hash().toString('hex')` with no `revHex`, and the
    /// same natural-order bytes are written into spending inputs' prevout hash
    /// (`Outpoint`). Reversing here would make our txid disagree with the node /
    /// explorer and — worse — make spends reference the wrong outpoint.
    pub fn txid(&self) -> String {
        let mut w = Writer::new();
        self.write_base(&mut w);
        hex::encode(blake2b256(&w.into_bytes()))
    }

    /// Serialize the transaction with witnesses for broadcast.
    ///
    /// hsd `tx.js` `write`: `version || varint(nin) || inputs ||
    /// varint(nout) || outputs || locktime || witnesses`. There is no segwit
    /// marker/flag byte; the witness stacks are appended after locktime in
    /// input order.
    pub fn serialize(&self) -> Vec<u8> {
        let mut w = Writer::new();
        self.write_base(&mut w);
        for input in &self.inputs {
            input.write_witness(&mut w);
        }
        w.into_bytes()
    }

    /// Serialize and hex-encode the transaction for broadcast over the node
    /// RPC `sendrawtransaction`.
    pub fn to_hex(&self) -> String {
        hex::encode(self.serialize())
    }
}

/// Convert a validated Handshake bech32 address into an `OutputAddress`
/// (witness version + program), verifying the HRP matches `network`.
pub fn output_address_from_string(
    network: Network,
    addr: &str,
) -> Result<OutputAddress, AppError> {
    let (version, program) = address::decode(network, addr)?;
    Ok(OutputAddress {
        version,
        hash: program,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Blake2b-256 of the empty input. Canonical BLAKE2b-256 test vector.
    #[test]
    fn blake2b256_empty_matches_reference() {
        let h = blake2b256(&[]);
        assert_eq!(
            hex::encode(h),
            "0e5751c026e543b2e8ab2eb06099daa1d1e5df47778f7787faab45cdf12fe3a8"
        );
    }

    #[test]
    fn p2wpkh_script_code_is_canonical_pubkeyhash() {
        let hash160 = [0x11u8; 20];
        let code = p2wpkh_script_code(&hash160);
        assert_eq!(code.len(), 25);
        assert_eq!(code[0], 0x76); // OP_DUP
        assert_eq!(code[1], 0xc0); // OP_BLAKE160
        assert_eq!(code[2], 0x14); // push 20
        assert_eq!(&code[3..23], &hash160[..]);
        assert_eq!(code[23], 0x88); // OP_EQUALVERIFY
        assert_eq!(code[24], 0xac); // OP_CHECKSIG
    }

    #[test]
    fn empty_covenant_serializes_to_two_zero_bytes() {
        let cov = Covenant::default();
        let mut w = Writer::new();
        cov.write(&mut w);
        assert_eq!(w.into_bytes(), vec![0x00, 0x00]);
    }

    #[test]
    fn varint_encoding_matches_compactsize() {
        let mut w = Writer::new();
        w.write_varint(0xfc);
        assert_eq!(w.into_bytes(), vec![0xfc]);

        let mut w = Writer::new();
        w.write_varint(0xfd);
        assert_eq!(w.into_bytes(), vec![0xfd, 0xfd, 0x00]);

        let mut w = Writer::new();
        w.write_varint(0x1_0000);
        assert_eq!(w.into_bytes(), vec![0xfe, 0x00, 0x00, 0x01, 0x00]);

        let mut w = Writer::new();
        w.write_varint(0x1_0000_0000);
        assert_eq!(
            w.into_bytes(),
            vec![0xff, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn output_serialization_layout() {
        // value(8 LE) || version(1) || hashlen(1) || hash || covenant(00 00)
        let out = Output {
            value: 0x0102_0304_0506_0708,
            address: OutputAddress {
                version: 0,
                hash: vec![0xaa; 20],
            },
            covenant: Covenant::default(),
        };
        let bytes = out.encode();
        // 8 value + 1 version + 1 len + 20 hash + 2 covenant = 32
        assert_eq!(bytes.len(), 32);
        assert_eq!(
            &bytes[0..8],
            &[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]
        );
        assert_eq!(bytes[8], 0x00); // address version
        assert_eq!(bytes[9], 20); // hash length
        assert_eq!(&bytes[10..30], &[0xaa; 20]);
        assert_eq!(&bytes[30..32], &[0x00, 0x00]); // empty covenant
    }

    #[test]
    fn signature_hash_commits_output_covenant() {
        // Two otherwise-identical 1-in/1-out txs differing only in the output
        // covenant must produce different sighashes (the covenant is committed
        // via hashOutputs).
        let mk = |covenant: Covenant| {
            let mut tx = Transaction::new();
            tx.inputs.push(Input::new(Outpoint { hash: [0x22; 32], index: 0 }));
            tx.outputs.push(Output {
                value: 1000,
                address: OutputAddress { version: 0, hash: vec![0xbb; 20] },
                covenant,
            });
            tx.signature_hash(0, &p2wpkh_script_code(&[0x11; 20]), 1000, sighash::ALL)
                .unwrap()
        };
        let plain = mk(Covenant::default());
        let with_cov = mk(Covenant {
            covenant_type: 2, // OPEN
            items: vec![vec![1u8; 32], vec![0, 0, 0, 0], b"abc".to_vec()],
        });
        assert_ne!(plain, with_cov);
    }

    #[test]
    fn serialize_layout_has_no_segwit_marker() {
        // A 1-in/1-out tx: witness stacks are appended after locktime with no
        // segwit marker/flag byte (Handshake-specific).
        let mut tx = Transaction::new();
        tx.inputs.push(Input::new(Outpoint {
            hash: [0x22; 32],
            index: 0,
        }));
        tx.outputs.push(Output {
            value: 1000,
            address: OutputAddress {
                version: 0,
                hash: vec![0xbb; 20],
            },
            covenant: Covenant::default(),
        });
        let bytes = tx.serialize();

        // version (4 zero bytes)
        assert_eq!(&bytes[0..4], &[0, 0, 0, 0]);
        // varint(nin) = 1 — NOT a 0x00 segwit marker.
        assert_eq!(bytes[4], 0x01);
        // prevout hash starts immediately after.
        assert_eq!(&bytes[5..37], &[0x22; 32]);
        // The final byte is the (empty) witness count for the single input.
        assert_eq!(*bytes.last().unwrap(), 0x00);
    }

    // A deterministic key + its P2WPKH hash160, for signing tests.
    fn test_key() -> (SecretKey, [u8; 20]) {
        let sk = SecretKey::from_slice(&[7u8; 32]).unwrap();
        let secp = Secp256k1::new();
        let pk = PublicKey::from_secret_key(&secp, &sk).serialize();
        (sk, address::pubkey_to_hash160(&pk))
    }

    fn out(value: u64, hash_byte: u8, covenant: Covenant) -> Output {
        Output {
            value,
            address: OutputAddress { version: 0, hash: vec![hash_byte; 20] },
            covenant,
        }
    }

    fn two_in_two_out() -> Transaction {
        let mut tx = Transaction::new();
        tx.inputs.push(Input::new(Outpoint { hash: [0x11; 32], index: 0 }));
        tx.inputs.push(Input::new(Outpoint { hash: [0x22; 32], index: 1 }));
        tx.outputs.push(out(500_000, 0xaa, Covenant::default()));
        tx.outputs.push(out(499_000, 0xbb, Covenant::default()));
        tx
    }

    #[test]
    fn txid_is_witness_independent_but_serialize_grows() {
        // The txid hashes the NON-witness form, so signing must not change it
        // (witness malleability can't alter the txid we previewed) — but the
        // broadcast serialization does grow by the witness stacks.
        let (sk, h160) = test_key();
        let mut tx = two_in_two_out();
        let txid_before = tx.txid();
        let unsigned_len = tx.serialize().len();

        tx.sign_p2wpkh_input(0, &sk, &h160, 1_000_000, sighash::ALL).unwrap();
        tx.sign_p2wpkh_input(1, &sk, &h160, 50_000, sighash::ALL).unwrap();

        assert_eq!(tx.txid(), txid_before, "txid must not depend on the witness");
        assert!(
            tx.serialize().len() > unsigned_len,
            "signed serialization must include witness bytes"
        );
        // Each P2WPKH witness is [sig(65), pubkey(33)] => 2 items.
        assert_eq!(tx.inputs[0].witness.len(), 2);
        assert_eq!(tx.inputs[1].witness.len(), 2);
        assert_eq!(tx.inputs[0].witness[0].len(), 65);
        assert_eq!(tx.inputs[0].witness[1].len(), 33);
    }

    #[test]
    fn multi_input_sighashes_are_distinct_and_value_committed() {
        let (_sk, h160) = test_key();
        let tx = two_in_two_out();
        let code = p2wpkh_script_code(&h160);
        let sh0 = tx.signature_hash(0, &code, 1_000_000, sighash::ALL).unwrap();
        let sh1 = tx.signature_hash(1, &code, 50_000, sighash::ALL).unwrap();
        // Different prevout/sequence per index => different sighash.
        assert_ne!(sh0, sh1);
        // The spent value is committed: same index, different value => different.
        let sh0b = tx.signature_hash(0, &code, 999_999, sighash::ALL).unwrap();
        assert_ne!(sh0, sh0b, "input value must be committed in the sighash");
    }

    #[test]
    fn sighash_all_commits_every_output_field() {
        // SIGHASH_ALL must commit each output's value, address, and covenant, so
        // tampering with ANY of them after signing invalidates the signature.
        let (_sk, h160) = test_key();
        let code = p2wpkh_script_code(&h160);
        let base = two_in_two_out();
        let sh = base.signature_hash(0, &code, 1_000_000, sighash::ALL).unwrap();

        // Tamper: recipient value.
        let mut t1 = base.clone();
        t1.outputs[0].value += 1;
        assert_ne!(sh, t1.signature_hash(0, &code, 1_000_000, sighash::ALL).unwrap());

        // Tamper: recipient address (redirect funds).
        let mut t2 = base.clone();
        t2.outputs[0].address.hash = vec![0xcc; 20];
        assert_ne!(sh, t2.signature_hash(0, &code, 1_000_000, sighash::ALL).unwrap());

        // Tamper: change output's covenant.
        let mut t3 = base.clone();
        t3.outputs[1].covenant = Covenant { covenant_type: 7, items: vec![vec![1u8; 32]] };
        assert_ne!(sh, t3.signature_hash(0, &code, 1_000_000, sighash::ALL).unwrap());
    }

    #[test]
    fn serialize_then_reparse_layout_round_trips() {
        // Walk the witness serialization back out and confirm the recovered
        // structure re-serializes to the identical bytes (no asymmetric framing).
        let (sk, h160) = test_key();
        let mut tx = two_in_two_out();
        tx.locktime = 42;
        tx.sign_p2wpkh_input(0, &sk, &h160, 1_000_000, sighash::ALL).unwrap();
        tx.sign_p2wpkh_input(1, &sk, &h160, 50_000, sighash::ALL).unwrap();
        let bytes = tx.serialize();

        let mut r = Reader::new(&bytes);
        let version = r.read_u32();
        let nin = r.read_varint() as usize;
        let mut inputs = Vec::new();
        for _ in 0..nin {
            let mut hash = [0u8; 32];
            hash.copy_from_slice(r.read_bytes(32));
            let index = r.read_u32();
            let sequence = r.read_u32();
            inputs.push((Outpoint { hash, index }, sequence));
        }
        let nout = r.read_varint() as usize;
        let mut outputs = Vec::new();
        for _ in 0..nout {
            let value = r.read_u64();
            let version = r.read_u8();
            let hlen = r.read_u8() as usize;
            let hash = r.read_bytes(hlen).to_vec();
            let ctype = r.read_u8();
            let nitems = r.read_varint() as usize;
            let mut items = Vec::new();
            for _ in 0..nitems {
                let ilen = r.read_varint() as usize;
                items.push(r.read_bytes(ilen).to_vec());
            }
            outputs.push(Output {
                value,
                address: OutputAddress { version, hash },
                covenant: Covenant { covenant_type: ctype, items },
            });
        }
        let locktime = r.read_u32();
        let mut rebuilt = Transaction { version, inputs: Vec::new(), outputs, locktime };
        for (i, (prevout, sequence)) in inputs.into_iter().enumerate() {
            let mut inp = Input::new(prevout);
            inp.sequence = sequence;
            // Recover the witness stack for this input.
            let nitems = r.read_varint() as usize;
            for _ in 0..nitems {
                let ilen = r.read_varint() as usize;
                inp.witness.push(r.read_bytes(ilen).to_vec());
            }
            rebuilt.inputs.push(inp);
            let _ = i;
        }
        assert!(r.at_end(), "parser must consume the entire buffer");
        assert_eq!(rebuilt.serialize(), bytes, "round-trip must be byte-identical");
        assert_eq!(rebuilt.txid(), tx.txid());
    }

    /// Minimal LE reader mirroring `Writer`, used only by the round-trip test.
    struct Reader<'a> {
        buf: &'a [u8],
        pos: usize,
    }
    impl<'a> Reader<'a> {
        fn new(buf: &'a [u8]) -> Self {
            Reader { buf, pos: 0 }
        }
        fn read_u8(&mut self) -> u8 {
            let v = self.buf[self.pos];
            self.pos += 1;
            v
        }
        fn read_u32(&mut self) -> u32 {
            let v = u32::from_le_bytes(self.buf[self.pos..self.pos + 4].try_into().unwrap());
            self.pos += 4;
            v
        }
        fn read_u64(&mut self) -> u64 {
            let v = u64::from_le_bytes(self.buf[self.pos..self.pos + 8].try_into().unwrap());
            self.pos += 8;
            v
        }
        fn read_bytes(&mut self, n: usize) -> &'a [u8] {
            let s = &self.buf[self.pos..self.pos + n];
            self.pos += n;
            s
        }
        fn read_varint(&mut self) -> u64 {
            let first = self.read_u8();
            match first {
                0xff => self.read_u64(),
                0xfe => self.read_u32() as u64,
                0xfd => {
                    let v = u16::from_le_bytes(self.buf[self.pos..self.pos + 2].try_into().unwrap());
                    self.pos += 2;
                    v as u64
                }
                n => n as u64,
            }
        }
        fn at_end(&self) -> bool {
            self.pos == self.buf.len()
        }
    }
}
