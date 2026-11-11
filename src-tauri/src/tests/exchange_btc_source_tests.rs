//! The Bitcoin sources' parsing (`exchange_io/btc_source.rs`), each served
//! by a local HTTP mock.
//!
//! The Esplora replies are **recorded** from `https://mempool.space/api` on
//! 2026-11-09 for the pizza transaction (`a1075db5…d48d`, block 57043), so
//! the parsing and the Merkle branch's byte order are checked against what
//! a real server sends; `/fee-estimates` came back `203`, which the client
//! takes as success like any 2xx. The Bitcoin Core replies are written from
//! Bitcoin Core's RPC documentation (`getindexinfo`, `estimatesmartfee`,
//! `getrawtransaction`, `getblockheader`, `gettxout`), **not recorded**: no
//! bitcoind ran where this was written, and the live regtest QA cases cover
//! the real program.

use bitcoin::{BlockHash, OutPoint, Txid};
use mockito::Matcher;

use crate::exchange_io::btc_source::{BitcoinCore, BtcSource, Esplora, Outspend};
use crate::noncustodial::exchange::btc_spv;

const PIZZA: &str = "a1075db55d416d3ca199f55b6084e2115b9345e16c5cf302fc80e9d5fbf5d48d";
const BLOCK_57043: &str = "00000000152340ca42227603908689183edc47355204e7aca59383b0aaac1fd8";
const TX_HEX: &str = include_str!("fixtures/esplora/tx_hex.txt");
const TX_STATUS: &str = include_str!("fixtures/esplora/tx_status.json");
const MERKLE_PROOF: &str = include_str!("fixtures/esplora/tx_merkle_proof.json");
const OUTSPEND_0: &str = include_str!("fixtures/esplora/tx_outspend_0.json");
const BLOCK: &str = include_str!("fixtures/esplora/block.json");
const HEADER: &str = include_str!("fixtures/esplora/block_header.txt");
const HEIGHT_57044: &str = include_str!("fixtures/esplora/block_height_57044.txt");
const HEADER_57044: &str = include_str!("fixtures/esplora/block_header_57044.txt");
const FEES: &str = include_str!("fixtures/esplora/fee_estimates.json");
const ADDRESS_UTXO: &str = include_str!("fixtures/esplora/address_utxo.json");

fn txid() -> Txid {
    PIZZA.parse().unwrap()
}

async fn esplora(server: &mockito::ServerGuard) -> Esplora {
    Esplora::new(&server.url()).unwrap()
}

#[tokio::test]
async fn esplora_reads_a_recorded_transaction_its_block_and_its_proof() {
    let mut server = mockito::Server::new_async().await;
    let _hex = server
        .mock("GET", format!("/tx/{PIZZA}/hex").as_str())
        .with_body(TX_HEX)
        .create_async()
        .await;
    let _status = server
        .mock("GET", format!("/tx/{PIZZA}/status").as_str())
        .with_body(TX_STATUS)
        .create_async()
        .await;
    let _proof = server
        .mock("GET", format!("/tx/{PIZZA}/merkle-proof").as_str())
        .with_body(MERKLE_PROOF)
        .create_async()
        .await;
    let _header = server
        .mock("GET", format!("/block/{BLOCK_57043}/header").as_str())
        .with_body(HEADER)
        .create_async()
        .await;
    let src = esplora(&server).await;

    // The txid is computed from the bytes, and is the one asked for.
    let tx = src.raw_tx(&txid()).await.unwrap().expect("known");
    assert_eq!(tx.compute_txid(), txid());

    let (height, hash) = src.tx_block(&txid()).await.unwrap().expect("confirmed");
    assert_eq!(height, 57043);
    assert_eq!(hash, BLOCK_57043.parse::<BlockHash>().unwrap());

    // The branch, in the order and byte order Esplora sends it, leads to the
    // root of the block's real header: this is what no in-memory chain shows.
    let (branch, pos) = src.merkle(&txid(), &hash).await.unwrap();
    let header = src.header(&hash).await.unwrap();
    assert_eq!(header.block_hash(), hash);
    assert_eq!(
        btc_spv::merkle_root(txid(), &branch, pos).unwrap(),
        header.merkle_root
    );
}

