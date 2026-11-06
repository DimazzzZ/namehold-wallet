//! Client for LearnHNS Market: reads (browsing, a listing's file, the fee)
//! and, on mainnet only, the three writes that publish our own listings
//! (R23, R25, R28), plus the checks that keep a fetched listing and its
//! market fee honest.

use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::Value;

use crate::error::AppError;
use crate::models::settings::SettingsMap;
use crate::noncustodial::network::Network;
use crate::noncustodial::shakedex::listing_file::{ListingFile, MAX_LISTING_FILE_BYTES};
use crate::noncustodial::shakedex::purchase;

pub const LEARNHNS_BASE_URL: &str = "https://market.learnhns.com";
const LEARNHNS_HOST: &str = "market.learnhns.com";
/// Cap on a listings page or the fee info: a page of 100 listings is a few
/// hundred KiB, so this is generous yet bounds what the market can make us
/// hold in memory.
const MARKET_RESPONSE_MAX_BYTES: usize = 8 * 1024 * 1024;
/// R23: LearnHNS Market lists mainnet names only; every write refuses with
/// this off mainnet, and the UI disables link import with the same words
/// (`marketText.ts`).
pub const MARKET_MAINNET_ONLY: &str = "LearnHNS Market lists mainnet names only";
/// Cap on a write's reply: the replies are a few hundred bytes.
const WRITE_REPLY_MAX_BYTES: usize = 64 * 1024;

/// What a write to the market came back with. Only the market's own JSON
/// answer is a verdict (shapes: MKT `app/blueprints/api.py` @3d117361);
/// anything else — a proxy's or Flask's HTML page, a 5xx, a timeout, a
/// redirect, a 2xx not in the market's shape — is `NoAnswer`, never "the
/// market said no".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketReply<T> {
    Accepted(T),
    /// The market's own refusal: a 4xx whose body is a JSON object with a
    /// string `error`; its status and words.
    Refused {
        status: u16,
        error: String,
    },
    /// The market's own answer that its node has not seen our chain state
    /// yet (the lock coin, the name's owner, a sale or cancel tx): one of the
    /// exact status-and-words pairs in [`UPLOAD_NOT_SEEN_YET`],
    /// [`PENDING_NOT_SEEN_YET`] and [`REPORT_NOT_SEEN_YET`], recognised here
    /// and nowhere else. Not a
    /// verdict on the listing: the caller backs off as for `NoAnswer` and
    /// checks the chain on our own node before each attempt; our node is the
    /// final word.
    NotSeenYet {
        status: u16,
        error: String,
    },
    /// Why there was no answer, for the listing's status.
    NoAnswer(String),
}

/// `POST /api/upload-proof` accepted: the name it listed and whether it
/// replaced an earlier copy with the same lock and key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadAccepted {
    pub name: String,
    pub replaced: bool,
}

pub type UploadResult = MarketReply<UploadAccepted>;

/// A renewal of a listing's market expiry (`POST /api/v2/listings/<name>/renew`,
/// LearnHNS `docs/listing-renewal.md` v1): the listing's lock outpoint and the
/// expiry the market holds now, and the one asked for. The market keeps
/// `expiresAt` from any upload because the Shakedex signature does not cover
/// it; this request is signed with the lock key instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenewalRequest<'a> {
    pub name: &'a str,
    pub lock_txid: &'a str,
    pub lock_vout: u32,
    pub previous_expires_at: u64,
    pub expires_at: u64,
}

/// What the renewal endpoint took: the expiry the market now holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenewAccepted {
    pub expires_at: u64,
    /// The same request again, already applied.
    pub replayed: bool,
}

/// How a listing prices the name, in the market's `listingMode` words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListingKind {
    FixedPrice,
    ReverseAuction,
}

impl ListingKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::FixedPrice => "fixed-price",
            Self::ReverseAuction => "reverse-auction",
        }
    }
}

/// R23 day 0: the pending listing, from the sent lock TRANSFER.
#[derive(Debug, Clone)]
pub struct PendingListing<'a> {
    pub name: &'a str,
    pub transfer_txid: &'a str,
    pub transfer_vout: u32,
    pub lock_address: &'a str,
    pub kind: ListingKind,
}

