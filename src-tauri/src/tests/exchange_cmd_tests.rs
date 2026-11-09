//! The exchange's gates (spec R17, review M3) and its settings rows.

use crate::commands::exchange::{
    refusal, settings_rows, NEEDS_LOCAL_NODE, NEEDS_NODE, NEEDS_RECOVERY_PHRASE, NEEDS_SENDING_NODE,
};
use crate::exchange_io::env;
use crate::noncustodial::network::Network;
use crate::noncustodial::rpc::ChainSource;

#[test]
fn every_refusal_has_its_own_sentence() {
    assert_eq!(
        refusal("ledger_hardware", ChainSource::LocalNode, true),
        Some(NEEDS_RECOVERY_PHRASE)
    );
    assert_eq!(
        refusal("watch_only", ChainSource::LocalNode, true),
        Some(NEEDS_RECOVERY_PHRASE)
    );
    assert_eq!(
        refusal("mnemonic_hot", ChainSource::Explorer, true),
        Some(NEEDS_NODE)
    );
    assert_eq!(
        refusal("mnemonic_hot", ChainSource::RemoteNode, true),
        Some(NEEDS_LOCAL_NODE)
    );
    assert_eq!(
        refusal("mnemonic_hot", ChainSource::LocalNode, false),
        Some(NEEDS_SENDING_NODE)
    );
    assert_eq!(refusal("mnemonic_hot", ChainSource::LocalNode, true), None);
}

/// A "local" source whose URL is another machine is a remote node to the
/// gates (`rpc::send_source`), and so to the exchange: `node_refusal` judges
/// the effective source, the commands and the sync step both ask it.
#[test]
fn a_local_source_on_another_machine_is_refused_as_remote() {
    use super::mock_node_rpc::MockNodeRpc;
    use crate::exchange_io::env::node_refusal;
    let settings = std::collections::HashMap::new();
    let mut far = MockNodeRpc::new().with_source(ChainSource::LocalNode);
    far.node_url = "http://10.0.0.5:14037".into();
    assert_eq!(
        node_refusal("mnemonic_hot", &far, &settings),
        Some(NEEDS_LOCAL_NODE)
    );
    let mut here = MockNodeRpc::new().with_source(ChainSource::LocalNode);
    here.node_url = "http://127.0.0.1:14037".into();
    assert_eq!(node_refusal("mnemonic_hot", &here, &settings), None);
    assert_eq!(
        node_refusal("watch_only", &here, &settings),
        Some(NEEDS_RECOVERY_PHRASE)
    );
}

#[test]
fn the_bitcoind_login_is_shown_only_as_set_changed_or_cleared() {
    let old = env::validate(Network::Regtest, "", "", &[], None).unwrap();
    let new = env::validate(
        Network::Regtest,
        "https://relay.example",
        "",
        &["bitcoind+http://127.0.0.1:18443".into()],
        Some("u:secretpass"),
    )
    .unwrap();
    let rows = serde_json::to_string(&settings_rows(&old, &new)).unwrap();
    assert!(rows.contains("set"));
    assert!(!rows.contains("secretpass"), "{rows}");
    let changed = env::validate(
        Network::Regtest,
        "https://relay.example",
        "",
        &["bitcoind+http://127.0.0.1:18443".into()],
        Some("u:other"),
    )
    .unwrap();
    let rows = serde_json::to_string(&settings_rows(&new, &changed)).unwrap();
    assert!(
        rows.contains("changed") && !rows.contains("other"),
        "{rows}"
    );
}

#[test]
fn settings_refuse_cleartext_to_another_host_and_logins_in_urls() {
    assert!(env::validate(Network::Regtest, "http://relay.example", "", &[], None).is_err());
    assert!(env::validate(
        Network::Regtest,
        "",
        "",
        &["http://mempool.example/api".into()],
        None
    )
    .is_err());
    assert!(env::validate(
        Network::Regtest,
        "",
        "",
        &["bitcoind+http://u:p@127.0.0.1:18443".into()],
        None
    )
    .is_err());
    assert!(env::validate(
        Network::Regtest,
        "http://127.0.0.1:8787",
        "",
        &["bitcoind+http://127.0.0.1:18443".into()],
        None
    )
    .is_ok());
    assert!(env::validate(
        Network::Regtest,
        "",
        "",
        &vec!["https://a.example".to_string(); 6],
        None
    )
    .is_err());
}

