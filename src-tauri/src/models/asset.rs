use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationStatus {
    NotStarted,
    NamebaseTransferRequested,
    WaitingTransferTx,
    TransferSeenOnChain,
    WaitingFinalize,
    FinalizedOwned,
    FailedOrStuck,
    DoNotTouchStaked,
}

impl MigrationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotStarted => "not_started",
            Self::NamebaseTransferRequested => "namebase_transfer_requested",
            Self::WaitingTransferTx => "waiting_transfer_tx",
            Self::TransferSeenOnChain => "transfer_seen_on_chain",
            Self::WaitingFinalize => "waiting_finalize",
            Self::FinalizedOwned => "finalized_owned",
            Self::FailedOrStuck => "failed_or_stuck",
            Self::DoNotTouchStaked => "do_not_touch_staked",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "namebase_transfer_requested" => Self::NamebaseTransferRequested,
            "waiting_transfer_tx" => Self::WaitingTransferTx,
            "transfer_seen_on_chain" => Self::TransferSeenOnChain,
            "waiting_finalize" => Self::WaitingFinalize,
            "finalized_owned" => Self::FinalizedOwned,
            "failed_or_stuck" => Self::FailedOrStuck,
            "do_not_touch_staked" => Self::DoNotTouchStaked,
            _ => Self::NotStarted,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub id: i64,
    pub tld: String,
    pub status: MigrationStatus,
    pub is_staked: bool,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub notes: Option<String>,
    pub hns_received: Option<i64>,
    pub transfer_tx_hash: Option<String>,
    pub finalize_tx_hash: Option<String>,
    pub name_state: Option<String>,
    pub expires_at_height: Option<i64>,
    pub days_until_expire: Option<f64>,
    pub last_synced_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Asset {
    pub fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        let status_str: String = row.get("status")?;
        let tags_str: Option<String> = row.get("tags")?;
        let tags: Vec<String> = tags_str
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
        let is_staked_int: i64 = row.get("is_staked")?;

        Ok(Self {
            id: row.get("id")?,
            tld: row.get("tld")?,
            status: MigrationStatus::from_str(&status_str),
            is_staked: is_staked_int != 0,
            category: row.get("category")?,
            tags,
            notes: row.get("notes")?,
            hns_received: row.get("hns_received")?,
            transfer_tx_hash: row.get("transfer_tx_hash")?,
            finalize_tx_hash: row.get("finalize_tx_hash")?,
            name_state: row.get("name_state")?,
            expires_at_height: row.get("expires_at_height")?,
            days_until_expire: row.get("days_until_expire")?,
            last_synced_at: row.get("last_synced_at")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub imported: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}
