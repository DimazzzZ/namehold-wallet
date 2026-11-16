//! A Bitcoin chain in memory, for the exchange's end-to-end tests: blocks
//! mined at regtest difficulty with real headers and Merkle roots, a
//! mempool, and every transaction it accepts run through Bitcoin Core's
//! interpreter (libbitcoinconsensus) against the coins it spends, with the
//! lock time judged against the median time past (BIP 113). It also serves
//! as an in-memory relay. Nothing here is trusted by the code under test:
//! the BTC lock is still proven by `btc_spv` over what it returns.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use bitcoin::block::{Header, Version};
use bitcoin::consensus::validation::verify_transaction;
use bitcoin::hashes::Hash;
use bitcoin::pow::CompactTarget;
use bitcoin::{
    Address, Amount, BlockHash, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxMerkleNode,
    TxOut, Txid, Witness,
};

use crate::error::AppError;
use crate::exchange_io::btc_source::{branch_for, BtcSource, Outspend, SourceUtxo};
use crate::exchange_io::meeting::{MeetingPoint, Watchtower};
use crate::noncustodial::exchange::relay_msg::{self, Envelope, SignedOffer};
use crate::noncustodial::exchange::watch::WatchJob;

struct Block {
    header: Header,
    txs: Vec<Transaction>,
}

struct Chain {
    blocks: Vec<Block>,
    mempool: Vec<Transaction>,
    /// Every coin ever created: (output, spent).
    coins: HashMap<OutPoint, (TxOut, bool)>,
    spent_by: HashMap<OutPoint, Txid>,
    txs: HashMap<Txid, Transaction>,
    time: u32,
    nonce: u32,
}

/// Cheap to clone: clones share one chain, so a test can keep a handle and
/// give the code under test a `Box<dyn BtcSource>` of the same chain.
#[derive(Clone)]
pub struct FakeBtc {
    chain: std::sync::Arc<Mutex<Chain>>,
    pub broadcasts: std::sync::Arc<Mutex<Vec<Transaction>>>,
}

fn merkle_root(txs: &[Transaction]) -> TxMerkleNode {
    let mut level: Vec<[u8; 32]> = txs
        .iter()
        .map(|t| t.compute_txid().to_raw_hash().to_byte_array())
        .collect();
    if level.is_empty() {
        return TxMerkleNode::all_zeros();
    }
    while level.len() > 1 {
        if level.len() % 2 == 1 {
            level.push(*level.last().unwrap());
        }
        level = level
            .chunks(2)
            .map(|c| {
                let mut b = [0u8; 64];
                b[..32].copy_from_slice(&c[0]);
                b[32..].copy_from_slice(&c[1]);
                bitcoin::hashes::sha256d::Hash::hash(&b).to_byte_array()
            })
            .collect();
    }
    TxMerkleNode::from_byte_array(level[0])
}

impl Chain {
    fn mtp(&self) -> u64 {
        let mut t: Vec<u32> = self
            .blocks
            .iter()
            .rev()
            .take(11)
            .map(|b| b.header.time)
            .collect();
        t.sort();
        u64::from(t[t.len() / 2])
    }

    fn mine(&mut self) {
        let txs = std::mem::take(&mut self.mempool);
        // A coinbase-like first transaction keeps every block's Merkle tree
        // non-trivial.
        self.nonce += 1;
        let cb = Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::from_bytes(self.nonce.to_le_bytes().to_vec()),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::ZERO,
                script_pubkey: ScriptBuf::new_op_return([0u8; 4]),
            }],
        };
        let mut all = vec![cb];
        all.extend(txs);
        self.time += 600;
        let prev = self
            .blocks
            .last()
            .map(|b| b.header.block_hash())
            .unwrap_or_else(BlockHash::all_zeros);
        let mut h = Header {
            version: Version::TWO,
            prev_blockhash: prev,
            merkle_root: merkle_root(&all),
            time: self.time,
            bits: CompactTarget::from_consensus(0x207f_ffff),
            nonce: 0,
        };
        while h.validate_pow(h.target()).is_err() {
            h.nonce += 1;
        }
        for t in &all {
            self.txs.insert(t.compute_txid(), t.clone());
        }
        self.blocks.push(Block {
            header: h,
            txs: all,
        });
    }

    fn height_of(&self, txid: &Txid) -> Option<u64> {
        self.blocks
            .iter()
            .position(|b| b.txs.iter().any(|t| t.compute_txid() == *txid))
            .map(|p| p as u64)
    }
}

