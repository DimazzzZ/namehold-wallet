//! OS notifications + deadline scanner (I1).
//!
//! Two deadlines are worth an OS notification even when the app isn't in the
//! foreground:
//!   * a BID's reveal window closing (miss it → the entire lockup is
//!     forfeit — see `014_reveal_end_height.sql` for how the close height is
//!     estimated);
//!   * a name's renewal window closing (miss it → the name is lost) — reuses
//!     Task 3's live `compute_renewals` days-until-expire, never recomputed
//!     here.
//!
//! The scanner core ([`scan_deadlines`]) is a PURE function: deadlines +
//! config + previously-notified state → notifications to emit + new state.
//! All IO (DB reads, settings, the actual OS notification call) lives in the
//! `#[tauri::command]` shell below, which is what both the app-start call and
//! the ~10-minute background timer (wired in `lib.rs`) invoke.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::queries;
use crate::error::AppError;
use crate::AppState;

// ---------------------------------------------------------------------------
// Pure core
// ---------------------------------------------------------------------------

/// One un-revealed bid whose reveal window is tracked.
#[derive(Debug, Clone, PartialEq)]
pub struct RevealDeadline {
    pub wallet_profile_id: String,
    pub name: String,
    /// `reveal_end_height - current_height`. Negative means the window has
    /// already closed (kept, not filtered, so a missed deadline still
    /// surfaces once — mirrors `RenewalRow::expiring_soon`'s "incl. negative
    /// = lapsed" convention).
    pub blocks_remaining: i64,
    /// The estimated height this specific auction's reveal window closes at
    /// — part of the dedup key (see `reveal_key`) so a lapsed, un-revealed
    /// bid on a name does not permanently silence alarms for a LATER,
    /// unrelated auction on the SAME name (review Finding 1: a name-scoped
    /// key would stay in `active_episodes` forever since
    /// `list_pending_reveal_deadlines` keeps returning the dead row, making
    /// any future re-bid on that name look like "already notified").
    pub reveal_end_height: i64,
}

/// One owned name's renewal deadline.
#[derive(Debug, Clone, PartialEq)]
pub struct RenewalDeadline {
    pub wallet_profile_id: String,
    pub name: String,
    pub days_remaining: f64,
    /// The height this specific renewal cycle expires at, when known — part
    /// of the dedup key (see `renewal_key`) for the same reason
    /// `reveal_end_height` is part of `reveal_key`: without it, a renewal
    /// episode "frozen" in `active_episodes` while notifications are
    /// disabled (Minor 7) would swallow the notification for the NEXT
    /// renewal cycle too, since a bare `renewal:{profile}:{name}` key never
    /// changes across cycles. `None` when the source row has no known expiry
    /// height (e.g. a CSV-import fallback row with only `days_until_expire`)
    /// — the key then falls back to the pre-fix unscoped form, an honest
    /// "can't tell episodes apart" rather than a guess.
    pub expires_at_height: Option<i64>,
}

/// User-configurable scan behavior, loaded from the generic `settings` KV
/// table (see `load_config`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeadlineNotifyConfig {
    pub enabled: bool,
    pub reveal_lead_blocks: i64,
    pub renewal_lead_days: f64,
}

impl Default for DeadlineNotifyConfig {
    fn default() -> Self {
        // Opt-in: no OS permission prompt / notification fires until the
        // user explicitly turns this on in Settings.
        Self {
            enabled: false,
            // ~1 day of mainnet blocks — enough runway to build+broadcast a
            // reveal, not so early it's noise.
            reveal_lead_blocks: 144,
            // Matches names::EXPIRING_SOON_THRESHOLD_DAYS by default, but is
            // independently configurable (notifications vs. in-app coloring
            // are different urgency knobs).
            renewal_lead_days: crate::commands::names::EXPIRING_SOON_THRESHOLD_DAYS,
        }
    }
}

/// One notification the shell should hand to the OS.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingNotification {
    /// Dedup identity — also the persisted "episode" key. Stable across
    /// scans for the same name+kind so a still-imminent deadline is not
    /// re-notified every tick.
    pub key: String,
    pub title: String,
    pub body: String,
}

