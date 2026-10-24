//! Buying names from Shakedex listings: browse LearnHNS Market, import a
//! listing (file, pasted JSON or a LearnHNS link), preview a purchase and
//! build its draft. The draft then goes through the usual secure confirm,
//! sign and broadcast commands in `commands::tx`.
//!
//! Every listing, whatever its source, is verified on the profile's own node
//! (`verify::verify_listing`) before it can be bought; the market's word is
//! never taken for availability, price or fee.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::State;

use crate::commands::draft_ctx::{self, random_id, Ctx};
use crate::db::queries::{
    self, DraftStatus, ListingMode, ListingState, MarketStatus, NameCoin, PurchaseState,
    ShakedexListing, ShakedexPurchase,
};
use crate::error::AppError;
use crate::market::learnhns::{
    market_fee_is_published, name_from_listing_link, FeeInfo, LearnHnsClient,
};
use crate::models::settings::SettingsMap;
use crate::noncustodial::actions::{self, PrimaryOutput};
use crate::noncustodial::derivation;
use crate::noncustodial::network::Network;
use crate::noncustodial::rpc::{self, ChainSource, NodeRpcClient};
use crate::noncustodial::send::{self, SpendableCoin, DUST_THRESHOLD};
use crate::noncustodial::session::{session_ttl_ms, SignerSession};
use crate::noncustodial::shakedex::cancel;
use crate::noncustodial::shakedex::listing_file::{
    add_step_to_listing_file, write_listing_file, ListingFile, NewListingFile, PriceStep,
    MAX_LISTING_FILE_BYTES,
};
use crate::noncustodial::shakedex::lock_key::{derive_lock_key, LockKey};
use crate::noncustodial::shakedex::purchase::{
    self, FinalizeInput, MarketFee, PurchaseFinalizeSummary, PurchaseInput, PurchaseSummary,
    PURCHASE_ACTION, PURCHASE_FINALIZE_ACTION,
};
use crate::noncustodial::shakedex::script::lock_address;
use crate::noncustodial::shakedex::sell::{self, expires_before_the_lock, LOCK_ACTION, LOCK_COSTS};
pub(crate) use crate::noncustodial::shakedex::sell::{lock_expiry_guard, ExpiryNotice};
use crate::noncustodial::shakedex::template;
use crate::noncustodial::shakedex::verify::{self, Buyable, Hidden, Verdict};
use crate::noncustodial::sync::{COV_FINALIZE, COV_REGISTER, COV_RENEW, COV_TRANSFER, COV_UPDATE};
use crate::noncustodial::tx::output_address_from_string;
use crate::noncustodial::types::TxDraftSummary;
use crate::providers::signer::WriteCapability;
use crate::shakedex_jobs;
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

/// A listing whose price steps share one lock time is Buy Now (a lowered
/// one included); steps at different times are a reverse auction
/// ([`sell::mode_of_step_times`]).
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
    /// The listing is locked to one of this device's own lock addresses (R33):
    /// its Buy shows disabled, with [`OWN_LISTING`].
    pub own: bool,
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
    /// The listing file as the market sent it, so a row that could not be
    /// checked can be checked again; `None` when it did not parse.
    pub listing_json: Option<String>,
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
    /// The network's transfer lockup, which the purchase waits out.
    pub lockup_blocks: u32,
    /// Whether that wait may be shown as a time
    /// (`Network::has_wall_clock_block_timing`).
    pub wall_clock_timing: bool,
    pub warn_expiry: bool,
}

// --- helpers ----------------------------------------------------------------

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
    let id = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        draft_ctx::active_profile(&conn)?.id
    };
    browse_ctx_for(state, &id)
}

/// The profile a read names when it exists, else the active one, as the
/// read commands resolve it (`commands/read.rs::resolve_profile`; this module
/// does not reach into its siblings).
fn named_or_active_profile(
    conn: &rusqlite::Connection,
    wallet_profile_id: Option<String>,
) -> Result<String, AppError> {
    if let Some(id) = wallet_profile_id.map(|id| id.trim().to_string()) {
        if !id.is_empty() && queries::get_wallet_profile(conn, &id)?.is_some() {
            return Ok(id);
        }
    }
    Ok(draft_ctx::active_profile(conn)?.id)
}

/// [`browse_ctx`] for the profile `profile_id`: its network and its node.
fn browse_ctx_for(state: &State<'_, AppState>, profile_id: &str) -> Result<BrowseCtx, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    Ok(BrowseCtx {
        network: queries::profile_network(&conn, profile_id)?,
        node: NodeRpcClient::for_profile(&conn, profile_id)?,
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
        kind: match sell::mode_of_step_times(l.steps.iter().map(|s| s.lock_time)) {
            ListingMode::BuyNow => ListingKind::BuyNow,
            ListingMode::ReverseAuction => ListingKind::ReverseAuction,
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
        own: false,
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

/// Why a market link cannot be imported off mainnet (and why every market
/// write refuses off mainnet); defined next to the client.
pub use crate::market::learnhns::MARKET_MAINNET_ONLY;

/// Why a Shakedex draft is refused on a node that cannot send (R6). The UI
/// shows the same sentence on the disabled Buy and Finalize
/// (`marketText.ts::NEEDS_SENDING_NODE`).
pub const NEEDS_SENDING_NODE: &str =
    "Shakedex needs a local node, or a remote node with sending allowed";

/// The gates every Shakedex money draft passes (R16, R6): a seed-backed
/// software profile and a node that can send. The profile kind is checked
/// before `load_ctx`, whose own watch-only refusal words it differently from
/// the sentence the UI shows.
pub(crate) fn software_writer_ctx(state: &State<'_, AppState>) -> Result<Ctx, AppError> {
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

/// What buying a listing locked by this device is refused with (R33). The
/// Market row of such a listing shows its Buy disabled with the same
/// sentence (`marketText.ts::OWN_LISTING`).
pub const OWN_LISTING: &str = "this is your own listing: cancel it instead (My listings), \
                           buying it would only cost you network fees";

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

/// R33: a listing locked to this device's own lock address is not bought.
/// The TRANSFER would commit to an address of ours, so the buyer would pay
/// the fees of a purchase and a FINALIZE for a name that comes home anyway,
/// and the cancel path would offer a second FINALIZE of the same TRANSFER.
fn refuse_own_listing(
    state: &State<'_, AppState>,
    ctx: &Ctx,
    listing: &ListingFile,
) -> Result<(), AppError> {
    let own = own_lock_addresses(state, ctx.network)?;
    if own.contains(&lock_address(ctx.network, &listing.public_key)?) {
        return Err(AppError::InvalidInput(OWN_LISTING.into()));
    }
    Ok(())
}

/// The lock addresses, on `network`, of every listing row on this device
/// (any profile, any state).
fn own_lock_addresses(
    state: &State<'_, AppState>,
    network: Network,
) -> Result<std::collections::HashSet<String>, AppError> {
    let keys = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        queries::shakedex_lock_pubkeys(&conn)?
    };
    Ok(keys
        .iter()
        .filter_map(|hex_key| {
            // A stored key that does not decode is skipped: it can name no
            // address. This fails OPEN: a listing locked with that key is not
            // recognised as ours and may be bought (R33's gate is a fee
            // saver, not a safety check; the name comes home either way).
            // Every row's key is written from one this wallet derived or a
            // file it parsed, so an undecodable one is a corrupted row.
            let key = <[u8; 33]>::try_from(hex::decode(hex_key).ok()?).ok()?;
            lock_address(network, &key).ok()
        })
        .collect())
}

/// `row` marked as ours (R33) when its lock address is in `own`.
fn mark_own(
    mut row: MarketRow,
    l: &ListingFile,
    network: Network,
    own: &std::collections::HashSet<String>,
) -> MarketRow {
    // Why `is_ok_and`: `lock_address` only hashes the parsed 33-byte key and
    // encodes the program, so it does not fail here; if it ever did, the row
    // is shown as anyone's and the purchase gate (`refuse_own_listing`)
    // returns that error instead of building.
    row.own = lock_address(network, &l.public_key).is_ok_and(|a| own.contains(&a));
    row
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
    refuse_own_listing(state, &ctx, &listing)?;
    let buyable = match verify::verify_listing(&ctx.node, ctx.network, &listing).await {
        Verdict::Buyable(b) => b,
        Verdict::Hidden(h) => return Err(hidden_to_error(h)),
    };
    let step = &listing.steps[buyable.current_step];
    let published = if from_market && step.fee > 0 {
        // Best-effort: a market that cannot say what it charges makes the fee
        // unpublished, so it is shown with a warning and not paid by default.
        let info: Option<FeeInfo> = LearnHnsClient::from_settings(&ctx.settings)?
            .fee_info()
            .await
            .ok();
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
        // A Locking listing whose lock draft is dead holds the name's one
        // open listing until the next sync ends it from the chain (R19); no
        // state is written here without that evidence.
        let dead =
            queries::listing_blocking_owner_actions(conn, &ctx.profile_id, i.name)?.is_none();
        return Err(AppError::InvalidInput(if dead {
            format!(
                "an earlier lock of '{}' did not go through: the next sync ends it, then the \
                 name can be locked again",
                i.name
            )
        } else {
            format!("'{}' is already locked for sale", i.name)
        }));
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
            lock_finalize_draft_id: None,
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
            market_attempts: 0,
            market_error: None,
            market_accepted: false,
            market_changed: false,
            market_told: false,
            market_sent_step: None,
            expires_at: None,
            abort_draft_id: None,
            abort_txid: None,
            sold_txid: None,
            cancel_txid: None,
            cancel_draft_id: None,
            cancel_vout: None,
            cancel_finalize_draft_id: None,
            cancel_blocks_remaining: None,
            managed_elsewhere_at: None,
            foreign_step_json: None,
            created_at: String::new(),
            updated_at: String::new(),
        },
    )?;
    tx.commit()?;
    draft_ctx::draft_summary(conn, &draft_id)
}

/// Run `f` on the unlocked signer session of the active profile, authorized
/// again for it: `WalletLocked` when no session is open. The one place the
/// selling commands take the signer, so every use checks it the same way.
fn with_signer<T>(
    state: &State<'_, AppState>,
    ctx: &Ctx,
    f: impl FnOnce(&mut SignerSession) -> Result<T, AppError>,
) -> Result<T, AppError> {
    let mut slot = state
        .signer
        .lock()
        .map_err(|e| AppError::Lock(e.to_string()))?;
    let session = slot.as_mut().ok_or(AppError::WalletLocked)?;
    session.authorize(&ctx.profile_id, session_ttl_ms(&ctx.settings))?;
    f(session)
}

/// The unlocked signer session of the active profile, or `WalletLocked`.
/// Checked before a selling command reads anything else.
pub(crate) fn authorize_signer(state: &State<'_, AppState>, ctx: &Ctx) -> Result<(), AppError> {
    with_signer(state, ctx, |_| Ok(()))
}

/// The lock key of `name` (ADR 0004), derived from the seed of the unlocked
/// session, which is authorized again. Called once every node read is done,
/// just before the key is used; it is never stored.
pub(crate) fn derive_listing_key(
    state: &State<'_, AppState>,
    ctx: &Ctx,
    name: &str,
) -> Result<LockKey, AppError> {
    with_signer(state, ctx, |session| {
        derive_lock_key(session.master()?, ctx.network, ctx.account, name)
    })
}

/// One price step the seller typed (R19): HNS as text, so the backend can
/// refuse what it cannot represent. A Buy Now's one price.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepInput {
    pub price: String,
}

/// A reverse auction's schedule as the seller typed it (R24): the start and
/// floor prices in HNS as text (R19's rule), the number of steps and the
/// seconds between them. Built into steps by `sell::reverse_schedule`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleInput {
    pub start_price: String,
    pub floor_price: String,
    pub steps: u32,
    pub interval_secs: u64,
}

/// R24: why Finalize & sign refuses a reverse auction without its schedule.
pub const REVERSE_NEEDS_SCHEDULE: &str = "a reverse auction needs its schedule: a start \
     price, a floor, the number of steps and the time between them";

/// The pricing of the mode a listing was locked with (R19, R24), parsed and
/// checked before any node read; the lock times wait for the node's median
/// time.
enum Pricing {
    BuyNow(u64),
    Schedule {
        start: u64,
        floor: u64,
        steps: u32,
        interval: u64,
    },
}

/// Everything Finalize & sign has checked and built before it asks (R20).
pub(crate) struct PreparedLockFinalize {
    pub(crate) ctx: Ctx,
    pub(crate) listing: ShakedexListing,
    pub(crate) key: LockKey,
    /// The FINALIZE into the lock; the lock coin is its FINALIZE output
    /// (`sell::lock_output`).
    pub(crate) plan: actions::PlanResult,
    /// `(price, lock time)` of every step about to be signed: a Buy Now's
    /// one, at R19's lock time; a reverse auction's schedule from it (R24).
    pub(crate) steps: Vec<(u64, u64)>,
    pub(crate) mtp: u64,
    pub(crate) payment_address: String,
}

/// "could not check" for a field of the node's reply that Finalize & sign
/// reads.
fn could_not_check(what: &str, name: &str) -> AppError {
    AppError::Rpc(format!(
        "node did not report {what}: could not check '{name}' before Finalize & sign"
    ))
}

