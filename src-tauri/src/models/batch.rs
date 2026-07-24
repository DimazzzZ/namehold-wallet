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
        assert!(matches!(
            BatchStatus::from_str("planned"),
            BatchStatus::Planned
        ));
        assert!(matches!(
            BatchStatus::from_str("in_progress"),
            BatchStatus::InProgress
        ));
        assert!(matches!(
            BatchStatus::from_str("completed"),
            BatchStatus::Completed
        ));
        assert!(matches!(
            BatchStatus::from_str("paused"),
            BatchStatus::Paused
        ));
        assert!(matches!(
            BatchStatus::from_str("cancelled"),
            BatchStatus::Cancelled
        ));
    }

    #[test]
    fn batch_status_from_str_unknown_defaults_to_planned() {
        assert!(matches!(BatchStatus::from_str(""), BatchStatus::Planned));
        assert!(matches!(
            BatchStatus::from_str("unknown"),
            BatchStatus::Planned
        ));
        assert!(matches!(
            BatchStatus::from_str("garbage"),
            BatchStatus::Planned
        ));
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
        assert!(matches!(back.status, BatchStatus::Cancelled));
        assert!(back.asset_count.is_none());
    }
}
