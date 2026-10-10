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
use crate::noncustodial::network::Network;
use crate::noncustodial::rpc::{self, ChainSource, NodeRpcClient};
use crate::noncustodial::send::{self, SpendableCoin, DUST_THRESHOLD};
use crate::noncustodial::session::{session_ttl_ms, SignerSession};
use crate::noncustodial::shakedex::cancel;
use crate::noncustodial::shakedex::listing_file::{
    write_listing_file, ListingFile, NewListingFile, PriceStep, MAX_LISTING_FILE_BYTES,
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
            expires_at: None,
            abort_draft_id: None,
            abort_txid: None,
            sold_txid: None,
            cancel_txid: None,
            cancel_draft_id: None,
            cancel_vout: None,
            cancel_finalize_draft_id: None,
            cancel_blocks_remaining: None,
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
fn authorize_signer(state: &State<'_, AppState>, ctx: &Ctx) -> Result<(), AppError> {
    with_signer(state, ctx, |_| Ok(()))
}

/// The lock key of `name` (ADR 0004), derived from the seed of the unlocked
/// session, which is authorized again. Called once every node read is done,
/// just before the key is used; it is never stored.
fn derive_listing_key(
    state: &State<'_, AppState>,
    ctx: &Ctx,
    name: &str,
) -> Result<LockKey, AppError> {
    with_signer(state, ctx, |session| {
        derive_lock_key(session.master()?, ctx.network, ctx.account, name)
    })
}

/// One price step the seller typed (R19): HNS as text, so the backend can
/// refuse what it cannot represent. T8 adds a schedule.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepInput {
    pub price: String,
}

/// Everything Finalize & sign has checked and built before it asks (R20).
pub(crate) struct PreparedLockFinalize {
    pub(crate) ctx: Ctx,
    pub(crate) listing: ShakedexListing,
    pub(crate) key: LockKey,
    /// The FINALIZE into the lock; the lock coin is its FINALIZE output
    /// (`sell::lock_output`).
    pub(crate) plan: actions::PlanResult,
    pub(crate) prices: Vec<u64>,
    /// R19: the Buy Now lock time, from the MTP at signing.
    pub(crate) lock_time: u64,
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
        match listing.state {
            ListingState::ReadyToFinalize => {}
            ListingState::Locking => {
                return Err(AppError::InvalidInput(
                    "Finalize & sign opens after the transfer lockup".into(),
                ))
            }
            other => {
                return Err(AppError::InvalidInput(
                    listing_over(other)
                        .unwrap_or("this listing is already finalized into its lock")
                        .into(),
                ))
            }
        }
        if listing.mode != ListingMode::BuyNow {
            return Err(AppError::InvalidInput(
                "reverse auctions are not supported yet".into(),
            ));
        }
        // The Cancel transfer and the FINALIZE spend the same TRANSFER coin.
        if let Some(cancel_id) = &listing.abort_draft_id {
            if let Some(cancel) = queries::get_tx_draft(&conn, cancel_id)? {
                if queries::draft_may_still_land(&cancel.status) {
                    return Err(AppError::InvalidInput(sell::CANCEL_TRANSFER_PENDING.into()));
                }
            }
        }
        let owner = queries::get_name_coin(&conn, &ctx.profile_id, &listing.name)?;
        (listing, owner)
    };
    if prices.len() != 1 {
        return Err(AppError::InvalidInput(
            "a Buy Now listing has exactly one price".into(),
        ));
    }
    let prices = prices
        .iter()
        .map(|p| sell::parse_step_price(&p.price))
        .collect::<Result<Vec<_>, _>>()?;
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
        .get_coin(&lock_transfer_txid, 0)
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
    Ok(PreparedLockFinalize {
        ctx,
        listing,
        key,
        plan,
        prices,
        lock_time: sell::buy_now_lock_time(mtp),
        mtp,
        payment_address,
    })
}

/// R20: the Finalize & sign confirmation's own title, never the
/// transaction prompt's.
pub const FINALIZE_AND_SIGN_TITLE: &str = "Confirm Finalize & sign";
const FINALIZE_AND_SIGN_MESSAGE: &str = "Review these details. This signs the finalize of \
     the name into its lock and every price below with the lock key.";

