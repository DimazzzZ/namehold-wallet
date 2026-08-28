//! Watched-name background scanner: polls each watched name via the hsd
//! node, diffs against the last-seen snapshot in `watched_name_states`,
//! and emits OS notifications on transitions.
//!
//! Architecture mirrors `commands/deadlines.rs`:
//!   * PURE core (`scan_watched_events`) is fully unit-testable, no IO.
//!   * IO SHELL (`run_watched_scan`) loads config + previous state, fetches
//!     fresh info via `NodeRpcClient::get_name_info`, calls the pure scanner,
//!     emits via `notify-rust`, and persists updated dedup + cache rows.
//!
//! The daemon is the SOLE writer of `watched_name_states` and of the
//! `watched_name_notify_state` KV dedup set. The Tauri app only reads them.

use std::collections::{BTreeSet, HashMap};

use crate::db::queries;
use crate::error::AppError;
use crate::hsd::types::HsdName;
use crate::noncustodial::rpc::NodeRpcClient;

// -------- Settings keys ----------------------------------------------------

/// Master enable for watched-name notifications. Opt-in (default false).
pub const SETTING_ENABLED: &str = "watchlist_notify_enabled";
/// Lead time before BIDDING opens that fires a "bidding-soon" notification.
/// Unit: hsd blocks (~144 blocks/day). Default 144 (~1 day).
pub const SETTING_BIDDING_SOON_LEAD_BLOCKS: &str = "watchlist_notify_bidding_soon_lead_blocks";
/// Global highest-bid threshold in HNS (decimal string, e.g. "100.5").
/// Empty string = disabled. When the highest bid on a watched name crosses
/// this threshold upward, fire once per (name, threshold) episode.
pub const SETTING_HIGH_BID_THRESHOLD_HNS: &str = "watchlist_notify_highest_bid_threshold_hns";
/// JSON-serialized `Vec<String>` of already-notified episode keys (dedup).
pub const SETTING_STATE: &str = "watched_name_notify_state";

/// Adaptive-skip threshold. If a name has more than this many blocks until
/// its next transition AND it was polled within the last 5 minutes, skip it
/// this cycle to save node round-trips.
pub const ADAPTIVE_SKIP_BLOCKS: i64 = 300;
pub const ADAPTIVE_SKIP_MIN_AGE_SECS: i64 = 300;

/// Default bidding-soon lead in blocks (~1 day).
pub const DEFAULT_BIDDING_SOON_LEAD_BLOCKS: u32 = 144;

/// One doo = 1e-6 HNS.
pub const DOOS_PER_HNS: i64 = 1_000_000;

// -------- Config -----------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchedNotifyConfig {
    pub enabled: bool,
    pub bidding_soon_lead_blocks: u32,
    /// `None` when disabled (setting is empty or unparsable).
    pub highest_bid_threshold_doos: Option<i64>,
}

impl Default for WatchedNotifyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bidding_soon_lead_blocks: DEFAULT_BIDDING_SOON_LEAD_BLOCKS,
            highest_bid_threshold_doos: None,
        }
    }
}

pub fn load_config(settings: &HashMap<String, String>) -> WatchedNotifyConfig {
    let enabled = settings
        .get(SETTING_ENABLED)
        .map(|v| v == "true")
        .unwrap_or(false);
    let bidding_soon_lead_blocks = settings
        .get(SETTING_BIDDING_SOON_LEAD_BLOCKS)
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(DEFAULT_BIDDING_SOON_LEAD_BLOCKS);
    let highest_bid_threshold_doos = settings.get(SETTING_HIGH_BID_THRESHOLD_HNS).and_then(|v| {
        let s = v.trim();
        if s.is_empty() {
            None
        } else {
            s.parse::<f64>().ok().and_then(|hns| {
                if hns.is_finite() && hns >= 0.0 {
                    Some((hns * DOOS_PER_HNS as f64) as i64)
                } else {
                    None
                }
            })
        }
    });
    WatchedNotifyConfig {
        enabled,
        bidding_soon_lead_blocks,
        highest_bid_threshold_doos,
    }
}

pub fn load_state(settings: &HashMap<String, String>) -> BTreeSet<String> {
    settings
        .get(SETTING_STATE)
        .and_then(|v| serde_json::from_str::<Vec<String>>(v).ok())
        .map(|v| v.into_iter().collect())
        .unwrap_or_default()
}

// -------- Pure scanner types ----------------------------------------------

/// Previous snapshot of a watched name (from `watched_name_states`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchedPrev {
    pub name: String,
    pub prev_phase: Option<String>,
    pub prev_highest_doos: Option<i64>,
}