#[test]
fn a_mainnet_lock_needs_two_sources_and_the_settings_say_so() {
    use crate::exchange_io::btc_observe::enough_sources;
    assert!(enough_sources(Network::Main, 1).is_err());
    assert!(enough_sources(Network::Testnet, 1).is_err());
    assert!(enough_sources(Network::Main, 2).is_ok());
    assert!(enough_sources(Network::Regtest, 1).is_ok());
    let one = vec!["https://a.example/api".to_string()];
    let e = env::validate(Network::Main, "", "", &one, None)
        .unwrap_err()
        .to_string();
    assert!(e.contains("at least 2"), "{e}");
    assert!(env::validate(Network::Regtest, "", "", &one, None).is_ok());
    let two = vec![
        "https://a.example/api".into(),
        "https://b.example/api".into(),
    ];
    assert!(env::validate(Network::Main, "", "", &two, None).is_ok());
    // An empty list is the defaults, which hold two.
    assert!(env::validate(Network::Main, "", "", &[], None).is_ok());
    assert_eq!(
        crate::exchange_io::btc_source::default_sources(Network::Main).len(),
        2
    );
}

#[test]
fn the_bitcoind_login_is_a_sensitive_setting_on_every_network() {
    for n in crate::noncustodial::network::Network::ALL {
        assert!(crate::security::is_sensitive_key(&env::key(
            env::BITCOIND_AUTH_KEY,
            n
        )));
    }
    assert!(!crate::security::is_sensitive_key(&env::key(
        env::RELAY_KEY,
        crate::noncustodial::network::Network::Main
    )));
}

#[test]
fn hns_amounts_cross_the_bridge_as_plain_decimals() {
    // QA finding: "1.000000 HNS" with its unit made every offer untakeable
    // in the UI, which parses digits only (src/lib/hnsAmount.ts).
    use crate::commands::exchange::hns_text;
    assert_eq!(hns_text(1_000_000), "1.000000");
    assert_eq!(hns_text(7_777_777), "7.777777");
    assert_eq!(hns_text(0), "0.000000");
    assert_eq!(hns_text(u64::MAX), "18446744073709.551615");
}

#[test]
fn get_settings_redacts_the_exchange_bitcoind_login() {
    // QA finding: the redaction listed sensitive keys by name and missed the
    // exchange's per-network login.
    let mut m = std::collections::HashMap::new();
    let k = env::key(
        env::BITCOIND_AUTH_KEY,
        crate::noncustodial::network::Network::Regtest,
    );
    m.insert(k.clone(), "u:secretpass".to_string());
    m.insert("node_rpc_api_key".to_string(), "abc".to_string());
    m.insert("close_to_tray".to_string(), "true".to_string());
    crate::commands::settings::redact(&mut m);
    assert!(!m.contains_key(&k));
    assert_eq!(
        m.get(&format!("__has_{k}")).map(String::as_str),
        Some("true")
    );
    assert!(!m.contains_key("node_rpc_api_key"));
    assert_eq!(m.get("close_to_tray").map(String::as_str), Some("true"));
    assert!(!serde_json::to_string(&m).unwrap().contains("secretpass"));
}

#[test]
fn every_event_has_a_notification_sentence() {
    use crate::exchange_io::job::{event_text, Event};
    for e in [
        Event::NeedsCommit {
            trade_id: "t".into(),
            deadline: 1_800_000_000,
        },
        Event::Revealed {
            trade_id: "t".into(),
            claim_by: 1_800_000_000,
        },
        Event::Completed {
            trade_id: "t".into(),
        },
        Event::Refunded {
            trade_id: "t".into(),
        },
        Event::NeedsAttention {
            trade_id: "t".into(),
        },
    ] {
        let (s, b) = event_text(&e);
        assert!(s.starts_with("Exchange: ") && !b.is_empty());
    }
    let (_, b) = event_text(&Event::Revealed {
        trade_id: "t".into(),
        claim_by: 1_800_000_000,
    });
    assert!(b.contains("2027-01-15"), "{b}");
}

