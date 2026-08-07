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
//!   - address coins are fetched over REST (`GET /coin/address/:addr`), NOT
//!     JSON-RPC — hsd has no node `getcoinsbyaddress`. Requires the node's
//!     address index (`--index-address`); callers handle the empty/err case.

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
    /// An SPV (Simplified Payment Verification) node. Can broadcast but
    /// cannot serve full-chain queries (no --index-address/--index-tx).
    SpvNode,
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

    /// Build the chain source considering both chain_source and node_mode settings.
    pub fn from_settings(settings: &std::collections::HashMap<String, String>) -> Self {
        let chain_source = settings
            .get("chain_source")
            .map(|s| s.as_str())
            .unwrap_or("local_node");
        let node_mode = settings
            .get("node_mode")
            .map(|s| s.as_str())
            .unwrap_or("full");
        match (chain_source, node_mode) {
            ("remote_node", "spv") => ChainSource::SpvNode,
            ("remote_node", _) => ChainSource::RemoteNode,
            ("explorer", _) => ChainSource::Explorer,
            (_, "spv") => ChainSource::SpvNode,
            _ => ChainSource::LocalNode,
        }
    }

    /// Whether this source can broadcast transactions via node RPC.
    pub fn can_broadcast(self) -> bool {
        matches!(
            self,
            ChainSource::LocalNode | ChainSource::RemoteNode | ChainSource::SpvNode
        )
    }
}

/// Node operating mode — determines sync behavior and data sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeMode {
    /// Full node with --index-address --index-tx (current behavior).
    Full,
    /// SPV node with --spv (faster sync, explorer-dependent).
    Spv,
}

impl NodeMode {
    pub fn from_setting(value: &str) -> Self {
        match value {
            "spv" => NodeMode::Spv,
            _ => NodeMode::Full,
        }
    }

    pub fn is_spv(self) -> bool {
        matches!(self, NodeMode::Spv)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            NodeMode::Full => "full",
            NodeMode::Spv => "spv",
        }
    }
}

/// Resolve node_mode from settings map.
pub fn resolve_node_mode(settings: &std::collections::HashMap<String, String>) -> NodeMode {
    NodeMode::from_setting(
        settings
            .get("node_mode")
            .map(|s| s.as_str())
            .unwrap_or("full"),
    )
}

/// A node-only JSON-RPC client.
#[derive(Clone)]
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

/// Resolve the api-key to authenticate the node RPC with: the explicit
/// `node_rpc_api_key` setting when set, otherwise the `api-key` from the data
/// directory's `hsd.conf` (so the app talks to a node configured via hsd.conf
/// without the user re-entering the key). Empty when neither is present (a node
/// with no api-key needs none).
pub fn resolve_node_api_key(settings: &HashMap<String, String>) -> String {
    let explicit = settings
        .get("node_rpc_api_key")
        .map(|s| s.trim())
        .unwrap_or("");
    if !explicit.is_empty() {
        return explicit.to_string();
    }
    // Only consult hsd.conf when a data dir is explicitly configured, so a bare
    // settings map never touches the filesystem (and stays deterministic).
    let prefix = settings.get("hsd_prefix").map(|s| s.trim()).unwrap_or("");
    if prefix.is_empty() {
        return String::new();
    }
    read_hsd_conf_api_key(prefix).unwrap_or_default()
}

/// Parse `api-key: <value>` (or `api-key <value>`) from `<prefix>/hsd.conf`.
fn read_hsd_conf_api_key(prefix: &str) -> Option<String> {
    let conf = std::path::Path::new(prefix).join("hsd.conf");
    let text = std::fs::read_to_string(conf).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some(rest) = line.strip_prefix("api-key") else {
            continue;
        };
        // The separator must be ':' or whitespace so we don't match e.g.
        // "api-keys" or "api-key-foo".
        let value = if let Some(after) = rest.strip_prefix(':') {
            after.trim()
        } else if rest.starts_with(char::is_whitespace) {
            rest.trim()
        } else {
            continue;
        };
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

/// Reject configurations that would send `api_key` over plaintext HTTP to a
/// non-loopback host. An empty api-key is always allowed (nothing to leak).
/// `https://` is always allowed. Loopback `http://` (127.0.0.1/::1/localhost)
/// is allowed for the common local-node case. Remote `http://` with a key
/// present is refused.
pub fn guard_transport(node_url: &str, api_key: &str) -> Result<(), AppError> {
    if api_key.is_empty() {
        return Ok(());
    }
    let parsed = url::Url::parse(node_url.trim())
        .map_err(|e| AppError::InvalidInput(format!("node RPC URL is not a valid URL: {e}")))?;
    let scheme = parsed.scheme();
    if scheme == "https" {
        return Ok(());
    }
    if scheme != "http" {
        return Err(AppError::InvalidInput(format!(
            "node RPC URL scheme '{scheme}' is not supported"
        )));
    }
    // http scheme: only loopback is allowed when an api-key is set.
    let is_loopback = match parsed.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(a)) => a.is_loopback(),
        Some(url::Host::Ipv6(a)) => a.is_loopback(),
        None => false,
    };
    if is_loopback {
        Ok(())
    } else {
        Err(AppError::InvalidInput(
            "node RPC api-key must not be sent over plaintext HTTP to a remote host — use https:// or a loopback URL".to_string(),
        ))
    }
}

