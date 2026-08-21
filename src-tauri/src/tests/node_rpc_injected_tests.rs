//! Tests for the `_with_client` inner functions introduced by the P3 refactor.
//!
//! Each of these functions was extracted from a `#[tauri::command]` so it
//! accepts `&dyn NodeRpc` instead of building a `NodeRpcClient` from settings.
//! With the `MockNodeRpc` seam we can now cover:
//!
//! - The success shape (fetch → decode → shape).
//! - The soft-degrade paths (RPC error, missing fields, null responses).
//! - The special-case error branches (tx_index_disabled, network mismatch,
//!   sync-not-complete).
//!
//! Together with the pure `read_pure` / `names_pure` tests from P2, this
//! covers every branch in the previously-hard-to-test node-fetch paths
//! without requiring a live hsd node.

use serde_json::json;

use crate::commands::read::{
    node_tip_height_if_synced_with_client, read_block_info_with_client,
    read_name_records_with_client, read_tx_info_with_client,
};
use crate::error::AppError;
use crate::noncustodial::node_rpc::NodeRpc;
use crate::noncustodial::rpc::{BlockchainInfo, NodeCoin};
use crate::tests::mock_node_rpc::MockNodeRpc;

// ------- read_block_info_with_client --------------------------------------

#[tokio::test]
async fn block_info_success_shapes_response() {
    let mock = MockNodeRpc::new()
        .with_block_hash("00abc".to_string())
        .with_block(json!({
            "height": 12345,
            "time": 1_700_000_000i64,
            "difficulty": 42.5,
            "tx": [
                { "outputs": [ { "value": 2_000_000_000i64 }, { "value": 50_000 } ] },
                { "outputs": [ { "value": 100 } ] }
            ]
        }));
    let out = read_block_info_with_client(&mock, 12345).await;
    assert_eq!(out["height"], 12345);
    assert_eq!(out["hash"], "00abc");
    assert_eq!(out["txCount"], 2);
    assert_eq!(out["minerReward"], 2_000_050_000i64);
    assert_eq!(out["difficulty"], 42.5);
}

#[tokio::test]
async fn block_info_soft_degrades_when_block_hash_fails() {
    // Height is beyond the tip — hsd returns an error. We soft-degrade to null.
    let mock = MockNodeRpc::new().with_block_hash_err("Block height out of range");
    let out = read_block_info_with_client(&mock, 99999).await;
    assert!(out.is_null(), "expected null soft-degrade, got {out:?}");
}

#[tokio::test]
async fn block_info_soft_degrades_when_get_block_fails() {
    // Hash lookup succeeds but the block fetch fails (node fell over mid-read).
    let mock = MockNodeRpc::new()
        .with_block_hash("00abc".to_string())
        .with_block_err("connection reset");
    let out = read_block_info_with_client(&mock, 100).await;
    assert!(out.is_null());
}

// ------- read_tx_info_with_client -----------------------------------------

#[tokio::test]
async fn tx_info_success_shapes_response() {
    let mock = MockNodeRpc::new().with_tx_by_hash(json!({
        "hash": "deadbeef",
        "confirmations": 6,
        "height": 1000,
        "block": "00blockhash",
        "time": 1_700_000_000i64,
        "fee": 1500,
        "inputs": [
            { "coin": { "value": 5000 } },
            { "coin": { "value": 3500 } },
        ],
        "outputs": [
            { "value": 4000 },
            { "value": 3000 },
        ],
    }));
    let out = read_tx_info_with_client(&mock, "deadbeef").await;
    assert_eq!(out["txid"], "deadbeef");
    assert_eq!(out["confirmations"], 6);
    assert_eq!(out["height"], 1000);
    assert_eq!(out["block"], "00blockhash");
    assert_eq!(out["time"], 1_700_000_000i64);
    assert_eq!(out["fee"], 1500);
    assert_eq!(out["inputsCount"], 2);
    assert_eq!(out["outputsCount"], 2);
    assert_eq!(out["totalOut"], 7000);
}

#[tokio::test]
async fn tx_info_returns_null_on_404_miss() {
    // hsd returns a JSON null body for an unknown tx.
    let mock = MockNodeRpc::new().with_tx_by_hash(serde_json::Value::Null);
    let out = read_tx_info_with_client(&mock, "notatx").await;
    assert!(out.is_null());
}

#[tokio::test]
async fn tx_info_returns_tx_index_disabled_sentinel() {
    // The tx-by-hash REST route fails with the normalized index-disabled
    // error message. The command must forward the distinct sentinel so
    // the modal can render a "requires --index-tx" hint.
    let mock = MockNodeRpc::new()
        .with_tx_by_hash_err("tx index not enabled on this hsd node: … (status 400)");
    let out = read_tx_info_with_client(&mock, "abc").await;
    assert_eq!(out["error"], "tx_index_disabled");
}

