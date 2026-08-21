//! `NodeRpc` — the trait-shaped seam over hsd node RPC that lets commands
//! accept an injected client instead of always constructing a concrete
//! `NodeRpcClient` from settings.
//!
//! This is the P3 refactor's core artifact. Production code keeps building the
//! real `NodeRpcClient` from settings and passes `&client as &dyn NodeRpc`
//! into command helpers; tests build a `MockNodeRpc` (in
//! `crate::tests::mock_node_rpc`) that returns pre-canned JSON responses,
//! finally unlocking coverage of the RPC-dependent branches inside command
//! functions (fetch-parse-persist paths, error/soft-degrade branches,
//! phase-derived task states, etc.).
//!
//! Only methods actually called by consumer code (commands, daemon) are on
//! the trait — the internal `call<T>` helper stays private to
//! `NodeRpcClient` because it isn't part of the mockable surface.
//!
//! ## Why `async_trait`
//!
//! Rust 1.75+ supports async fns in traits natively, but the resulting
//! trait is NOT dyn-compatible — you'd have to thread a concrete generic
//! `<R: NodeRpc>` parameter through every helper. That churns the signatures
//! of ~40 functions and every downstream caller. `#[async_trait]` desugars
//! to `-> Pin<Box<dyn Future>>`, which is dyn-compatible, so we can pass
//! `&dyn NodeRpc` and store `Arc<dyn NodeRpc>` uniformly. The per-call heap
//! alloc is negligible next to a network round-trip.

use async_trait::async_trait;

use crate::error::AppError;
use crate::noncustodial::rpc::{BlockchainInfo, ChainSource, NodeCoin, NodeRpcClient};

/// Node RPC surface used by commands and the daemon.
///
/// All methods forward to the corresponding hsd RPC/REST endpoint. The
/// canonical error mapping and null-vs-error semantics are documented on
/// each implementor method in `NodeRpcClient`.
#[async_trait]
pub trait NodeRpc: Send + Sync {
    /// The chain source this client is configured for — Full (writes allowed),
    /// Explorer (read-only), etc. Consulted by `send_raw_transaction` to reject
    /// broadcast attempts against a read-only source.
    fn source(&self) -> ChainSource;

    // --- Chain reads -------------------------------------------------------

    async fn get_blockchain_info(&self) -> Result<BlockchainInfo, AppError>;

    async fn get_info(&self) -> Result<serde_json::Value, AppError>;

    async fn get_name_info(&self, name: &str) -> Result<serde_json::Value, AppError>;

    async fn get_name_by_hash(&self, name_hash_hex: &str) -> Result<Option<String>, AppError>;

    async fn get_name_resource(&self, name: &str) -> Result<serde_json::Value, AppError>;

    async fn get_coins_by_address(&self, address: &str) -> Result<Vec<NodeCoin>, AppError>;

    async fn get_tx_out(
        &self,
        txid: &str,
        index: u32,
    ) -> Result<Option<serde_json::Value>, AppError>;

    async fn get_txs_by_address(&self, address: &str) -> Result<Vec<serde_json::Value>, AppError>;

    async fn get_raw_transaction(&self, txid: &str) -> Result<serde_json::Value, AppError>;

    async fn get_tx_by_hash(&self, txid: &str) -> Result<serde_json::Value, AppError>;

    async fn get_block_hash(&self, height: i64) -> Result<String, AppError>;

    async fn get_block(&self, hash: &str) -> Result<serde_json::Value, AppError>;

    async fn generate_to_address(
        &self,
        nblocks: u32,
        address: &str,
    ) -> Result<serde_json::Value, AppError>;

    async fn stop(&self) -> Result<(), AppError>;

    // --- Broadcast (write) -------------------------------------------------

    async fn send_raw_transaction(&self, raw_tx_hex: &str) -> Result<String, AppError>;

    async fn estimate_smart_fee(&self, blocks: u32) -> Result<u64, AppError>;
}

/// Trivial forwarding impl — every method delegates to the inherent method on
/// `NodeRpcClient` so production behavior is byte-identical. The trait exists
/// purely to allow test doubles (`crate::tests::mock_node_rpc::MockNodeRpc`).
#[async_trait]
impl NodeRpc for NodeRpcClient {
    fn source(&self) -> ChainSource {
        NodeRpcClient::source(self)
    }

    async fn get_blockchain_info(&self) -> Result<BlockchainInfo, AppError> {
        NodeRpcClient::get_blockchain_info(self).await
    }

    async fn get_info(&self) -> Result<serde_json::Value, AppError> {
        NodeRpcClient::get_info(self).await
    }

    async fn get_name_info(&self, name: &str) -> Result<serde_json::Value, AppError> {
        NodeRpcClient::get_name_info(self, name).await
    }

    async fn get_name_by_hash(&self, name_hash_hex: &str) -> Result<Option<String>, AppError> {
        NodeRpcClient::get_name_by_hash(self, name_hash_hex).await
    }

    async fn get_name_resource(&self, name: &str) -> Result<serde_json::Value, AppError> {
        NodeRpcClient::get_name_resource(self, name).await
    }

    async fn get_coins_by_address(&self, address: &str) -> Result<Vec<NodeCoin>, AppError> {
        NodeRpcClient::get_coins_by_address(self, address).await
    }

    async fn get_tx_out(
        &self,
        txid: &str,
        index: u32,
    ) -> Result<Option<serde_json::Value>, AppError> {
        NodeRpcClient::get_tx_out(self, txid, index).await
    }

    async fn get_txs_by_address(&self, address: &str) -> Result<Vec<serde_json::Value>, AppError> {
        NodeRpcClient::get_txs_by_address(self, address).await
    }

    async fn get_raw_transaction(&self, txid: &str) -> Result<serde_json::Value, AppError> {
        NodeRpcClient::get_raw_transaction(self, txid).await
    }

    async fn get_tx_by_hash(&self, txid: &str) -> Result<serde_json::Value, AppError> {
        NodeRpcClient::get_tx_by_hash(self, txid).await
    }

    async fn get_block_hash(&self, height: i64) -> Result<String, AppError> {
        NodeRpcClient::get_block_hash(self, height).await
    }

    async fn get_block(&self, hash: &str) -> Result<serde_json::Value, AppError> {
        NodeRpcClient::get_block(self, hash).await
    }

    async fn generate_to_address(
        &self,
        nblocks: u32,
        address: &str,
    ) -> Result<serde_json::Value, AppError> {
        NodeRpcClient::generate_to_address(self, nblocks, address).await
    }

    async fn stop(&self) -> Result<(), AppError> {
        NodeRpcClient::stop(self).await
    }

    async fn send_raw_transaction(&self, raw_tx_hex: &str) -> Result<String, AppError> {
        NodeRpcClient::send_raw_transaction(self, raw_tx_hex).await
    }

    async fn estimate_smart_fee(&self, blocks: u32) -> Result<u64, AppError> {
        NodeRpcClient::estimate_smart_fee(self, blocks).await
    }
}
