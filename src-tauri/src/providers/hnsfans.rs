//! External read-only adapter for the HNSFans explorer.
//!
//! This adapter is strictly read-only. It is used either as a fallback in
//! `auto_fallback` mode, or as the sole source in `external_read_only` mode.
//! It normalizes explorer responses into the same shapes the frontend already
//! consumes from a local hsd (`HsdBalance`, `HsdName`, transaction rows).
//!
//! The HNSFans API surface used here is intentionally small and defensive:
//! every field is treated as optional and missing data degrades gracefully
//! rather than failing the whole read.

use reqwest::Client;
use std::time::Duration;

use crate::error::AppError;
use crate::hsd::types::{HsdBalance, HsdName, HsdNameStats, HsdOwner};

pub struct HnsFansClient {
    http: Client,
    base_url: String,
}

impl HnsFansClient {
    pub fn new(base_url: &str) -> Self {
        let trimmed = base_url.trim_end_matches('/');
        let base = if trimmed.is_empty() {
            // Default to the explorer API host that serves the documented
            // `/api/addresses`, `/api/names`, and `/api/txs` routes.
            "https://e.hnsfans.com".to_string()
        } else {
            trimmed.to_string()
        };
        Self {
            http: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("failed to build HTTP client"),
            base_url: base,
        }
    }

    /// Lightweight health probe. Returns Ok(()) if the explorer host is
    /// reachable.
    ///
    /// The probe is intentionally lenient: the explorer is considered "up" as
    /// long as the host answers *any* HTTP response on a known endpoint. We do
    /// not require a 2xx because deployments differ — only a transport-level
    /// failure (DNS, connection refused, timeout) is treated as unhealthy.
    pub async fn health(&self) -> Result<(), AppError> {
        // Probe a lightweight, known route on the explorer API.
        let probe_url = format!("{}/api/txs?limit=1", self.base_url);
        if self.http.get(&probe_url).send().await.is_ok() {
            return Ok(());
        }
        // Fall back to the base URL. A reachable host (even a 4xx/5xx) means the
        // explorer is online; only a transport error is a real failure.
        match self.http.get(&self.base_url).send().await {
            Ok(_) => Ok(()),
            Err(e) => Err(AppError::Other(format!(
                "HNSFans is unreachable at {}: {}",
                self.base_url, e
            ))),
        }
    }

    /// Fetch the aggregate balance across a set of watch addresses.
    ///
    /// Uses the explorer's `/api/addresses/:address` endpoint, whose payload
    /// is `{ hash, received, spent, confirmed, unconfirmed }`.
    ///
    /// Failure handling is strict: if there are watch addresses but *every*
    /// per-address request fails (transport error or non-2xx), this returns an
    /// error rather than a misleading zero balance. A zero is only returned
    /// when the explorer genuinely reports zero for reachable addresses.
    pub async fn get_balance(&self, addresses: &[String]) -> Result<HsdBalance, AppError> {
        let mut confirmed: i64 = 0;
        let mut unconfirmed: i64 = 0;
        let mut attempted = 0usize;
        let mut succeeded = 0usize;
        let mut last_error: Option<String> = None;

        for address in addresses {
            let addr = address.trim();
            if addr.is_empty() {
                continue;
            }
            attempted += 1;
            let url = format!("{}/api/addresses/{}", self.base_url, addr);
            let resp = match self.http.get(&url).send().await {
                Ok(r) => r,
                Err(e) => {
                    last_error = Some(format!("{}: transport error: {}", addr, e));
                    continue;
                }
            };
            if !resp.status().is_success() {
                last_error = Some(format!("{}: HTTP {}", addr, resp.status()));
                continue;
            }
            let body: serde_json::Value = match resp.json().await {
                Ok(b) => b,
                Err(e) => {
                    last_error = Some(format!("{}: invalid JSON: {}", addr, e));
                    continue;
                }
            };
            succeeded += 1;
            confirmed += extract_amount(&body, &["confirmed", "balance", "confirmedBalance"]);
            unconfirmed +=
                extract_amount(&body, &["unconfirmed", "unconfirmedBalance", "pending"]);
        }

        // If we had addresses to check but none succeeded, this is a provider
        // failure, not a zero balance. Surface it loudly.
        if attempted > 0 && succeeded == 0 {
            return Err(AppError::Other(format!(
                "HNSFans balance lookup failed for all {} watched address(es); last error: {}",
                attempted,
                last_error.unwrap_or_else(|| "unknown".to_string())
            )));
        }

        Ok(HsdBalance {
            confirmed,
            unconfirmed,
            locked_unconfirmed: None,
            locked_confirmed: None,
        })
    }

