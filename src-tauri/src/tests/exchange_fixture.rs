//! A trade between two test wallets, built with the real Commit code: real
//! keys from the public BIP 39 test phrase (two passphrases, two wallets),
//! real presigned transactions. The engine, watchtower and Commit tests
//! share it.

use bitcoin::{Amount, TxOut};

use crate::noncustodial::exchange::btc_account::{self, BtcCoin};
use crate::noncustodial::exchange::commit::{self, BuyerCommit, SellerCommit};
use crate::noncustodial::exchange::engine::{
    BtcLockSeen, BtcSeen, HnsLockSeen, HnsSeen, Holdings, LockRef, SpendSeen,
};
use crate::noncustodial::exchange::hns_htlc;
use crate::noncustodial::exchange::keys;
use crate::noncustodial::exchange::policy::Policy;
use crate::noncustodial::exchange::relay_msg;
use crate::noncustodial::exchange::terms::{Terms, TermsDraft};
use crate::noncustodial::hd::{seed_from_mnemonic, ExtendedPrivKey};
use crate::noncustodial::network::Network;
use crate::noncustodial::tx::{Covenant, Input, Outpoint, Output, Transaction};

pub const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

pub const NET: Network = Network::Regtest;
pub const START: u64 = 1_800_000_000;
pub const HNS_AMOUNT: u64 = 5_000 * 1_000_000; // 5000 HNS
pub const PRICE: u64 = 20_000; // 20 sats per HNS → 100 000 sats

pub fn master(passphrase: &str) -> ExtendedPrivKey {
    ExtendedPrivKey::from_seed(&seed_from_mnemonic(PHRASE, passphrase).unwrap()).unwrap()
}

pub fn buyer_master() -> ExtendedPrivKey {
    master("buyer")
}

pub fn seller_master() -> ExtendedPrivKey {
    master("seller")
}

pub fn policy() -> Policy {
    Policy::for_network(NET)
}

pub fn hns_address(m: &ExtendedPrivKey, index: u32) -> String {
    let child = m
        .derive_path(&crate::noncustodial::hd::bip44_path(NET, 0, 0, index))
        .unwrap();
    crate::noncustodial::address::address_from_pubkey(NET, &child.compressed_pubkey()).unwrap()
}

pub fn btc_address(m: &ExtendedPrivKey, branch: u32, index: u32) -> bitcoin::Address {
    let xpub = btc_account::account_xpub(m, NET, 0).unwrap();
    btc_account::address_at(&xpub, NET, branch, index).unwrap()
}

pub struct Trade {
    pub terms: Terms,
    pub buyer: BuyerCommit,
    pub seller: SellerCommit,
    pub hns_funding: Transaction,
    pub btc_coin: (BtcCoin, TxOut),
}

/// The terms both wallets agree on, trade index 0 on each side.
pub fn terms_at(start: u64) -> Terms {
    let b = keys::trade_keys(&buyer_master(), NET, 0, 0).unwrap();
    let s = keys::trade_keys(&seller_master(), NET, 0, 0).unwrap();
    let br = relay_msg::new_relay_key();
    let sr = relay_msg::new_relay_key();
    let pk = |k: &secp256k1::SecretKey| {
        secp256k1::PublicKey::from_secret_key(&secp256k1::Secp256k1::new(), k).serialize()
    };
    Terms::from_draft(
        &TermsDraft {
            network: NET.as_str(),
            offer_id: &"ab".repeat(32),
            trade_id: &"cd".repeat(16),
            hns_amount: HNS_AMOUNT,
            price_msat_per_hns: PRICE,
            hash: b.hash(),
            seller_hns_refund_pubkey: s.hns_pubkey,
            buyer_hns_claim_pubkey: b.hns_pubkey,
            buyer_btc_refund_pubkey: b.btc_pubkey,
            seller_btc_claim_pubkey: s.btc_pubkey,
            seller_relay_pubkey: pk(&sr),
            buyer_relay_pubkey: pk(&br),
            start,
        },
        &policy(),
    )
    .unwrap()
}

/// A Buyer's confirmed coin in its BTC account, and the output it spends.
pub fn btc_coin(value: u64) -> (BtcCoin, TxOut) {
    let m = buyer_master();
    let xpub = btc_account::account_xpub(&m, NET, 0).unwrap();
    let spk = btc_account::script_pubkey_at(&xpub, NET, 0, 0).unwrap();
    (
        BtcCoin {
            txid: bitcoin::Txid::from_raw_hash(bitcoin::hashes::Hash::from_byte_array([0x5a; 32]))
                .to_string(),
            vout: 0,
            value,
            branch: 0,
            index: 0,
            height: Some(10),
        },
        TxOut {
            value: Amount::from_sat(value),
            script_pubkey: spk,
        },
    )
}

