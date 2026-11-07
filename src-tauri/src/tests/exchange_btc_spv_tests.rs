//! Proving a Bitcoin transaction by proof of work (spec §3.6, R9). Blocks
//! are mined here at regtest difficulty; the mainnet floor refuses them,
//! which is exactly the attack it exists for (a source inventing blocks).

use bitcoin::block::{Header, Version};
use bitcoin::hashes::Hash;
use bitcoin::pow::CompactTarget;
use bitcoin::{BlockHash, Transaction, TxMerkleNode};

use super::exchange_fixture as fx;
use crate::noncustodial::exchange::btc_spv::{self, InclusionProof};
use crate::noncustodial::network::Network;

const REGTEST_BITS: u32 = 0x207f_ffff;

fn mine(prev: BlockHash, merkle_root: TxMerkleNode, time: u32) -> Header {
    let mut h = Header {
        version: Version::TWO,
        prev_blockhash: prev,
        merkle_root,
        time,
        bits: CompactTarget::from_consensus(REGTEST_BITS),
        nonce: 0,
    };
    while h.validate_pow(h.target()).is_err() {
        h.nonce += 1;
    }
    h
}

/// A block holding `txs`, its Merkle branch for `pos`, and `later` headers.
fn proof(txs: &[Transaction], pos: usize, later: usize) -> InclusionProof {
    let leaves: Vec<TxMerkleNode> = txs
        .iter()
        .map(|t| TxMerkleNode::from_raw_hash(t.compute_txid().to_raw_hash()))
        .collect();
    // Build the branch by hand (Bitcoin duplicates the last odd node).
    let mut level = leaves.clone();
    let mut branch = Vec::new();
    let mut idx = pos;
    while level.len() > 1 {
        if level.len() % 2 == 1 {
            level.push(*level.last().unwrap());
        }
        branch.push(level[idx ^ 1]);
        level = level
            .chunks(2)
            .map(|c| {
                let mut b = [0u8; 64];
                b[..32].copy_from_slice(&c[0].to_byte_array());
                b[32..].copy_from_slice(&c[1].to_byte_array());
                TxMerkleNode::from_byte_array(
                    bitcoin::hashes::sha256d::Hash::hash(&b).to_byte_array(),
                )
            })
            .collect();
        idx /= 2;
    }
    let root = level[0];
    let header = mine(BlockHash::all_zeros(), root, 1_800_000_000);
    let mut prev = header.block_hash();
    let mut hs = Vec::new();
    for i in 0..later {
        let h = mine(prev, TxMerkleNode::all_zeros(), 1_800_000_600 + i as u32);
        prev = h.block_hash();
        hs.push(h);
    }
    InclusionProof {
        tx: txs[pos].clone(),
        header,
        merkle: branch,
        pos: pos as u32,
        later: hs,
    }
}

fn txs() -> Vec<Transaction> {
    let t = fx::trade();
    let mut v = vec![t.buyer.funding.clone()];
    for r in &t.buyer.btc_refunds {
        v.push(crate::noncustodial::exchange::btc_htlc::decode(&r.tx_hex).unwrap());
    }
    v
}

#[test]
fn a_transaction_in_a_mined_block_with_linked_headers_is_proven() {
    let all = txs();
    for pos in 0..all.len() {
        let p = proof(&all, pos, 2);
        let v = btc_spv::verify(&p, Network::Regtest).unwrap();
        assert_eq!(v.txid, all[pos].compute_txid());
        assert_eq!(v.confirmations, 3);
        assert_eq!(v.block_hash, p.header.block_hash());
    }
}

#[test]
fn a_wrong_branch_position_or_transaction_is_refused() {
    let all = txs();
    let mut p = proof(&all, 1, 0);
    p.pos = 2;
    assert!(btc_spv::verify(&p, Network::Regtest).is_err());
    let mut p = proof(&all, 1, 0);
    p.tx = all[0].clone();
    assert!(btc_spv::verify(&p, Network::Regtest).is_err());
    let mut p = proof(&all, 1, 0);
    p.merkle[0] = TxMerkleNode::all_zeros();
    assert!(btc_spv::verify(&p, Network::Regtest).is_err());
}

#[test]
fn a_header_without_proof_of_work_is_refused() {
    let all = txs();
    let mut p = proof(&all, 0, 1);
    // Find a nonce that fails the target.
    let mut h = p.header;
    loop {
        h.nonce = h.nonce.wrapping_add(1);
        if h.validate_pow(h.target()).is_err() {
            break;
        }
    }
    p.header = h;
    assert!(btc_spv::verify(&p, Network::Regtest).is_err());
}

#[test]
fn later_headers_must_link() {
    let all = txs();
    let mut p = proof(&all, 0, 2);
    p.later.swap(0, 1);
    assert!(btc_spv::verify(&p, Network::Regtest).is_err());
    let mut p = proof(&all, 0, 2);
    p.later[1] = mine(BlockHash::all_zeros(), TxMerkleNode::all_zeros(), 1);
    assert!(btc_spv::verify(&p, Network::Regtest).is_err());
}

#[test]
fn mainnet_refuses_blocks_below_the_difficulty_floor() {
    let all = txs();
    let p = proof(&all, 0, 1);
    let e = btc_spv::verify(&p, Network::Main).unwrap_err().to_string();
    assert!(
        e.contains("above the network's maximum") || e.contains("floor"),
        "{e}"
    );
}

#[test]
fn a_64_byte_transaction_is_refused() {
    // A stripped transaction of exactly 64 bytes: version 4, 1 input with
    // empty script 41, 0 outputs... 4+1+41+1+0+4 = 51; add an output of 13.
    use bitcoin::{
        absolute::LockTime, Amount, OutPoint, ScriptBuf, Sequence, TxIn, TxOut, Witness,
    };
    let tx = Transaction {
        version: bitcoin::transaction::Version::ONE,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::ZERO,
            script_pubkey: ScriptBuf::from_bytes(vec![0x51; 4]),
        }],
    };
    assert_eq!(bitcoin::consensus::serialize(&tx).len(), 64);
    let p = proof(&[tx], 0, 0);
    let e = btc_spv::verify(&p, Network::Regtest)
        .unwrap_err()
        .to_string();
    assert!(e.contains("64-byte"), "{e}");
}

#[test]
fn a_branch_longer_than_the_position_allows_is_refused() {
    assert!(btc_spv::merkle_root(
        bitcoin::Txid::all_zeros(),
        &[TxMerkleNode::all_zeros(); 2],
        4
    )
    .is_err());
    assert!(btc_spv::merkle_root(
        bitcoin::Txid::all_zeros(),
        &[TxMerkleNode::all_zeros(); 33],
        0
    )
    .is_err());
}