/// R18, R19, R31 at Finalize & sign: every gate and node read, then the lock
/// key (after the reads) and the FINALIZE plan. Writes nothing and asks
/// nothing; a refusal leaves the listing ReadyToFinalize. Every node field
/// it reads is matched on its own: a missing one is "could not check".
pub(crate) async fn prepare_lock_finalize(
    state: &State<'_, AppState>,
    listing_id: &str,
    prices: &[StepInput],
    schedule: Option<&ScheduleInput>,
    fee_rate: Option<u64>,
) -> Result<PreparedLockFinalize, AppError> {
    // R16, R6. No experimental flag: this acts on an existing listing (R15).
    let ctx = software_writer_ctx(state)?;
    authorize_signer(state, &ctx)?;
    let (listing, owner) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let listing = queries::get_shakedex_listing(&conn, listing_id)?
            .filter(|l| l.wallet_profile_id == ctx.profile_id)
            .ok_or_else(|| AppError::NotFound(format!("listing {listing_id}")))?;
        if let Some(why) = finalize_and_sign_refusal(&conn, &listing)? {
            return Err(AppError::InvalidInput(why));
        }
        let owner = queries::get_name_coin(&conn, &ctx.profile_id, &listing.name)?;
        (listing, owner)
    };
    // The pricing the mode was locked with (R19, R24), every price parsed
    // and the schedule checked before any node read.
    let pricing = match (listing.mode, schedule) {
        (ListingMode::BuyNow, Some(_)) => {
            return Err(AppError::InvalidInput(
                "a Buy Now listing has one price and no schedule".into(),
            ))
        }
        (ListingMode::BuyNow, None) => {
            if prices.len() != 1 {
                return Err(AppError::InvalidInput(
                    "a Buy Now listing has exactly one price".into(),
                ));
            }
            Pricing::BuyNow(sell::parse_step_price(&prices[0].price)?)
        }
        (ListingMode::ReverseAuction, None) => {
            return Err(AppError::InvalidInput(REVERSE_NEEDS_SCHEDULE.into()))
        }
        (ListingMode::ReverseAuction, Some(_)) if !prices.is_empty() => {
            return Err(AppError::InvalidInput(
                "a reverse auction's prices come from its schedule".into(),
            ))
        }
        (ListingMode::ReverseAuction, Some(s)) => {
            // Every rule before the node is read; the lock times again
            // from its median time.
            let (start, floor) =
                sell::parse_schedule(&s.start_price, &s.floor_price, s.steps, s.interval_secs)?;
            Pricing::Schedule {
                start,
                floor,
                steps: s.steps,
                interval: s.interval_secs,
            }
        }
    };
    let corrupted = |what: &str| AppError::Other(format!("corrupted listing: no {what}"));
    let lock_transfer_txid = listing
        .lock_transfer_txid
        .clone()
        .ok_or_else(|| corrupted("lock transfer"))?;
    let payment_address = listing
        .payment_address
        .clone()
        .ok_or_else(|| corrupted("payment address"))?;
    let owner = owner
        .filter(|o| {
            o.txid == lock_transfer_txid
                && o.vout == 0
                && o.covenant_type == i64::from(COV_TRANSFER)
        })
        .ok_or_else(|| {
            AppError::InvalidInput(
                "the wallet has not seen the lock transfer as the name's coin yet (sync?)".into(),
            )
        })?;

    // One getnameinfo reply and one tip for every check.
    let name = listing.name.clone();
    let reply = ctx.node.get_name_info(&name).await?;
    let chain = ctx
        .node
        .get_blockchain_info()
        .await
        .map_err(|e| could_not_check(&format!("its tip ({e})"), &name))?;
    let tip = chain.blocks;
    let Some(mtp) = chain.mediantime else {
        return Err(AppError::Rpc(
            "node did not report its median time: the price could not be signed".into(),
        ));
    };
    let info = match reply.get("info") {
        None => return Err(could_not_check("the name's info", &name)),
        Some(serde_json::Value::Null) => {
            return Err(AppError::InvalidInput(format!(
                "'{name}' has no on-chain state or has expired"
            )))
        }
        Some(i) => i,
    };
    let Some(owner_hash) = info
        .get("owner")
        .and_then(|o| o.get("hash"))
        .and_then(|h| h.as_str())
    else {
        return Err(could_not_check("the name's owner", &name));
    };
    let Some(owner_index) = info
        .get("owner")
        .and_then(|o| o.get("index"))
        .and_then(|i| i.as_u64())
    else {
        return Err(could_not_check("the name's owner output", &name));
    };
    let Some(revoked) = info.get("revoked").and_then(|r| r.as_u64()) else {
        return Err(could_not_check("whether the name was revoked", &name));
    };
    if !sell::lock_transfer_owns_name(owner_hash, owner_index, revoked, &lock_transfer_txid) {
        return Err(AppError::InvalidInput(format!(
            "'{name}' is no longer held by its lock transfer: there is nothing to finalize"
        )));
    }
    let coin = ctx
        .node
        .get_coin(&lock_transfer_txid, sell::LOCK_TRANSFER_NAME_VOUT)
        .await?
        .ok_or_else(|| {
            AppError::InvalidInput(
                "the lock transfer is no longer unspent: the name may already be finalized".into(),
            )
        })?;
    // The lockup counts from hsd's `info.transfer`, the fact hsd's FINALIZE
    // rule reads, as the sync job does (`sell::transfer_height`).
    let transfer_height = sell::transfer_height(info)
        .ok_or_else(|| could_not_check("the TRANSFER's block", &name))?;
    let params = ctx.network.name_params();
    let remaining = params.blocks_until_finalize(transfer_height, tip);
    if remaining > 0 {
        return Err(AppError::InvalidInput(format!(
            "the transfer lockup ends in {remaining} block{}",
            if remaining == 1 { "" } else { "s" }
        )));
    }
    // R31 with what is left of the lockup (0 here): the FINALIZE renews the
    // name.
    if let ExpiryNotice::Refuse { expiry_end } = lock_expiry_guard(&params, &reply, tip, remaining)?
    {
        return Err(sell::expires_before_finalize(&name, expiry_end));
    }
    let ns = draft_ctx::name_state_strict(&reply, &name)?;
    let cov_height = coin
        .covenant
        .as_ref()
        .and_then(purchase::covenant_name_height)
        .ok_or_else(|| could_not_check("a readable name height in the lock transfer", &name))?;
    if cov_height != ns.height {
        return Err(AppError::InvalidInput(
            "the name expired and was registered again since the lock: it cannot be finalized"
                .into(),
        ));
    }
    let lock_value = u64::try_from(coin.value)
        .map_err(|_| AppError::Rpc(format!("bad coin value {}", coin.value)))?;
    if lock_value != owner.value || coin.address.as_deref() != Some(owner.address.as_str()) {
        return Err(AppError::Rpc(
            "the node and the wallet disagree on the lock transfer coin (sync?)".into(),
        ));
    }
    let renewal_block = draft_ctx::renewal_block(&ctx.node, ctx.network).await?;
    let rate = draft_ctx::fee_rate(&ctx, fee_rate);

    // R18: re-derive the key now, after every read, and check the node's
    // confirmed TRANSFER commits to SHA3-256 of its lock script.
    let key = derive_listing_key(state, &ctx, &name)?;
    sell::lock_self_check(&key, ctx.network)?;
    if hex::encode(key.pubkey) != listing.lock_pubkey_hex
        || !purchase::transfer_commits_to(&coin, ctx.network, &key.address)?
    {
        return Err(AppError::InvalidInput(
            sell::LOCK_COMMITMENT_MISMATCH.into(),
        ));
    }
    let plan = sell::build_lock_finalize_plan(&sell::LockFinalizeInput {
        network: ctx.network,
        account: ctx.account,
        transfer: &SpendableCoin {
            txid: owner.txid.clone(),
            vout: owner.vout,
            value: owner.value,
            branch: owner.branch,
            child_index: owner.child_index,
        },
        lock_pubkey: key.pubkey,
        name: &name,
        name_height: ns.height,
        weak: ns.weak,
        claimed: ns.claimed,
        renewals: ns.renewals,
        renewal_block,
        funding: &ctx.funding,
        change_address: &ctx.change_address,
        rate,
        #[cfg(test)]
        fixed_fee: None,
    })?;
    // R19: the first step valid at once; a schedule's next steps follow it
    // (R24).
    let first = sell::buy_now_lock_time(mtp);
    let steps = match pricing {
        Pricing::BuyNow(price) => vec![(price, first)],
        Pricing::Schedule {
            start,
            floor,
            steps,
            interval,
        } => sell::reverse_schedule(start, floor, steps, interval, first)?,
    };
    Ok(PreparedLockFinalize {
        ctx,
        listing,
        key,
        plan,
        steps,
        mtp,
        payment_address,
    })
}

/// R20: the Finalize & sign confirmation's own title, never the
/// transaction prompt's.
pub const FINALIZE_AND_SIGN_TITLE: &str = "Confirm Finalize & sign";
const FINALIZE_AND_SIGN_MESSAGE: &str = "Review these details. This signs the finalize of \
     the name into its lock and every price below with the lock key.";

/// A listing as the UI reads it ("My listings").
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListingSummary {
    pub id: String,
    pub name: String,
    pub mode: ListingMode,
    pub state: ListingState,
    pub lock_txid: Option<String>,
    pub lock_vout: Option<i64>,
    pub payment_address: Option<String>,
    pub steps: Vec<sell::StoredStep>,
    pub expires_at: Option<i64>,
    /// The FINALIZE into the lock, signed; `broadcast_tx_draft` sends it.
    pub finalize_draft_id: Option<String>,
    /// The cancel's drafts, and blocks left until its FINALIZE, for the
    /// listing's cancel actions (R28).
    pub cancel_draft_id: Option<String>,
    pub cancel_finalize_draft_id: Option<String>,
    pub cancel_blocks_remaining: Option<i64>,
    /// Blocks left in the transfer lockup while Locking, counted as the
    /// Owned Names row counts them ([`queries::locking_blocks_until_finalize`]);
    /// `None` before the lock TRANSFER is seen mined and in every other state.
    pub blocks_until_finalize: Option<i64>,
    /// R35: blocks until buyers stop seeing the listing, because the name
    /// would expire before a purchase could be finalized (R9); 0 once they
    /// no longer see it. Only for a Listed or Restored row of My listings,
    /// counted on an authoritative node; `None` when unknown.
    pub blocks_until_unbuyable: Option<i64>,
    /// Where the listing stands on LearnHNS Market (R23, R25, R28); `None`
    /// while our current listing is not yet told (unpublished, not yet
    /// announced, or reset by a move back to Listed).
    pub market_status: Option<MarketStatus>,
    /// The market's own refusal, or why it gave no answer.
    pub market_error: Option<String>,
    /// RFC 3339 UTC: when the next market action on the listing is due.
    pub market_retry_at: Option<String>,
    /// R23: whether the seller ticked "Publish on LearnHNS Market" (or
    /// published it later from My listings).
    pub publish: bool,
}

impl ListingSummary {
    fn of(conn: &rusqlite::Connection, l: &ShakedexListing) -> Result<Self, AppError> {
        Ok(Self::with_steps(
            l,
            shakedex_jobs::stored_steps(l)?,
            lockup_left(conn, l)?,
        ))
    }

    fn with_steps(
        l: &ShakedexListing,
        steps: Vec<sell::StoredStep>,
        blocks_until_finalize: Option<i64>,
    ) -> Self {
        Self {
            id: l.id.clone(),
            name: l.name.clone(),
            mode: l.mode,
            state: l.state,
            lock_txid: l.lock_txid.clone(),
            lock_vout: l.lock_vout,
            payment_address: l.payment_address.clone(),
            steps,
            expires_at: l.expires_at,
            finalize_draft_id: l.lock_finalize_draft_id.clone(),
            cancel_draft_id: l.cancel_draft_id.clone(),
            cancel_finalize_draft_id: l.cancel_finalize_draft_id.clone(),
            cancel_blocks_remaining: l.cancel_blocks_remaining,
            blocks_until_finalize,
            blocks_until_unbuyable: None,
            market_status: l.market_status,
            market_error: l.market_error.clone(),
            market_retry_at: l.market_retry_at.clone(),
            publish: l.publish,
        }
    }
}

/// The lockup left on a Locking listing `l`, by the rule the Owned Names
/// row uses: its lock TRANSFER draft's confirmation height and the wallet's
/// sync height on the profile's network.
fn lockup_left(conn: &rusqlite::Connection, l: &ShakedexListing) -> Result<Option<i64>, AppError> {
    if l.state != ListingState::Locking {
        return Ok(None);
    }
    let Some(draft) = l
        .lock_transfer_draft_id
        .as_deref()
        .map(|id| queries::get_tx_draft(conn, id))
        .transpose()?
        .flatten()
    else {
        return Ok(None);
    };
    let params = queries::profile_network(conn, &l.wallet_profile_id)?.name_params();
    let tip = crate::noncustodial::sync::get_sync_height(conn, &l.wallet_profile_id)?;
    Ok(queries::locking_blocks_until_finalize(
        l.state,
        Some(draft.status),
        draft.confirmation_height,
        params,
        tip,
    ))
}

