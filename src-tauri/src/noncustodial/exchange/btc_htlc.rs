//! The BTC lock of a trade (spec §3.1, §3.2), on rust-bitcoin: the shared
//! script paid to as P2WSH (program = SHA-256 of the script), its refund
//! time as a Unix time (BIP 65 / BIP 113), and the two spends, signed with
//! BIP 143 `SIGHASH_ALL`.
//!
//! - **Claim** (the Seller's): spends the known lock outpoint to the Seller's
//!   address. Bitcoin has no NOINPUT, which is why the Seller commits after
//!   the BTC lock exists. It is presigned as a fee ladder (`fees`), every
//!   rung signalling replacement (BIP 125), with the secret slot empty.
//! - **Refund** (the Buyer's): spends the lock after its refund time, also a
//!   ladder.
//!
//! Every spend here is run through Bitcoin Core's interpreter in the tests
//! (`exchange_btc_htlc_tests`, spec R5).

use bitcoin::absolute::LockTime;
use bitcoin::hashes::Hash;
use bitcoin::secp256k1::{self as bsecp, Message, Secp256k1};
use bitcoin::sighash::{EcdsaSighashType, SighashCache};
use bitcoin::transaction::Version;
use bitcoin::{
    Address, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness,
};
use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::noncustodial::exchange::script::{self, HtlcParams};
use crate::noncustodial::network::Network;

/// BIP 113: a lock time at or above this is a Unix time.
pub const LOCKTIME_THRESHOLD: u32 = 500_000_000;

/// Sequence of every presigned spend: below `0xfffffffe`, so it signals
/// replacement (BIP 125) and enables the lock time.
pub const SPEND_SEQUENCE: Sequence = Sequence(0xffff_fffd);

/// The Bitcoin network that pairs with an HNS network: mainnet with
/// mainnet, testnet with testnet, regtest and simnet with regtest.
pub fn btc_network(network: Network) -> bitcoin::Network {
    match network {
        Network::Main => bitcoin::Network::Bitcoin,
        Network::Testnet => bitcoin::Network::Testnet,
        Network::Regtest | Network::Simnet => bitcoin::Network::Regtest,
    }
}

/// The Bitcoin lock time for "not before `unix_secs`". Refuses a time that
/// Bitcoin would read as a height or that does not fit 32 bits.
pub fn encode_time_lock(unix_secs: u64) -> Result<u32, AppError> {
    u32::try_from(unix_secs)
        .ok()
        .filter(|t| *t >= LOCKTIME_THRESHOLD)
        .ok_or_else(|| AppError::InvalidInput(format!("{unix_secs} cannot be a Bitcoin time lock")))
}

/// True once a transaction with time lock `lock` can be mined on top of a
/// tip whose median time past is `mtp`: Bitcoin Core `IsFinalTx` asks
/// `nLockTime < nBlockTime` with the median time past (BIP 113).
pub fn time_lock_open(lock: u32, mtp: u64) -> bool {
    lock >= LOCKTIME_THRESHOLD && u64::from(lock) < mtp
}

pub fn lock_script(params: &HtlcParams) -> ScriptBuf {
    ScriptBuf::from_bytes(params.script())
}

/// The lock's `scriptPubKey`: `OP_0 <SHA-256(script)>`.
pub fn lock_script_pubkey(params: &HtlcParams) -> ScriptBuf {
    ScriptBuf::new_p2wsh(&lock_script(params).wscript_hash())
}

pub fn lock_address(network: Network, params: &HtlcParams) -> Address {
    Address::p2wsh(&lock_script(params), btc_network(network))
}

/// True if `out` is exactly the lock of `params` worth exactly `value`.
pub fn is_lock_output(out: &TxOut, params: &HtlcParams, value: u64) -> bool {
    out.value == Amount::from_sat(value) && out.script_pubkey == lock_script_pubkey(params)
}

/// Parse a Bitcoin address and check it belongs to `network`'s Bitcoin.
pub fn parse_address(network: Network, addr: &str) -> Result<Address, AppError> {
    addr.trim()
        .parse::<Address<bitcoin::address::NetworkUnchecked>>()
        .map_err(|e| AppError::InvalidInput(format!("'{addr}' is not a Bitcoin address: {e}")))?
        .require_network(btc_network(network))
        .map_err(|_| {
            AppError::InvalidInput(format!(
                "'{addr}' is not an address of {:?}",
                btc_network(network)
            ))
        })
}

fn to_bitcoin_key(k: &secp256k1::SecretKey) -> Result<bsecp::SecretKey, AppError> {
    bsecp::SecretKey::from_slice(&k.secret_bytes())
        .map_err(|e| AppError::Crypto(format!("key conversion: {e}")))
}

fn pubkey_of(k: &bsecp::SecretKey) -> [u8; 33] {
    bsecp::PublicKey::from_secret_key(&Secp256k1::signing_only(), k).serialize()
}

