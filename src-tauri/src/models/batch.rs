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