/// R28: what `refresh-status` reports.
#[derive(Debug, Clone, Copy)]
pub enum StatusReport<'a> {
    Sold { sale_txid: &'a str },
    Cancelled { cancel_txid: &'a str },
}

/// What the market did with a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusRecorded {
    /// It marked its listing sold or cancelled.
    Recorded,
    /// It lists no active listing of the name ("Listing not found"): nothing
    /// to withdraw.
    NoListing,
}

/// R25: the market's copy of a listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofCopy {
    /// The body it serves (JSON; whether it is ours is the caller's compare).
    Copy(String),
    /// Its own 404 (`{"error": "Active listing not found"}`).
    NotListed,
    NoAnswer(String),
}

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
    /// The profile's network; writes refuse unless it is `main`; `None` (not
    /// told) refuses as well.
    network: Option<Network>,
    /// Debug builds only, and only with `learnhns_base_url` set (loopback):
    /// writes on regtest too, for a local market from its `compose.regtest.yaml`.
    /// A release build never sets it.
    test_network: bool,
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
        let http = crate::http::client_builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| AppError::Other(format!("Failed to create HTTP client: {e}")))?;
        Ok(Self {
            http,
            base_url: base_url.trim().trim_end_matches('/').to_string(),
            network: None,
            test_network: false,
        })
    }

    /// The client for the profile's `network`: writes need `Network::Main`.
    pub fn for_network(mut self, network: Network) -> Self {
        self.network = Some(network);
        self
    }

    /// The client the settings ask for: the real host, or in debug builds
    /// and tests the `learnhns_base_url` seam (loopback only, see
    /// [`validate_base_url`]). Release builds always use the real host.
    pub fn from_settings(settings: &SettingsMap) -> Result<Self, AppError> {
        let base = base_url_override(settings);
        if base.trim().is_empty() {
            Ok(Self::new())
        } else {
            let mut client = Self::with_base_url(base.trim())?;
            client.test_network = true;
            Ok(client)
        }
    }

    /// Whether this client writes for `network`: mainnet, or regtest against
    /// a local market in a debug build ([`Self::from_settings`]).
    pub fn writes_on(&self, network: Network) -> bool {
        network == Network::Main || (self.test_network && network == Network::Regtest)
    }

    /// The network the market's files and requests are for: the profile's.
    pub fn market_network(&self) -> Network {
        self.network.unwrap_or(Network::Main)
    }

    fn writes_allowed(&self) -> Result<(), AppError> {
        if self.network.is_some_and(|n| self.writes_on(n)) {
            Ok(())
        } else {
            Err(AppError::InvalidInput(MARKET_MAINNET_ONLY.into()))
        }
    }

    /// R23: publish `listing_file` (one step) with `POST /api/upload-proof`,
    /// the file in multipart field `proof` as written.
    pub async fn upload_proof(&self, listing_file: &str) -> Result<UploadResult, AppError> {
        self.writes_allowed()?;
        // The market saves the upload under the file name as sent, in one
        // folder shared by every upload (MKT api.py L2807–2808): name it by
        // the listing's (checked) name and lock txid (hex) so no two uploads
        // collide.
        let parsed = ListingFile::parse(listing_file, self.market_network())?;
        check_name(&parsed.name)?;
        let file_name = format!("{}-{}.json", parsed.name, hex::encode(parsed.lock_txid));
        let part = reqwest::multipart::Part::text(listing_file.to_string())
            .file_name(file_name)
            .mime_str("application/json")
            .map_err(|e| AppError::Other(format!("LearnHNS upload: {e}")))?;
        let form = reqwest::multipart::Form::new().part("proof", part);
        let req = self
            .http
            .post(format!("{}/api/upload-proof", self.base_url))
            .multipart(form);
        Ok(match self.send(req).await {
            Ok((status, Some(v))) if is_2xx(status) && v["success"] == true => {
                match (v["name"].as_str(), v["replaced"].as_bool()) {
                    (Some(name), Some(replaced)) => MarketReply::Accepted(UploadAccepted {
                        name: name.into(),
                        replaced,
                    }),
                    _ => no_shape(status),
                }
            }
            Ok((status, v)) => refusal_or_none(status, v, UPLOAD_NOT_SEEN_YET),
            Err(why) => MarketReply::NoAnswer(why),
        })
    }

    /// R23 day 0: `POST /api/v2/pending-listings`, JSON; no price and no note.
    pub async fn post_pending_listing(
        &self,
        p: &PendingListing<'_>,
    ) -> Result<MarketReply<()>, AppError> {
        self.writes_allowed()?;
        check_name(p.name)?;
        let body = serde_json::json!({
            "name": p.name,
            "network": self.market_network().as_str(),
            "transferTxHash": p.transfer_txid,
            "transferOutputIdx": p.transfer_vout,
            "lockScriptAddr": p.lock_address,
            "listingMode": p.kind.as_str(),
        });
        let req = self
            .http
            .post(format!("{}/api/v2/pending-listings", self.base_url))
            .json(&body);
        Ok(match self.send(req).await {
            Ok((status, Some(v)))
                if is_2xx(status) && v["success"] == true && v["pending"].is_object() =>
            {
                MarketReply::Accepted(())
            }
            Ok((status, v)) => refusal_or_none(status, v, PENDING_NOT_SEEN_YET),
            Err(why) => MarketReply::NoAnswer(why),
        })
    }

    /// Renew a listing's expiry on the market with a `signature` the caller
    /// made with the lock key (`shakedex::renewal`); this client never signs. Writes are mainnet only, like
    /// every other write here. A 409 (the market's expiry moved since it was
    /// read, or the listing can't be renewed) is the market's refusal.
    pub async fn renew_listing(
        &self,
        r: &RenewalRequest<'_>,
        signature: &str,
    ) -> Result<MarketReply<RenewAccepted>, AppError> {
        self.writes_allowed()?;
        check_name(r.name)?;
        let network = self.market_network();
        let body = serde_json::json!({
            "network": network.as_str(),
            "lockingTxHash": r.lock_txid.to_ascii_lowercase(),
            "lockingOutputIdx": r.lock_vout,
            "previousExpiresAt": r.previous_expires_at,
            "expiresAt": r.expires_at,
            "signature": signature,
        });
        let req = self
            .http
            .post(format!(
                "{}/api/v2/listings/{}/renew",
                self.base_url, r.name
            ))
            .json(&body);
        Ok(match self.send(req).await {
            Ok((status, Some(v))) if is_2xx(status) && v["success"] == true => {
                match v["expiresAt"].as_u64() {
                    Some(expires_at) => MarketReply::Accepted(RenewAccepted {
                        expires_at,
                        replayed: v["replayed"] == true,
                    }),
                    None => no_shape(status),
                }
            }
            Ok((status, v)) => refusal_or_none(status, v, &[]),
            Err(why) => MarketReply::NoAnswer(why),
        })
    }

    /// R28: `POST /api/v2/listings/<name>/refresh-status`, telling the
    /// market of a sale or a cancel. Its "Listing not found" means it lists
    /// nothing of the name to mark.
    pub async fn refresh_status(
        &self,
        name: &str,
        report: &StatusReport<'_>,
    ) -> Result<MarketReply<StatusRecorded>, AppError> {
        self.writes_allowed()?;
        check_name(name)?;
        let (body, key) = match report {
            StatusReport::Sold { sale_txid } => {
                (serde_json::json!({ "saleTxHash": sale_txid }), "sold")
            }
            StatusReport::Cancelled { cancel_txid } => (
                serde_json::json!({ "outcome": "cancelled", "cancelTxHash": cancel_txid }),
                "cancelled",
            ),
        };
        let req = self
            .http
            .post(format!(
                "{}/api/v2/listings/{name}/refresh-status",
                self.base_url
            ))
            .json(&body);
        Ok(match self.send(req).await {
            Ok((status, Some(v))) if is_2xx(status) && v[key] == true => {
                MarketReply::Accepted(StatusRecorded::Recorded)
            }
            // 400 on our path (we always send a txid: MKT
            // `_refreshable_listing_for_spend`, L478/L495), 404 on the others.
            Ok((400 | 404, Some(v))) if v["error"] == "Listing not found" => {
                MarketReply::Accepted(StatusRecorded::NoListing)
            }
            Ok((status, v)) => refusal_or_none(status, v, REPORT_NOT_SEEN_YET),
            Err(why) => MarketReply::NoAnswer(why),
        })
    }

    /// R25: `GET /listing/<name>/proof.json`, telling the market's "not
    /// listed" (its own JSON 404) from no answer. Reads are allowed on any
    /// network; the job calls it on mainnet only. The same path as
    /// [`Self::listing_file`], for another caller: the keep-listed job must
    /// not take a proxy's 404 page or a 5xx for "not listed" (it would
    /// upload on it), while an import only needs the file or an error.
    pub async fn proof_copy(&self, name: &str) -> Result<ProofCopy, AppError> {
        check_name(name)?;
        let req = self
            .http
            .get(format!("{}/listing/{name}/proof.json", self.base_url));
        let (status, body) = match self.send_raw(req, MAX_LISTING_FILE_BYTES).await {
            Ok(r) => r,
            Err(why) => return Ok(ProofCopy::NoAnswer(why)),
        };
        let json = json_object(&body);
        Ok(match (status, json) {
            (200, Some(_)) => match String::from_utf8(body) {
                Ok(text) => ProofCopy::Copy(text),
                Err(_) => ProofCopy::NoAnswer(no_answer_text(status)),
            },
            (404, Some(v)) if v["error"].is_string() => ProofCopy::NotListed,
            _ => ProofCopy::NoAnswer(no_answer_text(status)),
        })
    }

    /// Send a write: `Ok((status, body if it is a JSON object))`, or
    /// `Err(reason)` when nothing came back (transport error, timeout, a body
    /// over [`WRITE_REPLY_MAX_BYTES`]). A redirect is not followed and comes
    /// back as its 3xx status.
    async fn send(&self, req: reqwest::RequestBuilder) -> Result<(u16, Option<Value>), String> {
        let (status, body) = self.send_raw(req, WRITE_REPLY_MAX_BYTES).await?;
        Ok((status, json_object(&body)))
    }

    /// Send `req`: its status and body, at most `max_bytes` read.
    async fn send_raw(
        &self,
        req: reqwest::RequestBuilder,
        max_bytes: usize,
    ) -> Result<(u16, Vec<u8>), String> {
        let resp = req
            .send()
            .await
            .map_err(|e| format!("LearnHNS Market did not answer: {e}"))?;
        let status = resp.status().as_u16();
        let body = read_capped(resp, max_bytes)
            .await
            .map_err(|e| e.to_string())?;
        Ok((status, body))
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

    /// The listing file text for a name, or `None` when the market has none:
    /// the import of a market link (R5). Any 404 is `None` and any other
    /// failure an error, which an import shows; the keep-listed job reads
    /// the same path through [`Self::proof_copy`], which tells the
    /// market's own "not listed" from no answer.
    pub async fn listing_file(&self, name: &str) -> Result<Option<String>, AppError> {
        check_name(name)?;
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
        let resp = self.http.get(url).send().await.map_err(request_failed)?;
        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(AppError::Other(format!(
                "LearnHNS Market request failed: HTTP {status}"
            )));
        }
        let body = read_capped(resp, max_bytes).await?;
        String::from_utf8(body)
            .map(Some)
            .map_err(|_| AppError::Other("LearnHNS Market returned text that is not UTF-8".into()))
    }
}

