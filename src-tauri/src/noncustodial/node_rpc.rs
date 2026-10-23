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

/// Why a command refuses a node without the address index. hsd answers
/// `GET /coin/address` and `GET /tx/address` with an empty list, never an
/// error, when its indexes are off, so a wallet read through such a node
/// would look empty. Activity's banner ("not enabled on this hsd node", the
/// flag named from "address index") and the error mapper ("index-address")
/// key on this sentence. The history, which also needs the tx index, refuses
/// with [`NEEDS_TX_INDEX`] or [`NEEDS_ADDRESS_AND_TX_INDEX`] when that is
/// what the node lacks.
pub const NEEDS_ADDRESS_INDEX: &str =
    "address index not enabled on this hsd node: restart hsd with --index-address and --index-tx";

/// Why the history refuses a node with the address index but without the tx
/// index: hsd's `GET /tx/address` needs both (`ChainDB.getMetaByAddress`
/// returns nothing unless `indexTX` and `indexAddress` are on), and answers
/// with the mempool alone otherwise. Activity's banner keys on "not enabled
/// on this hsd node" and names the flag from the sentence.
pub const NEEDS_TX_INDEX: &str =
    "transaction index not enabled on this hsd node: restart hsd with --index-address and --index-tx";

/// Why the history refuses a node with neither index.
pub const NEEDS_ADDRESS_AND_TX_INDEX: &str =
    "address and transaction indexes not enabled on this hsd node: restart hsd with --index-address and --index-tx";

/// Why a command refuses an hsd older than 7.0: `chain.indexers` came to its
/// `GET /` reply in v7.0.0, so an older node cannot say whether it has the
/// address index, and its empty coin lists cannot be told from an index that
/// is off. The remedy is the update, not `--index-address` (such a node may
/// well have it); the error mapper leaves this sentence as it is.
pub const NODE_INDEXES_UNREPORTED: &str =
    "this hsd node does not report its indexes (hsd older than 7.0): update hsd to 7.0 or newer";

/// The indexes hsd was started with (`--index-tx`, `--index-address`), as its
/// REST `GET /` reports them. Read once per command or sync run: never
/// cached, as the user may restart the node with other flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NodeIndexes {
    pub tx: bool,
    pub address: bool,
}

/// Read the node's indexes from `chain.indexers.indexTX` and
/// `chain.indexers.indexAddress` of its `GET /` reply. A reply without both
/// as booleans is an [`AppError::Rpc`]: never taken as "on". A `chain` with
/// no `indexers` at all is an hsd older than 7.0, and its error is
/// [`NODE_INDEXES_UNREPORTED`].
pub(crate) async fn node_indexes(node: &dyn NodeRpc) -> Result<NodeIndexes, AppError> {
    let status = node.get_node_status().await?;
    if status
        .get("chain")
        .is_some_and(serde_json::Value::is_object)
        && status.pointer("/chain/indexers").is_none()
    {
        return Err(AppError::Rpc(NODE_INDEXES_UNREPORTED.into()));
    }
    let flag = |name: &str| {
        status
            .pointer(&format!("/chain/indexers/{name}"))
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| {
                AppError::Rpc(format!(
                    "node status did not report chain.indexers.{name}: could not check its indexes"
                ))
            })
    };
    Ok(NodeIndexes {
        tx: flag("indexTX")?,
        address: flag("indexAddress")?,
    })
}

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

    /// hsd's REST `GET /` (its indexes among others).
    async fn get_node_status(&self) -> Result<serde_json::Value, AppError>;

    async fn get_name_info(&self, name: &str) -> Result<serde_json::Value, AppError>;

    async fn get_name_by_hash(&self, name_hash_hex: &str) -> Result<Option<String>, AppError>;

    async fn get_name_resource(&self, name: &str) -> Result<serde_json::Value, AppError>;

    async fn get_coins_by_address(&self, address: &str) -> Result<Vec<NodeCoin>, AppError>;

    async fn get_coin(&self, txid: &str, index: u32) -> Result<Option<NodeCoin>, AppError>;

    async fn get_tx_out(
        &self,
        txid: &str,
        index: u32,
    ) -> Result<Option<serde_json::Value>, AppError>;

    async fn get_txs_by_address(&self, address: &str) -> Result<Vec<serde_json::Value>, AppError>;

    async fn get_raw_transaction(&self, txid: &str) -> Result<serde_json::Value, AppError>;

    async fn get_tx_by_hash(&self, txid: &str) -> Result<serde_json::Value, AppError>;

    /// `getmempoolentry`: the node's own mempool entry for `txid`, or hsd's
    /// `Transaction not in mempool.` refusal
    /// ([`crate::noncustodial::rpc::is_not_in_mempool`]).
    async fn get_mempool_entry(&self, txid: &str) -> Result<serde_json::Value, AppError>;

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
///
/// COVERAGE: ~69% — the `impl NodeRpc for NodeRpcClient` block below is a
/// zero-logic forwarding layer: each method is a single `self.method().await`
/// call that hits a live hsd node over HTTP. Covering these lines requires a
/// running hsd instance (or mockito pointed at the real `NodeRpcClient`
/// constructor, which would test the HTTP client, not the forwarding). The
/// trait's VALUE is that consumers use `&dyn NodeRpc` and tests inject
/// `MockNodeRpc` — the forwarding impl is exercised only in production. The
/// uncovered lines are structurally out of scope for unit tests.
#[cfg_attr(coverage_nightly, coverage(off))]
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

    async fn get_node_status(&self) -> Result<serde_json::Value, AppError> {
        NodeRpcClient::get_node_status(self).await
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

    async fn get_coin(&self, txid: &str, index: u32) -> Result<Option<NodeCoin>, AppError> {
        NodeRpcClient::get_coin(self, txid, index).await
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

    async fn get_mempool_entry(&self, txid: &str) -> Result<serde_json::Value, AppError> {
        NodeRpcClient::get_mempool_entry(self, txid).await
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
