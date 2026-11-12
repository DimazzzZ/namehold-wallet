//! Checking a Bitcoin transaction without trusting the source that reports
//! it (spec §3.6, R9): a Merkle proof places the transaction in a block
//! whose header has valid proof of work, and the required number of later
//! headers link onto it, each with valid proof of work. On mainnet every
//! header must also carry at least [`MAINNET_MIN_DIFFICULTY`]: a source that
//! invents a block would have to mine it at that difficulty.
//!
//! The 64-byte-transaction ambiguity of Bitcoin's Merkle tree
//! (CVE-2017-12842) is closed by refusing a transaction whose stripped size
//! is 64 bytes, as Bitcoin Core does since 0.16.

use bitcoin::block::Header;
use bitcoin::hashes::{sha256d, Hash};
use bitcoin::{BlockHash, Transaction, TxMerkleNode, Txid};

use crate::error::AppError;
use crate::noncustodial::exchange::btc_htlc::btc_network;
use crate::noncustodial::network::Network;

/// The least difficulty a mainnet header must have. Bitcoin's difficulty
/// has been above 10^14 since 2024; a floor at half of that leaves room for
/// a large fall while costing an attacker who fakes one block hundreds of
/// thousands of dollars of mining. If the real difficulty ever fell below
/// it, trades would be refused (fail closed), not accepted.
pub const MAINNET_MIN_DIFFICULTY: u128 = 50_000_000_000_000;

/// A proof that a transaction is in the Bitcoin chain, as a source reports
/// it: the raw transaction, the header of its block, the Merkle branch, and
/// the headers that follow (each the next block).
#[derive(Debug, Clone)]
pub struct InclusionProof {
    pub tx: Transaction,
    pub header: Header,
    /// Sibling hashes from the leaf up (Electrum `get_merkle` order).
    pub merkle: Vec<TxMerkleNode>,
    /// The transaction's index in the block.
    pub pos: u32,
    /// The headers after the block, in order.
    pub later: Vec<Header>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub txid: Txid,
    pub block_hash: BlockHash,
    /// 1 for the block itself plus one per linked later header.
    pub confirmations: u32,
}

fn bad(m: impl Into<String>) -> AppError {
    AppError::InvalidInput(format!("the Bitcoin proof is refused: {}", m.into()))
}

fn check_header(h: &Header, network: Network) -> Result<BlockHash, AppError> {
    let hash = h
        .validate_pow(h.target())
        .map_err(|e| bad(format!("a header's proof of work is invalid: {e}")))?;
    let net = btc_network(network);
    // The target must be attainable on the network at all.
    let params = bitcoin::params::Params::new(net);
    if h.target() > params.max_attainable_target {
        return Err(bad("a header's target is above the network's maximum"));
    }
    if network == Network::Main && h.difficulty(&params) < MAINNET_MIN_DIFFICULTY {
        return Err(bad("a header's difficulty is below the mainnet floor"));
    }
    Ok(hash)
}

/// The Merkle root the branch gives for `txid` at `pos`.
pub fn merkle_root(
    txid: Txid,
    merkle: &[TxMerkleNode],
    pos: u32,
) -> Result<TxMerkleNode, AppError> {
    if merkle.len() > 32 || (merkle.len() < 32 && u64::from(pos) >= (1u64 << merkle.len())) {
        return Err(bad("the Merkle branch does not fit the position"));
    }
    let mut h = txid.to_raw_hash().to_byte_array();
    for (i, sib) in merkle.iter().enumerate() {
        let s = sib.to_raw_hash().to_byte_array();
        let mut buf = [0u8; 64];
        if (pos >> i) & 1 == 1 {
            buf[..32].copy_from_slice(&s);
            buf[32..].copy_from_slice(&h);
        } else {
            buf[..32].copy_from_slice(&h);
            buf[32..].copy_from_slice(&s);
        }
        h = sha256d::Hash::hash(&buf).to_byte_array();
    }
    Ok(TxMerkleNode::from_byte_array(h))
}

/// Check `p` on `network` and return what it proves. The txid is computed
/// from the transaction's bytes here, never taken from the source.
pub fn verify(p: &InclusionProof, network: Network) -> Result<Verified, AppError> {
    let stripped = bitcoin::consensus::serialize(&strip_witness(&p.tx));
    if stripped.len() == 64 {
        return Err(bad("a 64-byte transaction is ambiguous in the Merkle tree"));
    }
    let txid = p.tx.compute_txid();
    let root = merkle_root(txid, &p.merkle, p.pos)?;
    if root != p.header.merkle_root {
        return Err(bad("the Merkle branch does not lead to the block's root"));
    }
    let block_hash = check_header(&p.header, network)?;
    let mut prev = block_hash;
    for h in &p.later {
        if h.prev_blockhash != prev {
            return Err(bad("a later header does not link to the one before"));
        }
        prev = check_header(h, network)?;
    }
    Ok(Verified {
        txid,
        block_hash,
        confirmations: 1 + p.later.len() as u32,
    })
}

fn strip_witness(tx: &Transaction) -> Transaction {
    let mut t = tx.clone();
    for i in &mut t.input {
        i.witness.clear();
    }
    t
}