fn request_failed(e: reqwest::Error) -> AppError {
    AppError::Other(format!("LearnHNS Market request failed: {e}"))
}

/// Read `resp`'s body; one over `max_bytes` is refused.
async fn read_capped(resp: reqwest::Response, max_bytes: usize) -> Result<Vec<u8>, AppError> {
    crate::http::read_capped(resp, max_bytes)
        .await
        .map_err(|e| match e {
            crate::http::BodyError::Transport(e) => request_failed(e),
            too_large => AppError::Other(format!("LearnHNS Market {too_large}")),
        })
}

/// Read the `learnhns_base_url` test seam, but ONLY in debug builds / tests.
/// Release builds always talk to the real LearnHNS Market host.
fn base_url_override(_settings: &SettingsMap) -> String {
    #[cfg(any(debug_assertions, test))]
    {
        // Unset is empty, which the caller reads as "use the real host".
        _settings
            .get("learnhns_base_url")
            .cloned()
            .unwrap_or_default()
    }
    #[cfg(not(any(debug_assertions, test)))]
    {
        String::new()
    }
}

fn is_2xx(status: u16) -> bool {
    (200..300).contains(&status)
}

/// `body` as a JSON object, or `None` (an HTML page, other JSON, not JSON).
fn json_object(body: &[u8]) -> Option<Value> {
    serde_json::from_slice::<Value>(body)
        .ok()
        .filter(Value::is_object)
}