/// The Seller's HNS funding: one wallet coin in, the lock and change out.
/// Unsigned: the engine and the Commit only read its outputs.
pub fn hns_funding(terms: &Terms) -> Transaction {
    let hp = terms.hns_params().unwrap();
    let mut tx = Transaction::new();
    tx.inputs.push(Input::new(Outpoint {
        hash: [0x11; 32],
        index: 0,
    }));
    tx.outputs
        .push(hns_htlc::lock_output(&hp, terms.hns_amount));
    tx.outputs.push(Output {
        value: 1_000_000,
        address: crate::noncustodial::tx::output_address_from_string(
            NET,
            &hns_address(&seller_master(), 1),
        )
        .unwrap(),
        covenant: Covenant::default(),
    });
    tx
}

pub fn trade() -> Trade {
    trade_at(START)
}

pub fn trade_at(start: u64) -> Trade {
    let terms = terms_at(start);
    let p = policy();
    let bm = buyer_master();
    let sm = seller_master();
    let coin = btc_coin(1_000_000);
    let buyer = commit::buyer_commit(&commit::BuyerCommitInput {
        master: &bm,
        account: 0,
        trade_index: 0,
        terms: &terms,
        policy: &p,
        btc_coins: std::slice::from_ref(&coin.0),
        btc_change_to: &btc_address(&bm, 1, 0),
        btc_refund_to: &btc_address(&bm, 0, 1),
        btc_rate: 2,
        hns_rate: 1,
        hns_to: &hns_address(&bm, 0),
    })
    .unwrap();
    let funding = hns_funding(&terms);
    let seller = commit::seller_commit(&commit::SellerCommitInput {
        master: &sm,
        account: 0,
        trade_index: 0,
        terms: &terms,
        policy: &p,
        hns_funding: &funding,
        btc_lock: &buyer.btc_lock,
        hns_rate: 1,
        hns_refund_to: &hns_address(&sm, 2),
        btc_rate: 2,
        btc_claim_to: &btc_address(&sm, 0, 0),
    })
    .unwrap();
    Trade {
        terms,
        buyer,
        seller,
        hns_funding: funding,
        btc_coin: coin,
    }
}

impl Trade {
    pub fn buyer(&self) -> Holdings {
        self.buyer.holdings.clone()
    }

    pub fn seller(&self) -> Holdings {
        self.seller.holdings.clone()
    }

    pub fn secret(&self) -> [u8; 32] {
        self.buyer.holdings.secret.unwrap()
    }

    pub fn hns_lock(&self) -> LockRef {
        self.seller.hns_lock.clone()
    }

    /// The HNS chain with the lock confirmed `confs` times and unspent.
    pub fn hns_locked(&self, mtp: u64, confs: u32) -> HnsSeen {
        HnsSeen {
            mtp,
            lock: Some(HnsLockSeen {
                outpoint: self.hns_lock(),
                confirmations: confs,
                spend: None,
            }),
            mismatched_payment: false,
        }
    }

    pub fn hns_claimed(&self, mtp: u64, claim_confs: u32) -> HnsSeen {
        let mut v = self.hns_locked(mtp, 10);
        v.lock.as_mut().unwrap().spend = Some(SpendSeen {
            txid: "ee".repeat(32),
            confirmations: claim_confs,
            secret: Some(self.secret()),
        });
        v
    }

    pub fn hns_refunded(&self, mtp: u64, confs: u32) -> HnsSeen {
        let mut v = self.hns_locked(mtp, 10);
        v.lock.as_mut().unwrap().spend = Some(SpendSeen {
            txid: "ff".repeat(32),
            confirmations: confs,
            secret: None,
        });
        v
    }

    pub fn hns_empty(mtp: u64) -> HnsSeen {
        HnsSeen {
            mtp,
            lock: None,
            mismatched_payment: false,
        }
    }

    pub fn btc_locked(&self, mtp: u64, confs: u32) -> BtcSeen {
        BtcSeen {
            mtp,
            lock: Some(BtcLockSeen {
                outpoint: self.buyer.btc_lock.clone(),
                confirmations: confs,
                matches: true,
                spend: None,
            }),
        }
    }

    pub fn btc_spent(&self, mtp: u64, confs: u32, claim: bool) -> BtcSeen {
        let mut v = self.btc_locked(mtp, 10);
        v.lock.as_mut().unwrap().spend = Some(SpendSeen {
            txid: "aa".repeat(32),
            confirmations: confs,
            secret: claim.then(|| self.secret()),
        });
        v
    }

    pub fn btc_empty(mtp: u64) -> BtcSeen {
        BtcSeen { mtp, lock: None }
    }

    /// The BTC lock output, for consensus checks.
    pub fn btc_lock_txout(&self) -> TxOut {
        self.buyer.funding.output[0].clone()
    }
}
