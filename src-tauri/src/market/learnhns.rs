//! Read-only client for LearnHNS Market, plus the checks that keep a fetched
//! listing and its market fee honest.

use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::Deserialize;

use crate::error::AppError;
use crate::noncustodial::shakedex::listing_file::MAX_LISTING_FILE_BYTES;

pub const LEARNHNS_BASE_URL: &str = "https://market.learnhns.com";
const LEARNHNS_HOST: &str = "market.learnhns.com";
/// Cap on a listings page or the fee info: a page of 100 listings is a few
/// hundred KiB, so this is generous yet bounds what the market can make us
/// hold in memory.
const MARKET_RESPONSE_MAX_BYTES: usize = 8 * 1024 * 1024;

/// The market's published fee: a rate in basis points and the address it is paid to.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FeeInfo {
    pub rate: u32,
    pub address: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LearnHnsClient {
    http: Client,
    base_url: String,
}

#[derive(Deserialize)]
struct AuctionsPage {
    auctions: Vec<serde_json::Value>,
    total: u64,
}

impl Default for LearnHnsClient {
    fn default() -> Self {
        Self::new()
    }
}

impl LearnHnsClient {
    pub fn new() -> Self {
        Self::with_base_url(LEARNHNS_BASE_URL).expect("the default LearnHNS base URL is valid")
    }

    pub fn with_base_url(base_url: &str) -> Result<Self, AppError> {
        validate_base_url(base_url)?;
        // No redirects: every request goes to the market host itself, and a
        // redirect could point anywhere (R5 fetches no arbitrary URLs).
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| AppError::Other(format!("Failed to create HTTP client: {e}")))?;
        Ok(Self {
            http,
            base_url: base_url.trim().trim_end_matches('/').to_string(),
        })
    }

    /// One page of available listings (raw rows) and the total count.
    pub async fn list_available(
        &self,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<serde_json::Value>, u64), AppError> {
        let url = format!(
            "{}/api/v2/auctions?availability=available&page={page}&per_page={per_page}",
            self.base_url
        );
        let body = self
            .get(&url, MARKET_RESPONSE_MAX_BYTES)
            .await?
            .ok_or_else(not_found)?;
        let parsed: AuctionsPage = serde_json::from_str(&body)
            .map_err(|e| AppError::Other(format!("LearnHNS Market returned bad data: {e}")))?;
        Ok((parsed.auctions, parsed.total))
    }

    /// The listing file text for a name, or `None` when the market has none.
    pub async fn listing_file(&self, name: &str) -> Result<Option<String>, AppError> {
        if !crate::noncustodial::names::verify_name(name) {
            return Err(AppError::InvalidInput(
                "not a valid Handshake name".to_string(),
            ));
        }
        self.get(
            &format!("{}/listing/{name}/proof.json", self.base_url),
            MAX_LISTING_FILE_BYTES,
        )
        .await
    }

    pub async fn fee_info(&self) -> Result<FeeInfo, AppError> {
        let body = self
            .get(
                &format!("{}/api/v2/fee_info", self.base_url),
                MARKET_RESPONSE_MAX_BYTES,
            )
            .await?
            .ok_or_else(not_found)?;
        serde_json::from_str(&body)
            .map_err(|e| AppError::Other(format!("LearnHNS Market returned bad data: {e}")))
    }

    /// GET `url`: `None` on 404, the body otherwise. A body over `max_bytes`
    /// is refused; at most `max_bytes + 1` bytes are ever read.
    async fn get(&self, url: &str, max_bytes: usize) -> Result<Option<String>, AppError> {
        let fail =
            |e: reqwest::Error| AppError::Other(format!("LearnHNS Market request failed: {e}"));
        let mut resp = self.http.get(url).send().await.map_err(fail)?;
        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(AppError::Other(format!(
                "LearnHNS Market request failed: HTTP {status}"
            )));
        }
        let too_large = || {
            AppError::Other(format!(
                "LearnHNS Market response is larger than {max_bytes} bytes"
            ))
        };
        if resp.content_length().is_some_and(|n| n > max_bytes as u64) {
            return Err(too_large());
        }
        let mut body: Vec<u8> = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(fail)? {
            if body.len() + chunk.len() > max_bytes {
                return Err(too_large());
            }
            body.extend_from_slice(&chunk);
        }
        String::from_utf8(body)
            .map(Some)
            .map_err(|_| AppError::Other("LearnHNS Market returned text that is not UTF-8".into()))
    }
}

fn not_found() -> AppError {
    AppError::Other("LearnHNS Market request failed: HTTP 404 Not Found".to_string())
}

/// https for the real host; loopback http only in debug builds and tests.
fn validate_base_url(base_url: &str) -> Result<(), AppError> {
    let trimmed = base_url.trim().trim_end_matches('/');
    let parsed = url::Url::parse(trimmed).map_err(|e| {
        AppError::InvalidInput(format!("LearnHNS base URL is not a valid URL: {e}"))
    })?;
    // A URL without a host (`file:`, `data:`) matches no allowed host below
    // and is refused.
    let host = parsed.host_str().unwrap_or("");
    if host.eq_ignore_ascii_case(LEARNHNS_HOST) {
        if parsed.scheme() != "https" {
            return Err(AppError::InvalidInput(
                "LearnHNS base URL must use https".to_string(),
            ));
        }
        return Ok(());
    }
    #[cfg(any(debug_assertions, test))]
    {
        let is_loopback = host == "localhost"
            || parsed
                .host()
                .map(|h| match h {
                    url::Host::Ipv4(a) => a.is_loopback(),
                    url::Host::Ipv6(a) => a.is_loopback(),
                    url::Host::Domain(_) => false,
                })
                // No host is not a loopback host.
                .unwrap_or(false);
        if is_loopback && matches!(parsed.scheme(), "http" | "https") {
            return Ok(());
        }
    }
    Err(AppError::InvalidInput(
        "LearnHNS base URL must be the LearnHNS Market host".to_string(),
    ))
}

/// The name in `https://market.learnhns.com/listing/<name>` (optional trailing
/// `/`), lowercased; anything else is refused.
pub fn name_from_listing_link(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix(LEARNHNS_BASE_URL)?
        .strip_prefix("/listing/")?;
    // One trailing slash is allowed, not required.
    let name = rest.strip_suffix('/').unwrap_or(rest).to_ascii_lowercase();
    crate::noncustodial::names::verify_name(&name).then_some(name)
}

/// True when the fee is paid to the market's published address and does not
/// exceed its published rate of the price.
pub fn market_fee_is_published(
    fee: u64,
    fee_addr: Option<&str>,
    price: u64,
    info: &FeeInfo,
) -> bool {
    fee > 0
        && fee_addr.is_some()
        && fee_addr == info.address.as_deref()
        && (fee as u128) * 10_000 <= (price as u128) * (info.rate as u128)
}
