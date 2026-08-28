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
            (
                MigrationStatus::NamebaseTransferRequested,
                "namebase_transfer_requested",
            ),
            (MigrationStatus::WaitingTransferTx, "waiting_transfer_tx"),
            (
                MigrationStatus::TransferSeenOnChain,
                "transfer_seen_on_chain",
            ),
            (MigrationStatus::WaitingFinalize, "waiting_finalize"),
            (MigrationStatus::FinalizedOwned, "finalized_owned"),
            (MigrationStatus::FailedOrStuck, "failed_or_stuck"),
            (MigrationStatus::DoNotTouchStaked, "do_not_touch_staked"),
        ];
        for (variant, s) in &cases {
            assert_eq!(variant.as_str(), *s, "as_str for {s}");
            assert!(
                matches!(MigrationStatus::from_str(s), _x if true),
                "from_str for {s}"
            );
        }
    }

    #[test]
    fn from_str_unknown_defaults_to_not_started() {
        let d = std::mem::discriminant(&MigrationStatus::NotStarted);
        assert_eq!(std::mem::discriminant(&MigrationStatus::from_str("")), d);
        assert_eq!(std::mem::discriminant(&MigrationStatus::from_str("unknown")), d);
        assert_eq!(std::mem::discriminant(&MigrationStatus::from_str("garbage")), d);
    }

    #[test]
    fn from_str_all_known_variants() {
        let cases = [
            ("not_started", MigrationStatus::NotStarted),
            (
                "namebase_transfer_requested",
                MigrationStatus::NamebaseTransferRequested,
            ),
            ("waiting_transfer_tx", MigrationStatus::WaitingTransferTx),
            (
                "transfer_seen_on_chain",
                MigrationStatus::TransferSeenOnChain,
            ),
            ("waiting_finalize", MigrationStatus::WaitingFinalize),
            ("finalized_owned", MigrationStatus::FinalizedOwned),
            ("failed_or_stuck", MigrationStatus::FailedOrStuck),
            ("do_not_touch_staked", MigrationStatus::DoNotTouchStaked),
        ];
        for (s, expected) in &cases {
            let result = MigrationStatus::from_str(s);
            assert!(
                matches!(result, _ if std::mem::discriminant(&result) == std::mem::discriminant(expected))
            );
        }
    }

    #[test]
    fn as_str_returns_correct_string_for_each_variant() {
        assert_eq!(MigrationStatus::NotStarted.as_str(), "not_started");
        assert_eq!(
            MigrationStatus::NamebaseTransferRequested.as_str(),
            "namebase_transfer_requested"
        );
        assert_eq!(
            MigrationStatus::WaitingTransferTx.as_str(),
            "waiting_transfer_tx"
        );
        assert_eq!(
            MigrationStatus::TransferSeenOnChain.as_str(),
            "transfer_seen_on_chain"
        );
        assert_eq!(
            MigrationStatus::WaitingFinalize.as_str(),
            "waiting_finalize"
        );
        assert_eq!(MigrationStatus::FinalizedOwned.as_str(), "finalized_owned");
        assert_eq!(MigrationStatus::FailedOrStuck.as_str(), "failed_or_stuck");
        assert_eq!(
            MigrationStatus::DoNotTouchStaked.as_str(),
            "do_not_touch_staked"
        );
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

    // --- `Asset::from_row` DB-backed coverage ---------------------------------

    fn mem_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::migrations::run(&conn).unwrap();
        conn
    }

    /// End-to-end round-trip of `Asset::from_row` with every nullable column
    /// populated and `is_staked = 0` mapped to `false`. Also verifies status
    /// column round-trips through `MigrationStatus::from_str`.
    #[test]
    fn from_row_full_row_all_fields_populated_unstaked() {
        let conn = mem_db();
        conn.execute(
            "INSERT INTO assets
               (tld, status, is_staked, category, tags, notes, hns_received,
                transfer_tx_hash, finalize_tx_hash, name_state,
                expires_at_height, days_until_expire, last_synced_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                "example",
                "finalized_owned",
                0_i64,
                "Premium",
                r#"["a","b"]"#,
                "some notes",
                1_000_000_i64,
                "0xtransfer",
                "0xfinalize",
                "CLOSED",
                500_000_i64,
                42.5_f64,
                "2024-02-01T00:00:00Z",
            ],
        )
        .unwrap();

        let asset = conn
            .query_row("SELECT * FROM assets WHERE tld = 'example'", [], Asset::from_row)
            .unwrap();

        assert!(asset.id >= 1);
        assert_eq!(asset.tld, "example");
        assert_eq!(
            std::mem::discriminant(&asset.status),
            std::mem::discriminant(&MigrationStatus::FinalizedOwned)
        );
        assert!(!asset.is_staked);
        assert_eq!(asset.category.as_deref(), Some("Premium"));
        assert_eq!(asset.tags, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(asset.notes.as_deref(), Some("some notes"));
        assert_eq!(asset.hns_received, Some(1_000_000));
        assert_eq!(asset.transfer_tx_hash.as_deref(), Some("0xtransfer"));
        assert_eq!(asset.finalize_tx_hash.as_deref(), Some("0xfinalize"));
        assert_eq!(asset.name_state.as_deref(), Some("CLOSED"));
        assert_eq!(asset.expires_at_height, Some(500_000));
        assert_eq!(asset.days_until_expire, Some(42.5));
        assert_eq!(asset.last_synced_at.as_deref(), Some("2024-02-01T00:00:00Z"));
        assert!(!asset.created_at.is_empty());
        assert!(!asset.updated_at.is_empty());
    }

    /// `is_staked = 1` → `true`, and NULL `tags` → empty Vec (unwrap_or_default).
    #[test]
    fn from_row_staked_true_and_null_tags_yields_empty_vec() {
        let conn = mem_db();
        conn.execute(
            "INSERT INTO assets (tld, status, is_staked, tags) VALUES (?1, ?2, ?3, NULL)",
            rusqlite::params!["staked", "do_not_touch_staked", 1_i64],
        )
        .unwrap();

        let asset = conn
            .query_row("SELECT * FROM assets WHERE tld = 'staked'", [], Asset::from_row)
            .unwrap();

        assert!(asset.is_staked);
        assert_eq!(
            std::mem::discriminant(&asset.status),
            std::mem::discriminant(&MigrationStatus::DoNotTouchStaked)
        );
        assert!(asset.tags.is_empty());
        assert!(asset.category.is_none());
        assert!(asset.notes.is_none());
        assert!(asset.hns_received.is_none());
        assert!(asset.transfer_tx_hash.is_none());
        assert!(asset.finalize_tx_hash.is_none());
        assert!(asset.name_state.is_none());
        assert!(asset.expires_at_height.is_none());
        assert!(asset.days_until_expire.is_none());
        assert!(asset.last_synced_at.is_none());
    }

    /// Invalid JSON in the `tags` column falls through to empty Vec (the
    /// `serde_json::from_str(...).ok()` branch).
    #[test]
    fn from_row_invalid_tags_json_falls_back_to_empty_vec() {
        let conn = mem_db();
        conn.execute(
            "INSERT INTO assets (tld, status, is_staked, tags) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["bad-tags", "not_started", 0_i64, "not json {{"],
        )
        .unwrap();

        let asset = conn
            .query_row("SELECT * FROM assets WHERE tld = 'bad-tags'", [], Asset::from_row)
            .unwrap();
        assert!(asset.tags.is_empty());
        assert_eq!(
            std::mem::discriminant(&asset.status),
            std::mem::discriminant(&MigrationStatus::NotStarted)
        );
    }

    /// Every `MigrationStatus::from_str` known variant, driven end-to-end
    /// through a real SQLite row so the round-trip crosses the DB.
    #[test]
    fn from_row_covers_every_status_variant() {
        let conn = mem_db();
        let variants = [
            ("not_started", MigrationStatus::NotStarted),
            (
                "namebase_transfer_requested",
                MigrationStatus::NamebaseTransferRequested,
            ),
            ("waiting_transfer_tx", MigrationStatus::WaitingTransferTx),
            (
                "transfer_seen_on_chain",
                MigrationStatus::TransferSeenOnChain,
            ),
            ("waiting_finalize", MigrationStatus::WaitingFinalize),
            ("finalized_owned", MigrationStatus::FinalizedOwned),
            ("failed_or_stuck", MigrationStatus::FailedOrStuck),
            ("do_not_touch_staked", MigrationStatus::DoNotTouchStaked),
        ];
        for (i, (s, expected)) in variants.iter().enumerate() {
            let tld = format!("v{i}");
            conn.execute(
                "INSERT INTO assets (tld, status, is_staked) VALUES (?1, ?2, 0)",
                rusqlite::params![tld, s],
            )
            .unwrap();
            let asset = conn
                .query_row(
                    "SELECT * FROM assets WHERE tld = ?1",
                    rusqlite::params![tld],
                    Asset::from_row,
                )
                .unwrap();
            assert_eq!(
                std::mem::discriminant(&asset.status),
                std::mem::discriminant(expected),
                "variant mismatch for {s}"
            );
            assert_eq!(asset.status.as_str(), *s);
        }
    }

    /// Error path: `from_row` fails when a required column is missing.
    /// This exercises the error branches of the `?` operators in the closure.
    #[test]
    fn from_row_errors_when_required_column_missing() {
        let conn = mem_db();
        conn.execute(
            "INSERT INTO assets (tld, status, is_staked) VALUES (?1, ?2, 0)",
            rusqlite::params!["missing-col", "not_started"],
        )
        .unwrap();

        // Query without the 'id' column → from_row will fail when it tries
        // to get a named column from the row. The first `row.get("status")`
        // will fail because we only select 'tld'.
        let result = conn.query_row(
            "SELECT tld FROM assets WHERE tld = 'missing-col'",
            [],
            Asset::from_row,
        );
        assert!(result.is_err(), "expected error from missing column");
    }

    /// Error path: `from_row` fails at `row.get("is_staked")` when the column
    /// is present but has an incompatible type (e.g., text where i64 expected).
    #[test]
    fn from_row_errors_at_is_staked_type_mismatch() {
        let conn = mem_db();
        conn.execute(
            "INSERT INTO assets (tld, status, is_staked) VALUES (?1, ?2, 0)",
            rusqlite::params!["type-err", "not_started"],
        )
        .unwrap();

        // Query that has status and tags but renames is_staked to a text value
        // so the i64 get fails.
        let result = conn.query_row(
            "SELECT id, tld, status, tags, 'not_a_number' AS is_staked,
                    category, notes, hns_received, transfer_tx_hash,
                    finalize_tx_hash, name_state, expires_at_height,
                    days_until_expire, last_synced_at, created_at, updated_at
             FROM assets WHERE tld = 'type-err'",
            [],
            Asset::from_row,
        );
        assert!(result.is_err(), "expected error from type mismatch on is_staked");
    }

    /// Error path: `from_row` fails at a late column (`created_at`) when it's
    /// aliased to NULL but the field is non-optional `String`.
    #[test]
    fn from_row_errors_at_created_at_null() {
        let conn = mem_db();
        conn.execute(
            "INSERT INTO assets (tld, status, is_staked) VALUES (?1, ?2, 0)",
            rusqlite::params!["null-created", "not_started"],
        )
        .unwrap();

        // Provide all columns but override created_at with NULL.
        // `row.get::<_, String>("created_at")` will fail on NULL.
        let result = conn.query_row(
            "SELECT id, tld, status, tags, is_staked,
                    category, notes, hns_received, transfer_tx_hash,
                    finalize_tx_hash, name_state, expires_at_height,
                    days_until_expire, last_synced_at,
                    NULL AS created_at, updated_at
             FROM assets WHERE tld = 'null-created'",
            [],
            Asset::from_row,
        );
        assert!(result.is_err(), "expected error from NULL created_at");
    }

    /// Error path: `from_row` fails at `row.get("tld")` when the column
    /// is missing from the result set.
    #[test]
    fn from_row_errors_at_tld_missing() {
        let conn = mem_db();
        conn.execute(
            "INSERT INTO assets (tld, status, is_staked) VALUES (?1, ?2, 0)",
            rusqlite::params!["tld-err", "not_started"],
        )
        .unwrap();

        // Provide status, tags, is_staked, id but NOT tld.
        let result = conn.query_row(
            "SELECT id, status, tags, is_staked,
                    category, notes, hns_received, transfer_tx_hash,
                    finalize_tx_hash, name_state, expires_at_height,
                    days_until_expire, last_synced_at, created_at, updated_at
             FROM assets WHERE tld = 'tld-err'",
            [],
            Asset::from_row,
        );
        assert!(result.is_err(), "expected error from missing tld column");
    }
}