impl NodeRpcClient {
    /// Construct a client against an explicit node URL / key / source.
    pub fn new(node_url: &str, api_key: &str, source: ChainSource) -> Self {
        Self::try_new(node_url, api_key, source).unwrap_or_else(|_| Self {
            // Fallback for callers that ignore the guard: keep the invalid URL
            // but blank the api-key so it can never be sent in the clear. The
            // subsequent RPC call will fail loudly at request time.
            http: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("failed to build HTTP client"),
            node_url: node_url.trim_end_matches('/').to_string(),
            api_key: String::new(),
            source,
        })
    }

    /// Fallible constructor: rejects configurations that would send the api-key
    /// over plaintext HTTP to a non-loopback host. Loopback `http://` and any
    /// `https://` remain accepted.
    pub fn try_new(node_url: &str, api_key: &str, source: ChainSource) -> Result<Self, AppError> {
        guard_transport(node_url, api_key)?;
        Ok(Self {
            http: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("failed to build HTTP client"),
            node_url: node_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            source,
        })
    }

    /// Construct from the Phase 1 non-custodial settings map.
    ///
    /// Reads `node_rpc_url`, `node_rpc_api_key`, and `chain_source`.
    pub fn from_settings(settings: &HashMap<String, String>) -> Self {
        let url = settings
            .get("node_rpc_url")
            .map(|s| s.as_str())
            .unwrap_or("http://127.0.0.1:12037");
        let key = resolve_node_api_key(settings);
        let source = ChainSource::from_settings(settings);
        Self::new(url, &key, source)
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
            AppError::Rpc(format!(
                "node returned non-JSON body (status {status}): {e}"
            ))
        })?;

        let envelope: RpcEnvelope<T> = serde_json::from_value(body.clone())
            .map_err(|e| AppError::Rpc(format!("malformed RPC envelope: {e}; body={body}")))?;

        if let Some(err) = envelope.error {
            let code = err.code.map(|c| format!(" (code {c})")).unwrap_or_default();
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

    /// `getnamebyhash` — resolve a nameHash (hex) to its raw name string.
    ///
    /// Handshake stores nameHash-only covenants (REVEAL/REDEEM/REGISTER/UPDATE/
    /// RENEW/TRANSFER) whose payloads don't carry the plaintext name. This RPC
    /// is the way to recover the name from a hash — used when node-only owned
    /// name discovery scans wallet coins and needs to resolve their name.
    ///
    /// Returns `Ok(Some(name))` on success, `Ok(None)` when the node can't
    /// resolve the hash (unknown / not-yet-committed name), and `Err` on a
    /// transport-level failure. hsd may serialize an unresolved hash as either
    /// a JSON `null` result or an error envelope — both degrade to `None` so
    /// callers can fall back to the paired covenant's `rawName` when present.
    pub async fn get_name_by_hash(&self, name_hash_hex: &str) -> Result<Option<String>, AppError> {
        match self
            .call::<serde_json::Value>("getnamebyhash", serde_json::json!([name_hash_hex]))
            .await
        {
            Ok(serde_json::Value::String(name)) if !name.is_empty() => Ok(Some(name)),
            // Non-string / null / missing — hash is unknown to the node.
            Ok(_) => Ok(None),
            // "Method not found" and similar surface as Rpc errors: treat them
            // as "hash not resolvable" so the caller can fall through, rather
            // than aborting the whole discovery pass.
            Err(AppError::Rpc(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// `getnameresource` — current DNS resource for a name (params: `["name"]`).
    pub async fn get_name_resource(&self, name: &str) -> Result<serde_json::Value, AppError> {
        self.call("getnameresource", serde_json::json!([name]))
            .await
    }

    /// UTXOs for an address via the node REST route `GET /coin/address/:addr`.
    /// (hsd has NO `getcoinsbyaddress` JSON-RPC on the node — it's wallet-only;
    /// the node serves address coins over REST when `--index-address` is enabled.)
    /// Returns an empty list if the address has no coins; errors if the node
    /// rejects the request (e.g. address index disabled).
    pub async fn get_coins_by_address(&self, address: &str) -> Result<Vec<NodeCoin>, AppError> {
        let url = format!("{}/coin/address/{}", self.node_url, address);
        let resp = self
            .http
            .get(&url)
            .basic_auth("x", Some(&self.api_key))
            .send()
            .await?;
        let status = resp.status();
        let body: serde_json::Value = resp.json().await.map_err(|e| {
            AppError::Rpc(format!(
                "node returned non-JSON for coins (status {status}): {e}"
            ))
        })?;
        if !status.is_success() {
            // hsd surfaces failures as `{"error":{"message":…}}` (or `{"message":…}`),
            // e.g. when the address index isn't enabled.
            let msg = body
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .or_else(|| body.get("message").and_then(|m| m.as_str()))
                .unwrap_or("address coin lookup failed");
            return Err(AppError::Rpc(format!("{msg} (status {status})")));
        }
        serde_json::from_value(body)
            .map_err(|e| AppError::Rpc(format!("malformed coins response: {e}")))
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

    /// All transactions touching an address, via the node REST route
    /// `GET /tx/address/:addr`. Requires the node's transaction AND address
    /// indexes (`--index-tx` and `--index-address`) — hsd docs: "Allows
    /// lookup of all transactions involving a certain address."
    ///
    /// Each returned tx is the fully-decoded shape hsd emits for
    /// `GET /tx/:hash`: top-level `hash`, `height`, `time`, `mtime`,
    /// `confirmations`, `inputs[]` (with a resolved `coin { value, address,
    /// covenant, height }` for non-coinbase spends), and `outputs[]` (with
    /// `value`, `address`, `covenant { type, action, items }`). Unconfirmed
    /// txs surface as `height: -1`, `block: null`, `confirmations: 0`.
    ///
    /// Returns an empty vec when the address has no history. On non-2xx we
    /// try to parse hsd's `{error:{message}}` envelope and surface a specific
    /// "address index not enabled" error the frontend can gate on — mirrors
    /// the shape used by `get_coins_by_address`.
    pub async fn get_txs_by_address(
        &self,
        address: &str,
    ) -> Result<Vec<serde_json::Value>, AppError> {
        let url = format!("{}/tx/address/{}", self.node_url, address);
        let resp = self
            .http
            .get(&url)
            .basic_auth("x", Some(&self.api_key))
            .send()
            .await?;
        let status = resp.status();
        let body: serde_json::Value = resp.json().await.map_err(|e| {
            AppError::Rpc(format!(
                "node returned non-JSON for tx-by-address (status {status}): {e}"
            ))
        })?;
        if !status.is_success() {
            let msg = body
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .or_else(|| body.get("message").and_then(|m| m.as_str()))
                .unwrap_or("tx-by-address lookup failed");
            // hsd surfaces the index-disabled case with a message like
            // "Address indexing not enabled." — normalize it so the UI can
            // detect it uniformly across hsd versions.
            let lc = msg.to_ascii_lowercase();
            if lc.contains("address index")
                || lc.contains("indexing")
                || lc.contains("--index-address")
            {
                return Err(AppError::Rpc(format!(
                    "address index not enabled on this hsd node: {msg} (status {status})"
                )));
            }
            return Err(AppError::Rpc(format!("{msg} (status {status})")));
        }
        match body {
            serde_json::Value::Array(arr) => Ok(arr),
            // Some proxies wrap the array in `{ result: [...] }` — accept it.
            serde_json::Value::Object(mut obj) => {
                if let Some(serde_json::Value::Array(arr)) = obj.remove("result") {
                    Ok(arr)
                } else {
                    Err(AppError::Rpc(
                        "tx-by-address returned non-array body".to_string(),
                    ))
                }
            }
            _ => Err(AppError::Rpc(
                "tx-by-address returned non-array body".to_string(),
            )),
        }
    }

    /// `getrawtransaction` with verbose=1 — full decoded tx by hash.
    pub async fn get_raw_transaction(&self, txid: &str) -> Result<serde_json::Value, AppError> {
        self.call("getrawtransaction", serde_json::json!([txid, 1]))
            .await
    }

    /// Full decoded tx by hash via hsd's REST `GET /tx/:hash`.
    ///
    /// Unlike the JSON-RPC `getrawtransaction` verbose path, this route
    /// resolves historical prevouts through the tx-index, so `fee` and
    /// `inputs[].coin { value, address, covenant, height }` are populated
    /// even for confirmed txs whose inputs are already spent (the RPC path
    /// silently omits them because those UTXOs left the current coin set).
    ///
    /// Returns `Ok(Value::Null)` for an unknown tx (HTTP 404 or a JSON `null`
    /// body some hsd versions emit on miss). Requires the node's
    /// transaction index (`--index-tx`); an index-disabled error is
    /// normalized into an `AppError::Rpc` whose message contains the
    /// phrase `"tx index not enabled"` so callers can detect it uniformly
    /// across hsd versions (mirrors the address-index normalization above).
    pub async fn get_tx_by_hash(&self, txid: &str) -> Result<serde_json::Value, AppError> {
        let url = format!("{}/tx/{}", self.node_url, txid);
        let resp = self
            .http
            .get(&url)
            .basic_auth("x", Some(&self.api_key))
            .send()
            .await?;
        let status = resp.status();
        if status.as_u16() == 404 {
            return Ok(serde_json::Value::Null);
        }
        let body: serde_json::Value = resp.json().await.map_err(|e| {
            AppError::Rpc(format!(
                "node returned non-JSON for tx-by-hash (status {status}): {e}"
            ))
        })?;
        if !status.is_success() {
            let msg = body
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .or_else(|| body.get("message").and_then(|m| m.as_str()))
                .unwrap_or("tx-by-hash lookup failed");
            // Normalize the index-disabled error so callers can detect it
            // uniformly (matches the address-index pattern above).
            let lc = msg.to_ascii_lowercase();
            if lc.contains("transaction index")
                || lc.contains("tx index")
                || lc.contains("--index-tx")
            {
                return Err(AppError::Rpc(format!(
                    "tx index not enabled on this hsd node: {msg} (status {status})"
                )));
            }
            return Err(AppError::Rpc(format!("{msg} (status {status})")));
        }
        // Accept the bare tx object, a `{ result: {...} }` wrapper, or a
        // JSON `null` body (some hsd versions on miss — caller guards
        // via `.is_null()`).
        match body {
            serde_json::Value::Object(mut obj) => {
                if let Some(inner) = obj.remove("result") {
                    Ok(inner)
                } else {
                    Ok(serde_json::Value::Object(obj))
                }
            }
            other => Ok(other),
        }
    }

    /// `getblockhash` — the block hash (display-order hex) at `height`.
    pub async fn get_block_hash(&self, height: i64) -> Result<String, AppError> {
        self.call("getblockhash", serde_json::json!([height])).await
    }

    /// `getblock` (verbose + verboseTx) — full block with decoded transactions.
    ///
    /// hsd returns `{ hash, height, tx: [ { hash, inputs, outputs: [ { value,
    /// address: { hash, version, string }, covenant: { type, action, items } }
    /// ] } ] }` when called with `(hash, true, true)`. Used by the chain
    /// scanner to index BID/REVEAL covenants per block without a per-tx
    /// `getrawtransaction` roundtrip.
    pub async fn get_block(&self, hash: &str) -> Result<serde_json::Value, AppError> {
        self.call("getblock", serde_json::json!([hash, true, true]))
            .await
    }

    /// `generatetoaddress` — mine `nblocks` to `address`. Regtest/simnet only;
    /// used by the live-node integration tests to advance auction phases on
    /// demand. Returns the array of mined block hashes.
    pub async fn generate_to_address(
        &self,
        nblocks: u32,
        address: &str,
    ) -> Result<serde_json::Value, AppError> {
        self.call("generatetoaddress", serde_json::json!([nblocks, address]))
            .await
    }

    /// `stop` — ask the node to shut down gracefully. Works for any reachable
    /// node (one we spawned OR one the user started), unlike killing our child
    /// handle. The connection may drop as the node exits, so a transport error is
    /// treated as success.
    pub async fn stop(&self) -> Result<(), AppError> {
        match self
            .call::<serde_json::Value>("stop", serde_json::json!([]))
            .await
        {
            Ok(_) | Err(AppError::Http(_)) => Ok(()),
            Err(e) => Err(e),
        }
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

    /// `estimatesmartfee` — suggested fee rate, returned in **dollarydoos per
    /// byte** (floored to the 1 doo/byte relay minimum).
    ///
    /// hsd returns `{ "fee": <HNS per kvB>, "blocks": n }` (and some variants a
    /// bare number). Many nodes (regtest, freshly-synced) have no estimate and
    /// return a non-positive value or an error; callers must treat any error as
    /// "use the fixed default rate" rather than failing the operation.
    pub async fn estimate_smart_fee(&self, blocks: u32) -> Result<u64, AppError> {
        let v: serde_json::Value = self
            .call("estimatesmartfee", serde_json::json!([blocks]))
            .await?;
        let rate_hns_per_kvb = v
            .get("fee")
            .and_then(|f| f.as_f64())
            .or_else(|| v.as_f64())
            .ok_or_else(|| AppError::Rpc("estimatesmartfee: no fee in response".into()))?;
        if !(rate_hns_per_kvb.is_finite()) || rate_hns_per_kvb <= 0.0 {
            return Err(AppError::Rpc(
                "estimatesmartfee: no estimate available".into(),
            ));
        }
        // HNS/kvB -> doos/kvB (×1e6) -> doos/byte (÷1000). Floor at the relay
        // minimum (1 doo/byte, == send::MIN_FEE_RATE_PER_BYTE).
        let doos_per_byte = ((rate_hns_per_kvb * 1_000_000.0) / 1000.0).floor() as i64;
        Ok((doos_per_byte.max(1)) as u64)
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
    /// 0.0..=1.0 sync progress. hsd sends this all-lowercase
    /// (`verificationprogress`), which the struct's camelCase `rename_all` would
    /// otherwise miss — so name it explicitly.
    #[serde(default, rename = "verificationprogress")]
    pub verification_progress: Option<f64>,
    /// Best block hash.
    #[serde(default)]
    pub bestblockhash: Option<String>,
}

/// Minimal typed view of a node coin from `GET /coin/address/:addr`.
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
        // SPV mode is read-only in Namehold — no UTXO tracking, no sending.
        assert!(ChainSource::SpvNode.can_broadcast());
    }

    #[test]
    fn node_mode_from_setting() {
        assert_eq!(NodeMode::from_setting("spv"), NodeMode::Spv);
        assert_eq!(NodeMode::from_setting("full"), NodeMode::Full);
        // Unknown values default to Full.
        assert_eq!(NodeMode::from_setting("bogus"), NodeMode::Full);
        assert_eq!(NodeMode::from_setting(""), NodeMode::Full);
    }

    #[test]
    fn node_mode_is_spv() {
        assert!(NodeMode::Spv.is_spv());
        assert!(!NodeMode::Full.is_spv());
    }

    #[test]
    fn node_mode_as_str() {
        assert_eq!(NodeMode::Spv.as_str(), "spv");
        assert_eq!(NodeMode::Full.as_str(), "full");
    }

    #[test]
    fn resolve_node_mode_from_settings() {
        let mut settings = HashMap::new();
        assert_eq!(resolve_node_mode(&settings), NodeMode::Full); // default

        settings.insert("node_mode".to_string(), "spv".to_string());
        assert_eq!(resolve_node_mode(&settings), NodeMode::Spv);

        settings.insert("node_mode".to_string(), "full".to_string());
        assert_eq!(resolve_node_mode(&settings), NodeMode::Full);
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
            "https://10.0.0.5:13037/".to_string(),
        );
        settings.insert("node_rpc_api_key".to_string(), "secret".to_string());
        settings.insert("chain_source".to_string(), "remote_node".to_string());
        let client = NodeRpcClient::from_settings(&settings);
        assert_eq!(client.node_url, "https://10.0.0.5:13037");
        assert_eq!(client.api_key, "secret");
        assert_eq!(client.source, ChainSource::RemoteNode);
    }

    #[test]
    fn from_settings_spv_mode() {
        let mut settings = HashMap::new();
        settings.insert("node_mode".to_string(), "spv".to_string());
        // Default chain_source (local_node) + spv mode → SpvNode
        let client = NodeRpcClient::from_settings(&settings);
        assert_eq!(client.source, ChainSource::SpvNode);

        // Remote node + spv mode → SpvNode
        settings.insert("chain_source".to_string(), "remote_node".to_string());
        let client = NodeRpcClient::from_settings(&settings);
        assert_eq!(client.source, ChainSource::SpvNode);

        // Explorer + spv mode → Explorer (explorer overrides)
        settings.insert("chain_source".to_string(), "explorer".to_string());
        let client = NodeRpcClient::from_settings(&settings);
        assert_eq!(client.source, ChainSource::Explorer);
    }

    #[test]
    fn guard_transport_blocks_remote_http_with_key() {
        // Remote http + api-key is refused (would leak the key in cleartext).
        assert!(guard_transport("http://10.0.0.5:13037", "secret").is_err());
        // https to a remote host is fine.
        assert!(guard_transport("https://10.0.0.5:13037", "secret").is_ok());
        // Loopback http is fine (the common local-node case).
        assert!(guard_transport("http://127.0.0.1:12037", "secret").is_ok());
        assert!(guard_transport("http://localhost:12037", "secret").is_ok());
        // No api-key: nothing to leak, anything goes.
        assert!(guard_transport("http://10.0.0.5:13037", "").is_ok());
    }

    #[test]
    fn new_blanks_key_for_remote_http_to_avoid_cleartext_leak() {
        // `new` (infallible) must never keep an api-key it would send over
        // plaintext http to a remote host — it blanks it as a safe fallback.
        let client = NodeRpcClient::new("http://10.0.0.5:13037", "secret", ChainSource::RemoteNode);
        assert_eq!(client.api_key, "", "remote-http api-key must be dropped");
        // https keeps the key.
        let ok = NodeRpcClient::new("https://10.0.0.5:13037", "secret", ChainSource::RemoteNode);
        assert_eq!(ok.api_key, "secret");
    }

    #[test]
    fn guard_transport_allows_ipv6_loopback_http() {
        // [::1] is loopback — an api-key over http to it stays on the machine.
        assert!(guard_transport("http://[::1]:12037", "secret").is_ok());
    }

    #[test]
    fn guard_transport_rejects_scheme_other_than_http_or_https() {
        // A non-http(s) scheme with a key present is refused outright.
        assert!(guard_transport("ftp://10.0.0.5:13037", "secret").is_err());
        assert!(guard_transport("ws://127.0.0.1:12037", "secret").is_err());
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

    #[tokio::test]
    async fn estimate_smart_fee_scales_hns_per_kvb_to_doos_per_byte() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/")
            .with_header("content-type", "application/json")
            .with_body(r#"{"result":{"fee":0.1,"blocks":6},"error":null,"id":1}"#)
            .create_async()
            .await;
        let client = NodeRpcClient::new(&server.url(), "", ChainSource::LocalNode);
        // 0.1 HNS/kvB -> 100000 doos/kvB -> 100 doos/byte.
        assert_eq!(client.estimate_smart_fee(6).await.unwrap(), 100);
    }

    #[tokio::test]
    async fn estimate_smart_fee_errors_when_no_estimate() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/")
            .with_body(r#"{"result":{"fee":0,"blocks":6},"error":null,"id":1}"#)
            .create_async()
            .await;
        let client = NodeRpcClient::new(&server.url(), "", ChainSource::LocalNode);
        assert!(client.estimate_smart_fee(6).await.is_err());
    }

    #[test]
    fn blockchain_info_deserializes_minimal() {
        // Field names match hsd's actual getblockchaininfo output — note
        // `verificationprogress` is ALL lowercase (not camelCase).
        let json = serde_json::json!({
            "blocks": 12345,
            "headers": 12345,
            "verificationprogress": 0.9999,
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

    #[tokio::test]
    async fn txs_by_address_returns_decoded_array() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/tx/address/hs1qexample")
            .with_header("content-type", "application/json")
            .with_body(
                r#"[
                  {"hash":"aa","height":10,"time":1000,"confirmations":3,
                   "inputs":[{"prevout":{"hash":"pp","index":0},
                              "coin":{"value":500000000,"address":"hs1qmine",
                                      "covenant":{"type":0,"items":[]}}}],
                   "outputs":[{"value":100000000,"address":"hs1qdest",
                               "covenant":{"type":0,"action":"NONE","items":[]}}]}
                ]"#,
            )
            .create_async()
            .await;
        let client = NodeRpcClient::new(&server.url(), "", ChainSource::LocalNode);
        let txs = client.get_txs_by_address("hs1qexample").await.unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0]["hash"], "aa");
        assert_eq!(txs[0]["inputs"][0]["coin"]["address"], "hs1qmine");
    }

    #[tokio::test]
    async fn txs_by_address_empty_is_ok() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/tx/address/hs1qnone")
            .with_body("[]")
            .create_async()
            .await;
        let client = NodeRpcClient::new(&server.url(), "", ChainSource::LocalNode);
        let txs = client.get_txs_by_address("hs1qnone").await.unwrap();
        assert!(txs.is_empty());
    }

    #[tokio::test]
    async fn txs_by_address_flags_index_disabled() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/tx/address/hs1qx")
            .with_status(400)
            .with_body(r#"{"error":{"message":"Address indexing not enabled."}}"#)
            .create_async()
            .await;
        let client = NodeRpcClient::new(&server.url(), "", ChainSource::LocalNode);
        let err = client.get_txs_by_address("hs1qx").await.unwrap_err();
        match err {
            AppError::Rpc(msg) => assert!(
                msg.contains("address index not enabled"),
                "unexpected: {msg}"
            ),
            other => panic!("expected Rpc, got {other:?}"),
        }
    }

    // ── get_tx_by_hash (REST /tx/:hash) ──────────────────────────────────

    #[tokio::test]
    async fn tx_by_hash_returns_decoded_object_with_fee() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/tx/aabbccdd")
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"hash":"aabbccdd","height":10,"time":1000,"confirmations":3,
                    "fee":12000,
                    "inputs":[{"prevout":{"hash":"pp","index":0},
                               "coin":{"value":500000,"address":"hs1qmine",
                                       "covenant":{"type":0,"items":[]}}}],
                    "outputs":[{"value":488000,"address":"hs1qdest",
                                "covenant":{"type":0,"action":"NONE","items":[]}}]}"#,
            )
            .create_async()
            .await;
        let client = NodeRpcClient::new(&server.url(), "", ChainSource::LocalNode);
        let tx = client.get_tx_by_hash("aabbccdd").await.unwrap();
        assert_eq!(tx["hash"], "aabbccdd");
        assert_eq!(tx["fee"], 12000);
        assert_eq!(tx["inputs"][0]["coin"]["value"], 500000);
    }

    #[tokio::test]
    async fn tx_by_hash_computes_fee_from_coins_when_fee_absent() {
        // The exact bug case: hsd omits top-level `fee` but provides
        // resolved input coins. compute_tx_fee_and_total should derive it.
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/tx/nofee")
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"hash":"nofee","height":5,"time":500,"confirmations":1,
                    "inputs":[{"coin":{"value":1000}},{"coin":{"value":2000}}],
                    "outputs":[{"value":1500},{"value":1300}]}"#,
            )
            .create_async()
            .await;
        let client = NodeRpcClient::new(&server.url(), "", ChainSource::LocalNode);
        let tx = client.get_tx_by_hash("nofee").await.unwrap();
        assert!(!tx.is_null());
        // Fee should be computed as (1000+2000) - (1500+1300) = 200
        let (fee, total_out) = crate::commands::read::compute_tx_fee_and_total(&tx);
        assert_eq!(fee, Some(200));
        assert_eq!(total_out, 2800);
    }

    #[tokio::test]
    async fn tx_by_hash_404_returns_null() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/tx/unknown")
            .with_status(404)
            .with_body("")
            .create_async()
            .await;
        let client = NodeRpcClient::new(&server.url(), "", ChainSource::LocalNode);
        let result = client.get_tx_by_hash("unknown").await.unwrap();
        assert!(result.is_null());
    }

    #[tokio::test]
    async fn tx_by_hash_flags_index_disabled() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/tx/someid")
            .with_status(400)
            .with_body(r#"{"error":{"message":"Transaction indexing (--index-tx) not enabled."}}"#)
            .create_async()
            .await;
        let client = NodeRpcClient::new(&server.url(), "", ChainSource::LocalNode);
        let err = client.get_tx_by_hash("someid").await.unwrap_err();
        match err {
            AppError::Rpc(msg) => {
                assert!(msg.contains("tx index not enabled"), "unexpected: {msg}")
            }
            other => panic!("expected Rpc, got {other:?}"),
        }
    }
}