/// A spend of the lock at `outpoint` to `to`, `value - fee`, unsigned.
fn spend_tx(outpoint: OutPoint, to: &Address, to_value: u64, lock_time: LockTime) -> Transaction {
    Transaction {
        version: Version::TWO,
        lock_time,
        input: vec![TxIn {
            previous_output: outpoint,
            script_sig: ScriptBuf::new(),
            sequence: SPEND_SEQUENCE,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(to_value),
            script_pubkey: to.script_pubkey(),
        }],
    }
}

/// The largest DER signature plus its sighash byte (73 bytes), for sizing a
/// fee before the signature exists. A real signature is 71–73 bytes, so the
/// fee is never short.
const MAX_SIG_LEN: usize = 73;

/// vsize of a claim (with a secret) or a refund (without) of `params`'s lock
/// to `to`.
pub fn spend_vsize(params: &HtlcParams, to: &Address, claim: bool) -> u64 {
    let lock_time = if claim {
        LockTime::ZERO
    } else {
        LockTime::from_consensus(params.refund_lock_time)
    };
    let mut tx = spend_tx(
        OutPoint {
            txid: Txid::all_zeros(),
            vout: 0,
        },
        to,
        0,
        lock_time,
    );
    let script = params.script();
    let items = if claim {
        script::claim_witness(&[0u8; MAX_SIG_LEN], &[0u8; 32], &script)
    } else {
        script::refund_witness(&[0u8; MAX_SIG_LEN], &script)
    };
    tx.input[0].witness = Witness::from_slice(&items);
    tx.vsize() as u64
}

fn sign_spend(
    tx: &Transaction,
    params: &HtlcParams,
    lock_value: u64,
    key: &bsecp::SecretKey,
) -> Result<Vec<u8>, AppError> {
    let script = lock_script(params);
    let sighash = SighashCache::new(tx)
        .p2wsh_signature_hash(
            0,
            &script,
            Amount::from_sat(lock_value),
            EcdsaSighashType::All,
        )
        .map_err(|e| AppError::Crypto(format!("sighash: {e}")))?;
    let sig =
        Secp256k1::signing_only().sign_ecdsa(&Message::from_digest(sighash.to_byte_array()), key);
    let mut out = sig.serialize_der().to_vec();
    out.push(EcdsaSighashType::All as u8);
    Ok(out)
}

/// One presigned spend of a BTC lock: the signed transaction with an empty
/// secret slot (claims) or complete (refunds), and its fee.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresignedSpend {
    /// Consensus hex of the transaction. For a claim, witness item 1 (the
    /// secret) is 32 zero bytes until [`finish_claim`] fills it.
    pub tx_hex: String,
    pub fee: u64,
    pub fee_rate: u64,
}

fn decode_tx(hex_tx: &str) -> Result<Transaction, AppError> {
    let bytes = hex::decode(hex_tx).map_err(|_| AppError::InvalidInput("tx is not hex".into()))?;
    bitcoin::consensus::deserialize(&bytes)
        .map_err(|e| AppError::InvalidInput(format!("not a Bitcoin transaction: {e}")))
}

pub fn encode_tx(tx: &Transaction) -> String {
    hex::encode(bitcoin::consensus::serialize(tx))
}

/// Sign the Seller's claim of the lock at `outpoint` worth `lock_value`,
/// paying `lock_value - fee` to `to`. `key` must be the lock's claim key.
pub fn sign_claim(
    params: &HtlcParams,
    outpoint: OutPoint,
    lock_value: u64,
    key: &secp256k1::SecretKey,
    to: &Address,
    fee: u64,
    fee_rate: u64,
) -> Result<PresignedSpend, AppError> {
    params.check()?;
    let key = to_bitcoin_key(key)?;
    if pubkey_of(&key) != params.claim_pubkey {
        return Err(AppError::InvalidInput(
            "the claim key does not belong to this lock".into(),
        ));
    }
    let to_value = spendable(lock_value, fee, to)?;
    let mut tx = spend_tx(outpoint, to, to_value, LockTime::ZERO);
    let sig = sign_spend(&tx, params, lock_value, &key)?;
    tx.input[0].witness =
        Witness::from_slice(&script::claim_witness(&sig, &[0u8; 32], &params.script()));
    Ok(PresignedSpend {
        tx_hex: encode_tx(&tx),
        fee,
        fee_rate,
    })
}

/// Sign the Buyer's refund of the lock at `outpoint`, valid once the refund
/// time is open. `key` must be the lock's refund key.
pub fn sign_refund(
    params: &HtlcParams,
    outpoint: OutPoint,
    lock_value: u64,
    key: &secp256k1::SecretKey,
    to: &Address,
    fee: u64,
    fee_rate: u64,
) -> Result<PresignedSpend, AppError> {
    params.check()?;
    if params.refund_lock_time < LOCKTIME_THRESHOLD {
        return Err(AppError::InvalidInput(
            "a BTC lock's refund time must be a time, not a height".into(),
        ));
    }
    let key = to_bitcoin_key(key)?;
    if pubkey_of(&key) != params.refund_pubkey {
        return Err(AppError::InvalidInput(
            "the refund key does not belong to this lock".into(),
        ));
    }
    let to_value = spendable(lock_value, fee, to)?;
    let mut tx = spend_tx(
        outpoint,
        to,
        to_value,
        LockTime::from_consensus(params.refund_lock_time),
    );
    let sig = sign_spend(&tx, params, lock_value, &key)?;
    tx.input[0].witness = Witness::from_slice(&script::refund_witness(&sig, &params.script()));
    Ok(PresignedSpend {
        tx_hex: encode_tx(&tx),
        fee,
        fee_rate,
    })
}

