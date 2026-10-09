//! Buying names from Shakedex listings: browse LearnHNS Market, import a
//! listing (file, pasted JSON or a LearnHNS link), preview a purchase and
//! build its draft. The draft then goes through the usual secure confirm,
//! sign and broadcast commands in `commands::tx`.
//!
//! Every listing, whatever its source, is verified on the profile's own node
//! (`verify::verify_listing`) before it can be bought; the market's word is
//! never taken for availability, price or fee.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::draft_ctx::{self, random_id, Ctx};
use crate::db::queries::{
    self, ListingMode, ListingState, NameCoin, PurchaseState, ShakedexListing, ShakedexPurchase,
};
use crate::error::AppError;
use crate::market::learnhns::{
    market_fee_is_published, name_from_listing_link, FeeInfo, LearnHnsClient,
};
use crate::models::settings::SettingsMap;
use crate::noncustodial::actions::{self, PrimaryOutput};
use crate::noncustodial::derivation;
use crate::noncustodial::network::{NameParams, Network};
use crate::noncustodial::rpc::{self, ChainSource, NodeRpcClient};
use crate::noncustodial::send::{self, DUST_THRESHOLD};
use crate::noncustodial::session::session_ttl_ms;
use crate::noncustodial::shakedex::listing_file::{ListingFile, MAX_LISTING_FILE_BYTES};
use crate::noncustodial::shakedex::lock_key::{derive_lock_key, LockKey};
use crate::noncustodial::shakedex::purchase::{
    self, FinalizeInput, MarketFee, PurchaseFinalizeSummary, PurchaseInput, PurchaseSummary,
    PURCHASE_ACTION, PURCHASE_FINALIZE_ACTION,
};
use crate::noncustodial::shakedex::script::lock_address;
use crate::noncustodial::shakedex::sell::{self, LOCK_ACTION, LOCK_COSTS};
use crate::noncustodial::shakedex::template;
use crate::noncustodial::shakedex::verify::{self, Buyable, Hidden, Verdict};
use crate::noncustodial::sync::{COV_FINALIZE, COV_REGISTER, COV_RENEW, COV_TRANSFER, COV_UPDATE};
use crate::noncustodial::tx::output_address_from_string;
use crate::noncustodial::types::TxDraftSummary;
use crate::providers::signer::WriteCapability;
use crate::AppState;

/// Listings fetched per market page; each one is verified in turn, against
/// one tip and MTP read for the whole page.
const MARKET_PER_PAGE: u32 = 100;
const UNPUBLISHED_FEE_WARNING: &str = "not signed by the seller and not the market's published \
     fee — anyone could have added it";
