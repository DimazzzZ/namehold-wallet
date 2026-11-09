//! The exchange's settings and the clients built from them, for a profile.
//!
//! Settings are per network (`exchange_relay_url.<network>` …), written only
//! by `commands::exchange::exchange_update_settings` after the secure window,
//! never by the renderer's `update_setting` (they are not on
//! `RENDERER_SETTING_KEYS`). The bitcoind RPC password is a sensitive key,
//! redacted from `get_settings`.

use std::collections::HashMap;

use crate::error::AppError;
use crate::exchange_io::btc_source::{self, BtcSource};
use crate::exchange_io::relay_client::{self, RelayClient};
use crate::noncustodial::exchange::policy::Policy;
use crate::noncustodial::network::Network;

pub const RELAY_KEY: &str = "exchange_relay_url";
pub const WATCHTOWER_KEY: &str = "exchange_watchtower_url";
pub const SOURCES_KEY: &str = "exchange_btc_sources";
pub const BITCOIND_AUTH_KEY: &str = "exchange_bitcoind_rpc_auth";

/// The most Bitcoin sources one network may list.
pub const MAX_SOURCES: usize = 5;

/// R17: the sentences every entry point shows (`src/lib/exchangeText.ts`).
pub const NEEDS_RECOVERY_PHRASE: &str =
    "The exchange needs a wallet whose recovery phrase is on this computer: a Ledger or watch-only wallet cannot sign a trade.";
pub const NEEDS_NODE: &str = "The exchange needs a node: an explorer cannot check a lock.";
pub const NEEDS_LOCAL_NODE: &str =
    "The exchange needs a node on this computer: a remote node could show a fake lock and learn your secret.";
pub const NEEDS_SENDING_NODE: &str =
    "The exchange needs Namehold's local node, or your own hsd on this computer, able to send.";

/// Why the exchange is refused for this profile and node, or `None`.
/// `source` is the effective one (`rpc::send_source`): a "local" source
/// whose URL is not this machine is a remote node.
pub fn refusal(
    profile_kind: &str,
    source: crate::noncustodial::rpc::ChainSource,
    can_send: bool,
) -> Option<&'static str> {
    use crate::noncustodial::rpc::ChainSource;
    if profile_kind != "mnemonic_hot" {
        return Some(NEEDS_RECOVERY_PHRASE);
    }
    if source == ChainSource::Explorer {
        return Some(NEEDS_NODE);
    }
    // Review M3: the Buyer reveals on the word of the node it reads, so that
    // node must be its own.
    if source == ChainSource::RemoteNode {
        return Some(NEEDS_LOCAL_NODE);
    }
    if !can_send {
        return Some(NEEDS_SENDING_NODE);
    }
    None
}

/// [`refusal`] for a node and the settings: the one rule the commands and
/// the sync step apply (SECURITY.md "The HNS node must be local").
pub fn node_refusal(
    profile_kind: &str,
    node: &dyn crate::noncustodial::node_rpc::NodeRpc,
    settings: &HashMap<String, String>,
) -> Option<&'static str> {
    use crate::noncustodial::rpc;
    let source = rpc::send_source(node.source(), node.node_url());
    let can_send = crate::providers::WriteCapability::evaluate(
        true,
        source,
        rpc::remote_broadcast_allowed(settings),
    )
    .broadcaster_available;
    refusal(profile_kind, source, can_send)
}

pub fn key(base: &str, network: Network) -> String {
    format!("{base}.{}", network.as_str())
}

/// The sensitive keys of every network (for `security::is_sensitive_key`).
pub fn is_sensitive(key: &str) -> bool {
    key.strip_prefix(BITCOIND_AUTH_KEY)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub relay_url: String,
    pub watchtower_url: String,
    pub btc_sources: Vec<String>,
    pub bitcoind_auth: Option<String>,
}

pub fn config(settings: &HashMap<String, String>, network: Network) -> Config {
    let get = |b: &str| {
        settings
            .get(&key(b, network))
            .map(|s| s.trim().to_owned())
            .unwrap_or_default()
    };
    let sources = get(SOURCES_KEY);
    let btc_sources = if sources.is_empty() {
        btc_source::default_sources(network)
    } else {
        sources
            .split(['\n', ','])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect()
    };
    let relay = get(RELAY_KEY);
    Config {
        relay_url: if relay.is_empty() {
            relay_client::default_relay(network)
                .unwrap_or("")
                .to_owned()
        } else {
            relay
        },
        watchtower_url: get(WATCHTOWER_KEY),
        btc_sources,
        bitcoind_auth: Some(get(BITCOIND_AUTH_KEY)).filter(|s| !s.is_empty()),
    }
}

fn check_url(what: &str, v: &str) -> Result<(), AppError> {
    if !v.chars().all(|c| c.is_ascii_graphic()) {
        return Err(AppError::InvalidInput(format!(
            "the {what} must be plain ASCII with no spaces"
        )));
    }
    let u = url::Url::parse(v)
        .map_err(|e| AppError::InvalidInput(format!("the {what} is not a URL: {e}")))?;
    if !u.username().is_empty() || u.password().is_some() {
        return Err(AppError::InvalidInput(format!(
            "the {what} must not carry a user name or password"
        )));
    }
    btc_source::check_transport(&u).map_err(|_| {
        AppError::InvalidInput(format!("the {what} must be https, or http to this machine"))
    })
}

