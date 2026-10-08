//! What the chain shows about a transaction hsd answers "Transaction not
//! found." for, and whether the node took one it answered
//! `sendrawtransaction` for.
//!
//! hsd looks a transaction up by txid in its mempool and then in its
//! transaction index (`node.getMeta` → `chaindb.getMeta`, null without
//! `--index-tx`), so its not-found is not evidence a transaction is in no
//! block. The coin lookups here need no transaction index
//! (`GET /coin/:hash/:index`: mempool, then the chain's UTXO set).

use crate::noncustodial::node_rpc::NodeRpc;
use crate::noncustodial::rpc::is_tx_not_found;
use crate::noncustodial::tx::Transaction;

/// What the chain shows about a draft's transaction that hsd answered with
/// its own "Transaction not found.".
///
/// That answer is not evidence the transaction is in no block: hsd looks a
/// transaction up in its mempool and then in its transaction index
/// (`node.getMeta` → `chaindb.getMeta`, null without `--index-tx`), so on a
/// node without the index every mined transaction reads as not found. The
/// coin lookups below need no transaction index (`GET /coin/:hash/:index`,
/// mempool then chain UTXO set), and decide the verdict instead.
#[derive(Debug, PartialEq, Eq)]
pub enum ChainEvidence {
    /// One of its outputs is a coin mined at this height: the transaction is
    /// in that block.
    Mined(i64),
    /// Every coin it spends is unspent: it is in no block and no mempool of
    /// this node, and its coins were not moved.
    InputsUnspent,
    /// A coin it spends is spent and none of its outputs is a coin: another
    /// transaction spent the coin, or — only on a node without a transaction
    /// index — it was mined and its outputs were spent since.
    InputsSpent,
    /// No answer that decides it (a transport error, a reply that is not
    /// hsd's, an output only in the mempool, a draft that does not parse).
    Unknown,
}

/// Read [`ChainEvidence`] for the transaction `txid`, serialized as
/// `raw_hex`, from the coins of its own outputs and of its inputs.
pub async fn chain_evidence_with_client(
    client: &dyn NodeRpc,
    raw_hex: &str,
    txid: &str,
) -> ChainEvidence {
    let Some(tx) = hex::decode(raw_hex)
        .ok()
        .and_then(|b| Transaction::decode(&b).ok())
        .filter(|tx| tx.txid() == txid)
    else {
        return ChainEvidence::Unknown;
    };
    for vout in 0..tx.outputs.len() as u32 {
        match client
            .get_coin(txid, vout)
            .await
            .map(|c| c.map(|c| c.mined_height()))
        {
            Ok(Some(Ok(Some(height)))) => return ChainEvidence::Mined(height),
            // Its coin in the mempool, though hsd just did not find the
            // transaction there; or a height that is not hsd's.
            Ok(Some(_)) | Err(_) => return ChainEvidence::Unknown,
            Ok(None) => {}
        }
    }
    let mut all_unspent = true;
    for input in &tx.inputs {
        let prev = hex::encode(input.prevout.hash);
        match client.get_coin(&prev, input.prevout.index).await {
            Ok(Some(_)) => {}
            Ok(None) => all_unspent = false,
            Err(_) => return ChainEvidence::Unknown,
        }
    }
    if all_unspent {
        ChainEvidence::InputsUnspent
    } else {
        ChainEvidence::InputsSpent
    }
}

/// How many times, and how far apart, a send is looked up after
/// `sendrawtransaction` answered: hsd answers with the txid before its
/// mempool has decided (`rpc.js` `sendRawTransaction` relays without
/// awaiting), and adds an accepted transaction asynchronously.
pub const TAKEN_CHECKS: u32 = 5;
pub const TAKEN_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(400);

/// Whether the node took a transaction it answered `sendrawtransaction` for.
#[derive(Debug, PartialEq, Eq)]
pub enum Taken {
    /// `getrawtransaction` finds it (mempool, or chain through the index).
    Yes,
    /// Not found, but one of its outputs is a coin mined at this height: on
    /// a node without a transaction index, a send mined in the meantime.
    Mined(i64),
    /// Every check got hsd's own "Transaction not found." and the chain shows
    /// no output of it mined: the node did not take it. hsd does not say why.
    No,
    /// A check got no answer, or one that is not hsd's: no verdict.
    Unknown,
}

/// Look `txid` up up to [`TAKEN_CHECKS`] times, [`TAKEN_CHECK_INTERVAL`]
/// apart; the first find ends the window. Only hsd's not-found on every check,
/// with no output mined ([`chain_evidence_with_client`]), is [`Taken::No`].
pub async fn taken_by_node_with_client(
    client: &dyn NodeRpc,
    signed_hex: &str,
    txid: &str,
) -> Taken {
    for check in 0..TAKEN_CHECKS {
        if check > 0 {
            tokio::time::sleep(TAKEN_CHECK_INTERVAL).await;
        }
        match client.get_raw_transaction(txid).await {
            Ok(_) => return Taken::Yes,
            Err(e) if is_tx_not_found(&e) => {}
            Err(_) => return Taken::Unknown,
        }
    }
    match chain_evidence_with_client(client, signed_hex, txid).await {
        ChainEvidence::Mined(height) => Taken::Mined(height),
        ChainEvidence::Unknown => Taken::Unknown,
        ChainEvidence::InputsUnspent | ChainEvidence::InputsSpent => Taken::No,
    }
}

/// The note a send the node did not take carries (spec honest-broadcast R2).
pub const NOT_TAKEN: &str = "The node did not take the transaction. hsd does not say why; often its coins are already spent elsewhere. You can try again; the coins stay held until the node is checked again.";

/// Whether the node keeps a transaction index: the coinbase of the block at
/// `tip` is mined, so hsd finds it with `getrawtransaction` exactly when it
/// has the index. `None` when the answer is not hsd's or does not come.
pub async fn node_has_tx_index_with_client(client: &dyn NodeRpc, tip: i64) -> Option<bool> {
    let hash = client.get_block_hash(tip).await.ok()?;
    let block = client.get_block(&hash).await.ok()?;
    let coinbase = block.get("tx")?.get(0)?.get("txid")?.as_str()?.to_string();
    match client.get_raw_transaction(&coinbase).await {
        Ok(_) => Some(true),
        Err(e) if is_tx_not_found(&e) => Some(false),
        Err(_) => None,
    }
}