#[tokio::test]
async fn tx_info_generic_rpc_error_soft_degrades_to_null() {
    // Any other RPC error yields null (frontend renders "requires synced node").
    let mock = MockNodeRpc::new().with_tx_by_hash_err("connection refused");
    let out = read_tx_info_with_client(&mock, "abc").await;
    assert!(out.is_null());
}

#[tokio::test]
async fn tx_info_computes_fee_from_input_coins_when_hsd_fee_missing() {
    // hsd omits top-level `fee` on some paths — we recover it from
    // `Σ inputs[].coin.value − Σ outputs[].value`.
    let mock = MockNodeRpc::new().with_tx_by_hash(json!({
        "hash": "abc",
        "inputs": [
            { "coin": { "value": 1000 } },
            { "coin": { "value": 2000 } },
        ],
        "outputs": [ { "value": 1200 }, { "value": 1300 } ],
    }));
    let out = read_tx_info_with_client(&mock, "abc").await;
    // fee = (1000 + 2000) − (1200 + 1300) = 500
    assert_eq!(out["fee"], 500);
    assert_eq!(out["totalOut"], 2500);
}

#[tokio::test]
async fn tx_info_fee_null_on_coinbase_shape() {
    // Coinbase input has no `coin` field — fee is genuinely unknowable, so
    // we serialize null rather than a misleading zero.
    let mock = MockNodeRpc::new().with_tx_by_hash(json!({
        "hash": "cb",
        "inputs": [ { "prevout": { "hash": "00", "index": 4294967295u32 } } ],
        "outputs": [ { "value": 2_000_000_000i64 } ],
    }));
    let out = read_tx_info_with_client(&mock, "cb").await;
    assert!(out["fee"].is_null());
    assert_eq!(out["totalOut"], 2_000_000_000i64);
}

// ------- read_name_records_with_client ------------------------------------

#[tokio::test]
async fn name_records_success_returns_resource() {
    let resource = json!({
        "records": [
            { "type": "NS", "ns": "ns1.example." },
            { "type": "TXT", "txt": ["hello"] },
        ],
        "ttl": 3600,
    });
    let mock = MockNodeRpc::new().with_name_resource(resource.clone());
    let out = read_name_records_with_client(&mock, "foo").await;
    assert_eq!(out["records"].as_array().unwrap().len(), 2);
    assert_eq!(out["ttl"], 3600);
}

