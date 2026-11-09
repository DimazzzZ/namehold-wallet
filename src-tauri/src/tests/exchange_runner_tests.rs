//! `runner::send_hns` (spec R13, CODING_STANDARDS §Rust layering): every HNS
//! send of the exchange passes `rpc::broadcast_gates`, and what reaches the
//! node is exactly the presigned hex it was given.

use std::collections::HashMap;

use super::exchange_fixture as fx;
use super::mock_node_rpc::{MockNodeRpc, RpcCall};
use crate::exchange_io::runner::{self, HnsIo};
use crate::noncustodial::rpc::{BlockchainInfo, ChainSource};

fn on_chain(chain: &str) -> BlockchainInfo {
    BlockchainInfo {
        blocks: 100,
        chain: Some(chain.to_owned()),
        ..Default::default()
    }
}

fn sends(node: &MockNodeRpc) -> usize {
    node.calls_matching(|c| matches!(c, RpcCall::SendRawTransaction { .. }))
        .len()
}

/// The Seller's signed HNS funding of the fixture trade, as the Commit sends it.
fn funding_hex() -> String {
    fx::trade().hns_funding.to_hex()
}

#[tokio::test]
async fn a_node_on_another_chain_gets_nothing() {
    let mut node = MockNodeRpc::new()
        .with_source(ChainSource::LocalNode)
        .with_blockchain_info(on_chain("main"))
        .with_send_raw_transaction("00".repeat(32));
    node.node_url = "http://127.0.0.1:14037".into();
    let io = HnsIo {
        node: &node,
        settings: &HashMap::new(),
        network: fx::NET,
    };
    let e = runner::send_hns(&io, &funding_hex())
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("not sent"), "{e}");
    assert_eq!(sends(&node), 0);
}

#[tokio::test]
async fn a_read_only_source_and_a_remote_node_without_the_opt_in_get_nothing() {
    let mut explorer = MockNodeRpc::new()
        .with_source(ChainSource::Explorer)
        .with_blockchain_info(on_chain("regtest"))
        .with_send_raw_transaction("00".repeat(32));
    explorer.node_url = "http://127.0.0.1:14037".into();
    let io = HnsIo {
        node: &explorer,
        settings: &HashMap::new(),
        network: fx::NET,
    };
    let e = runner::send_hns(&io, &funding_hex())
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("read-only"), "{e}");
    assert_eq!(sends(&explorer), 0);

    let mut remote = MockNodeRpc::new()
        .with_source(ChainSource::LocalNode)
        .with_blockchain_info(on_chain("regtest"))
        .with_send_raw_transaction("00".repeat(32));
    remote.node_url = "http://10.0.0.5:14037".into();
    let io = HnsIo {
        node: &remote,
        settings: &HashMap::new(),
        network: fx::NET,
    };
    let e = runner::send_hns(&io, &funding_hex())
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("remote node"), "{e}");
    assert_eq!(sends(&remote), 0);
}

#[tokio::test]
async fn an_own_node_on_the_trades_chain_gets_exactly_the_presigned_hex() {
    let hex = funding_hex();
    let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let log = sent.clone();
    let mut node = MockNodeRpc::new()
        .with_source(ChainSource::LocalNode)
        .with_blockchain_info(on_chain("regtest"))
        .with_raw_transaction_not_found()
        .with_send_raw_transaction_fn(move |raw: String| {
            log.lock().unwrap().push(raw);
            Ok("00".repeat(32))
        });
    node.node_url = "http://127.0.0.1:14037".into();
    let io = HnsIo {
        node: &node,
        settings: &HashMap::new(),
        network: fx::NET,
    };
    // The mock has no mempool: the send goes out and is reported not taken.
    assert!(!runner::send_hns(&io, &hex).await.unwrap());
    assert_eq!(*sent.lock().unwrap(), vec![hex]);
}
