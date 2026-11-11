//! The BTC account of a profile (spec §2, R15): BIP 84 single-key P2WPKH
//! addresses at `m/84'/<0' mainnet | 1' elsewhere>'/<account>'/<0 receive |
//! 1 change>/<i>`, derived from the profile's recovery phrase with the same
//! BIP 32 as the HNS keys ("Bitcoin seed"), so any BIP 84 wallet restores it
//! from the same phrase and passphrase.
//!
//! The account's extended public key is stored (`btc_accounts`), so
//! addresses are derived without the phrase; private keys only at a Commit
//! or a withdrawal, from the unlocked session.

use bitcoin::absolute::LockTime;
use bitcoin::hashes::Hash;
use bitcoin::secp256k1::{self as bsecp, Message, Secp256k1};
use bitcoin::sighash::{EcdsaSighashType, SighashCache};
use bitcoin::transaction::Version;
use bitcoin::{
    Address, Amount, CompressedPublicKey, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut,
    Witness,
};

use crate::error::AppError;
use crate::noncustodial::exchange::btc_htlc::btc_network;
use crate::noncustodial::hd::{ExtendedPrivKey, ExtendedPubKey, HARDENED_OFFSET};
use crate::noncustodial::network::Network;

pub const BRANCH_RECEIVE: u32 = 0;
pub const BRANCH_CHANGE: u32 = 1;

/// The account of the phrase that is the wallet's BTC account: always the
/// first one (`m/84'/coin'/0'`), whatever the profile's HNS account index
/// is, so any Bitcoin wallet restores it from the phrase. The stored public
/// key, the addresses and every signature of the account use this number.
pub const ACCOUNT: u32 = 0;

/// How many unused addresses past the last used one are scanned (BIP 44's
/// gap limit).
pub const GAP_LIMIT: u32 = 20;

/// The BIP 84 coin type: 0' on mainnet, 1' on every test network.
pub fn coin_type(network: Network) -> u32 {
    match network {
        Network::Main => 0,
        _ => 1,
    }
}

/// `m/84'/coin'/account'`.
pub fn account_path(network: Network, account: u32) -> Result<[u32; 3], AppError> {
    if account >= HARDENED_OFFSET {
        return Err(AppError::InvalidInput(format!(
            "account {account} is out of range for the BTC account"
        )));
    }
    Ok([
        HARDENED_OFFSET + 84,
        HARDENED_OFFSET + coin_type(network),
        HARDENED_OFFSET + account,
    ])
}

/// The account's extended public key, from the unlocked master key.
pub fn account_xpub(
    master: &ExtendedPrivKey,
    network: Network,
    account: u32,
) -> Result<ExtendedPubKey, AppError> {
    let node = master.derive_path(&account_path(network, account)?)?;
    Ok(ExtendedPubKey::from_priv(&node))
}

/// The P2WPKH address at `branch/index` below the account xpub.
pub fn address_at(
    xpub: &ExtendedPubKey,
    network: Network,
    branch: u32,
    index: u32,
) -> Result<Address, AppError> {
    if branch > BRANCH_CHANGE || index >= HARDENED_OFFSET {
        return Err(AppError::InvalidInput(
            "not a BTC account address path".into(),
        ));
    }
    let child = xpub.derive_path(&[branch, index])?;
    p2wpkh(network, &child.compressed_pubkey())
}

pub fn p2wpkh(network: Network, pubkey: &[u8; 33]) -> Result<Address, AppError> {
    let pk = CompressedPublicKey::from_slice(pubkey)
        .map_err(|e| AppError::Crypto(format!("not a compressed public key: {e}")))?;
    Ok(Address::p2wpkh(&pk, btc_network(network)))
}

/// The private key of `branch/index`, from the unlocked master key.
fn key_at(
    master: &ExtendedPrivKey,
    network: Network,
    account: u32,
    branch: u32,
    index: u32,
) -> Result<bsecp::SecretKey, AppError> {
    let mut path = account_path(network, account)?.to_vec();
    path.extend([branch, index]);
    let child = master.derive_path(&path)?;
    bsecp::SecretKey::from_slice(&child.secret.secret_bytes())
        .map_err(|e| AppError::Crypto(format!("key conversion: {e}")))
}

/// One coin of the account, as the Bitcoin source reports it, with the path
/// of the address that holds it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BtcCoin {
    pub txid: String,
    pub vout: u32,
    pub value: u64,
    pub branch: u32,
    pub index: u32,
    /// `None` when the source says unconfirmed.
    pub height: Option<u64>,
}

/// vbytes of a P2WPKH spend's input (BIP 141 weight 272 / 4 rounded up,
/// with a 72-byte signature), and of the transaction's fixed part.
const P2WPKH_INPUT_VBYTES: u64 = 68;
const TX_OVERHEAD_VBYTES: u64 = 11;

fn output_vbytes(spk: &ScriptBuf) -> u64 {
    8 + 1 + spk.len() as u64
}

