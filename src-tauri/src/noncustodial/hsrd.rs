//! Authenticated client for the `hsrd` noncustodial wallet RPC v1 boundary.
//!
//! The sidecar supplies chain state, indexes, proofs, mempool evidence, fee
//! quotes, admission, and relay. It never receives seed material or private
//! keys and never signs a transaction.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;

use blake2::digest::{Update, VariableOutput};
use blake2::Blake2bVar;
use hns_covenants::{decode_name_state, hash_name, Resource, ResourceRecord};
use hns_primitives::{NameHash, TreeRoot};
use hns_transaction::Transaction;
use hns_urkel_proof::{ProofKind, UrkelProof};
use reqwest::header::AUTHORIZATION;
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::AppError;
use crate::noncustodial::address;
use crate::noncustodial::network::Network;

const API_VERSION: u16 = 1;
const WALLET_PATH: &str = "/api/v1/wallet";
const MAX_CURSOR_PAGES: usize = 16_384;

/// Where Namehold reaches the sidecar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainSource {
    ManagedSidecar,
    RemoteSidecar,
}

impl ChainSource {
    pub fn from_setting(value: &str) -> Self {
        match value {
            "remote_sidecar" => Self::RemoteSidecar,
            _ => Self::ManagedSidecar,
        }
    }

    pub const fn can_broadcast(self) -> bool {
        true
    }
}

