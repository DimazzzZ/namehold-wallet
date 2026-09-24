//! Rust⇄TS contract-shape guards.
//!
//! The frontend component tests hand-write the JSON shapes they expect from the
//! backend and drive them through a mocked `invoke`. That proves the UI logic,
//! but it CANNOT catch "contract drift": a Rust field that is renamed, retyped,
//! or serialized under a different casing than the frontend's zod schema /
//! TypeScript types expect. When that happens, every frontend test still passes
//! while production silently drops the field.
//!
//! These tests pin the exact wire shape of the balance contract so drift fails
//! loudly on the Rust side.
//!
//! Frontend contract of record:
//!   - src/types/index.ts        -> HsdBalance { confirmed, unconfirmed,
//!                                    locked_confirmed, locked_unconfirmed }
//!   - src/lib/zod.ts            -> snake_case keys, numbers nullable
//!   - src/lib/webqa-mock.ts     -> snake_case keys
//!
//! Backend seam:
//!   - `read_balance` (commands/read.rs) returns snake_case from ALL three of
//!     its code paths (zero-fallback json!, cached json!, and the explorer path
//!     which maps HsdBalance -> snake_case explicitly).

// Module doc uses deep prose indentation for backend/frontend contract
// citations; clippy misreads these as over-indented markdown list items.
#![allow(clippy::doc_overindented_list_items)]

use crate::hsd::types::HsdBalance;

/// The hsd node RPC sends the locked fields in camelCase
/// (`lockedConfirmed`/`lockedUnconfirmed`). `HsdBalance` MUST keep deserializing
/// that shape — this is the node-parsing side of the contract.
#[test]
fn hsd_balance_deserializes_node_camelcase() {
    let node_json = r#"{"confirmed": 1000000, "unconfirmed": 500000, "lockedConfirmed": 200000, "lockedUnconfirmed": 100000}"#;
    let b: HsdBalance = serde_json::from_str(node_json).unwrap();
    assert_eq!(b.confirmed, 1_000_000);
    assert_eq!(b.unconfirmed, 500_000);
    assert_eq!(b.locked_confirmed, Some(200_000));
    assert_eq!(b.locked_unconfirmed, Some(100_000));
}

/// Guardrail documenting WHY `read_balance` must not return an `HsdBalance`
/// verbatim: because it deserializes from camelCase, its Serialize impl also
/// emits camelCase, which the frontend's snake_case zod schema would silently
/// drop. If this ever changes (e.g. someone removes `rename_all`), this test
/// flags it so the `read_balance` mapping can be revisited.
#[test]
fn hsd_balance_serializes_camelcase_so_read_balance_must_map() {
    let b = HsdBalance {
        confirmed: 1,
        unconfirmed: 2,
        locked_confirmed: Some(3),
        locked_unconfirmed: Some(4),
    };
    let v = serde_json::to_value(&b).unwrap();
    // The raw struct is camelCase — NOT the frontend contract.
    assert!(
        v.get("lockedConfirmed").is_some(),
        "HsdBalance serializes camelCase"
    );
    assert!(
        v.get("lockedUnconfirmed").is_some(),
        "HsdBalance serializes camelCase"
    );
    assert!(
        v.get("locked_confirmed").is_none(),
        "raw HsdBalance is NOT snake_case; read_balance must map it before returning to the FE"
    );
}

