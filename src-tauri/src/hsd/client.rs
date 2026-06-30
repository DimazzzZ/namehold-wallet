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
    pub fn new(wallet_url: &str, node_url: &str, api_key: &str, wallet_id: &str) -> Self {
        Self {
            http: Client::builder()
                .timeout(Duration::from_secs(30))
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
            settings.get("hsd_wallet_api_url").map(|s| s.as_str()).unwrap_or("http://127.0.0.1:12039"),
            settings.get("hsd_node_api_url").map(|s| s.as_str()).unwrap_or("http://127.0.0.1:12037"),
            settings.get("hsd_api_key").map(|s| s.as_str()).unwrap_or(""),
            settings.get("hsd_wallet_id").map(|s| s.as_str()).unwrap_or("primary"),
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
        self.http.post(&self.node_url).basic_auth("x", Some(&self.api_key))
    }

    pub async fn check_connection(&self) -> Result<HsdWalletInfo, AppError> {
        let resp = self.wallet_get("").send().await?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!("Wallet returned status {}", resp.status())));
        }
        Ok(resp.json().await?)
    }

    pub async fn stop_node(&self) -> Result<(), AppError> {
        let resp = self
            .node_post()
            .json(&serde_json::json!({"method": "stop", "params": []}))
            .send()
            .await?;
        // hsd returns "Stopping." even on success
        Ok(())
    }

    pub async fn get_blockchain_info(&self) -> Result<serde_json::Value, AppError> {
        let resp = self
            .node_post()
            .json(&serde_json::json!({"method": "getblockchaininfo", "params": []}))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!("Node returned status {}", resp.status())));
        }
        let body: serde_json::Value = resp.json().await?;
        Ok(body.get("result").cloned().unwrap_or(body))
    }

    pub async fn list_wallets(&self) -> Result<Vec<String>, AppError> {
        let url = format!("{}/wallet", self.wallet_url);
        let resp = self.http.get(&url).basic_auth("x", Some(&self.api_key)).send().await?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!("Wallet returned status {}", resp.status())));
        }
        Ok(resp.json().await?)
    }

    pub fn wallet_url_for_master(&self) -> &str { &self.wallet_url }
    pub fn wallet_id_for_master(&self) -> &str { &self.wallet_id }

    pub async fn http_get_master(&self, url: &str) -> Result<serde_json::Value, AppError> {
        let resp = self.http.get(url).basic_auth("x", Some(&self.api_key)).send().await?;
        if !resp.status().is_success() {
            return Err(AppError::Other(format!("Wallet returned status {}", resp.status())));
        }
        Ok(resp.json().await?)
    }

    pub async fn create_wallet(&self, id: &str, passphrase: &str, mnemonic: Option<&str>) -> Result<serde_json::Value, AppError> {
        let url = format!("{}/wallet/{}", self.wallet_url, id);
        let mut body = serde_json::json!({"passphrase": passphrase, "watchOnly": false});
        if let Some(m) = mnemonic {
            if !m.trim().is_empty() {
                body["mnemonic"] = serde_json::Value::String(m.trim().to_string());
            }
        }
        let resp = self.http.put(&url).basic_auth("x", Some(&self.api_key)).json(&body).send().await?;
        if !resp.status().is_success() {
            let err: serde_json::Value = resp.json().await.unwrap_or_default();
            let msg = err["error"]["message"].as_str().unwrap_or("Unknown error");
            return Err(AppError::Other(format!("Create wallet failed: {}", msg)));
        }
        Ok(resp.json().await?)
    }

    pub async fn delete_wallet(&self, id: &str) -> Result<(), AppError> {
        let url = format!("{}/wallet/{}", self.wallet_url, id);
        let resp = self.http.delete(&url).basic_auth("x", Some(&self.api_key)).send().await?;
        if !resp.status().is_success() {
            let err: serde_json::Value = resp.json().await.unwrap_or_default();
            let msg = err["error"]["message"].as_str().unwrap_or("Unknown error");
            return Err(AppError::Other(format!("Delete wallet failed: {}", msg)));
        }
        Ok(())
    }

    pub async fn get_wallet_info(&self) -> Result<HsdWalletInfo, AppError> {
        let resp = self.wallet_get("").send().await?;
        if !resp.status().is_success() { return Err(AppError::Other(format!("Wallet returned status {}", resp.status()))); }
        Ok(resp.json().await?)
    }

    pub async fn get_balance(&self) -> Result<HsdBalance, AppError> {
        let resp = self.wallet_get("/balance").send().await?;
        if !resp.status().is_success() { return Err(AppError::Other(format!("Wallet returned status {}", resp.status()))); }
        Ok(resp.json().await?)
    }

    pub async fn get_receive_address(&self) -> Result<String, AppError> {
        let resp = self.wallet_get("/account/default").send().await?;
        if !resp.status().is_success() { return Err(AppError::Other(format!("Wallet returned status {}", resp.status()))); }
        let account: serde_json::Value = resp.json().await?;
        let address = account["receiveAddress"].as_str().unwrap_or("").to_string();
        if address.is_empty() { return Err(AppError::Other("No receive address found".to_string())); }
        Ok(address)
    }

    pub async fn get_names(&self) -> Result<Vec<HsdName>, AppError> {
        let resp = self.wallet_get("/name?own=true").send().await?;
        if !resp.status().is_success() { return Err(AppError::Other(format!("Wallet returned status {}", resp.status()))); }
        let names: Vec<HsdName> = resp.json().await?;
        Ok(names.into_iter().filter(|n| {
            if let Some(ref owner) = n.owner { !owner.hash.chars().all(|c| c == '0') && owner.index != 4294967295 } else { false }
        }).collect())
    }

    pub async fn get_name_info(&self, name: &str) -> Result<HsdName, AppError> {
        let resp = self.wallet_get(&format!("/name/{}", name)).send().await?;
        if !resp.status().is_success() { return Err(AppError::Other(format!("Wallet returned status {} for name {}", resp.status(), name))); }
        Ok(resp.json().await?)
    }

    pub async fn get_resource(&self, name: &str) -> Result<serde_json::Value, AppError> {
        let resp = self.wallet_get(&format!("/resource/{}", name)).send().await?;
        if !resp.status().is_success() { return Err(AppError::Other(format!("Resource lookup failed for {}: status {}", name, resp.status()))); }
        Ok(resp.json().await?)
    }

    pub async fn get_transactions(&self) -> Result<serde_json::Value, AppError> {
        let resp = self.wallet_get("/tx/history?limit=50&reverse=true").send().await?;
        if !resp.status().is_success() { return Err(AppError::Other(format!("Transactions returned status {}", resp.status()))); }
        Ok(resp.json().await?)
    }

    pub async fn send_to_address(&self, address: &str, value: i64, passphrase: &str) -> Result<serde_json::Value, AppError> {
        let resp = self.wallet_post("/send").json(&serde_json::json!({"passphrase": passphrase, "outputs": [{"address": address, "value": value}]})).send().await?;
        if !resp.status().is_success() {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            return Err(AppError::Other(format!("Send failed: {}", body["error"].as_str().unwrap_or("unknown error"))));
        }
        Ok(resp.json().await?)
    }

    pub async fn send_transfer(&self, name: &str, address: &str, passphrase: &str) -> Result<serde_json::Value, AppError> {
        let resp = self.wallet_post("/transfer").json(&serde_json::json!({"passphrase": passphrase, "name": name, "address": address, "broadcast": true, "sign": true})).send().await?;
        if !resp.status().is_success() {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            return Err(AppError::Other(format!("Transfer failed for {}: {}", name, body["error"].as_str().unwrap_or("unknown error"))));
        }
        Ok(resp.json().await?)
    }

    pub async fn send_renewal(&self, name: &str, passphrase: &str) -> Result<serde_json::Value, AppError> {
        let resp = self.wallet_post("/renewal").json(&serde_json::json!({"passphrase": passphrase, "name": name, "broadcast": true, "sign": true})).send().await?;
        if !resp.status().is_success() {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            return Err(AppError::Other(format!("Renewal failed for {}: {}", name, body["error"].as_str().unwrap_or("unknown error"))));
        }
        Ok(resp.json().await?)
    }

    pub async fn send_finalize(&self, name: &str, passphrase: &str) -> Result<serde_json::Value, AppError> {
        let resp = self.wallet_post("/finalize").json(&serde_json::json!({"passphrase": passphrase, "name": name, "broadcast": true, "sign": true})).send().await?;
        if !resp.status().is_success() {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            return Err(AppError::Other(format!("Finalize failed for {}: {}", name, body["error"].as_str().unwrap_or("unknown error"))));
        }
        Ok(resp.json().await?)
    }

    pub async fn update_records(&self, name: &str, records: serde_json::Value, passphrase: &str) -> Result<serde_json::Value, AppError> {
        let resp = self.wallet_post("/update").json(&serde_json::json!({"passphrase": passphrase, "name": name, "data": {"records": records}, "broadcast": true, "sign": true})).send().await?;
        if !resp.status().is_success() {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            return Err(AppError::Other(format!("Update failed for {}: {}", name, body["error"].as_str().unwrap_or("unknown error"))));
        }
        Ok(resp.json().await?)
    }
}