/// R19 day 2, generic over the runtime so tests drive it: check
/// ([`prepare_lock_finalize`]), confirm (R20), then in one hold of the
/// unlocked session sign the FINALIZE and every step over the lock coin it
/// creates, write the listing file, and store the signed FINALIZE draft with
/// the listing — now Finalizing — in one database transaction. Sends
/// nothing: `commands::tx::broadcast_tx_draft` sends the draft, and the
/// steps leave the wallet only once that FINALIZE is mined
/// ([`export_listing_file_from_conn`]).
pub(crate) async fn finalize_and_sign_confirmed<R: tauri::Runtime>(
    state: &State<'_, AppState>,
    app: &tauri::AppHandle<R>,
    listing_id: &str,
    prices: &[StepInput],
    schedule: Option<&ScheduleInput>,
    fee_rate: Option<u64>,
) -> Result<ListingSummary, AppError> {
    let p = prepare_lock_finalize(state, listing_id, prices, schedule, fee_rate).await?;
    let net = p.ctx.network;
    let (lock_vout, lock_value) = sell::lock_output(&p.plan.plan, &p.key.address)?;
    let steps = p.steps.clone();
    crate::commands::secure_confirm::confirm_rows(
        app,
        FINALIZE_AND_SIGN_TITLE,
        FINALIZE_AND_SIGN_MESSAGE,
        sell::finalize_and_sign_rows(&sell::FinalizeAndSignRows {
            name: &p.listing.name,
            finalize_fee: p.plan.fee,
            lock_address: &p.key.address,
            payment_address: &p.payment_address,
            steps: &steps,
            mtp: p.mtp,
        }),
    )
    .await?;

    let mut lock_txid = [0u8; 32];
    hex::decode_to_slice(&p.plan.txid, &mut lock_txid)
        .map_err(|e| AppError::Other(format!("FINALIZE txid is not hex: {e}")))?;
    let payment = output_address_from_string(net, &p.payment_address)?;
    // One unlock (R19): the session is authorized once for the FINALIZE and
    // every step; the confirmation may have outlasted the earlier check.
    let (signed_hex, signed) = with_signer(state, &p.ctx, |session| {
        let (hex, txid) = actions::sign_plan(session, &p.plan.plan)?;
        if txid != p.plan.txid {
            return Err(AppError::Other(
                "the signed FINALIZE's txid is not the plan's".into(),
            ));
        }
        let signed = steps
            .iter()
            .map(|&(price, lock_time)| {
                sell::sign_step(
                    &p.key,
                    &template::StepTemplate {
                        lock_outpoint: (lock_txid, lock_vout),
                        lock_value,
                        lock_pubkey: &p.key.pubkey,
                        payment: payment.clone(),
                        price,
                        lock_time_secs: lock_time,
                    },
                )
                .map(|signature| PriceStep {
                    price,
                    lock_time,
                    signature,
                    fee: 0,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((hex, signed))
    })?;
    let expires_at = p.mtp + sell::LISTING_LIFETIME_SECS;
    let file = write_listing_file(
        &NewListingFile {
            name: &p.listing.name,
            lock_txid,
            lock_vout,
            public_key: p.key.pubkey,
            payment_addr: &p.payment_address,
            steps: &signed,
            expires_at,
            mode: p.listing.mode,
        },
        net,
    )?;
    let stored: Vec<sell::StoredStep> = signed
        .iter()
        .map(|s| sell::StoredStep {
            price: s.price,
            lock_time: s.lock_time,
            signature: hex::encode(s.signature),
        })
        .collect();

    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    // The signed FINALIZE and the Finalizing listing commit together: the
    // listing leaves CANCEL_ABORTABLE in the same transaction, so the
    // before-lock job never reads its FINALIZE as an abort.
    let tx = conn.unchecked_transaction()?;
    let draft_id = draft_ctx::persist_in_tx(
        &tx,
        &p.ctx.profile_id,
        &draft_ctx::DraftLabel {
            action: sell::LOCK_FINALIZE_ACTION,
            name: &p.listing.name,
            recipient: Some(&p.key.address),
            name_list: None,
            warnings: &[sell::STEP_SIGNATURE_PERMANENCE.to_string()],
        },
        &p.plan,
    )?;
    let summary = queries::get_tx_draft(&tx, &draft_id)?
        .ok_or_else(|| AppError::Other("draft vanished after insert".into()))?
        .summary_json;
    queries::update_tx_draft_signed(&tx, &draft_id, &signed_hex, &summary)?;
    let n = queries::mark_listing_finalizing_in_tx(
        &tx,
        &p.listing.id,
        &queries::FinalizingListing {
            finalize_draft_id: &draft_id,
            lock_txid: &p.plan.txid,
            lock_vout,
            steps_json: &serde_json::to_string(&stored)?,
            listing_file_json: &file,
            expires_at,
        },
    )?;
    if n != 1 {
        return Err(AppError::InvalidInput(
            "this listing changed meanwhile: nothing was saved; try again".into(),
        ));
    }
    tx.commit()?;
    let l = queries::get_shakedex_listing(&conn, &p.listing.id)?
        .ok_or_else(|| AppError::Other("listing vanished after Finalize & sign".into()))?;
    ListingSummary::of(&conn, &l)
}

/// Why Finalize & sign and the export refuse a listing that is over or was
/// finalized elsewhere, by its state; `None` for every other state.
fn listing_over(state: ListingState) -> Option<&'static str> {
    match state {
        ListingState::Aborted => Some(
            "this listing was aborted: the name left its lock transfer before the finalize \
             into the lock, so there is nothing to finalize, sign or export; lock the name \
             again to sell it",
        ),
        ListingState::Expired => Some("the name expired while it was locked: this listing is over"),
        ListingState::Cancelled => {
            Some("this listing is cancelled: its signed prices can no longer be used")
        }
        ListingState::Sold => Some("this name is sold: the listing is over"),
        ListingState::Restored => Some(
            "this lock was restored by name, or finalized by another wallet with the same \
             recovery phrase: it has no signed prices or listing file here; import its saved file",
        ),
        _ => None,
    }
}

/// Why Finalize & sign refuses a listing in `state` (R19); `None` for
/// ReadyToFinalize. [`finalize_and_sign_refusal`] asks it after the
/// pending abort.
pub(crate) fn finalize_refusal(state: ListingState) -> Option<&'static str> {
    match state {
        ListingState::ReadyToFinalize => None,
        ListingState::Locking => Some("Finalize & sign opens after the transfer lockup"),
        other => {
            Some(listing_over(other).unwrap_or("this listing is already finalized into its lock"))
        }
    }
}

/// Why Finalize & sign refuses listing `l` now (R19); `None` when it may
/// run. The command and My listings ask this one function. A Cancel
/// transfer that may still land is named first, while the lockup runs as
/// after it: it spends the coin the FINALIZE would, and once mined the
/// listing is Aborted, so the lockup is not what Finalize & sign waits for.
pub(crate) fn finalize_and_sign_refusal(
    conn: &rusqlite::Connection,
    l: &ShakedexListing,
) -> Result<Option<String>, AppError> {
    if l.state.aborts_by_cancel_transfer() && abort_may_still_land(conn, l)? {
        return Ok(Some(sell::CANCEL_TRANSFER_PENDING.to_string()));
    }
    Ok(finalize_refusal(l.state).map(str::to_string))
}

/// R19: the listing's Cancel transfer may still be mined (its draft alive
/// and not confirmed); it spends the coin the FINALIZE into the lock would.
pub(crate) fn abort_may_still_land(
    conn: &rusqlite::Connection,
    l: &ShakedexListing,
) -> Result<bool, AppError> {
    let Some(id) = l.abort_draft_id.as_deref() else {
        return Ok(false);
    };
    Ok(queries::get_tx_draft(conn, id)?.is_some_and(|d| queries::draft_may_still_land(d.status)))
}

/// Why the cancel's FINALIZE cannot be built for a listing in `state`
/// (R28); `None` for CancelAwaitingFinalize. The command and My listings ask
/// this one function.
pub(crate) fn finalize_cancel_refusal(state: ListingState) -> Option<&'static str> {
    match state {
        ListingState::CancelAwaitingFinalize => None,
        ListingState::Cancelling => Some(
            "the cancel is not mined yet: its finalize opens after the transfer lockup that \
             follows",
        ),
        ListingState::CancelFinalizing => Some(
            "the finalize of this cancel is already built: send it, or delete it to build \
             another",
        ),
        // A state `listing_over` does not name (before the lock, Finalizing,
        // Listed, SalePending) has no cancel.
        other => {
            Some(listing_over(other).unwrap_or("this listing has no mined cancel to finalize"))
        }
    }
}

/// R28: a mined cancel whose transfer lockup no sync has counted yet
/// (`cancel_blocks_remaining` NULL): its FINALIZE is not offered on a guess.
/// The UI's twin is `marketText.ts::CANCEL_LOCKUP_NOT_COUNTED`.
pub const CANCEL_LOCKUP_NOT_COUNTED: &str =
    "the cancel is mined, but its transfer lockup is not counted yet: sync, then finalize it";

/// R28: the lockup left before the cancel's FINALIZE is valid.
pub(crate) fn cancel_finalize_wait(remaining: i64) -> String {
    format!(
        "the cancel can be finalized in {remaining} block{}",
        if remaining == 1 { "" } else { "s" }
    )
}

/// R28: the cancel's own prompt title, never the transaction prompt's.
pub const CANCEL_TITLE: &str = "Confirm cancel";
const CANCEL_MESSAGE: &str = "Review these details. This signs, with the lock key, a \
     transfer of the name out of its lock back to this wallet.";
/// R28: hsd answers 404 for a lock coin spent in a block or in its mempool.
pub const CANCEL_LOCK_COIN_SPENT: &str = "the lock coin is no longer unspent (bought, \
     cancelled, or being spent in the node's mempool): there is nothing to cancel";
/// R21: the stored cancel address is not this profile's receive address at
/// the stored index.
pub const CANCEL_NOT_OUR_ADDRESS: &str = "this listing's cancel address is not this wallet's \
     reserved receive address: nothing was signed";
/// R28: a purchase the node already holds would beat the cancel.
const CANCEL_PURCHASE_PENDING: &str = "a purchase of this listing is in the node's mempool: \
     the node would not take a cancel now";
/// R28: the finalize into the lock is not mined, so the lock coin is not
/// there to spend yet.
const CANCEL_LOCK_NOT_MINED: &str = "the finalize into the lock is not mined yet: the listing \
     can be cancelled once it is";

/// Why Cancel refuses a listing in `state` (R28); `None` for the two whose
/// lock coin is mined and not being bought, Listed and Restored. The command
/// and My listings ask this one function.
pub(crate) fn cancel_refusal(state: ListingState) -> Option<&'static str> {
    match state {
        ListingState::Listed | ListingState::Restored => None,
        ListingState::Locking | ListingState::ReadyToFinalize => {
            Some("the name is not in its lock yet: take it back with Cancel transfer")
        }
        ListingState::Finalizing => Some(CANCEL_LOCK_NOT_MINED),
        ListingState::SalePending => Some(CANCEL_PURCHASE_PENDING),
        ListingState::Cancelling
        | ListingState::CancelAwaitingFinalize
        | ListingState::CancelFinalizing => Some("this listing is already being cancelled"),
        ListingState::Sold
        | ListingState::Cancelled
        | ListingState::Aborted
        | ListingState::Expired => listing_over(state),
    }
}

/// Everything the cancel has checked and built before it asks (R28).
pub(crate) struct PreparedCancel {
    ctx: Ctx,
    listing: ShakedexListing,
    plan: actions::PlanResult,
    lock: (String, u32),
    lock_address: String,
    cancel_address: String,
    cancel_index: u32,
    current_price: cancel::CancelPrice,
    /// R28: a stored step ahead at the MTP cheaper than the current one (any
    /// step ahead for a reverse auction with none valid yet),
    /// `cancel::still_buyable`.
    cheaper_ahead: bool,
}

/// "could not check" for a field `what` of a node reply that a command
/// signing over a listing's coin reads, `checked` naming what it checks
/// ("the cancel", "the price").
fn reply_could_not_check(what: &str, checked: &str) -> AppError {
    AppError::Rpc(format!(
        "node did not report {what}: could not check {checked}"
    ))
}

/// "could not check" for a node reply the cancel reads.
fn cancel_could_not_check(what: &str) -> AppError {
    reply_could_not_check(what, "the cancel")
}

/// R21, R32: reserve one receive address of `profile_id` and store
/// it as the cancel address of the listing `listing_id`, a lock restored by
/// name (Restored, or Listed by its own file) which has none, in one
/// database transaction: when the row write does not apply
/// (the row changed meanwhile), the reservation is rolled back too.
/// Returns the address and its receive index.
pub(crate) fn reserve_restored_cancel_address(
    conn: &rusqlite::Connection,
    profile_id: &str,
    listing_id: &str,
) -> Result<(String, u32), AppError> {
    let tx = conn.unchecked_transaction()?;
    let d = derivation::reserve_receive_address(&tx, profile_id)?;
    if queries::set_restored_lock_cancel_address(&tx, listing_id, &d.address, d.child_index)? != 1 {
        return Err(AppError::InvalidInput(
            "this lock changed meanwhile: nothing was signed; try again".into(),
        ));
    }
    tx.commit()?;
    Ok((d.address, d.child_index))
}

/// The words a command that signs over a listing's lock coin (Cancel, R28;
/// Lower price, R26) puts in [`lock_on_node`]'s refusals.
struct LockCoinWords {
    /// What could not be checked: "the cancel", "the price".
    checked: &'static str,
    /// hsd's 404 for the lock coin.
    spent: &'static str,
    /// The lock coin in the mempool: the finalize into the lock not mined.
    not_mined: &'static str,
    /// The end of a refusal for a name whose lock is gone.
    nothing: &'static str,
}

const CANCEL_LOCK_WORDS: LockCoinWords = LockCoinWords {
    checked: "the cancel",
    spent: CANCEL_LOCK_COIN_SPENT,
    not_mined: CANCEL_LOCK_NOT_MINED,
    nothing: "there is nothing to cancel",
};

/// What the node showed of a listing's lock coin once every check held.
struct LockOnNode {
    /// The lock coin's value, as hsd reports it.
    value: u64,
    /// The name's height, the lock coin's and the live registration's.
    name_height: u32,
    /// The node's MTP, when asked for.
    mtp: Option<u64>,
}

/// The node checks Cancel (R28) and Lower price (R26) both make before they
/// sign over a listing's stored lock coin, in one place so they cannot
/// drift: the lock coin of `name` unspent (hsd's 404 is `words.spent`), at
/// its stored outpoint (the txid in any case), a FINALIZE of the name at the
/// lock address `at`, mined, with a readable value and name height; the
/// name's live state (`info` not null) at that name height (a lock coin left
/// over from an earlier registration is refused); and, with `read_mtp`, the
/// node's MTP. A reply missing a field read here is "could not check".
async fn lock_on_node(
    ctx: &Ctx,
    name: &str,
    at: &sell::ListingLock,
    lock: (&str, u32),
    read_mtp: bool,
    words: &LockCoinWords,
) -> Result<LockOnNode, AppError> {
    let could_not_check = |what: &str| reply_could_not_check(what, words.checked);
    let (lock_txid, lock_vout) = lock;
    let coin = ctx
        .node
        .get_coin(lock_txid, lock_vout)
        .await?
        .ok_or_else(|| AppError::InvalidInput(words.spent.into()))?;
    match at.stored_coin(&coin, lock, COV_FINALIZE) {
        Ok(_) => {}
        Err(sell::StoredCoinRefusal::Unreadable) => {
            return Err(could_not_check("the lock coin's address or covenant"));
        }
        Err(sell::StoredCoinRefusal::SomethingElse) => {
            return Err(AppError::Rpc(format!(
                "node reported something else than this listing's lock at its lock outpoint: \
                 could not check {}",
                words.checked
            )));
        }
    }
    if coin.mined_height()?.is_none() {
        return Err(AppError::InvalidInput(words.not_mined.into()));
    }
    let value = u64::try_from(coin.value)
        .map_err(|_| AppError::Rpc(format!("bad lock coin value {}", coin.value)))?;
    let cov_height = coin
        .covenant
        .as_ref()
        .and_then(purchase::covenant_name_height)
        .ok_or_else(|| could_not_check("a readable name height in the lock coin"))?;
    let reply = ctx.node.get_name_info(name).await?;
    if reply.get("info").is_some_and(serde_json::Value::is_null) {
        return Err(AppError::InvalidInput(format!(
            "'{name}' has no on-chain state or has expired: {}",
            words.nothing
        )));
    }
    let ns = draft_ctx::name_state_strict(&reply, name)?;
    if ns.height != cov_height {
        return Err(AppError::InvalidInput(format!(
            "the lock coin is left over from an earlier registration of the name: {}",
            words.nothing
        )));
    }
    let mtp = if read_mtp {
        Some(
            ctx.node
                .get_blockchain_info()
                .await
                .map_err(|e| could_not_check(&format!("its tip ({e})")))?
                .mediantime
                .ok_or_else(|| could_not_check("its median time (the current price)"))?,
        )
    } else {
        None
    };
    Ok(LockOnNode {
        value,
        name_height: cov_height,
        mtp,
    })
}

/// R28's checks, before the prompt: the gates (R16, R6; no experimental
/// flag, R15) and the unlocked signer first; the listing the active
/// profile's and Listed or Restored; on the node, the lock coin unspent,
/// mined, a FINALIZE of the name at the lock address of the stored key, of
/// the name's live registration, and the MTP (the current price the prompt
/// shows); the cancel address reserved (a lock restored by name reserves
/// one now, R32) and the profile's receive address at its stored index
/// (R21); the lock key re-derived after the reads, its public
/// key the stored one; the plan built and checked to be for the profile's
/// account, that path, this name and the stored lock coin. Writes nothing
/// but that reservation.
pub(crate) async fn prepare_cancel(
    state: &State<'_, AppState>,
    listing_id: &str,
    fee_rate: Option<u64>,
) -> Result<PreparedCancel, AppError> {
    let ctx = software_writer_ctx(state)?;
    authorize_signer(state, &ctx)?;
    let listing = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let l = queries::get_shakedex_listing(&conn, listing_id)?
            .filter(|l| l.wallet_profile_id == ctx.profile_id)
            .ok_or_else(|| AppError::NotFound(format!("listing {listing_id}")))?;
        if let Some(why) = cancel_refusal(l.state) {
            return Err(AppError::InvalidInput(why.into()));
        }
        l
    };
    let corrupted = |what: &str| AppError::Other(format!("corrupted listing: no {what}"));
    let (lock_txid, lock_vout) = shakedex_jobs::stored_lock(&listing)?
        .map(|(t, v)| (t.to_string(), v))
        .ok_or_else(|| corrupted("lock coin"))?;
    let pubkey = shakedex_jobs::lock_pubkey(&listing)?;
    let name = listing.name.clone();
    let at = shakedex_jobs::listing_lock(ctx.network, &listing)?;
    let lock_addr = at.address.clone();
    let steps = shakedex_jobs::stored_steps(&listing)?;
    // A Restored lock without its listing file stores no steps (restored
    // by name, or finalized into our lock by another device): its current
    // price is not known here, so the MTP is not read for it.
    let on_node = lock_on_node(
        &ctx,
        &name,
        &at,
        (&lock_txid, lock_vout),
        !steps.is_empty(),
        &CANCEL_LOCK_WORDS,
    )
    .await?;
    let (lock_value, name_height) = (on_node.value, on_node.name_height);
    // A listing with steps but none valid at the MTP says so (R3).
    let current_price = match on_node.mtp {
        None => cancel::CancelPrice::NotKnown,
        Some(mtp) => match sell::current_step_price(&steps, mtp)? {
            Some(price) => cancel::CancelPrice::Step(price),
            None => cancel::CancelPrice::NoneValidYet,
        },
    };
    // R28: a step ahead cheaper than the current one is buyable once valid
    // (`sell::steps_ahead`), in every mode: a reverse auction's schedule, or
    // a lowered Buy Now's step after a reorg. With no step valid yet, any
    // step ahead counts for a reverse auction only: a Buy Now's one step not
    // valid yet becomes buyable at that same price.
    let cheaper_ahead = match (on_node.mtp, current_price) {
        (Some(mtp), cancel::CancelPrice::Step(price)) => {
            sell::steps_ahead(&steps, mtp, price)?.0 > 0
        }
        (Some(mtp), cancel::CancelPrice::NoneValidYet)
            if listing.mode == ListingMode::ReverseAuction =>
        {
            sell::steps_ahead(&steps, mtp, u64::MAX)?.0 > 0
        }
        _ => false,
    };

    // R21, R32: a lock restored by name reserves its cancel address
    // now, after the node reads, whether still Restored or since upgraded to
    // Listed by its own file (a lock made here stored one at lock time). The
    // reservation and the row commit together.
    let (cancel_address, cancel_index) =
        match (listing.cancel_address.clone(), listing.cancel_child_index) {
            (Some(a), Some(i)) => (a, u32::try_from(i).map_err(|_| corrupted("cancel index"))?),
            (None, None) => {
                let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
                reserve_restored_cancel_address(&conn, &ctx.profile_id, &listing.id)?
            }
            _ => return Err(corrupted("cancel address")),
        };
    // R21: the profile's own receive address at the stored index.
    {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let derived =
            queries::receive_address_at(&conn, &ctx.profile_id, ctx.account, cancel_index)?;
        if derived.as_deref() != Some(cancel_address.as_str()) {
            return Err(AppError::InvalidInput(CANCEL_NOT_OUR_ADDRESS.into()));
        }
    }

    let key = derive_listing_key(state, &ctx, &name)?;
    if key.pubkey != pubkey {
        return Err(AppError::InvalidInput(sell::LISTING_KEY_MISMATCH.into()));
    }
    let mut lock_bytes = [0u8; 32];
    hex::decode_to_slice(&lock_txid, &mut lock_bytes)
        .map_err(|_| corrupted("readable lock txid"))?;
    let plan = cancel::build_cancel_plan(&cancel::CancelInput {
        network: ctx.network,
        account: ctx.account,
        name: &name,
        name_height,
        lock_outpoint: (lock_bytes, lock_vout),
        lock_value,
        lock_pubkey: key.pubkey,
        cancel_address: &cancel_address,
        cancel_branch: derivation::BRANCH_RECEIVE,
        cancel_index,
        funding: &ctx.funding,
        change_address: &ctx.change_address,
        rate: draft_ctx::fee_rate(&ctx, fee_rate),
        #[cfg(test)]
        fixed_fee: None,
    })?;
    cancel::check_cancel_plan(
        &plan.plan,
        ctx.account,
        cancel_index,
        &name,
        (&lock_txid, lock_vout),
    )?;
    Ok(PreparedCancel {
        ctx,
        listing,
        plan,
        lock: (lock_txid, lock_vout),
        lock_address: lock_addr,
        cancel_address,
        cancel_index,
        current_price,
        cheaper_ahead,
    })
}

