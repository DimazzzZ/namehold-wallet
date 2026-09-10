//! `MockNodeRpc` — a test double for the `NodeRpc` trait that returns
//! pre-canned responses. Used to test RPC-consuming command logic without
//! needing a live hsd node.
//!
//! Each field is a boxed factory closure `Box<dyn Fn() -> Result<T, AppError>>`
//! so each call constructs a fresh result. We use closures because `AppError`
//! isn't `Clone` (its `#[from]` variants wrap non-Clone error types like
//! `rusqlite::Error` and `reqwest::Error`).
//!
//! Each test builds a mock, overrides the specific fields it needs, then
//! passes the mock to the command's `_with_client` inner function.
//! This unlocks coverage of error branches, soft-degrade paths, and
//! phase-dependent logic that were previously untestable without a node.

use async_trait::async_trait;
use std::sync::Mutex;

use crate::error::AppError;
use crate::noncustodial::node_rpc::NodeRpc;
use crate::noncustodial::rpc::{BlockchainInfo, ChainSource, NodeCoin};

/// Boxed factory closure. Constructed once at mock-build time, called once
/// per invocation. `Send + Sync` so the mock can be shared across tasks.
type ResponseFn<T> = Box<dyn Fn() -> Result<T, AppError> + Send + Sync>;

/// One recorded invocation of a [`MockNodeRpc`] method, capturing the method
/// name and its salient argument(s). Lets a test assert not just that the code
/// under test got the response it needed, but that it asked the node the RIGHT
/// question (correct name / address / txid / height). Previously every trait
/// method discarded its arguments (`_name`, `_address`, …), so a bug that
/// queried the wrong name but happened to receive a matching canned response
/// would pass silently.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RpcCall {
    BlockchainInfo,
    Info,
    NameInfo(String),
    NameByHash(String),
    NameResource(String),
    CoinsByAddress(String),
    TxOut(String, u32),
    TxsByAddress(String),
    RawTransaction(String),
    TxByHash(String),
    BlockHash(i64),
    Block(String),
    GenerateToAddress(u32, String),
    Stop,
    SendRawTransaction(String),
    EstimateSmartFee(u32),
}

/// A test double for `NodeRpc` that returns pre-canned responses.
///
/// Every method starts out returning `AppError::Rpc("not configured")`. Tests
/// override the ones they exercise by calling `.with_*(...)` methods before
/// passing the mock to the function under test.
pub struct MockNodeRpc {
    pub source: ChainSource,
    blockchain_info: ResponseFn<BlockchainInfo>,
    info: ResponseFn<serde_json::Value>,
    name_info: ResponseFn<serde_json::Value>,
    name_by_hash: ResponseFn<Option<String>>,
    name_resource: ResponseFn<serde_json::Value>,
    coins_by_address: ResponseFn<Vec<NodeCoin>>,
    tx_out: ResponseFn<Option<serde_json::Value>>,
    txs_by_address: ResponseFn<Vec<serde_json::Value>>,
    raw_transaction: ResponseFn<serde_json::Value>,
    tx_by_hash: ResponseFn<serde_json::Value>,
    block_hash: ResponseFn<String>,
    block: ResponseFn<serde_json::Value>,
    generate_to_address: ResponseFn<serde_json::Value>,
    stop: ResponseFn<()>,
    send_raw_transaction: ResponseFn<String>,
    estimate_smart_fee: ResponseFn<u64>,
    /// Ordered log of every trait-method call this mock has seen, with the
    /// caller's arguments captured. Interior mutability so the log can be
    /// updated through `&self` (the trait methods all take `&self`).
    calls: Mutex<Vec<RpcCall>>,
}

fn err<T>(msg: &'static str) -> ResponseFn<T> {
    Box::new(move || Err(AppError::Rpc(msg.to_string())))
}

impl MockNodeRpc {
    /// Create a new mock with all methods returning `AppError::Rpc("not configured")`.
    pub fn new() -> Self {
        Self {
            source: ChainSource::LocalNode,
            blockchain_info: err("not configured"),
            info: err("not configured"),
            name_info: err("not configured"),
            name_by_hash: err("not configured"),
            name_resource: err("not configured"),
            coins_by_address: err("not configured"),
            tx_out: err("not configured"),
            txs_by_address: err("not configured"),
            raw_transaction: err("not configured"),
            tx_by_hash: err("not configured"),
            block_hash: err("not configured"),
            block: err("not configured"),
            generate_to_address: err("not configured"),
            stop: Box::new(|| Ok(())),
            send_raw_transaction: err("not configured"),
            estimate_smart_fee: err("not configured"),
            calls: Mutex::new(Vec::new()),
        }
    }

    // ----- builder helpers for the common overrides ----------------------

