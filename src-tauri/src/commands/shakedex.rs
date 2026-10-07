//! Shakedex listings: browse LearnHNS Market and import a listing (file,
//! pasted JSON or a LearnHNS link).
//!
//! Every listing, whatever its source, is verified on the profile's own node
//! (`verify::verify_listing`) before it can be bought; the market's word is
//! never taken for availability, price or fee.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::draft_ctx;
use crate::db::queries;
use crate::error::AppError;
use crate::market::learnhns::{name_from_listing_link, LearnHnsClient};
use crate::models::settings::SettingsMap;
use crate::noncustodial::derivation;
use crate::noncustodial::network::Network;
use crate::noncustodial::rpc::{ChainSource, NodeRpcClient};
use crate::noncustodial::shakedex::listing_file::{ListingFile, MAX_LISTING_FILE_BYTES};
use crate::noncustodial::shakedex::template;
use crate::noncustodial::shakedex::verify::{self, Hidden, Verdict};
use crate::AppState;

/// Listings fetched per market page; each one is verified in turn, against
/// one tip and MTP read for the whole page.
const MARKET_PER_PAGE: u32 = 100;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceStepView {
    pub price: u64,
    pub lock_time: u64,
    /// Seconds of the node's MTP until this step is valid for the next block
    /// (0: valid now), by R3's rule; `None` when the listing is not buyable,
    /// so no MTP was read.
    pub valid_in_secs: Option<u64>,
}

