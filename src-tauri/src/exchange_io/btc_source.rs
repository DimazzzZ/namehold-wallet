//! Bitcoin sources (spec §3.6): an Esplora HTTP API (mempool.space,
//! blockstream.info, a self-hosted electrs) or a Bitcoin Core node over
//! JSON-RPC (`bitcoind+http://user:pass@host:port`, for regtest and for
//! users with their own node). Neither is trusted: a lock is accepted only
//! by `btc_spv::verify` over what a source returns, and every source must
//! report the same block for it.

use async_trait::async_trait;
use bitcoin::block::Header;
use bitcoin::hashes::Hash;
use bitcoin::{Address, BlockHash, OutPoint, Transaction, TxMerkleNode, Txid};

use crate::error::AppError;

/// Whether an outpoint is spent, as a source knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outspend {
    Unspent,
    /// Spent; `txid` when the source can name the spender (Esplora can,
    /// Bitcoin Core cannot).
    Spent {
        txid: Option<Txid>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceUtxo {
    pub outpoint: OutPoint,
    pub value: u64,
    pub height: Option<u64>,
}

#[async_trait]
pub trait BtcSource: Send + Sync {
    /// For messages: the source's host.
    fn label(&self) -> String;
    /// Whether the source can answer what the exchange asks (Bitcoin Core
    /// needs `-txindex`, review H2). `Err` says why not.
    async fn ready(&self) -> Result<(), AppError> {
        Ok(())
    }
    /// The tip's height and median time past.
    async fn tip(&self) -> Result<(u64, u64), AppError>;
    /// The transaction, or `None` when the source does not know it.
    async fn raw_tx(&self, txid: &Txid) -> Result<Option<Transaction>, AppError>;
    /// The block holding `txid`, or `None` while unconfirmed or unknown.
    async fn tx_block(&self, txid: &Txid) -> Result<Option<(u64, BlockHash)>, AppError>;
    /// The Merkle branch of `txid` in `block` and its position.
    async fn merkle(
        &self,
        txid: &Txid,
        block: &BlockHash,
    ) -> Result<(Vec<TxMerkleNode>, u32), AppError>;
    async fn header_at(&self, height: u64) -> Result<Header, AppError>;
    async fn header(&self, hash: &BlockHash) -> Result<Header, AppError>;
    /// Whether `outpoint` is spent. `Ok(None)` when the source does not
    /// know the transaction that created it.
    async fn outspend(&self, outpoint: &OutPoint) -> Result<Option<Outspend>, AppError>;
    async fn utxos(&self, address: &Address) -> Result<Vec<SourceUtxo>, AppError>;
    /// A rate to be mined within about two blocks, sat/vbyte, rounded up.
    async fn fee_rate(&self) -> Result<u64, AppError>;
    /// The lowest rate this source's node relays and keeps in its mempool,
    /// sat/vbyte, rounded up: not an estimate of anything, only the floor a
    /// transaction needs to travel. Used when no source has an estimate (a
    /// node that has just synced or restarted, QA 2026-11-09 Q3). A source
    /// that cannot say answers with an error.
    async fn floor_rate(&self) -> Result<u64, AppError> {
        Err(AppError::Other(
            "this source gives no minimum fee rate".into(),
        ))
    }
    /// Send a transaction; `Ok` when the source accepted it or already has it.
    async fn broadcast(&self, tx_hex: &str) -> Result<Txid, AppError>;
}

fn rpc_err(src: &str, m: impl std::fmt::Display) -> AppError {
    AppError::Rpc(format!("Bitcoin source {src}: {m}"))
}

// ------------------------------------------------------------------ Esplora

pub struct Esplora {
    base: String,
    http: reqwest::Client,
}

impl Esplora {
    pub fn new(base: &str) -> Result<Esplora, AppError> {
        let url = url::Url::parse(base)
            .map_err(|e| AppError::InvalidInput(format!("'{base}' is not a URL: {e}")))?;
        check_transport(&url)?;
        Ok(Esplora {
            base: base.trim_end_matches('/').to_owned(),
            http: crate::http::client_builder()
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .map_err(|e| AppError::Other(format!("http client: {e}")))?,
        })
    }

    async fn get_text(&self, path: &str) -> Result<Option<String>, AppError> {
        let r = self
            .http
            .get(format!("{}{}", self.base, path))
            .send()
            .await
            .map_err(|e| rpc_err(&self.base, e))?;
        if r.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !r.status().is_success() {
            return Err(rpc_err(&self.base, format!("{path}: HTTP {}", r.status())));
        }
        r.text().await.map(Some).map_err(|e| rpc_err(&self.base, e))
    }

    async fn need_text(&self, path: &str) -> Result<String, AppError> {
        self.get_text(path)
            .await?
            .ok_or_else(|| rpc_err(&self.base, format!("{path}: not found")))
    }

    async fn get_json(&self, path: &str) -> Result<Option<serde_json::Value>, AppError> {
        match self.get_text(path).await? {
            None => Ok(None),
            Some(t) => serde_json::from_str(&t)
                .map(Some)
                .map_err(|e| rpc_err(&self.base, format!("{path}: {e}"))),
        }
    }
}

/// `https://` anywhere, `http://` only to this machine (as the node URL).
pub fn check_transport(url: &url::Url) -> Result<(), AppError> {
    match url.scheme() {
        "https" => Ok(()),
        "http" if crate::http::is_loopback_url(url) => Ok(()),
        _ => Err(AppError::InvalidInput(format!(
            "a Bitcoin source must be https, or http to this machine: {url}"
        ))),
    }
}

fn parse_hex<T: bitcoin::consensus::Decodable>(src: &str, hex_s: &str) -> Result<T, AppError> {
    let bytes = hex::decode(hex_s.trim()).map_err(|_| rpc_err(src, "malformed hex"))?;
    bitcoin::consensus::deserialize(&bytes).map_err(|e| rpc_err(src, e))
}

fn parse_hash<T: std::str::FromStr>(src: &str, s: &str) -> Result<T, AppError> {
    s.trim()
        .parse::<T>()
        .map_err(|_| rpc_err(src, format!("'{s}' is not a hash")))
}

#[async_trait]
impl BtcSource for Esplora {
    fn label(&self) -> String {
        url::Url::parse(&self.base)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_else(|| self.base.clone())
    }

    async fn tip(&self) -> Result<(u64, u64), AppError> {
        let hash = self.need_text("/blocks/tip/hash").await?;
        let b = self
            .get_json(&format!("/block/{}", hash.trim()))
            .await?
            .ok_or_else(|| rpc_err(&self.base, "tip block not found"))?;
        let (Some(h), Some(mtp)) = (b["height"].as_u64(), b["mediantime"].as_u64()) else {
            return Err(rpc_err(
                &self.base,
                "tip block without height or median time",
            ));
        };
        Ok((h, mtp))
    }

    async fn raw_tx(&self, txid: &Txid) -> Result<Option<Transaction>, AppError> {
        match self.get_text(&format!("/tx/{txid}/hex")).await? {
            None => Ok(None),
            Some(h) => parse_hex(&self.base, &h).map(Some),
        }
    }

    async fn tx_block(&self, txid: &Txid) -> Result<Option<(u64, BlockHash)>, AppError> {
        let Some(s) = self.get_json(&format!("/tx/{txid}/status")).await? else {
            return Ok(None);
        };
        if s["confirmed"].as_bool() != Some(true) {
            return Ok(None);
        }
        let (Some(h), Some(bh)) = (s["block_height"].as_u64(), s["block_hash"].as_str()) else {
            return Err(rpc_err(&self.base, "confirmed status without block"));
        };
        Ok(Some((h, parse_hash(&self.base, bh)?)))
    }

    async fn merkle(
        &self,
        txid: &Txid,
        _block: &BlockHash,
    ) -> Result<(Vec<TxMerkleNode>, u32), AppError> {
        let p = self
            .get_json(&format!("/tx/{txid}/merkle-proof"))
            .await?
            .ok_or_else(|| rpc_err(&self.base, "no merkle proof"))?;
        let Some(pos) = p["pos"].as_u64().and_then(|n| u32::try_from(n).ok()) else {
            return Err(rpc_err(&self.base, "merkle proof without position"));
        };
        let Some(list) = p["merkle"].as_array() else {
            return Err(rpc_err(&self.base, "merkle proof without branch"));
        };
        // Electrum order, each hash in display (reversed) hex.
        let branch = list
            .iter()
            .map(|v| {
                let s = v
                    .as_str()
                    .ok_or_else(|| rpc_err(&self.base, "bad merkle node"))?;
                let t: Txid = parse_hash(&self.base, s)?;
                Ok(TxMerkleNode::from_raw_hash(t.to_raw_hash()))
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        Ok((branch, pos))
    }

    async fn header_at(&self, height: u64) -> Result<Header, AppError> {
        let hash: BlockHash = parse_hash(
            &self.base,
            &self.need_text(&format!("/block-height/{height}")).await?,
        )?;
        self.header(&hash).await
    }

    async fn header(&self, hash: &BlockHash) -> Result<Header, AppError> {
        parse_hex(
            &self.base,
            &self.need_text(&format!("/block/{hash}/header")).await?,
        )
    }

    async fn outspend(&self, o: &OutPoint) -> Result<Option<Outspend>, AppError> {
        let Some(v) = self
            .get_json(&format!("/tx/{}/outspend/{}", o.txid, o.vout))
            .await?
        else {
            return Ok(None);
        };
        match v["spent"].as_bool() {
            Some(false) => Ok(Some(Outspend::Unspent)),
            Some(true) => {
                let txid = v["txid"]
                    .as_str()
                    .map(|s| parse_hash(&self.base, s))
                    .transpose()?;
                Ok(Some(Outspend::Spent { txid }))
            }
            None => Err(rpc_err(&self.base, "outspend without `spent`")),
        }
    }

    async fn utxos(&self, address: &Address) -> Result<Vec<SourceUtxo>, AppError> {
        let v = self
            .get_json(&format!("/address/{address}/utxo"))
            .await?
            .unwrap_or(serde_json::Value::Array(Vec::new()));
        let Some(list) = v.as_array() else {
            return Err(rpc_err(&self.base, "utxo list is not a list"));
        };
        list.iter()
            .map(|u| {
                let (Some(txid), Some(vout), Some(value)) =
                    (u["txid"].as_str(), u["vout"].as_u64(), u["value"].as_u64())
                else {
                    return Err(rpc_err(&self.base, "malformed utxo"));
                };
                let height = if u["status"]["confirmed"].as_bool() == Some(true) {
                    u["status"]["block_height"].as_u64()
                } else {
                    None
                };
                Ok(SourceUtxo {
                    outpoint: OutPoint {
                        txid: parse_hash(&self.base, txid)?,
                        vout: u32::try_from(vout).map_err(|_| rpc_err(&self.base, "bad vout"))?,
                    },
                    value,
                    height,
                })
            })
            .collect()
    }

    async fn fee_rate(&self) -> Result<u64, AppError> {
        let v = self
            .get_json("/fee-estimates")
            .await?
            .ok_or_else(|| rpc_err(&self.base, "no fee estimates"))?;
        // The presigned ladder is built from this rate and cannot be re-signed
        // (spec §6), so a reply without one is an error, never 1 sat/vB.
        let r = v["2"]
            .as_f64()
            .or_else(|| v["3"].as_f64())
            .ok_or_else(|| rpc_err(&self.base, "fee estimates without a 2- or 3-block rate"))?;
        Ok(r.ceil().max(1.0) as u64)
    }

    async fn broadcast(&self, tx_hex: &str) -> Result<Txid, AppError> {
        let r = self
            .http
            .post(format!("{}/tx", self.base))
            .body(tx_hex.to_owned())
            .send()
            .await
            .map_err(|e| rpc_err(&self.base, e))?;
        let ok = r.status().is_success();
        let body = r.text().await.unwrap_or_default();
        if !ok {
            // Esplora relays bitcoind's reason; "already in block chain" or
            // "txn-already-known" mean we are done.
            if body.contains("already") {
                return txid_of(tx_hex);
            }
            return Err(rpc_err(
                &self.base,
                format!("refused: {}", body.chars().take(200).collect::<String>()),
            ));
        }
        parse_hash(&self.base, &body)
    }
}

fn txid_of(tx_hex: &str) -> Result<Txid, AppError> {
    let tx: Transaction = parse_hex("local", tx_hex)?;
    Ok(tx.compute_txid())
}

// ---------------------------------------------------------- Bitcoin Core

/// Bitcoin Core over JSON-RPC. Needs `-txindex` for transactions that are
/// not ours; `scantxoutset` lists an address's coins.
pub struct BitcoinCore {
    url: String,
    user: String,
    pass: String,
    http: reqwest::Client,
}

impl BitcoinCore {
    /// `spec` is `bitcoind+http://user:pass@host:port` (or `+https`).
    pub fn new(spec: &str) -> Result<BitcoinCore, AppError> {
        let rest = spec
            .strip_prefix("bitcoind+")
            .ok_or_else(|| AppError::InvalidInput("not a bitcoind source".into()))?;
        let mut url =
            // The spec may carry the RPC password: never put it in an error.
            url::Url::parse(rest).map_err(|e| AppError::InvalidInput(format!("a bitcoind source is not a URL: {e}")))?;
        let user = url.username().to_owned();
        let pass = url.password().unwrap_or("").to_owned();
        let _ = url.set_username("");
        let _ = url.set_password(None);
        check_transport(&url)?;
        Ok(BitcoinCore {
            url: url.to_string(),
            user,
            pass,
            http: crate::http::client_builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .map_err(|e| AppError::Other(format!("http client: {e}")))?,
        })
    }

    async fn call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, AppError> {
        let r = self
            .http
            .post(&self.url)
            .basic_auth(&self.user, Some(&self.pass))
            .json(&serde_json::json!({"jsonrpc": "1.0", "id": "namehold", "method": method, "params": params}))
            .send()
            .await
            .map_err(|e| rpc_err(&self.url, e))?;
        let v: serde_json::Value = r.json().await.map_err(|e| rpc_err(&self.url, e))?;
        if !v["error"].is_null() {
            return Err(AppError::NodeRefused {
                message: v["error"]["message"].as_str().unwrap_or("error").to_owned(),
                code: v["error"]["code"].as_i64().unwrap_or(0),
            });
        }
        Ok(v["result"].clone())
    }

    /// `getrawtransaction` with Core's "not found" (-5) as `None`.
    async fn raw_verbose(&self, txid: &Txid) -> Result<Option<serde_json::Value>, AppError> {
        match self
            .call(
                "getrawtransaction",
                serde_json::json!([txid.to_string(), true]),
            )
            .await
        {
            Ok(v) => Ok(Some(v)),
            Err(AppError::NodeRefused { code: -5, .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[async_trait]
impl BtcSource for BitcoinCore {
    fn label(&self) -> String {
        url::Url::parse(&self.url)
            .ok()
            .and_then(|u| u.host_str().map(|h| format!("bitcoind@{h}")))
            .unwrap_or_else(|| "bitcoind".into())
    }

    /// Spec §3.6: without `-txindex` Core answers `getrawtransaction` for
    /// the other party's transaction with -5, as if it did not exist, and
    /// a lock would read as missing. `getindexinfo` (Core 0.21+) says
    /// whether the index exists and has caught up; an older Core has no
    /// such call and is refused with its error.
    async fn ready(&self) -> Result<(), AppError> {
        let v = self
            .call("getindexinfo", serde_json::json!(["txindex"]))
            .await?;
        match v["txindex"]["synced"].as_bool() {
            Some(true) => Ok(()),
            Some(false) => Err(rpc_err(&self.url, "its -txindex is still building")),
            None => Err(rpc_err(
                &self.url,
                "Bitcoin Core runs without -txindex, which the exchange needs",
            )),
        }
    }

    async fn tip(&self) -> Result<(u64, u64), AppError> {
        let v = self
            .call("getblockchaininfo", serde_json::json!([]))
            .await?;
        let (Some(h), Some(mtp)) = (v["blocks"].as_u64(), v["mediantime"].as_u64()) else {
            return Err(rpc_err(
                &self.url,
                "getblockchaininfo without blocks or mediantime",
            ));
        };
        Ok((h, mtp))
    }

    async fn raw_tx(&self, txid: &Txid) -> Result<Option<Transaction>, AppError> {
        let Some(v) = self.raw_verbose(txid).await? else {
            return Ok(None);
        };
        let Some(h) = v["hex"].as_str() else {
            return Err(rpc_err(&self.url, "transaction without hex"));
        };
        parse_hex(&self.url, h).map(Some)
    }

    async fn tx_block(&self, txid: &Txid) -> Result<Option<(u64, BlockHash)>, AppError> {
        let Some(v) = self.raw_verbose(txid).await? else {
            return Ok(None);
        };
        let Some(bh) = v["blockhash"].as_str() else {
            return Ok(None);
        };
        let hash: BlockHash = parse_hash(&self.url, bh)?;
        let hdr = self
            .call("getblockheader", serde_json::json!([bh, true]))
            .await?;
        let Some(height) = hdr["height"].as_u64() else {
            return Err(rpc_err(&self.url, "header without height"));
        };
        Ok(Some((height, hash)))
    }

    async fn merkle(
        &self,
        txid: &Txid,
        block: &BlockHash,
    ) -> Result<(Vec<TxMerkleNode>, u32), AppError> {
        // The block's txids; the branch is computed here and checked against
        // the header's root by btc_spv.
        let b = self
            .call("getblock", serde_json::json!([block.to_string(), 1]))
            .await?;
        let Some(list) = b["tx"].as_array() else {
            return Err(rpc_err(&self.url, "block without tx list"));
        };
        let ids = list
            .iter()
            .map(|v| {
                let s = v
                    .as_str()
                    .ok_or_else(|| rpc_err(&self.url, "a block's txid is not a string"))?;
                parse_hash::<Txid>(&self.url, s)
            })
            .collect::<Result<Vec<_>, _>>()?;
        branch_for(&ids, txid)
            .ok_or_else(|| rpc_err(&self.url, "the transaction is not in that block"))
    }

    async fn header_at(&self, height: u64) -> Result<Header, AppError> {
        let h = self
            .call("getblockhash", serde_json::json!([height]))
            .await?;
        let h = h
            .as_str()
            .ok_or_else(|| rpc_err(&self.url, "getblockhash is not a string"))?;
        let hash: BlockHash = parse_hash(&self.url, h)?;
        self.header(&hash).await
    }

    async fn header(&self, hash: &BlockHash) -> Result<Header, AppError> {
        let h = self
            .call(
                "getblockheader",
                serde_json::json!([hash.to_string(), false]),
            )
            .await?;
        let h = h
            .as_str()
            .ok_or_else(|| rpc_err(&self.url, "getblockheader is not a string"))?;
        parse_hex(&self.url, h)
    }

    async fn outspend(&self, o: &OutPoint) -> Result<Option<Outspend>, AppError> {
        if self.raw_verbose(&o.txid).await?.is_none() {
            return Ok(None);
        }
        let v = self
            .call(
                "gettxout",
                serde_json::json!([o.txid.to_string(), o.vout, true]),
            )
            .await?;
        Ok(Some(if v.is_null() {
            Outspend::Spent { txid: None }
        } else {
            Outspend::Unspent
        }))
    }

    async fn utxos(&self, address: &Address) -> Result<Vec<SourceUtxo>, AppError> {
        let v = self
            .call(
                "scantxoutset",
                serde_json::json!(["start", [format!("addr({address})")]]),
            )
            .await?;
        let Some(list) = v["unspents"].as_array() else {
            return Err(rpc_err(&self.url, "scantxoutset without unspents"));
        };
        list.iter()
            .map(|u| {
                let (Some(txid), Some(vout), Some(amount)) =
                    (u["txid"].as_str(), u["vout"].as_u64(), u["amount"].as_f64())
                else {
                    return Err(rpc_err(&self.url, "malformed unspent"));
                };
                Ok(SourceUtxo {
                    outpoint: OutPoint {
                        txid: parse_hash(&self.url, txid)?,
                        vout: u32::try_from(vout).map_err(|_| rpc_err(&self.url, "bad vout"))?,
                    },
                    // 8-decimal BTC fits an f64 exactly enough to round.
                    value: (amount * 100_000_000.0).round() as u64,
                    height: u["height"].as_u64(),
                })
            })
            .collect()
    }

    async fn fee_rate(&self) -> Result<u64, AppError> {
        // As for Esplora: no estimate is an error, never 1 sat/vB.
        let v = self
            .call("estimatesmartfee", serde_json::json!([2]))
            .await?;
        match v["feerate"].as_f64() {
            // BTC per kvB → sat per vB.
            Some(r) => Ok((r * 100_000.0).ceil().max(1.0) as u64),
            None => Err(rpc_err(&self.url, "estimatesmartfee gave no rate")),
        }
    }

    async fn floor_rate(&self) -> Result<u64, AppError> {
        let v = self.call("getmempoolinfo", serde_json::json!([])).await?;
        // Both are BTC per kvB; a transaction below either is not relayed.
        let floor = ["mempoolminfee", "minrelaytxfee"]
            .iter()
            .filter_map(|k| v[*k].as_f64())
            .fold(None, |m: Option<f64>, r| Some(m.map_or(r, |x| x.max(r))));
        match floor {
            Some(r) if r > 0.0 => Ok((r * 100_000.0).ceil().max(1.0) as u64),
            _ => Err(rpc_err(&self.url, "getmempoolinfo gave no minimum fee")),
        }
    }

    async fn broadcast(&self, tx_hex: &str) -> Result<Txid, AppError> {
        match self
            .call("sendrawtransaction", serde_json::json!([tx_hex]))
            .await
        {
            Ok(v) => {
                let s = v
                    .as_str()
                    .ok_or_else(|| rpc_err(&self.url, "sendrawtransaction is not a string"))?;
                parse_hash(&self.url, s)
            }
            Err(AppError::NodeRefused { message, .. }) if message.contains("already") => {
                txid_of(tx_hex)
            }
            Err(e) => Err(e),
        }
    }
}

/// The Merkle branch of `txid` among a block's `txids`, and its position.
pub fn branch_for(txids: &[Txid], txid: &Txid) -> Option<(Vec<TxMerkleNode>, u32)> {
    let pos = txids.iter().position(|t| t == txid)?;
    let mut level: Vec<[u8; 32]> = txids
        .iter()
        .map(|t| t.to_raw_hash().to_byte_array())
        .collect();
    let mut idx = pos;
    let mut branch = Vec::new();
    while level.len() > 1 {
        if level.len() % 2 == 1 {
            level.push(*level.last().expect("non-empty"));
        }
        branch.push(TxMerkleNode::from_byte_array(level[idx ^ 1]));
        level = level
            .chunks(2)
            .map(|c| {
                let mut b = [0u8; 64];
                b[..32].copy_from_slice(&c[0]);
                b[32..].copy_from_slice(&c[1]);
                bitcoin::hashes::sha256d::Hash::hash(&b).to_byte_array()
            })
            .collect();
        idx /= 2;
    }
    Some((branch, u32::try_from(pos).ok()?))
}

/// A source from its configured spelling.
pub fn source_from_spec(spec: &str) -> Result<Box<dyn BtcSource>, AppError> {
    if spec.starts_with("bitcoind+") {
        Ok(Box::new(BitcoinCore::new(spec)?))
    } else {
        Ok(Box::new(Esplora::new(spec)?))
    }
}

/// The default sources of a network: two independent Esplora operators on
/// mainnet and testnet; none on regtest (the user names a local one).
pub fn default_sources(network: crate::noncustodial::network::Network) -> Vec<String> {
    use crate::noncustodial::network::Network;
    match network {
        Network::Main => vec![
            "https://mempool.space/api".into(),
            "https://blockstream.info/api".into(),
        ],
        Network::Testnet => vec![
            "https://mempool.space/testnet/api".into(),
            "https://blockstream.info/testnet/api".into(),
        ],
        Network::Regtest | Network::Simnet => Vec::new(),
    }
}
