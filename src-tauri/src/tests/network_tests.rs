//! Tests for `crate::noncustodial::network` — pure Network functions.

use crate::noncustodial::network::{NameParams, Network};

// ── address_hrp ──────────────────────────────────────────────────────

#[test]
fn test_address_hrp_all_variants() {
    assert_eq!(Network::Main.address_hrp(), "hs");
    assert_eq!(Network::Testnet.address_hrp(), "ts");
    assert_eq!(Network::Regtest.address_hrp(), "rs");
    assert_eq!(Network::Simnet.address_hrp(), "ss");
}

// ── coin_type ────────────────────────────────────────────────────────

#[test]
fn test_coin_type_all_variants() {
    assert_eq!(Network::Main.coin_type(), 5353);
    assert_eq!(Network::Testnet.coin_type(), 5354);
    assert_eq!(Network::Regtest.coin_type(), 5355);
    assert_eq!(Network::Simnet.coin_type(), 5356);
}

// ── xprv_version ─────────────────────────────────────────────────────

#[test]
fn test_xprv_version_all_variants() {
    // All networks use the same xprv version in hsd
    assert_eq!(Network::Main.xprv_version(), 0x0488_ade4);
    assert_eq!(Network::Testnet.xprv_version(), 0x0488_ade4);
    assert_eq!(Network::Regtest.xprv_version(), 0x0488_ade4);
    assert_eq!(Network::Simnet.xprv_version(), 0x0488_ade4);
}

// ── xpub_version ─────────────────────────────────────────────────────

#[test]
fn test_xpub_version_all_variants() {
    assert_eq!(Network::Main.xpub_version(), 0x0488_b21e);
    assert_eq!(Network::Testnet.xpub_version(), 0x0488_b21e);
    assert_eq!(Network::Regtest.xpub_version(), 0x0488_b21e);
    assert_eq!(Network::Simnet.xpub_version(), 0x0488_b21e);
}

// ── as_str ───────────────────────────────────────────────────────────

#[test]
fn test_as_str_all_variants() {
    assert_eq!(Network::Main.as_str(), "main");
    assert_eq!(Network::Testnet.as_str(), "testnet");
    assert_eq!(Network::Regtest.as_str(), "regtest");
    assert_eq!(Network::Simnet.as_str(), "simnet");
}

// ── from_str_opt ─────────────────────────────────────────────────────

#[test]
fn test_from_str_opt_valid() {
    assert_eq!(Network::from_str_opt("main"), Some(Network::Main));
    assert_eq!(Network::from_str_opt("mainnet"), Some(Network::Main));
    assert_eq!(Network::from_str_opt("testnet"), Some(Network::Testnet));
    assert_eq!(Network::from_str_opt("regtest"), Some(Network::Regtest));
    assert_eq!(Network::from_str_opt("simnet"), Some(Network::Simnet));
}

#[test]
fn test_from_str_opt_invalid() {
    assert_eq!(Network::from_str_opt(""), None);
    assert_eq!(Network::from_str_opt("Main"), None);
    assert_eq!(Network::from_str_opt("TESTNET"), None);
    assert_eq!(Network::from_str_opt("bitcoin"), None);
    assert_eq!(Network::from_str_opt("main "), None);
}

// ── Default ──────────────────────────────────────────────────────────

#[test]
fn test_default_is_main() {
    assert_eq!(Network::default(), Network::Main);
}

// ── name_params ──────────────────────────────────────────────────────

#[test]
fn test_name_params_mainnet() {
    let p = Network::Main.name_params();
    assert_eq!(p.tree_interval, 36);
    assert_eq!(p.bidding_period, 720);
    assert_eq!(p.reveal_period, 1440);
    assert_eq!(p.renewal_window, 105_120);
    assert_eq!(p.transfer_lockup, 288);
    assert_eq!(p.revocation_delay, 2016);
    assert_eq!(p.renewal_maturity, 4320);
}

#[test]
fn test_name_params_testnet() {
    let p = Network::Testnet.name_params();
    assert_eq!(p.tree_interval, 36);
    assert_eq!(p.bidding_period, 144);
    assert_eq!(p.reveal_period, 288);
    assert_eq!(p.renewal_window, 4320);
    assert_eq!(p.transfer_lockup, 288);
    assert_eq!(p.revocation_delay, 576);
    assert_eq!(p.renewal_maturity, 144);
}

#[test]
fn test_name_params_regtest() {
    let p = Network::Regtest.name_params();
    assert_eq!(p.tree_interval, 5);
    assert_eq!(p.bidding_period, 5);
    assert_eq!(p.reveal_period, 10);
    assert_eq!(p.renewal_window, 5000);
    assert_eq!(p.transfer_lockup, 10);
    assert_eq!(p.revocation_delay, 50);
    assert_eq!(p.renewal_maturity, 50);
}

#[test]
fn test_name_params_simnet() {
    let p = Network::Simnet.name_params();
    assert_eq!(p.tree_interval, 2);
    assert_eq!(p.bidding_period, 25);
    assert_eq!(p.reveal_period, 50);
    assert_eq!(p.renewal_window, 2500);
    assert_eq!(p.transfer_lockup, 5);
    assert_eq!(p.revocation_delay, 25);
    assert_eq!(p.renewal_maturity, 25);
}

// ── roundtrip as_str / from_str_opt ──────────────────────────────────

#[test]
fn test_as_str_from_str_roundtrip() {
    for net in [Network::Main, Network::Testnet, Network::Regtest, Network::Simnet] {
        let s = net.as_str();
        assert_eq!(Network::from_str_opt(s), Some(net));
    }
}

// ── NameParams struct is Debug + Clone + Copy + PartialEq ────────────

#[test]
fn test_name_params_derives() {
    let p = Network::Main.name_params();
    let p2 = p; // Copy
    assert_eq!(p, p2); // PartialEq
    let _ = format!("{:?}", p); // Debug
    let _clone = p.clone(); // Clone
}
