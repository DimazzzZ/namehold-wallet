//! The BTC account (spec R15): BIP 84 test vectors, and funding a lock from
//! account coins, checked by Bitcoin Core's interpreter.

use bitcoin::consensus::validation::verify_transaction;
use bitcoin::{Amount, TxOut};

use crate::noncustodial::exchange::btc_account::{self, BtcCoin};
use crate::noncustodial::hd::{seed_from_mnemonic, ExtendedPrivKey};
use crate::noncustodial::network::Network;

/// The public BIP 39 / BIP 84 test phrase.
const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

fn master() -> ExtendedPrivKey {
    ExtendedPrivKey::from_seed(&seed_from_mnemonic(PHRASE, "").unwrap()).unwrap()
}

#[test]
fn bip84_test_vectors() {
    // BIP 84 "Test vectors": m/84'/0'/0'/0/0, m/84'/0'/0'/0/1, m/84'/0'/0'/1/0.
    let xpub = btc_account::account_xpub(&master(), Network::Main, 0).unwrap();
    let a = |b, i| {
        btc_account::address_at(&xpub, Network::Main, b, i)
            .unwrap()
            .to_string()
    };
    assert_eq!(a(0, 0), "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu");
    assert_eq!(a(0, 1), "bc1qnjg0jd8228aq7egyzacy8cys3knf9xvrerkf9g");
    assert_eq!(a(1, 0), "bc1q8c6fshw2dlwun7ekn9qwf37cu2rn755upcp6el");
}

#[test]
fn test_networks_use_coin_type_one() {
    assert_eq!(btc_account::coin_type(Network::Main), 0);
    for n in [Network::Testnet, Network::Regtest, Network::Simnet] {
        assert_eq!(btc_account::coin_type(n), 1);
    }
    let xpub = btc_account::account_xpub(&master(), Network::Regtest, 0).unwrap();
    let addr = btc_account::address_at(&xpub, Network::Regtest, 0, 0)
        .unwrap()
        .to_string();
    assert!(addr.starts_with("bcrt1q"), "{addr}");
}

#[test]
fn a_path_off_the_account_is_refused() {
    let xpub = btc_account::account_xpub(&master(), Network::Main, 0).unwrap();
    assert!(btc_account::address_at(&xpub, Network::Main, 2, 0).is_err());
    assert!(btc_account::account_path(Network::Main, 1 << 31).is_err());
}

fn coin(
    xpub_net: Network,
    branch: u32,
    index: u32,
    value: u64,
    n: u8,
    height: Option<u64>,
) -> (BtcCoin, TxOut) {
    let xpub = btc_account::account_xpub(&master(), xpub_net, 0).unwrap();
    let spk = btc_account::script_pubkey_at(&xpub, xpub_net, branch, index).unwrap();
    (
        BtcCoin {
            txid: bitcoin::Txid::from_raw_hash(bitcoin::hashes::Hash::from_byte_array([n; 32]))
                .to_string(),
            vout: u32::from(n),
            value,
            branch,
            index,
            height,
        },
        TxOut {
            value: Amount::from_sat(value),
            script_pubkey: spk,
        },
    )
}

#[test]
fn funding_signs_only_account_coins_and_passes_consensus() {
    let net = Network::Regtest;
    let xpub = btc_account::account_xpub(&master(), net, 0).unwrap();
    let (c1, o1) = coin(net, 0, 3, 40_000, 1, Some(100));
    let (c2, o2) = coin(net, 1, 0, 70_000, 2, Some(101));
    let dest = TxOut {
        value: Amount::from_sat(90_000),
        script_pubkey: btc_account::script_pubkey_at(&xpub, net, 0, 9).unwrap(),
    };
    let change = btc_account::address_at(&xpub, net, 1, 5).unwrap();
    let f = btc_account::fund(&[c1.clone(), c2.clone()], vec![dest], &change, 3).unwrap();
    assert_eq!(f.coins.len(), 2);
    let tx = btc_account::sign_funding(&f, &master(), net, 0).unwrap();
    let spent = |o: &bitcoin::OutPoint| {
        [(&c1, &o1), (&c2, &o2)]
            .iter()
            .find(|(c, _)| c.txid == o.txid.to_string() && c.vout == o.vout)
            .map(|(_, out)| (*out).clone())
    };
    verify_transaction(&tx, spent).unwrap();
    // The fee pays at least the asked rate on the signed transaction.
    assert!(
        f.fee >= 3 * tx.vsize() as u64,
        "fee {} vsize {}",
        f.fee,
        tx.vsize()
    );
    let outs: u64 = tx.output.iter().map(|o| o.value.to_sat()).sum();
    assert_eq!(outs + f.fee, 110_000);
}

#[test]
fn unconfirmed_coins_are_not_spent() {
    let net = Network::Regtest;
    let xpub = btc_account::account_xpub(&master(), net, 0).unwrap();
    let (c, _) = coin(net, 0, 0, 1_000_000, 1, None);
    let dest = TxOut {
        value: Amount::from_sat(10_000),
        script_pubkey: btc_account::script_pubkey_at(&xpub, net, 0, 9).unwrap(),
    };
    let change = btc_account::address_at(&xpub, net, 1, 0).unwrap();
    let err = btc_account::fund(&[c], vec![dest], &change, 1).unwrap_err();
    assert!(
        err.to_string().contains("not enough confirmed BTC"),
        "{err}"
    );
}

#[test]
fn change_below_dust_goes_to_the_fee() {
    let net = Network::Regtest;
    let xpub = btc_account::account_xpub(&master(), net, 0).unwrap();
    let (c, _) = coin(net, 0, 0, 10_200, 1, Some(5));
    let dest = TxOut {
        value: Amount::from_sat(10_000),
        script_pubkey: btc_account::script_pubkey_at(&xpub, net, 0, 9).unwrap(),
    };
    let change = btc_account::address_at(&xpub, net, 1, 0).unwrap();
    let f = btc_account::fund(&[c], vec![dest], &change, 1).unwrap();
    assert_eq!(f.change, 0);
    assert_eq!(f.fee, 200);
    assert_eq!(f.tx.output.len(), 1);
}