/// R28, generic over the runtime so tests drive it: check
/// ([`prepare_cancel`]), ask (R28's own prompt), then sign the cancel with
/// the unlocked session (the signer derives the lock key and re-checks the
/// TRANSFER's commitment, `cancel::check_lock_key_input`) and store the
/// signed draft with the listing's move to Cancelling in one database
/// transaction. Sends nothing: `commands::tx::broadcast_tx_draft` does.
pub(crate) async fn cancel_listing_confirmed<R: tauri::Runtime>(
    state: &State<'_, AppState>,
    app: &tauri::AppHandle<R>,
    listing_id: &str,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let p = prepare_cancel(state, listing_id, fee_rate).await?;
    crate::commands::secure_confirm::confirm_rows(
        app,
        CANCEL_TITLE,
        CANCEL_MESSAGE,
        cancel::cancel_rows(&cancel::CancelRows {
            name: &p.listing.name,
            fee: p.plan.fee,
            cancel_address: &p.cancel_address,
            lock_address: &p.lock_address,
            current_price: p.current_price,
            published: p.listing.publish,
            cheaper_ahead: p.cheaper_ahead,
            elsewhere_price: foreign_step_of(&p.listing).map(|s| s.price),
        }),
    )
    .await?;
    // The prompt may have outlasted the earlier check: authorized again.
    let signed_hex = with_signer(state, &p.ctx, |session| {
        let (hex, txid) = actions::sign_plan(session, &p.plan.plan)?;
        if txid != p.plan.txid {
            return Err(AppError::Other(
                "the signed cancel's txid is not the plan's".into(),
            ));
        }
        Ok(hex)
    })?;
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    // The signed cancel and the Cancelling listing commit together.
    let tx = conn.unchecked_transaction()?;
    let draft_id = draft_ctx::persist_in_tx(
        &tx,
        &p.ctx.profile_id,
        &draft_ctx::DraftLabel {
            action: cancel::CANCEL_ACTION,
            name: &p.listing.name,
            recipient: Some(&p.cancel_address),
            name_list: None,
            warnings: &[
                cancel::still_buyable(p.cheaper_ahead).to_string(),
                cancel::CANCEL_MEMPOOL_PURCHASE.to_string(),
            ],
        },
        &p.plan,
    )?;
    let summary = queries::get_tx_draft(&tx, &draft_id)?
        .ok_or_else(|| AppError::Other("draft vanished after insert".into()))?
        .summary_json;
    queries::update_tx_draft_signed(&tx, &draft_id, &signed_hex, &summary)?;
    let n = queries::mark_listing_cancelling_in_tx(
        &tx,
        &p.listing.id,
        &queries::CancellingListing {
            cancel_draft_id: &draft_id,
            cancel_txid: &p.plan.txid,
            lock: (&p.lock.0, p.lock.1),
            cancel_address: &p.cancel_address,
            cancel_child_index: p.cancel_index,
        },
    )?;
    if n != 1 {
        return Err(AppError::InvalidInput(
            "this listing changed meanwhile: nothing was saved; try again".into(),
        ));
    }
    tx.commit()?;
    draft_ctx::draft_summary(&conn, &draft_id)
}

/// R26: the Lower price prompt's own title.
pub const LOWER_PRICE_TITLE: &str = "Confirm Lower price";
const LOWER_PRICE_MESSAGE: &str = "Review these details. This signs, with the lock key, one \
     more, lower price for the name.";
/// R26: disabled while a cancel is unconfirmed.
pub const LOWER_DURING_CANCEL: &str = "a cancel of this listing is not mined yet: its price \
     cannot be lowered";
/// R26: hsd answers 404 for a lock coin spent in a block or in its mempool.
pub const LOWER_LOCK_COIN_SPENT: &str = "the lock coin is no longer unspent (bought, \
     cancelled, or being spent in the node's mempool): the price cannot be lowered";
/// R26: a listing without signed prices here has no current price.
pub const LOWER_NO_PRICE_HERE: &str = "this listing has no signed prices here: import its \
     listing file first";
/// R40: Lower price waits for a take-over while another device manages the
/// listing (`marketText.ts` holds the UI's copy, word for word).
pub const LOWER_MANAGED_ELSEWHERE: &str = "another device or profile with your recovery phrase \
     manages this listing: lower its price there, or manage it from this device first";

const LOWER_LOCK_WORDS: LockCoinWords = LockCoinWords {
    checked: "the price",
    spent: LOWER_LOCK_COIN_SPENT,
    not_mined: "the finalize into the lock is not mined: the price cannot be lowered until it is",
    nothing: "the price cannot be lowered",
};

/// Why Lower price refuses a listing in `state` (R26) whose stored steps
/// are `no_steps` empty, `managed_elsewhere` its mark (R40: refused until
/// the seller takes it over); `None` for Listed with signed steps here, not
/// marked. The command and My listings ask this one function.
pub(crate) fn lower_refusal(
    state: ListingState,
    no_steps: bool,
    managed_elsewhere: bool,
) -> Option<&'static str> {
    match state {
        ListingState::Listed if managed_elsewhere => Some(LOWER_MANAGED_ELSEWHERE),
        ListingState::Listed if no_steps => Some(LOWER_NO_PRICE_HERE),
        ListingState::Listed => None,
        ListingState::Cancelling => Some(LOWER_DURING_CANCEL),
        ListingState::CancelAwaitingFinalize | ListingState::CancelFinalizing => {
            Some("this listing is cancelled: its price cannot be lowered")
        }
        ListingState::SalePending => {
            Some("a purchase of this listing is in the node's mempool: its price cannot be lowered")
        }
        ListingState::Restored => Some(LOWER_NO_PRICE_HERE),
        ListingState::Locking | ListingState::ReadyToFinalize => {
            Some("the price is set at Finalize & sign")
        }
        ListingState::Finalizing => {
            Some("the price can be lowered once the finalize into the lock is mined")
        }
        ListingState::Sold
        | ListingState::Cancelled
        | ListingState::Aborted
        | ListingState::Expired => listing_over(state),
    }
}

/// R26, generic over the runtime so tests drive it. The gates (R16, R6; no
/// experimental flag, R15) and the unlocked signer first; the listing the
/// active profile's, Listed, a Buy Now or a reverse auction (R26: the
/// lowered step joins the schedule; R3 keeps the later steps below it, the
/// dearer ones are moot) with signed steps here; the price parsed by R19's
/// rule; on the node the lock coin checks Cancel makes too
/// ([`lock_on_node`]: unspent, at the stored outpoint, a FINALIZE of the
/// name at the lock address of the stored key, mined, of the name's live
/// registration) and the MTP; the new price below the current step at that
/// MTP (R3, `sell::check_lower_price`); the lock key re-derived after the
/// reads, its public key the stored one; then R26's own prompt (for a
/// schedule, how many later steps still take over and how many are moot:
/// `sell::steps_ahead`), one step signed valid now (R19: the MTP minus 512 s) over the
/// stored lock coin at hsd's value, paying the listing's payment address,
/// added to the stored listing file as written, unknown fields kept, once
/// that file is checked to be the row's (`add_step_to_listing_file`, read
/// back by the strict parser), and the steps and file stored only while the
/// listing is still Listed on that lock coin with the steps read here
/// (`queries::lower_listing_price`). Sends nothing.
pub(crate) async fn lower_price_confirmed<R: tauri::Runtime>(
    state: &State<'_, AppState>,
    app: &tauri::AppHandle<R>,
    listing_id: &str,
    price: &str,
) -> Result<ListingSummary, AppError> {
    let ctx = software_writer_ctx(state)?;
    authorize_signer(state, &ctx)?;
    let (listing, stored) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let l = queries::get_shakedex_listing(&conn, listing_id)?
            .filter(|l| l.wallet_profile_id == ctx.profile_id)
            .ok_or_else(|| AppError::NotFound(format!("listing {listing_id}")))?;
        let stored = shakedex_jobs::stored_steps(&l)?;
        if let Some(why) =
            lower_refusal(l.state, stored.is_empty(), l.managed_elsewhere_at.is_some())
        {
            return Err(AppError::InvalidInput(why.into()));
        }
        (l, stored)
    };
    let new_price = sell::parse_step_price(price)?;
    let corrupted = |what: &str| AppError::Other(format!("corrupted listing: no {what}"));
    let (lock_txid, lock_vout) = shakedex_jobs::stored_lock(&listing)?
        .map(|(t, v)| (t.to_string(), v))
        .ok_or_else(|| corrupted("lock coin"))?;
    let pubkey = shakedex_jobs::lock_pubkey(&listing)?;
    let payment_address = listing
        .payment_address
        .clone()
        .ok_or_else(|| corrupted("payment address"))?;
    let name = listing.name.clone();
    let at = shakedex_jobs::listing_lock(ctx.network, &listing)?;
    let lock_addr = at.address.clone();
    // The step is added to the stored file as written (R26, R32): it must
    // be this row's file, its lock, key, payment address and steps.
    let stored_file = rows_own_file(&listing, ctx.network, &stored)?;

    let on_node = lock_on_node(
        &ctx,
        &name,
        &at,
        (&lock_txid, lock_vout),
        true,
        &LOWER_LOCK_WORDS,
    )
    .await?;
    let mtp = on_node
        .mtp
        .ok_or_else(|| AppError::Other("the node's median time was not read".into()))?;
    let current = sell::current_step_price(&stored, mtp)?.ok_or_else(|| {
        AppError::InvalidInput(
            "no price of this listing is valid yet at the node's median time: there is no \
             current price to lower"
                .into(),
        )
    })?;
    sell::check_lower_price(new_price, current)?;
    let ahead = sell::steps_ahead(&stored, mtp, new_price)?;
    let lock_time = sell::buy_now_lock_time(mtp);
    let key = derive_listing_key(state, &ctx, &name)?;
    if key.pubkey != pubkey {
        return Err(AppError::InvalidInput(sell::LISTING_KEY_MISMATCH.into()));
    }
    crate::commands::secure_confirm::confirm_rows(
        app,
        LOWER_PRICE_TITLE,
        LOWER_PRICE_MESSAGE,
        sell::lower_price_rows(&sell::LowerPriceRows {
            name: &name,
            current_price: current,
            new_price,
            lock_time,
            mtp,
            payment_address: &payment_address,
            lock_address: &lock_addr,
            ahead,
        }),
    )
    .await?;

    let mut lock_bytes = [0u8; 32];
    hex::decode_to_slice(&lock_txid, &mut lock_bytes)
        .map_err(|_| corrupted("readable lock txid"))?;
    let payment = output_address_from_string(ctx.network, &payment_address)?;
    // The prompt may have outlasted the earlier check: authorized again.
    let signature = with_signer(state, &ctx, |_| {
        sell::sign_step(
            &key,
            &template::StepTemplate {
                lock_outpoint: (lock_bytes, lock_vout),
                lock_value: on_node.value,
                lock_pubkey: &key.pubkey,
                payment,
                price: new_price,
                lock_time_secs: lock_time,
            },
        )
    })?;
    // The file's expiry stays as signed; a file imported without one (R32)
    // gets R23's, from this signing's MTP.
    let (file, expires_at) = add_step_to_listing_file(
        &stored_file,
        &PriceStep {
            price: new_price,
            lock_time,
            signature,
            fee: 0,
        },
        mtp + sell::LISTING_LIFETIME_SECS,
        listing.mode,
        ctx.network,
    )?;
    let mut stored_new = stored.clone();
    stored_new.push(sell::StoredStep {
        price: new_price,
        lock_time,
        signature: hex::encode(signature),
    });
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let n = queries::lower_listing_price(
        &conn,
        &listing.id,
        &queries::LoweredPrice {
            lock: (&lock_txid, lock_vout),
            old_steps_json: &listing.steps_json,
            steps_json: &serde_json::to_string(&stored_new)?,
            listing_file_json: &file,
            expires_at: i64::try_from(expires_at).map_err(|_| corrupted("readable expiry"))?,
        },
    )?;
    if n != 1 {
        return Err(AppError::InvalidInput(
            "this listing changed meanwhile: nothing was saved; try again".into(),
        ));
    }
    let l = queries::get_shakedex_listing(&conn, &listing.id)?
        .ok_or_else(|| AppError::Other("listing vanished after Lower price".into()))?;
    ListingSummary::of(&conn, &l)
}

/// The stored listing file of `l`, whose readable stored steps are
/// `stored`, once it is checked to be the row's own: its lock outpoint,
/// public key, payment address and steps the stored ones (R26, R44: a step
/// is added to it as written). A file that is not is a corrupted row.
fn rows_own_file(
    l: &ShakedexListing,
    network: Network,
    stored: &[sell::StoredStep],
) -> Result<String, AppError> {
    let corrupted = |what: &str| AppError::Other(format!("corrupted listing: no {what}"));
    let (lock_txid, lock_vout) =
        shakedex_jobs::stored_lock(l)?.ok_or_else(|| corrupted("lock coin"))?;
    let pubkey = shakedex_jobs::lock_pubkey(l)?;
    let stored_file = l
        .listing_file_json
        .clone()
        .ok_or_else(|| corrupted("listing file"))?;
    let file_is_the_rows = ListingFile::parse(&stored_file, network)
        .ok()
        .is_some_and(|f| {
            hex::encode(f.lock_txid) == lock_txid
                && f.lock_vout == lock_vout
                && f.public_key == pubkey
                && Some(f.payment_addr.as_str()) == l.payment_address.as_deref()
                && f.steps
                    .iter()
                    .map(|s| sell::StoredStep {
                        price: s.price,
                        lock_time: s.lock_time,
                        signature: hex::encode(s.signature),
                    })
                    .eq(stored.iter().cloned())
        });
    if !file_is_the_rows {
        return Err(corrupted("listing file matching its lock and steps"));
    }
    Ok(stored_file)
}

/// R42: whether My listings shows `l` as managed elsewhere: marked, and in
/// a state the mark means something in (Listed, or Cancelling while the
/// other device's price is still buyable); an ended listing is not shown so.
fn shown_managed_elsewhere(l: &ShakedexListing) -> bool {
    l.managed_elsewhere_at.is_some()
        && matches!(l.state, ListingState::Listed | ListingState::Cancelling)
}

/// R38: the foreign step stored on `l` while it is managed elsewhere;
/// `None` when it is not marked or the step stored does not read.
pub(crate) fn foreign_step_of(l: &ShakedexListing) -> Option<sell::ForeignStep> {
    l.managed_elsewhere_at.as_ref()?;
    serde_json::from_str(l.foreign_step_json.as_deref()?).ok()
}