    pub fn with_blockchain_info(mut self, info: BlockchainInfo) -> Self {
        self.blockchain_info = Box::new(move || Ok(info.clone()));
        self
    }
    pub fn with_blockchain_info_err(mut self, msg: &'static str) -> Self {
        self.blockchain_info = Box::new(move || Err(AppError::Rpc(msg.to_string())));
        self
    }

    pub fn with_name_info(mut self, v: serde_json::Value) -> Self {
        self.name_info = Box::new(move || Ok(v.clone()));
        self
    }
    pub fn with_name_info_err(mut self, msg: &'static str) -> Self {
        self.name_info = Box::new(move || Err(AppError::Rpc(msg.to_string())));
        self
    }

    pub fn with_name_resource(mut self, v: serde_json::Value) -> Self {
        self.name_resource = Box::new(move || Ok(v.clone()));
        self
    }
    pub fn with_name_resource_err(mut self, msg: &'static str) -> Self {
        self.name_resource = Box::new(move || Err(AppError::Rpc(msg.to_string())));
        self
    }

    pub fn with_tx_by_hash(mut self, v: serde_json::Value) -> Self {
        self.tx_by_hash = Box::new(move || Ok(v.clone()));
        self
    }
    pub fn with_tx_by_hash_err(mut self, msg: &'static str) -> Self {
        self.tx_by_hash = Box::new(move || Err(AppError::Rpc(msg.to_string())));
        self
    }

    pub fn with_block_hash(mut self, hash: String) -> Self {
        self.block_hash = Box::new(move || Ok(hash.clone()));
        self
    }
    pub fn with_block_hash_err(mut self, msg: &'static str) -> Self {
        self.block_hash = Box::new(move || Err(AppError::Rpc(msg.to_string())));
        self
    }

    pub fn with_block(mut self, v: serde_json::Value) -> Self {
        self.block = Box::new(move || Ok(v.clone()));
        self
    }
    pub fn with_block_err(mut self, msg: &'static str) -> Self {
        self.block = Box::new(move || Err(AppError::Rpc(msg.to_string())));
        self
    }

    pub fn with_source(mut self, source: ChainSource) -> Self {
        self.source = source;
        self
    }

    pub fn with_name_by_hash(mut self, v: Option<String>) -> Self {
        self.name_by_hash = Box::new(move || Ok(v.clone()));
        self
    }

    pub fn with_name_by_hash_err(mut self, msg: &'static str) -> Self {
        self.name_by_hash = Box::new(move || Err(AppError::Rpc(msg.to_string())));
        self
    }

    pub fn with_coins_by_address(mut self, coins: Vec<NodeCoin>) -> Self {
        self.coins_by_address = Box::new(move || Ok(coins.clone()));
        self
    }

    pub fn with_coins_by_address_err(mut self, msg: &'static str) -> Self {
        self.coins_by_address = Box::new(move || Err(AppError::Rpc(msg.to_string())));
        self
    }

    pub fn with_tx_out(mut self, v: Option<serde_json::Value>) -> Self {
        self.tx_out = Box::new(move || Ok(v.clone()));
        self
    }

    pub fn with_tx_out_err(mut self, msg: &'static str) -> Self {
        self.tx_out = Box::new(move || Err(AppError::Rpc(msg.to_string())));
        self
    }

    pub fn with_txs_by_address(mut self, txs: Vec<serde_json::Value>) -> Self {
        self.txs_by_address = Box::new(move || Ok(txs.clone()));
        self
    }

    pub fn with_txs_by_address_err(mut self, msg: &'static str) -> Self {
        self.txs_by_address = Box::new(move || Err(AppError::Rpc(msg.to_string())));
        self
    }

    pub fn with_raw_transaction(mut self, v: serde_json::Value) -> Self {
        self.raw_transaction = Box::new(move || Ok(v.clone()));
        self
    }

    pub fn with_raw_transaction_err(mut self, msg: &'static str) -> Self {
        self.raw_transaction = Box::new(move || Err(AppError::Rpc(msg.to_string())));
        self
    }

    pub fn with_send_raw_transaction(mut self, txid: String) -> Self {
        self.send_raw_transaction = Box::new(move || Ok(txid.clone()));
        self
    }

    pub fn with_send_raw_transaction_rpc_err(mut self, msg: &'static str) -> Self {
        self.send_raw_transaction = Box::new(move || Err(AppError::Rpc(msg.to_string())));
        self
    }

    /// Simulate a transport-level error (non-Rpc variant) for broadcast.
    pub fn with_send_raw_transaction_transport_err(mut self, msg: &'static str) -> Self {
        // Use `Lock` as a stand-in for any non-Rpc error variant (reqwest::Error
        // can't be constructed in tests). The broadcast classifier only checks
        // `matches!(e, AppError::Rpc(_))` — anything else is transport.
        self.send_raw_transaction = Box::new(move || Err(AppError::Lock(msg.to_string())));
        self
    }