/// Freshly-fetched view of a watched name.
#[derive(Debug, Clone)]
pub struct WatchedFresh {
    pub name: String,
    /// Coarse auction phase (see `phase_of`). One of:
    /// `"OPENING" | "BIDDING" | "REVEAL" | "CLOSED" | "OTHER"`.
    pub phase: String,
    pub highest_doos: Option<i64>,
    /// `stats.blocks_until_bidding` for OPENING names; `None` otherwise.
    pub blocks_until_bidding: Option<i64>,
    /// Height anchor for uniquely identifying an auction episode; used in
    /// dedup keys so re-auctions on the same name are fresh episodes.
    /// Prefers `bid_period_start`, then `open_period_start`, then `height`.
    pub episode_height: Option<u64>,
    /// Approx hours-until-bidding, for the notification body only.
    pub hours_until_bidding: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingNotification {
    pub key: String,
    pub summary: String,
    pub body: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ScanResult {
    pub notifications: Vec<PendingNotification>,
    pub active_episodes: BTreeSet<String>,
}

// -------- Phase helpers ---------------------------------------------------

/// Coarse phase of a raw hsd state string. See `src/lib/auction.ts::auctionPhase`.
pub fn phase_of(state: Option<&str>) -> &'static str {
    match state.unwrap_or("").to_ascii_uppercase().as_str() {
        "OPENING" => "OPENING",
        "BIDDING" => "BIDDING",
        "REVEAL" => "REVEAL",
        // CLOSED / TRANSFER / REVOKED / REGISTERED / (empty) all treated as CLOSED-family
        // for phase transition purposes. "AVAILABLE" is treated as OTHER (nothing to notify).
        "CLOSED" => "CLOSED",
        "TRANSFER" => "CLOSED",
        "REVOKED" => "CLOSED",
        "REGISTERED" => "CLOSED",
        _ => "OTHER",
    }
}

// -------- Pure scanner ----------------------------------------------------

/// Compute the notifications to emit given the previous snapshot, the fresh
/// snapshot, and the dedup set. Pure — no IO.
///
/// Emission rules (see plan §B.4):
///   * `-> BIDDING`: prev_phase in {None, OPENING} and fresh.phase == BIDDING.
///   * `re-opened`:  prev_phase == CLOSED and fresh.phase != CLOSED.
///   * `bidding-soon`: fresh.phase == OPENING and blocks_until_bidding is
///     Some(x) with 0 <= x <= config.bidding_soon_lead_blocks.
///   * `high-bid crossed`: config.highest_bid_threshold_doos is Some(t) and
///     prev_highest_doos (or 0 if None) < t <= fresh.highest_doos.
///
/// Disabled: returns no notifications and echoes `previously_notified`
/// unchanged (frozen — matches `deadlines.rs`), so re-enabling doesn't
/// storm the user with a full backlog.
pub fn scan_watched_events(
    fresh: &[WatchedFresh],
    prev: &HashMap<String, WatchedPrev>,
    config: &WatchedNotifyConfig,
    previously_notified: &BTreeSet<String>,
) -> ScanResult {
    if !config.enabled {
        return ScanResult {
            notifications: Vec::new(),
            active_episodes: previously_notified.clone(),
        };
    }

    let mut out = ScanResult::default();

    for f in fresh {
        let prev = prev.get(&f.name);
        let prev_phase = prev.and_then(|p| p.prev_phase.as_deref());
        let prev_highest = prev.and_then(|p| p.prev_highest_doos).unwrap_or(0);
        let anchor = f.episode_height.unwrap_or(0);

        // 1. -> BIDDING
        if matches!(prev_phase, None | Some("OPENING")) && f.phase == "BIDDING" {
            let key = format!("watched:bidding:{}:{}", f.name, anchor);
            if !previously_notified.contains(&key) {
                out.notifications.push(PendingNotification {
                    key: key.clone(),
                    summary: "Watchlist".to_string(),
                    body: format!(".{} \u{2014} bidding is open now", f.name),
                });
            }
            out.active_episodes.insert(key);
        }

        // 2. re-opened / available
        if matches!(prev_phase, Some("CLOSED")) && f.phase != "CLOSED" {
            let key = format!("watched:reopened:{}:{}", f.name, anchor);
            if !previously_notified.contains(&key) {
                out.notifications.push(PendingNotification {
                    key: key.clone(),
                    summary: "Watchlist".to_string(),
                    body: format!(".{} \u{2014} is available again for auction", f.name),
                });
            }
            out.active_episodes.insert(key);
        }

        // 3. bidding-soon (lead-time before BIDDING opens)
        if f.phase == "OPENING" {
            if let Some(bub) = f.blocks_until_bidding {
                if bub >= 0 && (bub as u32) <= config.bidding_soon_lead_blocks {
                    // Anchor to the block at which bidding is expected to open,
                    // so a re-auction is a fresh episode.
                    let bidding_anchor = f.episode_height.map(|h| h as i64 + bub).unwrap_or(bub);
                    let key = format!("watched:biddingsoon:{}:{}", f.name, bidding_anchor);
                    if !previously_notified.contains(&key) {
                        let hours_note = f
                            .hours_until_bidding
                            .map(|h| {
                                if h >= 1.0 {
                                    format!(" (~{}h)", h.round() as i64)
                                } else {
                                    format!(" (~{}m)", (h * 60.0).max(1.0).round() as i64)
                                }
                            })
                            .unwrap_or_default();
                        out.notifications.push(PendingNotification {
                            key: key.clone(),
                            summary: "Watchlist".to_string(),
                            body: format!(
                                ".{} \u{2014} bidding opens in {} block{}{}",
                                f.name,
                                bub,
                                if bub == 1 { "" } else { "s" },
                                hours_note,
                            ),
                        });
                    }
                    out.active_episodes.insert(key);
                }
            }
        }

        // 4. highest-bid crossed threshold (upward)
        if let Some(threshold) = config.highest_bid_threshold_doos {
            if let Some(now) = f.highest_doos {
                if prev_highest < threshold && now >= threshold {
                    let key = format!("watched:highbid:{}:{}", f.name, threshold);
                    if !previously_notified.contains(&key) {
                        out.notifications.push(PendingNotification {
                            key: key.clone(),
                            summary: "Watchlist".to_string(),
                            body: format!(
                                ".{} \u{2014} highest bid crossed {:.4} HNS (now {:.4} HNS)",
                                f.name,
                                (threshold as f64) / DOOS_PER_HNS as f64,
                                (now as f64) / DOOS_PER_HNS as f64,
                            ),
                        });
                    }
                    out.active_episodes.insert(key);
                }
            }
        }
    }

    out
}

// -------- IO shell --------------------------------------------------------

/// Build a `WatchedFresh` from an `HsdName` value (as returned by the
/// `getnameinfo` normalization in `providers::hnsfans::normalize_name` or the
/// synthesized AVAILABLE fallback in `commands/read.rs`).
pub fn watched_fresh_from_hsd(name: &str, hsd: &HsdName) -> WatchedFresh {
    let phase = phase_of(hsd.state.as_deref()).to_string();
    let highest_doos = hsd.highest.map(|v| v as i64);
    let (blocks_until_bidding, hours_until_bidding, episode_height) = match &hsd.stats {
        Some(s) => {
            let episode = s.bid_period_start.or(s.open_period_start).or(hsd.height);
            (s.blocks_until_bidding, s.hours_until_bidding, episode)
        }
        None => (None, None, hsd.height),
    };
    WatchedFresh {
        name: name.to_string(),
        phase,
        highest_doos,
        blocks_until_bidding,
        episode_height,
        hours_until_bidding,
    }
}

/// Extract the min-blocks-until-next-transition, used by the daemon's
/// adaptive skip. Returns None when no relevant countdown is present.
pub fn min_blocks_until_next(hsd: &HsdName) -> Option<i64> {
    let s = hsd.stats.as_ref()?;
    [
        s.blocks_until_open,
        s.blocks_until_bidding,
        s.blocks_until_reveal,
        s.blocks_until_close,
        s.blocks_until_expire,
    ]
    .into_iter()
    .flatten()
    .filter(|v| *v >= 0)
    .min()
}

// -------- DB helpers ------------------------------------------------------

/// Load previous snapshot rows from `watched_name_states`.
pub fn load_prev_snapshots(
    conn: &rusqlite::Connection,
) -> Result<HashMap<String, WatchedPrev>, AppError> {
    let mut stmt =
        conn.prepare("SELECT name, last_phase, last_highest_doos FROM watched_name_states")?;
    let rows = stmt
        .query_map([], |row| {
            let name: String = row.get(0)?;
            let last_phase: Option<String> = row.get(1)?;
            let last_highest_doos: Option<i64> = row.get(2)?;
            Ok((
                name.clone(),
                WatchedPrev {
                    name,
                    prev_phase: last_phase,
                    prev_highest_doos: last_highest_doos,
                },
            ))
        })?
        .collect::<Result<HashMap<_, _>, _>>()?;
    Ok(rows)
}

/// List all watched names (from the global `watched_names` table).
pub fn list_watched_names(conn: &rusqlite::Connection) -> Result<Vec<String>, AppError> {
    let mut stmt = conn.prepare("SELECT name FROM watched_names ORDER BY name")?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(names)
}

/// Load `(polled_at, blocks_until_next)` for adaptive skip.
#[derive(Debug, Clone, Default)]
pub struct PollMeta {
    pub polled_at: Option<String>,
    pub blocks_until_next: Option<i64>,
}

pub fn load_poll_meta(conn: &rusqlite::Connection) -> Result<HashMap<String, PollMeta>, AppError> {
    let mut stmt =
        conn.prepare("SELECT name, polled_at, blocks_until_next FROM watched_name_states")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                PollMeta {
                    polled_at: Some(row.get::<_, String>(1)?),
                    blocks_until_next: row.get::<_, Option<i64>>(2)?,
                },
            ))
        })?
        .collect::<Result<HashMap<_, _>, _>>()?;
    Ok(rows)
}

/// Upsert one row into `watched_name_states`.
pub fn upsert_state_row(
    conn: &rusqlite::Connection,
    name: &str,
    hsd: &HsdName,
) -> Result<(), AppError> {
    let phase = phase_of(hsd.state.as_deref()).to_string();
    let json = serde_json::to_string(hsd).unwrap_or_else(|_| "null".to_string());
    let highest = hsd.highest.map(|v| v as i64);
    let bun = min_blocks_until_next(hsd);
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO watched_name_states
           (name, last_phase, last_state_json, last_highest_doos, blocks_until_next, polled_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
         ON CONFLICT(name) DO UPDATE SET
           last_phase        = excluded.last_phase,
           last_state_json   = excluded.last_state_json,
           last_highest_doos = excluded.last_highest_doos,
           blocks_until_next = excluded.blocks_until_next,
           polled_at         = excluded.polled_at,
           updated_at        = datetime('now')",
        rusqlite::params![name, phase, json, highest, bun, now],
    )?;
    Ok(())
}