/// Reject an Authorization value over cleartext transport to a non-loopback
/// host. The managed sidecar is expected to use loopback HTTP; remote endpoints
/// must use HTTPS.
pub fn guard_transport(endpoint: &str, authorization: &str) -> Result<(), AppError> {
    if authorization.is_empty() {
        return Ok(());
    }
    let parsed = url::Url::parse(endpoint.trim())
        .map_err(|e| AppError::InvalidInput(format!("sidecar RPC URL is invalid: {e}")))?;
    if parsed.scheme() == "https" {
        return Ok(());
    }
    if parsed.scheme() != "http" {
        return Err(AppError::InvalidInput(
            "sidecar RPC must use http or https".to_string(),
        ));
    }
    let loopback = match parsed.host() {
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    if loopback {
        Ok(())
    } else {
        Err(AppError::InvalidInput(
            "sidecar Authorization must not cross remote plaintext HTTP; use HTTPS".to_string(),
        ))
    }
}

/// Resolve the exact HTTP Authorization value configured for wallet RPC v1.
pub fn resolve_authorization(settings: &HashMap<String, String>) -> String {
    settings
        .get("hsrd_authorization")
        .map(|value| value.trim_end_matches(['\r', '\n']).to_string())
        .unwrap_or_default()
}

pub struct HsrdClient {
    http: Client,
    base_url: String,
    wallet_url: String,
    authorization: String,
    source: ChainSource,
    network: Network,
}

#[derive(Serialize)]
struct RpcRequest {
    api_version: u16,
    request_id: String,
    call: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RpcResponse<T> {
    api_version: u16,
    request_id: Option<String>,
    result: Option<T>,
    error: Option<RpcError>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RpcError {
    code: String,
    message: String,
    retryable: bool,
}

impl HsrdClient {
    pub fn new(endpoint: &str, authorization: &str, source: ChainSource) -> Self {
        Self::try_new(endpoint, authorization, source, Network::Main).unwrap_or_else(|_| {
            let base_url = endpoint.trim_end_matches('/').to_string();
            Self {
                http: Client::builder()
                    .timeout(Duration::from_secs(30))
                    .build()
                    .expect("HTTP client"),
                wallet_url: wallet_url(&base_url),
                base_url,
                authorization: String::new(),
                source,
                network: Network::Main,
            }
        })
    }

    pub fn try_new(
        endpoint: &str,
        authorization: &str,
        source: ChainSource,
        network: Network,
    ) -> Result<Self, AppError> {
        guard_transport(endpoint, authorization)?;
        let base_url = normalize_base_url(endpoint);
        Ok(Self {
            http: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|e| AppError::Other(format!("failed to build HTTP client: {e}")))?,
            wallet_url: wallet_url(&base_url),
            base_url,
            authorization: authorization.to_string(),
            source,
            network,
        })
    }

    pub fn from_settings(settings: &HashMap<String, String>) -> Self {
        let endpoint = settings
            .get("hsrd_rpc_url")
            .map(String::as_str)
            .unwrap_or("http://127.0.0.1:12037");
        let authorization = resolve_authorization(settings);
        let source = ChainSource::from_setting(
            settings
                .get("chain_source")
                .map(String::as_str)
                .unwrap_or("managed_sidecar"),
        );
        let network = settings
            .get("hsrd_network")
            .and_then(|value| Network::from_str_opt(value))
            .unwrap_or_default();
        Self::try_new(endpoint, &authorization, source, network)
            .unwrap_or_else(|_| Self::new(endpoint, "", source))
    }

    pub const fn source(&self) -> ChainSource {
        self.source
    }

    async fn rpc<T: DeserializeOwned>(&self, call: Value) -> Result<T, AppError> {
        #[cfg(test)]
        let request_id = "namehold-test".to_string();
        #[cfg(not(test))]
        let request_id = format!("namehold-{}", rand::random::<u64>());
        let request = RpcRequest {
            api_version: API_VERSION,
            request_id: request_id.clone(),
            call,
        };
        let mut builder = self.http.post(&self.wallet_url).json(&request);
        if !self.authorization.is_empty() {
            builder = builder.header(AUTHORIZATION, &self.authorization);
        }
        let response = builder.send().await?;
        let status = response.status();
        let bytes = response.bytes().await?;
        let envelope: RpcResponse<T> = serde_json::from_slice(&bytes).map_err(|_| {
            AppError::Rpc(format!(
                "sidecar returned a malformed wallet RPC response (HTTP {status})"
            ))
        })?;
        if envelope.api_version != API_VERSION
            || envelope.request_id.as_deref() != Some(request_id.as_str())
        {
            return Err(AppError::Rpc(
                "sidecar wallet RPC response binding is invalid".to_string(),
            ));
        }
        if let Some(error) = envelope.error {
            let retry = if error.retryable { " (retryable)" } else { "" };
            return Err(AppError::Rpc(format!(
                "{}: {}{}",
                error.code, error.message, retry
            )));
        }
        if !status.is_success() {
            return Err(AppError::Rpc(format!(
                "sidecar wallet RPC failed with HTTP {status}"
            )));
        }
        envelope
            .result
            .ok_or_else(|| AppError::Rpc("sidecar wallet RPC returned no result".to_string()))
    }

    async fn diagnostic(&self, path: &str) -> Result<Value, AppError> {
        let mut request = self.http.get(format!("{}{}", self.base_url, path));
        if !self.authorization.is_empty() {
            request = request.header(AUTHORIZATION, &self.authorization);
        }
        let response = request.send().await?;
        let status = response.status();
        if !status.is_success() {
            return Err(AppError::Rpc(format!(
                "sidecar diagnostic failed with HTTP {status}"
            )));
        }
        response
            .json()
            .await
            .map_err(|e| AppError::Rpc(format!("malformed sidecar diagnostic: {e}")))
    }

    pub async fn capabilities(&self) -> Result<Value, AppError> {
        self.rpc(json!({ "method": "capabilities" })).await
    }

    pub async fn get_blockchain_info(&self) -> Result<BlockchainInfo, AppError> {
        let tip: Option<WireTip> = self.rpc(json!({ "method": "chain_tip" })).await?;
        let tip = tip.ok_or_else(|| AppError::Rpc("sidecar has no active chain tip".into()))?;
        validate_hex_32(&tip.hash)?;
        validate_hex_32(&tip.tree_root)?;

        let sync = self.diagnostic("/api/v1/sync").await.ok();
        let target = sync
            .as_ref()
            .and_then(|value| value.get("target_height"))
            .and_then(Value::as_i64);
        let active = sync
            .as_ref()
            .and_then(|value| value.pointer("/active_tip/height"))
            .and_then(Value::as_i64)
            .unwrap_or(i64::from(tip.height));
        let progress = target
            .filter(|height| *height > 0)
            .map(|height| (active as f64 / height as f64).clamp(0.0, 1.0));
        Ok(BlockchainInfo {
            blocks: active,
            headers: target,
            verification_progress: progress,
            bestblockhash: Some(tip.hash),
        })
    }

    pub async fn get_info(&self) -> Result<Value, AppError> {
        self.diagnostic("/api/v1/status").await
    }

    pub async fn get_name_info(&self, name: &str) -> Result<Value, AppError> {
        let name_hash = hash_name(name.as_bytes())
            .map_err(|e| AppError::InvalidInput(format!("invalid Handshake name: {e}")))?;
        let binding = self.chain_binding().await?;
        let evidence = self.name_evidence(name_hash, binding.chain_epoch).await?;
        let Some(state) = evidence.current_state else {
            return Ok(json!({
                "info": null
            }));
        };
        let decoded_name = decode_utf8_hex(&state.name_hex, "name")?;
        if decoded_name != name || state.name_hash != name_hash.to_string() {
            return Err(AppError::Rpc(
                "sidecar returned name evidence for a different name".into(),
            ));
        }
        let phase = auction_phase(&state, binding.tip.height, self.network);
        Ok(json!({
            "info": {
                "name": decoded_name,
                "nameHash": state.name_hash,
                "state": phase,
                "height": state.height,
                "renewal": state.renewal,
                "owner": { "hash": state.owner.txid, "index": state.owner.index },
                "value": state.value,
                "highest": state.highest,
                "registered": state.registered,
                "expired": state.expired,
                "transfer": state.transfer,
                "revoked": state.revoked != 0,
                "claimed": state.claimed,
                "renewals": state.renewals,
                "weak": state.weak,
                "stats": auction_stats(&state, binding.tip.height, self.network),
                "evidence": {
                    "chainEpoch": evidence.chain_epoch,
                    "tip": evidence.tip,
                    "currentStateHex": evidence.current_state_hex,
                    "proofStateHex": evidence.proof_state_hex,
                    "proof": evidence.proof
                }
            }
        }))
    }

    pub async fn get_name_by_hash(&self, name_hash_hex: &str) -> Result<Option<String>, AppError> {
        validate_hex_32(name_hash_hex)?;
        let name_hash = NameHash::from_hex(name_hash_hex)
            .map_err(|_| AppError::InvalidInput("invalid name hash".into()))?;
        let binding = self.chain_binding().await?;
        let evidence = self.name_evidence(name_hash, binding.chain_epoch).await?;
        evidence
            .current_state
            .map(|state| decode_utf8_hex(&state.name_hex, "name"))
            .transpose()
    }

    pub async fn get_name_resource(&self, name: &str) -> Result<Value, AppError> {
        let name_hash = hash_name(name.as_bytes())
            .map_err(|e| AppError::InvalidInput(format!("invalid Handshake name: {e}")))?;
        let binding = self.chain_binding().await?;
        let evidence = self.name_evidence(name_hash, binding.chain_epoch).await?;
        let Some(state) = evidence.current_state else {
            return Ok(Value::Null);
        };
        let raw = decode_hex(&state.data_hex, 512, "name resource")?;
        if raw.is_empty() {
            return Ok(json!({ "records": [] }));
        }
        let resource = Resource::decode(&raw)
            .map_err(|e| AppError::Rpc(format!("invalid authenticated name resource: {e}")))?;
        Ok(json!({
            "records": resource.records().iter().map(resource_record_json).collect::<Vec<_>>()
        }))
    }

    pub async fn get_coins_by_address(
        &self,
        address_text: &str,
    ) -> Result<Vec<NodeCoin>, AppError> {
        let restored = self.restore_address(address_text).await?;
        let mut coins = BTreeMap::<(String, u32), NodeCoin>::new();
        for entry in restored.confirmed_utxos {
            coins.insert(
                (entry.coin.outpoint.txid.clone(), entry.coin.outpoint.index),
                node_coin(entry.coin, address_text, restored.tip.height)?,
            );
        }

        for activity in restored.mempool_entries {
            for spend in activity.spent {
                coins.remove(&(spend.outpoint.txid, spend.outpoint.index));
            }
            for received in activity.received {
                let binding = SnapshotBinding {
                    chain_epoch: restored.chain_epoch,
                    tip: restored.tip.clone(),
                };
                let evidence = self
                    .transaction_evidence(&activity.txid, &binding, Some(&restored.mempool))
                    .await?;
                let Some(raw) = evidence.transaction_hex else {
                    return Err(AppError::Rpc(
                        "sidecar omitted a mempool transaction payload".into(),
                    ));
                };
                let transaction = decode_transaction(&raw)?;
                let output = transaction
                    .outputs
                    .get(received.outpoint.index as usize)
                    .ok_or_else(|| AppError::Rpc("mempool outpoint is out of range".into()))?;
                let coin = NodeCoin {
                    txid: received.outpoint.txid.clone(),
                    vout: received.outpoint.index,
                    value: i64::try_from(output.value.get())
                        .map_err(|_| AppError::Rpc("coin value is out of range".into()))?,
                    script: None,
                    address: Some(address_text.to_string()),
                    height: Some(-1),
                    confirmations: Some(0),
                    coinbase: Some(false),
                    covenant: Some(NodeCovenant {
                        kind: output.covenant.kind.as_u8(),
                        action: Some(covenant_action(output.covenant.kind.as_u8()).to_string()),
                        items: output.covenant.items.iter().map(hex::encode).collect(),
                    }),
                };
                coins.insert((coin.txid.clone(), coin.vout), coin);
            }
        }
        Ok(coins.into_values().collect())
    }

    pub async fn get_txs_by_address(&self, address_text: &str) -> Result<Vec<Value>, AppError> {
        let restored = self.restore_address(address_text).await?;
        let mut txids = BTreeSet::new();
        for row in &restored.confirmed_history {
            txids.insert(row.txid.clone());
        }
        for row in &restored.mempool_entries {
            txids.insert(row.txid.clone());
        }

        let confirmed = restored
            .confirmed_history
            .iter()
            .map(|entry| (entry.txid.clone(), entry.clone()))
            .collect::<BTreeMap<_, _>>();
        let admitted = restored
            .mempool_entries
            .iter()
            .map(|entry| (entry.txid.clone(), entry.admitted_at))
            .collect::<BTreeMap<_, _>>();
        let binding = SnapshotBinding {
            chain_epoch: restored.chain_epoch,
            tip: restored.tip.clone(),
        };
        let mut transactions = Vec::with_capacity(txids.len());
        for txid in txids {
            let evidence = self
                .transaction_evidence(&txid, &binding, Some(&restored.mempool))
                .await?;
            let Some(raw) = evidence.transaction_hex else {
                continue;
            };
            let tx = decode_transaction(&raw)?;
            let row = confirmed.get(&txid);
            transactions.push(transaction_json(
                &tx,
                &txid,
                row.map(|item| i64::from(item.height)).unwrap_or(-1),
                row.and_then(|item| item.block_time)
                    .or_else(|| admitted.get(&txid).copied()),
                row.map(|item| restored.tip.height.saturating_sub(item.height) + 1)
                    .unwrap_or(0),
            ));
        }
        Ok(transactions)
    }

    pub async fn get_raw_transaction(&self, txid: &str) -> Result<Value, AppError> {
        self.get_tx_by_hash(txid).await
    }

    pub async fn get_tx_by_hash(&self, txid: &str) -> Result<Value, AppError> {
        validate_hex_32(txid)?;
        let binding = self.chain_binding().await?;
        let evidence = self.transaction_evidence(txid, &binding, None).await?;
        if evidence.status == "unknown" {
            return Err(AppError::Rpc(
                "transaction is unknown to the sidecar".into(),
            ));
        }
        let Some(raw) = evidence.transaction_hex else {
            return Err(AppError::Rpc(
                "sidecar did not retain the requested transaction payload".into(),
            ));
        };
        let transaction = decode_transaction(&raw)?;
        let inclusion = evidence.inclusion;
        Ok(transaction_json(
            &transaction,
            txid,
            inclusion
                .as_ref()
                .map(|value| i64::from(value.height))
                .unwrap_or(-1),
            None,
            inclusion.map(|value| value.confirmations).unwrap_or(0),
        ))
    }

    pub async fn get_tx_out(&self, txid: &str, index: u32) -> Result<Option<Value>, AppError> {
        validate_hex_32(txid)?;
        let (binding, mempool) = self.snapshot().await?;
        let spending: WireSpendingEvidence = self
            .rpc(json!({
                "method": "spending_transaction",
                "params": {
                    "txid": txid,
                    "output_index": index,
                    "expected_chain_epoch": binding.chain_epoch
                }
            }))
            .await?;
        require_binding(spending.chain_epoch, spending.tip.as_ref(), &binding)?;
        let spent = spending
            .entries
            .first()
            .ok_or_else(|| AppError::Rpc("sidecar omitted outpoint evidence".into()))?
            .spending
            .is_some();
        if spent {
            return Ok(None);
        }
        let evidence = self
            .transaction_evidence(txid, &binding, Some(&mempool))
            .await?;
        let Some(raw) = evidence.transaction_hex else {
            return Ok(None);
        };
        let transaction = decode_transaction(&raw)?;
        let Some(output) = transaction.outputs.get(index as usize) else {
            return Ok(None);
        };
        Ok(Some(output_json(output)))
    }

    pub async fn get_block_hash(&self, height: i64) -> Result<String, AppError> {
        let height = u32::try_from(height)
            .map_err(|_| AppError::InvalidInput("block height is out of range".into()))?;
        let binding = self.chain_binding().await?;
        let response: WireBlockHashEvidence = self
            .rpc(json!({
                "method": "block_hash",
                "params": {
                    "height": height,
                    "expected_chain_epoch": binding.chain_epoch
                }
            }))
            .await?;
        require_binding(response.chain_epoch, response.tip.as_ref(), &binding)?;
        response
            .hash
            .ok_or_else(|| AppError::Rpc("block height is not on the active chain".into()))
    }

    pub async fn send_raw_transaction(&self, raw_tx_hex: &str) -> Result<String, AppError> {
        if !self.source.can_broadcast() {
            return Err(AppError::InvalidInput("broadcast is disabled".into()));
        }
        let transaction = decode_transaction(raw_tx_hex)?;
        if hex::encode(
            transaction
                .encode()
                .map_err(|e| AppError::InvalidInput(e.to_string()))?,
        ) != raw_tx_hex.to_ascii_lowercase()
        {
            return Err(AppError::InvalidInput(
                "transaction is not canonically encoded".into(),
            ));
        }
        let result: WireBroadcast = self
            .rpc(json!({
                "method": "broadcast_transaction",
                "params": { "transaction_hex": raw_tx_hex }
            }))
            .await?;
        if result.attempted_peers != result.queued_peers + result.failed_peers {
            return Err(AppError::Rpc(
                "sidecar returned inconsistent relay accounting".into(),
            ));
        }
        Ok(result.txid)
    }

    /// Return the sidecar estimate in atomic units per policy virtual byte for
    /// compatibility with Namehold's draft planner. Final signed artifacts are
    /// always checked with `quote_transaction_fee` before broadcast.
    pub async fn estimate_smart_fee(&self, blocks: u32) -> Result<u64, AppError> {
        let estimate: WireFeeEstimate = self
            .rpc(json!({
                "method": "estimate_fee_rate",
                "params": { "target_blocks": blocks }
            }))
            .await?;
        if estimate.target_blocks != blocks || estimate.atomic_units_per_kvb == 0 {
            return Err(AppError::Rpc(
                "sidecar returned an invalid fee estimate".into(),
            ));
        }
        Ok(estimate.atomic_units_per_kvb.div_ceil(1_000))
    }

    pub async fn quote_transaction_fee(
        &self,
        raw_tx_hex: &str,
        target_blocks: u32,
    ) -> Result<TransactionFeeQuote, AppError> {
        decode_transaction(raw_tx_hex)?;
        let (binding, mempool) = self.snapshot().await?;
        let quote: TransactionFeeQuote = self
            .rpc(json!({
                "method": "quote_transaction_fee",
                "params": {
                    "transaction_hex": raw_tx_hex,
                    "target_blocks": target_blocks,
                    "expected_chain_epoch": binding.chain_epoch,
                    "expected_mempool": {
                        "instance_nonce": mempool.instance_nonce,
                        "generation": mempool.generation
                    }
                }
            }))
            .await?;
        if quote.chain_epoch != binding.chain_epoch
            || quote.tip.as_ref() != Some(&binding.tip)
            || quote.mempool_instance_nonce != mempool.instance_nonce
            || quote.mempool_generation != mempool.generation
        {
            return Err(AppError::Rpc(
                "fee quote is bound to a stale snapshot".into(),
            ));
        }
        Ok(quote)
    }

    pub async fn tracked_contract_known(&self, contract_id: &str) -> Result<bool, AppError> {
        validate_hex_32(contract_id)?;
        let result: WireTrackedContractKnown = self
            .rpc(json!({
                "method": "tracked_contract_known",
                "params": { "contract_id": contract_id }
            }))
            .await?;
        if result.contract_id != contract_id
            || result.descriptor != "opaque_unpublished_protocol_boundary"
        {
            return Err(AppError::Rpc("tracked-contract identity is invalid".into()));
        }
        Ok(result.known)
    }

    pub async fn tracked_contract_fundings(
        &self,
        contract_id: &str,
    ) -> Result<Vec<Value>, AppError> {
        self.tracked_contract_pages(contract_id, "tracked_contract_fundings", false)
            .await
    }

    pub async fn tracked_contract_events(&self, contract_id: &str) -> Result<Vec<Value>, AppError> {
        self.tracked_contract_pages(contract_id, "tracked_contract_events", false)
            .await
    }

    pub async fn mempool_tracked_contract(
        &self,
        contract_id: &str,
    ) -> Result<Vec<Value>, AppError> {
        self.tracked_contract_pages(contract_id, "mempool_tracked_contract", true)
            .await
    }

    async fn tracked_contract_pages(
        &self,
        contract_id: &str,
        method: &str,
        mempool: bool,
    ) -> Result<Vec<Value>, AppError> {
        validate_hex_32(contract_id)?;
        let (binding, expected_mempool) = self.snapshot().await?;
        let mut cursor: Option<String> = None;
        let mut entries = Vec::new();
        for _ in 0..MAX_CURSOR_PAGES {
            let limit = if mempool {
                json!({ "scan_limit": 1024 })
            } else {
                json!({ "limit": 256 })
            };
            let mut params = json!({
                "contract_id": contract_id,
                "expected_chain_epoch": binding.chain_epoch,
                "cursor": cursor.clone()
            });
            params
                .as_object_mut()
                .expect("JSON object")
                .extend(limit.as_object().expect("JSON object").clone());
            let page: WireTrackedContractPage = self
                .rpc(json!({ "method": method, "params": params }))
                .await?;
            require_binding(page.chain_epoch, page.tip.as_ref(), &binding)?;
            if mempool {
                if page.instance_nonce.as_deref() != Some(&expected_mempool.instance_nonce)
                    || page.generation != Some(expected_mempool.generation)
                    || page.preimage_transport.as_deref() != Some("opaque_unavailable")
                {
                    return Err(AppError::Rpc(
                        "tracked-contract mempool evidence is stale or malformed".into(),
                    ));
                }
            } else if method == "tracked_contract_events"
                && page.preimage_transport.as_deref() != Some("opaque_unavailable")
            {
                return Err(AppError::Rpc(
                    "tracked-contract preimage boundary is malformed".into(),
                ));
            }
            entries.extend(page.entries);
            cursor = page.continuation;
            if cursor.is_none() {
                return Ok(entries);
            }
        }
        Err(AppError::Rpc(
            "tracked-contract restoration exceeded page bound".into(),
        ))
    }

    async fn chain_binding(&self) -> Result<SnapshotBinding, AppError> {
        let confirmed: WireConfirmedPage = self
            .rpc(json!({
                "method": "confirmed_scripts_page",
                "params": {
                    "script_ids": ["0000000000000000000000000000000000000000000000000000000000000000"],
                    "cursor": null,
                    "limit": 1
                }
            }))
            .await?;
        let tip = confirmed
            .tip
            .ok_or_else(|| AppError::Rpc("sidecar has no active chain tip".into()))?;
        validate_hex_32(&tip.hash)?;
        validate_hex_32(&tip.tree_root)?;
        Ok(SnapshotBinding {
            chain_epoch: confirmed.chain_epoch,
            tip,
        })
    }

    async fn snapshot(&self) -> Result<(SnapshotBinding, MempoolBinding), AppError> {
        let confirmed: WireConfirmedPage = self
            .rpc(json!({
                "method": "confirmed_scripts_page",
                "params": {
                    "script_ids": ["0000000000000000000000000000000000000000000000000000000000000000"],
                    "cursor": null,
                    "limit": 1
                }
            }))
            .await?;
        let tip = confirmed
            .tip
            .ok_or_else(|| AppError::Rpc("sidecar has no active chain tip".into()))?;
        let binding = SnapshotBinding {
            chain_epoch: confirmed.chain_epoch,
            tip,
        };
        let mempool: WireMempoolPage = self
            .rpc(json!({
                "method": "mempool_scripts_page",
                "params": {
                    "script_ids": ["0000000000000000000000000000000000000000000000000000000000000000"],
                    "expected_chain_epoch": binding.chain_epoch,
                    "cursor": null,
                    "scan_limit": 1
                }
            }))
            .await?;
        require_binding(mempool.chain_epoch, mempool.tip.as_ref(), &binding)?;
        validate_hex_32(&mempool.instance_nonce)?;
        Ok((
            binding,
            MempoolBinding {
                instance_nonce: mempool.instance_nonce,
                generation: mempool.generation,
            },
        ))
    }

    async fn restore_address(&self, address_text: &str) -> Result<RestoredAddress, AppError> {
        let (version, hash) = address::decode(self.network, address_text)?;
        let script_id = script_id(version, &hash)?;
        let script_ids = [hex::encode(script_id)];
        let mut cursor: Option<String> = None;
        let mut confirmed_history = Vec::new();
        let mut confirmed_utxos = Vec::new();
        let mut binding: Option<SnapshotBinding> = None;
        for _ in 0..MAX_CURSOR_PAGES {
            let page: WireConfirmedPage = self
                .rpc(json!({
                    "method": "confirmed_scripts_page",
                    "params": {
                        "script_ids": &script_ids,
                        "cursor": cursor.clone(),
                        "limit": 256
                    }
                }))
                .await?;
            let page_binding = SnapshotBinding {
                chain_epoch: page.chain_epoch,
                tip: page.tip.clone().ok_or_else(|| {
                    AppError::Rpc("sidecar restoration has no active chain tip".into())
                })?,
            };
            if let Some(expected) = &binding {
                if expected != &page_binding {
                    return Err(AppError::Rpc("chain changed during restoration".into()));
                }
            } else {
                binding = Some(page_binding);
            }
            confirmed_history.extend(page.history);
            confirmed_utxos.extend(page.utxos);
            cursor = page.continuation;
            if cursor.is_none() {
                break;
            }
        }
        if cursor.is_some() {
            return Err(AppError::Rpc(
                "confirmed restoration exceeded page bound".into(),
            ));
        }
        let binding = binding.ok_or_else(|| AppError::Rpc("empty restoration".into()))?;

        let mut cursor: Option<String> = None;
        let mut mempool_entries = Vec::new();
        let mut mempool_binding: Option<MempoolBinding> = None;
        for _ in 0..MAX_CURSOR_PAGES {
            let page: WireMempoolPage = self
                .rpc(json!({
                    "method": "mempool_scripts_page",
                    "params": {
                        "script_ids": &script_ids,
                        "expected_chain_epoch": binding.chain_epoch,
                        "cursor": cursor.clone(),
                        "scan_limit": 1024
                    }
                }))
                .await?;
            require_binding(page.chain_epoch, page.tip.as_ref(), &binding)?;
            validate_hex_32(&page.instance_nonce)?;
            let page_mempool = MempoolBinding {
                instance_nonce: page.instance_nonce,
                generation: page.generation,
            };
            if let Some(expected) = &mempool_binding {
                if expected != &page_mempool {
                    return Err(AppError::Rpc("mempool changed during restoration".into()));
                }
            } else {
                mempool_binding = Some(page_mempool);
            }
            mempool_entries.extend(page.entries);
            cursor = page.continuation;
            if cursor.is_none() {
                break;
            }
        }
        if cursor.is_some() {
            return Err(AppError::Rpc(
                "mempool restoration exceeded page bound".into(),
            ));
        }
        Ok(RestoredAddress {
            chain_epoch: binding.chain_epoch,
            tip: binding.tip,
            mempool: mempool_binding
                .ok_or_else(|| AppError::Rpc("empty mempool restoration".into()))?,
            confirmed_history,
            confirmed_utxos,
            mempool_entries,
        })
    }

    async fn transaction_evidence(
        &self,
        txid: &str,
        binding: &SnapshotBinding,
        mempool: Option<&MempoolBinding>,
    ) -> Result<WireTransactionEvidence, AppError> {
        validate_hex_32(txid)?;
        let expected_mempool = mempool.map(|binding| {
            json!({
                "instance_nonce": binding.instance_nonce,
                "generation": binding.generation
            })
        });
        let evidence: WireTransactionEvidence = self
            .rpc(json!({
                "method": "transaction_evidence",
                "params": {
                    "txid": txid,
                    "expected_chain_epoch": binding.chain_epoch,
                    "expected_mempool": expected_mempool
                }
            }))
            .await?;
        require_binding(evidence.chain_epoch, evidence.tip.as_ref(), binding)?;
        if let Some(expected) = mempool {
            if evidence.mempool_instance_nonce != expected.instance_nonce
                || evidence.mempool_generation != expected.generation
            {
                return Err(AppError::Rpc(
                    "transaction mempool evidence is stale".into(),
                ));
            }
        }
        if let Some(raw) = &evidence.transaction_hex {
            let transaction = decode_transaction(raw)?;
            let actual = transaction
                .transaction_hash()
                .map_err(|e| AppError::Rpc(e.to_string()))?
                .to_string();
            if actual != txid {
                return Err(AppError::Rpc("transaction evidence hash mismatch".into()));
            }
        }
        let shape_is_valid = match evidence.status.as_str() {
            "unknown" => {
                evidence.payload == "absent"
                    && evidence.transaction_hex.is_none()
                    && evidence.inclusion.is_none()
            }
            "mempool" => {
                evidence.payload == "retained"
                    && evidence.transaction_hex.is_some()
                    && evidence.inclusion.is_none()
            }
            "confirmed" => {
                matches!(evidence.payload.as_str(), "retained" | "pruned")
                    && (evidence.payload == "retained") == evidence.transaction_hex.is_some()
                    && evidence.inclusion.is_some()
            }
            _ => false,
        };
        if !shape_is_valid {
            return Err(AppError::Rpc(
                "sidecar returned inconsistent transaction evidence".into(),
            ));
        }
        Ok(evidence)
    }

    async fn name_evidence(
        &self,
        name_hash: NameHash,
        chain_epoch: u64,
    ) -> Result<WireNameEvidence, AppError> {
        let response: WireNameEvidence = self
            .rpc(json!({
                "method": "name_evidence",
                "params": {
                    "name_hash": name_hash.to_string(),
                    "expected_chain_epoch": chain_epoch
                }
            }))
            .await?;
        if response.chain_epoch != chain_epoch
            || response.proof.name_hash != name_hash.to_string()
            || response.data_semantics
                != "projected_data_hex_is_resource_bytes_not_encoded_name_state"
        {
            return Err(AppError::Rpc("invalid name-evidence binding".into()));
        }
        let tip = response
            .tip
            .as_ref()
            .ok_or_else(|| AppError::Rpc("name evidence has no chain tip".into()))?;
        if response.proof.root != tip.tree_root {
            return Err(AppError::Rpc(
                "name proof root does not match chain tip".into(),
            ));
        }
        let raw_proof = decode_hex(&response.proof.proof_hex, 82_469, "name proof")?;
        let kind = match response.proof.kind.as_str() {
            "inclusion" => ProofKind::Inclusion,
            "non_inclusion" => ProofKind::NonInclusion,
            _ => return Err(AppError::Rpc("unknown name-proof kind".into())),
        };
        let proof = UrkelProof {
            name_hash,
            kind,
            raw: raw_proof,
        };
        let root = TreeRoot::from_hex(&response.proof.root)
            .map_err(|_| AppError::Rpc("invalid name-proof root".into()))?;
        let verified = proof
            .verify_strict(root)
            .map_err(|e| AppError::Rpc(format!("name proof verification failed: {e}")))?;
        let projected = response
            .proof_state_hex
            .as_deref()
            .map(|value| decode_hex(value, 1_024, "proof name state"))
            .transpose()?;
        if verified != projected {
            return Err(AppError::Rpc(
                "name proof value does not match projected state".into(),
            ));
        }
        validate_name_state_projection(
            name_hash,
            response.current_state_hex.as_deref(),
            response.current_state.as_ref(),
            "current",
        )?;
        validate_name_state_projection(
            name_hash,
            response.proof_state_hex.as_deref(),
            response.proof_state.as_ref(),
            "proof",
        )?;
        Ok(response)
    }
}

fn validate_name_state_projection(
    name_hash: NameHash,
    encoded_hex: Option<&str>,
    projected: Option<&WireNameState>,
    label: &str,
) -> Result<(), AppError> {
    match (encoded_hex, projected) {
        (None, None) => Ok(()),
        (Some(encoded_hex), Some(projected)) => {
            let encoded = decode_hex(encoded_hex, 1_024, &format!("{label} name state"))?;
            let state = decode_name_state(name_hash, &encoded).map_err(|e| {
                AppError::Rpc(format!("invalid {label} authenticated name state: {e}"))
            })?;
            let matches = projected.name_hash == state.name_hash.to_string()
                && projected.name_hex == hex::encode(&state.name)
                && projected.height == state.height.get()
                && projected.renewal == state.renewal.get()
                && projected.owner.txid == state.owner.transaction_hash.to_string()
                && projected.owner.index == state.owner.index
                && projected.value == state.value.get()
                && projected.highest == state.highest.get()
                && projected.data_hex == hex::encode(&state.resource_data)
                && projected.transfer == state.transfer.get()
                && projected.revoked == state.revoked.get()
                && projected.claimed == state.claimed.get()
                && projected.renewals == state.renewals
                && projected.registered == state.registered
                && projected.expired == state.expired
                && projected.weak == state.weak;
            if matches {
                Ok(())
            } else {
                Err(AppError::Rpc(format!(
                    "{label} name-state projection does not match its canonical bytes"
                )))
            }
        }
        _ => Err(AppError::Rpc(format!(
            "{label} name-state bytes and projection must be present together"
        ))),
    }
}

fn normalize_base_url(endpoint: &str) -> String {
    endpoint
        .trim()
        .trim_end_matches('/')
        .strip_suffix(WALLET_PATH)
        .unwrap_or(endpoint.trim().trim_end_matches('/'))
        .to_string()
}

fn wallet_url(base_url: &str) -> String {
    format!("{base_url}{WALLET_PATH}")
}

fn script_id(version: u8, hash: &[u8]) -> Result<[u8; 32], AppError> {
    let length = u8::try_from(hash.len())
        .map_err(|_| AppError::InvalidInput("address program is too long".into()))?;
    let mut canonical = Vec::with_capacity(hash.len() + 2);
    canonical.push(version);
    canonical.push(length);
    canonical.extend_from_slice(hash);
    let mut hasher =
        Blake2bVar::new(32).map_err(|_| AppError::Crypto("invalid BLAKE2b output size".into()))?;
    hasher.update(&canonical);
    let mut output = [0_u8; 32];
    hasher
        .finalize_variable(&mut output)
        .map_err(|_| AppError::Crypto("BLAKE2b finalization failed".into()))?;
    Ok(output)
}

fn require_binding(
    chain_epoch: u64,
    tip: Option<&WireTip>,
    expected: &SnapshotBinding,
) -> Result<(), AppError> {
    if chain_epoch != expected.chain_epoch || tip != Some(&expected.tip) {
        return Err(AppError::Rpc(
            "sidecar snapshot changed during request".into(),
        ));
    }
    Ok(())
}

fn validate_hex_32(value: &str) -> Result<(), AppError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AppError::InvalidInput(
            "expected a 32-byte hexadecimal identity".into(),
        ));
    }
    Ok(())
}

