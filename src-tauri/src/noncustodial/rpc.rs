//! Node-only JSON-RPC client for the non-custodial signing engine.
//!
//! This client talks ONLY to the hsd **node** API (default port 12037 mainnet),
//! never the wallet API. The non-custodial engine holds its own keys and signs
//! locally; it uses the node purely to read chain state and broadcast already
//! -signed raw transactions.
//!
//! Verified against hsd:
//!   - Node RPC is `POST /` with body `{"method": "...", "params": [...]}` and
//!     HTTP Basic auth `x:<api-key>` (lib/node/http.js, bweb RPC mount).
//!   - Default node ports: 12037 main / 13037 testnet / 14037 regtest
//!     (lib/protocol/networks.js `ports.rpc`), matching skill reference.
//!   - JSON-RPC envelope: `{ "result": <value>, "error": <null|{message,code}>,
//!     "id": <n> }` (bcurl / brpc convention used by hsd).
//!   - `sendrawtransaction` takes a hex-encoded raw tx and returns the txid hex.
//!   - `getnameinfo` / `getnameresource` take `["name"]`.
//!   - `getcoinsbyaddress` requires the node's address index to be enabled
//!     (`--index-address`); callers must handle the empty/err case.

use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

use crate::error::AppError;

/// Where the engine reads chain state and broadcasts transactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainSource {
    /// A managed/local hsd node we control.
    LocalNode,
    /// A user-provided remote hsd node RPC endpoint.
    RemoteNode,
    /// A read-only block explorer. Broadcast is disabled in this mode.
    Explorer,
}

impl ChainSource {
    pub fn from_setting(value: &str) -> Self {
        match value {
            "remote_node" => ChainSource::RemoteNode,
            "explorer" => ChainSource::Explorer,
            // Default/fallback is the safest local option.
            _ => ChainSource::LocalNode,
        }
    }

    /// Whether this source can broadcast transactions via node RPC.
    pub fn can_broadcast(self) -> bool {
        matches!(self, ChainSource::LocalNode | ChainSource::RemoteNode)
    }
}

/// A node-only JSON-RPC client.
pub struct NodeRpcClient {
    http: Client,
    node_url: String,
    api_key: String,
    source: ChainSource,
}

/// The JSON-RPC envelope returned by hsd's node RPC.
#[derive(Debug, Deserialize)]
struct RpcEnvelope<T> {
    result: Option<T>,
    error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
struct RpcError {
    message: String,
    #[serde(default)]
    code: Option<i64>,
}

#[derive(Debug, Serialize)]
struct RpcRequest<'a> {
    method: &'a str,
    params: serde_json::Value,
}

impl NodeRpcClient {
    /// Construct a client against an explicit node URL / key / source.
    pub fn new(node_url: &str, api_key: &str, source: ChainSource) -> Self {
        Self {
            http: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("failed to build HTTP client"),
            node_url: node_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            source,
        }
    }

    /// Construct from the Phase 1 non-custodial settings map.
    ///
    /// Reads `node_rpc_url`, `node_rpc_api_key`, and `chain_source`.
    pub fn from_settings(settings: &HashMap<String, String>) -> Self {
        let url = settings
            .get("node_rpc_url")
            .map(|s| s.as_str())
            .unwrap_or("http://127.0.0.1:12037");
        let key = settings
            .get("node_rpc_api_key")
            .map(|s| s.as_str())
            .unwrap_or("");
        let source = ChainSource::from_setting(
            settings
                .get("chain_source")
                .map(|s| s.as_str())
                .unwrap_or("local_node"),
        );
        Self::new(url, key, source)
    }

    pub fn source(&self) -> ChainSource {
        self.source
    }