/// Should the daemon skip polling this name this cycle? See adaptive-skip
/// constants at the top of this file. Names never polled are always fetched.
pub fn should_skip(meta: Option<&PollMeta>, now_epoch_secs: i64) -> bool {
    let Some(meta) = meta else { return false };
    let Some(bun) = meta.blocks_until_next else {
        return false;
    };
    if bun < ADAPTIVE_SKIP_BLOCKS {
        return false;
    }
    let Some(polled_at) = meta.polled_at.as_deref() else {
        return false;
    };
    let Ok(polled) = chrono::DateTime::parse_from_rfc3339(polled_at) else {
        return false;
    };
    let age = now_epoch_secs - polled.timestamp();
    age < ADAPTIVE_SKIP_MIN_AGE_SECS
}

// -------- IO orchestrator: main entry point called by daemon --------------

/// Run one watched-name scan cycle end-to-end. Called by the daemon after
/// each `sync_all_profiles`. Never propagates errors — logs and moves on.
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn run_watched_scan(db_path: &str) {
    if let Err(e) = try_run_watched_scan(db_path).await {
        eprintln!("namehold-syncd: watched-scan error: {e}");
    }
}

async fn try_run_watched_scan(db_path: &str) -> Result<(), AppError> {
    // 1. Load config + settings + previously-notified set.
    let (settings, config, previously_notified, watched, prev_states, poll_meta) = {
        let conn = crate::commands::sync::open_conn(db_path)?;
        let settings = queries::get_settings(&conn)?;
        let config = load_config(&settings);
        if !config.enabled {
            // Frozen: don't clear state, don't probe the node.
            return Ok(());
        }
        let previously_notified = load_state(&settings);
        let watched = list_watched_names(&conn)?;
        let prev_states = load_prev_snapshots(&conn)?;
        let poll_meta = load_poll_meta(&conn)?;
        (
            settings,
            config,
            previously_notified,
            watched,
            prev_states,
            poll_meta,
        )
    };

    if watched.is_empty() {
        return Ok(());
    }

    // 2. Build node client from settings.
    let node = NodeRpcClient::from_settings(&settings);
    let node_ready = crate::commands::read::node_ready_from_settings(&settings).await;

    // 3. Adaptive skip + fetch. Bounded concurrency (4) to avoid hammering hsd.
    let now_secs = chrono::Utc::now().timestamp();
    let to_fetch: Vec<String> = watched
        .into_iter()
        .filter(|n| !should_skip(poll_meta.get(n), now_secs))
        .collect();

    let fetched: Vec<(String, HsdName)> = if node_ready {
        fetch_all(&node, &to_fetch).await
    } else {
        Vec::new()
    };

    if fetched.is_empty() {
        return Ok(());
    }

    // 4. Upsert state rows + build the fresh vec for the pure scanner.
    let fresh: Vec<WatchedFresh> = {
        let conn = crate::commands::sync::open_conn(db_path)?;
        let mut fresh = Vec::with_capacity(fetched.len());
        for (name, hsd) in &fetched {
            let _ = upsert_state_row(&conn, name, hsd); // best-effort per row
            fresh.push(watched_fresh_from_hsd(name, hsd));
        }
        fresh
    };

    // 5. Run the pure scanner.
    let result = scan_watched_events(&fresh, &prev_states, &config, &previously_notified);

    // 6. Emit notifications via notify-rust. Best-effort per notification.
    for n in &result.notifications {
        emit_os_notification(&n.summary, &n.body);
    }

    // 7. Persist updated dedup set.
    {
        let conn = crate::commands::sync::open_conn(db_path)?;
        let vec: Vec<String> = result.active_episodes.iter().cloned().collect();
        let json = serde_json::to_string(&vec).unwrap_or_else(|_| "[]".to_string());
        queries::set_setting(&conn, SETTING_STATE, &json)?;
    }

    Ok(())
}

/// Fetch `getnameinfo` for every name. Sequential — watchlists are small
/// (dozens of names) and this runs on the daemon's current-thread runtime
/// every 60s, so simple sequential polling is preferable to pulling in a
/// streaming-concurrency dependency. Names that error out or return
/// null/unparsable data are silently dropped; they'll be retried next cycle.
#[cfg_attr(coverage_nightly, coverage(off))]
async fn fetch_all(
    node: &dyn crate::noncustodial::node_rpc::NodeRpc,
    names: &[String],
) -> Vec<(String, HsdName)> {
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        match node.get_name_info(name).await {
            Ok(raw) => {
                if let Some(hsd) = parse_getnameinfo(name, &raw) {
                    out.push((name.clone(), hsd));
                }
            }
            Err(_) => { /* retry next cycle */ }
        }
    }
    out
}

/// Parse a raw `getnameinfo` JSON into an `HsdName`. Uses the same
/// normalization path as `commands/read.rs::read_name_info`.
fn parse_getnameinfo(name: &str, raw: &serde_json::Value) -> Option<HsdName> {
    if let Some(info) = raw.get("info").filter(|v| !v.is_null()) {
        return crate::providers::hnsfans::normalize_name(info);
    }
    // `info` is null → the name has never been touched on-chain.
    Some(HsdName {
        name: name.to_string(),
        name_hash: None,
        state: Some("AVAILABLE".to_string()),
        height: None,
        renewal: None,
        owner: None,
        value: None,
        highest: None,
        registered: Some(false),
        expired: None,
        stats: None,
        transfer: None,
        revoked: None,
        bids: None,
    })
}

// -------- Notification emission (notify-rust) -----------------------------

/// Claim "Namehold" as this process's OS-notification sender identity.
///
/// The daemon binary (`namehold-syncd`) loads no Tauri plugin, so nothing
/// calls `notify_rust::set_application` for us — without this the macOS
/// notification center defaults to Finder (or whatever process launched us,
/// e.g. Terminal). `Once`-guarded because `set_application` locks the
/// process's identity permanently and must only run once.
///
/// Note: the sender NAME always resolves; the ICON resolves only when the
/// bundled `.app` is registered with Launch Services. In practice: end users
/// see the Namehold icon after installing/opening the `.app` once; during
/// dev with no built bundle, the icon may stay generic but the text is
/// correct.
#[cfg(all(target_os = "macos", not(test)))]
fn ensure_notify_identity() {
    use std::sync::Once;
    static SET_APP_ONCE: Once = Once::new();
    SET_APP_ONCE.call_once(|| {
        let _ = notify_rust::set_application("org.zhavoronkov.nameholdwallet");
    });
}

#[cfg(not(test))]
fn emit_os_notification(summary: &str, body: &str) {
    #[cfg(target_os = "macos")]
    ensure_notify_identity();
    // notify-rust is cross-platform (macOS via mac-notification-sys, Linux via
    // libnotify/D-Bus, Windows via tauri-winrt-notification). Fire-and-forget:
    // if the OS notification center is unavailable we just log and move on.
    if let Err(e) = notify_rust::Notification::new()
        .summary(summary)
        .body(body)
        .show()
    {
        eprintln!("namehold-syncd: OS notification failed: {e}");
    }
}

#[cfg(test)]
fn emit_os_notification(_summary: &str, _body: &str) {
    // No-op in tests: the pure scanner is tested directly; the shell is not.
}