/// The market's chain-lag replies to an upload (MKT api.py @3d117361,
/// `_verify_listing_proof_on_chain`): its hsd has not seen the lock coin
/// (`_fetch_hsd_coin`, L993) or the name (`_fetch_hsd_name_info`, L1082), or
/// does not yet see the lock as the name's owner (L1111).
const UPLOAD_NOT_SEEN_YET: &[(u16, &str)] = &[
    (404, "Listing coin was not found or is already spent"),
    (404, "Name was not found"),
    (
        409,
        "The proof locking output is not the name's current on-chain owner coin",
    ),
];

/// The market's replies to a pending listing whose TRANSFER is not yet its
/// view of the name's confirmed owner (learnhns-market#2, `create_pending_listing`).
/// We post once the TRANSFER is broadcast, so the first answer is usually one
/// of these. A TRANSFER that never confirms backs off until the listing moves.
const PENDING_NOT_SEEN_YET: &[(u16, &str)] = &[
    (409, "The referenced TRANSFER is not the current name owner"),
    (409, "Current owner must be a confirmed unspent TRANSFER"),
];

/// The market's chain-lag reply to a sale or cancel report: neither its hsd
/// (`_fetch_hsd_tx`, L997–1001) nor its explorer has seen the tx
/// (`_verified_listing_spend`, L598–611).
const REPORT_NOT_SEEN_YET: &[(u16, &str)] = &[(404, "Transaction was not found")];