fn decode_hex(value: &str, maximum: usize, label: &str) -> Result<Vec<u8>, AppError> {
    if !value.len().is_multiple_of(2) || value.len() > maximum.saturating_mul(2) {
        return Err(AppError::Rpc(format!("{label} exceeds its wire bound")));
    }
    hex::decode(value).map_err(|_| AppError::Rpc(format!("{label} is not hexadecimal")))
}

fn decode_utf8_hex(value: &str, label: &str) -> Result<String, AppError> {
    String::from_utf8(decode_hex(value, 255, label)?)
        .map_err(|_| AppError::Rpc(format!("{label} is not UTF-8")))
}

fn decode_transaction(value: &str) -> Result<Transaction, AppError> {
    let raw = decode_hex(value, 4_000_000, "transaction")?;
    let transaction = Transaction::decode(&raw)
        .map_err(|e| AppError::Rpc(format!("invalid canonical transaction: {e}")))?;
    let encoded = transaction
        .encode()
        .map_err(|e| AppError::Rpc(format!("could not re-encode transaction: {e}")))?;
    if encoded != raw {
        return Err(AppError::Rpc("transaction encoding is noncanonical".into()));
    }
    Ok(transaction)
}

fn node_coin(coin: WireCoin, address_text: &str, tip_height: u32) -> Result<NodeCoin, AppError> {
    let value = i64::try_from(coin.value)
        .map_err(|_| AppError::Rpc("coin value is outside SQLite range".into()))?;
    Ok(NodeCoin {
        txid: coin.outpoint.txid,
        vout: coin.outpoint.index,
        value,
        script: None,
        address: Some(address_text.to_string()),
        height: Some(i64::from(coin.height)),
        confirmations: Some(i64::from(
            tip_height.saturating_sub(coin.height).saturating_add(1),
        )),
        coinbase: Some(coin.coinbase),
        covenant: Some(NodeCovenant {
            kind: coin.covenant.kind,
            action: Some(covenant_action(coin.covenant.kind).to_string()),
            items: coin.covenant.items,
        }),
    })
}