/// A listing with one price step is Buy Now; with several, a reverse auction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ListingKind {
    BuyNow,
    ReverseAuction,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketRow {
    pub listing_json: String,
    pub name: String,
    pub verdict: Verdict,
    pub kind: ListingKind,
    pub current_price: Option<u64>,
    pub next_price: Option<u64>,
    pub next_valid_in_secs: Option<u64>,
    pub floor_price: u64,
    pub steps: Vec<PriceStepView>,
    pub expires_at: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HiddenCounts {
    pub sold_or_cancelled: u32,
    pub failed_verification: u32,
    pub expires_before_finalize: u32,
    pub not_yet_valid: u32,
    pub could_not_check: u32,
}

impl HiddenCounts {
    fn count(&mut self, h: &Hidden) {
        let slot = match h {
            Hidden::SoldOrCancelled => &mut self.sold_or_cancelled,
            Hidden::FailedVerification { .. } => &mut self.failed_verification,
            Hidden::ExpiresBeforeFinalize => &mut self.expires_before_finalize,
            Hidden::NotYetValid { .. } => &mut self.not_yet_valid,
            // Unverified rows are listed, not hidden, so this arm is only
            // for completeness: it is the nearest counter.
            Hidden::CouldNotCheck { .. } | Hidden::Unverified => &mut self.could_not_check,
        };
        *slot += 1;
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketPage {
    pub rows: Vec<MarketRow>,
    pub hidden: HiddenCounts,
    /// False in SPV and Explorer modes, where listings cannot be checked.
    pub verified: bool,
    /// LearnHNS Market exists on mainnet only.
    pub network_has_market: bool,
    /// Every hidden listing with its reason, behind the counter (R8).
    pub hidden_rows: Vec<HiddenRow>,
    /// This page's number, from 1.
    pub page: u32,
    /// How many pages the market has, at least 1. `rows` and `hidden` count
    /// this page only.
    pub page_count: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HiddenRow {
    /// The listing's name; `None` when the row carries none.
    pub name: Option<String>,
    pub reason: Hidden,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ImportSource {
    File { path: String },
    Text { json: String },
    Link { url: String },
}

// --- helpers ----------------------------------------------------------------

/// Read the `learnhns_base_url` test seam, but ONLY in debug builds / tests.
/// Release builds always talk to the real LearnHNS Market host.
fn learnhns_base_url_override(_settings: &SettingsMap) -> String {
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

fn learnhns_client(settings: &SettingsMap) -> Result<LearnHnsClient, AppError> {
    let base = learnhns_base_url_override(settings);
    if base.trim().is_empty() {
        Ok(LearnHnsClient::new())
    } else {
        LearnHnsClient::with_base_url(base.trim())
    }
}

/// What browsing and importing need: no keys, any profile kind.
struct BrowseCtx {
    network: Network,
    node: NodeRpcClient,
    settings: SettingsMap,
}

impl BrowseCtx {
    /// Listings can be verified only against a node that serves coins and
    /// name state (R7): a local or remote full node.
    fn can_verify(&self) -> bool {
        matches!(
            self.node.source(),
            ChainSource::LocalNode | ChainSource::RemoteNode
        )
    }
}

fn browse_ctx(state: &State<'_, AppState>) -> Result<BrowseCtx, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let profile = draft_ctx::active_profile(&conn)?;
    Ok(BrowseCtx {
        network: derivation::network_from_profile(&profile.network)?,
        node: NodeRpcClient::for_profile(&conn, &profile.id)?,
        settings: queries::get_settings(&conn)?,
    })
}

fn unverified() -> Verdict {
    Verdict::Hidden(Hidden::Unverified)
}

async fn verdict_for(ctx: &BrowseCtx, l: &ListingFile) -> Verdict {
    if ctx.can_verify() {
        verify::verify_listing(&ctx.node, ctx.network, l).await
    } else {
        unverified()
    }
}

fn market_row(listing_json: String, l: &ListingFile, verdict: Verdict) -> MarketRow {
    let mtp = match &verdict {
        Verdict::Buyable(b) => Some(b.mtp),
        Verdict::Hidden(_) => None,
    };
    let (current_price, next_price, next_valid_in_secs) = match &verdict {
        Verdict::Buyable(b) => {
            let next = b.next_step.and_then(|i| l.steps.get(i));
            (
                l.steps.get(b.current_step).map(|s| s.price),
                next.map(|s| s.price),
                next.map(|s| template::secs_until_valid(s.lock_time, b.mtp)),
            )
        }
        Verdict::Hidden(_) => (None, None, None),
    };
    MarketRow {
        listing_json,
        name: l.name.clone(),
        kind: if l.steps.len() == 1 {
            ListingKind::BuyNow
        } else {
            ListingKind::ReverseAuction
        },
        current_price,
        next_price,
        next_valid_in_secs,
        // Unreachable: `ListingFile::parse` refuses a listing without steps.
        floor_price: l.steps.iter().map(|s| s.price).min().unwrap_or(0),
        steps: l
            .steps
            .iter()
            .map(|s| PriceStepView {
                price: s.price,
                lock_time: s.lock_time,
                valid_in_secs: mtp.map(|mtp| template::secs_until_valid(s.lock_time, mtp)),
            })
            .collect(),
        expires_at: l.expires_at,
        verdict,
    }
}

/// Why a market link cannot be imported off mainnet. The UI disables link
/// import with the same words (`marketText.ts`).
pub const MARKET_MAINNET_ONLY: &str = "LearnHNS Market lists mainnet names only";

/// Read a listing file chosen in the UI. The path comes from the renderer,
/// so only a regular file is read, and never more than the cap plus one byte
/// whatever its reported size.
fn read_listing_file(path: &str) -> Result<String, AppError> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(AppError::InvalidInput(
            "listing file: not a regular file".into(),
        ));
    }
    let mut text = String::new();
    file.take(MAX_LISTING_FILE_BYTES as u64 + 1)
        .read_to_string(&mut text)?;
    if text.len() > MAX_LISTING_FILE_BYTES {
        return Err(AppError::InvalidInput(format!(
            "listing file: larger than {MAX_LISTING_FILE_BYTES} bytes"
        )));
    }
    Ok(text)
}

// --- commands ---------------------------------------------------------------

/// One page of LearnHNS Market listings, each verified on the profile's node.
/// Only buyable listings are returned; the rest are counted in `hidden`. In
/// SPV and Explorer modes every row is returned unverified.
#[tauri::command]
pub async fn shakedex_list_market(
    state: State<'_, AppState>,
    page: Option<u32>,
) -> Result<MarketPage, AppError> {
    let ctx = browse_ctx(&state)?;
    let verified = ctx.can_verify();
    if ctx.network != Network::Main {
        return Ok(MarketPage {
            rows: Vec::new(),
            hidden: HiddenCounts::default(),
            verified,
            network_has_market: false,
            hidden_rows: Vec::new(),
            page: 1,
            page_count: 1,
        });
    }
    let client = learnhns_client(&ctx.settings)?;
    // No page asked for is the first page; pages count from 1.
    let page = page.unwrap_or(1).max(1);
    let (raw_rows, total) = client.list_available(page, MARKET_PER_PAGE).await?;
    // A total past u32 pages only caps how far the pager reaches; it decides
    // nothing about any listing.
    let page_count = u32::try_from(total.div_ceil(u64::from(MARKET_PER_PAGE)))
        .unwrap_or(u32::MAX)
        .max(1);
    let mut rows = Vec::new();
    let mut hidden = HiddenCounts::default();
    let mut hidden_rows = Vec::new();
    // One tip/MTP for the whole page: every listing is judged at the same
    // point, and the node is asked once rather than once per listing.
    let at = if verified {
        Some(verify::chain_point(&ctx.node).await)
    } else {
        None
    };
    for raw in raw_rows {
        let text = serde_json::to_string(&raw)?;
        let listing = match ListingFile::parse(&text, ctx.network) {
            Ok(l) => l,
            Err(e) => {
                let reason = Hidden::FailedVerification {
                    reason: e.to_string(),
                };
                hidden.count(&reason);
                hidden_rows.push(HiddenRow {
                    // A listing that failed to parse may lack even its
                    // name; it is still counted and listed as hidden.
                    name: raw["name"].as_str().map(str::to_owned),
                    reason,
                });
                continue;
            }
        };
        let verdict = match &at {
            None => {
                rows.push(market_row(text, &listing, unverified()));
                continue;
            }
            Some(Err(h)) => Verdict::Hidden(h.clone()),
            Some(Ok(at)) => verify::verify_listing_at(&ctx.node, ctx.network, &listing, *at).await,
        };
        match verdict {
            v @ Verdict::Buyable(_) => rows.push(market_row(text, &listing, v)),
            Verdict::Hidden(h) => {
                hidden.count(&h);
                hidden_rows.push(HiddenRow {
                    name: Some(listing.name.clone()),
                    reason: h,
                });
            }
        }
    }
    Ok(MarketPage {
        rows,
        hidden,
        verified,
        network_has_market: true,
        hidden_rows,
        page,
        page_count,
    })
}

/// Import a listing from a file, pasted JSON or a LearnHNS listing link and
/// verify it. The row is returned whatever the verdict, so the UI can say why
/// it cannot be bought.
#[tauri::command]
pub async fn shakedex_import_listing(
    state: State<'_, AppState>,
    source: ImportSource,
) -> Result<MarketRow, AppError> {
    let ctx = browse_ctx(&state)?;
    let mut linked_name = None;
    let text = match source {
        ImportSource::File { path } => read_listing_file(&path)?,
        ImportSource::Text { json } => json,
        ImportSource::Link { url } => {
            let name = name_from_listing_link(url.trim()).ok_or_else(|| {
                AppError::InvalidInput(
                    "only https://market.learnhns.com/listing/<name> links can be imported".into(),
                )
            })?;
            if ctx.network != Network::Main {
                return Err(AppError::InvalidInput(MARKET_MAINNET_ONLY.into()));
            }
            linked_name = Some(name.clone());
            learnhns_client(&ctx.settings)?
                .listing_file(&name)
                .await?
                .ok_or_else(|| {
                    AppError::NotFound(format!("LearnHNS Market has no listing for {name}"))
                })?
        }
    };
    let listing = ListingFile::parse(&text, ctx.network)?;
    if let Some(name) = linked_name.filter(|n| *n != listing.name) {
        return Err(AppError::InvalidInput(format!(
            "the market returned a listing for {} at the link for {name}",
            listing.name
        )));
    }
    let verdict = verdict_for(&ctx, &listing).await;
    Ok(market_row(text, &listing, verdict))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTING_FILE: &str = include_str!("../../tests/vectors/shakedex/proof_dexreviews.json");

    /// The live listing with a second, cheaper step a day later.
    fn two_steps() -> ListingFile {
        let mut j: serde_json::Value = serde_json::from_str(LISTING_FILE).unwrap();
        let mut step = j["data"][0].clone();
        step["price"] = 200_000_000.into();
        step["lockTime"] = (1_783_696_480u64 + 86_400).into();
        j["data"].as_array_mut().unwrap().push(step);
        ListingFile::parse(&j.to_string(), Network::Main).unwrap()
    }

    /// Every step's wait comes from the backend's consensus rule (R3), so the
    /// Market's step list cannot drift from it: step 0 (lockTime 1783696480)
    /// is valid from MTP 1783696385, step 1 (+86400 s) 86016 s later — both
    /// rounded down to 512 s, plus one.
    #[test]
    fn market_row_gives_each_steps_wait_at_the_nodes_mtp() {
        let l = two_steps();
        let buyable = Verdict::Buyable(verify::Buyable {
            current_step: 0,
            next_step: Some(1),
            lock_value: 0,
            name_height: 0,
            expiry_end: 0,
            warn_expiry: false,
            mtp: 1_783_696_385,
            tip: 0,
        });
        let row = market_row(l.to_json().unwrap(), &l, buyable);
        let waits: Vec<_> = row.steps.iter().map(|s| s.valid_in_secs).collect();
        assert_eq!(waits, [Some(0), Some(86_016)]);
        assert_eq!(row.next_valid_in_secs, Some(86_016));

        let row = market_row(l.to_json().unwrap(), &l, unverified());
        assert!(row.steps.iter().all(|s| s.valid_in_secs.is_none()));
    }

    #[test]
    fn listing_kind_has_the_ui_spelling() {
        // `MarketRow.kind` in src/types/index.ts.
        assert_eq!(serde_json::to_value(ListingKind::BuyNow).unwrap(), "buyNow");
        assert_eq!(
            serde_json::to_value(ListingKind::ReverseAuction).unwrap(),
            "reverseAuction"
        );
    }
}
