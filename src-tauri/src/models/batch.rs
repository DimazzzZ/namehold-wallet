use serde::{Deserialize, Serialize};

use super::asset::Asset;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchStatus {
    Planned,
    InProgress,
    Completed,
    Paused,
    Cancelled,
}

impl BatchStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Paused => "paused",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            "paused" => Self::Paused,
            "cancelled" => Self::Cancelled,
            _ => Self::Planned,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Batch {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub status: BatchStatus,
    pub asset_count: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

impl Batch {
    pub fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        let status_str: String = row.get("status")?;
        Ok(Self {
            id: row.get("id")?,
            name: row.get("name")?,
            description: row.get("description")?,
            status: BatchStatus::from_str(&status_str),
            asset_count: row.get("asset_count").ok(),
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchWithAssets {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub status: BatchStatus,
    pub asset_count: Option<i64>,
    pub assets: Vec<Asset>,
    pub created_at: String,
    pub updated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_status_as_str_all_variants() {
        assert_eq!(BatchStatus::Planned.as_str(), "planned");
        assert_eq!(BatchStatus::InProgress.as_str(), "in_progress");
        assert_eq!(BatchStatus::Completed.as_str(), "completed");
        assert_eq!(BatchStatus::Paused.as_str(), "paused");
        assert_eq!(BatchStatus::Cancelled.as_str(), "cancelled");
    }

    #[test]
    fn batch_status_from_str_all_known() {
        let cases = [
            ("planned", BatchStatus::Planned),
            ("in_progress", BatchStatus::InProgress),
            ("completed", BatchStatus::Completed),
            ("paused", BatchStatus::Paused),
            ("cancelled", BatchStatus::Cancelled),
        ];
        for (s, expected) in &cases {
            assert_eq!(
                std::mem::discriminant(&BatchStatus::from_str(s)),
                std::mem::discriminant(expected),
                "unexpected variant for {s}"
            );
        }
    }

    #[test]
    fn batch_status_from_str_unknown_defaults_to_planned() {
        let d = std::mem::discriminant(&BatchStatus::Planned);
        assert_eq!(std::mem::discriminant(&BatchStatus::from_str("")), d);
        assert_eq!(std::mem::discriminant(&BatchStatus::from_str("unknown")), d);
        assert_eq!(std::mem::discriminant(&BatchStatus::from_str("garbage")), d);
    }

    #[test]
    fn batch_serialization_snake_case() {
        let batch = Batch {
            id: 1,
            name: "test-batch".into(),
            description: Some("A test".into()),
            status: BatchStatus::InProgress,
            asset_count: Some(5),
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-02T00:00:00Z".into(),
        };
        let json = serde_json::to_value(&batch).unwrap();
        assert_eq!(json["name"], "test-batch");
        assert_eq!(json["status"], "in_progress");
        assert_eq!(json["asset_count"], serde_json::json!(5));
        assert_eq!(json["created_at"], "2024-01-01T00:00:00Z");
        assert_eq!(json["updated_at"], "2024-01-02T00:00:00Z");
    }

    #[test]
    fn batch_round_trip_via_json() {
        let batch = Batch {
            id: 99,
            name: "round-trip".into(),
            description: None,
            status: BatchStatus::Cancelled,
            asset_count: None,
            created_at: "2024-06-01T00:00:00Z".into(),
            updated_at: "2024-06-02T00:00:00Z".into(),
        };
        let json = serde_json::to_value(&batch).unwrap();
        let back: Batch = serde_json::from_value(json).unwrap();
        assert_eq!(back.id, 99);
        assert_eq!(back.name, "round-trip");
        assert!(back.description.is_none());
        assert_eq!(
            std::mem::discriminant(&back.status),
            std::mem::discriminant(&BatchStatus::Cancelled)
        );
        assert!(back.asset_count.is_none());
    }

    // --- `Batch::from_row` DB-backed coverage ---------------------------------

    fn mem_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::migrations::run(&conn).unwrap();
        conn
    }

    /// `Batch::from_row` when the query includes a `COUNT(...) as asset_count`
    /// alias → `asset_count` is `Some`. Also exercises the `description` and
    /// `status` columns end-to-end.
    #[test]
    fn from_row_with_asset_count_alias_yields_some() {
        let conn = mem_db();
        conn.execute(
            "INSERT INTO batches (name, description, status) VALUES (?1, ?2, ?3)",
            rusqlite::params!["b1", "a description", "in_progress"],
        )
        .unwrap();
        let batch_id: i64 = conn.last_insert_rowid();
        // Seed two assets and link them to the batch so the COUNT is non-zero.
        for tld in ["one", "two"] {
            conn.execute(
                "INSERT INTO assets (tld, status, is_staked) VALUES (?1, 'not_started', 0)",
                rusqlite::params![tld],
            )
            .unwrap();
            let asset_id: i64 = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO batch_assets (batch_id, asset_id) VALUES (?1, ?2)",
                rusqlite::params![batch_id, asset_id],
            )
            .unwrap();
        }

        let batch = conn
            .query_row(
                "SELECT b.*, COUNT(ba.asset_id) AS asset_count
                 FROM batches b
                 LEFT JOIN batch_assets ba ON ba.batch_id = b.id
                 WHERE b.id = ?1
                 GROUP BY b.id",
                rusqlite::params![batch_id],
                Batch::from_row,
            )
            .unwrap();

        assert_eq!(batch.id, batch_id);
        assert_eq!(batch.name, "b1");
        assert_eq!(batch.description.as_deref(), Some("a description"));
        assert_eq!(
            std::mem::discriminant(&batch.status),
            std::mem::discriminant(&BatchStatus::InProgress)
        );
        assert_eq!(batch.asset_count, Some(2));
        assert!(!batch.created_at.is_empty());
        assert!(!batch.updated_at.is_empty());
    }

    /// `Batch::from_row` when the query has no `asset_count` column → the
    /// `row.get("asset_count").ok()` branch yields `None`. Also covers a NULL
    /// `description` and the default `planned` status.
    #[test]
    fn from_row_without_asset_count_alias_yields_none() {
        let conn = mem_db();
        conn.execute(
            "INSERT INTO batches (name) VALUES (?1)",
            rusqlite::params!["plain"],
        )
        .unwrap();
        let batch_id: i64 = conn.last_insert_rowid();

        let batch = conn
            .query_row(
                "SELECT * FROM batches WHERE id = ?1",
                rusqlite::params![batch_id],
                Batch::from_row,
            )
            .unwrap();

        assert_eq!(batch.name, "plain");
        assert!(batch.description.is_none());
        assert_eq!(
            std::mem::discriminant(&batch.status),
            std::mem::discriminant(&BatchStatus::Planned)
        );
        assert!(batch.asset_count.is_none());
    }

    /// Error path: `from_row` fails when a required column is missing.
    /// This exercises the error branches of the `?` operators in the closure.
    #[test]
    fn from_row_errors_when_required_column_missing() {
        let conn = mem_db();
        conn.execute(
            "INSERT INTO batches (name, status) VALUES (?1, ?2)",
            rusqlite::params!["missing-col", "planned"],
        )
        .unwrap();

        // Query without the 'id' column → from_row will fail when it tries
        // to get 'id' from the row.
        let result = conn.query_row(
            "SELECT name, status FROM batches WHERE name = 'missing-col'",
            [],
            Batch::from_row,
        );
        assert!(result.is_err(), "expected error from missing column");
    }
}