fn covenant_action(kind: u8) -> &'static str {
    match kind {
        0 => "NONE",
        1 => "CLAIM",
        2 => "OPEN",
        3 => "BID",
        4 => "REVEAL",
        5 => "REDEEM",
        6 => "REGISTER",
        7 => "UPDATE",
        8 => "RENEW",
        9 => "TRANSFER",
        10 => "FINALIZE",
        11 => "REVOKE",
        _ => "UNKNOWN",
    }
}

fn output_json(output: &hns_transaction::Output) -> Value {
    json!({
        "value": output.value.get(),
        "address": {
            "version": output.address.version,
            "hash": hex::encode(&output.address.hash)
        },
        "covenant": {
            "type": output.covenant.kind.as_u8(),
            "action": covenant_action(output.covenant.kind.as_u8()),
            "items": output.covenant.items.iter().map(hex::encode).collect::<Vec<_>>()
        }
    })
}

fn transaction_json(
    transaction: &Transaction,
    txid: &str,
    height: i64,
    time: Option<u64>,
    confirmations: u32,
) -> Value {
    json!({
        "hash": txid,
        "height": height,
        "time": time,
        "confirmations": confirmations,
        "inputs": transaction.inputs.iter().map(|input| json!({
            "prevout": {
                "hash": input.previous_output.transaction_hash.to_string(),
                "index": input.previous_output.index
            }
        })).collect::<Vec<_>>(),
        "outputs": transaction.outputs.iter().map(output_json).collect::<Vec<_>>()
    })
}

