use reqwest::header::{CONTENT_TYPE, SET_COOKIE};
use reqwest::Client;
use std::sync::Mutex;
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

/// One `Cookie:`-header segment. Most entries are `name=value`, but some of our
/// own test fixtures (and possibly a user's raw paste) are bare opaque tokens
/// with no `=` — keeping the distinction lets us re-serialize an untouched
/// cookie string byte-for-byte instead of appending a spurious `=`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct CookiePair {
    name: String,
    value: Option<String>,
}

impl CookiePair {
    fn to_segment(&self) -> String {
        match &self.value {
            Some(v) => format!("{}={}", self.name, v),
            None => self.name.clone(),
        }
    }
}

/// Parse a `Cookie:`-header-shaped string ("a=1; b=2") into an ordered list of
/// name/value pairs.
fn parse_cookie_pairs(raw: &str) -> Vec<CookiePair> {
    raw.split(';')
        .filter_map(|segment| {
            let segment = segment.trim();
            if segment.is_empty() {
                return None;
            }
            match segment.split_once('=') {
                Some((name, value)) => Some(CookiePair {
                    name: name.trim().to_string(),
                    value: Some(value.trim().to_string()),
                }),
                None => Some(CookiePair {
                    name: segment.to_string(),
                    value: None,
                }),
            }
        })
        .collect()
}

fn cookie_pairs_to_string(pairs: &[CookiePair]) -> String {
    pairs
        .iter()
        .map(CookiePair::to_segment)
        .collect::<Vec<_>>()
        .join("; ")
}

/// True if a `Set-Cookie` attribute list (everything after the first `;`)
/// carries `Max-Age=0`/negative or an `Expires` date in the past — i.e. the
/// server is asking us to drop this cookie. Deliberately naive: an
/// unparseable `Expires` value is treated as "not expired" (keep the cookie)
/// rather than risking a false-positive drop of a still-valid session.
fn set_cookie_is_expired(attrs: &str) -> bool {
    for attr in attrs.split(';') {
        let attr = attr.trim();
        if let Some(v) = attr
            .strip_prefix("Max-Age=")
            .or_else(|| attr.strip_prefix("max-age="))
        {
            if let Ok(n) = v.trim().parse::<i64>() {
                if n <= 0 {
                    return true;
                }
            }
        } else if let Some(v) = attr
            .strip_prefix("Expires=")
            .or_else(|| attr.strip_prefix("expires="))
        {
            if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(v.trim(), "%a, %d %b %Y %H:%M:%S GMT") {
                if dt <= chrono::Utc::now().naive_utc() {
                    return true;
                }
            }
        }
    }
    false
}

/// Apply one `Set-Cookie` response-header value to the ordered cookie jar:
/// replace an existing name in place, append a new one, or delete on expiry.
fn apply_set_cookie(jar: &mut Vec<CookiePair>, raw: &str) {
    let mut parts = raw.splitn(2, ';');
    let first = match parts.next() {
        Some(s) => s.trim(),
        None => return,
    };
    let Some((name, value)) = first.split_once('=') else {
        return;
    };
    let name = name.trim().to_string();
    let value = value.trim().to_string();
    let rest = parts.next().unwrap_or("");

    if set_cookie_is_expired(rest) {
        jar.retain(|p| p.name != name);
        return;
    }
    match jar.iter_mut().find(|p| p.name == name) {
        Some(existing) => existing.value = Some(value),
        None => jar.push(CookiePair { name, value: Some(value) }),
    }
}

/// True if a successful-status response looks like Namebase's HTML login page
/// rather than API JSON — their session cookies can go stale without the API
/// actually returning 401.
fn looks_like_session_expired(content_type: Option<&str>, body: &str) -> bool {
    let looks_html = body.trim_start().starts_with('<');
    let ct_not_json = content_type.map(|ct| !ct.contains("json")).unwrap_or(false);
    looks_html || ct_not_json
}