const DNS_RECORDS_HINT: &str =
    "The name still carries the seller's DNS records: update them once it is yours.";

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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketFeeLine {
    pub value_doos: u64,
    /// "1.99%", or `None` when the fee is no meaningful share of the price
    /// (`purchase::fee_percent_text`).
    pub percent_text: Option<String>,
    pub published: bool,
    /// True only when a fee output would actually be added: a valid fee
    /// address and a fee at or above the dust limit.
    pub payable: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchasePreview {
    pub name: String,
    pub price_doos: u64,
    pub market_fee: Option<MarketFeeLine>,
    pub network_fee_doos: u64,
    pub total_doos: u64,
    /// `purchase::finalize_wait_text` for the profile's network.
    pub finalize_wait: String,
    pub warn_expiry: bool,
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

/// The user-facing reason a listing that did not verify cannot be bought.
fn hidden_to_error(h: Hidden) -> AppError {
    AppError::InvalidInput(match h {
        Hidden::SoldOrCancelled => "this listing is already sold or cancelled".into(),
        Hidden::FailedVerification { reason } => {
            format!("this listing failed verification: {reason}")
        }
        Hidden::ExpiresBeforeFinalize => {
            "the name expires before the purchase could be finalized".into()
        }
        Hidden::NotYetValid {
            first_valid_in_secs,
        } => format!(
            "no price step of this listing is valid yet (the first in about {} minutes)",
            first_valid_in_secs.div_ceil(60)
        ),
        Hidden::CouldNotCheck { reason } => {
            format!("could not check this listing on your node: {reason}")
        }
        Hidden::Unverified => {
            "this listing cannot be checked here: buying needs a full or remote node".into()
        }
    })
}

/// The market fee line a purchase shows, and the fee output it pays (if any).
///
/// `published` must come from a freshly fetched `fee_info` and only for a
/// listing that came from LearnHNS (R11). A missing fee address, or a fee
/// below dust, is never paid: the line says why.
fn decide_market_fee(
    l: &ListingFile,
    step: usize,
    network: Network,
    pay_market_fee: bool,
    published: bool,
) -> (Option<MarketFeeLine>, Option<MarketFee>) {
    let Some(s) = l.steps.get(step) else {
        return (None, None);
    };
    if s.fee == 0 {
        return (None, None);
    }
    let percent_text = purchase::fee_percent_text(s.fee, s.price);
    let line = |published: bool, payable: bool, warning: Option<&str>| MarketFeeLine {
        value_doos: s.fee,
        percent_text: percent_text.clone(),
        published,
        payable,
        warning: warning.map(str::to_owned),
    };
    let Some(address) = l.fee_output_address(network) else {
        return (
            Some(line(
                false,
                false,
                Some("the listing names no valid fee address, so no market fee is paid"),
            )),
            None,
        );
    };
    if s.fee < DUST_THRESHOLD {
        return (
            Some(line(
                false,
                false,
                Some("the market fee is below the dust limit, so it is not paid"),
            )),
            None,
        );
    }
    let warning = (!published).then_some(UNPUBLISHED_FEE_WARNING);
    let pay = pay_market_fee.then_some(MarketFee {
        address,
        value: s.fee,
    });
    (Some(line(published, true, warning)), pay)
}

/// Everything a purchase preview and draft are built from, after every gate.
struct Prepared {
    ctx: Ctx,
    listing: ListingFile,
    buyable: Buyable,
    fee_line: Option<MarketFeeLine>,
    market_fee: Option<MarketFee>,
}

/// Why a market link cannot be imported off mainnet. The UI disables link
/// import with the same words (`marketText.ts`).
pub const MARKET_MAINNET_ONLY: &str = "LearnHNS Market lists mainnet names only";

/// Why a Shakedex draft is refused on a node that cannot send (R6). The UI
/// shows the same sentence on the disabled Buy and Finalize
/// (`marketText.ts::NEEDS_SENDING_NODE`).
pub const NEEDS_SENDING_NODE: &str =
    "Shakedex needs a local node, or a remote node with sending allowed";

/// The gates every Shakedex money draft passes (R16, R6): a seed-backed
/// software profile and a node that can send. The profile kind is checked
/// before `load_ctx`, whose own watch-only refusal words it differently from
/// the sentence the UI shows.
fn software_writer_ctx(state: &State<'_, AppState>) -> Result<Ctx, AppError> {
    let kind = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        draft_ctx::active_profile(&conn)?.kind
    };
    if kind != "mnemonic_hot" {
        return Err(AppError::InvalidInput(
            crate::noncustodial::shakedex::RECOVERY_PHRASE_ONLY.into(),
        ));
    }
    let ctx = draft_ctx::load_ctx(state)?;
    let cap = WriteCapability::evaluate(
        true,
        ctx.node.source(),
        rpc::remote_broadcast_allowed(&ctx.settings),
    );
    if !cap.broadcaster_available {
        return Err(AppError::InvalidInput(NEEDS_SENDING_NODE.into()));
    }
    Ok(ctx)
}

/// R15/R29: a new purchase or a new listing on mainnet needs the
/// `shakedex_experimental` setting; testnet and regtest need nothing.
/// Actions on what already exists never ask.
fn new_trade_allowed(ctx: &Ctx) -> bool {
    ctx.network != Network::Main
        || ctx
            .settings
            .get("shakedex_experimental")
            .map(String::as_str)
            == Some("true")
}

/// Run the purchase gates in order, verify the listing on the node and
/// decide the market fee. `for_draft` also enforces the mainnet experimental
/// flag, which gates only new purchase drafts (R15).
async fn prepare(
    state: &State<'_, AppState>,
    listing_json: &str,
    pay_market_fee: bool,
    from_market: bool,
    for_draft: bool,
) -> Result<Prepared, AppError> {
    let ctx = software_writer_ctx(state)?;
    if for_draft && !new_trade_allowed(&ctx) {
        return Err(AppError::InvalidInput(
            crate::noncustodial::shakedex::MAINNET_EXPERIMENTAL.into(),
        ));
    }
    let listing = ListingFile::parse(listing_json, ctx.network)?;
    let buyable = match verify::verify_listing(&ctx.node, ctx.network, &listing).await {
        Verdict::Buyable(b) => b,
        Verdict::Hidden(h) => return Err(hidden_to_error(h)),
    };
    let step = &listing.steps[buyable.current_step];
    let published = if from_market && step.fee > 0 {
        // Best-effort: a market that cannot say what it charges makes the fee
        // unpublished, so it is shown with a warning and not paid by default.
        let info: Option<FeeInfo> = learnhns_client(&ctx.settings)?.fee_info().await.ok();
        info.is_some_and(|i| {
            market_fee_is_published(step.fee, listing.fee_addr.as_deref(), step.price, &i)
        })
    } else {
        false
    };
    let (fee_line, market_fee) = decide_market_fee(
        &listing,
        buyable.current_step,
        ctx.network,
        pay_market_fee,
        published,
    );
    Ok(Prepared {
        ctx,
        listing,
        buyable,
        fee_line,
        market_fee,
    })
}

fn plan_purchase(
    p: &Prepared,
    dest_address: &str,
    fee_rate: Option<u64>,
) -> Result<crate::noncustodial::actions::PlanResult, AppError> {
    purchase::build_purchase_plan(&PurchaseInput {
        network: p.ctx.network,
        account: p.ctx.account,
        listing: &p.listing,
        step: p.buyable.current_step,
        lock_value: p.buyable.lock_value,
        name_height: p.buyable.name_height,
        dest: output_address_from_string(p.ctx.network, dest_address)?,
        market_fee: p.market_fee.clone(),
        funding: &p.ctx.funding,
        change_address: &p.ctx.change_address,
        rate: draft_ctx::fee_rate(&p.ctx, fee_rate),
        #[cfg(test)]
        fixed_fee: None,
    })
}

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

fn expiry_warning() -> String {
    "This name expires soon after the purchase: finalize it in time or it is lost.".into()
}

/// R31's verdict on locking a name now, or (T3) on finalizing it into the
/// lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExpiryNotice {
    Ok,
    /// Allowed, with R31's warning: the name expires less than 180 R9 days
    /// after the tip.
    Warn {
        blocks_left: i64,
    },
    /// The name expires at or before `tip + 1 + remaining lockup + day`: it
    /// would expire on a TRANSFER coin before its FINALIZE into the lock, or
    /// within a day of it.
    Refuse {
        expiry_end: i64,
    },
}

