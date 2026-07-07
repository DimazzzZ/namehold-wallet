use reqwest::Client;
use std::time::Duration;

use crate::error::AppError;

/// Normalize a pasted cookie value: if the input looks like a full `Cookie:`
/// header (starts with "cookie:" or contains "=" with whitespace), strip the
/// prefix and trim. Otherwise use the value as-is.
fn normalize_cookie(raw: &str) -> String {
    let trimmed = raw.trim();
    // If it starts with "Cookie:" or "cookie:", strip it.
    if let Some(val) = trimmed.strip_prefix("Cookie:") {
        return val.trim().to_string();
    }
    if let Some(val) = trimmed.strip_prefix("cookie:") {
        return val.trim().to_string();
    }
    trimmed.to_string()
}

#[derive(Clone)]
pub struct NamebaseClient {
    http: Client,
    base_url: String,
    cookie: String,
}

impl NamebaseClient {
    /// Default base URL for the Namebase Sunset API.
    pub fn new(cookie: &str) -> Result<Self, AppError> {
        Self::with_base_url(cookie, "https://sunset.namebase.io")
    }

    /// Construct against an explicit base URL. Used to point the client at a mock
    /// server in tests; production always uses `new` (the real Namebase host).
    pub fn with_base_url(raw_cookie: &str, base_url: &str) -> Result<Self, AppError> {
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| AppError::Other(format!("Failed to create HTTP client: {}", e)))?;
        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            cookie: normalize_cookie(raw_cookie),
        })
    }

    /// Expose the base URL for diagnostic purposes.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Expose the cookie value for diagnostic purposes.
    pub fn cookie_value(&self) -> &str {
        &self.cookie
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.base_url, path);
        self.http
            .get(&url)
            .header("Cookie", &self.cookie)
            .header("User-Agent", "Namehold/0.1.0")
    }

    fn post(&self, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.base_url, path);
        self.http
            .post(&url)
            .header("Cookie", &self.cookie)
            .header("Content-Type", "application/json")
            .header("User-Agent", "Namehold/0.1.0")
    }

    pub async fn check_session(&self) -> Result<bool, AppError> {
        let resp = self.get("/api/account").send().await?;
        Ok(resp.status().is_success())
    }

    pub async fn get_account(&self) -> Result<serde_json::Value, AppError> {
        let resp = self.get("/api/account").send().await?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!("Namebase returned status {}", resp.status())));
        }
        Ok(resp.json().await?)
    }

    pub async fn get_domains(&self) -> Result<serde_json::Value, AppError> {
        let resp = self.get("/api/domains").send().await?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!("Namebase returned status {}", resp.status())));
        }
        Ok(resp.json().await?)
    }

    pub async fn get_staked_domains(&self) -> Result<serde_json::Value, AppError> {
        let resp = self.get("/api/domains/staked").send().await?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!("Namebase returned status {}", resp.status())));
        }
        Ok(resp.json().await?)
    }

    pub async fn get_renewals(&self) -> Result<serde_json::Value, AppError> {
        let resp = self.get("/api/domains/renewals").send().await?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!("Namebase returned status {}", resp.status())));
        }
        Ok(resp.json().await?)
    }

    pub async fn get_withdrawals(&self) -> Result<serde_json::Value, AppError> {
        let resp = self.get("/api/withdrawals").send().await?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!("Namebase returned status {}", resp.status())));
        }
        Ok(resp.json().await?)
    }

    pub async fn get_slds(&self) -> Result<serde_json::Value, AppError> {
        let resp = self.get("/api/slds").send().await?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!("Namebase returned status {}", resp.status())));
        }
        Ok(resp.json().await?)
    }

    pub async fn transfer_domain(&self, name: &str, address: &str) -> Result<(), AppError> {
        let resp = self.post(&format!("/api/domains/{}/withdraw", name))
            .json(&serde_json::json!({"address": address}))
            .send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            let fallback = format!("status {}", status);
            let msg = body["error"].as_str().unwrap_or(&fallback);
            return Err(AppError::Other(format!("Transfer failed for {}: {}", name, msg)));
        }
        Ok(())
    }

    pub async fn withdraw_hns(&self, address: &str, amount: &str) -> Result<(), AppError> {
        let resp = self.post("/api/withdrawals")
            .json(&serde_json::json!({"currency": "hns", "amount": amount, "address": address}))
            .send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            let fallback = format!("status {}", status);
            let msg = body["error"].as_str().unwrap_or(&fallback);
            return Err(AppError::Other(format!("HNS withdrawal failed: {}", msg)));
        }
        Ok(())
    }

    pub async fn get_domain_withdrawals(&self) -> Result<serde_json::Value, AppError> {
        let resp = self.get("/api/domains/withdrawals").send().await?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!("Namebase returned status {}", resp.status())));
        }
        Ok(resp.json().await?)
    }
}
