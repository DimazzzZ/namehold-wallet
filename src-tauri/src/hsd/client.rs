use reqwest::Client;
use std::time::Duration;

use super::types::*;
use crate::error::AppError;

pub struct HandshakeClient {
    http: Client,
    wallet_url: String,
    node_url: String,
    api_key: String,
    wallet_id: String,
}

impl HandshakeClient {
    pub fn new(
        wallet_url: &str,
        node_url: &str,
        api_key: &str,
        wallet_id: &str,
    ) -> Self {
        Self {
            http: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("failed to build HTTP client"),
            wallet_url: wallet_url.trim_end_matches('/').to_string(),
            node_url: node_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            wallet_id: wallet_id.to_string(),
        }
    }

    pub fn from_settings(settings: &std::collections::HashMap<String, String>) -> Self {
        Self::new(
            settings
                .get("hsd_wallet_api_url")
                .map(|s| s.as_str())
                .unwrap_or("http://127.0.0.1:12039"),
            settings
                .get("hsd_node_api_url")
                .map(|s| s.as_str())
                .unwrap_or("http://127.0.0.1:12037"),
            settings
                .get("hsd_api_key")
                .map(|s| s.as_str())
                .unwrap_or(""),
            settings
                .get("hsd_wallet_id")
                .map(|s| s.as_str())
                .unwrap_or("primary"),
        )
    }

    fn wallet_get(&self, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}/wallet/{}{}", self.wallet_url, self.wallet_id, path);
        self.http.get(&url).basic_auth("x", Some(&self.api_key))
    }

    fn wallet_post(&self, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}/wallet/{}{}", self.wallet_url, self.wallet_id, path);
        self.http.post(&url).basic_auth("x", Some(&self.api_key))
    }

    fn node_post(&self) -> reqwest::RequestBuilder {
        self.http
            .post(&self.node_url)
            .basic_auth("x", Some(&self.api_key))
    }

    pub async fn check_connection(&self) -> Result<HsdWalletInfo, AppError> {
        let resp = self.wallet_get("").send().await?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!(
                "Wallet returned status {}",
                resp.status()
            )));
        }
        let info = resp.json().await?;
        Ok(info)
    }

    pub async fn get_wallet_info(&self) -> Result<HsdWalletInfo, AppError> {
        let resp = self.wallet_get("").send().await?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!(
                "Wallet returned status {}",
                resp.status()
            )));
        }
        let info = resp.json().await?;
        Ok(info)
    }

    pub async fn get_balance(&self) -> Result<HsdBalance, AppError> {
        let resp = self.wallet_get("/balance").send().await?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!(
                "Wallet returned status {}",
                resp.status()
            )));
        }
        let balance = resp.json().await?;
        Ok(balance)
    }

    pub async fn get_receive_address(&self) -> Result<HsdAddress, AppError> {
        let resp = self
            .wallet_post("/address")
            .json(&serde_json::json!({"account": "default"}))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!(
                "Wallet returned status {}",
                resp.status()
            )));
        }
        let addr = resp.json().await?;
        Ok(addr)
    }

    pub async fn get_names(&self) -> Result<Vec<HsdName>, AppError> {
        let resp = self
            .wallet_get("/name?own=true")
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!(
                "Wallet returned status {}",
                resp.status()
            )));
        }
        let names: Vec<HsdName> = resp.json().await?;
        let verified: Vec<HsdName> = names
            .into_iter()
            .filter(|n| {
                if let Some(ref owner) = n.owner {
                    !owner.hash.chars().all(|c| c == '0') && owner.index != 4294967295
                } else {
                    false
                }
            })
            .collect();
        Ok(verified)
    }

    pub async fn get_name_info(&self, name: &str) -> Result<HsdName, AppError> {
        let resp = self
            .wallet_get(&format!("/name/{}", name))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!(
                "Wallet returned status {} for name {}",
                resp.status(),
                name
            )));
        }
        let info = resp.json().await?;
        Ok(info)
    }

    pub async fn get_resource(&self, name: &str) -> Result<serde_json::Value, AppError> {
        let resp = self
            .wallet_get(&format!("/resource/{}", name))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!(
                "Resource lookup failed for {}: status {}",
                name,
                resp.status()
            )));
        }
        let resource = resp.json().await?;
        Ok(resource)
    }

    pub async fn get_transactions(&self) -> Result<serde_json::Value, AppError> {
        let resp = self.wallet_get("/tx/history?limit=50&reverse=true").send().await?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!(
                "Transactions returned status {}",
                resp.status()
            )));
        }
        let txs = resp.json().await?;
        Ok(txs)
    }

    pub async fn send_to_address(
        &self,
        address: &str,
        value: i64,
        passphrase: &str,
    ) -> Result<serde_json::Value, AppError> {
        let resp = self
            .wallet_post("/send")
            .json(&serde_json::json!({
                "passphrase": passphrase,
                "outputs": [{"address": address, "value": value}]
            }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            return Err(AppError::Other(format!(
                "Send failed: {}",
                body["error"].as_str().unwrap_or("unknown error")
            )));
        }
        let result = resp.json().await?;
        Ok(result)
    }

    pub async fn send_transfer(
        &self,
        name: &str,
        address: &str,
        passphrase: &str,
    ) -> Result<serde_json::Value, AppError> {
        let resp = self
            .wallet_post("/transfer")
            .json(&serde_json::json!({
                "passphrase": passphrase,
                "name": name,
                "address": address,
                "broadcast": true,
                "sign": true
            }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            return Err(AppError::Other(format!(
                "Transfer failed for {}: {}",
                name,
                body["error"].as_str().unwrap_or("unknown error")
            )));
        }
        let result = resp.json().await?;
        Ok(result)
    }

    pub async fn send_renewal(
        &self,
        name: &str,
        passphrase: &str,
    ) -> Result<serde_json::Value, AppError> {
        let resp = self
            .wallet_post("/renewal")
            .json(&serde_json::json!({
                "passphrase": passphrase,
                "name": name,
                "broadcast": true,
                "sign": true
            }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            return Err(AppError::Other(format!(
                "Renewal failed for {}: {}",
                name,
                body["error"].as_str().unwrap_or("unknown error")
            )));
        }
        let result = resp.json().await?;
        Ok(result)
    }

    pub async fn send_finalize(
        &self,
        name: &str,
        passphrase: &str,
    ) -> Result<serde_json::Value, AppError> {
        let resp = self
            .wallet_post("/finalize")
            .json(&serde_json::json!({
                "passphrase": passphrase,
                "name": name,
                "broadcast": true,
                "sign": true
            }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            return Err(AppError::Other(format!(
                "Finalize failed for {}: {}",
                name,
                body["error"].as_str().unwrap_or("unknown error")
            )));
        }
        let result = resp.json().await?;
        Ok(result)
    }

    pub async fn update_records(
        &self,
        name: &str,
        records: serde_json::Value,
        passphrase: &str,
    ) -> Result<serde_json::Value, AppError> {
        let resp = self
            .wallet_post("/update")
            .json(&serde_json::json!({
                "passphrase": passphrase,
                "name": name,
                "data": {"records": records},
                "broadcast": true,
                "sign": true
            }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            return Err(AppError::Other(format!(
                "Update failed for {}: {}",
                name,
                body["error"].as_str().unwrap_or("unknown error")
            )));
        }
        let result = resp.json().await?;
        Ok(result)
    }
}