fn resource_name(name: &hns_covenants::ResourceName) -> String {
    if name.is_root() {
        return ".".to_string();
    }
    let labels = name
        .labels()
        .iter()
        .map(|label| String::from_utf8_lossy(label))
        .collect::<Vec<_>>();
    format!("{}.", labels.join("."))
}

fn resource_record_json(record: &ResourceRecord) -> Value {
    match record {
        ResourceRecord::Ds {
            key_tag,
            algorithm,
            digest_type,
            digest,
        } => json!({
            "type": "DS", "keyTag": key_tag, "algorithm": algorithm,
            "digestType": digest_type, "digest": hex::encode(digest)
        }),
        ResourceRecord::Ns { name_server } => {
            json!({ "type": "NS", "ns": resource_name(name_server) })
        }
        ResourceRecord::Glue4 {
            name_server,
            address,
        } => json!({
            "type": "GLUE4", "ns": resource_name(name_server),
            "address": std::net::Ipv4Addr::from(*address).to_string()
        }),
        ResourceRecord::Glue6 {
            name_server,
            address,
        } => json!({
            "type": "GLUE6", "ns": resource_name(name_server),
            "address": std::net::Ipv6Addr::from(*address).to_string()
        }),
        ResourceRecord::Synth4 { address } => json!({
            "type": "SYNTH4", "address": std::net::Ipv4Addr::from(*address).to_string()
        }),
        ResourceRecord::Synth6 { address } => json!({
            "type": "SYNTH6", "address": std::net::Ipv6Addr::from(*address).to_string()
        }),
        ResourceRecord::Txt { strings } => json!({
            "type": "TXT",
            "txt": strings.iter().map(|value| String::from_utf8_lossy(value)).collect::<Vec<_>>()
        }),
    }
}