/// R44: the take-over of a listing that is not managed elsewhere.
pub const TAKE_OVER_NOT_MARKED: &str = "this listing is not managed from another device: there \
     is nothing to take over";
/// R44: a foreign step paying another address than the listing's.
pub const TAKE_OVER_OTHER_ADDRESS: &str = "the price set on the other device pays an address \
     this listing does not know: import that device's listing file, or cancel the listing and \
     list it again";
/// R44: the stored foreign step no longer verifies over the lock coin.
pub const TAKE_OVER_NOT_SIGNED: &str = "the price set on the other device is not signed by \
     this lock over the lock coin the node reports: nothing was saved";
/// R44: hsd answers 404 for a lock coin spent in a block or its mempool.
pub const TAKE_OVER_LOCK_COIN_SPENT: &str = "the lock coin is no longer unspent (bought, \
     cancelled, or being spent in the node's mempool): the listing cannot be taken over";

const TAKE_OVER_LOCK_WORDS: LockCoinWords = LockCoinWords {
    checked: "the price set on the other device",
    spent: TAKE_OVER_LOCK_COIN_SPENT,
    not_mined: "the finalize into the lock is not mined: the listing cannot be taken over \
                until it is",
    nothing: "the listing cannot be taken over",
};

/// Why "Manage from this device" (R44) refuses `l`; `None` for a Listed
/// listing managed elsewhere whose foreign step reads and pays the
/// listing's payment address. The command and My listings ask this one
/// function.
pub(crate) fn take_over_refusal(l: &ShakedexListing) -> Option<&'static str> {
    if l.managed_elsewhere_at.is_none() {
        return Some(TAKE_OVER_NOT_MARKED);
    }
    if l.state != ListingState::Listed {
        return Some(listing_over(l.state).unwrap_or(
            "only a listed listing can be managed from this device: its cancel is on its way",
        ));
    }
    let Some(step) = foreign_step_of(l) else {
        return Some(
            "the price seen from the other device cannot be read here: import that device's \
             listing file",
        );
    };
    (l.payment_address.as_deref() != Some(step.payment_address.as_str()))
        .then_some(TAKE_OVER_OTHER_ADDRESS)
}

/// R44: "Manage from this device". The gates of every selling write (a
/// recovery-phrase profile, a node that can send, the unlocked signer)
/// first; the listing the active profile's, refused by
/// [`take_over_refusal`] (Listed, managed elsewhere, its foreign step
/// paying the listing's payment address); its stored file the row's own
/// ([`rows_own_file`]); on the node the lock coin checks Lower price makes
/// ([`lock_on_node`]: unspent, at the stored outpoint, a FINALIZE of the
/// name at the lock address of the stored key, mined, of the name's live
/// registration) and its median time; the foreign step verified once more
/// over that coin at hsd's value, paying the listing's payment address
/// (`template::verify_step_signature`); then the step added to the stored
/// steps and listing file as written ([`add_step_to_listing_file`]) and
/// the mark cleared in one write (`queries::take_over_listing`: still
/// Listed on that lock coin with the steps and the foreign step read).
/// Nothing is signed; the next keep-listed run uploads over the market's
/// copy as our own.
pub(crate) async fn take_over_confirmed(
    state: &State<'_, AppState>,
    listing_id: &str,
) -> Result<ListingSummary, AppError> {
    let ctx = software_writer_ctx(state)?;
    authorize_signer(state, &ctx)?;
    let (listing, stored) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let l = queries::get_shakedex_listing(&conn, listing_id)?
            .filter(|l| l.wallet_profile_id == ctx.profile_id)
            .ok_or_else(|| AppError::NotFound(format!("listing {listing_id}")))?;
        let stored = readable_steps(&l)?;
        if let Some(why) = take_over_refusal(&l) {
            return Err(AppError::InvalidInput(why.into()));
        }
        (l, stored)
    };
    let step = foreign_step_of(&listing)
        .ok_or_else(|| AppError::Other("the foreign step was not read".into()))?;
    let stored_file = rows_own_file(&listing, ctx.network, &stored)?;
    let corrupted = |what: &str| AppError::Other(format!("corrupted listing: no {what}"));
    let (lock_txid, lock_vout) = shakedex_jobs::stored_lock(&listing)?
        .map(|(t, v)| (t.to_string(), v))
        .ok_or_else(|| corrupted("lock coin"))?;
    let pubkey = shakedex_jobs::lock_pubkey(&listing)?;
    let payment_address = listing
        .payment_address
        .clone()
        .ok_or_else(|| corrupted("payment address"))?;
    let at = shakedex_jobs::listing_lock(ctx.network, &listing)?;
    let on_node = lock_on_node(
        &ctx,
        &listing.name,
        &at,
        (&lock_txid, lock_vout),
        true,
        &TAKE_OVER_LOCK_WORDS,
    )
    .await?;
    let mtp = on_node
        .mtp
        .ok_or_else(|| AppError::Other("the node's median time was not read".into()))?;
    let mut lock_bytes = [0u8; 32];
    hex::decode_to_slice(&lock_txid, &mut lock_bytes)
        .map_err(|_| corrupted("readable lock txid"))?;
    let signature: [u8; 65] = hex::decode(&step.signature)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| AppError::InvalidInput(TAKE_OVER_NOT_SIGNED.into()))?;
    // A `0x84` signature commits to its payment output: verified paying
    // the listing's own address, never the one the market's copy named.
    template::verify_step_signature(
        &template::StepTemplate {
            lock_outpoint: (lock_bytes, lock_vout),
            lock_value: on_node.value,
            lock_pubkey: &pubkey,
            payment: output_address_from_string(ctx.network, &payment_address)?,
            price: step.price,
            lock_time_secs: step.lock_time,
        },
        &signature,
    )
    .map_err(|_| AppError::InvalidInput(TAKE_OVER_NOT_SIGNED.into()))?;
    let (file, expires_at) = add_step_to_listing_file(
        &stored_file,
        &PriceStep {
            price: step.price,
            lock_time: step.lock_time,
            signature,
            fee: 0,
        },
        mtp + sell::LISTING_LIFETIME_SECS,
        listing.mode,
        ctx.network,
    )?;
    let mut stored_new = stored.clone();
    stored_new.push(sell::StoredStep {
        price: step.price,
        lock_time: step.lock_time,
        signature: step.signature.to_ascii_lowercase(),
    });
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let n = queries::take_over_listing(
        &conn,
        &listing.id,
        &queries::TakenOver {
            lock: (&lock_txid, lock_vout),
            old_steps_json: &listing.steps_json,
            // `take_over_refusal` read the step from this column.
            old_foreign_step_json: listing
                .foreign_step_json
                .as_deref()
                .ok_or_else(|| AppError::InvalidInput(TAKE_OVER_NOT_MARKED.into()))?,
            steps_json: &serde_json::to_string(&stored_new)?,
            listing_file_json: &file,
            expires_at: Some(i64::try_from(expires_at).map_err(|_| corrupted("readable expiry"))?),
        },
    )?;
    if n != 1 {
        return Err(AppError::InvalidInput(
            "this listing changed meanwhile: nothing was saved; try again".into(),
        ));
    }
    let l = queries::get_shakedex_listing(&conn, &listing.id)?
        .ok_or_else(|| AppError::Other("listing vanished after the take-over".into()))?;
    ListingSummary::of(&conn, &l)
}

/// R23: the saved listing file of one of the profile's listings, once its
/// FINALIZE into the lock is mined (Listed or later): before that its steps
/// are over a coin that may never exist.
pub(crate) fn export_listing_file_from_conn(
    conn: &rusqlite::Connection,
    profile_id: &str,
    listing_id: &str,
) -> Result<String, AppError> {
    let l = queries::get_shakedex_listing(conn, listing_id)?
        .filter(|l| l.wallet_profile_id == profile_id)
        .ok_or_else(|| AppError::NotFound(format!("listing {listing_id}")))?;
    if let Some(why) = export_refusal(&l) {
        return Err(AppError::InvalidInput(why.into()));
    }
    // `export_refusal` refused a listing without its file; this keeps the
    // command's sentence the same if that check ever moves.
    l.listing_file_json
        .ok_or_else(|| AppError::InvalidInput(EXPORT_NO_FILE.into()))
}

/// R23: a lock with no listing file here (restored by name, or finalized by
/// another wallet) has nothing to export.
const EXPORT_NO_FILE: &str = "this lock has no listing file here: import its saved file";

/// Whether one action of My listings can run now, and the command's own
/// sentence when it cannot (the button is shown disabled with it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListingAction {
    pub allowed: bool,
    pub reason: Option<String>,
}

impl ListingAction {
    fn refused_by(why: Option<String>) -> Self {
        Self {
            allowed: why.is_none(),
            reason: why,
        }
    }
}

/// What a listing can do now, each from the refusal function its command
/// runs first (the state and stored facts; the node's checks come when it
/// runs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListingActions {
    pub finalize_and_sign: ListingAction,
    pub lower_price: ListingAction,
    pub cancel: ListingAction,
    pub finalize_cancel: ListingAction,
    pub export: ListingAction,
    pub publish: ListingAction,
    /// Import the saved listing file of a Restored lock (R32), or of a
    /// listing managed elsewhere (R45).
    pub import_file: ListingAction,
    /// "Manage from this device" (R44): take over a listing another device
    /// or profile of the phrase manages.
    pub manage_here: ListingAction,
}

/// The listing's own draft that was never sent (`queries::never_sent`), or
/// whose send the node may not hold (`broadcast_pending`, which
/// `broadcast_tx_draft` sends again): the lock TRANSFER (Locking), the
/// FINALIZE into the lock (Finalizing), the cancel (Cancelling) or the
/// cancel's FINALIZE (CancelFinalizing). `signed`: only the broadcast is
/// left. `discardable`: never sent, so `delete_tx_draft` takes it; it
/// refuses a `broadcast_pending` one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingSend {
    pub draft_id: String,
    pub action: String,
    pub signed: bool,
    pub discardable: bool,
}

/// A row's prices (R12), or why it has none: each case its own variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ListingPrices {
    /// No signed steps here: before Finalize & sign, or a Restored lock
    /// without its listing file.
    NoSteps,
    /// SPV or Explorer: no node can say the median time.
    Unverified,
    /// The node did not say its median time.
    CouldNotCheck { reason: String },
    /// The stored steps cannot be read (a corrupted row); every action of
    /// the row is refused in the same words.
    Unreadable { reason: String },
    /// The prices at the node's median time (`sell::step_view`).
    #[serde(rename_all = "camelCase")]
    At {
        current_price: Option<u64>,
        next_price: Option<u64>,
        next_valid_in_secs: Option<u64>,
        floor_price: u64,
    },
}

/// The prices of readable stored `steps` at the median time `chain` gives
/// (see [`my_listings_from_conn`]).
fn listing_prices(
    steps: &[sell::StoredStep],
    chain: Option<&Result<u64, String>>,
) -> ListingPrices {
    if steps.is_empty() {
        return ListingPrices::NoSteps;
    }
    match chain {
        None => ListingPrices::Unverified,
        Some(Err(reason)) => ListingPrices::CouldNotCheck {
            reason: reason.clone(),
        },
        Some(Ok(mtp)) => match sell::step_view(steps, *mtp) {
            Ok(None) => ListingPrices::NoSteps,
            Ok(Some(v)) => ListingPrices::At {
                current_price: v.current_price,
                next_price: v.next_price,
                next_valid_in_secs: v.next_valid_in_secs,
                floor_price: v.floor_price,
            },
            Err(e) => ListingPrices::Unreadable {
                reason: e.to_string(),
            },
        },
    }
}

/// One row of My listings.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MyListing {
    pub listing: ListingSummary,
    pub actions: ListingActions,
    pub prices: ListingPrices,
    pub pending_send: Option<PendingSend>,
    /// R42: another device or profile of the phrase manages the listing
    /// (R38's mark): this device no longer uploads it.
    pub managed_elsewhere: bool,
    /// R42: the price of the foreign step seen, when it reads.
    pub foreign_step_price_doos: Option<u64>,
    /// R43: a Sold listing whose purchase paid a price none of this
    /// device's steps names: a step set on another device.
    pub sold_elsewhere: bool,
    /// R43: a mined cancel this device has no cancel draft of (another
    /// device's or profile's, or a purchase of our own made elsewhere).
    pub cancelled_elsewhere: bool,
    /// R35: the window within which `listing.blocks_until_unbuyable` is
    /// shown, R9's 30 days, at most half the renewal window, in blocks
    /// (`NameParams::unbuyable_warn_blocks`).
    pub unbuyable_warn_blocks: u32,
    /// Whether blocks keep time on the profile's network, so that count may
    /// be shown as days (`Network::has_wall_clock_block_timing`).
    pub wall_clock_timing: bool,
}

/// R43: whether `l`, Sold, was bought at a price none of its stored
/// `steps` names: the wallet's coin at its payment address in the sale
/// transaction (a `0x84` step commits to that output, worth exactly the
/// step's price) is worth another amount. No coin recorded, or no payment
/// address (a lock restored by name), is no such fact: `false`.
fn sold_elsewhere(
    conn: &rusqlite::Connection,
    l: &ShakedexListing,
    steps: &[sell::StoredStep],
) -> Result<bool, AppError> {
    let (ListingState::Sold, Some(txid), Some(address)) = (
        l.state,
        l.sold_txid.as_deref(),
        l.payment_address.as_deref(),
    ) else {
        return Ok(false);
    };
    if steps.is_empty() {
        return Ok(false);
    }
    Ok(
        queries::own_coin_value_in_tx(conn, &l.wallet_profile_id, txid, address)?
            .is_some_and(|paid| !steps.iter().any(|s| i64::try_from(s.price) == Ok(paid))),
    )
}

/// R43: whether `l`'s mined cancel (`cancel_txid` in a cancel-mined state
/// or Cancelled) is not this device's own: no cancel draft here, or one
/// whose transaction is another.
fn cancelled_elsewhere(conn: &rusqlite::Connection, l: &ShakedexListing) -> Result<bool, AppError> {
    let mined = ListingState::CANCEL_MINED.contains(&l.state) || l.state == ListingState::Cancelled;
    let Some(txid) = l.cancel_txid.as_deref().filter(|_| mined) else {
        return Ok(false);
    };
    let ours = match l.cancel_draft_id.as_deref() {
        None => None,
        Some(id) => queries::get_tx_draft(conn, id)?.and_then(|d| d.txid),
    };
    Ok(!ours.is_some_and(|t| t.eq_ignore_ascii_case(txid)))
}

fn pending_send(
    conn: &rusqlite::Connection,
    l: &ShakedexListing,
) -> Result<Option<PendingSend>, AppError> {
    let id = match l.state {
        ListingState::Locking => l.lock_transfer_draft_id.as_deref(),
        ListingState::Finalizing => l.lock_finalize_draft_id.as_deref(),
        ListingState::Cancelling => l.cancel_draft_id.as_deref(),
        ListingState::CancelFinalizing => l.cancel_finalize_draft_id.as_deref(),
        _ => None,
    };
    let Some(id) = id else { return Ok(None) };
    Ok(queries::get_tx_draft(conn, id)?
        .filter(|d| queries::never_sent(d.status) || d.status == DraftStatus::BroadcastPending)
        .map(|d| PendingSend {
            signed: d.signed_tx_hex.is_some(),
            discardable: queries::never_sent(d.status),
            draft_id: d.id,
            action: d.action,
        }))
}