/// Six months, in R9 days (R31's warning).
const LOCK_WARN_DAYS: i64 = 180;

/// R31. `name_info` is hsd's `getnameinfo` reply; its renewal height and
/// claimed count are read field by field, and a reply without either is "could
/// not check" (fail closed). `remaining_lockup` is the full transfer lockup at
/// Lock and `NameParams::blocks_until_finalize` of the lock TRANSFER at
/// Finalize & sign.
pub(crate) fn lock_expiry_guard(
    params: &NameParams,
    name_info: &serde_json::Value,
    tip: i64,
    remaining_lockup: i64,
) -> Result<ExpiryNotice, AppError> {
    let info = match name_info.get("info") {
        None => {
            return Err(AppError::Rpc(
                "node did not report the name's info: could not check when it expires".into(),
            ))
        }
        Some(serde_json::Value::Null) => {
            return Err(AppError::InvalidInput(
                "the name has no on-chain state or has expired".into(),
            ))
        }
        Some(i) => i,
    };
    let Some(renewal) = info
        .get("renewal")
        .and_then(serde_json::Value::as_u64)
        .and_then(|r| u32::try_from(r).ok())
    else {
        return Err(AppError::Rpc(
            "node did not report the name's renewal height: could not check when it expires".into(),
        ));
    };
    let Some(claimed) = info.get("claimed").and_then(serde_json::Value::as_u64) else {
        return Err(AppError::Rpc(
            "node did not report whether the name was claimed: could not check when it expires"
                .into(),
        ));
    };
    let end = params.expiry_end(i64::from(renewal), claimed > 0);
    if end <= params.finalize_margin(tip, remaining_lockup) {
        return Ok(ExpiryNotice::Refuse { expiry_end: end });
    }
    let blocks_left = end - tip;
    if blocks_left < LOCK_WARN_DAYS * i64::from(params.margin_day()) {
        Ok(ExpiryNotice::Warn { blocks_left })
    } else {
        Ok(ExpiryNotice::Ok)
    }
}

/// What the day-0 lock draft is built from, every node read and the lock key
/// already in hand.
pub(crate) struct LockDraftInput<'a> {
    pub(crate) ctx: &'a Ctx,
    pub(crate) key: &'a LockKey,
    pub(crate) name: &'a str,
    pub(crate) mode: ListingMode,
    pub(crate) publish: bool,
    /// Our owner coin of the name.
    pub(crate) owner: &'a NameCoin,
    pub(crate) name_height: u32,
    /// R31's verdict at the node's tip.
    pub(crate) notice: ExpiryNotice,
    pub(crate) rate: u64,
}

fn expires_before_the_lock(name: &str, expiry_end: i64) -> AppError {
    AppError::InvalidInput(format!(
        "'{name}' expires at block {expiry_end}, before its transfer into the lock could be \
         finalized: renew it first"
    ))
}