fn auction_phase(state: &WireNameState, tip: u32, network: Network) -> &'static str {
    if state.revoked != 0 {
        return "REVOKED";
    }
    if state.registered || state.expired {
        return "CLOSED";
    }
    let params = network.name_params();
    let open_end = state.height.saturating_add(params.tree_interval + 1);
    let bid_end = open_end.saturating_add(params.bidding_period);
    let reveal_end = bid_end.saturating_add(params.reveal_period);
    if tip < open_end {
        "OPENING"
    } else if tip < bid_end {
        "BIDDING"
    } else if tip < reveal_end {
        "REVEAL"
    } else {
        "CLOSED"
    }
}

fn auction_stats(state: &WireNameState, tip: u32, network: Network) -> Value {
    let params = network.name_params();
    let open_end = state.height.saturating_add(params.tree_interval + 1);
    let bid_end = open_end.saturating_add(params.bidding_period);
    let reveal_end = bid_end.saturating_add(params.reveal_period);
    json!({
        "openPeriodStart": state.height,
        "openPeriodEnd": open_end,
        "bidPeriodStart": open_end,
        "bidPeriodEnd": bid_end,
        "revealPeriodStart": bid_end,
        "revealPeriodEnd": reveal_end,
        "blocksUntilBidding": i64::from(open_end) - i64::from(tip),
        "blocksUntilReveal": i64::from(bid_end) - i64::from(tip),
        "blocksUntilClose": i64::from(reveal_end) - i64::from(tip)
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlockchainInfo {
    pub blocks: i64,
    pub headers: Option<i64>,
    pub verification_progress: Option<f64>,
    pub bestblockhash: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NodeCoin {
    pub txid: String,
    pub vout: u32,
    pub value: i64,
    pub script: Option<String>,
    pub address: Option<String>,
    pub height: Option<i64>,
    pub confirmations: Option<i64>,
    pub coinbase: Option<bool>,
    pub covenant: Option<NodeCovenant>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NodeCovenant {
    pub kind: u8,
    pub action: Option<String>,
    pub items: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct WireTip {
    hash: String,
    height: u32,
    tree_root: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SnapshotBinding {
    chain_epoch: u64,
    tip: WireTip,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MempoolBinding {
    instance_nonce: String,
    generation: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireOutpoint {
    txid: String,
    index: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireAddress {
    version: u8,
    hash: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireCovenant {
    kind: u8,
    items: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireCoin {
    outpoint: WireOutpoint,
    value: u64,
    height: u32,
    coinbase: bool,
    address: WireAddress,
    covenant: WireCovenant,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireConfirmedHistory {
    script_index: usize,
    txid: String,
    block_hash: String,
    height: u32,
    transaction_position: u32,
    block_time: Option<u64>,
    received: bool,
    spent: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireConfirmedUtxo {
    script_index: usize,
    coin: WireCoin,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireConfirmedPage {
    chain_epoch: u64,
    tip: Option<WireTip>,
    history: Vec<WireConfirmedHistory>,
    utxos: Vec<WireConfirmedUtxo>,
    script_examinations: usize,
    continuation: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireMempoolOutput {
    script_index: usize,
    outpoint: WireOutpoint,
    value: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireMempoolSpend {
    script_index: usize,
    outpoint: WireOutpoint,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireMempoolActivity {
    txid: String,
    admitted_at: u64,
    received: Vec<WireMempoolOutput>,
    spent: Vec<WireMempoolSpend>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireMempoolPage {
    chain_epoch: u64,
    tip: Option<WireTip>,
    instance_nonce: String,
    generation: u64,
    entries: Vec<WireMempoolActivity>,
    continuation: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireInclusion {
    block_hash: String,
    height: u32,
    transaction_index: Option<u32>,
    confirmations: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireTransactionEvidence {
    chain_epoch: u64,
    mempool_instance_nonce: String,
    mempool_generation: u64,
    tip: Option<WireTip>,
    status: String,
    inclusion: Option<WireInclusion>,
    payload: String,
    transaction_hex: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBlockHashEvidence {
    chain_epoch: u64,
    tip: Option<WireTip>,
    height: u32,
    hash: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSpendingTransaction {
    txid: String,
    input_position: u32,
    block_hash: String,
    height: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSpendingEvidence {
    chain_epoch: u64,
    tip: Option<WireTip>,
    entries: Vec<WireSpendingEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSpendingEntry {
    outpoint: WireOutpoint,
    spending: Option<WireSpendingTransaction>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBroadcast {
    txid: String,
    newly_admitted: bool,
    attempted_peers: usize,
    queued_peers: usize,
    failed_peers: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireFeeEstimate {
    target_blocks: u32,
    atomic_units_per_kvb: u64,
    sampled_transactions: usize,
    source: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireTrackedContractKnown {
    contract_id: String,
    known: bool,
    descriptor: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireTrackedContractPage {
    chain_epoch: u64,
    tip: Option<WireTip>,
    entries: Vec<Value>,
    continuation: Option<String>,
    #[serde(default)]
    instance_nonce: Option<String>,
    #[serde(default)]
    generation: Option<u64>,
    #[serde(default)]
    preimage_transport: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireNameState {
    name_hash: String,
    name_hex: String,
    height: u32,
    renewal: u32,
    owner: WireOutpoint,
    value: u64,
    highest: u64,
    data_hex: String,
    transfer: u32,
    revoked: u32,
    claimed: u32,
    renewals: u32,
    registered: bool,
    expired: bool,
    weak: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireNameProof {
    root: String,
    name_hash: String,
    kind: String,
    proof_hex: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireNameOwner {
    name_state: WireNameState,
    owner: WireOutpoint,
    transaction_hex: String,
    owner_output: Value,
    inclusion: WireInclusion,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireNameEvidence {
    chain_epoch: u64,
    tip: Option<WireTip>,
    current_state_hex: Option<String>,
    proof_state_hex: Option<String>,
    current_state: Option<WireNameState>,
    proof_state: Option<WireNameState>,
    proof: WireNameProof,
    current_owner: Option<WireNameOwner>,
    proof_owner: Option<WireNameOwner>,
    data_semantics: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransactionFeeQuote {
    pub txid: String,
    pub chain_epoch: u64,
    tip: Option<WireTip>,
    pub mempool_instance_nonce: String,
    pub mempool_generation: u64,
    pub target_blocks: u32,
    pub rate_atomic_units_per_1000_policy_vbytes: u64,
    pub rate_sample_count: usize,
    pub rate_source: String,
    pub transaction_weight: usize,
    pub transaction_sigops: u32,
    pub sigop_adjusted_policy_vbytes: usize,
    pub minimum_policy_fee_atomic_units: u64,
    pub actual_fee_atomic_units: u64,
    pub meets_minimum_policy_fee: bool,
    pub minimum_policy_fee_shortfall_atomic_units: u64,
}

struct RestoredAddress {
    chain_epoch: u64,
    tip: WireTip,
    mempool: MempoolBinding,
    confirmed_history: Vec<WireConfirmedHistory>,
    confirmed_utxos: Vec<WireConfirmedUtxo>,
    mempool_entries: Vec<WireMempoolActivity>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wallet_url_is_versioned_once() {
        assert_eq!(
            wallet_url(&normalize_base_url("http://127.0.0.1:12037")),
            "http://127.0.0.1:12037/api/v1/wallet"
        );
        assert_eq!(
            wallet_url(&normalize_base_url("http://127.0.0.1:12037/api/v1/wallet")),
            "http://127.0.0.1:12037/api/v1/wallet"
        );
    }

    #[test]
    fn transport_guard_requires_tls_off_loopback() {
        assert!(guard_transport("http://127.0.0.1:12037", "Bearer test").is_ok());
        assert!(guard_transport("http://[::1]:12037", "Bearer test").is_ok());
        assert!(guard_transport("https://node.example", "Bearer test").is_ok());
        assert!(guard_transport("http://node.example", "Bearer test").is_err());
    }

    #[test]
    fn script_identity_hashes_canonical_address_bytes() {
        let hash = [0x11; 20];
        let id = script_id(0, &hash).expect("script id");
        assert_eq!(id.len(), 32);
        assert_ne!(id, [0; 32]);
    }
}
