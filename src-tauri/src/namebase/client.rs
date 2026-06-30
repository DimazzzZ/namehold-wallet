use reqwest::Client;
use std::time::Duration;

use crate::error::AppError;

#[derive(Clone)]
pub struct NamebaseClient {
    http: Client,
    base_url: String,
    cookie: String,
}

impl NamebaseClient {
    pub fn new(cookie: &str) -> Result<Self, AppError> {
        Self::with_base_url(cookie, "https://www.namebase.io")
    }

    /// Construct against an explicit base URL. Used to point the client at a mock
    /// server in tests; production always uses `new` (the real Namebase host).
    pub fn with_base_url(cookie: &str, base_url: &str) -> Result<Self, AppError> {
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| AppError::Other(format!("Failed to create HTTP client: {}", e)))?;
        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            cookie: cookie.to_string(),
        })
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