/// Output of one pure scan pass.
#[derive(Debug, Clone, PartialEq)]
pub struct ScanResult {
    pub notifications: Vec<PendingNotification>,
    /// The dedup state to persist for the NEXT scan. Keys present here but
    /// absent from the input `previously_notified` are exactly the ones in
    /// `notifications` (newly-entered episodes); keys that fell out of the
    /// imminent set (deadline resolved — revealed, renewed, or simply no
    /// longer in the input list) are dropped, so the SAME name can notify
    /// again on a future, unrelated episode (e.g. next year's renewal).
    pub active_episodes: BTreeSet<String>,
}

/// Auction-scoped: includes `reveal_end_height` so a NEW auction on the same
/// name (a fresh `reveal_end_height`) is a genuinely new episode, never
/// mistaken for a lapsed earlier one still sitting in `active_episodes` (see
/// `RevealDeadline::reveal_end_height` doc, review Finding 1).
fn reveal_key(d: &RevealDeadline) -> String {
    format!(
        "reveal:{}:{}:{}",
        d.wallet_profile_id, d.name, d.reveal_end_height
    )
}

/// Cycle-scoped when the expiry height is known (see
/// `RenewalDeadline::expires_at_height` doc, review Minor 7); falls back to
/// the un-scoped `renewal:{profile}:{name}` form otherwise.
fn renewal_key(d: &RenewalDeadline) -> String {
    match d.expires_at_height {
        Some(h) => format!("renewal:{}:{}:{}", d.wallet_profile_id, d.name, h),
        None => format!("renewal:{}:{}", d.wallet_profile_id, d.name),
    }
}

/// A reveal deadline is dead once its window closed more than one full
/// reveal-period ago: a final "you missed it" alarm already fired for it
/// while `blocks_remaining` was still within `[-reveal_period_blocks, 0]`
/// (see `negative_remaining_still_notifies_once_missed_deadline`), so
/// keeping it in the scanner's input forever serves no purpose — it would
/// just be an ever-growing set of permanently-un-revealed bids re-evaluated,
/// and persisted, on every single tick (review Finding 1b). Pure so it's
/// directly unit-tested without touching the DB; the actual filtering lives
/// in `collect_deadlines`, the one place that has both a row's
/// `blocks_remaining` and its network's `reveal_period`.
fn is_reveal_deadline_stale(blocks_remaining: i64, reveal_period_blocks: i64) -> bool {
    blocks_remaining < -reveal_period_blocks
}

/// Pure scanner core. Given the full set of currently-known reveal/renewal
/// deadlines (already computed elsewhere — this function does no chain math),
/// the user's config, and the dedup state left over from the previous scan,
/// decide what to notify now and what state to persist next.
///
/// When `config.enabled` is `false` this is a no-op passthrough: no
/// notifications, and `previously_notified` is echoed back unchanged (rather
/// than cleared) so re-enabling later doesn't re-fire everything that was
/// already acknowledged before the user turned notifications off.
pub fn scan_deadlines(
    reveal: &[RevealDeadline],
    renewal: &[RenewalDeadline],
    config: &DeadlineNotifyConfig,
    previously_notified: &BTreeSet<String>,
) -> ScanResult {
    if !config.enabled {
        return ScanResult {
            notifications: Vec::new(),
            active_episodes: previously_notified.clone(),
        };
    }

    let mut active = BTreeSet::new();
    let mut notifications = Vec::new();

    for d in reveal {
        if d.blocks_remaining > config.reveal_lead_blocks {
            continue; // not imminent yet
        }
        let key = reveal_key(d);
        if !previously_notified.contains(&key) {
            notifications.push(PendingNotification {
                key: key.clone(),
                title: "Reveal window closing".into(),
                body: format!(
                    "{} — reveal in {} blocks or the bid lockup is forfeit",
                    d.name, d.blocks_remaining
                ),
            });
        }
        active.insert(key);
    }

    for d in renewal {
        if d.days_remaining > config.renewal_lead_days {
            continue;
        }
        let key = renewal_key(d);
        if !previously_notified.contains(&key) {
            notifications.push(PendingNotification {
                key: key.clone(),
                title: "Renewal due soon".into(),
                body: format!("{} — expires in {:.1} days", d.name, d.days_remaining),
            });
        }
        active.insert(key);
    }

    ScanResult {
        notifications,
        active_episodes: active,
    }
}

