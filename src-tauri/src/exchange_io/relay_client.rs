//! The relay and watchtower client (spec §8). The relay is a convenience:
//! every offer and message it returns is verified here
//! (`relay_msg::SignedOffer::parse`, `Envelope::parse_for`) and dropped when
//! it fails; nothing it says moves a coin.

use crate::error::AppError;
use crate::noncustodial::exchange::engine::{SentLog, Status};
use crate::noncustodial::exchange::relay_msg::{Envelope, SignedOffer};
use crate::noncustodial::exchange::watch::WatchJob;

pub struct RelayClient {
    base: String,
    http: reqwest::Client,
}

/// There is no default relay: the project runs none yet, and a URL in the
/// code would be a promise. The user names one (or runs `namehold-relay`).
pub fn default_relay(_network: crate::noncustodial::network::Network) -> Option<&'static str> {
    None
}

impl RelayClient {
    pub fn new(base: &str) -> Result<RelayClient, AppError> {
        let url = url::Url::parse(base)
            .map_err(|e| AppError::InvalidInput(format!("'{base}' is not a URL: {e}")))?;
        crate::exchange_io::btc_source::check_transport(&url).map_err(|_| {
            AppError::InvalidInput(format!(
                "the relay must be https, or http to this machine: {base}"
            ))
        })?;
        Ok(RelayClient {
            base: base.trim_end_matches('/').to_owned(),
            http: crate::http::client_builder()
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .map_err(|e| AppError::Other(format!("http client: {e}")))?,
        })
    }

    fn err(&self, m: impl std::fmt::Display) -> AppError {
        AppError::Rpc(format!("relay {}: {m}", self.base))
    }

    async fn post(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, AppError> {
        let r = self
            .http
            .post(format!("{}{}", self.base, path))
            .json(body)
            .send()
            .await
            .map_err(|e| self.err(e))?;
        let status = r.status();
        let v: serde_json::Value = r.json().await.unwrap_or(serde_json::Value::Null);
        if !status.is_success() {
            let why = v["error"].as_str().unwrap_or("refused").to_owned();
            return Err(self.err(format!("{path}: {status}: {why}")));
        }
        Ok(v)
    }

    async fn get(&self, path: &str) -> Result<serde_json::Value, AppError> {
        let r = self
            .http
            .get(format!("{}{}", self.base, path))
            .send()
            .await
            .map_err(|e| self.err(e))?;
        if !r.status().is_success() {
            return Err(self.err(format!("{path}: HTTP {}", r.status())));
        }
        r.json().await.map_err(|e| self.err(e))
    }

    /// Publish, or renew (the heartbeat), an offer.
    pub async fn publish_offer(&self, o: &SignedOffer) -> Result<(), AppError> {
        self.post(
            "/v1/offers",
            &serde_json::to_value(o).expect("offers serialize"),
        )
        .await?;
        Ok(())
    }

    pub async fn withdraw_offer(&self, id: &str, signature: &str) -> Result<(), AppError> {
        self.post(
            &format!("/v1/offers/{id}/withdraw"),
            &serde_json::json!({ "signature": signature }),
        )
        .await?;
        Ok(())
    }

    /// Live offers of `network`, each verified; the ones that fail are
    /// dropped and counted.
    pub async fn offers(
        &self,
        network: &str,
        now: u64,
    ) -> Result<(Vec<SignedOffer>, usize), AppError> {
        let v = self.get(&format!("/v1/offers?network={network}")).await?;
        let Some(list) = v["offers"].as_array() else {
            return Err(self.err("offers is not a list"));
        };
        let mut good = Vec::new();
        let mut dropped = 0;
        for o in list {
            match SignedOffer::parse(&o.to_string()) {
                Ok(o) if o.offer.network == network && o.offer.check_shape(now).is_ok() => {
                    good.push(o)
                }
                _ => dropped += 1,
            }
        }
        Ok((good, dropped))
    }

    pub async fn send(&self, e: &Envelope) -> Result<(), AppError> {
        self.post(
            "/v1/messages",
            &serde_json::to_value(e).expect("messages serialize"),
        )
        .await?;
        Ok(())
    }

    /// Messages to `to` after `cursor`, each verified for `to` and
    /// `network`; the ones that fail are dropped. Returns the new cursor.
    pub async fn inbox(
        &self,
        to: &str,
        network: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<Envelope>, Option<String>), AppError> {
        let mut path = format!("/v1/messages?to={to}");
        if let Some(c) = cursor {
            path.push_str(&format!("&after={c}"));
        }
        let v = self.get(&path).await?;
        let Some(list) = v["messages"].as_array() else {
            return Err(self.err("messages is not a list"));
        };
        let msgs = list
            .iter()
            .filter_map(|m| Envelope::parse_for(&m.to_string(), to, network).ok())
            .collect();
        let next = v["cursor"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| cursor.map(str::to_owned));
        Ok((msgs, next))
    }

    pub async fn watch(&self, job: &WatchJob) -> Result<String, AppError> {
        let v = self
            .post(
                "/v1/watch",
                &serde_json::to_value(job).expect("jobs serialize"),
            )
            .await?;
        v["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| self.err("watch without id"))
    }

    pub async fn watch_status(&self, id: &str) -> Result<(Status, SentLog), AppError> {
        let v = self.get(&format!("/v1/watch/{id}")).await?;
        let status = serde_json::from_value(v["status"].clone()).map_err(|e| self.err(e))?;
        let sent = serde_json::from_value(v["sent"].clone()).map_err(|e| self.err(e))?;
        Ok((status, sent))
    }
}