pub struct NamebaseClient {
    http: Client,
    base_url: String,
    /// Ordered cookie jar, seeded from the pasted cookie string and updated in
    /// place from every response's `Set-Cookie` headers. `&self`-only methods
    /// (no `&mut self`) need interior mutability here since the client is
    /// shared across the several sequential calls a single command can make.
    cookie: Mutex<Vec<CookiePair>>,
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
            cookie: Mutex::new(parse_cookie_pairs(&normalize_cookie(raw_cookie))),
        })
    }

    /// Expose the base URL for diagnostic purposes.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The current `Cookie:`-header value, including any replacements/deletions
    /// picked up from `Set-Cookie` response headers since construction. Used by
    /// the command layer to detect and persist a server-side cookie rotation.
    pub fn current_cookie(&self) -> String {
        let jar = self.lock_jar();
        cookie_pairs_to_string(&jar)
    }

    fn lock_jar(&self) -> std::sync::MutexGuard<'_, Vec<CookiePair>> {
        // Recover from a poisoned mutex instead of panicking: a panic elsewhere
        // while holding the lock shouldn't take the whole cookie jar down with
        // it, and the jar's contents are still perfectly valid to read/write.
        self.cookie.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Merge every `Set-Cookie` header on a response into the jar.
    fn capture_set_cookie(&self, headers: &reqwest::header::HeaderMap) {
        let mut jar = self.lock_jar();
        for value in headers.get_all(SET_COOKIE).iter() {
            if let Ok(s) = value.to_str() {
                apply_set_cookie(&mut jar, s);
            }
        }
    }

    async fn send_get(&self, path: &str) -> Result<reqwest::Response, AppError> {
        let url = format!("{}{}", self.base_url, path);
        let cookie = self.current_cookie();
        let resp = self
            .http
            .get(&url)
            .header("Cookie", cookie)
            .header("User-Agent", "Namehold/0.2.0")
            .send()
            .await?;
        self.capture_set_cookie(resp.headers());
        Ok(resp)
    }

    async fn send_post(&self, path: &str, body: &serde_json::Value) -> Result<reqwest::Response, AppError> {
        let url = format!("{}{}", self.base_url, path);
        let cookie = self.current_cookie();
        let resp = self
            .http
            .post(&url)
            .header("Cookie", cookie)
            .header("Content-Type", "application/json")
            .header("User-Agent", "Namehold/0.2.0")
            .json(body)
            .send()
            .await?;
        self.capture_set_cookie(resp.headers());
        Ok(resp)
    }

    /// Read the body of a successful-status response and report whether it
    /// looks like a session-expired HTML page rather than JSON.
    async fn read_body_and_check_expired(resp: reqwest::Response) -> Result<(bool, String), AppError> {
        let content_type = resp
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_ascii_lowercase());
        let body = resp.text().await?;
        let expired = looks_like_session_expired(content_type.as_deref(), &body);
        Ok((expired, body))
    }

    /// Turn a GET response into JSON, or a typed session-expired error when the
    /// response looks like Namebase's HTML login page (see
    /// `looks_like_session_expired`) instead of a raw JSON-parse error.
    async fn json_or_session_expired(resp: reqwest::Response) -> Result<serde_json::Value, AppError> {
        let status = resp.status();
        if !status.is_success() {
            return Err(AppError::Other(format!("Namebase returned status {}", status)));
        }
        let (expired, body) = Self::read_body_and_check_expired(resp).await?;
        if expired {
            return Err(AppError::NamebaseSessionExpired);
        }
        serde_json::from_str(&body).map_err(AppError::from)
    }

    pub async fn check_session(&self) -> Result<bool, AppError> {
        let resp = self.send_get("/api/account").await?;
        if !resp.status().is_success() {
            return Ok(false);
        }
        let (expired, _body) = Self::read_body_and_check_expired(resp).await?;
        Ok(!expired)
    }

    pub async fn get_account(&self) -> Result<serde_json::Value, AppError> {
        let resp = self.send_get("/api/account").await?;
        Self::json_or_session_expired(resp).await
    }

    pub async fn get_domains(&self) -> Result<serde_json::Value, AppError> {
        let resp = self.send_get("/api/domains").await?;
        Self::json_or_session_expired(resp).await
    }

    pub async fn get_staked_domains(&self) -> Result<serde_json::Value, AppError> {
        let resp = self.send_get("/api/domains/staked").await?;
        Self::json_or_session_expired(resp).await
    }

    pub async fn get_renewals(&self) -> Result<serde_json::Value, AppError> {
        let resp = self.send_get("/api/domains/renewals").await?;
        Self::json_or_session_expired(resp).await
    }

    pub async fn get_withdrawals(&self) -> Result<serde_json::Value, AppError> {
        let resp = self.send_get("/api/withdrawals").await?;
        Self::json_or_session_expired(resp).await
    }

    pub async fn get_slds(&self) -> Result<serde_json::Value, AppError> {
        let resp = self.send_get("/api/slds").await?;
        Self::json_or_session_expired(resp).await
    }

    pub async fn transfer_domain(&self, name: &str, address: &str) -> Result<(), AppError> {
        let resp = self
            .send_post(
                &format!("/api/domains/{}/withdraw", name),
                &serde_json::json!({"address": address}),
            )
            .await?;
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
        let resp = self
            .send_post(
                "/api/withdrawals",
                &serde_json::json!({"currency": "hns", "amount": amount, "address": address}),
            )
            .await?;
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
        let resp = self.send_get("/api/domains/withdrawals").await?;
        Self::json_or_session_expired(resp).await
    }
}
