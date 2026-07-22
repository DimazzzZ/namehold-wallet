use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HsdNodeInfo {
    pub version: Option<String>,
    pub chain: Option<serde_json::Value>,
    pub network: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HsdWalletInfo {
    #[serde(default)]
    pub wid: Option<serde_json::Value>,
    pub id: Option<String>,
    pub network: Option<String>,
    pub account_depth: Option<u64>,
    pub token: Option<String>,
    pub watch_only: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HsdBalance {
    pub confirmed: i64,
    pub unconfirmed: i64,
    pub locked_unconfirmed: Option<i64>,
    pub locked_confirmed: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HsdName {
    pub name: String,
    pub name_hash: Option<String>,
    pub state: Option<String>,
    pub height: Option<u64>,
    pub renewal: Option<u64>,
    pub owner: Option<HsdOwner>,
    pub value: Option<u64>,
    pub highest: Option<u64>,
    pub registered: Option<bool>,
    pub expired: Option<bool>,
    pub stats: Option<HsdNameStats>,
    pub transfer: Option<serde_json::Value>,
    pub revoked: Option<bool>,
    /// Per-bid detail for this name, sourced only from the HNSFans explorer
    /// (`getnameinfo` on the node returns only aggregates — no per-bid array).
    /// `None` when the source didn't supply a `bids` array at all (e.g. a
    /// node-sourced `HsdName`); `Some(vec![])` when it did but there are no
    /// bids yet. Always the LAST field — see `read_name_bids` (Task 1).
    pub bids: Option<Vec<HsdBid>>,
}

/// A single bid on a name, as reported by the HNSFans explorer's `bids` array.
/// Every field is optional and defensive — a bid without a `txid` (not yet
/// broadcast/indexed) is still kept so it counts toward totals; it simply
/// can't be matched to a local commitment (see `merge_name_bids`).
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HsdBid {
    pub txid: Option<String>,
    pub index: Option<u32>,
    pub lockup: Option<u64>,
    pub value: Option<u64>,
    pub revealed: Option<bool>,
    pub win: Option<bool>,
    pub reveal: Option<serde_json::Value>,
    pub time: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HsdOwner {
    pub hash: String,
    pub index: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HsdNameStats {
    pub renewal_period_start: Option<u64>,
    pub renewal_period_end: Option<u64>,
    pub blocks_until_expire: Option<i64>,
    pub days_until_expire: Option<f64>,
    // Auction-phase fields (present only in the relevant phase of a
    // `getnameinfo` response; all optional so a name in any state parses).
    pub open_period_start: Option<u64>,
    pub open_period_end: Option<u64>,
    pub bid_period_start: Option<u64>,
    pub bid_period_end: Option<u64>,
    pub reveal_period_start: Option<u64>,
    pub reveal_period_end: Option<u64>,
    pub blocks_until_open: Option<i64>,
    pub blocks_until_bidding: Option<i64>,
    pub blocks_until_reveal: Option<i64>,
    pub blocks_until_close: Option<i64>,
    pub hours_until_open: Option<f64>,
    pub hours_until_bidding: Option<f64>,
    pub hours_until_reveal: Option<f64>,
    pub hours_until_close: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HsdAddress {
    pub name: Option<String>,
    pub account: Option<u64>,
    pub branch: Option<u64>,
    pub index: Option<u64>,
    pub public_key: Option<String>,
    pub address: String,
}
