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
    pub wid: Option<String>,
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