// ---------------------------------------------------------------------------
// IO shell
// ---------------------------------------------------------------------------

const SETTING_ENABLED: &str = "deadline_notify_enabled";
const SETTING_REVEAL_LEAD_BLOCKS: &str = "deadline_notify_reveal_lead_blocks";
const SETTING_RENEWAL_LEAD_DAYS: &str = "deadline_notify_renewal_lead_days";
const SETTING_STATE: &str = "deadline_notify_state";

fn load_config(settings: &std::collections::HashMap<String, String>) -> DeadlineNotifyConfig {
    let default = DeadlineNotifyConfig::default();
    DeadlineNotifyConfig {
        enabled: settings
            .get(SETTING_ENABLED)
            .map(|v| v == "true")
            .unwrap_or(default.enabled),
        reveal_lead_blocks: settings
            .get(SETTING_REVEAL_LEAD_BLOCKS)
            .and_then(|v| v.parse().ok())
            .unwrap_or(default.reveal_lead_blocks),
        renewal_lead_days: settings
            .get(SETTING_RENEWAL_LEAD_DAYS)
            .and_then(|v| v.parse().ok())
            .unwrap_or(default.renewal_lead_days),
    }
}

fn load_state(settings: &std::collections::HashMap<String, String>) -> BTreeSet<String> {
    settings
        .get(SETTING_STATE)
        .and_then(|v| serde_json::from_str::<Vec<String>>(v).ok())
        .map(|v| v.into_iter().collect())
        .unwrap_or_default()
}

/// Pull every reveal + renewal deadline across ALL wallet profiles (deadlines
/// matter regardless of which profile happens to be "active" when the
/// background timer fires) and the best available current height per
/// profile — reusing `compute_renewals`' own height resolution
/// (node-live-when-synced, else persisted-estimate) rather than
/// re-implementing it.
fn collect_deadlines(
    conn: &rusqlite::Connection,
    live_node_height: Option<i64>,
) -> Result<(Vec<RevealDeadline>, Vec<RenewalDeadline>), AppError> {
    use crate::noncustodial::network::Network;

    let mut reveal_out = Vec::new();
    let mut renewal_out = Vec::new();

    // Reveal deadlines: every un-revealed bid with a known reveal_end_height
    // estimate, across all profiles.
    let reveal_rows = queries::list_pending_reveal_deadlines(conn)?;
    // Cache each profile's current-height resolution — `compute_renewals`
    // below already does this per profile; reuse its `current_height` so we
    // don't compute it twice — plus its network, needed to know that
    // profile's `reveal_period` for the staleness cutoff below (Finding 1b).
    let mut profile_ctx: std::collections::HashMap<String, (Option<i64>, Network)> =
        std::collections::HashMap::new();

    for profile in queries::list_wallet_profiles(conn)? {
        let renewals =
            crate::commands::read::compute_renewals(conn, &profile.id, live_node_height)?;
        let network = Network::from_str_opt(&profile.network).unwrap_or_default();
        profile_ctx.insert(profile.id.clone(), (renewals.current_height, network));
        for row in renewals.names {
            if let Some(days) = row.days_until_expire {
                renewal_out.push(RenewalDeadline {
                    wallet_profile_id: profile.id.clone(),
                    name: row.name,
                    days_remaining: days,
                    expires_at_height: row.expires_at_height,
                });
            }
        }
    }

    for (profile_id, name, reveal_end_height) in reveal_rows {
        let Some((Some(current_height), network)) = profile_ctx.get(&profile_id).cloned() else {
            // No height known for this profile at all (never synced, no
            // persisted estimate) — cannot honestly compute a blocks-
            // remaining figure, so skip rather than fabricate one.
            continue;
        };
        let blocks_remaining = reveal_end_height - current_height;
        // Finding 1b: a reveal window that closed more than one full
        // reveal-period ago is dead — one final alarm already fired for it —
        // so drop it here rather than let `list_pending_reveal_deadlines`'
        // unbounded `reveal_txid IS NULL` result linger in the scanner's
        // input (and its persisted episode key) forever.
        let reveal_period = network.name_params().reveal_period as i64;
        if is_reveal_deadline_stale(blocks_remaining, reveal_period) {
            continue;
        }
        reveal_out.push(RevealDeadline {
            wallet_profile_id: profile_id,
            name,
            blocks_remaining,
            reveal_end_height,
        });
    }

    Ok((reveal_out, renewal_out))
}