/// Debug-only accessor around the real `notify-rust` dispatch, so the
/// in-app `simulate_notification` command can exercise the exact same
/// path the headless daemon uses. Returns `Some(err_string)` on failure
/// (mirroring `send_os_notification` in `commands/deadlines.rs`) so the
/// UI can surface an honest delivery error instead of silently doing
/// nothing. Gated with `debug_assertions` so it is compiled out of
/// release builds entirely.
#[cfg(all(debug_assertions, not(test)))]
pub(crate) fn emit_watchlist_os(summary: &str, body: &str) -> Option<String> {
    #[cfg(target_os = "macos")]
    ensure_notify_identity();
    match notify_rust::Notification::new()
        .summary(summary)
        .body(body)
        .show()
    {
        Ok(_) => None,
        Err(e) => Some(e.to_string()),
    }
}

// -------- Tests: the pure scanner ----------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(enabled: bool, high: Option<i64>) -> WatchedNotifyConfig {
        WatchedNotifyConfig {
            enabled,
            bidding_soon_lead_blocks: 144,
            highest_bid_threshold_doos: high,
        }
    }

    fn fresh(name: &str, phase: &str) -> WatchedFresh {
        WatchedFresh {
            name: name.to_string(),
            phase: phase.to_string(),
            highest_doos: None,
            blocks_until_bidding: None,
            episode_height: Some(1000),
            hours_until_bidding: None,
        }
    }

    fn prev(name: &str, phase: Option<&str>, highest: Option<i64>) -> (String, WatchedPrev) {
        (
            name.to_string(),
            WatchedPrev {
                name: name.to_string(),
                prev_phase: phase.map(str::to_string),
                prev_highest_doos: highest,
            },
        )
    }

    #[test]
    fn disabled_config_emits_nothing_and_freezes_state() {
        let f = vec![fresh("foo", "BIDDING")];
        let mut prev_notified = BTreeSet::new();
        prev_notified.insert("watched:bidding:foo:1000".to_string());
        let result = scan_watched_events(&f, &HashMap::new(), &cfg(false, None), &prev_notified);
        assert!(result.notifications.is_empty());
        assert_eq!(result.active_episodes, prev_notified);
    }

    #[test]
    fn transition_to_bidding_from_opening_fires_once() {
        let f = vec![fresh("foo", "BIDDING")];
        let mut prev_map = HashMap::new();
        let (k, v) = prev("foo", Some("OPENING"), None);
        prev_map.insert(k, v);
        let empty = BTreeSet::new();
        let r1 = scan_watched_events(&f, &prev_map, &cfg(true, None), &empty);
        assert_eq!(r1.notifications.len(), 1);
        assert!(r1.notifications[0].body.contains("bidding is open now"));

        // Second scan with the episode already in the dedup set: no new notif.
        let r2 = scan_watched_events(&f, &prev_map, &cfg(true, None), &r1.active_episodes);
        assert!(r2.notifications.is_empty());
        assert_eq!(r2.active_episodes, r1.active_episodes);
    }

    #[test]
    fn transition_to_bidding_from_none_prev_fires_once() {
        // First-time observation of a name that's already BIDDING is treated
        // as a fresh transition.
        let f = vec![fresh("foo", "BIDDING")];
        let r = scan_watched_events(&f, &HashMap::new(), &cfg(true, None), &BTreeSet::new());
        assert_eq!(r.notifications.len(), 1);
    }

    #[test]
    fn stable_bidding_does_not_re_fire() {
        let f = vec![fresh("foo", "BIDDING")];
        let mut prev_map = HashMap::new();
        let (k, v) = prev("foo", Some("BIDDING"), None);
        prev_map.insert(k, v);
        let r = scan_watched_events(&f, &prev_map, &cfg(true, None), &BTreeSet::new());
        assert!(r.notifications.is_empty());
    }

    #[test]
    fn reopen_from_closed_fires() {
        let f = vec![fresh("foo", "OPENING")];
        let mut prev_map = HashMap::new();
        let (k, v) = prev("foo", Some("CLOSED"), None);
        prev_map.insert(k, v);
        let r = scan_watched_events(&f, &prev_map, &cfg(true, None), &BTreeSet::new());
        assert_eq!(r.notifications.len(), 1);
        assert!(r.notifications[0].body.contains("available again"));
    }

    #[test]
    fn bidding_soon_within_lead_fires_and_dedupes() {
        let mut f = fresh("foo", "OPENING");
        f.blocks_until_bidding = Some(100);
        f.episode_height = Some(2000);
        let r1 = scan_watched_events(
            &[f.clone()],
            &HashMap::new(),
            &cfg(true, None),
            &BTreeSet::new(),
        );
        assert_eq!(r1.notifications.len(), 1);
        assert!(r1.notifications[0]
            .body
            .contains("bidding opens in 100 blocks"));

        // Rerun with dedup set: no new notification.
        let r2 = scan_watched_events(&[f], &HashMap::new(), &cfg(true, None), &r1.active_episodes);
        assert!(r2.notifications.is_empty());
    }

    #[test]
    fn bidding_soon_outside_lead_does_not_fire() {
        let mut f = fresh("foo", "OPENING");
        f.blocks_until_bidding = Some(200); // > default lead 144
        let r = scan_watched_events(&[f], &HashMap::new(), &cfg(true, None), &BTreeSet::new());
        assert!(r.notifications.is_empty());
    }

    #[test]
    fn high_bid_crossing_upward_fires_once_per_threshold() {
        // threshold = 100 HNS = 100_000_000 doos
        let threshold = 100 * DOOS_PER_HNS;
        let mut f = fresh("foo", "BIDDING");
        f.highest_doos = Some(threshold + 1);
        let mut prev_map = HashMap::new();
        // Was BIDDING already (to isolate the high-bid event), highest was 50 HNS.
        let (k, v) = prev("foo", Some("BIDDING"), Some(50 * DOOS_PER_HNS));
        prev_map.insert(k, v);
        let r1 = scan_watched_events(
            &[f.clone()],
            &prev_map,
            &cfg(true, Some(threshold)),
            &BTreeSet::new(),
        );
        assert_eq!(r1.notifications.len(), 1);
        assert!(r1.notifications[0].body.contains("crossed"));

        // Second cycle: prev is now ABOVE the threshold. Should not fire again.
        let (k2, v2) = prev("foo", Some("BIDDING"), Some(threshold + 1));
        let mut prev_map2 = HashMap::new();
        prev_map2.insert(k2, v2);
        let r2 = scan_watched_events(
            &[f],
            &prev_map2,
            &cfg(true, Some(threshold)),
            &r1.active_episodes,
        );
        assert!(r2.notifications.is_empty());
    }

    #[test]
    fn high_bid_no_threshold_configured_never_fires() {
        let mut f = fresh("foo", "BIDDING");
        f.highest_doos = Some(999 * DOOS_PER_HNS);
        // prev_phase == BIDDING isolates the high-bid path (no ->BIDDING event).
        let mut prev_map = HashMap::new();
        let (k, v) = prev("foo", Some("BIDDING"), Some(0));
        prev_map.insert(k, v);
        let r = scan_watched_events(&[f], &prev_map, &cfg(true, None), &BTreeSet::new());
        assert!(r.notifications.is_empty());
    }

    #[test]
    fn phase_of_maps_states_correctly() {
        assert_eq!(phase_of(Some("OPENING")), "OPENING");
        assert_eq!(phase_of(Some("BIDDING")), "BIDDING");
        assert_eq!(phase_of(Some("REVEAL")), "REVEAL");
        assert_eq!(phase_of(Some("CLOSED")), "CLOSED");
        assert_eq!(phase_of(Some("TRANSFER")), "CLOSED");
        assert_eq!(phase_of(Some("REVOKED")), "CLOSED");
        assert_eq!(phase_of(Some("AVAILABLE")), "OTHER");
        assert_eq!(phase_of(None), "OTHER");
    }

    #[test]
    fn load_config_defaults_and_parsing() {
        let empty: HashMap<String, String> = HashMap::new();
        let c = load_config(&empty);
        assert!(!c.enabled);
        assert_eq!(c.bidding_soon_lead_blocks, 144);
        assert_eq!(c.highest_bid_threshold_doos, None);

        let mut m = HashMap::new();
        m.insert(SETTING_ENABLED.into(), "true".into());
        m.insert(SETTING_BIDDING_SOON_LEAD_BLOCKS.into(), "72".into());
        m.insert(SETTING_HIGH_BID_THRESHOLD_HNS.into(), "1.5".into());
        let c = load_config(&m);
        assert!(c.enabled);
        assert_eq!(c.bidding_soon_lead_blocks, 72);
        assert_eq!(c.highest_bid_threshold_doos, Some(1_500_000));

        // Empty threshold string → None
        let mut m = HashMap::new();
        m.insert(SETTING_HIGH_BID_THRESHOLD_HNS.into(), "".into());
        let c = load_config(&m);
        assert_eq!(c.highest_bid_threshold_doos, None);
    }

    #[test]
    fn should_skip_never_polled_returns_false() {
        assert!(!should_skip(None, 0));
    }

    #[test]
    fn should_skip_recent_and_far_future_returns_true() {
        let meta = PollMeta {
            polled_at: Some(chrono::Utc::now().to_rfc3339()),
            blocks_until_next: Some(500),
        };
        assert!(should_skip(Some(&meta), chrono::Utc::now().timestamp()));
    }

    #[test]
    fn should_skip_recent_but_imminent_returns_false() {
        let meta = PollMeta {
            polled_at: Some(chrono::Utc::now().to_rfc3339()),
            blocks_until_next: Some(50), // < 300
        };
        assert!(!should_skip(Some(&meta), chrono::Utc::now().timestamp()));
    }

    // --- Coverage-driven tests ---

    #[test]
    fn default_config_matches_expected_values() {
        let d = WatchedNotifyConfig::default();
        assert!(!d.enabled);
        assert_eq!(d.bidding_soon_lead_blocks, DEFAULT_BIDDING_SOON_LEAD_BLOCKS);
        assert_eq!(d.highest_bid_threshold_doos, None);
    }

    #[test]
    fn load_config_rejects_negative_and_non_finite_threshold() {
        // Negative HNS value → None (line 85 guard).
        let mut m = HashMap::new();
        m.insert(SETTING_HIGH_BID_THRESHOLD_HNS.into(), "-1.5".into());
        let c = load_config(&m);
        assert_eq!(c.highest_bid_threshold_doos, None);

        // Non-finite (inf) → None.
        let mut m = HashMap::new();
        m.insert(SETTING_HIGH_BID_THRESHOLD_HNS.into(), "inf".into());
        let c = load_config(&m);
        assert_eq!(c.highest_bid_threshold_doos, None);
    }

    #[test]
    fn load_state_parses_json_vec_and_defaults_on_missing() {
        let empty: HashMap<String, String> = HashMap::new();
        assert!(load_state(&empty).is_empty());

        let mut m = HashMap::new();
        m.insert(SETTING_STATE.into(), r#"["a","b"]"#.into());
        let s = load_state(&m);
        assert!(s.contains("a"));
        assert!(s.contains("b"));
        assert_eq!(s.len(), 2);

        // Invalid JSON → empty set (graceful)
        let mut m = HashMap::new();
        m.insert(SETTING_STATE.into(), "not json".into());
        assert!(load_state(&m).is_empty());
    }

    #[test]
    fn bidding_soon_hours_note_above_one_hour() {
        // hours_until_bidding >= 1.0 → " (~Xh)" format (line 239)
        let mut f = fresh("foo", "OPENING");
        f.blocks_until_bidding = Some(100);
        f.hours_until_bidding = Some(16.7);
        let r = scan_watched_events(&[f], &HashMap::new(), &cfg(true, None), &BTreeSet::new());
        assert_eq!(r.notifications.len(), 1);
        assert!(r.notifications[0].body.contains("(~17h)"));
    }

    #[test]
    fn bidding_soon_hours_note_below_one_hour() {
        // hours_until_bidding < 1.0 → " (~Xm)" format (line 242)
        let mut f = fresh("foo", "OPENING");
        f.blocks_until_bidding = Some(5);
        f.hours_until_bidding = Some(0.5);
        let r = scan_watched_events(&[f], &HashMap::new(), &cfg(true, None), &BTreeSet::new());
        assert_eq!(r.notifications.len(), 1);
        assert!(r.notifications[0].body.contains("(~30m)"));
    }

    #[test]
    fn high_bid_already_notified_does_not_re_fire() {
        // Covers the `previously_notified.contains(&key)` guard on high-bid
        // (line 282).
        let threshold = 100 * DOOS_PER_HNS;
        let mut f = fresh("foo", "BIDDING");
        f.highest_doos = Some(threshold + 1);
        let mut prev_map = HashMap::new();
        let (k, v) = prev("foo", Some("BIDDING"), Some(50 * DOOS_PER_HNS));
        prev_map.insert(k, v);

        // Pre-seed the dedup set with the exact key that would be emitted.
        let mut already = BTreeSet::new();
        already.insert(format!("watched:highbid:foo:{}", threshold));

        let r = scan_watched_events(&[f], &prev_map, &cfg(true, Some(threshold)), &already);
        assert!(r.notifications.is_empty());
        // Episode is still tracked.
        assert!(r
            .active_episodes
            .contains(&format!("watched:highbid:foo:{}", threshold)));
    }

    #[test]
    fn watched_fresh_from_hsd_with_stats() {
        use crate::hsd::types::{HsdName, HsdNameStats};
        let hsd = HsdName {
            name: "hello".to_string(),
            name_hash: None,
            state: Some("OPENING".to_string()),
            height: Some(5000),
            renewal: None,
            owner: None,
            value: None,
            highest: Some(42_000_000),
            registered: None,
            expired: None,
            stats: Some(HsdNameStats {
                renewal_period_start: None,
                renewal_period_end: None,
                blocks_until_expire: None,
                days_until_expire: None,
                open_period_start: Some(4900),
                open_period_end: None,
                bid_period_start: Some(5100),
                bid_period_end: None,
                reveal_period_start: None,
                reveal_period_end: None,
                blocks_until_open: None,
                blocks_until_bidding: Some(100),
                blocks_until_reveal: None,
                blocks_until_close: None,
                hours_until_open: None,
                hours_until_bidding: Some(16.5),
                hours_until_reveal: None,
                hours_until_close: None,
            }),
            transfer: None,
            revoked: None,
            bids: None,
        };
        let wf = watched_fresh_from_hsd("hello", &hsd);
        assert_eq!(wf.name, "hello");
        assert_eq!(wf.phase, "OPENING");
        assert_eq!(wf.highest_doos, Some(42_000_000));
        assert_eq!(wf.blocks_until_bidding, Some(100));
        assert_eq!(wf.hours_until_bidding, Some(16.5));
        // episode_height prefers bid_period_start over open_period_start.
        assert_eq!(wf.episode_height, Some(5100));
    }

    #[test]
    fn watched_fresh_from_hsd_without_stats() {
        use crate::hsd::types::HsdName;
        let hsd = HsdName {
            name: "bare".to_string(),
            name_hash: None,
            state: Some("CLOSED".to_string()),
            height: Some(3000),
            renewal: None,
            owner: None,
            value: None,
            highest: None,
            registered: None,
            expired: None,
            stats: None,
            transfer: None,
            revoked: None,
            bids: None,
        };
        let wf = watched_fresh_from_hsd("bare", &hsd);
        assert_eq!(wf.phase, "CLOSED");
        assert_eq!(wf.blocks_until_bidding, None);
        assert_eq!(wf.episode_height, Some(3000)); // falls back to hsd.height
    }

    #[test]
    fn min_blocks_until_next_picks_smallest_positive() {
        use crate::hsd::types::{HsdName, HsdNameStats};
        let hsd = HsdName {
            name: "test".to_string(),
            name_hash: None,
            state: None,
            height: None,
            renewal: None,
            owner: None,
            value: None,
            highest: None,
            registered: None,
            expired: None,
            stats: Some(HsdNameStats {
                renewal_period_start: None,
                renewal_period_end: None,
                blocks_until_expire: Some(5000),
                days_until_expire: None,
                open_period_start: None,
                open_period_end: None,
                bid_period_start: None,
                bid_period_end: None,
                reveal_period_start: None,
                reveal_period_end: None,
                blocks_until_open: Some(-1), // negative, filtered out
                blocks_until_bidding: Some(200),
                blocks_until_reveal: None,
                blocks_until_close: Some(50),
                hours_until_open: None,
                hours_until_bidding: None,
                hours_until_reveal: None,
                hours_until_close: None,
            }),
            transfer: None,
            revoked: None,
            bids: None,
        };
        assert_eq!(min_blocks_until_next(&hsd), Some(50));
    }

    #[test]
    fn min_blocks_until_next_none_when_no_stats() {
        use crate::hsd::types::HsdName;
        let hsd = HsdName {
            name: "x".to_string(),
            name_hash: None,
            state: None,
            height: None,
            renewal: None,
            owner: None,
            value: None,
            highest: None,
            registered: None,
            expired: None,
            stats: None,
            transfer: None,
            revoked: None,
            bids: None,
        };
        assert_eq!(min_blocks_until_next(&hsd), None);
    }

    #[test]
    fn should_skip_no_blocks_until_next_returns_false() {
        let meta = PollMeta {
            polled_at: Some(chrono::Utc::now().to_rfc3339()),
            blocks_until_next: None,
        };
        assert!(!should_skip(Some(&meta), chrono::Utc::now().timestamp()));
    }

    #[test]
    fn should_skip_no_polled_at_returns_false() {
        let meta = PollMeta {
            polled_at: None,
            blocks_until_next: Some(500),
        };
        assert!(!should_skip(Some(&meta), chrono::Utc::now().timestamp()));
    }

    #[test]
    fn should_skip_stale_poll_returns_false() {
        // polled_at is 10 minutes ago → age > ADAPTIVE_SKIP_MIN_AGE_SECS → don't skip.
        let ten_min_ago = chrono::Utc::now() - chrono::Duration::seconds(600);
        let meta = PollMeta {
            polled_at: Some(ten_min_ago.to_rfc3339()),
            blocks_until_next: Some(500),
        };
        assert!(!should_skip(Some(&meta), chrono::Utc::now().timestamp()));
    }

    #[test]
    fn should_skip_unparseable_polled_at_returns_false() {
        // Corrupt timestamp → parse failure → don't skip (line 431).
        let meta = PollMeta {
            polled_at: Some("not-a-timestamp".to_string()),
            blocks_until_next: Some(500),
        };
        assert!(!should_skip(Some(&meta), chrono::Utc::now().timestamp()));
    }

    #[test]
    fn parse_getnameinfo_with_info_object() {
        let raw = serde_json::json!({
            "info": {
                "name": "hello",
                "nameHash": "abc123",
                "state": "CLOSED",
                "height": 1000,
                "stats": {
                    "renewalPeriodStart": 900,
                    "renewalPeriodEnd": 2000,
                    "blocksUntilExpire": 5000
                }
            }
        });
        let parsed = parse_getnameinfo("hello", &raw).unwrap();
        assert_eq!(parsed.name, "hello");
        assert_eq!(parsed.state.as_deref(), Some("CLOSED"));
    }

    #[test]
    fn parse_getnameinfo_null_info_returns_available() {
        let raw = serde_json::json!({ "info": null });
        let parsed = parse_getnameinfo("newname", &raw).unwrap();
        assert_eq!(parsed.name, "newname");
        assert_eq!(parsed.state.as_deref(), Some("AVAILABLE"));
        assert_eq!(parsed.registered, Some(false));
    }

    // --- DB helper tests (in-memory SQLite) ---

    fn test_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::migrations::run(&conn).unwrap();
        conn
    }

    #[test]
    fn db_list_watched_names_empty() {
        let conn = test_conn();
        let names = list_watched_names(&conn).unwrap();
        assert!(names.is_empty());
    }

    #[test]
    fn db_list_watched_names_returns_sorted() {
        let conn = test_conn();
        conn.execute("INSERT INTO watched_names (name) VALUES ('zeta')", [])
            .unwrap();
        conn.execute("INSERT INTO watched_names (name) VALUES ('alpha')", [])
            .unwrap();
        let names = list_watched_names(&conn).unwrap();
        assert_eq!(names, vec!["alpha", "zeta"]);
    }

    #[test]
    fn db_upsert_and_load_prev_snapshots() {
        use crate::hsd::types::HsdName;
        let conn = test_conn();
        let hsd = HsdName {
            name: "test".to_string(),
            name_hash: None,
            state: Some("BIDDING".to_string()),
            height: Some(1000),
            renewal: None,
            owner: None,
            value: None,
            highest: Some(50_000_000),
            registered: None,
            expired: None,
            stats: None,
            transfer: None,
            revoked: None,
            bids: None,
        };
        upsert_state_row(&conn, "test", &hsd).unwrap();

        let snaps = load_prev_snapshots(&conn).unwrap();
        assert_eq!(snaps.len(), 1);
        let s = &snaps["test"];
        assert_eq!(s.prev_phase.as_deref(), Some("BIDDING"));
        assert_eq!(s.prev_highest_doos, Some(50_000_000));

        // Upsert again with a new state → updates in place.
        let hsd2 = HsdName {
            state: Some("REVEAL".to_string()),
            highest: Some(80_000_000),
            ..hsd
        };
        upsert_state_row(&conn, "test", &hsd2).unwrap();
        let snaps = load_prev_snapshots(&conn).unwrap();
        assert_eq!(snaps["test"].prev_phase.as_deref(), Some("REVEAL"));
        assert_eq!(snaps["test"].prev_highest_doos, Some(80_000_000));
    }

    #[test]
    fn db_load_poll_meta() {
        use crate::hsd::types::HsdName;
        let conn = test_conn();
        let hsd = HsdName {
            name: "poll".to_string(),
            name_hash: None,
            state: Some("OPENING".to_string()),
            height: Some(2000),
            renewal: None,
            owner: None,
            value: None,
            highest: None,
            registered: None,
            expired: None,
            stats: None,
            transfer: None,
            revoked: None,
            bids: None,
        };
        upsert_state_row(&conn, "poll", &hsd).unwrap();

        let meta = load_poll_meta(&conn).unwrap();
        assert_eq!(meta.len(), 1);
        let m = &meta["poll"];
        assert!(m.polled_at.is_some());
        // No stats → blocks_until_next is None.
        assert_eq!(m.blocks_until_next, None);
    }

    // --- Additional coverage-driven tests ---

    #[test]
    fn phase_of_registered_and_lowercase_and_empty() {
        // REGISTERED maps to CLOSED (line 159) — not covered by the earlier
        // phase_of test.
        assert_eq!(phase_of(Some("REGISTERED")), "CLOSED");
        // Case-insensitive: lowercase input is upcased before matching.
        assert_eq!(phase_of(Some("bidding")), "BIDDING");
        assert_eq!(phase_of(Some("registered")), "CLOSED");
        // Empty string → OTHER (the `unwrap_or("")` default path).
        assert_eq!(phase_of(Some("")), "OTHER");
        // Arbitrary unknown → OTHER.
        assert_eq!(phase_of(Some("SOMETHINGELSE")), "OTHER");
    }

    #[test]
    fn bidding_soon_singular_block_text() {
        // blocks_until_bidding == 1 → "block" (singular, no trailing "s").
        // Covers the `if bub == 1 { "" } else { "s" }` true branch (line 253).
        let mut f = fresh("foo", "OPENING");
        f.blocks_until_bidding = Some(1);
        f.hours_until_bidding = None;
        let r = scan_watched_events(&[f], &HashMap::new(), &cfg(true, None), &BTreeSet::new());
        assert_eq!(r.notifications.len(), 1);
        let body = &r.notifications[0].body;
        assert!(body.contains("bidding opens in 1 block"));
        // Must NOT be pluralized.
        assert!(!body.contains("1 blocks"));
    }

    #[test]
    fn bidding_soon_zero_blocks_pluralized() {
        // bub == 0 is within lead and is NOT the singular case → "blocks".
        let mut f = fresh("foo", "OPENING");
        f.blocks_until_bidding = Some(0);
        let r = scan_watched_events(&[f], &HashMap::new(), &cfg(true, None), &BTreeSet::new());
        assert_eq!(r.notifications.len(), 1);
        assert!(r.notifications[0].body.contains("bidding opens in 0 blocks"));
    }

    #[test]
    fn bidding_soon_negative_blocks_does_not_fire() {
        // bub < 0 → the `bub >= 0` guard fails; no notification.
        let mut f = fresh("foo", "OPENING");
        f.blocks_until_bidding = Some(-5);
        let r = scan_watched_events(&[f], &HashMap::new(), &cfg(true, None), &BTreeSet::new());
        assert!(r.notifications.is_empty());
        assert!(r.active_episodes.is_empty());
    }

    #[test]
    fn bidding_soon_opening_without_blocks_until_bidding_does_not_fire() {
        // OPENING phase but blocks_until_bidding is None → inner `if let`
        // short-circuits; nothing emitted.
        let f = fresh("foo", "OPENING"); // blocks_until_bidding defaults to None
        let r = scan_watched_events(&[f], &HashMap::new(), &cfg(true, None), &BTreeSet::new());
        assert!(r.notifications.is_empty());
    }

    #[test]
    fn reopen_already_notified_does_not_re_fire_but_tracks_episode() {
        // prev CLOSED → fresh OPENING, but the reopen key is already in the
        // dedup set → no notification, episode still tracked (lines 219-224).
        let f = fresh("foo", "OPENING"); // episode_height = Some(1000)
        let mut prev_map = HashMap::new();
        let (k, v) = prev("foo", Some("CLOSED"), None);
        prev_map.insert(k, v);
        let mut already = BTreeSet::new();
        already.insert("watched:reopened:foo:1000".to_string());
        let r = scan_watched_events(&[f], &prev_map, &cfg(true, None), &already);
        assert!(r.notifications.is_empty());
        assert!(r
            .active_episodes
            .contains("watched:reopened:foo:1000"));
    }

    #[test]
    fn bidding_already_notified_does_not_re_fire_but_tracks_episode() {
        // prev OPENING → fresh BIDDING, key already in dedup set → no notif,
        // episode still tracked (the `previously_notified.contains` guard on
        // the ->BIDDING path).
        let f = fresh("foo", "BIDDING"); // episode_height = Some(1000)
        let mut prev_map = HashMap::new();
        let (k, v) = prev("foo", Some("OPENING"), None);
        prev_map.insert(k, v);
        let mut already = BTreeSet::new();
        already.insert("watched:bidding:foo:1000".to_string());
        let r = scan_watched_events(&[f], &prev_map, &cfg(true, None), &already);
        assert!(r.notifications.is_empty());
        assert!(r.active_episodes.contains("watched:bidding:foo:1000"));
    }

    #[test]
    fn high_bid_threshold_set_but_highest_missing_does_not_fire() {
        // config threshold Some(_) but fresh.highest_doos is None → inner
        // `if let Some(now)` short-circuits; no notification (line 264 branch).
        let threshold = 100 * DOOS_PER_HNS;
        let mut f = fresh("foo", "BIDDING");
        f.highest_doos = None;
        let mut prev_map = HashMap::new();
        let (k, v) = prev("foo", Some("BIDDING"), Some(0));
        prev_map.insert(k, v);
        let r = scan_watched_events(&[f], &prev_map, &cfg(true, Some(threshold)), &BTreeSet::new());
        assert!(r.notifications.is_empty());
    }

    #[test]
    fn high_bid_below_threshold_does_not_fire() {
        // now < threshold → the crossing condition is false; nothing emitted.
        let threshold = 100 * DOOS_PER_HNS;
        let mut f = fresh("foo", "BIDDING");
        f.highest_doos = Some(threshold - 1);
        let mut prev_map = HashMap::new();
        let (k, v) = prev("foo", Some("BIDDING"), Some(0));
        prev_map.insert(k, v);
        let r = scan_watched_events(&[f], &prev_map, &cfg(true, Some(threshold)), &BTreeSet::new());
        assert!(r.notifications.is_empty());
        assert!(r.active_episodes.is_empty());
    }

    #[test]
    fn high_bid_prev_already_above_threshold_does_not_fire() {
        // prev_highest already >= threshold → the `prev_highest < threshold`
        // guard fails, so an even-higher bid does not re-notify.
        let threshold = 100 * DOOS_PER_HNS;
        let mut f = fresh("foo", "BIDDING");
        f.highest_doos = Some(threshold + 500);
        let mut prev_map = HashMap::new();
        let (k, v) = prev("foo", Some("BIDDING"), Some(threshold + 1));
        prev_map.insert(k, v);
        let r = scan_watched_events(&[f], &prev_map, &cfg(true, Some(threshold)), &BTreeSet::new());
        assert!(r.notifications.is_empty());
    }

    #[test]
    fn high_bid_exact_threshold_with_no_prev_row_fires() {
        // No prev row at all → prev_highest defaults to 0; a bid exactly AT
        // the threshold (>=) crosses it and fires. Exercises the
        // `prev.and_then(...).unwrap_or(0)` default and the `>=` boundary.
        let threshold = 100 * DOOS_PER_HNS;
        let mut f = fresh("foo", "REVEAL"); // non-BIDDING to isolate high-bid
        f.highest_doos = Some(threshold);
        let r = scan_watched_events(&[f], &HashMap::new(), &cfg(true, Some(threshold)), &BTreeSet::new());
        assert_eq!(r.notifications.len(), 1);
        assert!(r.notifications[0].body.contains("crossed"));
    }

    #[test]
    fn multiple_events_can_fire_for_one_name() {
        // A first-time BIDDING observation with a high bid over threshold and
        // no prev row fires BOTH the ->BIDDING and high-bid notifications.
        let threshold = 10 * DOOS_PER_HNS;
        let mut f = fresh("foo", "BIDDING");
        f.highest_doos = Some(threshold * 2);
        let r = scan_watched_events(&[f], &HashMap::new(), &cfg(true, Some(threshold)), &BTreeSet::new());
        assert_eq!(r.notifications.len(), 2);
        assert_eq!(r.active_episodes.len(), 2);
    }

    #[test]
    fn watched_fresh_from_hsd_episode_height_prefers_open_when_no_bid_start() {
        // stats present, bid_period_start None → falls back to
        // open_period_start for episode_height.
        use crate::hsd::types::{HsdName, HsdNameStats};
        let hsd = HsdName {
            name: "x".to_string(),
            name_hash: None,
            state: Some("OPENING".to_string()),
            height: Some(9000),
            renewal: None,
            owner: None,
            value: None,
            highest: None,
            registered: None,
            expired: None,
            stats: Some(HsdNameStats {
                renewal_period_start: None,
                renewal_period_end: None,
                blocks_until_expire: None,
                days_until_expire: None,
                open_period_start: Some(8800),
                open_period_end: None,
                bid_period_start: None,
                bid_period_end: None,
                reveal_period_start: None,
                reveal_period_end: None,
                blocks_until_open: None,
                blocks_until_bidding: None,
                blocks_until_reveal: None,
                blocks_until_close: None,
                hours_until_open: None,
                hours_until_bidding: None,
                hours_until_reveal: None,
                hours_until_close: None,
            }),
            transfer: None,
            revoked: None,
            bids: None,
        };
        let wf = watched_fresh_from_hsd("x", &hsd);
        assert_eq!(wf.episode_height, Some(8800));
    }

    #[test]
    fn watched_fresh_from_hsd_episode_height_falls_back_to_height() {
        // stats present but neither bid_period_start nor open_period_start →
        // falls back to hsd.height.
        use crate::hsd::types::{HsdName, HsdNameStats};
        let hsd = HsdName {
            name: "y".to_string(),
            name_hash: None,
            state: Some("REVEAL".to_string()),
            height: Some(7777),
            renewal: None,
            owner: None,
            value: None,
            highest: None,
            registered: None,
            expired: None,
            stats: Some(HsdNameStats {
                renewal_period_start: None,
                renewal_period_end: None,
                blocks_until_expire: None,
                days_until_expire: None,
                open_period_start: None,
                open_period_end: None,
                bid_period_start: None,
                bid_period_end: None,
                reveal_period_start: None,
                reveal_period_end: None,
                blocks_until_open: None,
                blocks_until_bidding: None,
                blocks_until_reveal: None,
                blocks_until_close: None,
                hours_until_open: None,
                hours_until_bidding: None,
                hours_until_reveal: None,
                hours_until_close: None,
            }),
            transfer: None,
            revoked: None,
            bids: None,
        };
        let wf = watched_fresh_from_hsd("y", &hsd);
        assert_eq!(wf.episode_height, Some(7777));
        assert_eq!(wf.phase, "REVEAL");
    }

    #[test]
    fn min_blocks_until_next_all_negative_returns_none() {
        // All countdowns negative → filtered out → min() over empty → None.
        use crate::hsd::types::{HsdName, HsdNameStats};
        let hsd = HsdName {
            name: "n".to_string(),
            name_hash: None,
            state: None,
            height: None,
            renewal: None,
            owner: None,
            value: None,
            highest: None,
            registered: None,
            expired: None,
            stats: Some(HsdNameStats {
                renewal_period_start: None,
                renewal_period_end: None,
                blocks_until_expire: Some(-1),
                days_until_expire: None,
                open_period_start: None,
                open_period_end: None,
                bid_period_start: None,
                bid_period_end: None,
                reveal_period_start: None,
                reveal_period_end: None,
                blocks_until_open: Some(-2),
                blocks_until_bidding: None,
                blocks_until_reveal: None,
                blocks_until_close: None,
                hours_until_open: None,
                hours_until_bidding: None,
                hours_until_reveal: None,
                hours_until_close: None,
            }),
            transfer: None,
            revoked: None,
            bids: None,
        };
        assert_eq!(min_blocks_until_next(&hsd), None);
    }

    #[test]
    fn min_blocks_until_next_zero_is_included() {
        // 0 satisfies `>= 0` and is the min.
        use crate::hsd::types::{HsdName, HsdNameStats};
        let hsd = HsdName {
            name: "z".to_string(),
            name_hash: None,
            state: None,
            height: None,
            renewal: None,
            owner: None,
            value: None,
            highest: None,
            registered: None,
            expired: None,
            stats: Some(HsdNameStats {
                renewal_period_start: None,
                renewal_period_end: None,
                blocks_until_expire: Some(100),
                days_until_expire: None,
                open_period_start: None,
                open_period_end: None,
                bid_period_start: None,
                bid_period_end: None,
                reveal_period_start: None,
                reveal_period_end: None,
                blocks_until_open: None,
                blocks_until_bidding: None,
                blocks_until_reveal: Some(0),
                blocks_until_close: None,
                hours_until_open: None,
                hours_until_bidding: None,
                hours_until_reveal: None,
                hours_until_close: None,
            }),
            transfer: None,
            revoked: None,
            bids: None,
        };
        assert_eq!(min_blocks_until_next(&hsd), Some(0));
    }

    #[test]
    fn upsert_state_row_persists_blocks_until_next_from_stats() {
        // Upsert a name whose stats yield a positive blocks_until_next, then
        // read it back via load_poll_meta.
        use crate::hsd::types::{HsdName, HsdNameStats};
        let conn = test_conn();
        let hsd = HsdName {
            name: "meta".to_string(),
            name_hash: None,
            state: Some("OPENING".to_string()),
            height: Some(100),
            renewal: None,
            owner: None,
            value: None,
            highest: None,
            registered: None,
            expired: None,
            stats: Some(HsdNameStats {
                renewal_period_start: None,
                renewal_period_end: None,
                blocks_until_expire: None,
                days_until_expire: None,
                open_period_start: None,
                open_period_end: None,
                bid_period_start: None,
                bid_period_end: None,
                reveal_period_start: None,
                reveal_period_end: None,
                blocks_until_open: None,
                blocks_until_bidding: Some(42),
                blocks_until_reveal: None,
                blocks_until_close: None,
                hours_until_open: None,
                hours_until_bidding: None,
                hours_until_reveal: None,
                hours_until_close: None,
            }),
            transfer: None,
            revoked: None,
            bids: None,
        };
        upsert_state_row(&conn, "meta", &hsd).unwrap();
        let meta = load_poll_meta(&conn).unwrap();
        assert_eq!(meta["meta"].blocks_until_next, Some(42));
    }

    #[test]
    fn load_prev_snapshots_empty_db_returns_empty_map() {
        let conn = test_conn();
        let snaps = load_prev_snapshots(&conn).unwrap();
        assert!(snaps.is_empty());
    }

    #[test]
    fn load_poll_meta_empty_db_returns_empty_map() {
        let conn = test_conn();
        let meta = load_poll_meta(&conn).unwrap();
        assert!(meta.is_empty());
    }

    #[test]
    fn parse_getnameinfo_missing_info_key_returns_available() {
        // Raw JSON without an `info` key at all → the `raw.get("info")` is
        // None → treated like null-info → synthesized AVAILABLE.
        let raw = serde_json::json!({ "something": "else" });
        let parsed = parse_getnameinfo("brandnew", &raw).unwrap();
        assert_eq!(parsed.name, "brandnew");
        assert_eq!(parsed.state.as_deref(), Some("AVAILABLE"));
        assert_eq!(parsed.registered, Some(false));
        assert!(parsed.stats.is_none());
    }

    #[test]
    fn parse_getnameinfo_info_missing_name_returns_none() {
        // `info` present and non-null but missing the required `name` field →
        // normalize_name returns None → parse_getnameinfo returns None.
        let raw = serde_json::json!({ "info": { "state": "CLOSED" } });
        assert!(parse_getnameinfo("whatever", &raw).is_none());
    }

    #[test]
    fn emit_os_notification_test_stub_is_noop() {
        // In cfg(test) the emitter is a no-op; just ensure it is callable and
        // returns unit (covers the test-only stub body).
        emit_os_notification("Watchlist", "some body");
    }

    #[test]
    fn scan_ignores_other_phase_names() {
        // A name in OTHER phase with no prev row triggers none of the four
        // event kinds.
        let f = fresh("foo", "OTHER");
        let r = scan_watched_events(&[f], &HashMap::new(), &cfg(true, None), &BTreeSet::new());
        assert!(r.notifications.is_empty());
        assert!(r.active_episodes.is_empty());
    }
}