    /// Perform a JSON-RPC call and deserialize the `result` field into `T`.
    ///
    /// Returns `AppError::Rpc` for protocol-level errors (non-null `error`),
    /// `AppError::Http` for transport failures, and `AppError::Rpc` for a
    /// success envelope that is missing a `result`.
    async fn call<T: DeserializeOwned>(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<T, AppError> {
        let req = RpcRequest { method, params };
        let resp = self
            .http
            .post(&self.node_url)
            .basic_auth("x", Some(&self.api_key))
            .json(&req)
            .send()
            .await?;

        let status = resp.status();
        // hsd returns the JSON-RPC envelope even for some 4xx (e.g. method
        // errors), so parse the body before treating status as fatal.
        let body: serde_json::Value = resp.json().await.map_err(|e| {
            AppError::Rpc(format!("node returned non-JSON body (status {status}): {e}"))
        })?;

        let envelope: RpcEnvelope<T> = serde_json::from_value(body.clone())
            .map_err(|e| AppError::Rpc(format!("malformed RPC envelope: {e}; body={body}")))?;

        if let Some(err) = envelope.error {
            let code = err
                .code
                .map(|c| format!(" (code {c})"))
                .unwrap_or_default();
            return Err(AppError::Rpc(format!("{}{code}", err.message)));
        }

        envelope
            .result
            .ok_or_else(|| AppError::Rpc(format!("RPC '{method}' returned no result")))
    }

    // --- Chain reads -------------------------------------------------------

    /// `getblockchaininfo` — chain height, sync progress, network.
    pub async fn get_blockchain_info(&self) -> Result<BlockchainInfo, AppError> {
        self.call("getblockchaininfo", serde_json::json!([])).await
    }

    /// `getinfo` — general node info (version, network, height).
    pub async fn get_info(&self) -> Result<serde_json::Value, AppError> {
        self.call("getinfo", serde_json::json!([])).await
    }

    /// `getnameinfo` — on-chain name state (params: `["name"]`).
    pub async fn get_name_info(&self, name: &str) -> Result<serde_json::Value, AppError> {
        self.call("getnameinfo", serde_json::json!([name])).await
    }

    /// `getnameresource` — current DNS resource for a name (params: `["name"]`).
    pub async fn get_name_resource(&self, name: &str) -> Result<serde_json::Value, AppError> {
        self.call("getnameresource", serde_json::json!([name]))
            .await
    }

    /// `getcoinsbyaddress` — UTXOs for an address. Requires node address index
    /// (`--index-address`). Returns an empty list if the address has no coins.
    pub async fn get_coins_by_address(&self, address: &str) -> Result<Vec<NodeCoin>, AppError> {
        self.call("getcoinsbyaddress", serde_json::json!([address]))
            .await
    }

    /// `gettxout` — a single UTXO by `(txid, vout)`. Returns `None` if the
    /// output is unspent-unknown/spent (hsd yields null `result`).
    pub async fn get_tx_out(
        &self,
        txid: &str,
        index: u32,
    ) -> Result<Option<serde_json::Value>, AppError> {
        // includeMempool=true so freshly-broadcast outputs are visible.
        self.call("gettxout", serde_json::json!([txid, index, true]))
            .await
    }

    /// `getrawtransaction` with verbose=1 — full decoded tx by hash.
    pub async fn get_raw_transaction(&self, txid: &str) -> Result<serde_json::Value, AppError> {
        self.call("getrawtransaction", serde_json::json!([txid, 1]))
            .await
    }

    // --- Broadcast (write) -------------------------------------------------

    /// `sendrawtransaction` — broadcast an already-signed, hex-encoded tx.
    ///
    /// Returns the txid hex on success. Refuses to broadcast when the configured
    /// chain source is read-only (`Explorer`), so a misconfigured profile can
    /// never silently drop a signed transaction.
    pub async fn send_raw_transaction(&self, raw_tx_hex: &str) -> Result<String, AppError> {
        if !self.source.can_broadcast() {
            return Err(AppError::InvalidInput(
                "chain source is read-only; broadcasting is disabled".to_string(),
            ));
        }
        self.call("sendrawtransaction", serde_json::json!([raw_tx_hex]))
            .await
    }
}

/// Minimal typed view of `getblockchaininfo` (extra fields ignored).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockchainInfo {
    /// Current best chain height.
    pub blocks: i64,
    /// Header height (peers' best).
    #[serde(default)]
    pub headers: Option<i64>,
    /// 0.0..=1.0 sync progress.
    #[serde(default)]
    pub verification_progress: Option<f64>,
    /// Best block hash.
    #[serde(default)]
    pub bestblockhash: Option<String>,
}

/// Minimal typed view of a node coin from `getcoinsbyaddress`.
///
/// Only the fields the UTXO sync / draft builder depends on are typed; the rest
/// of hsd's coin shape is ignored by serde.
#[derive(Debug, Clone, Deserialize)]
pub struct NodeCoin {
    /// Funding transaction hash (hex).
    #[serde(rename = "hash")]
    pub txid: String,
    /// Output index within the funding tx.
    #[serde(rename = "index")]
    pub vout: u32,
    /// Value in dollarydoos.
    pub value: i64,
    /// Output script (hex).
    #[serde(default)]
    pub script: Option<String>,
    /// Address the coin pays to.
    #[serde(default)]
    pub address: Option<String>,
    /// Block height the coin was confirmed at (`-1`/absent = mempool).
    #[serde(default)]
    pub height: Option<i64>,
    /// Confirmations (0 = mempool).
    #[serde(default)]
    pub confirmations: Option<i64>,
    /// Whether the coin is part of a coinbase (maturity rules apply).
    #[serde(default)]
    pub coinbase: Option<bool>,
    /// Covenant attached to the output (name operations live here). hsd shapes
    /// this as `{ "type": <u8>, "action": "<NAME>", "items": ["<hex>", ...] }`.
    #[serde(default)]
    pub covenant: Option<NodeCovenant>,
}

