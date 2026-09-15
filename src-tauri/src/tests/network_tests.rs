//! Tests for `crate::noncustodial::network` — pure Network functions.

use crate::noncustodial::network::{
    network_check, network_name_matches, NameParams, Network, BLOCKS_PER_DAY,
};

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

// ── coinbase_maturity ────────────────────────────────────────────────

/// Blocks a coinbase output must age before it can be spent. Values mirror
/// hsd `lib/protocol/networks.js` (`main`/`testnet` 100, `regtest` 2,
/// `simnet` 6); a wrong value here either blocks spendable funds or builds a
/// tx the node rejects with `bad-txns-premature-spend-of-coinbase`.
#[test]
fn test_coinbase_maturity_all_variants() {
    assert_eq!(Network::Main.coinbase_maturity(), 100);
    assert_eq!(Network::Testnet.coinbase_maturity(), 100);
    assert_eq!(Network::Regtest.coinbase_maturity(), 2);
    assert_eq!(Network::Simnet.coinbase_maturity(), 6);
}

// ── expiring_soon_threshold_days ─────────────────────────────────────

/// Mainnet keeps its established 30-day warning; every other network gets the
/// same SHARE of its own (much shorter) renewal window. A flat 30 days is the
/// entire testnet window and more than the simnet one, so the alarm would be
/// permanently on there.
#[test]
fn test_expiring_soon_threshold_scales_with_the_renewal_window() {
    assert!((Network::Main.expiring_soon_threshold_days() - 30.0).abs() < 1e-9);
    for n in [Network::Testnet, Network::Regtest, Network::Simnet] {
        let window_days = n.name_params().renewal_window as f64 / BLOCKS_PER_DAY;
        let threshold = n.expiring_soon_threshold_days();
        assert!(
            threshold > 0.0 && threshold < window_days,
            "{n:?}: threshold {threshold} must fall inside its {window_days}-day window"
        );
        assert!(
            threshold < 30.0,
            "{n:?}: a mainnet-sized warning would cover the whole lease"
        );
    }
}

// ── has_wall_clock_block_timing ──────────────────────────────────────

/// Only chains with miners produce blocks on a schedule. Regtest and simnet
/// mine on demand, so ageing a stored height by elapsed time invents blocks.
#[test]
fn test_has_wall_clock_block_timing_all_variants() {
    assert!(Network::Main.has_wall_clock_block_timing());
    assert!(Network::Testnet.has_wall_clock_block_timing());
    assert!(!Network::Regtest.has_wall_clock_block_timing());
    assert!(!Network::Simnet.has_wall_clock_block_timing());
}

// ── default_rpc_port ─────────────────────────────────────────────────

/// hsd listens on a different RPC port per network (`networks.js` `rpcPort`).
/// `start_hsd` passes the network flag, so these are the ports the app must
/// talk to; a wrong value here makes a local node unreachable.
#[test]
fn test_default_rpc_port_all_variants() {
    assert_eq!(Network::Main.default_rpc_port(), 12037);
    assert_eq!(Network::Testnet.default_rpc_port(), 13037);
    assert_eq!(Network::Regtest.default_rpc_port(), 14037);
    assert_eq!(Network::Simnet.default_rpc_port(), 15037);
}

#[test]
fn test_default_rpc_url_is_loopback_on_the_networks_port() {
    assert_eq!(Network::Main.default_rpc_url(), "http://127.0.0.1:12037");
    assert_eq!(Network::Regtest.default_rpc_url(), "http://127.0.0.1:14037");
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

// ── default_explorer_base_url ────────────────────────────────────────

/// G2: only mainnet has a documented public HNSFans explorer. Non-mainnet
/// networks return None so callers disable the explorer fallback rather than
/// silently querying mainnet — the concrete miss that used to make a fresh
/// testnet wallet look empty.
#[test]
fn test_default_explorer_base_url_mainnet_only() {
    assert_eq!(
        Network::Main.default_explorer_base_url(),
        Some("https://e.hnsfans.com")
    );
    assert_eq!(Network::Testnet.default_explorer_base_url(), None);
    assert_eq!(Network::Regtest.default_explorer_base_url(), None);
    assert_eq!(Network::Simnet.default_explorer_base_url(), None);
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
    for net in [
        Network::Main,
        Network::Testnet,
        Network::Regtest,
        Network::Simnet,
    ] {
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
    let _clone: NameParams = p; // Clone (Copy implies Clone; avoid clippy::clone_on_copy)
}

// ── network_name_matches / network_check ─────────────────────────────

#[test]
fn network_name_matches_canonicalizes_mainnet_both_sides() {
    // Exact matches (each canonical form pairs with itself).
    assert!(network_name_matches("main", "main"));
    assert!(network_name_matches("testnet", "testnet"));
    assert!(network_name_matches("regtest", "regtest"));
    assert!(network_name_matches("simnet", "simnet"));
    // `mainnet` ↔ `main` normalization (the profile-schema vs hsd-chain gap).
    assert!(network_name_matches("mainnet", "main"));
    assert!(network_name_matches("main", "mainnet"));
    assert!(network_name_matches("mainnet", "mainnet"));
    // Non-matching canonical forms — every cross-pair.
    assert!(!network_name_matches("main", "testnet"));
    assert!(!network_name_matches("mainnet", "testnet"));
    assert!(!network_name_matches("testnet", "regtest"));
    assert!(!network_name_matches("regtest", "simnet"));
    assert!(!network_name_matches("main", "regtest"));
    // Unknown strings only match themselves (no canonicalization outside `mainnet`).
    assert!(network_name_matches("weirdnet", "weirdnet"));
    assert!(!network_name_matches("weirdnet", "main"));
    // Two DIFFERENT unknown strings must not compare equal — this is why the
    // helper is string-based and not `Network::from_str_opt(a) == from_str_opt(b)`.
    assert!(!network_name_matches("weirdnet", "othernet"));
    // Empty vs known network is a mismatch (defensive — no accidental match).
    assert!(!network_name_matches("", "main"));
}

#[test]
fn network_name_matches_same_network() {
    assert!(network_name_matches("main", "main"));
    assert!(network_name_matches("mainnet", "main"));
    assert!(network_name_matches("main", "mainnet"));
    assert!(network_name_matches("mainnet", "mainnet"));
    assert!(network_name_matches("testnet", "testnet"));
    assert!(network_name_matches("regtest", "regtest"));
    assert!(network_name_matches("simnet", "simnet"));
}

#[test]
fn network_name_matches_different_network() {
    assert!(!network_name_matches("mainnet", "regtest"));
    assert!(!network_name_matches("main", "regtest"));
    assert!(!network_name_matches("mainnet", "testnet"));
    assert!(!network_name_matches("testnet", "regtest"));
    assert!(!network_name_matches("regtest", "main"));
}

#[test]
fn network_check_compares_only_when_both_sides_are_known() {
    // Both known → a real answer.
    assert_eq!(network_check(Some("main"), Some("main")), Some(true));
    assert_eq!(network_check(Some("mainnet"), Some("main")), Some(true));
    assert_eq!(network_check(Some("main"), Some("testnet")), Some(false));
    // Either side unknown → "cannot validate", never a mismatch.
    assert_eq!(network_check(None, Some("main")), None);
    assert_eq!(network_check(Some("main"), None), None);
    assert_eq!(network_check(None, None), None);
}