/// A listing as the UI reads it (T7's "My listings").
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
    /// The cancel's drafts, and blocks left until its FINALIZE, for T7's
    /// actions (R28).
    pub cancel_draft_id: Option<String>,
    pub cancel_finalize_draft_id: Option<String>,
    pub cancel_blocks_remaining: Option<i64>,
}

impl ListingSummary {
    fn of(l: &ShakedexListing) -> Result<Self, AppError> {
        let steps = serde_json::from_str(&l.steps_json)
            .map_err(|e| AppError::Other(format!("corrupted listing: unreadable steps: {e}")))?;
        Ok(Self {
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
        })
    }
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
    fee_rate: Option<u64>,
) -> Result<ListingSummary, AppError> {
    let p = prepare_lock_finalize(state, listing_id, prices, fee_rate).await?;
    let net = p.ctx.network;
    let (lock_vout, lock_value) = sell::lock_output(&p.plan.plan, &p.key.address)?;
    let steps: Vec<(u64, u64)> = p.prices.iter().map(|&price| (price, p.lock_time)).collect();
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
    ListingSummary::of(&l)
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

/// R28: the cancel's own prompt title, never the transaction prompt's.
pub const CANCEL_TITLE: &str = "Confirm cancel";
const CANCEL_MESSAGE: &str = "Review these details. This signs, with the lock key, a \
     transfer of the name out of its lock back to this wallet.";
/// R28: hsd answers 404 for a lock coin spent in a block or in its mempool.
pub const CANCEL_LOCK_COIN_SPENT: &str = "the lock coin is no longer unspent (bought, \
     cancelled, or being spent in the node's mempool): there is nothing to cancel";
/// T1b carry: the stored cancel address is not this profile's receive
/// address at the stored index.
pub const CANCEL_NOT_OUR_ADDRESS: &str = "this listing's cancel address is not this wallet's \
     reserved receive address: nothing was signed";
/// R28: a purchase the node already holds would beat the cancel.
const CANCEL_PURCHASE_PENDING: &str = "a purchase of this listing is in the node's mempool: \
     the node would not take a cancel now";
/// R28: the finalize into the lock is not mined, so the lock coin is not
/// there to spend yet.
const CANCEL_LOCK_NOT_MINED: &str = "the finalize into the lock is not mined yet: the listing \
     can be cancelled once it is";

/// Why Cancel refuses a listing in `state` (R28, deviation 6); `None` for
/// the two it acts on, Listed and Restored.
fn cancel_refusal(state: ListingState) -> Option<&'static str> {
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
}

/// "could not check" for a node reply the cancel reads.
fn cancel_could_not_check(what: &str) -> AppError {
    AppError::Rpc(format!(
        "node did not report {what}: could not check the cancel"
    ))
}

