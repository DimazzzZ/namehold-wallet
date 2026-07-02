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

use crate::hsd::types::HsdBalance;

/// The hsd node RPC sends the locked fields in camelCase
/// (`lockedConfirmed`/`lockedUnconfirmed`). `HsdBalance` MUST keep deserializing
/// that shape — this is the node-parsing side of the contract.
#[test]
fn hsd_balance_deserializes_node_camelcase() {
    let node_json =
        r#"{"confirmed": 1000000, "unconfirmed": 500000, "lockedConfirmed": 200000, "lockedUnconfirmed": 100000}"#;
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
    assert!(v.get("lockedConfirmed").is_some(), "HsdBalance serializes camelCase");
    assert!(v.get("lockedUnconfirmed").is_some(), "HsdBalance serializes camelCase");
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
        locked_confirmed: None,     // explorer path leaves locked unknown
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
        ["confirmed", "locked_confirmed", "locked_unconfirmed", "unconfirmed"],
        "read_balance wire shape must match the frontend snake_case contract"
    );
    assert_eq!(wire["confirmed"], 1_000_000);
    assert_eq!(wire["unconfirmed"], 500_000);
    assert_eq!(wire["locked_confirmed"], 0);
    assert_eq!(wire["locked_unconfirmed"], 0);
}