    /// Resolve a set of names into the standard `HsdName` shape.
    ///
    /// The `e.hnsfans.com` explorer does not expose a "names owned by address"
    /// route, so name discovery by address is not possible in external
    /// read-only mode. Instead, names are resolved from the explicit
    /// `watch_names` list (typically backed by local inventory TLDs) via the
    /// per-name `/api/names/:name` endpoint.
    ///
    /// The `_addresses` argument is accepted for interface compatibility but is
    /// not used: the explorer cannot enumerate names for an address.
    pub async fn get_names(
        &self,
        _addresses: &[String],
        watch_names: &[String],
    ) -> Result<Vec<HsdName>, AppError> {
        let mut names: Vec<HsdName> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

        for watch in watch_names {
            let trimmed = watch.trim();
            if trimmed.is_empty() || seen.contains(trimmed) {
                continue;
            }
            if let Ok(name) = self.get_name_info(trimmed).await {
                if seen.insert(name.name.clone()) {
                    names.push(name);
                }
            }
        }

        Ok(names)
    }

    /// Fetch detail for a single name via `/api/names/:name`.
    pub async fn get_name_info(&self, name: &str) -> Result<HsdName, AppError> {
        let url = format!("{}/api/names/{}", self.base_url, name.trim());
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!(
                "HNSFans name lookup failed for {}: status {}",
                name,
                resp.status()
            )));
        }
        let body: serde_json::Value = resp.json().await?;
        normalize_name(&body)
            .ok_or_else(|| AppError::Other(format!("HNSFans returned no data for {}", name)))
    }

    /// Fetch recent transactions touching any watch address.
    ///
    /// The `e.hnsfans.com` explorer exposes `/api/txs/:hash` (single tx) and
    /// `/api/txs?limit&offset&height` (recent global txs), but it does **not**
    /// expose a "transactions for address" route. Address-scoped transaction
    /// history is therefore not available in external read-only mode, so this
    /// returns an empty list rather than querying a non-existent endpoint.
    ///
    /// The `_addresses` argument is kept for interface compatibility.
    pub async fn get_transactions(
        &self,
        _addresses: &[String],
    ) -> Result<serde_json::Value, AppError> {
        Ok(serde_json::Value::Array(Vec::new()))
    }

    // --- Owned-name discovery (node-free) ----------------------------------
    //
    // HNSFans has no "names owned by address" route, but it DOES expose enough
    // to reconstruct ownership without a node:
    //   * `/api/txs?address=&limit=&offset=` — the txs an address touched.
    //   * `/api/txs/:hash` — single-tx detail whose outputs are flattened with
    //     `action` + `name` + `address` (the covenant info the list endpoint and
    //     the `covenant` field omit).
    //   * `/api/names/:name/history` — newest-first covenant history; its latest
    //     entry's (txid,index) points at the current owner output.
    // The explorer rate-limits rapid requests (HTTP 403), so callers throttle
    // and treat any error mid-crawl as "stop this pass, keep what we have".

    /// One page of the txids an address participated in, plus the total count.
    pub async fn get_address_txids(
        &self,
        address: &str,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<String>, u64), AppError> {
        let url = format!(
            "{}/api/txs?address={}&limit={}&offset={}",
            self.base_url,
            address.trim(),
            limit,
            offset
        );
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!(
                "HNSFans txs lookup failed for {}: status {}",
                address,
                resp.status()
            )));
        }
        let body: serde_json::Value = resp.json().await?;
        let total = body.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
        let hashes = body
            .get("result")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| t.get("hash").and_then(|h| h.as_str()).map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok((hashes, total))
    }

    /// The name-bearing outputs of a single tx. Each carries the covenant
    /// `action`, the resolved `name`, the owning `address`, and its output
    /// `index` (position in the full output list).
    pub async fn get_tx_named_outputs(&self, hash: &str) -> Result<Vec<NamedOutput>, AppError> {
        let url = format!("{}/api/txs/{}", self.base_url, hash.trim());
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!(
                "HNSFans tx detail failed for {}: status {}",
                hash,
                resp.status()
            )));
        }
        let body: serde_json::Value = resp.json().await?;
        Ok(parse_named_outputs(&body))
    }

    /// The current owner outpoint `(txid, index)` of a name, from the newest
    /// entry of `/api/names/:name/history`. `None` if the name has no history.
    pub async fn get_name_current_owner(
        &self,
        name: &str,
    ) -> Result<Option<(String, u32)>, AppError> {
        let url = format!("{}/api/names/{}/history", self.base_url, name.trim());
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!(
                "HNSFans history failed for {}: status {}",
                name,
                resp.status()
            )));
        }
        let body: serde_json::Value = resp.json().await?;
        Ok(newest_owner_outpoint(&body))
    }
}