/// One row of My listings (T7): the listing, its actions and its unsent
/// draft. Each action is refused by the function its command runs first; a
/// listing whose stored steps cannot be read is listed with every action
/// refused, so it does not hide the others.
pub(crate) fn my_listing_from_conn(
    conn: &rusqlite::Connection,
    network: Network,
    l: &ShakedexListing,
    chain: Option<&Result<u64, String>>,
    buyable: &HashMap<String, i64>,
) -> Result<MyListing, AppError> {
    let unbuyable_warn_blocks = network.name_params().unbuyable_warn_blocks();
    let wall_clock_timing = network.has_wall_clock_block_timing();
    let steps = match readable_steps(l) {
        Ok(steps) => steps,
        // One unreadable row must not hide the others: it is listed with
        // every action that reads its steps refused in the read's words, and
        // nothing to send. The import reads none (the file replaces them).
        Err(e) => {
            let refused = || ListingAction::refused_by(Some(e.to_string()));
            return Ok(MyListing {
                // The degraded row must not fail on the lockup count either.
                listing: ListingSummary::with_steps(
                    l,
                    Vec::new(),
                    lockup_left(conn, l).ok().flatten(),
                ),
                actions: ListingActions {
                    finalize_and_sign: refused(),
                    lower_price: refused(),
                    cancel: refused(),
                    finalize_cancel: refused(),
                    export: refused(),
                    publish: refused(),
                    import_file: ListingAction::refused_by(
                        import_refusal(l.state, l.managed_elsewhere_at.is_some())
                            .map(str::to_string),
                    ),
                    manage_here: refused(),
                },
                prices: ListingPrices::Unreadable {
                    reason: e.to_string(),
                },
                pending_send: None,
                managed_elsewhere: shown_managed_elsewhere(l),
                foreign_step_price_doos: shown_managed_elsewhere(l)
                    .then(|| foreign_step_of(l).map(|s| s.price))
                    .flatten(),
                sold_elsewhere: false,
                cancelled_elsewhere: false,
                unbuyable_warn_blocks,
                wall_clock_timing,
            });
        }
    };
    let prices = listing_prices(&steps, chain);
    let mut listing = ListingSummary::with_steps(l, steps, lockup_left(conn, l)?);
    if counts_buyable_blocks(l.state) {
        listing.blocks_until_unbuyable = buyable.get(&l.name).copied();
    }
    let finalize = finalize_and_sign_refusal(conn, l)?;
    let lower = lower_refusal(
        l.state,
        listing.steps.is_empty(),
        l.managed_elsewhere_at.is_some(),
    )
    .map(str::to_string);
    // The stored lockup is the last sync's count; the command counts again
    // on the node's tip when it runs.
    let finalize_cancel = finalize_cancel_refusal(l.state)
        .map(str::to_string)
        .or_else(|| match l.cancel_blocks_remaining {
            None => Some(CANCEL_LOCKUP_NOT_COUNTED.to_string()),
            Some(n) => (n > 0).then(|| cancel_finalize_wait(n)),
        });
    Ok(MyListing {
        actions: ListingActions {
            finalize_and_sign: ListingAction::refused_by(finalize),
            lower_price: ListingAction::refused_by(lower),
            cancel: ListingAction::refused_by(cancel_refusal(l.state).map(str::to_string)),
            finalize_cancel: ListingAction::refused_by(finalize_cancel),
            export: ListingAction::refused_by(export_refusal(l).map(str::to_string)),
            publish: ListingAction::refused_by(publish_refusal(l, network)),
            import_file: ListingAction::refused_by(
                import_refusal(l.state, l.managed_elsewhere_at.is_some()).map(str::to_string),
            ),
            manage_here: ListingAction::refused_by(take_over_refusal(l).map(str::to_string)),
        },
        prices,
        pending_send: pending_send(conn, l)?,
        managed_elsewhere: shown_managed_elsewhere(l),
        foreign_step_price_doos: shown_managed_elsewhere(l)
            .then(|| foreign_step_of(l).map(|s| s.price))
            .flatten(),
        sold_elsewhere: sold_elsewhere(conn, l, &listing.steps)?,
        cancelled_elsewhere: cancelled_elsewhere(conn, l)?,
        listing,
        unbuyable_warn_blocks,
        wall_clock_timing,
    })
}

/// R35: whether a row in `state` says how long buyers still see it: a lock
/// on the market that the seller can still cancel (Listed, Restored). A
/// SalePending row is not counted: its purchase is already in the mempool
/// and Cancel is refused.
fn counts_buyable_blocks(state: ListingState) -> bool {
    matches!(state, ListingState::Listed | ListingState::Restored)
}

/// R35: the names whose expiry My listings asks the node for, those of rows
/// that say how long buyers still see them.
pub(crate) fn buyable_count_names(listings: &[ShakedexListing]) -> Vec<String> {
    let mut names: Vec<String> = listings
        .iter()
        .filter(|l| counts_buyable_blocks(l.state))
        .map(|l| l.name.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// The stored steps of `l` as My listings and publishing read them: steps
/// that cannot be parsed, or a lock time past R3's 40 bits, make a corrupted
/// row, refused in this error's words.
fn readable_steps(l: &ShakedexListing) -> Result<Vec<sell::StoredStep>, AppError> {
    shakedex_jobs::stored_steps(l)
        .and_then(|steps| sell::check_stored_steps(&steps).map(|()| steps))
}

/// Whether My listings needs the node's median time: only when a readable
/// row has steps (an unreadable row is listed as such).
pub(crate) fn needs_median_time(listings: &[ShakedexListing]) -> bool {
    listings
        .iter()
        .any(|l| readable_steps(l).is_ok_and(|s| !s.is_empty()))
}

/// Every listing of the profile, newest first, as My listings shows it, on
/// the profile's `network` (publishing is mainnet only). `chain` is the one
/// median time every row's prices are read at: `None`
/// when no node can say it (SPV, Explorer), `Some(Err(reason))` when the
/// node did not say it, `Some(Ok(mtp))`.
pub(crate) fn my_listings_from_conn(
    conn: &rusqlite::Connection,
    profile_id: &str,
    network: Network,
    chain: Option<Result<u64, String>>,
) -> Result<Vec<MyListing>, AppError> {
    my_listings_from_conn_at(conn, profile_id, network, chain, &HashMap::new())
}

/// [`my_listings_from_conn`] with the node's per-name count of blocks until
/// each listing stops being buyable (R35,
/// `verify::buyable_blocks_by_name`); a name missing from `buyable` is
/// unknown.
pub(crate) fn my_listings_from_conn_at(
    conn: &rusqlite::Connection,
    profile_id: &str,
    network: Network,
    chain: Option<Result<u64, String>>,
    buyable: &HashMap<String, i64>,
) -> Result<Vec<MyListing>, AppError> {
    queries::list_shakedex_listings(conn, profile_id)?
        .iter()
        .map(|l| my_listing_from_conn(conn, network, l, chain.as_ref(), buyable))
        .collect()
}

/// Why the listing file of `l` cannot leave the wallet (R23); `None` when
/// it can. The command and My listings ask this one function.
pub(crate) fn export_refusal(l: &ShakedexListing) -> Option<&'static str> {
    if !ListingState::EXPORT_ALLOWED.contains(&l.state) {
        return Some(listing_over(l.state).unwrap_or(
            "the listing file can be exported once the finalize into the lock is mined",
        ));
    }
    l.listing_file_json.is_none().then_some(EXPORT_NO_FILE)
}

/// R32, R45: why the saved listing file of a listing in `state`, marked
/// managed elsewhere or not (`managed_elsewhere`), cannot be imported;
/// `None` for a Restored lock, the only listing without its own file
/// details, and for a Listed listing managed elsewhere (the other device's
/// file takes it over). The import reads no stored step of a Restored lock:
/// the file replaces them. The command ([`restored_lock_for_file`]) and My
/// listings ask this one function.
pub(crate) fn import_refusal(state: ListingState, managed_elsewhere: bool) -> Option<&'static str> {
    match state {
        ListingState::Restored => None,
        ListingState::Listed if managed_elsewhere => None,
        _ => Some(
            "only a lock restored by name, or a listing managed from another device, takes its \
             saved listing file",
        ),
    }
}

/// Whether a listing file names a market fee: this wallet writes
/// `feeAddr: null` and every `fee` 0 (`listing_file::write_listing_file`).
fn names_a_market_fee(f: &ListingFile) -> bool {
    f.names_a_fee_address() || f.steps.iter().any(|s| s.fee != 0)
}

/// Why `l` cannot be published on LearnHNS Market now (R23, carries 1 and
/// 2 of T6); `None` when it can: on mainnet only, a Listed listing not
/// published yet, with its own file here naming no market fee (R25
/// compares the market's copy with one that has none, so a fee would be
/// uploaded again every hour). Publishing is never switched off: R28's
/// report of a mined cancel or sale reads it. The command and My listings
/// ask this one function.
pub(crate) fn publish_refusal(l: &ShakedexListing, network: Network) -> Option<String> {
    if network != Network::Main {
        return Some(MARKET_MAINNET_ONLY.into());
    }
    if l.publish {
        return Some("this listing is already published on LearnHNS Market".into());
    }
    if l.state != ListingState::Listed {
        return Some(
            match l.state {
                ListingState::Locking
                | ListingState::ReadyToFinalize
                | ListingState::Finalizing => {
                    "a listing is published once its finalize into the lock is mined"
                }
                other => listing_over(other)
                    .unwrap_or("only a Listed listing can be published on LearnHNS Market"),
            }
            .into(),
        );
    }
    let Some(text) = l.listing_file_json.as_deref() else {
        return Some(EXPORT_NO_FILE.into());
    };
    match ListingFile::parse(text, network) {
        Ok(f) if names_a_market_fee(&f) => Some(
            "this listing file names a market fee, which this wallet never writes: it is not \
             published"
                .into(),
        ),
        Ok(_) => None,
        Err(e) => Some(format!("this listing's file cannot be read: {e}")),
    }
}

/// R23 later (T7): publish one of the profile's listings on LearnHNS
/// Market, refused as its My listings row is (a corrupted row,
/// [`readable_steps`], then [`publish_refusal`]) and written by
/// `queries::publish_shakedex_listing`, which guards the same state again.
/// Writes the one field; the market jobs upload it from the next sync.
pub(crate) fn publish_listing_from_conn(
    conn: &rusqlite::Connection,
    profile_id: &str,
    network: Network,
    listing_id: &str,
) -> Result<ListingSummary, AppError> {
    let l = queries::get_shakedex_listing(conn, listing_id)?
        .filter(|l| l.wallet_profile_id == profile_id)
        .ok_or_else(|| AppError::NotFound(format!("listing {listing_id}")))?;
    // A corrupted row is refused as its My listings row is, in its words.
    readable_steps(&l)?;
    if let Some(why) = publish_refusal(&l, network) {
        return Err(AppError::InvalidInput(why));
    }
    if queries::publish_shakedex_listing(conn, profile_id, listing_id)? != 1 {
        return Err(AppError::InvalidInput(
            "this listing changed meanwhile: nothing was saved; try again".into(),
        ));
    }
    let l = queries::get_shakedex_listing(conn, listing_id)?
        .ok_or_else(|| AppError::Other("listing vanished after publish".into()))?;
    ListingSummary::of(conn, &l)
}

/// R23: write the saved listing file of one of the profile's listings to
/// `path` (chosen in the native save dialog), as written; refused as the
/// export is ([`export_listing_file_from_conn`]), writing nothing, and
/// refused for a symbolic link or a path that is not a regular file
/// ([`super::user_file::write_chosen_file`]).
pub(crate) fn save_listing_file_from_conn(
    conn: &rusqlite::Connection,
    profile_id: &str,
    listing_id: &str,
    path: &std::path::Path,
) -> Result<(), AppError> {
    let text = export_listing_file_from_conn(conn, profile_id, listing_id)?;
    // The file holds no secret (it is meant to be shared): not owner-only.
    super::user_file::write_chosen_file(path, text.as_bytes(), false)
}

/// What a restore by name (R32) decides from: every node read, and the lock
/// key already in hand.
pub(crate) struct RestoreInput<'a> {
    pub(crate) profile_id: &'a str,
    pub(crate) network: Network,
    pub(crate) name: &'a str,
    pub(crate) key: &'a LockKey,
    pub(crate) owner: &'a sell::RestoreOwner,
    pub(crate) coin: &'a rpc::NodeCoin,
}

/// R32: adopt the name's owner coin as a Restored lock when it is our lock
/// ([`sell::restore_verdict`]), keyed by name and that outpoint, with the
/// key's public key and no listing details (its mode is recorded as Buy Now
/// until its file is imported). Refused while a listing of the name is open,
/// or when a listing of the profile already holds that lock coin. Writes the
/// listing only.
pub(crate) fn restore_lock_inner(
    conn: &rusqlite::Connection,
    i: &RestoreInput,
) -> Result<ListingSummary, AppError> {
    if queries::open_shakedex_listing_for_name(conn, i.profile_id, i.name)?.is_some() {
        return Err(AppError::InvalidInput(format!(
            "'{}' is already tracked by one of this wallet's listings",
            i.name
        )));
    }
    if queries::shakedex_listing_holds_lock_coin(conn, i.profile_id, &i.owner.txid, i.owner.vout)? {
        return Err(AppError::InvalidInput(
            "this lock coin is already tracked by one of this wallet's listings".into(),
        ));
    }
    sell::restore_verdict(i.network, i.name, &i.key.address, i.owner, i.coin)?;
    let l = ShakedexListing {
        id: random_id(),
        wallet_profile_id: i.profile_id.into(),
        name: i.name.into(),
        mode: ListingMode::BuyNow,
        state: ListingState::Restored,
        lock_pubkey_hex: hex::encode(i.key.pubkey),
        lock_transfer_draft_id: None,
        lock_finalize_draft_id: None,
        lock_transfer_txid: None,
        lock_txid: Some(i.owner.txid.clone()),
        lock_vout: Some(i64::from(i.owner.vout)),
        payment_address: None,
        cancel_address: None,
        cancel_child_index: None,
        steps_json: "[]".into(),
        listing_file_json: None,
        publish: false,
        market_status: None,
        market_retry_at: None,
        market_attempts: 0,
        market_error: None,
        market_accepted: false,
        market_changed: false,
        market_told: false,
        market_sent_step: None,
        expires_at: None,
        abort_draft_id: None,
        abort_txid: None,
        sold_txid: None,
        cancel_txid: None,
        cancel_draft_id: None,
        cancel_vout: None,
        cancel_finalize_draft_id: None,
        cancel_blocks_remaining: None,
        managed_elsewhere_at: None,
        foreign_step_json: None,
        created_at: String::new(),
        updated_at: String::new(),
    };
    queries::insert_shakedex_listing(conn, &l)?;
    let l = queries::get_shakedex_listing(conn, &l.id)?
        .ok_or_else(|| AppError::Other("listing vanished after the restore".into()))?;
    ListingSummary::of(conn, &l)
}

/// R32: the active profile's Restored lock that `file` (our own listing
/// file, already parsed) is for: a Restored lock of the file's name (a
/// listing of it in any other state, or none, is refused) whose lock key and
/// lock outpoint the file carries, and a file paying one of this profile's
/// derived addresses (the parser has checked its network), or one of its
/// first [`OWN_FILE_RECEIVE_SEARCH_LIMIT`] receive addresses past them, which
/// is then derived and marked used ([`derivation::adopt_receive_address`]).
/// Writes nothing else.
pub(crate) fn restored_lock_for_file(
    conn: &rusqlite::Connection,
    profile_id: &str,
    file: &ListingFile,
) -> Result<ShakedexListing, AppError> {
    let listing = queries::open_shakedex_listing_for_name(conn, profile_id, &file.name)?
        .filter(|l| import_refusal(l.state, l.managed_elsewhere_at.is_some()).is_none())
        .ok_or_else(|| {
            AppError::InvalidInput(format!(
                "there is no Restored lock of '{}' here, nor a listing of it managed from \
                 another device: restore it by name first",
                file.name
            ))
        })?;
    if hex::encode(file.public_key) != listing.lock_pubkey_hex {
        return Err(AppError::InvalidInput(
            "this listing file is for another lock key".into(),
        ));
    }
    let lock_txid = hex::encode(file.lock_txid);
    if listing.lock_txid.as_deref() != Some(lock_txid.as_str())
        || listing.lock_vout != Some(i64::from(file.lock_vout))
    {
        return Err(AppError::InvalidInput(
            "this listing file is for another lock coin".into(),
        ));
    }
    // R45: a listing managed elsewhere keeps its own payment address; the
    // other device's file must pay it (the same lock, the same listing).
    if listing.state != ListingState::Restored {
        if listing.payment_address.as_deref() != Some(file.payment_addr.as_str()) {
            return Err(AppError::InvalidInput(IMPORT_OTHER_ADDRESS.into()));
        }
        return Ok(listing);
    }
    // The payment address was reserved by the profile that locked; one
    // restored from the same phrase holds only its restore window, so a
    // receive address past it is looked for (and adopted) up to the limit.
    let ours = queries::get_profile_addresses(conn, profile_id)?.contains(&file.payment_addr)
        || derivation::adopt_receive_address(
            conn,
            profile_id,
            &file.payment_addr,
            OWN_FILE_RECEIVE_SEARCH_LIMIT,
        )?
        .is_some();
    if !ours {
        return Err(AppError::InvalidInput(
            "this listing file's payment address is not one of this wallet's addresses".into(),
        ));
    }
    Ok(listing)
}