/// Day 0 (R18, R19, R21, R31): check the lock key, build the TRANSFER that
/// commits the name to its lock address, and write the draft and the
/// Locking listing with its reserved payment and cancel addresses in one
/// database transaction. Every refusal comes before any write.
pub(crate) fn build_lock_draft_inner(
    conn: &rusqlite::Connection,
    i: &LockDraftInput,
) -> Result<TxDraftSummary, AppError> {
    let ctx = i.ctx;
    if let ExpiryNotice::Refuse { expiry_end } = i.notice {
        return Err(expires_before_the_lock(i.name, expiry_end));
    }
    // hsd lets REGISTER, UPDATE, RENEW and FINALIZE go to a TRANSFER
    // (`rules.verifyCovenants`); a TRANSFER coin cannot.
    let t = i.owner.covenant_type;
    if t == i64::from(COV_TRANSFER) {
        return Err(AppError::InvalidInput(format!(
            "a transfer of '{}' is pending: cancel or finalize it before locking the name",
            i.name
        )));
    }
    if ![COV_REGISTER, COV_UPDATE, COV_RENEW, COV_FINALIZE]
        .iter()
        .any(|c| i64::from(*c) == t)
    {
        return Err(AppError::InvalidInput(format!(
            "'{}' is not registered to this wallet",
            i.name
        )));
    }
    if queries::open_shakedex_listing_for_name(conn, &ctx.profile_id, i.name)?.is_some() {
        return Err(AppError::InvalidInput(format!(
            "'{}' is already locked for sale",
            i.name
        )));
    }
    sell::lock_self_check(i.key, ctx.network)?;

    let res = actions::build_plan(
        ctx.network,
        ctx.account,
        Some(draft_ctx::name_input_from(i.owner.clone())),
        PrimaryOutput {
            value: i.owner.value,
            address: i.owner.address.clone(),
            covenant: sell::lock_transfer_covenant(i.name, i.name_height, &i.key.pubkey)?,
        },
        &ctx.funding,
        &ctx.change_address,
        i.rate,
    )?;
    let mut warnings = vec![LOCK_COSTS.to_string()];
    if let ExpiryNotice::Warn { blocks_left } = i.notice {
        warnings.push(sell::near_expiry_warning(blocks_left));
    }
    // The draft, the two reserved addresses and the listing commit together:
    // a lock TRANSFER without its listing could never be followed, and a
    // listing without its draft locks nothing.
    let tx = conn.unchecked_transaction()?;
    let draft_id = draft_ctx::persist_in_tx(
        &tx,
        &ctx.profile_id,
        &draft_ctx::DraftLabel {
            action: LOCK_ACTION,
            name: i.name,
            recipient: Some(&i.key.address),
            name_list: None,
            warnings: &warnings,
        },
        &res,
    )?;
    // Both on the receive branch (R21); the cancel's index rides on its lock
    // input in T5.
    let payment = derivation::reserve_receive_address(&tx, &ctx.profile_id)?;
    let cancel = derivation::reserve_receive_address(&tx, &ctx.profile_id)?;
    queries::insert_shakedex_listing(
        &tx,
        &ShakedexListing {
            id: random_id(),
            wallet_profile_id: ctx.profile_id.clone(),
            name: i.name.into(),
            mode: i.mode,
            state: ListingState::Locking,
            lock_pubkey_hex: hex::encode(i.key.pubkey),
            lock_transfer_draft_id: Some(draft_id.clone()),
            lock_transfer_txid: Some(res.txid.clone()),
            lock_txid: None,
            lock_vout: None,
            payment_address: Some(payment.address),
            cancel_address: Some(cancel.address),
            cancel_child_index: Some(i64::from(cancel.child_index)),
            steps_json: "[]".into(),
            listing_file_json: None,
            publish: i.publish,
            market_status: None,
            market_retry_at: None,
            expires_at: None,
            abort_draft_id: None,
            abort_txid: None,
            sold_txid: None,
            cancel_txid: None,
            created_at: String::new(),
            updated_at: String::new(),
        },
    )?;
    tx.commit()?;
    draft_ctx::draft_summary(conn, &draft_id)
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

/// Price, market fee, network fee and total of buying `listing_json` at its
/// current step, without writing anything.
#[tauri::command]
pub async fn shakedex_preview_purchase(
    state: State<'_, AppState>,
    listing_json: String,
    pay_market_fee: bool,
    from_market: bool,
    fee_rate: Option<u64>,
) -> Result<PurchasePreview, AppError> {
    let p = prepare(&state, &listing_json, pay_market_fee, from_market, false).await?;
    // The change address stands in for the name destination: the same size,
    // and a preview must not allocate a receive address.
    let res = plan_purchase(&p, &p.ctx.change_address, fee_rate)?;
    let price = p.listing.steps[p.buyable.current_step].price;
    let paid_fee = p.market_fee.as_ref().map_or(0, |f| f.value);
    Ok(PurchasePreview {
        name: p.listing.name.clone(),
        price_doos: price,
        market_fee: p.fee_line.clone(),
        network_fee_doos: res.fee,
        total_doos: price + paid_fee + res.fee,
        finalize_wait: purchase::finalize_wait_text(p.ctx.network),
        warn_expiry: p.buyable.warn_expiry,
    })
}

/// Build the purchase draft (action `shakedex_purchase`) at the current step,
/// re-verified on the node right now, and record the purchase.
///
/// `accepted_market_fee_doos` is the market fee the user reviewed and agreed
/// to pay (`None`: pay none). If the fee the current step would pay differs
/// (the step moved, the listing changed), the build is refused so the user
/// reviews it again: consent is bound to the amount seen.
#[tauri::command]
pub async fn shakedex_build_purchase_draft(
    state: State<'_, AppState>,
    listing_json: String,
    accepted_market_fee_doos: Option<u64>,
    from_market: bool,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let p = prepare(
        &state,
        &listing_json,
        accepted_market_fee_doos.is_some(),
        from_market,
        true,
    )
    .await?;
    if accepted_market_fee_doos.is_some()
        && p.market_fee.as_ref().map(|f| f.value) != accepted_market_fee_doos
    {
        return Err(AppError::InvalidInput(
            "the market fee changed since you reviewed it — review the purchase again".into(),
        ));
    }
    let lock_txid = hex::encode(p.listing.lock_txid);
    let lock_vout = i64::from(p.listing.lock_vout);
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    if queries::list_open_shakedex_purchases(&conn, &p.ctx.profile_id)?
        .iter()
        .any(|o| {
            o.lock_txid == lock_txid
                && o.lock_vout == lock_vout
                && matches!(
                    o.state,
                    PurchaseState::PendingSend | PurchaseState::Unconfirmed
                )
        })
    {
        return Err(AppError::InvalidInput(
            "a purchase of this listing is already in progress".into(),
        ));
    }
    let dest = derivation::next_unused_receive_address(
        &conn,
        &p.ctx.profile_id,
        p.ctx.account,
        p.ctx.network,
        &p.ctx.account_xpub,
    )?;
    let res = plan_purchase(&p, &dest.address, fee_rate)?;
    let price = p.listing.steps[p.buyable.current_step].price;
    let market_fee = p.market_fee.as_ref().map_or(0, |f| f.value);
    let send_total = price + market_fee;
    let mut warnings: Vec<String> = p
        .buyable
        .warn_expiry
        .then(expiry_warning)
        .into_iter()
        .collect();
    let fee_published = p.fee_line.as_ref().is_some_and(|l| l.published);
    if p.market_fee.is_some() && !fee_published {
        warnings.push(format!("Market fee: {UNPUBLISHED_FEE_WARNING}"));
    }
    let market_fee_address = p.market_fee.as_ref().and(p.listing.fee_addr.as_deref());
    let summary = PurchaseSummary {
        action: PURCHASE_ACTION.into(),
        name: p.listing.name.clone(),
        price_doos: price,
        market_fee_doos: market_fee,
        market_fee_address: market_fee_address.map(str::to_owned),
        fee_doos: res.fee,
        total_doos: send_total + res.fee,
        send_total_doos: send_total,
        change_doos: res.change,
        input_total_doos: res.input_total - p.buyable.lock_value,
        num_inputs: res.plan.inputs.len(),
        recipient_address: p.listing.payment_addr.clone(),
        payment_address: p.listing.payment_addr.clone(),
        destination_address: dest.address.clone(),
        finalize_wait: purchase::finalize_wait_text(p.ctx.network),
        txid: res.txid.clone(),
        warnings,
    };
    let draft_id = random_id();
    // The draft and its purchase row commit together or not at all: a draft
    // without its row would send money that sync could never attribute to a
    // purchase.
    let tx = conn.unchecked_transaction()?;
    queries::insert_tx_draft_reserving_coins_in_tx(
        &tx,
        &draft_id,
        &p.ctx.profile_id,
        PURCHASE_ACTION,
        &res.unsigned_tx_hex,
        &serde_json::to_string(&res.plan)?,
        &serde_json::to_string(&summary)?,
        &res.plan.own_inputs(),
    )?;
    queries::insert_shakedex_purchase(
        &tx,
        &ShakedexPurchase {
            id: random_id(),
            wallet_profile_id: p.ctx.profile_id.clone(),
            name: p.listing.name.clone(),
            listing_json: listing_json.clone(),
            lock_txid,
            lock_vout,
            price_doos: price as i64,
            purchase_draft_id: draft_id.clone(),
            purchase_txid: res.txid.clone(),
            destination_address: dest.address.clone(),
            state: PurchaseState::PendingSend,
            purchase_height: None,
            blocks_remaining: None,
            missing_since_height: None,
            rebroadcast_count: 0,
            lost_reason: None,
            finalize_draft_id: None,
            created_at: String::new(),
            updated_at: String::new(),
        },
    )?;
    tx.commit()?;
    queries::get_tx_draft(&conn, &draft_id)?
        .map(|d| d.to_summary())
        .ok_or_else(|| AppError::Other("draft vanished after insert".into()))
}

/// Build the draft (action `shakedex_purchase_finalize`) that finalizes a
/// purchased name out of the lock to the purchase's reserved destination, and
/// record it on the purchase. Not gated by `shakedex_experimental`: finishing
/// a purchase already made always works (R15).
///
/// Everything is re-checked on the node right now rather than taken from the
/// purchase row: the TRANSFER at `purchase_txid:0` is still unspent, sits at
/// the lock address and commits to our destination (R13), and the transfer
/// lockup is over at the live tip.
#[tauri::command]
pub async fn shakedex_build_purchase_finalize_draft(
    state: State<'_, AppState>,
    purchase_id: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let mut ctx = software_writer_ctx(&state)?;
    let (p, replaces) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let p = queries::get_shakedex_purchase(&conn, &purchase_id)?
            .filter(|p| p.wallet_profile_id == ctx.profile_id)
            .ok_or_else(|| AppError::NotFound(format!("purchase {purchase_id}")))?;
        if p.state != PurchaseState::AwaitingFinalize {
            return Err(AppError::InvalidInput(
                "this purchase is not awaiting finalize".into(),
            ));
        }
        // One finalize per purchase: an earlier one already sent stands; an
        // unsent one is replaced, and the coins it reserved fund the new one.
        // A failed or dropped one is left as history; its coins are free.
        let old_status = match p.finalize_draft_id.as_deref() {
            Some(old) => queries::get_tx_draft(&conn, old)?.map(|d| d.status),
            None => None,
        };
        let replaces = match old_status.as_deref() {
            Some(s) if queries::may_have_reached_chain(s) => {
                return Err(AppError::InvalidInput(
                    "a finalize of this purchase is already sent".into(),
                ));
            }
            Some("draft" | "signed") => p.finalize_draft_id.clone(),
            _ => None,
        };
        if let Some(old) = replaces.as_deref() {
            ctx.funding =
                send::load_spendable_coins(&conn, &ctx.profile_id, Some(old), ctx.network)?;
        }
        (p, replaces)
    };
    let listing = ListingFile::parse(&p.listing_json, ctx.network)?;

    let transfer = ctx
        .node
        .get_coin(&p.purchase_txid, 0)
        .await?
        .ok_or_else(|| {
            AppError::InvalidInput(
                "the purchase's transfer is no longer unspent: the name may already be finalized"
                    .into(),
            )
        })?;
    if !purchase::transfer_commits_to(&transfer, ctx.network, &p.destination_address)? {
        return Err(AppError::InvalidInput(
            "the purchase's transfer does not commit to your address: it cannot be finalized"
                .into(),
        ));
    }
    // A reply without the address is not hsd's answer (`Coin.getJSON` always
    // sends one), so it is not read as "somewhere else" either.
    let transfer_address = transfer.address.as_deref().ok_or_else(|| {
        AppError::Rpc(format!(
            "node did not report the address of coin {}:0",
            p.purchase_txid
        ))
    })?;
    if transfer_address != lock_address(ctx.network, &listing.public_key)? {
        return Err(AppError::InvalidInput(
            "the purchase's transfer is not at the listing's lock address".into(),
        ));
    }
    let transfer_height = transfer
        .mined_height()?
        .ok_or_else(|| AppError::InvalidInput("the purchase is not mined yet".into()))?;
    let transfer_value = u64::try_from(transfer.value)
        .map_err(|_| AppError::Rpc(format!("bad transfer value {}", transfer.value)))?;
    let tip = ctx.node.get_blockchain_info().await?.blocks;
    let remaining = ctx
        .network
        .name_params()
        .blocks_until_finalize(transfer_height, tip);
    if remaining > 0 {
        return Err(AppError::InvalidInput(format!(
            "the purchase can be finalized in {remaining} block{}",
            if remaining == 1 { "" } else { "s" }
        )));
    }
    let ns = draft_ctx::fetch_name_state_strict(&ctx.node, &p.name).await?;
    // hsd links a FINALIZE to its TRANSFER only at the same name height
    // (rules.js, TRANSFER → FINALIZE). A different height is a name that
    // expired and was registered again since: the FINALIZE would be refused
    // only after the user signed it.
    let transfer_name_height = transfer
        .covenant
        .as_ref()
        .and_then(purchase::covenant_name_height)
        .ok_or_else(|| {
            AppError::Rpc("node did not report a readable height in the transfer's covenant".into())
        })?;
    if transfer_name_height != ns.height {
        return Err(AppError::InvalidInput(
            "the name expired and was registered again since the purchase: it can no longer \
             be finalized"
                .into(),
        ));
    }
    let renewal_block = draft_ctx::renewal_block(&ctx.node, ctx.network).await?;
    let mut transfer_txid = [0u8; 32];
    hex::decode_to_slice(&p.purchase_txid, &mut transfer_txid)
        .map_err(|e| AppError::Other(format!("purchase txid is not hex: {e}")))?;
    let res = purchase::build_purchase_finalize_plan(&FinalizeInput {
        network: ctx.network,
        account: ctx.account,
        transfer_outpoint: (transfer_txid, 0),
        transfer_value,
        lock_pubkey: listing.public_key,
        name: &p.name,
        name_height: ns.height,
        weak: ns.weak,
        claimed: ns.claimed,
        renewals: ns.renewals,
        renewal_block,
        dest_address: &p.destination_address,
        funding: &ctx.funding,
        change_address: &ctx.change_address,
        rate: draft_ctx::fee_rate(&ctx, fee_rate),
        #[cfg(test)]
        fixed_fee: None,
    })?;

    let summary = PurchaseFinalizeSummary {
        action: PURCHASE_FINALIZE_ACTION.into(),
        name: p.name.clone(),
        purchase_id: p.id.clone(),
        send_total_doos: 0,
        fee_doos: res.fee,
        total_doos: res.fee,
        change_doos: res.change,
        input_total_doos: res.input_total - transfer_value,
        num_inputs: res.plan.inputs.len(),
        recipient_address: p.destination_address.clone(),
        destination_address: p.destination_address.clone(),
        txid: res.txid.clone(),
        warnings: vec![DNS_RECORDS_HINT.into()],
    };

    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let draft_id = random_id();
    // The replaced draft, the new draft and its link on the purchase commit
    // together. Deleting the old draft refuses one that was sent meanwhile.
    let tx = conn.unchecked_transaction()?;
    // Another build of this purchase may have linked its own draft while this
    // one waited on the node: linking ours over it would orphan that draft
    // and the coins it reserved.
    let linked = queries::get_shakedex_purchase(&tx, &p.id)?.and_then(|q| q.finalize_draft_id);
    if linked != p.finalize_draft_id {
        return Err(AppError::InvalidInput(
            "a finalize of this purchase was prepared meanwhile — try again".into(),
        ));
    }
    if let Some(old) = replaces.as_deref() {
        queries::delete_tx_draft_in_tx(&tx, old)?;
    }
    queries::insert_tx_draft_reserving_coins_in_tx(
        &tx,
        &draft_id,
        &ctx.profile_id,
        PURCHASE_FINALIZE_ACTION,
        &res.unsigned_tx_hex,
        &serde_json::to_string(&res.plan)?,
        &serde_json::to_string(&summary)?,
        &res.plan.own_inputs(),
    )?;
    queries::set_shakedex_purchase_finalize_draft(&tx, &p.id, &draft_id)?;
    tx.commit()?;
    queries::get_tx_draft(&conn, &draft_id)?
        .map(|d| d.to_summary())
        .ok_or_else(|| AppError::Other("draft vanished after insert".into()))
}