#[test]
fn the_commit_windows_show_what_the_signed_transactions_do() {
    // Spec R16: amounts and destinations read from the transactions.
    use super::exchange_fixture as fx;
    use crate::noncustodial::exchange::{btc_htlc, hns_htlc, review};
    let t = fx::trade();
    let lock = btc_htlc::lock_address(fx::NET, &t.terms.btc_params().unwrap()).to_string();
    let b = review::buyer_rows(&t.buyer, &t.terms, &lock)
        .unwrap()
        .to_string();
    assert!(b.contains(&review::sats_text(t.buyer.funding.output[0].value.to_sat())));
    assert!(b.contains(&t.buyer.hns_claim.to_address));
    assert!(b.contains(&lock));
    assert!(b.contains("1 confirmation,"), "{b}");
    assert!(!b.contains("HNS HNS"), "{b}");
    let hlock = hns_htlc::lock_address(fx::NET, &t.terms.hns_params().unwrap()).unwrap();
    let s = review::seller_rows(&t.seller, &t.terms, 1234, &hlock)
        .unwrap()
        .to_string();
    assert!(s.contains("5000.000000 HNS"), "{s}");
    // R16: the deadline on the Seller's side too, not only the refund times.
    assert!(
        s.contains(&review::time_text(t.terms.reveal_deadline)),
        "{s}"
    );
    assert!(s.contains("0.001234 HNS"), "{s}");
    assert!(s.contains(&fx::btc_address(&fx::seller_master(), 0, 0).to_string()));
    assert!(!s.contains("HNS HNS"), "{s}");
    assert_eq!(review::confirmations_text(6), "6 confirmations");
}

/// What the window shows is what the user trusts: a Commit whose signed
/// transactions cannot be read is refused, never shown with a 0 or a blank
/// (CODING_STANDARDS "Fail closed on what the user is asked to trust").
#[test]
fn a_commit_whose_transactions_cannot_be_read_is_refused_not_shown() {
    use super::exchange_fixture as fx;
    use crate::noncustodial::exchange::{btc_htlc, hns_htlc, review};
    let refused = |e: crate::error::AppError| {
        let m = e.to_string();
        assert!(m.contains("nothing is signed"), "{m}");
        assert!(
            !m.contains('0'),
            "no number stands in for the missing one: {m}"
        );
    };
    let lock =
        |t: &fx::Trade| btc_htlc::lock_address(fx::NET, &t.terms.btc_params().unwrap()).to_string();
    let hlock =
        |t: &fx::Trade| hns_htlc::lock_address(fx::NET, &t.terms.hns_params().unwrap()).unwrap();

    let mut t = fx::trade();
    t.buyer.funding.output.clear();
    refused(review::buyer_rows(&t.buyer, &t.terms, &lock(&t)).unwrap_err());

    let mut t = fx::trade();
    t.buyer.btc_refunds.clear();
    refused(review::buyer_rows(&t.buyer, &t.terms, &lock(&t)).unwrap_err());

    let mut t = fx::trade();
    t.buyer.hns_claim.to_value = t.buyer.hns_claim.lock_value + 1;
    refused(review::buyer_rows(&t.buyer, &t.terms, &lock(&t)).unwrap_err());

    let mut t = fx::trade();
    t.seller.btc_claims.clear();
    refused(review::seller_rows(&t.seller, &t.terms, 1, &hlock(&t)).unwrap_err());

    let mut t = fx::trade();
    t.seller.hns_refund.outputs.clear();
    refused(review::seller_rows(&t.seller, &t.terms, 1, &hlock(&t)).unwrap_err());

    let mut t = fx::trade();
    t.seller.btc_claims[0].tx_hex = "zz".into();
    refused(review::seller_rows(&t.seller, &t.terms, 1, &hlock(&t)).unwrap_err());
}