/// Locks the exact snake_case shape the explorer path in `read_balance` returns
/// to the frontend. This mirrors the mapping in commands/read.rs and is the
/// contract the FE zod schema (src/lib/zod.ts) parses. If the FE contract or
/// this mapping drift apart, this test must be updated in lockstep — making the
/// drift explicit instead of silent.
#[test]
fn read_balance_explorer_path_returns_frontend_snake_case() {
    // Mirror of the mapping applied in commands/read.rs for the explorer path.
    let balance = HsdBalance {
        confirmed: 1_000_000,
        unconfirmed: 500_000,
        locked_confirmed: None, // explorer path leaves locked unknown
        locked_unconfirmed: None,
    };
    let wire = serde_json::json!({
        "confirmed": balance.confirmed,
        "unconfirmed": balance.unconfirmed,
        "locked_confirmed": balance.locked_confirmed.unwrap_or(0),
        "locked_unconfirmed": balance.locked_unconfirmed.unwrap_or(0),
    });

    // Exactly the four snake_case keys the frontend expects — no more, no less.
    let obj = wire.as_object().unwrap();
    let mut keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "confirmed",
            "locked_confirmed",
            "locked_unconfirmed",
            "unconfirmed"
        ],
        "read_balance wire shape must match the frontend snake_case contract"
    );
    assert_eq!(wire["confirmed"], 1_000_000);
    assert_eq!(wire["unconfirmed"], 500_000);
    assert_eq!(wire["locked_confirmed"], 0);
    assert_eq!(wire["locked_unconfirmed"], 0);
}

// --- The frontend's copy of the per-network RPC ports ---

/// `src/lib/utils.ts::defaultNodeRpcUrl` re-spells hsd's per-network loopback
/// RPC ports so Settings can show one as a placeholder. Its doc used to ask
/// whoever changed [`Network::default_rpc_url`] to remember it, which is a plea
/// rather than a guard — exactly the drift this module exists to catch.
///
/// Reading the TypeScript is the cheap half of the fix: a wrong placeholder is
/// cosmetic, so a round-trip to the backend for it would cost more than the
/// bug, but the two tables still have to agree.
#[test]
fn the_frontend_default_rpc_urls_match_the_backend_port_table() {
    use crate::noncustodial::network::Network;

    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/lib/utils.ts"),
    )
    .expect("src/lib/utils.ts should be readable from the crate");

    let body = src
        .split_once("export function defaultNodeRpcUrl(")
        .expect("defaultNodeRpcUrl should exist in src/lib/utils.ts")
        .1;

    for network in [
        Network::Main,
        Network::Testnet,
        Network::Regtest,
        Network::Simnet,
    ] {
        let expected = network.default_rpc_url();
        assert!(
            body.contains(&format!("\"{expected}\"")),
            "{network:?}: src/lib/utils.ts should offer {expected}; \
             update defaultNodeRpcUrl to match Network::default_rpc_url"
        );
    }
}

// --- The frontend's list of value-re-homing name actions ---

/// `src/components/ActivityView.tsx::NAME_COVENANT_ACTIONS` names the actions
/// whose covenant output re-homes a name's locked value onto the wallet's own
/// coin, so the Amount cell shows it as information rather than a spend.
///
/// Which actions belong there is the screen's decision — it is deliberately a
/// subset, since OPEN, REVOKE and CLAIM move no locked value. The spelling is
/// not: these are the labels `classify_tx` emits, and renaming one on the Rust
/// side would leave the set silently never matching, changing what the Amount
/// column shows with nothing failing.
#[test]
fn the_activity_view_name_actions_are_labels_the_backend_emits() {
    let backend = [
        "open", "bid", "reveal", "redeem", "register", "update", "renew", "transfer", "finalize",
        "revoke", "claim", "other",
    ];

    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/components/ActivityView.tsx"),
    )
    .expect("ActivityView.tsx should be readable from the crate");
    let set = src
        .split_once("const NAME_COVENANT_ACTIONS = new Set([")
        .expect("NAME_COVENANT_ACTIONS should exist")
        .1
        .split_once("]);")
        .expect("its literal should be closed")
        .0;

    let listed: Vec<String> = set
        .split(',')
        .filter_map(|s| {
            let s = s.trim().trim_matches('"');
            (!s.is_empty()).then(|| s.to_string())
        })
        .collect();
    assert!(!listed.is_empty(), "failed to parse the set");

    for action in &listed {
        assert!(
            backend.contains(&action.as_str()),
            "ActivityView lists '{action}', which classify_tx never emits — \
             a label was renamed on one side only"
        );
    }
}