/// Best-effort OS notification dispatch. Never returns an error to the
/// caller — a denied/unavailable OS permission must not break the scan or
/// crash the app; the outcome's `delivery_error` communicates it to the UI
/// instead.
#[cfg(not(test))]
fn send_os_notification<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    n: &PendingNotification,
) -> Option<String> {
    use tauri_plugin_notification::NotificationExt;
    match app
        .notification()
        .builder()
        .title(&n.title)
        .body(&n.body)
        .show()
    {
        Ok(()) => None,
        Err(e) => Some(e.to_string()),
    }
}

/// Outcome of one scan pass, returned to the frontend / used in tests.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScanOutcome {
    pub enabled: bool,
    pub notified: Vec<PendingNotification>,
    /// Set when at least one notification's OS dispatch failed (e.g. denied
    /// permission on macOS) — surfaced so Settings can show an honest status
    /// instead of silently doing nothing.
    pub delivery_error: Option<String>,
}

/// Run one scan pass: load config + dedup state, gather deadlines, decide
/// what's newly imminent, dispatch OS notifications for it, and persist the
/// updated dedup state. Called both by the ~10-minute background timer
/// (`lib.rs` setup) and directly as a command (e.g. a manual "check now").
#[tauri::command]
pub async fn scan_deadline_notifications<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<ScanOutcome, AppError> {
    // Finding 3: load config FIRST and bail out immediately when disabled —
    // BEFORE probing the node, collecting deadlines (a per-profile
    // `compute_renewals` walk plus the reveal-deadline query), or touching
    // `deadline_notify_state` at all. A background timer ticking every ~10
    // minutes for a user who has the feature off should do no chain probing
    // and no DB writes on any tick. The persisted dedup state is left
    // exactly as it was — not read, let alone rewritten — matching
    // `scan_deadlines`'s own disabled branch (state is frozen, never
    // cleared, so re-enabling later doesn't re-fire a notification storm).
    let config = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        load_config(&queries::get_settings(&conn)?)
    };
    if !config.enabled {
        return Ok(ScanOutcome {
            enabled: false,
            notified: Vec::new(),
            delivery_error: None,
        });
    }

    // Probe the node BEFORE taking the DB lock (guard is !Send across await,
    // same discipline as `read_renewals`).
    let live_height = crate::commands::read::node_tip_height_if_synced(&state).await;

    let (previously_notified, reveal, renewal) = {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let previously_notified = load_state(&queries::get_settings(&conn)?);
        let (reveal, renewal) = collect_deadlines(&conn, live_height)?;
        (previously_notified, reveal, renewal)
    };

    let result = scan_deadlines(&reveal, &renewal, &config, &previously_notified);

    #[cfg_attr(test, allow(unused_mut))]
    let mut delivery_error = None;
    #[cfg(not(test))]
    for n in &result.notifications {
        if let Some(e) = send_os_notification(&app, n) {
            delivery_error = Some(e);
        }
    }
    #[cfg(test)]
    let _ = &app; // unused in unit/integration test builds (no real OS notification)

    {
        let conn = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        let state_json = serde_json::to_string(&result.active_episodes.iter().collect::<Vec<_>>())?;
        queries::set_setting(&conn, SETTING_STATE, &state_json)?;
    }

    Ok(ScanOutcome {
        enabled: config.enabled,
        notified: result.notifications,
        delivery_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(enabled: bool) -> DeadlineNotifyConfig {
        DeadlineNotifyConfig {
            enabled,
            reveal_lead_blocks: 144,
            renewal_lead_days: 30.0,
        }
    }

    // A fixed sentinel `reveal_end_height` for tests that only care about
    // `blocks_remaining` moving tick-to-tick within the SAME auction — the
    // key must stay stable across those calls. Tests that specifically
    // exercise auction-scoping (Finding 1a) use `reveal_at` instead.
    const SENTINEL_REVEAL_END_HEIGHT: i64 = 999_999;

    fn reveal(profile: &str, name: &str, blocks_remaining: i64) -> RevealDeadline {
        reveal_at(profile, name, blocks_remaining, SENTINEL_REVEAL_END_HEIGHT)
    }

    fn reveal_at(
        profile: &str,
        name: &str,
        blocks_remaining: i64,
        reveal_end_height: i64,
    ) -> RevealDeadline {
        RevealDeadline {
            wallet_profile_id: profile.into(),
            name: name.into(),
            blocks_remaining,
            reveal_end_height,
        }
    }

    fn renewal(profile: &str, name: &str, days_remaining: f64) -> RenewalDeadline {
        RenewalDeadline {
            wallet_profile_id: profile.into(),
            name: name.into(),
            days_remaining,
            expires_at_height: None,
        }
    }

    fn renewal_at(
        profile: &str,
        name: &str,
        days_remaining: f64,
        expires_at_height: i64,
    ) -> RenewalDeadline {
        RenewalDeadline {
            wallet_profile_id: profile.into(),
            name: name.into(),
            days_remaining,
            expires_at_height: Some(expires_at_height),
        }
    }

    // --- emission ------------------------------------------------------

    #[test]
    fn emits_reveal_notification_when_within_lead_blocks() {
        let result = scan_deadlines(
            &[reveal("p1", "namea", 50)],
            &[],
            &cfg(true),
            &Default::default(),
        );
        assert_eq!(result.notifications.len(), 1);
        assert_eq!(
            result.notifications[0].key,
            format!("reveal:p1:namea:{SENTINEL_REVEAL_END_HEIGHT}")
        );
        assert!(result.notifications[0].body.contains("namea"));
        assert!(result
            .active_episodes
            .contains(&result.notifications[0].key));
    }

    #[test]
    fn emits_renewal_notification_when_within_lead_days() {
        let result = scan_deadlines(
            &[],
            &[renewal("p1", "nameb", 5.0)],
            &cfg(true),
            &Default::default(),
        );
        assert_eq!(result.notifications.len(), 1);
        assert_eq!(result.notifications[0].key, "renewal:p1:nameb");
        assert!(result.active_episodes.contains("renewal:p1:nameb"));
    }

    #[test]
    fn does_not_emit_when_deadline_is_far_out() {
        let result = scan_deadlines(
            &[reveal("p1", "namea", 10_000)],
            &[renewal("p1", "nameb", 90.0)],
            &cfg(true),
            &Default::default(),
        );
        assert!(result.notifications.is_empty());
        assert!(result.active_episodes.is_empty());
    }

    #[test]
    fn negative_remaining_still_notifies_once_missed_deadline() {
        // A window that already closed is still worth ONE alert (the user
        // may not have seen the app in days) — matches RenewalRow's
        // documented "incl. negative = lapsed" convention.
        let result = scan_deadlines(
            &[reveal("p1", "namea", -20)],
            &[],
            &cfg(true),
            &Default::default(),
        );
        assert_eq!(result.notifications.len(), 1);
    }

    // --- thresholds (boundary) -----------------------------------------

    #[test]
    fn boundary_exactly_at_lead_time_is_imminent() {
        let result = scan_deadlines(
            &[reveal("p1", "namea", 144)],
            &[],
            &cfg(true),
            &Default::default(),
        );
        assert_eq!(
            result.notifications.len(),
            1,
            "== lead_blocks must count as imminent"
        );
    }

    #[test]
    fn boundary_one_block_past_lead_time_is_not_imminent() {
        let result = scan_deadlines(
            &[reveal("p1", "namea", 145)],
            &[],
            &cfg(true),
            &Default::default(),
        );
        assert!(result.notifications.is_empty());
    }

    // --- dedup -----------------------------------------------------------

    #[test]
    fn does_not_renotify_same_episode_on_next_tick() {
        let first = scan_deadlines(
            &[reveal("p1", "namea", 50)],
            &[],
            &cfg(true),
            &Default::default(),
        );
        assert_eq!(first.notifications.len(), 1);

        // Next tick, still imminent, same name — dedup state carried over.
        let second = scan_deadlines(
            &[reveal("p1", "namea", 40)],
            &[],
            &cfg(true),
            &first.active_episodes,
        );
        assert!(
            second.notifications.is_empty(),
            "already-notified episode must not re-fire"
        );
        assert!(second
            .active_episodes
            .contains(&format!("reveal:p1:namea:{SENTINEL_REVEAL_END_HEIGHT}")));
    }

    #[test]
    fn renotifies_after_episode_resolves_and_recurs() {
        // Episode 1: notified.
        let first = scan_deadlines(
            &[],
            &[renewal("p1", "namea", 10.0)],
            &cfg(true),
            &Default::default(),
        );
        assert_eq!(first.notifications.len(), 1);

        // Renewed: the deadline drops out of the input entirely.
        let resolved = scan_deadlines(&[], &[], &cfg(true), &first.active_episodes);
        assert!(resolved.notifications.is_empty());
        assert!(
            resolved.active_episodes.is_empty(),
            "resolved episode must be dropped from state"
        );

        // A YEAR later it's imminent again — must notify again since it's
        // a genuinely new episode, not a repeat of the old one.
        let second = scan_deadlines(
            &[],
            &[renewal("p1", "namea", 10.0)],
            &cfg(true),
            &resolved.active_episodes,
        );
        assert_eq!(
            second.notifications.len(),
            1,
            "a new episode for the same name must notify"
        );
    }

    #[test]
    fn independent_names_dedup_independently() {
        let result = scan_deadlines(
            &[reveal("p1", "namea", 10), reveal("p1", "nameb", 10)],
            &[],
            &cfg(true),
            &Default::default(),
        );
        assert_eq!(result.notifications.len(), 2);
    }

    // --- disabled gate ---------------------------------------------------

    #[test]
    fn disabled_config_emits_nothing() {
        let result = scan_deadlines(
            &[reveal("p1", "namea", 10)],
            &[renewal("p1", "nameb", 5.0)],
            &cfg(false),
            &Default::default(),
        );
        assert!(result.notifications.is_empty());
    }

    #[test]
    fn disabled_config_preserves_previous_state_rather_than_clearing() {
        let mut prior = BTreeSet::new();
        prior.insert(format!("reveal:p1:namea:{SENTINEL_REVEAL_END_HEIGHT}"));
        let result = scan_deadlines(&[reveal("p1", "namea", 10)], &[], &cfg(false), &prior);
        assert_eq!(
            result.active_episodes, prior,
            "disabling must not clear dedup state (avoids a notification storm on re-enable)"
        );
    }

    // --- Finding 1a: auction-scoped reveal key --------------------------

    #[test]
    fn reauction_same_name_with_new_reveal_end_height_notifies() {
        // Episode 1 (an earlier, now-lapsed auction on "namea") was already
        // notified and its key is still sitting in `active_episodes` — e.g.
        // the bid never got revealed and the row just fell out of the
        // scanner's input (revealed/dropped), OR (Minor 7) notifications
        // were disabled while it was active and the state simply never got
        // a chance to drop it.
        let mut prior = BTreeSet::new();
        prior.insert("reveal:p1:namea:100".to_string());

        // A BRAND NEW auction on the SAME name — different reveal_end_height
        // — must notify even though a same-named key is already "active".
        let result = scan_deadlines(
            &[reveal_at("p1", "namea", 10, 200)],
            &[],
            &cfg(true),
            &prior,
        );
        assert_eq!(
            result.notifications.len(),
            1,
            "a new auction (new reveal_end_height) on a name with a stale active episode must still notify"
        );
        assert_eq!(result.notifications[0].key, "reveal:p1:namea:200");
        // The old episode is simply absent from the new input, so it is not
        // carried forward either — this scan's `active_episodes` reflects
        // only what's currently imminent.
        assert!(!result.active_episodes.contains("reveal:p1:namea:100"));
    }

    #[test]
    fn same_name_different_reveal_end_heights_are_independent_episodes() {
        let result = scan_deadlines(
            &[
                reveal_at("p1", "namea", 10, 100),
                reveal_at("p1", "namea", 10, 200),
            ],
            &[],
            &cfg(true),
            &Default::default(),
        );
        assert_eq!(
            result.notifications.len(),
            2,
            "two concurrent auctions on the same name are independent episodes"
        );
    }

    #[test]
    fn reveal_disabled_state_freeze_does_not_swallow_a_new_auction_episode() {
        // Minor 7, reveal side: an old episode frozen in state while
        // notifications were disabled (the disabled pass-through echoes
        // `previously_notified` back unchanged, see `scan_deadlines`'s
        // `!config.enabled` branch) must not swallow a later, genuinely new
        // auction on the same name once notifications are re-enabled.
        let mut frozen = BTreeSet::new();
        frozen.insert("reveal:p1:namea:100".to_string());
        let still_disabled = scan_deadlines(
            &[reveal_at("p1", "namea", 10, 100)],
            &[],
            &cfg(false),
            &frozen,
        );
        assert_eq!(
            still_disabled.active_episodes, frozen,
            "disabled: state is frozen, not recomputed"
        );

        // Re-enabled, and it's now a NEW auction (fresh reveal_end_height).
        let re_enabled = scan_deadlines(
            &[reveal_at("p1", "namea", 10, 200)],
            &[],
            &cfg(true),
            &still_disabled.active_episodes,
        );
        assert_eq!(
            re_enabled.notifications.len(),
            1,
            "the new auction must notify despite the frozen old key"
        );
    }

    // --- Finding 1b: stale reveal rows are dropped from the input -------

    #[test]
    fn is_reveal_deadline_stale_boundary() {
        // Exactly one reveal-period past close: NOT yet stale (kept — one
        // more chance for the "you missed it" alarm to fire this tick).
        assert!(!is_reveal_deadline_stale(-10, 10));
        // One block further past: stale.
        assert!(is_reveal_deadline_stale(-11, 10));
        // Comfortably within the window, or not yet closed: never stale.
        assert!(!is_reveal_deadline_stale(-5, 10));
        assert!(!is_reveal_deadline_stale(50, 10));
    }

    // --- Minor 7, renewal side: cycle-scoped renewal key -----------------

    #[test]
    fn renewal_disabled_state_freeze_does_not_swallow_a_new_renewal_episode() {
        let mut frozen = BTreeSet::new();
        frozen.insert("renewal:p1:namea:1000".to_string());
        let still_disabled = scan_deadlines(
            &[],
            &[renewal_at("p1", "namea", 5.0, 1000)],
            &cfg(false),
            &frozen,
        );
        assert_eq!(
            still_disabled.active_episodes, frozen,
            "disabled: state is frozen, not recomputed"
        );

        // Re-enabled, and the name has since renewed — a NEW cycle, new
        // expires_at_height — must notify despite the frozen old key.
        let re_enabled = scan_deadlines(
            &[],
            &[renewal_at("p1", "namea", 5.0, 2000)],
            &cfg(true),
            &still_disabled.active_episodes,
        );
        assert_eq!(
            re_enabled.notifications.len(),
            1,
            "the new renewal cycle must notify despite the frozen old key"
        );
    }

    #[test]
    fn renewal_without_known_expiry_height_falls_back_to_unscoped_key() {
        let result = scan_deadlines(
            &[],
            &[renewal("p1", "namea", 5.0)],
            &cfg(true),
            &Default::default(),
        );
        assert_eq!(result.notifications[0].key, "renewal:p1:namea");
    }

    // --- per-profile / per-kind key isolation -----------------------------

    #[test]
    fn same_name_different_profiles_are_independent_episodes() {
        let result = scan_deadlines(
            &[reveal("p1", "namea", 10), reveal("p2", "namea", 10)],
            &[],
            &cfg(true),
            &Default::default(),
        );
        assert_eq!(result.notifications.len(), 2);
    }

    #[test]
    fn same_name_reveal_and_renewal_are_independent_episodes() {
        let result = scan_deadlines(
            &[reveal("p1", "namea", 10)],
            &[renewal("p1", "namea", 5.0)],
            &cfg(true),
            &Default::default(),
        );
        assert_eq!(result.notifications.len(), 2);
    }
}
