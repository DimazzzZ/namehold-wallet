//! Shared security constants and helpers.
//!
//! Centralizes which setting keys are secrets (redacted from the renderer and
//! the audit log), which keys the renderer may write with `update_setting`,
//! and which keys are the trust root that only `update_protected_settings`
//! writes, after the user confirms the change in the secure window. The two
//! write lists are allowlists: a key on neither is refused from the renderer,
//! so a setting added later stays closed until someone decides where it goes.

use crate::noncustodial::network::Network;
use crate::noncustodial::rpc::managed_node_api_key_setting;

const MANAGED_MAIN: &str = managed_node_api_key_setting(Network::Main);
const MANAGED_TESTNET: &str = managed_node_api_key_setting(Network::Testnet);
const MANAGED_REGTEST: &str = managed_node_api_key_setting(Network::Regtest);
const MANAGED_SIMNET: &str = managed_node_api_key_setting(Network::Simnet);

/// Setting keys whose values are secrets and MUST NOT be exposed to the
/// renderer (React webview) or logged in plaintext in the audit log.
pub const SENSITIVE_SETTING_KEYS: &[&str] = &[
    "namebase_cookie",
    "namebase_cookie_v1",
    "node_rpc_api_key",
    "hsd_api_key",
    MANAGED_MAIN,
    MANAGED_TESTNET,
    MANAGED_REGTEST,
    MANAGED_SIMNET,
];

/// Setting keys the renderer may write with `update_setting`: preferences that
/// choose neither which program runs, nor which node or explorer is trusted,
/// nor where an api-key is sent, nor whether a send may go to a remote node.
/// These are the keys the frontend writes (Settings, Onboarding, the Market's
/// mainnet opt-in) minus [`PROTECTED_SETTING_KEYS`]. Every other key is
/// refused: the Namebase cookie and base URL, the LearnHNS base URL, the
/// active profile and the backend's own bookkeeping are written only by
/// backend flows.
pub const RENDERER_SETTING_KEYS: &[&str] = &[
    "onboarding_complete",
    "autostart_hsd",
    "autostart_hsd.main",
    "autostart_hsd.testnet",
    "autostart_hsd.regtest",
    "autostart_hsd.simnet",
    "background_sync_enabled",
    "close_to_tray",
    "launch_at_login",
    "address_gap_limit",
    "fee_rate_doos_per_kvb",
    "shakedex_experimental",
    "deadline_notify_enabled",
    "deadline_notify_reveal_lead_blocks",
    "deadline_notify_renewal_lead_days",
    "watchlist_notify_enabled",
    "watchlist_notify_bidding_soon_lead_blocks",
    "watchlist_notify_highest_bid_threshold_hns",
    "update_notify_enabled",
];

/// The trust root: settings that choose which program runs as hsd and with
/// which data directory (its `hsd.conf` can load plugins), which node and
/// explorer the wallet trusts, where a node api-key is sent, whether a send
/// may go through a remote node, and how long an unlocked wallet stays
/// unlocked. `update_setting` refuses them; `update_protected_settings`
/// validates each value and writes only what the user confirmed in the secure
/// window. `src/lib/protectedSettings.ts` holds the same list for the
/// frontend, and a contract test there reads this one.
pub const PROTECTED_SETTING_KEYS: &[&str] = &[
    "node_rpc_url",
    "node_rpc_api_key",
    "hsd_api_key",
    "chain_source",
    "node_mode",
    "node_mode.main",
    "node_mode.testnet",
    "node_mode.regtest",
    "node_mode.simnet",
    "allow_remote_broadcast",
    "hsd_path",
    "hsd_prefix",
    "explorer_api_url",
    "explorer_fallback_url",
    "signer_session_timeout_seconds",
];

/// Returns `true` if `key` is a sensitive setting that must be redacted.
pub fn is_sensitive_key(key: &str) -> bool {
    SENSITIVE_SETTING_KEYS.contains(&key) || crate::exchange_io::env::is_sensitive(key)
}

/// Returns `true` if the renderer may write `key` with `update_setting`.
pub fn is_renderer_writable(key: &str) -> bool {
    RENDERER_SETTING_KEYS.contains(&key)
}

/// Returns `true` if `key` is written only through
/// `update_protected_settings`.
pub fn is_protected_setting(key: &str) -> bool {
    PROTECTED_SETTING_KEYS.contains(&key)
}

/// Keep a crash from writing this process's memory, decrypted keys among it, to
/// disk: no core file, soft limit and hard limit both, and on Linux not dumpable,
/// which also covers a `core_pattern` that pipes to systemd-coredump.
#[cfg(unix)]
pub(crate) fn disable_core_dumps() -> std::io::Result<()> {
    let none = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: both calls take plain values and change only this process's limits.
    if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &none) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    #[cfg(target_os = "linux")]
    if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Whether a webview may go to `url`: the app's own pages, nowhere else. The CSP
/// says nothing about top-level navigation, so without this a compromised page
/// could send data out in a URL, or a GET to a node on a loopback port. External
/// links do not come through here: they go through the opener plugin to the
/// system browser.
pub(crate) fn navigation_allowed(url: &url::Url, dev: bool) -> bool {
    match (url.scheme(), url.host_str()) {
        ("tauri", Some("localhost")) => true,
        // How Windows serves the app's own pages.
        ("http" | "https", Some("tauri.localhost")) => url.port().is_none(),
        // `devUrl` in tauri.conf.json.
        ("http", Some("localhost")) => dev && url.port() == Some(1420),
        _ => false,
    }
}