/// Minimal typed view of an output covenant from a node coin.
///
/// `type` is the numeric covenant type (0 = NONE, others are name ops); the
/// `items` are the covenant's raw hex pushdata. Verified against hsd
/// `lib/covenants/covenant.js` JSON shape.
#[derive(Debug, Clone, Deserialize)]
pub struct NodeCovenant {
    /// Numeric covenant type (hsd `covenant.type`).
    #[serde(rename = "type")]
    pub kind: u8,
    /// Symbolic action name (e.g. "NONE", "OPEN", "BID", "REVEAL", ...).
    #[serde(default)]
    pub action: Option<String>,
    /// Raw covenant items as hex strings.
    #[serde(default)]
    pub items: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_source_parses_and_gates_broadcast() {
        assert_eq!(
            ChainSource::from_setting("local_node"),
            ChainSource::LocalNode
        );
        assert_eq!(
            ChainSource::from_setting("remote_node"),
            ChainSource::RemoteNode
        );
        assert_eq!(ChainSource::from_setting("explorer"), ChainSource::Explorer);
        // Unknown values fall back to the safe local default.
        assert_eq!(ChainSource::from_setting("garbage"), ChainSource::LocalNode);

        assert!(ChainSource::LocalNode.can_broadcast());
        assert!(ChainSource::RemoteNode.can_broadcast());
        assert!(!ChainSource::Explorer.can_broadcast());
    }

    #[test]
    fn from_settings_uses_defaults_when_missing() {
        let settings = HashMap::new();
        let client = NodeRpcClient::from_settings(&settings);
        assert_eq!(client.node_url, "http://127.0.0.1:12037");
        assert_eq!(client.api_key, "");
        assert_eq!(client.source, ChainSource::LocalNode);
    }

    #[test]
    fn from_settings_reads_overrides_and_trims_trailing_slash() {
        let mut settings = HashMap::new();
        settings.insert(
            "node_rpc_url".to_string(),
            "http://10.0.0.5:13037/".to_string(),
        );
        settings.insert("node_rpc_api_key".to_string(), "secret".to_string());
        settings.insert("chain_source".to_string(), "remote_node".to_string());
        let client = NodeRpcClient::from_settings(&settings);
        assert_eq!(client.node_url, "http://10.0.0.5:13037");
        assert_eq!(client.api_key, "secret");
        assert_eq!(client.source, ChainSource::RemoteNode);
    }

    #[tokio::test]
    async fn explorer_source_refuses_broadcast() {
        let client = NodeRpcClient::new("http://127.0.0.1:12037", "", ChainSource::Explorer);
        let err = client.send_raw_transaction("deadbeef").await.unwrap_err();
        match err {
            AppError::InvalidInput(msg) => assert!(msg.contains("read-only")),
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[test]
    fn blockchain_info_deserializes_minimal() {
        let json = serde_json::json!({
            "blocks": 12345,
            "headers": 12345,
            "verificationProgress": 0.9999,
            "bestblockhash": "abc123",
            "extraFieldWeIgnore": true
        });
        let info: BlockchainInfo = serde_json::from_value(json).unwrap();
        assert_eq!(info.blocks, 12345);
        assert_eq!(info.headers, Some(12345));
        assert_eq!(info.verification_progress, Some(0.9999));
        assert_eq!(info.bestblockhash.as_deref(), Some("abc123"));
    }

    #[test]
    fn node_coin_deserializes_with_renames() {
        let json = serde_json::json!({
            "version": 0,
            "height": 100,
            "value": 5000000,
            "address": "hs1qexample",
            "hash": "ffee00",
            "index": 2,
            "script": "0014abcd",
            "confirmations": 6,
            "coinbase": false
        });
        let coin: NodeCoin = serde_json::from_value(json).unwrap();
        assert_eq!(coin.txid, "ffee00");
        assert_eq!(coin.vout, 2);
        assert_eq!(coin.value, 5_000_000);
        assert_eq!(coin.address.as_deref(), Some("hs1qexample"));
        assert_eq!(coin.script.as_deref(), Some("0014abcd"));
        assert_eq!(coin.confirmations, Some(6));
        assert_eq!(coin.coinbase, Some(false));
    }

    #[test]
    fn rpc_envelope_error_parses() {
        let json = serde_json::json!({
            "result": null,
            "error": { "message": "Name not found.", "code": -1 },
            "id": 1
        });
        let env: RpcEnvelope<serde_json::Value> = serde_json::from_value(json).unwrap();
        assert!(env.result.is_none());
        let err = env.error.unwrap();
        assert_eq!(err.message, "Name not found.");
        assert_eq!(err.code, Some(-1));
    }
}