/// R28's checks, before the prompt: the gates (R16, R6; no experimental
/// flag, R15) and the unlocked signer first; the listing the active
/// profile's and Listed or Restored; on the node, the lock coin unspent,
/// mined, a FINALIZE of the name at the lock address of the stored key, of
/// the name's live registration, and the MTP (the current price the prompt
/// shows); the cancel address reserved (a lock restored by name reserves
/// one now, deviation 7) and the profile's receive address at its stored
/// index (T1b carry); the lock key re-derived after the reads, its public
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
    let lock_txid = listing
        .lock_txid
        .clone()
        .ok_or_else(|| corrupted("lock coin"))?;
    let lock_vout = listing
        .lock_vout
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| corrupted("lock output"))?;
    let pubkey: [u8; 33] = hex::decode(&listing.lock_pubkey_hex)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| corrupted("lock public key"))?;
    let name = listing.name.clone();
    let lock_addr = lock_address(ctx.network, &pubkey)?;
    let at = sell::ListingLock::new(lock_addr.clone(), &name)?;

    let coin = ctx
        .node
        .get_coin(&lock_txid, lock_vout)
        .await?
        .ok_or_else(|| AppError::InvalidInput(CANCEL_LOCK_COIN_SPENT.into()))?;
    let Some(coin_at) = sell::CoinAt::of_coin(&coin) else {
        return Err(cancel_could_not_check(
            "the lock coin's address or covenant",
        ));
    };
    if !(coin.txid.eq_ignore_ascii_case(&lock_txid)
        && coin.vout == lock_vout
        && at.holds(coin_at, COV_FINALIZE, None))
    {
        return Err(AppError::Rpc(
            "node reported something else than this listing's lock at its lock outpoint: could \
             not check the cancel"
                .into(),
        ));
    }
    if coin.mined_height()?.is_none() {
        return Err(AppError::InvalidInput(CANCEL_LOCK_NOT_MINED.into()));
    }
    let lock_value = u64::try_from(coin.value)
        .map_err(|_| AppError::Rpc(format!("bad lock coin value {}", coin.value)))?;
    let cov_height = coin
        .covenant
        .as_ref()
        .and_then(purchase::covenant_name_height)
        .ok_or_else(|| cancel_could_not_check("a readable name height in the lock coin"))?;
    let reply = ctx.node.get_name_info(&name).await?;
    if reply.get("info").is_some_and(serde_json::Value::is_null) {
        return Err(AppError::InvalidInput(format!(
            "'{name}' has no on-chain state or has expired: there is nothing to cancel"
        )));
    }
    let ns = draft_ctx::name_state_strict(&reply, &name)?;
    if ns.height != cov_height {
        return Err(AppError::InvalidInput(
            "the lock coin is left over from an earlier registration of the name: there is \
             nothing to cancel"
                .into(),
        ));
    }
    let steps: Vec<sell::StoredStep> = serde_json::from_str(&listing.steps_json)
        .map_err(|e| AppError::Other(format!("corrupted listing: unreadable steps: {e}")))?;
    // A lock restored by name stores no steps; a listing with steps but
    // none valid at the MTP says so (R3).
    let current_price = if steps.is_empty() {
        cancel::CancelPrice::NotKnown
    } else {
        let mtp = ctx
            .node
            .get_blockchain_info()
            .await
            .map_err(|e| cancel_could_not_check(&format!("its tip ({e})")))?
            .mediantime
            .ok_or_else(|| cancel_could_not_check("its median time (the current price)"))?;
        match sell::current_step_price(&steps, mtp)? {
            Some(price) => cancel::CancelPrice::Step(price),
            None => cancel::CancelPrice::NoneValidYet,
        }
    };

    // R21, deviation 7: a lock restored by name reserves its cancel address
    // now, after the node reads. The reservation and the row commit
    // together.
    let (cancel_address, cancel_index) =
        match (listing.cancel_address.clone(), listing.cancel_child_index) {
            (Some(a), Some(i)) => (a, u32::try_from(i).map_err(|_| corrupted("cancel index"))?),
            (None, None) if listing.state == ListingState::Restored => {
                let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
                let tx = conn.unchecked_transaction()?;
                let d = derivation::reserve_receive_address(&tx, &ctx.profile_id)?;
                if queries::set_restored_lock_cancel_address(
                    &tx,
                    &listing.id,
                    &d.address,
                    d.child_index,
                )? != 1
                {
                    return Err(AppError::InvalidInput(
                        "this lock changed meanwhile: nothing was signed; try again".into(),
                    ));
                }
                tx.commit()?;
                (d.address, d.child_index)
            }
            _ => return Err(corrupted("cancel address")),
        };
    // T1b carry: the profile's own receive address at the stored index.
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
        name_height: ns.height,
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
                cancel::CANCEL_STILL_BUYABLE.to_string(),
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
    if !ListingState::EXPORT_ALLOWED.contains(&l.state) {
        return Err(AppError::InvalidInput(
            listing_over(l.state)
                .unwrap_or(
                    "the listing file can be exported once the finalize into the lock is mined",
                )
                .into(),
        ));
    }
    l.listing_file_json.ok_or_else(|| {
        AppError::InvalidInput("this lock has no listing file here: import its saved file".into())
    })
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
        expires_at: None,
        abort_draft_id: None,
        abort_txid: None,
        sold_txid: None,
        cancel_txid: None,
        cancel_draft_id: None,
        cancel_vout: None,
        cancel_finalize_draft_id: None,
        cancel_blocks_remaining: None,
        created_at: String::new(),
        updated_at: String::new(),
    };
    queries::insert_shakedex_listing(conn, &l)?;
    let l = queries::get_shakedex_listing(conn, &l.id)?
        .ok_or_else(|| AppError::Other("listing vanished after the restore".into()))?;
    ListingSummary::of(&l)
}