/// Holds every webview of the app to [`navigation_allowed`].
pub(crate) fn navigation_guard<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("navigation-guard")
        .on_navigation(|_, url| {
            let allowed = navigation_allowed(url, tauri::is_dev());
            if !allowed {
                eprintln!(
                    "refused a webview navigation to {}://{}",
                    url.scheme(),
                    url.host_str().unwrap_or("")
                );
            }
            allowed
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_webview_goes_to_the_apps_own_pages_and_nowhere_else() {
        let allowed = |u: &str, dev| navigation_allowed(&url::Url::parse(u).unwrap(), dev);
        for own in [
            "tauri://localhost/",
            "tauri://localhost/secure.html",
            "http://tauri.localhost/",
            "https://tauri.localhost/secure.html",
        ] {
            assert!(allowed(own, false), "{own}");
        }
        for elsewhere in [
            "http://127.0.0.1:12037/",
            "http://127.0.0.1:12039/wallet",
            "http://localhost:12037/",
            "http://localhost:1420/",
            "http://[::1]:12037/",
            "https://evil.example/?seed=x",
            "http://tauri.localhost.evil.example/",
            "http://tauri.localhost:12037/",
            "tauri://evil.example/",
            "file:///etc/passwd",
            "data:text/html,x",
            "about:blank",
        ] {
            assert!(!allowed(elsewhere, false), "{elsewhere}");
        }
        assert!(allowed("http://localhost:1420/", true));
        assert!(!allowed("http://localhost:12037/", true), "dev: not a node");
        assert!(!allowed("http://127.0.0.1:12037/", true));
    }

    /// The guard above guards nothing unless the app installs it.
    #[test]
    fn the_app_installs_the_navigation_guard() {
        assert!(include_str!("lib.rs").contains(".plugin(security::navigation_guard())"));
    }

    /// The hard limit too: a soft limit alone a process can raise back.
    #[cfg(unix)]
    #[test]
    fn disable_core_dumps_leaves_no_core_limit() {
        disable_core_dumps().unwrap();
        let mut now = libc::rlimit {
            rlim_cur: 1,
            rlim_max: 1,
        };
        assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut now) }, 0);
        assert_eq!((now.rlim_cur, now.rlim_max), (0, 0));
    }

    /// The app and the daemon both start hardened: the call comes first in each
    /// entry point, before anything holds a secret.
    #[test]
    fn the_app_and_the_daemon_harden_first_thing() {
        for (at, source, entry, call) in [
            (
                "lib.rs",
                include_str!("lib.rs"),
                "pub fn run() {",
                "harden_process();",
            ),
            (
                "namehold-syncd.rs",
                include_str!("bin/namehold-syncd.rs"),
                "fn main() {",
                "namehold_wallet_lib::harden_process();",
            ),
        ] {
            let body = source.split_once(entry).expect(at).1;
            let first = body
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty() && !l.starts_with("//"));
            assert_eq!(first, Some(call), "{at}");
        }
    }

    #[test]
    fn sensitive_keys_cover_known_secrets() {
        assert!(is_sensitive_key("namebase_cookie"));
        assert!(is_sensitive_key("namebase_cookie_v1"));
        assert!(is_sensitive_key("node_rpc_api_key"));
        assert!(is_sensitive_key("hsd_api_key"));
        for network in Network::ALL {
            assert!(is_sensitive_key(managed_node_api_key_setting(network)));
        }
        assert!(!is_sensitive_key("hsd_prefix"));
        assert!(!is_sensitive_key("__has_namebase_cookie"));
    }

    #[test]
    fn the_renderer_writes_no_host_override_secret_or_trust_root_key() {
        for key in [
            "namebase_base_url",
            "learnhns_base_url",
            "namebase_cookie",
            "namebase_cookie_v1",
            "active_wallet_profile_id",
            "node_rpc_url",
            "node_rpc_api_key",
            "hsd_path",
            "hsd_prefix",
            "explorer_api_url",
            "chain_source",
            "allow_remote_broadcast",
        ] {
            assert!(!is_renderer_writable(key), "{key} must not be writable");
        }
        // Generated and stored by `start_hsd` alone; a key the webview chose
        // would be a key the webview knows.
        for network in Network::ALL {
            let key = managed_node_api_key_setting(network);
            assert!(!is_renderer_writable(key), "{key} must not be writable");
            assert!(!is_protected_setting(key), "{key} must not be settable");
        }
        assert!(!is_renderer_writable("a_key_nobody_listed"));
    }

    #[test]
    fn the_trust_root_covers_the_binary_the_node_the_explorer_and_their_secrets() {
        for key in [
            "node_rpc_url",
            "node_rpc_api_key",
            "hsd_api_key",
            "chain_source",
            "allow_remote_broadcast",
            "hsd_path",
            "hsd_prefix",
            "explorer_api_url",
            "explorer_fallback_url",
        ] {
            assert!(is_protected_setting(key), "{key} must be protected");
        }
        // Every network's own node mode is the trust root too, like the
        // app-wide one it overrides.
        for n in Network::ALL {
            let key = format!("node_mode.{}", n.as_str());
            assert!(is_protected_setting(&key), "{key} must be protected");
        }
    }

    #[test]
    fn every_network_has_its_own_autostart_key_on_the_renderer_list() {
        for n in Network::ALL {
            let key = format!("autostart_hsd.{}", n.as_str());
            assert!(is_renderer_writable(&key), "{key} must be writable");
        }
    }

    #[test]
    fn no_key_is_both_renderer_writable_and_protected() {
        for key in RENDERER_SETTING_KEYS {
            assert!(!is_protected_setting(key), "{key} is on both lists");
        }
    }

    #[test]
    fn every_secret_the_renderer_can_set_goes_through_the_confirmation() {
        for key in SENSITIVE_SETTING_KEYS {
            assert!(!is_renderer_writable(key), "{key} is a secret");
        }
    }
}