/// R45: the other device's file pays another address than the listing.
pub const IMPORT_OTHER_ADDRESS: &str = "this listing file pays another address than this \
     listing: it is not this listing's file";
/// R45: the other device's file leaves out a price this device signed.
pub const IMPORT_DROPS_OUR_STEP: &str = "this listing file leaves out a price this device \
     signed: it is not this listing's file as the other device holds it; nothing was saved";

/// R45: a Listed listing managed elsewhere takes over the other device's
/// own listing file, already matched to it ([`restored_lock_for_file`]:
/// the same lock outpoint, public key and payment address). The file's
/// public key must be the lock key this wallet derives for the name; every
/// stored step of ours must be in it (price, lock time, signature); the
/// lock coin must still be a coin (a spent one is refused with
/// `TAKE_OVER_LOCK_COIN_SPENT`, unlike R32's import), and every step of the
/// file must verify over it at hsd's value ([`sell::verify_file_steps`]).
/// The steps and file
/// become the file's (kept as parsed, unknown fields included), the mark is
/// cleared and `market_changed` set (`queries::take_over_listing`: still
/// Listed on that lock coin with the steps and foreign step read). Writes
/// nothing on a refusal.
pub(crate) fn take_over_with_file_inner(
    conn: &rusqlite::Connection,
    i: &OwnFileInput,
) -> Result<ListingSummary, AppError> {
    let file = i.file;
    let l = i.listing;
    if file.public_key != i.key.pubkey {
        return Err(AppError::InvalidInput(format!(
            "this listing file's public key is not the lock key this wallet derives for '{}'",
            file.name
        )));
    }
    let ours = readable_steps(l)?;
    let theirs: Vec<sell::StoredStep> = file
        .steps
        .iter()
        .map(|s| sell::StoredStep {
            price: s.price,
            lock_time: s.lock_time,
            signature: hex::encode(s.signature),
        })
        .collect();
    let lower = |s: &sell::StoredStep| (s.price, s.lock_time, s.signature.to_ascii_lowercase());
    if !ours
        .iter()
        .all(|o| theirs.iter().any(|t| lower(t) == lower(o)))
    {
        return Err(AppError::InvalidInput(IMPORT_DROPS_OUR_STEP.into()));
    }
    // Unlike R32's Restored lock, a take-over needs the lock coin unspent:
    // steps that cannot be verified are never stored over verified ones.
    let coin = i
        .coin
        .ok_or_else(|| AppError::InvalidInput(TAKE_OVER_LOCK_COIN_SPENT.into()))?;
    let lock_value = sell::lock_coin_value(coin, file, &i.key.address)?;
    sell::verify_file_steps(file, lock_value, i.network)?;
    let expires_at = file
        .expires_at
        .map(i64::try_from)
        .transpose()
        .map_err(|_| AppError::InvalidInput("the listing file's expiry is out of range".into()))?;
    let old_foreign = l
        .foreign_step_json
        .as_deref()
        .ok_or_else(|| AppError::InvalidInput(TAKE_OVER_NOT_MARKED.into()))?;
    let (lock_txid, lock_vout) = shakedex_jobs::stored_lock(l)?
        .ok_or_else(|| AppError::Other("corrupted listing: no lock coin".into()))?;
    let n = queries::take_over_listing(
        conn,
        &l.id,
        &queries::TakenOver {
            lock: (lock_txid, lock_vout),
            old_steps_json: &l.steps_json,
            old_foreign_step_json: old_foreign,
            steps_json: &serde_json::to_string(&theirs)?,
            listing_file_json: &file.to_json()?,
            expires_at,
        },
    )?;
    if n != 1 {
        return Err(AppError::InvalidInput(
            "this listing changed meanwhile: nothing was saved; try again".into(),
        ));
    }
    let l = queries::get_shakedex_listing(conn, &l.id)?
        .ok_or_else(|| AppError::Other("listing vanished after the import".into()))?;
    ListingSummary::of(conn, &l)
}

/// R32, R45: the upgrade of a Restored lock ([`upgrade_restored_lock_inner`])
/// or the take-over of a listing managed elsewhere
/// ([`take_over_with_file_inner`]) by its own listing file, as the listing
/// matched to the file stands.
pub(crate) fn import_own_file_inner(
    conn: &rusqlite::Connection,
    i: &OwnFileInput,
) -> Result<ListingSummary, AppError> {
    if i.listing.state == ListingState::Restored {
        upgrade_restored_lock_inner(conn, i)
    } else {
        take_over_with_file_inner(conn, i)
    }
}

/// R32: how many receive addresses of the profile an own listing file's
/// payment address is looked for among ([`restored_lock_for_file`]); a file
/// paying none of them is not ours, and nothing past them is derived.
pub const OWN_FILE_RECEIVE_SEARCH_LIMIT: u32 = 1000;

/// What the upgrade of a Restored lock decides from: the listing and the
/// file already matched ([`restored_lock_for_file`]), the lock coin as hsd
/// reports it (`None`: its 404, spent), and the lock key derived from the
/// seed after the node read.
pub(crate) struct OwnFileInput<'a> {
    pub(crate) network: Network,
    pub(crate) listing: &'a ShakedexListing,
    pub(crate) file: &'a ListingFile,
    pub(crate) key: &'a LockKey,
    pub(crate) coin: Option<&'a rpc::NodeCoin>,
}

/// R32: upgrade a Restored lock with its own listing file. The file's public
/// key must be the lock key this wallet derives for the name; while the lock
/// coin is a coin, every step must also be signed by that key over it
/// ([`sell::verify_file_steps`], at the coin's value as hsd reports
/// it). A spent lock coin leaves the outpoint and key to check: the chain
/// then judges the sale (R22) by the file's payment address. The listing
/// becomes Listed with the file's payment address, steps, expiry and the
/// mode it names (`ListingFile::mode`), or else the one its step times give
/// ([`sell::mode_of_step_times`]);
/// the file is kept as parsed (`ListingFile::to_json`, unknown fields
/// included). Writes nothing on a refusal.
pub(crate) fn upgrade_restored_lock_inner(
    conn: &rusqlite::Connection,
    i: &OwnFileInput,
) -> Result<ListingSummary, AppError> {
    let file = i.file;
    if file.public_key != i.key.pubkey {
        return Err(AppError::InvalidInput(format!(
            "this listing file's public key is not the lock key this wallet derives for '{}'",
            file.name
        )));
    }
    if let Some(coin) = i.coin {
        let lock_value = sell::lock_coin_value(coin, file, &i.key.address)?;
        sell::verify_file_steps(file, lock_value, i.network)?;
    }
    let steps: Vec<sell::StoredStep> = file
        .steps
        .iter()
        .map(|s| sell::StoredStep {
            price: s.price,
            lock_time: s.lock_time,
            signature: hex::encode(s.signature),
        })
        .collect();
    // The mode the file names; the step times only for a file without one.
    let mode = file
        .mode
        .unwrap_or_else(|| sell::mode_of_step_times(file.steps.iter().map(|s| s.lock_time)));
    let expires_at = file
        .expires_at
        .map(i64::try_from)
        .transpose()
        .map_err(|_| AppError::InvalidInput("the listing file's expiry is out of range".into()))?;
    let n = queries::upgrade_restored_lock(
        conn,
        &i.listing.id,
        &hex::encode(file.lock_txid),
        file.lock_vout,
        &hex::encode(file.public_key),
        &queries::UpgradedListing {
            mode,
            payment_address: &file.payment_addr,
            steps_json: &serde_json::to_string(&steps)?,
            listing_file_json: &file.to_json()?,
            expires_at,
        },
    )?;
    if n != 1 {
        return Err(AppError::InvalidInput(
            "this lock changed meanwhile: nothing was saved; try again".into(),
        ));
    }
    let l = queries::get_shakedex_listing(conn, &i.listing.id)?
        .ok_or_else(|| AppError::Other("listing vanished after the import".into()))?;
    ListingSummary::of(conn, &l)
}

// --- commands ---------------------------------------------------------------

/// One page of LearnHNS Market listings, each verified on the node of the
/// profile `wallet_profile_id` (the active one when none is named; R7, so a
/// read issued before a switch answers for its own profile). Only buyable
/// listings are returned; the rest are counted in `hidden`. In SPV and
/// Explorer modes every row is returned unverified.
#[tauri::command]
pub async fn shakedex_list_market(
    state: State<'_, AppState>,
    page: Option<u32>,
    wallet_profile_id: Option<String>,
) -> Result<MarketPage, AppError> {
    let profile_id = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        named_or_active_profile(&conn, wallet_profile_id)?
    };
    let ctx = browse_ctx_for(&state, &profile_id)?;
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
    let client = LearnHnsClient::from_settings(&ctx.settings)?;
    // No page asked for is the first page; pages count from 1.
    let page = page.unwrap_or(1).max(1);
    let (raw_rows, total) = client.list_available(page, MARKET_PER_PAGE).await?;
    // A total past u32 pages only caps how far the pager reaches; it decides
    // nothing about any listing.
    let page_count = u32::try_from(total.div_ceil(u64::from(MARKET_PER_PAGE)))
        .unwrap_or(u32::MAX)
        .max(1);
    let own = own_lock_addresses(&state, ctx.network)?;
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
                    listing_json: None,
                });
                continue;
            }
        };
        let verdict = match &at {
            None => {
                rows.push(mark_own(
                    market_row(text, &listing, unverified()),
                    &listing,
                    ctx.network,
                    &own,
                ));
                continue;
            }
            Some(Err(h)) => Verdict::Hidden(h.clone()),
            Some(Ok(at)) => verify::verify_listing_at(&ctx.node, ctx.network, &listing, *at).await,
        };
        match verdict {
            v @ Verdict::Buyable(_) => rows.push(mark_own(
                market_row(text, &listing, v),
                &listing,
                ctx.network,
                &own,
            )),
            Verdict::Hidden(h) => {
                hidden.count(&h);
                hidden_rows.push(HiddenRow {
                    name: Some(listing.name.clone()),
                    reason: h,
                    listing_json: Some(text),
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
            LearnHnsClient::from_settings(&ctx.settings)?
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
    let own = own_lock_addresses(&state, ctx.network)?;
    Ok(mark_own(
        market_row(text, &listing, verdict),
        &listing,
        ctx.network,
        &own,
    ))
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
        lockup_blocks: p.ctx.network.name_params().transfer_lockup,
        wall_clock_timing: p.ctx.network.has_wall_clock_block_timing(),
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
        let replaces = match old_status {
            Some(s) if queries::may_have_reached_chain(s) => {
                return Err(AppError::InvalidInput(
                    "a finalize of this purchase is already sent".into(),
                ));
            }
            Some(DraftStatus::Draft | DraftStatus::Signed) => p.finalize_draft_id.clone(),
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
    authorize_signer(&state, &ctx)?;
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
    let key = derive_listing_key(&state, &ctx, &name)?;
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

/// Finalize & sign, day 2 (R19, R20): see [`finalize_and_sign_confirmed`].
/// Buy Now: `prices` holds its one price and `schedule` is absent; a reverse
/// auction: `schedule` (R24) and no `prices`. Returns the listing, now
/// Finalizing; its `finalizeDraftId` is the signed FINALIZE, which
/// `broadcast_tx_draft` sends.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn shakedex_finalize_and_sign(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    listing_id: String,
    prices: Vec<StepInput>,
    schedule: Option<ScheduleInput>,
    fee_rate: Option<u64>,
) -> Result<ListingSummary, AppError> {
    finalize_and_sign_confirmed(
        &state,
        &app,
        &listing_id,
        &prices,
        schedule.as_ref(),
        fee_rate,
    )
    .await
}

/// R23: the saved listing file of one of the active profile's listings; see
/// [`export_listing_file_from_conn`].
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn shakedex_export_listing_file(
    state: State<'_, AppState>,
    listing_id: String,
) -> Result<String, AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let profile = draft_ctx::active_profile(&conn)?;
    export_listing_file_from_conn(&conn, &profile.id, &listing_id)
}

/// R23: save the listing file of one of the active profile's listings to
/// `path`, a path the renderer took from the native save dialog; see
/// [`save_listing_file_from_conn`].
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn shakedex_save_listing_file(
    state: State<'_, AppState>,
    listing_id: String,
    path: String,
) -> Result<(), AppError> {
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let profile = draft_ctx::active_profile(&conn)?;
    save_listing_file_from_conn(&conn, &profile.id, &listing_id, std::path::Path::new(&path))
}

/// R23 (carry 1 of T6): publish a Listed listing on LearnHNS Market after
/// the lock. Authorized as every selling write is (recovery-phrase profile,
/// a node that can send, the unlocked signer), then refused by
/// [`publish_refusal`] on the profile's network; see
/// [`publish_listing_from_conn`].
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn shakedex_publish_listing(
    state: State<'_, AppState>,
    listing_id: String,
) -> Result<ListingSummary, AppError> {
    let ctx = software_writer_ctx(&state)?;
    authorize_signer(&state, &ctx)?;
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    publish_listing_from_conn(&conn, &ctx.profile_id, ctx.network, &listing_id)
}

/// Restore a lock by name (R32): the name's owner coin is read on the node
/// (`getnameinfo`, then `GET /coin`), the lock key of `name` is derived from
/// the seed of the unlocked session after those reads (for its public key
/// and lock address only; nothing is signed), and the coin is adopted as a
/// Restored lock when it is a FINALIZE at that key's lock address
/// ([`restore_lock_inner`]). Acts on the active profile, which must be
/// seed-backed (R16: Ledger and watch-only profiles cannot derive the
/// hardened lock key); no experimental flag (the lock exists already, R15).
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn shakedex_restore_lock(
    state: State<'_, AppState>,
    name: String,
) -> Result<ListingSummary, AppError> {
    let ctx = software_writer_ctx(&state)?;
    authorize_signer(&state, &ctx)?;
    let reply = ctx.node.get_name_info(&name).await?;
    let owner = sell::restore_owner(&reply, &name)?;
    let coin = ctx
        .node
        .get_coin(&owner.txid, owner.vout)
        .await?
        .ok_or_else(|| AppError::InvalidInput(sell::RESTORE_SPENT_IN_MEMPOOL.into()))?;
    let key = derive_listing_key(&state, &ctx, &name)?;
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    restore_lock_inner(
        &conn,
        &RestoreInput {
            profile_id: &ctx.profile_id,
            network: ctx.network,
            name: &name,
            key: &key,
            owner: &owner,
            coin: &coin,
        },
    )
}

/// Import our own saved listing file for a Restored lock (R32): the file is
/// read by the strict parser (size limit first), a file charging a market
/// fee is refused (we never write one), the Restored lock it is for is found
/// ([`restored_lock_for_file`]), the lock coin is read with `GET /coin`, the
/// lock key of the name is derived from the seed of the unlocked session
/// after that read (to compare its public key; nothing is signed), and the
/// lock is upgraded ([`upgrade_restored_lock_inner`]). Acts on the active
/// profile, which must be seed-backed (R16); no experimental flag (R15).
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn shakedex_import_own_listing_file(
    state: State<'_, AppState>,
    text: String,
) -> Result<ListingSummary, AppError> {
    let ctx = software_writer_ctx(&state)?;
    authorize_signer(&state, &ctx)?;
    let file = ListingFile::parse(&text, ctx.network)?;
    if names_a_market_fee(&file) {
        return Err(AppError::InvalidInput(
            "this listing file names a market fee, which this wallet never writes: it is not \
             this wallet's own listing file"
                .into(),
        ));
    }
    let listing = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        restored_lock_for_file(&conn, &ctx.profile_id, &file)?
    };
    let coin = ctx
        .node
        .get_coin(&hex::encode(file.lock_txid), file.lock_vout)
        .await?;
    let key = derive_listing_key(&state, &ctx, &file.name)?;
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    import_own_file_inner(
        &conn,
        &OwnFileInput {
            network: ctx.network,
            listing: &listing,
            file: &file,
            key: &key,
            coin: coin.as_ref(),
        },
    )
}

/// R32: [`shakedex_import_own_listing_file`] for a file the renderer took
/// from the native open dialog. The writer gates come before the path is
/// opened; the file is read with the listing-file size limit
/// ([`read_listing_file`]) before anything is parsed.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn shakedex_import_own_listing_from_file(
    state: State<'_, AppState>,
    path: String,
) -> Result<ListingSummary, AppError> {
    let ctx = software_writer_ctx(&state)?;
    authorize_signer(&state, &ctx)?;
    let text = read_listing_file(&path)?;
    shakedex_import_own_listing_file(state, text).await
}

/// Cancel a listing (R28): see [`cancel_listing_confirmed`]. Returns the
/// signed cancel draft, which `broadcast_tx_draft` sends.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn shakedex_cancel_listing(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    listing_id: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    cancel_listing_confirmed(&state, &app, &listing_id, fee_rate).await
}