/// Lock a name for sale, day 0 (R19): the TRANSFER committing it to its lock
/// address (action `shakedex_lock`), and the Locking listing. Prices are
/// chosen at Finalize & sign (T3); day 0 records the mode only. Needs the
/// unlocked signer: the lock key is derived from the seed to read its public
/// key and run the R18 self-check, and is not kept.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn shakedex_build_lock_draft(
    state: State<'_, AppState>,
    name: String,
    mode: ListingMode,
    publish: bool,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = software_writer_ctx(&state)?;
    if !new_trade_allowed(&ctx) {
        return Err(AppError::InvalidInput(
            crate::noncustodial::shakedex::MAINNET_SELLING_EXPERIMENTAL.into(),
        ));
    }
    if publish && ctx.network != Network::Main {
        return Err(AppError::InvalidInput(MARKET_MAINNET_ONLY.into()));
    }
    // The signer is checked before anything else is read; the key itself is
    // derived only once every read is done, just before it is used.
    {
        let mut slot = state
            .signer
            .lock()
            .map_err(|e| AppError::Lock(e.to_string()))?;
        let session = slot.as_mut().ok_or(AppError::WalletLocked)?;
        session.authorize(&ctx.profile_id, session_ttl_ms(&ctx.settings))?;
    }
    let owner = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        queries::get_name_coin(&conn, &ctx.profile_id, &name)?
            .ok_or_else(|| AppError::NotFound(format!("wallet does not hold '{name}' (sync?)")))?
    };
    // R31 and the covenant read one `getnameinfo` reply, and the tip comes
    // from the node's own report of it: either missing is "could not check".
    let reply = ctx.node.get_name_info(&name).await?;
    let tip = ctx
        .node
        .get_blockchain_info()
        .await
        .map_err(|e| {
            AppError::Rpc(format!(
                "node did not report its tip: could not check when '{name}' expires ({e})"
            ))
        })?
        .blocks;
    let params = ctx.network.name_params();
    let notice = lock_expiry_guard(&params, &reply, tip, i64::from(params.transfer_lockup))?;
    let ns = draft_ctx::name_state_strict(&reply, &name)?;
    let rate = draft_ctx::fee_rate(&ctx, fee_rate);
    let key = {
        let mut slot = state
            .signer
            .lock()
            .map_err(|e| AppError::Lock(e.to_string()))?;
        let session = slot.as_mut().ok_or(AppError::WalletLocked)?;
        session.authorize(&ctx.profile_id, session_ttl_ms(&ctx.settings))?;
        derive_lock_key(session.master()?, ctx.network, ctx.account, &name)?
    };
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    build_lock_draft_inner(
        &conn,
        &LockDraftInput {
            ctx: &ctx,
            key: &key,
            name: &name,
            mode,
            publish,
            owner: &owner,
            name_height: ns.height,
            notice,
            rate,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noncustodial::address;

    const LISTING_FILE: &str = include_str!("../../tests/vectors/shakedex/proof_dexreviews.json");

    fn listing(fee: u64, fee_addr: Option<String>) -> ListingFile {
        let mut j: serde_json::Value = serde_json::from_str(LISTING_FILE).unwrap();
        j["data"][0]["fee"] = fee.into();
        j["feeAddr"] = fee_addr.map_or(serde_json::Value::Null, Into::into);
        ListingFile::parse(&j.to_string(), Network::Main).unwrap()
    }

    fn addr() -> String {
        address::encode_p2wpkh(Network::Main, &[7; 20]).unwrap()
    }

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

    /// The largest fee a listing file can carry (price plus fee up to the
    /// money supply, R2) is past any percentage a u32 of basis points holds.
    #[test]
    fn fee_past_any_percentage_has_no_percent() {
        let max_fee = purchase::MAX_MONEY - 435_000_000;
        let (line, _) = decide_market_fee(
            &listing(max_fee, Some(addr())),
            0,
            Network::Main,
            true,
            true,
        );
        let line = line.unwrap();
        assert_eq!(line.value_doos, max_fee);
        assert_eq!(line.percent_text, None);
    }

    #[test]
    fn zero_fee_has_no_line() {
        let (line, pay) =
            decide_market_fee(&listing(0, Some(addr())), 0, Network::Main, true, true);
        assert!(line.is_none() && pay.is_none());
    }

    #[test]
    fn missing_fee_address_is_never_paid() {
        let (line, pay) = decide_market_fee(&listing(5_000, None), 0, Network::Main, true, true);
        let line = line.unwrap();
        assert!(!line.published);
        assert!(line.warning.unwrap().contains("no valid fee address"));
        assert!(pay.is_none());
    }

    #[test]
    fn dust_fee_is_never_paid() {
        let (line, pay) = decide_market_fee(
            &listing(DUST_THRESHOLD - 1, Some(addr())),
            0,
            Network::Main,
            true,
            true,
        );
        assert!(line.unwrap().warning.unwrap().contains("dust"));
        assert!(pay.is_none());
    }

    #[test]
    fn fee_follows_the_users_choice() {
        let l = listing(4_350_000, Some(addr()));
        let (line, pay) = decide_market_fee(&l, 0, Network::Main, false, true);
        assert!(line.as_ref().unwrap().published);
        assert_eq!(line.unwrap().percent_text.as_deref(), Some("1.00%"));
        assert!(pay.is_none());
        let (line, pay) = decide_market_fee(&l, 0, Network::Main, true, false);
        assert_eq!(
            line.unwrap().warning.as_deref(),
            Some(UNPUBLISHED_FEE_WARNING)
        );
        assert_eq!(pay.unwrap().value, 4_350_000);
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

    #[test]
    fn hidden_errors_carry_the_reason() {
        let e = hidden_to_error(Hidden::CouldNotCheck {
            reason: "node did not report median time".into(),
        });
        assert!(e.to_string().contains("median time"));
        let e = hidden_to_error(Hidden::NotYetValid {
            first_valid_in_secs: 61,
        });
        assert!(e.to_string().contains("2 minutes"));
    }

    fn info(renewal: u64, claimed: u64) -> serde_json::Value {
        serde_json::json!({ "info": { "renewal": renewal, "claimed": claimed } })
    }

    /// R31 at Lock, both sides of the refusal block on mainnet and regtest:
    /// refused while the expiry end is at or below tip + 1 + lockup + day,
    /// allowed (with the warning, being near) one block earlier.
    #[test]
    fn lock_expiry_guard_refuses_at_the_margin_and_locks_one_block_earlier() {
        for (net, lockup, day) in [(Network::Main, 288i64, 144i64), (Network::Regtest, 10, 10)] {
            let p = net.name_params();
            assert_eq!(i64::from(p.transfer_lockup), lockup, "{net:?}");
            let end = 1_000 + i64::from(p.renewal_window);
            let refused_tip = end - 1 - lockup - day;
            assert_eq!(
                lock_expiry_guard(&p, &info(1_000, 0), refused_tip, lockup).unwrap(),
                ExpiryNotice::Refuse { expiry_end: end },
                "{net:?}"
            );
            assert_eq!(
                lock_expiry_guard(&p, &info(1_000, 0), refused_tip + 1, lockup).unwrap(),
                ExpiryNotice::Refuse { expiry_end: end },
                "{net:?}: past the margin"
            );
            assert_eq!(
                lock_expiry_guard(&p, &info(1_000, 0), refused_tip - 1, lockup).unwrap(),
                ExpiryNotice::Warn {
                    blocks_left: 1 + 1 + lockup + day
                },
                "{net:?}"
            );
        }
    }

    /// Finalize & sign (T3) passes what is left of the lockup, 0 once it is
    /// over: no second lockup is required there.
    #[test]
    fn lock_expiry_guard_counts_only_the_remaining_lockup() {
        let p = Network::Regtest.name_params();
        let end: i64 = 100 + 5_000;
        assert!(matches!(
            lock_expiry_guard(&p, &info(100, 0), end - 1 - 10, 0).unwrap(),
            ExpiryNotice::Refuse { .. }
        ));
        assert!(matches!(
            lock_expiry_guard(&p, &info(100, 0), end - 1 - 10 - 1, 0).unwrap(),
            ExpiryNotice::Warn { .. }
        ));
    }

    /// Six months is 180 R9 days from the tip: 25 920 blocks on mainnet, 1800
    /// on regtest. The last tip that warns and the first that does not, and
    /// the warning never carries a non-positive block count.
    #[test]
    fn lock_expiry_guard_warns_below_180_days() {
        for (net, day) in [(Network::Main, 144i64), (Network::Regtest, 10)] {
            let p = net.name_params();
            let lockup = i64::from(p.transfer_lockup);
            let end = 1_000 + i64::from(p.renewal_window);
            assert_eq!(
                lock_expiry_guard(&p, &info(1_000, 0), end - 180 * day, lockup).unwrap(),
                ExpiryNotice::Ok,
                "{net:?}"
            );
            assert_eq!(
                lock_expiry_guard(&p, &info(1_000, 0), end - 180 * day + 1, lockup).unwrap(),
                ExpiryNotice::Warn {
                    blocks_left: 180 * day - 1
                },
                "{net:?}"
            );
            for tip in (end - 180 * day..end + 5).step_by(7) {
                if let ExpiryNotice::Warn { blocks_left } =
                    lock_expiry_guard(&p, &info(1_000, 0), tip, lockup).unwrap()
                {
                    assert!(blocks_left > 0, "{net:?} tip {tip}");
                }
            }
        }
    }

    /// A claimed name does not expire before the claim period (regtest
    /// 250 000), however old its renewal.
    #[test]
    fn lock_expiry_guard_reads_the_claim_period() {
        let p = Network::Regtest.name_params();
        assert_eq!(
            lock_expiry_guard(&p, &info(100, 1), 6_000, 10).unwrap(),
            ExpiryNotice::Ok
        );
        assert!(matches!(
            lock_expiry_guard(&p, &info(100, 0), 6_000, 10).unwrap(),
            ExpiryNotice::Refuse { .. }
        ));
    }

    /// Fail closed: a reply without the renewal height or the claimed count
    /// is "could not check", never a guess.
    #[test]
    fn lock_expiry_guard_cannot_check_without_renewal_or_claimed() {
        let p = Network::Regtest.name_params();
        for bad in [
            serde_json::json!({ "info": { "claimed": 0 } }),
            serde_json::json!({ "info": { "renewal": 100 } }),
            serde_json::json!({}),
        ] {
            let e = lock_expiry_guard(&p, &bad, 1_000, 10).unwrap_err();
            assert!(matches!(e, AppError::Rpc(_)), "{bad}: {e:?}");
        }
        let e = lock_expiry_guard(&p, &serde_json::json!({ "info": null }), 1_000, 10).unwrap_err();
        assert!(
            e.to_string().contains("no on-chain state or has expired"),
            "{e}"
        );
    }
}
