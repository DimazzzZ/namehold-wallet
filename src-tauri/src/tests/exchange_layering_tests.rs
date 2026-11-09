//! The exchange code that runs in the daemon holds no signing key (spec R13,
//! SECURITY.md "The exchange"): a lexical scan, like
//! `shakedex_layering_tests`, of every module the sync step reaches. A
//! wallet key can only come from the unlocked session (`master()`), from a
//! `SignerSession`, or through a function that derives a private key
//! (`trade_keys`, `account_xpub` takes a master too); a signature over a
//! transaction only from the `sign_*` functions of the pure layer.
//!
//! Relay messages are signed with relay keys, which control no coin
//! (`relay_msg::Message::sign`), and are allowed.

const DAEMON_SIDE: &[(&str, &str)] = &[
    ("exchange_io/job.rs", include_str!("../exchange_io/job.rs")),
    (
        "exchange_io/runner.rs",
        include_str!("../exchange_io/runner.rs"),
    ),
    ("exchange_io/env.rs", include_str!("../exchange_io/env.rs")),
    (
        "exchange_io/btc_observe.rs",
        include_str!("../exchange_io/btc_observe.rs"),
    ),
    (
        "exchange_io/hns_observe.rs",
        include_str!("../exchange_io/hns_observe.rs"),
    ),
    (
        "exchange_io/btc_source.rs",
        include_str!("../exchange_io/btc_source.rs"),
    ),
    (
        "exchange_io/relay_client.rs",
        include_str!("../exchange_io/relay_client.rs"),
    ),
    (
        "exchange_io/store.rs",
        include_str!("../exchange_io/store.rs"),
    ),
    (
        "exchange_io/btc_wallet.rs",
        include_str!("../exchange_io/btc_wallet.rs"),
    ),
    (
        "noncustodial/exchange/engine.rs",
        include_str!("../noncustodial/exchange/engine.rs"),
    ),
    (
        "noncustodial/exchange/watch.rs",
        include_str!("../noncustodial/exchange/watch.rs"),
    ),
];

const FORBIDDEN: &[&str] = &[
    "master(",
    "SignerSession",
    "trade_keys(",
    "account_xpub(",
    "branch_xpub(",
    "sign_claim(",
    "sign_refund(",
    "sign_funding(",
    "sign_plan(",
    "sign_p2wpkh_input(",
    "sign_p2wsh_input(",
    "derive_lock_key(",
    "ExtendedPrivKey",
];

/// Code lines only: comments name these functions to explain the rule.
fn code_lines(src: &str) -> impl Iterator<Item = (usize, &str)> {
    src.lines().enumerate().filter(|(_, l)| {
        let t = l.trim_start();
        !t.starts_with("//") && !t.starts_with("///") && !t.starts_with("//!")
    })
}

#[test]
fn the_daemon_side_of_the_exchange_names_no_signing_key_or_call() {
    let mut found = Vec::new();
    for (file, src) in DAEMON_SIDE {
        for (n, line) in code_lines(src) {
            for f in FORBIDDEN {
                if line.contains(f) {
                    found.push(format!("{file}:{}: {f}", n + 1));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "signing in the daemon's exchange code:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_sync_step_runs_the_exchange_step() {
    let sync = include_str!("../commands/sync.rs");
    assert!(sync.contains("crate::exchange_io::env::sync_step(db_path, profile_id)"));
}

/// The step applies the same node rule as the commands before it reads or
/// sends through a node (SECURITY.md "The HNS node must be local"), and
/// every HNS send goes through `send_hns`, which runs the broadcast gates.
#[test]
fn the_sync_step_applies_the_node_rule_and_sends_only_through_the_gates() {
    let env = include_str!("../exchange_io/env.rs");
    let step = &env[env.find("pub async fn sync_step").unwrap()..];
    assert!(step.contains("node_refusal(&kind, n, &settings)"), "{step}");
    let runner = include_str!("../exchange_io/runner.rs");
    let send = &runner[runner.find("pub async fn send_hns").unwrap()..];
    let send = &send[..send.find("\n}\n").unwrap()];
    assert!(send.contains("rpc::broadcast_gates("), "{send}");
    for (file, src) in [
        ("job.rs", include_str!("../exchange_io/job.rs")),
        (
            "watchtower.rs",
            include_str!("../relay_server/watchtower.rs"),
        ),
        (
            "commands/exchange.rs",
            include_str!("../commands/exchange.rs"),
        ),
    ] {
        for (n, line) in code_lines(src) {
            assert!(
                !line.contains("send_raw_transaction("),
                "{file}:{}: an HNS send outside send_hns",
                n + 1
            );
        }
    }
}

#[test]
fn a_watch_job_has_no_field_for_a_secret_or_a_claim_of_the_buyer() {
    let watch = include_str!("../noncustodial/exchange/watch.rs");
    let job = &watch[watch.find("pub struct WatchJob").unwrap()..];
    let job = &job[..job.find('}').unwrap()];
    assert!(!job.contains("secret"), "{job}");
    assert!(!job.contains("hns_claim"), "{job}");
}