/// R32: the active profile's Restored lock that `file` (our own listing
/// file, already parsed) is for: a Restored lock of the file's name (a
/// listing of it in any other state, or none, is refused) whose lock key and
/// lock outpoint the file carries, and a file paying one of this profile's
/// derived addresses (the parser has checked its network). Reads the
/// database only.
pub(crate) fn restored_lock_for_file(
    conn: &rusqlite::Connection,
    profile_id: &str,
    file: &ListingFile,
) -> Result<ShakedexListing, AppError> {
    let listing = queries::open_shakedex_listing_for_name(conn, profile_id, &file.name)?
        .filter(|l| l.state == ListingState::Restored)
        .ok_or_else(|| {
            AppError::InvalidInput(format!(
                "there is no Restored lock of '{}' here: restore it by name first",
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
    if !queries::get_profile_addresses(conn, profile_id)?.contains(&file.payment_addr) {
        return Err(AppError::InvalidInput(
            "this listing file's payment address is not one of this wallet's addresses".into(),
        ));
    }
    Ok(listing)
}

/// The value of the lock coin hsd reports for `file`'s lock outpoint, read
/// field by field: a coin of another outpoint, without its address, at
/// another address than `lock_address`, or with a negative value, is "could
/// not check".
fn lock_coin_value(
    coin: &rpc::NodeCoin,
    file: &ListingFile,
    lock_address: &str,
) -> Result<u64, AppError> {
    let lock_txid = hex::encode(file.lock_txid);
    if !(coin.txid == lock_txid && coin.vout == file.lock_vout) {
        return Err(AppError::Rpc(format!(
            "node answered coin {}:{} for the lock coin {lock_txid}:{}",
            coin.txid, coin.vout, file.lock_vout
        )));
    }
    let Some(address) = coin.address.as_deref() else {
        return Err(AppError::Rpc(
            "node did not report the lock coin's address".into(),
        ));
    };
    if address != lock_address {
        return Err(AppError::Rpc(format!(
            "node reported the lock coin at {address}, not at this lock's address"
        )));
    }
    u64::try_from(coin.value).map_err(|_| {
        AppError::Rpc(format!(
            "node reported a lock coin value of {} doos",
            coin.value
        ))
    })
}

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
/// (`template::verify_step_signature`, at the coin's value as hsd reports
/// it). A spent lock coin leaves the outpoint and key to check: the chain
/// then judges the sale (R22) by the file's payment address. The listing
/// becomes Listed with the file's payment address, steps, expiry and mode;
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
        let lock_value = lock_coin_value(coin, file, &i.key.address)?;
        let payment = output_address_from_string(i.network, &file.payment_addr)?;
        for step in &file.steps {
            template::verify_step_signature(
                &template::StepTemplate {
                    lock_outpoint: (file.lock_txid, file.lock_vout),
                    lock_value,
                    lock_pubkey: &file.public_key,
                    payment: payment.clone(),
                    price: step.price,
                    lock_time_secs: step.lock_time,
                },
                &step.signature,
            )
            .map_err(|_| {
                AppError::InvalidInput(
                    "a price in this listing file is not signed by this lock".into(),
                )
            })?;
        }
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
    let mode = if file.steps.len() == 1 {
        ListingMode::BuyNow
    } else {
        ListingMode::ReverseAuction
    };
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
    ListingSummary::of(&l)
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
/// Returns the listing, now Finalizing; its `finalizeDraftId` is the signed
/// FINALIZE, which `broadcast_tx_draft` sends.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn shakedex_finalize_and_sign(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    listing_id: String,
    prices: Vec<StepInput>,
    fee_rate: Option<u64>,
) -> Result<ListingSummary, AppError> {
    finalize_and_sign_confirmed(&state, &app, &listing_id, &prices, fee_rate).await
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
    // This wallet writes `feeAddr: null` and every `fee` 0
    // (`listing_file::write_listing_file`).
    if file.names_a_fee_address() || file.steps.iter().any(|s| s.fee != 0) {
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
    upgrade_restored_lock_inner(
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