/// A name-bearing transaction output as flattened by `/api/txs/:hash`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedOutput {
    pub index: u32,
    pub action: String,
    pub name: String,
    pub address: String,
}

/// Extract name-bearing outputs from a `/api/txs/:hash` payload. The output
/// `index` is its position in the full `outputs` array.
fn parse_named_outputs(body: &serde_json::Value) -> Vec<NamedOutput> {
    let mut out = Vec::new();
    let Some(arr) = body.get("outputs").and_then(|v| v.as_array()) else {
        return out;
    };
    for (i, o) in arr.iter().enumerate() {
        let name = o.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let address = o.get("address").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() || address.is_empty() {
            continue;
        }
        out.push(NamedOutput {
            index: i as u32,
            action: o
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            name: name.to_string(),
            address: address.to_string(),
        });
    }
    out
}

/// The newest `(txid, index)` from a `/api/names/:name/history` payload, which
/// lists entries newest-first.
fn newest_owner_outpoint(body: &serde_json::Value) -> Option<(String, u32)> {
    let rows = body
        .get("result")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())?;
    for entry in rows {
        let txid = entry.get("txid").and_then(|v| v.as_str());
        let index = entry.get("index").and_then(|v| v.as_u64());
        if let (Some(txid), Some(index)) = (txid, index) {
            return Some((txid.to_string(), index as u32));
        }
    }
    None
}

fn extract_amount(body: &serde_json::Value, keys: &[&str]) -> i64 {
    for key in keys {
        if let Some(v) = body.get(*key) {
            if let Some(n) = v.as_i64() {
                return n;
            }
            if let Some(f) = v.as_f64() {
                return f.round() as i64;
            }
        }
    }
    0
}

/// Extract an array of name entries from a variety of wrapper shapes.
///
/// Retained as a defensive helper for parsing list-style name payloads; the
/// current `e.hnsfans.com` contract resolves names individually, so this is
/// only exercised by tests today.
#[cfg_attr(not(test), allow(dead_code))]
fn extract_name_array(body: &serde_json::Value) -> Vec<serde_json::Value> {
    let arr = body
        .get("result")
        .or_else(|| body.get("names"))
        .or_else(|| body.get("data"))
        .cloned()
        .unwrap_or_else(|| body.clone());
    match arr {
        serde_json::Value::Array(items) => items,
        _ => Vec::new(),
    }
}

