//! Pure computation helpers extracted from `read.rs` — testable without RPC/DB.
//!
//! These functions operate on already-fetched JSON blobs (from hsd RPC) and
//! produce the final response object shape. Extraction means every guard,
//! fallback, and shape-mismatch branch gets a dedicated unit test.

use serde_json::{json, Value};

/// Build the compact block-info response object from an hsd verbose block.
///
/// Given the verbose-block JSON returned by `getblock(hash, verbose, verboseTx)`,
/// this computes:
/// - `minerReward` = sum of `tx[0].outputs[].value` (the coinbase). Missing or
///   malformed shapes yield `0`, never a panic.
/// - `txCount` = length of `tx` array (0 if missing).
/// - `height`, `time`, `difficulty` = pulled from the block with fallbacks
///   (`height` falls back to the caller-supplied `fallback_height`).
///
/// This is the pure back-half of `commands::read::read_block_info` — the async
/// front-half fetches the block via RPC, this shapes the response.
pub fn build_block_info(block: &Value, hash: &str, fallback_height: i64) -> Value {
    let txs = block.get("tx").and_then(|v| v.as_array());
    let tx_count = txs.map(|a| a.len()).unwrap_or(0);
    let miner_reward: i64 = txs
        .and_then(|a| a.first())
        .and_then(|coinbase| coinbase.get("outputs"))
        .and_then(|o| o.as_array())
        .map(|outs| {
            outs.iter()
                .filter_map(|out| out.get("value").and_then(|v| v.as_i64()))
                .sum()
        })
        .unwrap_or(0);

    let block_height = block
        .get("height")
        .and_then(|v| v.as_i64())
        .unwrap_or(fallback_height);
    let time = block.get("time").and_then(|v| v.as_i64()).unwrap_or(0);
    let difficulty = block
        .get("difficulty")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    json!({
        "height": block_height,
        "hash": hash,
        "time": time,
        "txCount": tx_count,
        "minerReward": miner_reward,
        "difficulty": difficulty,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typical_block() -> Value {
        json!({
            "height": 12345,
            "time": 1700000000,
            "difficulty": 42.5,
            "tx": [
                { "outputs": [ { "value": 2_000_000_000 }, { "value": 50_000 } ] },
                { "outputs": [ { "value": 100 } ] },
                { "outputs": [ { "value": 200 } ] }
            ]
        })
    }

    #[test]
    fn typical_block_info() {
        let info = build_block_info(&typical_block(), "abc123", 0);
        assert_eq!(info["height"], 12345);
        assert_eq!(info["hash"], "abc123");
        assert_eq!(info["time"], 1_700_000_000i64);
        assert_eq!(info["txCount"], 3);
        // miner reward is coinbase (tx[0]) outputs summed.
        assert_eq!(info["minerReward"], 2_000_050_000i64);
        assert_eq!(info["difficulty"], 42.5);
    }

    #[test]
    fn empty_tx_array_yields_zero_reward_and_count() {
        let block = json!({
            "height": 1,
            "time": 0,
            "difficulty": 1.0,
            "tx": []
        });
        let info = build_block_info(&block, "hash", 0);
        assert_eq!(info["txCount"], 0);
        assert_eq!(info["minerReward"], 0);
    }

    #[test]
    fn missing_tx_field_yields_zero() {
        let block = json!({ "height": 5, "time": 0, "difficulty": 1.0 });
        let info = build_block_info(&block, "h", 999);
        assert_eq!(info["txCount"], 0);
        assert_eq!(info["minerReward"], 0);
        // height comes from the block, not the fallback.
        assert_eq!(info["height"], 5);
    }

    #[test]
    fn height_falls_back_when_missing_on_block() {
        let block = json!({ "time": 0, "difficulty": 1.0, "tx": [] });
        let info = build_block_info(&block, "h", 42);
        assert_eq!(info["height"], 42);
    }

    #[test]
    fn coinbase_with_no_outputs_field_yields_zero_reward() {
        let block = json!({
            "height": 1,
            "tx": [ { "no_outputs_here": true }, { "outputs": [{"value": 99}] } ]
        });
        let info = build_block_info(&block, "h", 0);
        assert_eq!(info["minerReward"], 0);
        assert_eq!(info["txCount"], 2);
    }

    #[test]
    fn coinbase_output_with_no_value_field_is_skipped() {
        let block = json!({
            "height": 1,
            "tx": [ { "outputs": [
                { "value": 500 },
                { "no_value": true },  // filtered out
                { "value": 1500 }
            ] } ]
        });
        let info = build_block_info(&block, "h", 0);
        // 500 + 1500 = 2000, missing-value output silently skipped.
        assert_eq!(info["minerReward"], 2000);
    }

    #[test]
    fn missing_time_and_difficulty_defaults_to_zero() {
        let block = json!({ "height": 1, "tx": [] });
        let info = build_block_info(&block, "h", 0);
        assert_eq!(info["time"], 0);
        assert_eq!(info["difficulty"], 0.0);
    }

    #[test]
    fn tx_field_wrong_type_yields_zero() {
        // `tx` is a string, not an array → all guards fall through.
        let block = json!({ "height": 1, "tx": "not-an-array" });
        let info = build_block_info(&block, "h", 0);
        assert_eq!(info["txCount"], 0);
        assert_eq!(info["minerReward"], 0);
    }

    #[test]
    fn hash_is_passed_through_verbatim() {
        let block = json!({ "height": 1, "tx": [] });
        let info = build_block_info(&block, "0000abc...", 0);
        assert_eq!(info["hash"], "0000abc...");
    }
}