/// A spend of account coins, before signing.
#[derive(Debug, Clone)]
pub struct Funding {
    pub tx: Transaction,
    pub coins: Vec<BtcCoin>,
    pub fee: u64,
    pub change: u64,
}

/// Pay `outputs` from `coins` (largest first, confirmed coins only) at
/// `rate` sat/vbyte, with change to `change_to` when it is above dust. The
/// fee is sized for the signed transaction (signatures at most 72 bytes, so
/// it is never short).
pub fn fund(
    coins: &[BtcCoin],
    outputs: Vec<TxOut>,
    change_to: &Address,
    rate: u64,
) -> Result<Funding, AppError> {
    let mut usable: Vec<&BtcCoin> = coins.iter().filter(|c| c.height.is_some()).collect();
    usable.sort_by(|a, b| {
        b.value
            .cmp(&a.value)
            .then(a.txid.cmp(&b.txid))
            .then(a.vout.cmp(&b.vout))
    });
    let want: u64 = outputs
        .iter()
        .map(|o| o.value.to_sat())
        .try_fold(0u64, |a, v| a.checked_add(v))
        .ok_or_else(|| AppError::InvalidInput("amount overflows".into()))?;
    let change_spk = change_to.script_pubkey();
    let dust = change_spk.minimal_non_dust().to_sat();
    let outs_vbytes: u64 = outputs
        .iter()
        .map(|o| output_vbytes(&o.script_pubkey))
        .sum();
    let mut have = 0u64;
    let mut taken = Vec::new();
    for c in usable {
        taken.push(c.clone());
        have = have
            .checked_add(c.value)
            .ok_or_else(|| AppError::InvalidInput("amount overflows".into()))?;
        let base = TX_OVERHEAD_VBYTES + outs_vbytes + P2WPKH_INPUT_VBYTES * taken.len() as u64;
        let fee_no_change = base * rate;
        let fee_change = (base + output_vbytes(&change_spk)) * rate;
        let Some(left) = have.checked_sub(want) else {
            continue;
        };
        let (fee, change) = if left >= fee_change + dust {
            (fee_change, left - fee_change)
        } else if left >= fee_no_change {
            (left, 0) // what is left below dust goes to the fee
        } else {
            continue;
        };
        let mut out = outputs.clone();
        if change > 0 {
            out.push(TxOut {
                value: Amount::from_sat(change),
                script_pubkey: change_spk.clone(),
            });
        }
        let tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: taken
                .iter()
                .map(|c| {
                    Ok(TxIn {
                        previous_output: OutPoint {
                            txid: crate::noncustodial::exchange::btc_htlc::parse_txid(&c.txid)?,
                            vout: c.vout,
                        },
                        script_sig: ScriptBuf::new(),
                        sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                        witness: Witness::new(),
                    })
                })
                .collect::<Result<_, AppError>>()?,
            output: out,
        };
        return Ok(Funding {
            tx,
            coins: taken,
            fee,
            change,
        });
    }
    Err(AppError::InvalidInput(format!(
        "not enough confirmed BTC: need {} sats plus the fee, have {} sats",
        want, have
    )))
}

/// Sign every input of `f` with the account's keys (BIP 143, SIGHASH_ALL).
/// Each coin's address is re-derived and must be the one the coin is at:
/// the key signs only its own coin.
pub fn sign_funding(
    f: &Funding,
    master: &ExtendedPrivKey,
    network: Network,
    account: u32,
) -> Result<Transaction, AppError> {
    let secp = Secp256k1::new();
    let mut tx = f.tx.clone();
    let mut sigs = Vec::with_capacity(f.coins.len());
    {
        let mut cache = SighashCache::new(&tx);
        for (i, c) in f.coins.iter().enumerate() {
            let key = key_at(master, network, account, c.branch, c.index)?;
            let pk = CompressedPublicKey(bsecp::PublicKey::from_secret_key(&secp, &key));
            let spk = ScriptBuf::new_p2wpkh(&pk.wpubkey_hash());
            let sighash = cache
                .p2wpkh_signature_hash(i, &spk, Amount::from_sat(c.value), EcdsaSighashType::All)
                .map_err(|e| AppError::Crypto(format!("sighash: {e}")))?;
            let sig = secp.sign_ecdsa(&Message::from_digest(sighash.to_byte_array()), &key);
            let mut der = sig.serialize_der().to_vec();
            der.push(EcdsaSighashType::All as u8);
            sigs.push((der, pk.to_bytes()));
        }
    }
    for (i, (sig, pk)) in sigs.into_iter().enumerate() {
        tx.input[i].witness = Witness::from_slice(&[sig, pk.to_vec()]);
    }
    Ok(tx)
}

/// The `scriptPubKey` of the account address at `branch/index`.
pub fn script_pubkey_at(
    xpub: &ExtendedPubKey,
    network: Network,
    branch: u32,
    index: u32,
) -> Result<ScriptBuf, AppError> {
    Ok(address_at(xpub, network, branch, index)?.script_pubkey())
}