    // ----- call-log accessors --------------------------------------------
    //
    // Tests use these to assert the code under test invoked the right method
    // with the right arguments — not just that it consumed some response.

    /// Snapshot of every recorded call, in invocation order.
    pub fn calls(&self) -> Vec<RpcCall> {
        self.calls.lock().expect("mock call log").clone()
    }

    /// Number of calls recorded so far. Convenience for `assert_eq!` on totals.
    pub fn call_count(&self) -> usize {
        self.calls.lock().expect("mock call log").len()
    }

    /// Every recorded call that matches `pred`. Preserves invocation order.
    pub fn calls_matching<F>(&self, mut pred: F) -> Vec<RpcCall>
    where
        F: FnMut(&RpcCall) -> bool,
    {
        self.calls
            .lock()
            .expect("mock call log")
            .iter()
            .filter(|c| pred(c))
            .cloned()
            .collect()
    }

    /// Number of times `pred` matches a recorded call.
    pub fn count_matching<F>(&self, pred: F) -> usize
    where
        F: FnMut(&RpcCall) -> bool,
    {
        self.calls_matching(pred).len()
    }

    /// Record one call. Internal helper used by the trait impl below.
    fn record(&self, call: RpcCall) {
        self.calls.lock().expect("mock call log").push(call);
    }
}

impl Default for MockNodeRpc {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl NodeRpc for MockNodeRpc {
    fn source(&self) -> ChainSource {
        self.source
    }

    async fn get_blockchain_info(&self) -> Result<BlockchainInfo, AppError> {
        self.record(RpcCall::BlockchainInfo);
        (self.blockchain_info)()
    }

    async fn get_info(&self) -> Result<serde_json::Value, AppError> {
        self.record(RpcCall::Info);
        (self.info)()
    }

    async fn get_name_info(&self, name: &str) -> Result<serde_json::Value, AppError> {
        self.record(RpcCall::NameInfo(name.to_string()));
        (self.name_info)()
    }

    async fn get_name_by_hash(&self, name_hash_hex: &str) -> Result<Option<String>, AppError> {
        self.record(RpcCall::NameByHash(name_hash_hex.to_string()));
        (self.name_by_hash)()
    }

    async fn get_name_resource(&self, name: &str) -> Result<serde_json::Value, AppError> {
        self.record(RpcCall::NameResource(name.to_string()));
        (self.name_resource)()
    }

    async fn get_coins_by_address(&self, address: &str) -> Result<Vec<NodeCoin>, AppError> {
        self.record(RpcCall::CoinsByAddress(address.to_string()));
        (self.coins_by_address)()
    }

    async fn get_tx_out(
        &self,
        txid: &str,
        index: u32,
    ) -> Result<Option<serde_json::Value>, AppError> {
        self.record(RpcCall::TxOut(txid.to_string(), index));
        (self.tx_out)()
    }

    async fn get_txs_by_address(&self, address: &str) -> Result<Vec<serde_json::Value>, AppError> {
        self.record(RpcCall::TxsByAddress(address.to_string()));
        (self.txs_by_address)()
    }

    async fn get_raw_transaction(&self, txid: &str) -> Result<serde_json::Value, AppError> {
        self.record(RpcCall::RawTransaction(txid.to_string()));
        (self.raw_transaction)()
    }

    async fn get_tx_by_hash(&self, txid: &str) -> Result<serde_json::Value, AppError> {
        self.record(RpcCall::TxByHash(txid.to_string()));
        (self.tx_by_hash)()
    }

    async fn get_block_hash(&self, height: i64) -> Result<String, AppError> {
        self.record(RpcCall::BlockHash(height));
        (self.block_hash)()
    }

    async fn get_block(&self, hash: &str) -> Result<serde_json::Value, AppError> {
        self.record(RpcCall::Block(hash.to_string()));
        (self.block)()
    }

    async fn generate_to_address(
        &self,
        nblocks: u32,
        address: &str,
    ) -> Result<serde_json::Value, AppError> {
        self.record(RpcCall::GenerateToAddress(nblocks, address.to_string()));
        (self.generate_to_address)()
    }

    async fn stop(&self) -> Result<(), AppError> {
        self.record(RpcCall::Stop);
        (self.stop)()
    }

    async fn send_raw_transaction(&self, raw_tx_hex: &str) -> Result<String, AppError> {
        self.record(RpcCall::SendRawTransaction(raw_tx_hex.to_string()));
        (self.send_raw_transaction)()
    }

    async fn estimate_smart_fee(&self, blocks: u32) -> Result<u64, AppError> {
        self.record(RpcCall::EstimateSmartFee(blocks));
        (self.estimate_smart_fee)()
    }
}
