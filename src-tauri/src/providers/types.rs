use serde::{Deserialize, Serialize};

/// User-selected connection mode (mirrors the `connection_mode` setting).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionMode {
    /// Local managed hsd: full read+write, app manages lifecycle.
    LocalManagedHsd,
    /// User-controlled remote hsd: read+write only after explicit trust.
    RemoteHsd,
    /// Prefer local/remote hsd, fall back to external read-only on failure.
    AutoFallback,
    /// External read-only explorer is the sole source. No writes.
    ExternalReadOnly,
}

impl ConnectionMode {
    pub fn from_setting(value: &str) -> Self {
        match value {
            "remote_hsd" => ConnectionMode::RemoteHsd,
            "auto_fallback" => ConnectionMode::AutoFallback,
            "external_read_only" => ConnectionMode::ExternalReadOnly,
            _ => ConnectionMode::LocalManagedHsd,
        }
    }
}

/// Which concrete backend is actively serving reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadProviderKind {
    LocalHsd,
    RemoteHsd,
    ExternalHnsfans,
}

/// Which external read provider is configured (mirrors `external_read_provider`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExternalReadProvider {
    None,
    Hnsfans,
}

impl ExternalReadProvider {
    pub fn from_setting(value: &str) -> Self {
        match value {
            "hnsfans" => ExternalReadProvider::Hnsfans,
            _ => ExternalReadProvider::None,
        }
    }
}

/// Status of the active read provider, serialized to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub kind: ReadProviderKind,
    pub label: String,
    pub healthy: bool,
    pub write_capable: bool,
    /// Whether NodeControl may start/stop this backend.
    pub manageable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_height: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_progress: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub syncing: Option<bool>,
}

/// Resolved context for the active read path, serialized to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadContext {
    pub connection_mode: String,
    pub active_read_provider: ProviderStatus,
    pub fallback_active: bool,
    pub local_node_healthy: bool,
    pub wallet_available: bool,
    pub write_allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_reason: Option<String>,
}

impl ConnectionMode {
    pub fn as_setting(&self) -> &'static str {
        match self {
            ConnectionMode::LocalManagedHsd => "local_managed_hsd",
            ConnectionMode::RemoteHsd => "remote_hsd",
            ConnectionMode::AutoFallback => "auto_fallback",
            ConnectionMode::ExternalReadOnly => "external_read_only",
        }
    }
}