impl FakeBtc {
    pub fn new(start_time: u32) -> FakeBtc {
        let mut c = Chain {
            blocks: Vec::new(),
            mempool: Vec::new(),
            coins: HashMap::new(),
            spent_by: HashMap::new(),
            txs: HashMap::new(),
            time: start_time - 600 * 12,
            nonce: 0,
        };
        for _ in 0..11 {
            c.mine();
        }
        FakeBtc {
            chain: std::sync::Arc::new(Mutex::new(c)),
            broadcasts: std::sync::Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// A confirmed coin of `value` to `address`, out of nothing.
    pub fn fund(&self, address: &Address, value: u64) -> OutPoint {
        let mut c = self.chain.lock().unwrap();
        c.nonce += 1;
        let tx = Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::from_bytes(c.nonce.to_be_bytes().to_vec()),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(value),
                script_pubkey: address.script_pubkey(),
            }],
        };
        let op = OutPoint {
            txid: tx.compute_txid(),
            vout: 0,
        };
        c.coins.insert(op, (tx.output[0].clone(), false));
        c.mempool.push(tx);
        c.mine();
        op
    }

    pub fn mine(&self, n: usize) {
        let mut c = self.chain.lock().unwrap();
        for _ in 0..n {
            c.mine();
        }
    }

    pub fn spender_of(&self, op: &OutPoint) -> Option<Transaction> {
        let c = self.chain.lock().unwrap();
        c.spent_by
            .get(op)
            .and_then(|t| c.txs.get(t).cloned())
            .or_else(|| {
                c.mempool
                    .iter()
                    .find(|t| t.input.iter().any(|i| i.previous_output == *op))
                    .cloned()
            })
    }
}

fn err(m: impl std::fmt::Display) -> AppError {
    AppError::Rpc(format!("fake btc: {m}"))
}

#[async_trait]
impl BtcSource for FakeBtc {
    fn label(&self) -> String {
        "fake".into()
    }

    async fn tip(&self) -> Result<(u64, u64), AppError> {
        let c = self.chain.lock().unwrap();
        Ok(((c.blocks.len() - 1) as u64, c.mtp()))
    }

    async fn raw_tx(&self, txid: &Txid) -> Result<Option<Transaction>, AppError> {
        let c = self.chain.lock().unwrap();
        Ok(c.txs.get(txid).cloned().or_else(|| {
            c.mempool
                .iter()
                .find(|t| t.compute_txid() == *txid)
                .cloned()
        }))
    }

    async fn tx_block(&self, txid: &Txid) -> Result<Option<(u64, BlockHash)>, AppError> {
        let c = self.chain.lock().unwrap();
        Ok(c.height_of(txid)
            .map(|h| (h, c.blocks[h as usize].header.block_hash())))
    }

    async fn merkle(
        &self,
        txid: &Txid,
        block: &BlockHash,
    ) -> Result<(Vec<TxMerkleNode>, u32), AppError> {
        let c = self.chain.lock().unwrap();
        let b = c
            .blocks
            .iter()
            .find(|b| b.header.block_hash() == *block)
            .ok_or_else(|| err("no such block"))?;
        let ids: Vec<Txid> = b.txs.iter().map(|t| t.compute_txid()).collect();
        branch_for(&ids, txid).ok_or_else(|| err("not in block"))
    }

    async fn header_at(&self, height: u64) -> Result<Header, AppError> {
        let c = self.chain.lock().unwrap();
        c.blocks
            .get(height as usize)
            .map(|b| b.header)
            .ok_or_else(|| err("no such height"))
    }

    async fn header(&self, hash: &BlockHash) -> Result<Header, AppError> {
        let c = self.chain.lock().unwrap();
        c.blocks
            .iter()
            .find(|b| b.header.block_hash() == *hash)
            .map(|b| b.header)
            .ok_or_else(|| err("no such block"))
    }

    async fn outspend(&self, o: &OutPoint) -> Result<Option<Outspend>, AppError> {
        let c = self.chain.lock().unwrap();
        let Some((_, spent)) = c.coins.get(o) else {
            return Ok(None);
        };
        if !spent {
            return Ok(Some(Outspend::Unspent));
        }
        Ok(Some(Outspend::Spent {
            txid: c.spent_by.get(o).copied(),
        }))
    }

    async fn utxos(&self, address: &Address) -> Result<Vec<SourceUtxo>, AppError> {
        let c = self.chain.lock().unwrap();
        let spk = address.script_pubkey();
        Ok(c.coins
            .iter()
            .filter(|(_, (o, spent))| !spent && o.script_pubkey == spk)
            .map(|(op, (o, _))| SourceUtxo {
                outpoint: *op,
                value: o.value.to_sat(),
                height: c.height_of(&op.txid),
            })
            .collect())
    }

    async fn fee_rate(&self) -> Result<u64, AppError> {
        Ok(1)
    }