/// Normalize a name payload into the standard `HsdName` shape.
///
/// This handles two payload variants:
///   * the `e.hnsfans.com` explorer shape, where the name hash is in `hash`,
///     `transfer` is a numeric block height (or `0`/null when not in transfer),
///     and `revoked` is a numeric flag (`0`/`1`);
///   * the older/object-style shape, where the hash is in `nameHash`, `owner`
///     is an object/string, and renewal stats live under `stats`.
fn normalize_name(entry: &serde_json::Value) -> Option<HsdName> {
    let name = entry
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())?;

    let owner = entry.get("owner").and_then(|o| {
        let hash = o
            .get("hash")
            .and_then(|v| v.as_str())
            .or_else(|| o.as_str())
            .map(|s| s.to_string());
        hash.map(|hash| HsdOwner {
            hash,
            index: o.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        })
    });

    let stats = entry.get("stats").map(|s| HsdNameStats {
        renewal_period_start: s.get("renewalPeriodStart").and_then(|v| v.as_u64()),
        renewal_period_end: s.get("renewalPeriodEnd").and_then(|v| v.as_u64()),
        blocks_until_expire: s.get("blocksUntilExpire").and_then(|v| v.as_i64()),
        days_until_expire: s.get("daysUntilExpire").and_then(|v| v.as_f64()),
    });

    // The explorer puts the name hash in `hash`; older payloads use `nameHash`.
    let name_hash = entry
        .get("nameHash")
        .and_then(|v| v.as_str())
        .or_else(|| entry.get("hash").and_then(|v| v.as_str()))
        .map(|s| s.to_string());

    // `transfer` may be a numeric block height (explorer) or a structured
    // object (older shape). Treat 0/null as "not transferring".
    let transfer = match entry.get("transfer") {
        Some(serde_json::Value::Number(n)) => {
            if n.as_i64() == Some(0) {
                None
            } else {
                Some(serde_json::Value::Number(n.clone()))
            }
        }
        Some(serde_json::Value::Null) | None => None,
        Some(other) => Some(other.clone()),
    };

    // `revoked` may be numeric (explorer: 0/1) or boolean (older shape).
    let revoked = entry.get("revoked").and_then(|v| {
        v.as_bool()
            .or_else(|| v.as_i64().map(|n| n != 0))
            .or_else(|| v.as_u64().map(|n| n != 0))
    });

    Some(HsdName {
        name,
        name_hash,
        state: entry
            .get("state")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        height: entry.get("height").and_then(|v| v.as_u64()),
        renewal: entry.get("renewal").and_then(|v| v.as_u64()),
        owner,
        value: entry.get("value").and_then(|v| v.as_u64()),
        highest: entry.get("highest").and_then(|v| v.as_u64()),
        registered: entry.get("registered").and_then(|v| v.as_bool()),
        expired: entry.get("expired").and_then(|v| v.as_bool()),
        stats,
        transfer,
        revoked,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn new_normalizes_base_url() {
        // Trailing slashes are trimmed.
        let client = HnsFansClient::new("https://example.com/");
        assert_eq!(client.base_url, "https://example.com");

        // Multiple trailing slashes are all trimmed.
        let client = HnsFansClient::new("https://example.com///");
        assert_eq!(client.base_url, "https://example.com");

        // Empty input falls back to the default explorer API host.
        let client = HnsFansClient::new("");
        assert_eq!(client.base_url, "https://e.hnsfans.com");

        // A normal URL is preserved verbatim.
        let client = HnsFansClient::new("https://my.node:1234");
        assert_eq!(client.base_url, "https://my.node:1234");
    }

    #[test]
    fn extract_amount_reads_first_matching_key() {
        let body = json!({ "balance": 500, "unconfirmed": 25 });
        assert_eq!(extract_amount(&body, &["confirmed", "balance"]), 500);
        assert_eq!(extract_amount(&body, &["unconfirmed"]), 25);
    }

    #[test]
    fn extract_amount_rounds_floats_and_defaults_to_zero() {
        let body = json!({ "confirmed": 12.7 });
        assert_eq!(extract_amount(&body, &["confirmed"]), 13);
        // No matching key -> 0.
        assert_eq!(extract_amount(&body, &["missing"]), 0);
    }

    #[test]
    fn extract_name_array_handles_wrappers_and_bare_arrays() {
        let wrapped = json!({ "result": [{ "name": "a" }, { "name": "b" }] });
        assert_eq!(extract_name_array(&wrapped).len(), 2);

        let names_key = json!({ "names": [{ "name": "c" }] });
        assert_eq!(extract_name_array(&names_key).len(), 1);

        let bare = json!([{ "name": "d" }]);
        assert_eq!(extract_name_array(&bare).len(), 1);

        // Non-array, non-wrapped payloads yield an empty vec.
        let scalar = json!({ "unexpected": true });
        assert!(extract_name_array(&scalar).is_empty());
    }

    #[test]
    fn normalize_name_requires_a_name_field() {
        // Without a `name` string, normalization fails (returns None).
        assert!(normalize_name(&json!({ "state": "OWNED" })).is_none());
    }

    #[test]
    fn normalize_name_maps_core_and_owner_fields() {
        let entry = json!({
            "name": "example",
            "nameHash": "deadbeef",
            "state": "CLOSED",
            "height": 100,
            "value": 1_000_000,
            "registered": true,
            "owner": { "hash": "ownerhash", "index": 2 },
            "stats": {
                "renewalPeriodStart": 10,
                "renewalPeriodEnd": 20,
                "blocksUntilExpire": 5,
                "daysUntilExpire": 1.5
            }
        });
        let name = normalize_name(&entry).expect("should normalize");
        assert_eq!(name.name, "example");
        assert_eq!(name.name_hash.as_deref(), Some("deadbeef"));
        assert_eq!(name.state.as_deref(), Some("CLOSED"));
        assert_eq!(name.height, Some(100));
        assert_eq!(name.value, Some(1_000_000));
        assert_eq!(name.registered, Some(true));
        let owner = name.owner.expect("owner present");
        assert_eq!(owner.hash, "ownerhash");
        assert_eq!(owner.index, 2);
        let stats = name.stats.expect("stats present");
        assert_eq!(stats.renewal_period_start, Some(10));
        assert_eq!(stats.renewal_period_end, Some(20));
        assert_eq!(stats.blocks_until_expire, Some(5));
        assert_eq!(stats.days_until_expire, Some(1.5));
    }

    #[test]
    fn normalize_name_accepts_owner_hash_as_string() {
        let entry = json!({ "name": "x", "owner": "barehash" });
        let name = normalize_name(&entry).expect("should normalize");
        let owner = name.owner.expect("owner present");
        assert_eq!(owner.hash, "barehash");
        // Missing index defaults to 0.
        assert_eq!(owner.index, 0);
    }

    #[test]
    fn normalize_name_handles_explorer_shape() {
        // Mirrors the e.hnsfans.com `/api/names/:name` payload: the name hash
        // lives in `hash`, `transfer` is a block height, `revoked` is 0/1.
        let entry = json!({
            "name": "examplename",
            "reserved": false,
            "hash": "1111111111111111111111111111111111111111111111111111111111111111",
            "state": "CLOSED",
            "height": 5040,
            "value": 400000,
            "renewal": 329999,
            "transfer": 335606,
            "revoked": 0
        });
        let name = normalize_name(&entry).expect("should normalize explorer shape");
        assert_eq!(name.name, "examplename");
        assert_eq!(
            name.name_hash.as_deref(),
            Some("1111111111111111111111111111111111111111111111111111111111111111")
        );
        assert_eq!(name.state.as_deref(), Some("CLOSED"));
        assert_eq!(name.height, Some(5040));
        assert_eq!(name.value, Some(400000));
        assert_eq!(name.renewal, Some(329999));
        // Non-zero transfer height is preserved.
        assert_eq!(name.transfer, Some(json!(335606)));
        // `revoked: 0` normalizes to false.
        assert_eq!(name.revoked, Some(false));
        // The explorer name shape has no owner object.
        assert!(name.owner.is_none());
    }

    #[test]
    fn normalize_name_treats_zero_transfer_as_none() {
        let entry = json!({ "name": "x", "transfer": 0, "revoked": 1 });
        let name = normalize_name(&entry).expect("should normalize");
        // A transfer height of 0 means "not transferring".
        assert!(name.transfer.is_none());
        // `revoked: 1` normalizes to true.
        assert_eq!(name.revoked, Some(true));
    }

    #[test]
    fn parse_named_outputs_keeps_only_named_with_true_index() {
        // Mirrors /api/txs/:hash: outputs are flattened with action+name+address;
        // plain (NONE) outputs carry no name and must be skipped, but the named
        // output keeps its true position in the full outputs array.
        let body = json!({
            "outputs": [
                { "address": "hs1qplain", "value": 1000 },
                { "action": "NONE", "address": "hs1qplain2", "value": 0 },
                { "action": "FINALIZE", "name": "examplename", "address": "hs1qowner", "value": 400000 }
            ]
        });
        let outs = parse_named_outputs(&body);
        assert_eq!(outs.len(), 1);
        assert_eq!(outs[0].index, 2);
        assert_eq!(outs[0].action, "FINALIZE");
        assert_eq!(outs[0].name, "examplename");
        assert_eq!(outs[0].address, "hs1qowner");
        // Missing outputs array -> empty.
        assert!(parse_named_outputs(&json!({})).is_empty());
    }

    #[test]
    fn newest_owner_outpoint_takes_first_history_entry() {
        // /api/names/:name/history is newest-first; take the first (txid,index).
        let body = json!({
            "result": [
                { "action": "Finalize", "txid": "aa", "index": 32 },
                { "action": "Transfer", "txid": "bb", "index": 0 }
            ]
        });
        assert_eq!(newest_owner_outpoint(&body), Some(("aa".to_string(), 32)));
        // Bare array form also supported.
        let bare = json!([{ "txid": "cc", "index": 1 }]);
        assert_eq!(newest_owner_outpoint(&bare), Some(("cc".to_string(), 1)));
        // Empty -> None.
        assert_eq!(newest_owner_outpoint(&json!({ "result": [] })), None);
    }

    #[test]
    fn get_balance_sums_explorer_payload() {
        // The explorer balance payload is { hash, received, spent, confirmed,
        // unconfirmed }; we read `confirmed`/`unconfirmed`.
        let body = json!({
            "hash": "hs1qexample",
            "received": 1000000,
            "spent": 0,
            "confirmed": 1000000,
            "unconfirmed": 0
        });
        assert_eq!(
            extract_amount(&body, &["confirmed", "balance", "confirmedBalance"]),
            1000000
        );
        assert_eq!(
            extract_amount(&body, &["unconfirmed", "unconfirmedBalance", "pending"]),
            0
        );
    }
}