/// The market's own answer that is not acceptance: its chain-lag reply (an
/// exact pair of `not_seen_yet`), its refusal (any other 4xx whose JSON body
/// has a string `error`), or no answer: a 5xx, a 3xx, an HTML page, a 2xx
/// not in the shape.
fn refusal_or_none<T>(
    status: u16,
    v: Option<Value>,
    not_seen_yet: &[(u16, &str)],
) -> MarketReply<T> {
    match v.as_ref().and_then(|v| v["error"].as_str()) {
        Some(error) if not_seen_yet.contains(&(status, error)) => MarketReply::NotSeenYet {
            status,
            error: error.to_string(),
        },
        Some(error) if (400..500).contains(&status) => MarketReply::Refused {
            status,
            error: error.to_string(),
        },
        _ => no_shape(status),
    }
}

fn no_shape<T>(status: u16) -> MarketReply<T> {
    MarketReply::NoAnswer(no_answer_text(status))
}

fn no_answer_text(status: u16) -> String {
    format!("LearnHNS Market gave no answer it could be read by (HTTP {status})")
}

/// Every name in a market URL is checked first: a bad one is our refusal.
fn check_name(name: &str) -> Result<(), AppError> {
    if crate::noncustodial::names::verify_name(name) {
        Ok(())
    } else {
        Err(AppError::InvalidInput(
            "not a valid Handshake name".to_string(),
        ))
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
/// `/`), or in the listing's own `…/listing/<name>/proof.json`, lowercased;
/// anything else is refused.
pub fn name_from_listing_link(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix(LEARNHNS_BASE_URL)?
        .strip_prefix("/listing/")?;
    let rest = rest.strip_suffix("/proof.json").unwrap_or(rest);
    // One trailing slash is allowed, not required.
    let name = rest.strip_suffix('/').unwrap_or(rest).to_ascii_lowercase();
    crate::noncustodial::names::verify_name(&name).then_some(name)
}

/// True when the fee is paid to the market's published address and does not
/// exceed its published rate of the price, nor
/// [`purchase::MAX_MARKET_FEE_BPS`] of it whatever rate the market publishes:
/// the market's own word does not make a larger fee its own.
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
        && !purchase::market_fee_over_cap(fee, price)
}