#[tokio::test]
async fn esplora_reads_the_tip_the_next_header_a_spend_and_the_coins() {
    let mut server = mockito::Server::new_async().await;
    let _tip = server
        .mock("GET", "/blocks/tip/hash")
        .with_body(BLOCK_57043)
        .create_async()
        .await;
    let _block = server
        .mock("GET", format!("/block/{BLOCK_57043}").as_str())
        .with_body(BLOCK)
        .create_async()
        .await;
    let _at = server
        .mock("GET", "/block-height/57044")
        .with_body(HEIGHT_57044)
        .create_async()
        .await;
    let _next = server
        .mock(
            "GET",
            format!("/block/{}/header", HEIGHT_57044.trim()).as_str(),
        )
        .with_body(HEADER_57044)
        .create_async()
        .await;
    let _spend = server
        .mock("GET", format!("/tx/{PIZZA}/outspend/0").as_str())
        .with_body(OUTSPEND_0)
        .create_async()
        .await;
    let _utxo = server
        .mock("GET", "/address/17SkEw2md5avVNyYgj6RiXuQKNwkXaxFyQ/utxo")
        .with_body(ADDRESS_UTXO)
        .create_async()
        .await;
    let src = esplora(&server).await;

    let (height, mtp) = src.tip().await.unwrap();
    assert_eq!(height, 57043);
    assert!(mtp > 1_270_000_000 && mtp < 1_274_552_191, "{mtp}");

    let next = src.header_at(57044).await.unwrap();
    assert_eq!(
        next.prev_blockhash,
        BLOCK_57043.parse::<BlockHash>().unwrap()
    );

    let spend = src
        .outspend(&OutPoint {
            txid: txid(),
            vout: 0,
        })
        .await
        .unwrap();
    assert_eq!(
        spend,
        Some(Outspend::Spent {
            txid: Some(
                "cca7507897abc89628f450e8b1e0c6fca4ec3f7b34cccf55f3f531c659ff4d79"
                    .parse()
                    .unwrap()
            )
        })
    );

    let coins = src
        .utxos(
            &"17SkEw2md5avVNyYgj6RiXuQKNwkXaxFyQ"
                .parse::<bitcoin::Address<_>>()
                .unwrap()
                .assume_checked(),
        )
        .await
        .unwrap();
    assert!(!coins.is_empty());
    assert!(coins.iter().all(|c| c.height.is_some() && c.value > 0));
}