    async fn broadcast(&self, tx_hex: &str) -> Result<Txid, AppError> {
        let tx: Transaction =
            bitcoin::consensus::deserialize(&hex::decode(tx_hex).map_err(err)?).map_err(err)?;
        let txid = tx.compute_txid();
        let mut c = self.chain.lock().unwrap();
        if c.txs.contains_key(&txid) || c.mempool.iter().any(|t| t.compute_txid() == txid) {
            return Ok(txid);
        }
        // BIP 125 replacement of our own spends: a transaction spending the
        // same outpoints as one in the mempool replaces it if it pays more.
        let conflicts: Vec<usize> = c
            .mempool
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                m.input.iter().any(|i| {
                    tx.input
                        .iter()
                        .any(|j| j.previous_output == i.previous_output)
                })
            })
            .map(|(n, _)| n)
            .collect();
        for i in &tx.input {
            let Some((_, spent)) = c.coins.get(&i.previous_output) else {
                return Err(err("spends a coin that does not exist"));
            };
            if *spent && conflicts.is_empty() {
                return Err(err("spends a spent coin"));
            }
        }
        // Lock time (BIP 113), only for a non-final transaction.
        let lt = tx.lock_time.to_consensus_u32();
        if lt != 0 && tx.input.iter().any(|i| i.sequence != Sequence::MAX) {
            let ok = if lt >= 500_000_000 {
                u64::from(lt) < c.mtp()
            } else {
                u64::from(lt) < c.blocks.len() as u64
            };
            if !ok {
                return Err(err("non-final"));
            }
        }
        let coins = c.coins.clone();
        verify_transaction(&tx, |o| coins.get(o).map(|(out, _)| out.clone()))
            .map_err(|e| err(format!("{e:?}")))?;
        let fee_in: u64 = tx
            .input
            .iter()
            .map(|i| coins[&i.previous_output].0.value.to_sat())
            .sum();
        let fee_out: u64 = tx.output.iter().map(|o| o.value.to_sat()).sum();
        let fee = fee_in - fee_out;
        for n in conflicts.iter().rev() {
            let old = c.mempool[*n].clone();
            let old_out: u64 = old.output.iter().map(|o| o.value.to_sat()).sum();
            if fee_in - old_out >= fee {
                return Err(err("replacement pays no more"));
            }
            c.mempool.remove(*n);
        }
        for i in &tx.input {
            c.coins.get_mut(&i.previous_output).unwrap().1 = true;
            c.spent_by.insert(i.previous_output, txid);
        }
        for (v, o) in tx.output.iter().enumerate() {
            c.coins.insert(
                OutPoint {
                    txid,
                    vout: v as u32,
                },
                (o.clone(), false),
            );
        }
        c.mempool.push(tx.clone());
        drop(c);
        self.broadcasts.lock().unwrap().push(tx);
        Ok(txid)
    }
}

/// An in-memory relay: offers by id, messages in arrival order.
#[derive(Default)]
pub struct FakeRelay {
    pub offers: Mutex<Vec<SignedOffer>>,
    pub messages: Mutex<Vec<Envelope>>,
    pub jobs: Mutex<Vec<WatchJob>>,
}

#[async_trait]
impl MeetingPoint for FakeRelay {
    async fn publish_offer(
        &self,
        o: &SignedOffer,
        key: &secp256k1::SecretKey,
    ) -> Result<(), AppError> {
        assert_eq!(
            relay_msg::relay_pubkey_hex(key),
            o.offer.maker_relay_pubkey,
            "the code under test published an offer with another key"
        );
        let mut v = self.offers.lock().unwrap();
        if !v.iter().any(|x| x.offer.id() == o.offer.id()) {
            v.push(o.clone());
        }
        Ok(())
    }
    async fn withdraw_offer(
        &self,
        o: &SignedOffer,
        signature: &str,
        key: &secp256k1::SecretKey,
    ) -> Result<(), AppError> {
        assert_eq!(relay_msg::relay_pubkey_hex(key), o.offer.maker_relay_pubkey);
        assert!(relay_msg::verify_withdraw(
            &o.offer.maker_relay_pubkey,
            &o.offer.id(),
            signature
        ));
        self.offers
            .lock()
            .unwrap()
            .retain(|x| x.offer.id() != o.offer.id());
        Ok(())
    }
    async fn offers(
        &self,
        network: &str,
        _now: u64,
    ) -> Result<(Vec<SignedOffer>, usize), AppError> {
        let v = self.offers.lock().unwrap();
        Ok((
            v.iter()
                .filter(|o| o.offer.network == network)
                .cloned()
                .collect(),
            0,
        ))
    }
    async fn send(&self, e: &Envelope, key: &secp256k1::SecretKey) -> Result<(), AppError> {
        assert!(e.verify(), "the code under test sent an unsigned message");
        assert_eq!(
            relay_msg::relay_pubkey_hex(key),
            e.message.from,
            "the code under test sent a message with another key"
        );
        self.messages.lock().unwrap().push(e.clone());
        Ok(())
    }
    async fn inbox(
        &self,
        key: &secp256k1::SecretKey,
        _network: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<Envelope>, Option<String>), AppError> {
        let to = relay_msg::relay_pubkey_hex(key);
        let v = self.messages.lock().unwrap();
        let from: usize = cursor.and_then(|c| c.parse().ok()).unwrap_or(0);
        let msgs = v
            .iter()
            .skip(from)
            .filter(|e| e.message.to == to)
            .cloned()
            .collect();
        Ok((msgs, Some(v.len().to_string())))
    }
}

#[async_trait]
impl Watchtower for FakeRelay {
    async fn watch(&self, job: &WatchJob) -> Result<String, AppError> {
        job.check()?;
        self.jobs.lock().unwrap().push(job.clone());
        Ok(format!("{}-{:?}", job.terms.id(), job.role))
    }
}