/// Validate what the user typed; returns it normalized (trimmed).
pub fn validate(
    relay: &str,
    watchtower: &str,
    sources: &[String],
    auth: Option<&str>,
) -> Result<Config, AppError> {
    let relay = relay.trim();
    let watchtower = watchtower.trim();
    if !relay.is_empty() {
        check_url("relay URL", relay)?;
    }
    if !watchtower.is_empty() {
        check_url("watchtower URL", watchtower)?;
    }
    let sources: Vec<String> = sources
        .iter()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    if sources.len() > MAX_SOURCES {
        return Err(AppError::InvalidInput(format!(
            "at most {MAX_SOURCES} Bitcoin sources"
        )));
    }
    for s in &sources {
        check_url("Bitcoin source", s.strip_prefix("bitcoind+").unwrap_or(s))?;
    }
    let auth = match auth.map(str::trim) {
        None => None,
        Some("") => Some(String::new()),
        Some(a) => {
            if !a.chars().all(|c| c.is_ascii_graphic()) || !a.contains(':') {
                return Err(AppError::InvalidInput(
                    "the bitcoind RPC login must be user:password, plain ASCII".into(),
                ));
            }
            Some(a.to_owned())
        }
    };
    Ok(Config {
        relay_url: relay.to_owned(),
        watchtower_url: watchtower.to_owned(),
        btc_sources: sources,
        bitcoind_auth: auth,
    })
}

/// The sources of `cfg`, the bitcoind login put into a `bitcoind+` URL.
pub fn sources(cfg: &Config) -> Result<Vec<Box<dyn BtcSource>>, AppError> {
    cfg.btc_sources
        .iter()
        .map(|spec| {
            let spec = match (spec.strip_prefix("bitcoind+"), &cfg.bitcoind_auth) {
                (Some(rest), Some(auth)) => {
                    let (scheme, host) = rest.split_once("://").unwrap_or(("http", rest));
                    format!("bitcoind+{scheme}://{auth}@{host}")
                }
                _ => spec.clone(),
            };
            btc_source::source_from_spec(&spec)
        })
        .collect()
}

pub fn relay(cfg: &Config) -> Result<Option<RelayClient>, AppError> {
    if cfg.relay_url.is_empty() {
        return Ok(None);
    }
    RelayClient::new(&cfg.relay_url).map(Some)
}

pub fn watchtower(cfg: &Config) -> Result<Option<RelayClient>, AppError> {
    if cfg.watchtower_url.is_empty() {
        return Ok(None);
    }
    RelayClient::new(&cfg.watchtower_url).map(Some)
}

pub fn policy(network: Network) -> Policy {
    Policy::for_network(network)
}

/// The sync step: one exchange tick for `profile_id`, best effort, run by
/// the app and the daemon (`commands::sync::run_sync_steps`). It sends only
/// transactions the user signed at a Commit (spec R13, SECURITY.md).
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn sync_step(db_path: &str, profile_id: &str) {
    let Ok(conn) = crate::db::connection::open_migrated(db_path) else {
        return;
    };
    // Best effort like every sync step: no account means the exchange is off.
    let Ok(Some(account)) = crate::exchange_io::store::account(&conn, profile_id) else {
        return;
    };
    let Some(network) = Network::from_str_opt(&account.network) else {
        return;
    };
    let Ok(settings) = crate::db::queries::get_settings(&conn) else {
        return;
    };
    let cfg = config(&settings, network);
    let srcs = match sources(&cfg) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("exchange: Bitcoin sources: {e}");
            Vec::new()
        }
    };
    let relay = relay(&cfg).ok().flatten();
    let watch = watchtower(&cfg).ok().flatten();
    // The same gate as the commands (R17): a profile moved to a remote node
    // after its Commit must not reveal on that node's word, so the step runs
    // without an HNS node and the trade reads "can't check the HNS chain".
    let kind = crate::db::queries::get_wallet_profile(&conn, profile_id)
        .ok()
        .flatten()
        .map(|p| p.kind)
        .unwrap_or_default();
    let node = crate::noncustodial::rpc::NodeRpcClient::for_profile(&conn, profile_id)
        .ok()
        .filter(|n| match node_refusal(&kind, n, &settings) {
            None => true,
            Some(why) => {
                eprintln!("exchange: {profile_id}: HNS not read or sent this step: {why}");
                false
            }
        });
    let now = now_secs();
    let ctx = crate::exchange_io::job::Ctx {
        conn: &conn,
        profile_id,
        network,
        policy: policy(network),
        node: node
            .as_ref()
            .map(|n| n as &dyn crate::noncustodial::node_rpc::NodeRpc),
        sources: &srcs,
        relay: relay
            .as_ref()
            .map(|r| r as &dyn crate::exchange_io::job::Relay),
        watchtower: watch
            .as_ref()
            .map(|r| r as &dyn crate::exchange_io::job::Relay),
        now,
    };
    let report = crate::exchange_io::job::tick(&ctx).await;
    for e in &report.errors {
        eprintln!("exchange: {profile_id}: {e}");
    }
    let notify = settings
        .get("deadline_notify_enabled")
        .is_none_or(|v| v.trim() != "false");
    for ev in &report.events {
        eprintln!("exchange: {profile_id}: {ev:?}");
        if notify {
            let (summary, body) = crate::exchange_io::job::event_text(ev);
            if let Err(e) = crate::daemon::watched_names::emit_os_notification(&summary, &body) {
                eprintln!("exchange: notification not shown: {e}");
            }
        }
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