#[tokio::test]
async fn esplora_fee_estimates_round_up_and_a_reply_without_one_is_an_error() {
    let mut server = mockito::Server::new_async().await;
    let _fees = server
        .mock("GET", "/fee-estimates")
        .with_body(FEES)
        .create_async()
        .await;
    let src = esplora(&server).await;
    // The recorded reply says 2.114 sat/vB for 2 blocks.
    assert_eq!(src.fee_rate().await.unwrap(), 3);

    let mut server = mockito::Server::new_async().await;
    let _fees = server
        .mock("GET", "/fee-estimates")
        .with_body(r#"{"144":0.5}"#)
        .create_async()
        .await;
    let src = esplora(&server).await;
    let e = src.fee_rate().await.unwrap_err().to_string();
    assert!(e.contains("2- or 3-block"), "{e}");
}

#[tokio::test]
async fn esplora_404_is_unknown_not_an_error() {
    let mut server = mockito::Server::new_async().await;
    let _hex = server
        .mock("GET", format!("/tx/{PIZZA}/hex").as_str())
        .with_status(404)
        .create_async()
        .await;
    let src = esplora(&server).await;
    assert!(src.raw_tx(&txid()).await.unwrap().is_none());
}

// ------------------------------------------------------------ Bitcoin Core

fn rpc(server: &mut mockito::ServerGuard, method: &str, result: &str) -> mockito::Mock {
    server
        .mock("POST", "/")
        .match_body(Matcher::Regex(format!("\"method\":\"{method}\"")))
        .with_body(format!(
            r#"{{"result":{result},"error":null,"id":"namehold"}}"#
        ))
        .create()
}

fn rpc_error(
    server: &mut mockito::ServerGuard,
    method: &str,
    code: i64,
    message: &str,
) -> mockito::Mock {
    server
        .mock("POST", "/")
        .match_body(Matcher::Regex(format!("\"method\":\"{method}\"")))
        .with_status(500)
        .with_body(format!(
            r#"{{"result":null,"error":{{"code":{code},"message":"{message}"}},"id":"namehold"}}"#
        ))
        .create()
}

fn core(server: &mockito::ServerGuard) -> BitcoinCore {
    let url = server.url();
    BitcoinCore::new(&format!(
        "bitcoind+{}",
        url.replacen("http://", "http://user:pass@", 1)
    ))
    .unwrap()
}

#[tokio::test]
async fn core_is_ready_only_with_a_synced_txindex() {
    let mut server = mockito::Server::new_async().await;
    let _ok = rpc(
        &mut server,
        "getindexinfo",
        r#"{"txindex":{"synced":true,"best_block_height":100}}"#,
    );
    core(&server).ready().await.unwrap();

    let mut server = mockito::Server::new_async().await;
    let _none = rpc(&mut server, "getindexinfo", "{}");
    let e = core(&server).ready().await.unwrap_err().to_string();
    assert!(e.contains("without -txindex"), "{e}");

    let mut server = mockito::Server::new_async().await;
    let _building = rpc(
        &mut server,
        "getindexinfo",
        r#"{"txindex":{"synced":false,"best_block_height":10}}"#,
    );
    let e = core(&server).ready().await.unwrap_err().to_string();
    assert!(e.contains("still building"), "{e}");
}

#[tokio::test]
async fn core_fee_estimate_converts_btc_per_kvb_and_refuses_no_estimate() {
    let mut server = mockito::Server::new_async().await;
    let _fee = rpc(
        &mut server,
        "estimatesmartfee",
        r#"{"feerate":0.00002114,"blocks":2}"#,
    );
    assert_eq!(core(&server).fee_rate().await.unwrap(), 3);

    let mut server = mockito::Server::new_async().await;
    let _fee = rpc(
        &mut server,
        "estimatesmartfee",
        r#"{"errors":["Insufficient data or no feerate found"],"blocks":2}"#,
    );
    let e = core(&server).fee_rate().await.unwrap_err().to_string();
    assert!(e.contains("no rate"), "{e}");
}

/// The replies are Bitcoin Core v32.1's on a regtest chain without fee
/// history (recorded 2026-11-09 and 2026-11-11): `estimatesmartfee` has no
/// rate, `getmempoolinfo` has the relay floor. The floor is what a fallback
/// rate is built on (QA Q3).
#[tokio::test]
async fn core_without_an_estimate_still_gives_its_relay_floor() {
    let mut server = mockito::Server::new_async().await;
    let _fee = rpc(
        &mut server,
        "estimatesmartfee",
        r#"{"errors":["Insufficient data or no feerate found"],"blocks":0}"#,
    );
    let _info = rpc(
        &mut server,
        "getmempoolinfo",
        r#"{"loaded":true,"size":0,"bytes":0,"usage":0,"total_fee":0.00000000,"maxmempool":300000000,"mempoolminfee":0.00000100,"minrelaytxfee":0.00000100,"incrementalrelayfee":0.00000100,"unbroadcastcount":0,"permitbaremultisig":true,"maxdatacarriersize":100000,"limitclustercount":64,"limitclustersize":101000,"optimal":true}"#,
    );
    let src = core(&server);
    assert!(src.fee_rate().await.is_err());
    assert_eq!(src.floor_rate().await.unwrap(), 1);

    // A node under load raises its mempool minimum above the relay minimum:
    // the higher one is the floor, rounded up.
    let mut server = mockito::Server::new_async().await;
    let _info = rpc(
        &mut server,
        "getmempoolinfo",
        r#"{"loaded":true,"mempoolminfee":0.00003421,"minrelaytxfee":0.00000100}"#,
    );
    assert_eq!(core(&server).floor_rate().await.unwrap(), 4);

    // A reply without either field is an error, never a guessed 1 sat/vB.
    let mut server = mockito::Server::new_async().await;
    let _info = rpc(&mut server, "getmempoolinfo", r#"{"loaded":true}"#);
    let e = core(&server).floor_rate().await.unwrap_err().to_string();
    assert!(e.contains("no minimum fee"), "{e}");
}

#[tokio::test]
async fn core_reads_a_transaction_its_block_and_a_spent_output() {
    let mut server = mockito::Server::new_async().await;
    let _raw = rpc(
        &mut server,
        "getrawtransaction",
        &format!(
            r#"{{"txid":"{PIZZA}","hex":"{}","blockhash":"{BLOCK_57043}","confirmations":3}}"#,
            TX_HEX.trim()
        ),
    );
    let _hdr = rpc(
        &mut server,
        "getblockheader",
        &format!(r#"{{"hash":"{BLOCK_57043}","height":57043,"confirmations":3}}"#),
    );
    let _spent = rpc(&mut server, "gettxout", "null");
    let src = core(&server);
    assert_eq!(
        src.raw_tx(&txid()).await.unwrap().unwrap().compute_txid(),
        txid()
    );
    let (height, hash) = src.tx_block(&txid()).await.unwrap().unwrap();
    assert_eq!((height, hash), (57043, BLOCK_57043.parse().unwrap()));
    assert_eq!(
        src.outspend(&OutPoint {
            txid: txid(),
            vout: 0
        })
        .await
        .unwrap(),
        Some(Outspend::Spent { txid: None })
    );

    // Core's "No such mempool or blockchain transaction" (-5) is unknown,
    // not an error; any other refusal is one.
    let mut server = mockito::Server::new_async().await;
    let _nf = rpc_error(
        &mut server,
        "getrawtransaction",
        -5,
        "No such mempool or blockchain transaction",
    );
    assert!(core(&server).raw_tx(&txid()).await.unwrap().is_none());
}