/// `lock_value - fee`, refused when it would be dust (Bitcoin Core's dust
/// limit for the output type) or nothing.
fn spendable(lock_value: u64, fee: u64, to: &Address) -> Result<u64, AppError> {
    let v = lock_value
        .checked_sub(fee)
        .ok_or_else(|| AppError::InvalidInput("the BTC fee is more than the lock".into()))?;
    let dust = to.script_pubkey().minimal_non_dust().to_sat();
    if v < dust {
        return Err(AppError::InvalidInput(
            "what is left of the BTC lock after the fee would be dust".into(),
        ));
    }
    Ok(v)
}

/// The claim with `secret` in its slot. Refuses a secret that does not hash
/// to the lock's `H`, and a transaction that is not a claim of `params`.
pub fn finish_claim(
    presigned: &PresignedSpend,
    params: &HtlcParams,
    secret: &[u8; 32],
) -> Result<Transaction, AppError> {
    let h: [u8; 32] = Sha256::digest(secret).into();
    if h != params.hash {
        return Err(AppError::InvalidInput(
            "the secret does not open this lock".into(),
        ));
    }
    let mut tx = decode_tx(&presigned.tx_hex)?;
    let items: Vec<Vec<u8>> = tx
        .input
        .first()
        .map(|i| i.witness.to_vec())
        .unwrap_or_default();
    if tx.input.len() != 1 || items.len() != 4 || items[2] != [0x01] || !params.matches(&items[3]) {
        return Err(AppError::InvalidInput(
            "the presigned claim is not a claim of this lock".into(),
        ));
    }
    let filled = script::claim_witness(&items[0], secret, &items[3]);
    tx.input[0].witness = Witness::from_slice(&filled);
    Ok(tx)
}

/// The refund as a transaction, checked to be a refund of `params`.
pub fn refund_tx(presigned: &PresignedSpend, params: &HtlcParams) -> Result<Transaction, AppError> {
    let tx = decode_tx(&presigned.tx_hex)?;
    let items: Vec<Vec<u8>> = tx
        .input
        .first()
        .map(|i| i.witness.to_vec())
        .unwrap_or_default();
    if tx.input.len() != 1
        || items.len() != 3
        || !items[1].is_empty()
        || !params.matches(&items[2])
        || tx.lock_time.to_consensus_u32() != params.refund_lock_time
    {
        return Err(AppError::InvalidInput(
            "the presigned refund is not a refund of this lock".into(),
        ));
    }
    Ok(tx)
}

/// The txid as Bitcoin displays it (byte-reversed hex).
pub fn txid_hex(tx: &Transaction) -> String {
    tx.compute_txid().to_string()
}

/// Parse a displayed txid.
pub fn parse_txid(s: &str) -> Result<Txid, AppError> {
    s.parse::<Txid>()
        .map_err(|e| AppError::InvalidInput(format!("'{s}' is not a txid: {e}")))
}

/// Decode a transaction from consensus hex.
pub fn decode(hex_tx: &str) -> Result<Transaction, AppError> {
    decode_tx(hex_tx)
}

/// Whether input 0 of `tx`, a spend of `params`'s lock worth `lock_value`,
/// carries a valid `SIGHASH_ALL` signature of `pubkey` (BIP 143, low S),
/// whatever sits in the secret slot (the signature does not cover the
/// witness). A watchtower checks a job with this, so that only the holder
/// of the lock's key can register spends of it.
pub fn spend_signed_by(
    tx: &Transaction,
    params: &HtlcParams,
    lock_value: u64,
    pubkey: &[u8; 33],
) -> bool {
    let Some(sig_item) = tx.input.first().and_then(|i| i.witness.nth(0)) else {
        return false;
    };
    let Some((&hash_type, der)) = sig_item.split_last() else {
        return false;
    };
    if hash_type != EcdsaSighashType::All as u8 {
        return false;
    }
    let Ok(sig) = bsecp::ecdsa::Signature::from_der(der) else {
        return false;
    };
    let mut low = sig;
    low.normalize_s();
    if low != sig {
        return false;
    }
    let Ok(pk) = bsecp::PublicKey::from_slice(pubkey) else {
        return false;
    };
    let Ok(sighash) = SighashCache::new(tx).p2wsh_signature_hash(
        0,
        &lock_script(params),
        Amount::from_sat(lock_value),
        EcdsaSighashType::All,
    ) else {
        return false;
    };
    Secp256k1::verification_only()
        .verify_ecdsa(&Message::from_digest(sighash.to_byte_array()), &sig, &pk)
        .is_ok()
}
