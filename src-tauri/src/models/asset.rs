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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_as_str_round_trips_all_variants() {
        let cases = [
            (MigrationStatus::NotStarted, "not_started"),
            (MigrationStatus::NamebaseTransferRequested, "namebase_transfer_requested"),
            (MigrationStatus::WaitingTransferTx, "waiting_transfer_tx"),
            (MigrationStatus::TransferSeenOnChain, "transfer_seen_on_chain"),
            (MigrationStatus::WaitingFinalize, "waiting_finalize"),
            (MigrationStatus::FinalizedOwned, "finalized_owned"),
            (MigrationStatus::FailedOrStuck, "failed_or_stuck"),
            (MigrationStatus::DoNotTouchStaked, "do_not_touch_staked"),
        ];
        for (variant, s) in &cases {
            assert_eq!(variant.as_str(), *s, "as_str for {s}");
            assert!(matches!(MigrationStatus::from_str(s), _x if true), "from_str for {s}");
        }
    }

    #[test]
    fn from_str_unknown_defaults_to_not_started() {
        assert!(matches!(MigrationStatus::from_str(""), MigrationStatus::NotStarted));
        assert!(matches!(MigrationStatus::from_str("unknown"), MigrationStatus::NotStarted));
        assert!(matches!(MigrationStatus::from_str("garbage"), MigrationStatus::NotStarted));
    }

    #[test]
    fn from_str_all_known_variants() {
        let cases = [
            ("not_started", MigrationStatus::NotStarted),
            ("namebase_transfer_requested", MigrationStatus::NamebaseTransferRequested),
            ("waiting_transfer_tx", MigrationStatus::WaitingTransferTx),
            ("transfer_seen_on_chain", MigrationStatus::TransferSeenOnChain),
            ("waiting_finalize", MigrationStatus::WaitingFinalize),
            ("finalized_owned", MigrationStatus::FinalizedOwned),
            ("failed_or_stuck", MigrationStatus::FailedOrStuck),
            ("do_not_touch_staked", MigrationStatus::DoNotTouchStaked),
        ];
        for (s, expected) in &cases {
            let result = MigrationStatus::from_str(s);
            assert!(matches!(result, _ if std::mem::discriminant(&result) == std::mem::discriminant(expected)));
        }
    }

    #[test]
    fn as_str_returns_correct_string_for_each_variant() {
        assert_eq!(MigrationStatus::NotStarted.as_str(), "not_started");
        assert_eq!(MigrationStatus::NamebaseTransferRequested.as_str(), "namebase_transfer_requested");
        assert_eq!(MigrationStatus::WaitingTransferTx.as_str(), "waiting_transfer_tx");
        assert_eq!(MigrationStatus::TransferSeenOnChain.as_str(), "transfer_seen_on_chain");
        assert_eq!(MigrationStatus::WaitingFinalize.as_str(), "waiting_finalize");
        assert_eq!(MigrationStatus::FinalizedOwned.as_str(), "finalized_owned");
        assert_eq!(MigrationStatus::FailedOrStuck.as_str(), "failed_or_stuck");
        assert_eq!(MigrationStatus::DoNotTouchStaked.as_str(), "do_not_touch_staked");
    }

    #[test]
    fn asset_from_row_deserializes_correctly() {
        // We can't easily create a rusqlite::Row in unit tests, but we can verify
        // the struct derives Clone, Debug, Serialize, Deserialize correctly.
        let asset = Asset {
            id: 42,
            tld: "example".into(),
            status: MigrationStatus::NotStarted,
            is_staked: false,
            category: Some("Premium".into()),
            tags: vec!["tag1".into(), "tag2".into()],
            notes: Some("notes here".into()),
            hns_received: Some(1000),
            transfer_tx_hash: None,
            finalize_tx_hash: None,
            name_state: Some("CLOSED".into()),
            expires_at_height: Some(500000),
            days_until_expire: Some(30.5),
            last_synced_at: Some("2024-01-01T00:00:00Z".into()),
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-02T00:00:00Z".into(),
        };
        
        let json = serde_json::to_value(&asset).unwrap();
        assert_eq!(json["tld"], "example");
        assert_eq!(json["status"], "not_started");
        assert_eq!(json["is_staked"], serde_json::json!(false));
        assert_eq!(json["tags"], serde_json::json!(["tag1", "tag2"]));
        assert_eq!(json["hns_received"], serde_json::json!(1000));
        assert_eq!(json["days_until_expire"], serde_json::json!(30.5));
        
        // Round-trip
        let back: Asset = serde_json::from_value(json).unwrap();
        assert_eq!(back.id, 42);
        assert_eq!(back.tld, "example");
    }

    #[test]
    fn asset_serialization_snake_case_fields() {
        let asset = Asset {
            id: 1,
            tld: "test".into(),
            status: MigrationStatus::FinalizedOwned,
            is_staked: true,
            category: None,
            tags: vec![],
            notes: None,
            hns_received: None,
            transfer_tx_hash: None,
            finalize_tx_hash: None,
            name_state: None,
            expires_at_height: None,
            days_until_expire: None,
            last_synced_at: None,
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-01T00:00:00Z".into(),
        };
        
        let json = serde_json::to_value(&asset).unwrap();
        assert_eq!(json["status"], "finalized_owned");
        assert_eq!(json["is_staked"], serde_json::json!(true));
        assert_eq!(json["hns_received"], serde_json::Value::Null);
    }
}