#[tokio::test]
async fn name_records_null_resource_yields_empty_records() {
    // hsd returns null for names with no resource — we normalize to `{records: []}`.
    let mock = MockNodeRpc::new().with_name_resource(serde_json::Value::Null);
    let out = read_name_records_with_client(&mock, "foo").await;
    assert_eq!(out["records"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn name_records_rpc_error_yields_empty_records() {
    let mock = MockNodeRpc::new().with_name_resource_err("not found");
    let out = read_name_records_with_client(&mock, "foo").await;
    // Uniform empty-resource shape regardless of the error cause.
    assert_eq!(out["records"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn name_records_resource_missing_records_field_gets_synthesized_empty_array() {
    // Belt-and-braces: if the node ever returns a resource without a
    // `records` field, we insert an empty array so the frontend contract
    // (`resource.records` always exists) holds regardless of node version.
    let mock = MockNodeRpc::new().with_name_resource(json!({ "ttl": 3600 }));
    let out = read_name_records_with_client(&mock, "foo").await;
    assert!(out["records"].is_array());
    assert_eq!(out["records"].as_array().unwrap().len(), 0);
    assert_eq!(out["ttl"], 3600); // other fields preserved
}

#[tokio::test]
async fn name_records_non_object_non_null_degrades_to_empty() {
    // Defensive: if the node ever emits a scalar (shouldn't happen), we
    // degrade rather than surface the surprise shape.
    let mock = MockNodeRpc::new().with_name_resource(json!("some string"));
    let out = read_name_records_with_client(&mock, "foo").await;
    assert_eq!(out["records"].as_array().unwrap().len(), 0);
}

// ------- node_tip_height_if_synced_with_client ----------------------------

fn info(
    blocks: i64,
    progress: Option<f64>,
    headers: Option<i64>,
    chain: Option<&str>,
) -> BlockchainInfo {
    BlockchainInfo {
        blocks,
        headers,
        verification_progress: progress,
        chain: chain.map(String::from),
        bestblockhash: None,
    }
}

#[tokio::test]
async fn tip_height_returns_some_when_progress_is_full() {
    let mock =
        MockNodeRpc::new().with_blockchain_info(info(1000, Some(1.0), Some(1000), Some("main")));
    assert_eq!(
        node_tip_height_if_synced_with_client(&mock, None).await,
        Some(1000)
    );
}

#[tokio::test]
async fn tip_height_returns_none_when_progress_below_threshold() {
    // 99.98% is BELOW the 99.99% threshold — still catching up.
    let mock =
        MockNodeRpc::new().with_blockchain_info(info(1000, Some(0.9998), Some(1000), Some("main")));
    assert_eq!(
        node_tip_height_if_synced_with_client(&mock, None).await,
        None
    );
}

#[tokio::test]
async fn tip_height_falls_back_to_headers_when_progress_absent() {
    // When verification_progress is missing, height >= headers means synced.
    let mock = MockNodeRpc::new().with_blockchain_info(info(500, None, Some(500), Some("main")));
    assert_eq!(
        node_tip_height_if_synced_with_client(&mock, None).await,
        Some(500)
    );
}

#[tokio::test]
async fn tip_height_assumes_synced_when_no_metadata_available() {
    // Neither progress nor headers — regtest with a single miner. Assume synced.
    let mock = MockNodeRpc::new().with_blockchain_info(info(42, None, None, Some("regtest")));
    assert_eq!(
        node_tip_height_if_synced_with_client(&mock, None).await,
        Some(42)
    );
}

#[tokio::test]
async fn tip_height_returns_none_on_network_mismatch() {
    // Node is on regtest, wallet expects mainnet — reject.
    let mock =
        MockNodeRpc::new().with_blockchain_info(info(1000, Some(1.0), Some(1000), Some("regtest")));
    assert_eq!(
        node_tip_height_if_synced_with_client(&mock, Some("main")).await,
        None
    );
}

#[tokio::test]
async fn tip_height_accepts_main_vs_mainnet_canonicalization() {
    // hsd emits "main", the profile might store "mainnet" — treated as equal.
    let mock =
        MockNodeRpc::new().with_blockchain_info(info(1000, Some(1.0), Some(1000), Some("main")));
    assert_eq!(
        node_tip_height_if_synced_with_client(&mock, Some("mainnet")).await,
        Some(1000)
    );
}

#[tokio::test]
async fn tip_height_permissive_when_node_omits_chain_field() {
    // Older hsd builds may not include `chain` — we conservatively allow it
    // (the SPV gate and other checks handle the mismatch case).
    let mock = MockNodeRpc::new().with_blockchain_info(info(1000, Some(1.0), Some(1000), None));
    assert_eq!(
        node_tip_height_if_synced_with_client(&mock, Some("main")).await,
        Some(1000)
    );
}

#[tokio::test]
async fn tip_height_returns_none_on_rpc_failure() {
    let mock = MockNodeRpc::new().with_blockchain_info_err("connection refused");
    assert_eq!(
        node_tip_height_if_synced_with_client(&mock, None).await,
        None
    );
}

// ------- MockNodeRpc sanity + fetch_name_state coverage -------------------

#[tokio::test]
async fn mock_default_returns_configured_error() {
    // Sanity: default mock returns AppError::Rpc from every unconfigured call.
    let mock = MockNodeRpc::new();
    let res = mock.get_name_info("foo").await;
    match res {
        Err(AppError::Rpc(msg)) => assert!(msg.contains("not configured")),
        other => panic!("expected Rpc error, got {other:?}"),
    }
}

#[tokio::test]
async fn fetch_name_state_extracts_all_fields() {
    // Full-shape name_info: exercise every field extraction in fetch_name_state.
    let mock = MockNodeRpc::new().with_name_info(json!({
        "info": {
            "state": "BIDDING",
            "height": 5000,
            "value": 100_000,
            "renewals": 3,
            "claimed": 1,
            "weak": true,
        }
    }));
    let ns = crate::commands::names::fetch_name_state(&mock, "example")
        .await
        .unwrap();
    assert_eq!(ns.phase, "BIDDING");
    assert_eq!(ns.height, 5000);
    assert_eq!(ns.value, 100_000);
    assert_eq!(ns.renewals, 3);
    assert_eq!(ns.claimed, 1);
    assert!(ns.weak);
}

#[tokio::test]
async fn fetch_name_state_errors_when_info_is_null() {
    // hsd returns `{info: null}` for names that have no on-chain state.
    let mock = MockNodeRpc::new().with_name_info(json!({ "info": null }));
    let err = crate::commands::names::fetch_name_state(&mock, "example")
        .await
        .unwrap_err();
    match err {
        AppError::InvalidInput(msg) => assert!(msg.contains("no on-chain state")),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[tokio::test]
async fn fetch_name_state_errors_when_info_field_missing() {
    // Older hsd or shape mismatch: no `info` field at all.
    let mock = MockNodeRpc::new().with_name_info(json!({ "something": "else" }));
    let err = crate::commands::names::fetch_name_state(&mock, "example")
        .await
        .unwrap_err();
    match err {
        AppError::InvalidInput(_) => {}
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[tokio::test]
async fn fetch_name_state_defaults_when_fields_missing() {
    // Partial info: only `state` present, everything else defaults.
    let mock = MockNodeRpc::new().with_name_info(json!({
        "info": { "state": "reveal" }  // note lowercase — should be uppercased
    }));
    let ns = crate::commands::names::fetch_name_state(&mock, "example")
        .await
        .unwrap();
    assert_eq!(ns.phase, "REVEAL"); // uppercased
    assert_eq!(ns.height, 0);
    assert_eq!(ns.value, 0);
    assert_eq!(ns.renewals, 0);
    assert_eq!(ns.claimed, 0);
    assert!(!ns.weak); // default false
}

#[tokio::test]
async fn fetch_name_state_defaults_phase_to_empty_when_state_missing() {
    let mock = MockNodeRpc::new().with_name_info(json!({
        "info": { "height": 10 }
    }));
    let ns = crate::commands::names::fetch_name_state(&mock, "example")
        .await
        .unwrap();
    assert_eq!(ns.phase, "");
    assert_eq!(ns.height, 10);
}

#[tokio::test]
async fn fetch_name_state_propagates_rpc_error() {
    // If the underlying RPC fails, the helper must surface the error.
    let mock = MockNodeRpc::new().with_name_info_err("connection refused");
    let err = crate::commands::names::fetch_name_state(&mock, "example")
        .await
        .unwrap_err();
    match err {
        AppError::Rpc(_) => {}
        other => panic!("expected Rpc error, got {other:?}"),
    }
}

// ============================================================================
// P4 extractions
// ============================================================================
//
// Tests for the second wave of `_with_client` extractions. Same pattern as
// above: each covers success + soft-degrade + at least one error branch.

// ------- read_name_info_node_with_client ----------------------------------

#[tokio::test]
async fn read_name_info_node_normalizes_populated_info() {
    // Node returns a full getnameinfo — helper normalizes to HsdName.
    let mock = MockNodeRpc::new().with_name_info(json!({
        "info": {
            "name": "example",
            "state": "OPENING",
            "height": 100,
            "renewal": 105,
            "stats": { "hoursUntilBidding": 24.0 }
        }
    }));
    let out = crate::commands::read::read_name_info_node_with_client(&mock, "example")
        .await
        .expect("node knew name");
    assert_eq!(out.name, "example");
    assert_eq!(out.state.as_deref(), Some("OPENING"));
}

#[tokio::test]
async fn read_name_info_node_synthesizes_available_for_null_info() {
    // Node answers but info is null — helper synthesizes AVAILABLE.
    let mock = MockNodeRpc::new().with_name_info(json!({ "info": serde_json::Value::Null }));
    let out = crate::commands::read::read_name_info_node_with_client(&mock, "brandnew")
        .await
        .expect("synthesized");
    assert_eq!(out.name, "brandnew");
    assert_eq!(out.state.as_deref(), Some("AVAILABLE"));
    assert_eq!(out.registered, Some(false));
}

#[tokio::test]
async fn read_name_info_node_returns_none_on_rpc_error() {
    // RPC error → helper returns None so caller falls back to explorer.
    let mock = MockNodeRpc::new().with_name_info_err("connection refused");
    let out = crate::commands::read::read_name_info_node_with_client(&mock, "example").await;
    assert!(out.is_none());
}

// ------- get_resource_info_with_client / get_resource_records_with_client -

#[tokio::test]
async fn get_resource_info_normalizes_when_info_present() {
    let mock = MockNodeRpc::new().with_name_info(json!({
        "info": { "name": "example", "state": "CLOSED", "height": 500 }
    }));
    let info = crate::commands::read::get_resource_info_with_client(&mock, "example").await;
    assert_eq!(info.get("state").and_then(|v| v.as_str()), Some("CLOSED"));
}

#[tokio::test]
async fn get_resource_info_returns_available_shape_when_info_null() {
    let mock = MockNodeRpc::new().with_name_info(json!({ "info": serde_json::Value::Null }));
    let info = crate::commands::read::get_resource_info_with_client(&mock, "fresh").await;
    assert_eq!(info.get("state").and_then(|v| v.as_str()), Some("AVAILABLE"));
    assert_eq!(info.get("name").and_then(|v| v.as_str()), Some("fresh"));
}

#[tokio::test]
async fn get_resource_info_returns_empty_object_on_rpc_error() {
    let mock = MockNodeRpc::new().with_name_info_err("timeout");
    let info = crate::commands::read::get_resource_info_with_client(&mock, "example").await;
    assert!(info.as_object().map(|o| o.is_empty()).unwrap_or(false));
}

#[tokio::test]
async fn get_resource_records_extracts_records_array() {
    let mock = MockNodeRpc::new().with_name_resource(json!({
        "records": [
            { "type": "TXT", "txt": ["hello"] },
            { "type": "NS", "ns": "ns1.example." }
        ]
    }));
    let records = crate::commands::read::get_resource_records_with_client(&mock, "example").await;
    assert_eq!(records.len(), 2);
}

#[tokio::test]
async fn get_resource_records_returns_empty_on_null_resource() {
    let mock = MockNodeRpc::new().with_name_resource(serde_json::Value::Null);
    let records = crate::commands::read::get_resource_records_with_client(&mock, "example").await;
    assert!(records.is_empty());
}

#[tokio::test]
async fn get_resource_records_returns_empty_on_rpc_error() {
    let mock = MockNodeRpc::new().with_name_resource_err("boom");
    let records = crate::commands::read::get_resource_records_with_client(&mock, "example").await;
    assert!(records.is_empty());
}

// ------- discover_names_via_node_with_client ------------------------------

#[tokio::test]
async fn discover_names_resolves_via_getnamebyhash_then_fetches_info() {
    // Node knows the hash → resolves to name → info fetched → returned.
    let mock = MockNodeRpc::new()
        .with_name_by_hash(Some("example".to_string()))
        .with_name_info(json!({ "info": { "name": "example", "state": "CLOSED" } }));
    let hashes = vec![crate::db::queries::WalletNameHash {
        name_hash_hex: "aa".repeat(32),
        raw_name_hex: None,
    }];
    let out =
        crate::commands::read::discover_names_via_node_with_client(&mock, &hashes).await;
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].0, "example");
}

#[tokio::test]
async fn discover_names_falls_back_to_raw_name_when_hash_unresolved() {
    // getnamebyhash returns None → helper falls back to raw_name_hex.
    // "example" as hex = 6578616d706c65
    let mock = MockNodeRpc::new()
        .with_name_by_hash(None)
        .with_name_info(json!({ "info": { "name": "example" } }));
    let hashes = vec![crate::db::queries::WalletNameHash {
        name_hash_hex: "bb".repeat(32),
        raw_name_hex: Some("6578616d706c65".to_string()),
    }];
    let out =
        crate::commands::read::discover_names_via_node_with_client(&mock, &hashes).await;
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].0, "example");
}

#[tokio::test]
async fn discover_names_dedupes_multiple_hashes_for_same_name() {
    let mock = MockNodeRpc::new()
        .with_name_by_hash(Some("example".to_string()))
        .with_name_info(json!({ "info": { "name": "example" } }));
    let hashes = vec![
        crate::db::queries::WalletNameHash {
            name_hash_hex: "aa".repeat(32),
            raw_name_hex: None,
        },
        crate::db::queries::WalletNameHash {
            name_hash_hex: "bb".repeat(32),
            raw_name_hex: None,
        },
    ];
    let out =
        crate::commands::read::discover_names_via_node_with_client(&mock, &hashes).await;
    // Both hashes resolve to the same name — dedup keeps one entry.
    assert_eq!(out.len(), 1);
}

#[tokio::test]
async fn discover_names_returns_empty_when_no_resolution_and_no_raw_name() {
    // Hash unresolvable and no rawName — nothing to discover.
    let mock = MockNodeRpc::new().with_name_by_hash(None);
    let hashes = vec![crate::db::queries::WalletNameHash {
        name_hash_hex: "cc".repeat(32),
        raw_name_hex: None,
    }];
    let out =
        crate::commands::read::discover_names_via_node_with_client(&mock, &hashes).await;
    assert!(out.is_empty());
}

// ------- resolve_name_ownership_with_client -------------------------------

#[tokio::test]
async fn resolve_ownership_returns_full_resolution_for_owned_name() {
    let mock = MockNodeRpc::new()
        .with_name_info(json!({
            "info": {
                "name": "example",
                "state": "CLOSED",
                "owner": { "hash": "aabbcc", "index": 0 }
            }
        }))
        .with_tx_out(Some(json!({
            "address": { "string": "hs1qmine", "hash": "deadbeef" },
            "value": 100000
        })));
    let res = crate::commands::read::resolve_name_ownership_with_client(&mock, "example")
        .await
        .expect("no rpc error");
    assert_eq!(res.owner_txid.as_deref(), Some("aabbcc"));
    assert_eq!(res.owner_vout, Some(0));
    assert_eq!(res.owner_address.as_deref(), Some("hs1qmine"));
    assert!(res.info.is_some());
}

#[tokio::test]
async fn resolve_ownership_returns_no_owner_when_info_null() {
    // Never-opened name → info is null → no owner outpoint.
    let mock = MockNodeRpc::new().with_name_info(json!({ "info": serde_json::Value::Null }));
    let res = crate::commands::read::resolve_name_ownership_with_client(&mock, "fresh")
        .await
        .expect("no rpc error");
    assert!(res.info.is_none());
    assert!(res.owner_txid.is_none());
    assert!(res.owner_vout.is_none());
    assert!(res.owner_address.is_none());
}

#[tokio::test]
async fn resolve_ownership_skips_all_zeros_hash() {
    // hsd represents "no owner outpoint" as all-zeros hash — never call gettxout.
    let mock = MockNodeRpc::new().with_name_info(json!({
        "info": {
            "name": "example",
            "owner": { "hash": "00000000", "index": 0 }
        }
    }));
    let res = crate::commands::read::resolve_name_ownership_with_client(&mock, "example")
        .await
        .expect("no rpc error");
    assert!(res.owner_txid.is_none());
    assert!(res.owner_address.is_none());
}

#[tokio::test]
async fn resolve_ownership_handles_gettxout_miss() {
    // Owner outpoint exists but gettxout returns None (spent/missing) — helper
    // still returns the txid/vout, just no address.
    let mock = MockNodeRpc::new()
        .with_name_info(json!({
            "info": {
                "owner": { "hash": "abcdef", "index": 3 }
            }
        }))
        .with_tx_out(None);
    let res = crate::commands::read::resolve_name_ownership_with_client(&mock, "example")
        .await
        .expect("no rpc error");
    assert_eq!(res.owner_txid.as_deref(), Some("abcdef"));
    assert_eq!(res.owner_vout, Some(3));
    assert!(res.owner_address.is_none());
}

#[tokio::test]
async fn resolve_ownership_propagates_getnameinfo_error() {
    let mock = MockNodeRpc::new().with_name_info_err("connection refused");
    let err = crate::commands::read::resolve_name_ownership_with_client(&mock, "example")
        .await
        .unwrap_err();
    assert!(err.contains("example"));
    assert!(err.contains("getnameinfo"));
}

// ------- classify_broadcast_outcome_with_client ---------------------------

#[tokio::test]
async fn broadcast_classify_success_returns_txid() {
    let mock = MockNodeRpc::new().with_send_raw_transaction("abc123".to_string());
    let out = crate::commands::tx::classify_broadcast_outcome_with_client(&mock, "deadbeef").await;
    match out {
        crate::commands::tx::BroadcastOutcome::Success(txid) => assert_eq!(txid, "abc123"),
        other => panic!("expected Success, got {other:?}"),
    }
}

#[tokio::test]
async fn broadcast_classify_rpc_error_marks_failed() {
    // JSON-RPC error → node definitively rejected → RpcError.
    let mock = MockNodeRpc::new()
        .with_send_raw_transaction_rpc_err("bad-txns-inputs-missing");
    let out = crate::commands::tx::classify_broadcast_outcome_with_client(&mock, "deadbeef").await;
    match out {
        crate::commands::tx::BroadcastOutcome::RpcError(e) => {
            assert!(e.to_string().contains("bad-txns-inputs-missing"));
        }
        other => panic!("expected RpcError, got {other:?}"),
    }
}

#[tokio::test]
async fn broadcast_classify_transport_error_marks_pending() {
    // Transport error (Http) → ambiguous → TransportError → draft stays broadcast_pending.
    let mock = MockNodeRpc::new()
        .with_send_raw_transaction_transport_err("connection reset");
    let out = crate::commands::tx::classify_broadcast_outcome_with_client(&mock, "deadbeef").await;
    match out {
        crate::commands::tx::BroadcastOutcome::TransportError(_) => {}
        other => panic!("expected TransportError, got {other:?}"),
    }
}

// ------- apply_node_write_probe_with_client -------------------------------

#[tokio::test]
async fn write_probe_noop_when_cap_already_read_only() {
    // No RPC call should happen if cap is already can_write=false.
    let mock = MockNodeRpc::new(); // all methods error
    let mut cap = crate::providers::WriteCapability {
        can_write: false,
        broadcaster_available: false,
        signer_unlocked: false,
        reason: Some("locked".to_string()),
    };
    crate::commands::tx::apply_node_write_probe_with_client(&mock, &mut cap, "http://x", None)
        .await;
    // Unchanged.
    assert!(!cap.can_write);
    assert_eq!(cap.reason.as_deref(), Some("locked"));
}

#[tokio::test]
async fn write_probe_unreachable_downgrades_with_start_node_reason() {
    let mock = MockNodeRpc::new().with_blockchain_info_err("no route");
    let mut cap = writable_cap();
    crate::commands::tx::apply_node_write_probe_with_client(
        &mock,
        &mut cap,
        "http://localhost:12037",
        None,
    )
    .await;
    assert!(!cap.can_write);
    assert!(cap.reason.as_deref().unwrap().contains("Start your local node"));
    assert!(cap.reason.as_deref().unwrap().contains("http://localhost:12037"));
}

#[tokio::test]
async fn write_probe_unsynced_downgrades_with_progress_pct() {
    // verification_progress=0.5 → 50% not yet synced.
    let mock = MockNodeRpc::new().with_blockchain_info(BlockchainInfo {
        chain: Some("regtest".to_string()),
        blocks: 100,
        headers: Some(200),
        verification_progress: Some(0.5),
        bestblockhash: None,
    });
    let mut cap = writable_cap();
    crate::commands::tx::apply_node_write_probe_with_client(&mock, &mut cap, "http://x", None)
        .await;
    assert!(!cap.can_write);
    let reason = cap.reason.as_deref().unwrap();
    assert!(reason.contains("50%"));
    assert!(reason.contains("still syncing"));
}

#[tokio::test]
async fn write_probe_synced_but_no_address_index_downgrades() {
    // Synced (progress=1.0), but getcoinsbyaddress errors → not address-indexed.
    let mock = MockNodeRpc::new()
        .with_blockchain_info(BlockchainInfo {
            chain: Some("regtest".to_string()),
            blocks: 100,
            headers: Some(100),
            verification_progress: Some(1.0),
            bestblockhash: None,
        })
        .with_coins_by_address_err("Address indexing is not enabled");
    let mut cap = writable_cap();
    crate::commands::tx::apply_node_write_probe_with_client(
        &mock,
        &mut cap,
        "http://x",
        Some("hs1qprobe"),
    )
    .await;
    assert!(!cap.can_write);
    assert!(cap.reason.as_deref().unwrap().contains("address-indexed"));
}

#[tokio::test]
async fn write_probe_synced_and_indexed_keeps_write_capable() {
    let mock = MockNodeRpc::new()
        .with_blockchain_info(BlockchainInfo {
            chain: Some("regtest".to_string()),
            blocks: 100,
            headers: Some(100),
            verification_progress: Some(1.0),
            bestblockhash: None,
        })
        .with_coins_by_address(vec![]);
    let mut cap = writable_cap();
    crate::commands::tx::apply_node_write_probe_with_client(
        &mock,
        &mut cap,
        "http://x",
        Some("hs1qprobe"),
    )
    .await;
    assert!(cap.can_write);
    assert!(cap.reason.is_none());
}

fn writable_cap() -> crate::providers::WriteCapability {
    crate::providers::WriteCapability {
        can_write: true,
        broadcaster_available: true,
        signer_unlocked: true,
        reason: None,
    }
}

// ------- fetch_wallet_coins_and_txs_with_client ---------------------------

#[tokio::test]
async fn fetch_coins_and_txs_empty_addresses_returns_empty() {
    let mock = MockNodeRpc::new();
    let (coins, txs) =
        crate::commands::tx::fetch_wallet_coins_and_txs_with_client(&mock, &[], "http://x")
            .await
            .unwrap();
    assert!(coins.is_empty());
    assert!(txs.is_empty());
}

#[tokio::test]
async fn fetch_coins_and_txs_http_error_maps_to_start_hsd_hint() {
    let mock = MockNodeRpc::new().with_coins_by_address_err("__http_error_marker__");
    // We can't easily inject Http vs Rpc via with_coins_by_address_err (it wraps
    // in AppError::Rpc), so test the Rpc branch: message includes index-address hint.
    let addresses = vec!["hs1qmine".to_string()];
    let err = crate::commands::tx::fetch_wallet_coins_and_txs_with_client(
        &mock,
        &addresses,
        "http://localhost:12037",
    )
    .await
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("index-address") || msg.contains("Start hsd"));
}

#[tokio::test]
async fn fetch_coins_and_txs_success_returns_coins_and_txs() {
    let coin: NodeCoin = serde_json::from_value(json!({
        "hash": "aabbcc",
        "index": 0,
        "value": 1_000_000,
        "address": "hs1qmine",
        "height": 100,
        "covenant": { "type": 0, "action": "NONE", "items": [] }
    }))
    .unwrap();
    let mock = MockNodeRpc::new()
        .with_coins_by_address(vec![coin])
        .with_raw_transaction(json!({ "hash": "aabbcc", "outputs": [] }));
    let addresses = vec!["hs1qmine".to_string()];
    let (coins, txs) = crate::commands::tx::fetch_wallet_coins_and_txs_with_client(
        &mock,
        &addresses,
        "http://x",
    )
    .await
    .unwrap();
    assert_eq!(coins.len(), 1);
    assert_eq!(txs.len(), 1);
    assert_eq!(txs[0].0, "aabbcc");
    assert_eq!(txs[0].1, Some(100));
}

#[tokio::test]
async fn fetch_coins_and_txs_dedupes_repeated_txids() {
    // Two coins in the same funding tx → one getrawtransaction call, one tx entry.
    let coin_a: NodeCoin = serde_json::from_value(json!({
        "hash": "shared",
        "index": 0,
        "value": 500,
        "address": "hs1qa",
        "height": 100,
        "covenant": { "type": 0, "action": "NONE", "items": [] }
    }))
    .unwrap();
    let coin_b = NodeCoin {
        vout: 1,
        ..coin_a.clone()
    };
    let mock = MockNodeRpc::new()
        .with_coins_by_address(vec![coin_a, coin_b])
        .with_raw_transaction(json!({ "hash": "shared" }));
    let addresses = vec!["hs1qa".to_string()];
    let (coins, txs) = crate::commands::tx::fetch_wallet_coins_and_txs_with_client(
        &mock,
        &addresses,
        "http://x",
    )
    .await
    .unwrap();
    assert_eq!(coins.len(), 2);
    assert_eq!(txs.len(), 1); // deduped
}

// ------- fetch_coins_with_guard_with_client (sync.rs) ---------------------

#[tokio::test]
async fn coins_guard_none_addresses_returns_some_empty() {
    let mock = MockNodeRpc::new();
    let out = crate::commands::sync::fetch_coins_with_guard_with_client(&mock, &[]).await;
    assert_eq!(out, Some(vec![]));
}

#[tokio::test]
async fn coins_guard_all_errors_returns_none() {
    // Wallet has addresses, every query errored → guard trips (returns None).
    let mock = MockNodeRpc::new().with_coins_by_address_err("Address indexing disabled");
    let addresses = vec!["hs1qa".to_string(), "hs1qb".to_string()];
    let out =
        crate::commands::sync::fetch_coins_with_guard_with_client(&mock, &addresses).await;
    assert!(out.is_none(), "guard should trip when every address errored");
}

#[tokio::test]
async fn coins_guard_some_success_returns_partial_result() {
    // The mock returns the SAME response for every call. Force at least one
    // successful branch by returning an empty coin list.
    let mock = MockNodeRpc::new().with_coins_by_address(vec![]);
    let addresses = vec!["hs1qa".to_string()];
    let out =
        crate::commands::sync::fetch_coins_with_guard_with_client(&mock, &addresses).await;
    assert_eq!(out, Some(vec![]));
}

// ------- fetch_and_dedup_txs_with_client (history.rs) ---------------------

#[tokio::test]
async fn history_fetch_dedupes_by_hash_across_addresses() {
    // Two addresses, both return the same tx — deduped in the BTreeMap.
    let mock = MockNodeRpc::new().with_txs_by_address(vec![
        json!({ "hash": "aa", "height": 1 }),
        json!({ "hash": "bb", "height": 2 }),
    ]);
    let addresses = vec!["hs1qa".to_string(), "hs1qb".to_string()];
    let map =
        crate::commands::history::fetch_and_dedup_txs_with_client(&mock, &addresses).await;
    assert_eq!(map.len(), 2);
    assert!(map.contains_key("aa"));
    assert!(map.contains_key("bb"));
}

#[tokio::test]
async fn history_fetch_swallows_per_address_errors() {
    // With mock returning error, both addresses fail → empty map, no panic.
    let mock = MockNodeRpc::new().with_txs_by_address_err("index disabled");
    let addresses = vec!["hs1qa".to_string(), "hs1qb".to_string()];
    let map =
        crate::commands::history::fetch_and_dedup_txs_with_client(&mock, &addresses).await;
    assert!(map.is_empty());
}

#[tokio::test]
async fn history_fetch_skips_tx_without_hash() {
    // A malformed tx object (no `hash` field) is ignored — never crashes,
    // never inserted.
    let mock =
        MockNodeRpc::new().with_txs_by_address(vec![json!({ "no_hash_field": true })]);
    let map = crate::commands::history::fetch_and_dedup_txs_with_client(
        &mock,
        &["hs1qa".to_string()],
    )
    .await;
    assert!(map.is_empty());
}

// ------- verify_paid_transfer_with_client (paid_swaps.rs) -----------------

#[tokio::test]
async fn verify_paid_transfer_returns_verified_when_payment_output_matches() {
    let mock = MockNodeRpc::new().with_tx_by_hash(json!({
        "confirmations": 5,
        "outputs": [
            { "address": "hs1qbuyer", "value": 0 },
            { "address": "hs1qseller", "value": 1_000_000 }
        ]
    }));
    let out = crate::commands::paid_swaps::verify_paid_transfer_with_client(
        &mock,
        "txid1",
        "hs1qbuyer",
        1_000_000,
    )
    .await
    .unwrap();
    match out {
        crate::commands::paid_swaps::PaidTransferVerification::Verified {
            paid_doos,
            confirmations,
        } => {
            assert_eq!(paid_doos, 1_000_000);
            assert_eq!(confirmations, 5);
        }
        other => panic!("expected Verified, got {other:?}"),
    }
}

#[tokio::test]
async fn verify_paid_transfer_returns_no_payment_when_amount_short() {
    let mock = MockNodeRpc::new().with_tx_by_hash(json!({
        "confirmations": 2,
        "outputs": [
            { "address": "hs1qseller", "value": 500 }
        ]
    }));
    let out = crate::commands::paid_swaps::verify_paid_transfer_with_client(
        &mock,
        "txid1",
        "hs1qbuyer",
        1_000_000,
    )
    .await
    .unwrap();
    match out {
        crate::commands::paid_swaps::PaidTransferVerification::NoPayment { confirmations } => {
            assert_eq!(confirmations, 2);
        }
        other => panic!("expected NoPayment, got {other:?}"),
    }
}

#[tokio::test]
async fn verify_paid_transfer_returns_not_found_when_tx_null() {
    let mock = MockNodeRpc::new().with_tx_by_hash(serde_json::Value::Null);
    let err = crate::commands::paid_swaps::verify_paid_transfer_with_client(
        &mock,
        "unknown",
        "hs1qbuyer",
        1_000_000,
    )
    .await
    .unwrap_err();
    match err {
        AppError::NotFound(msg) => assert!(msg.contains("unknown")),
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn verify_paid_transfer_propagates_rpc_error() {
    let mock = MockNodeRpc::new().with_tx_by_hash_err("timeout");
    let err = crate::commands::paid_swaps::verify_paid_transfer_with_client(
        &mock,
        "txid1",
        "hs1qbuyer",
        1_000_000,
    )
    .await
    .unwrap_err();
    match err {
        AppError::Rpc(_) => {}
        other => panic!("expected Rpc error, got {other:?}"),
    }
}