/// R28: hsd answers 404 for the cancel's TRANSFER: something spends it in a
/// block or in the mempool (a FINALIZE home, from here or another device),
/// or a reorg left the cancel in no block and no mempool. Only that is said.
pub const CANCEL_TRANSFER_SPENT: &str = "the cancel's transfer is no longer an unspent coin \
     on the node: it is on its way home, home, or undone by a reorg; nothing was built";

/// R28: the FINALIZE that brings a cancelled listing's name home, built
/// once its cancel TRANSFER is mined and the transfer lockup is over.
/// Everything is read on the node now, field by field: the cancel's
/// TRANSFER `(cancel_txid, cancel_vout)` a coin (hsd's 404, the TRANSFER
/// already spent by a FINALIZE from here or another device, is refused
/// first: nothing is built a second time), mined, a TRANSFER of the name at
/// the lock address of the stored key, committing to an address of this
/// profile's (the FINALIZE must pay the address the TRANSFER commits to);
/// the name live, unrevoked and owned by that TRANSFER; the lockup over at
/// tip + 1, counted from hsd's `info.transfer`; the TRANSFER's name height
/// the name's. The draft and the listing's move to CancelFinalizing commit
/// together; the usual confirm, sign and broadcast commands send it. No lock
/// key is needed: the FINALIZE's lock input carries only `[lockScript]`
/// (R1). No experimental flag (R15), and the signer is asked when the draft
/// is signed.
pub(crate) async fn finalize_cancel(
    state: &State<'_, AppState>,
    listing_id: &str,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    let ctx = software_writer_ctx(state)?;
    let (listing, own) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let l = queries::get_shakedex_listing(&conn, listing_id)?
            .filter(|l| l.wallet_profile_id == ctx.profile_id)
            .ok_or_else(|| AppError::NotFound(format!("listing {listing_id}")))?;
        if let Some(why) = finalize_cancel_refusal(l.state) {
            return Err(AppError::InvalidInput(why.into()));
        }
        // As My listings refuses it; the count itself is taken again on the
        // node's tip below.
        if l.cancel_blocks_remaining.is_none() {
            return Err(AppError::InvalidInput(CANCEL_LOCKUP_NOT_COUNTED.into()));
        }
        let own: std::collections::HashSet<String> =
            queries::get_profile_addresses(&conn, &ctx.profile_id)?
                .into_iter()
                .collect();
        (l, own)
    };
    let corrupted = |what: &str| AppError::Other(format!("corrupted listing: no {what}"));
    let (cancel_txid, cancel_vout) = shakedex_jobs::stored_cancel(&listing)?;
    let cancel_txid = cancel_txid.to_string();
    let pubkey = shakedex_jobs::lock_pubkey(&listing)?;
    let name = listing.name.clone();
    let at = shakedex_jobs::listing_lock(ctx.network, &listing)?;
    let coin = ctx
        .node
        .get_coin(&cancel_txid, cancel_vout)
        .await?
        .ok_or_else(|| AppError::InvalidInput(CANCEL_TRANSFER_SPENT.into()))?;
    let coin_at = match at.stored_coin(&coin, (&cancel_txid, cancel_vout), COV_TRANSFER) {
        Ok(coin_at) => coin_at,
        Err(sell::StoredCoinRefusal::Unreadable) => {
            return Err(cancel_could_not_check(
                "the cancel transfer's address or covenant",
            ));
        }
        Err(sell::StoredCoinRefusal::SomethingElse) => {
            return Err(AppError::Rpc(
                "node reported something else than this listing's cancel at its outpoint".into(),
            ));
        }
    };
    if coin.mined_height()?.is_none() {
        return Err(AppError::InvalidInput("the cancel is not mined yet".into()));
    }
    let dest = sell::commitment_address(coin_at.items, ctx.network)?
        .filter(|a| own.contains(a))
        .ok_or_else(|| {
            AppError::InvalidInput(
                "the cancel commits the name to an address that is not an address of this \
                 wallet: it cannot be finalized here"
                    .into(),
            )
        })?;
    let transfer_value = u64::try_from(coin.value)
        .map_err(|_| AppError::Rpc(format!("bad cancel coin value {}", coin.value)))?;
    let cov_height = coin
        .covenant
        .as_ref()
        .and_then(purchase::covenant_name_height)
        .ok_or_else(|| cancel_could_not_check("a readable name height in the cancel's transfer"))?;
    let reply = ctx.node.get_name_info(&name).await?;
    let info = match reply.get("info") {
        None => return Err(cancel_could_not_check("the name's info")),
        Some(serde_json::Value::Null) => {
            return Err(AppError::InvalidInput(format!(
                "'{name}' has no on-chain state or has expired"
            )))
        }
        Some(i) => i,
    };
    let owner = info.get("owner");
    let Some(owner_hash) = owner.and_then(|o| o.get("hash")).and_then(|h| h.as_str()) else {
        return Err(cancel_could_not_check("the name's owner"));
    };
    let Some(owner_index) = owner.and_then(|o| o.get("index")).and_then(|i| i.as_u64()) else {
        return Err(cancel_could_not_check("the name's owner output"));
    };
    let Some(revoked) = info.get("revoked").and_then(|r| r.as_u64()) else {
        return Err(cancel_could_not_check("whether the name was revoked"));
    };
    if revoked != 0
        || !owner_hash.eq_ignore_ascii_case(&cancel_txid)
        || owner_index != u64::from(cancel_vout)
    {
        return Err(AppError::InvalidInput(format!(
            "'{name}' is no longer held by the cancel's transfer: there is nothing to finalize"
        )));
    }
    let transfer_height = sell::transfer_height(info)
        .ok_or_else(|| cancel_could_not_check("the cancel transfer's block"))?;
    let tip = ctx
        .node
        .get_blockchain_info()
        .await
        .map_err(|e| cancel_could_not_check(&format!("its tip ({e})")))?
        .blocks;
    let remaining = ctx
        .network
        .name_params()
        .blocks_until_finalize(transfer_height, tip);
    if remaining > 0 {
        return Err(AppError::InvalidInput(cancel_finalize_wait(remaining)));
    }
    let ns = draft_ctx::name_state_strict(&reply, &name)?;
    if cov_height != ns.height {
        return Err(AppError::InvalidInput(
            "the name expired and was registered again since the cancel: it cannot be finalized"
                .into(),
        ));
    }
    let renewal_block = draft_ctx::renewal_block(&ctx.node, ctx.network).await?;
    let mut transfer_txid = [0u8; 32];
    hex::decode_to_slice(&cancel_txid, &mut transfer_txid)
        .map_err(|_| corrupted("readable cancel txid"))?;
    let res = cancel::build_cancel_finalize_plan(&cancel::CancelFinalizeInput {
        network: ctx.network,
        account: ctx.account,
        transfer_outpoint: (transfer_txid, cancel_vout),
        transfer_value,
        lock_pubkey: pubkey,
        name: &name,
        name_height: ns.height,
        weak: ns.weak,
        claimed: ns.claimed,
        renewals: ns.renewals,
        renewal_block,
        dest_address: &dest,
        funding: &ctx.funding,
        change_address: &ctx.change_address,
        rate: draft_ctx::fee_rate(&ctx, fee_rate),
        #[cfg(test)]
        fixed_fee: None,
    })?;
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    // The FINALIZE draft and the CancelFinalizing listing commit together.
    let tx = conn.unchecked_transaction()?;
    let draft_id = draft_ctx::persist_in_tx(
        &tx,
        &ctx.profile_id,
        &draft_ctx::DraftLabel {
            action: cancel::CANCEL_FINALIZE_ACTION,
            name: &name,
            recipient: Some(&dest),
            name_list: None,
            warnings: &[cancel::CANCEL_FINALIZE_NOTE.to_string()],
        },
        &res,
    )?;
    let n = queries::mark_listing_cancel_finalizing_in_tx(
        &tx,
        &listing.id,
        &draft_id,
        (&cancel_txid, cancel_vout),
    )?;
    if n != 1 {
        return Err(AppError::InvalidInput(
            "this listing changed meanwhile: nothing was saved; try again".into(),
        ));
    }
    tx.commit()?;
    draft_ctx::draft_summary(&conn, &draft_id)
}

/// The FINALIZE home of a cancelled listing (R28): see [`finalize_cancel`].
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn shakedex_finalize_cancel(
    state: State<'_, AppState>,
    listing_id: String,
    fee_rate: Option<u64>,
) -> Result<TxDraftSummary, AppError> {
    finalize_cancel(&state, &listing_id, fee_rate).await
}

/// "Manage from this device" (R44): see [`take_over_confirmed`]. Returns the
/// listing with the other device's step added.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn shakedex_manage_listing_here(
    state: State<'_, AppState>,
    listing_id: String,
) -> Result<ListingSummary, AppError> {
    take_over_confirmed(&state, &listing_id).await
}

/// Lower a listing's price (R26): see [`lower_price_confirmed`]. Returns the
/// listing with its new step.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn shakedex_lower_price(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    listing_id: String,
    price: String,
) -> Result<ListingSummary, AppError> {
    lower_price_confirmed(&state, &app, &listing_id, &price).await
}

/// The listings of the profile `wallet_profile_id` (the active one when
/// none is named; R7) ("My listings", T7), each with what it can do now and
/// its prices ([`my_listings_from_conn`]), on that profile's network and
/// node. Every row action is still refused by its command for a listing
/// that is not the active profile's (`l.wallet_profile_id == ctx.profile_id`). The node's median time
/// is read once for every row, and only when a row has steps.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn shakedex_list_listings(
    state: State<'_, AppState>,
    wallet_profile_id: Option<String>,
) -> Result<Vec<MyListing>, AppError> {
    // R7, per-wallet read isolation: the profile the caller names (the
    // active one when none is), so a read issued before a switch answers
    // for its own profile only.
    let profile_id = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        named_or_active_profile(&conn, wallet_profile_id)?
    };
    let (needs_chain, names) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let listings = queries::list_shakedex_listings(&conn, &profile_id)?;
        (needs_median_time(&listings), buyable_count_names(&listings))
    };
    let mut chain = None;
    let mut buyable = HashMap::new();
    if needs_chain || !names.is_empty() {
        let ctx = browse_ctx_for(&state, &profile_id)?;
        if ctx.can_verify() {
            let point = verify::chain_point(&ctx.node).await;
            if let Ok(p) = &point {
                // R35: one `getnameinfo` per name, at the tip just read.
                buyable =
                    verify::buyable_blocks_by_name(&ctx.node, ctx.network, &names, p.tip).await;
            }
            if needs_chain {
                chain = Some(point.map(|p| p.mtp).map_err(|h| match h {
                    Hidden::CouldNotCheck { reason } => reason,
                    // `chain_point` answers CouldNotCheck only.
                    other => format!("{other:?}"),
                }));
            }
        }
    }
    let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
    let network = queries::profile_network(&conn, &profile_id)?;
    my_listings_from_conn_at(&conn, &profile_id, network, chain, &buyable)
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

    /// Review M4: a market row's kind follows the step times, as the own-file
    /// import's mode does (`sell::mode_of_step_times`): a lowered Buy Now's
    /// two steps at one lock time are a Buy Now, steps a day apart a reverse
    /// auction.
    #[test]
    fn market_row_kind_follows_the_step_times() {
        let mut j: serde_json::Value = serde_json::from_str(LISTING_FILE).unwrap();
        let mut lowered = j["data"][0].clone();
        lowered["price"] = 200_000_000.into();
        j["data"].as_array_mut().unwrap().push(lowered);
        let same_time = ListingFile::parse(&j.to_string(), Network::Main).unwrap();
        let row = market_row(same_time.to_json().unwrap(), &same_time, unverified());
        assert_eq!(row.kind, ListingKind::BuyNow);
        let l = two_steps();
        let row = market_row(l.to_json().unwrap(), &l, unverified());
        assert_eq!(row.kind, ListingKind::ReverseAuction);
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

    /// A step the listing does not have pays no fee and shows no line,
    /// whatever its other steps' fees.
    #[test]
    fn a_step_past_the_listing_has_no_fee_line() {
        let (line, pay) = decide_market_fee(
            &listing(4_350_000, Some(addr())),
            1,
            Network::Main,
            true,
            true,
        );
        assert!(line.is_none() && pay.is_none());
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

    /// Every hidden reason the buy path refuses on is its own sentence, and
    /// every one is our refusal (InvalidInput), never a node error.
    #[test]
    fn each_hidden_reason_is_its_own_refusal() {
        for (h, words) in [
            (Hidden::SoldOrCancelled, "already sold or cancelled"),
            (
                Hidden::FailedVerification {
                    reason: "bad signature".into(),
                },
                "failed verification: bad signature",
            ),
            (
                Hidden::ExpiresBeforeFinalize,
                "expires before the purchase could be finalized",
            ),
            (Hidden::Unverified, "buying needs a full or remote node"),
        ] {
            let e = hidden_to_error(h.clone());
            assert!(
                matches!(&e, AppError::InvalidInput(m) if m.contains(words)),
                "{h:?}: {e:?}"
            );
        }
    }

    /// Each hidden reason tallies in its own counter; an Unverified row (never
    /// hidden in practice) lands with "could not check", the nearest one.
    #[test]
    fn hidden_counts_tally_each_reason_in_its_own_counter() {
        let mut c = HiddenCounts::default();
        c.count(&Hidden::SoldOrCancelled);
        for _ in 0..2 {
            c.count(&Hidden::FailedVerification { reason: "x".into() });
        }
        for _ in 0..3 {
            c.count(&Hidden::ExpiresBeforeFinalize);
        }
        for _ in 0..4 {
            c.count(&Hidden::NotYetValid {
                first_valid_in_secs: 1,
            });
        }
        c.count(&Hidden::CouldNotCheck { reason: "x".into() });
        c.count(&Hidden::Unverified);
        assert_eq!(
            (
                c.sold_or_cancelled,
                c.failed_verification,
                c.expires_before_finalize,
                c.not_yet_valid,
                c.could_not_check
            ),
            (1, 2, 3, 4, 2)
        );
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
