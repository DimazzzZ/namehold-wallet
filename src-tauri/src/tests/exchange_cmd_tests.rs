//! The exchange's gates (spec R17, review M3) and its settings rows.

use crate::commands::exchange::{
    refusal, settings_rows, NEEDS_LOCAL_NODE, NEEDS_NODE, NEEDS_RECOVERY_PHRASE, NEEDS_SENDING_NODE,
};
use crate::exchange_io::env;
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

#[test]
fn the_bitcoind_login_is_shown_only_as_set_changed_or_cleared() {
    let old = env::validate("", "", &[], None).unwrap();
    let new = env::validate(
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
    assert!(env::validate("http://relay.example", "", &[], None).is_err());
    assert!(env::validate("", "", &["http://mempool.example/api".into()], None).is_err());
    assert!(env::validate(
        "",
        "",
        &["bitcoind+http://u:p@127.0.0.1:18443".into()],
        None
    )
    .is_err());
    assert!(env::validate(
        "http://127.0.0.1:8787",
        "",
        &["bitcoind+http://127.0.0.1:18443".into()],
        None
    )
    .is_ok());
    assert!(env::validate("", "", &vec!["https://a.example".to_string(); 6], None).is_err());
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
