//! The webview's Content-Security-Policy, read from every app config
//! (`tauri.conf*.json`, its `csp` and `devCsp`).
//!
//! The renderer reaches the node and the explorer only through Rust
//! commands; nothing in `src/` fetches a URL. A loopback source in
//! `connect-src` therefore served nobody but a compromised webview, which it
//! let post to an hsd on the same machine (hsd parses a `text/plain` body as
//! JSON, so no preflight stands in the way). The browser QA mock
//! (`src/lib/webqa-mock.ts`) runs outside Tauri and is not under this CSP.
//!
//! Dev mode needs no loopback source either: on desktop, Tauri injects the
//! CSP only into the assets it serves itself (tauri-codegen embeds it, the
//! `tauri://` protocol adds the header); `tauri dev` loads the Vite server at
//! `devUrl` directly, and Vite's own pages carry no CSP.

use std::collections::HashMap;

/// Every CSP an app config sets, as `(where, policy)`.
fn app_csps() -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = Vec::new();
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        let file = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !(file.starts_with("tauri.conf") && file.ends_with(".json")) {
            continue;
        }
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for key in ["csp", "devCsp"] {
            match &doc["app"]["security"][key] {
                serde_json::Value::Null => {}
                serde_json::Value::String(s) => found.push((format!("{file} {key}"), s.clone())),
                // Tauri also takes a directive map; read it as the string it
                // stands for.
                serde_json::Value::Object(map) => {
                    let policy = map
                        .iter()
                        .map(|(directive, sources)| {
                            let sources = match sources {
                                serde_json::Value::Array(a) => a
                                    .iter()
                                    .filter_map(|s| s.as_str())
                                    .collect::<Vec<_>>()
                                    .join(" "),
                                other => other.as_str().unwrap_or("").to_string(),
                            };
                            format!("{directive} {sources}")
                        })
                        .collect::<Vec<_>>()
                        .join("; ");
                    found.push((format!("{file} {key}"), policy));
                }
                other => panic!("{file} {key}: unexpected CSP value {other}"),
            }
        }
    }
    found
}

/// A policy's directives: name -> sources.
fn directives(policy: &str) -> HashMap<String, Vec<String>> {
    policy
        .split(';')
        .filter_map(|d| {
            let mut words = d.split_whitespace();
            let name = words.next()?.to_ascii_lowercase();
            Some((name, words.map(str::to_string).collect()))
        })
        .collect()
}

/// Whether one CSP source lets a request reach a port on this machine: a
/// loopback host (`localhost`, `*.localhost`, `127.0.0.0/8`, `[::1]`,
/// `0.0.0.0`), a wildcard host, `*`, or a bare network scheme (`http:`,
/// `ws:`, ...), which allows every host. Tauri's own IPC origin on Windows
/// (`http://ipc.localhost`, port 80, served by the app itself) is the one
/// `.localhost` source allowed.
fn reaches_loopback(source: &str) -> bool {
    let s = source.trim().to_ascii_lowercase();
    if s.starts_with('\'') {
        return false; // 'self', 'none', nonces, hashes
    }
    if s == "*" {
        return true;
    }
    if let Some(scheme) = s.strip_suffix(':') {
        // A bare scheme source: only the app's own non-network schemes.
        return !matches!(scheme, "ipc" | "data" | "blob" | "tauri" | "asset");
    }
    if s == "http://ipc.localhost" {
        return false;
    }
    let rest = s.split_once("://").map_or(s.as_str(), |(_, r)| r);
    let authority = rest.split('/').next().unwrap_or("");
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    host == "*"
        || host.starts_with("*.")
        || host == "localhost"
        || host.ends_with(".localhost")
        || host == "0.0.0.0"
        || host == "::1"
        || host.split('.').next() == Some("127")
}

/// The sources of `policy` through which a request can reach loopback:
/// `connect-src`, or `default-src` when it has none.
fn loopback_connect_sources(policy: &str) -> Vec<String> {
    let d = directives(policy);
    d.get("connect-src")
        .or_else(|| d.get("default-src"))
        .into_iter()
        .flatten()
        .filter(|s| reaches_loopback(s))
        .cloned()
        .collect()
}

/// The checker itself: it finds what the policy carried before, and passes
/// what the app needs.
#[test]
fn the_loopback_check_sees_loopback_sources() {
    let before = "default-src 'self'; connect-src 'self' ipc: http://ipc.localhost \
                  https://e.hnsfans.com https://sunset.namebase.io http://127.0.0.1:* \
                  http://localhost:*; frame-src 'none'";
    assert_eq!(
        loopback_connect_sources(before),
        vec!["http://127.0.0.1:*", "http://localhost:*"]
    );
    for bad in [
        "*",
        "http:",
        "ws:",
        "ws://localhost:1420",
        "http://[::1]:12037",
        "http://127.1.2.3",
        "http://0.0.0.0:12037",
        "http://*:12037",
        "http://evil.localhost",
        "http://ipc.localhost:12037",
    ] {
        assert!(reaches_loopback(bad), "{bad}");
    }
    for ok in [
        "'self'",
        "ipc:",
        "http://ipc.localhost",
        "https://e.hnsfans.com",
        "data:",
    ] {
        assert!(!reaches_loopback(ok), "{ok}");
    }
    // No connect-src: default-src governs connections.
    assert_eq!(
        loopback_connect_sources("default-src 'self' http://localhost:*"),
        vec!["http://localhost:*"]
    );
}

/// No request from the webview reaches a port on this machine: not hsd's
/// node or wallet API, not anything else listening there.
#[test]
fn the_webview_csp_lets_no_request_reach_a_loopback_port() {
    let csps = app_csps();
    assert!(!csps.is_empty(), "no CSP found in tauri.conf*.json");
    for (at, policy) in csps {
        assert!(
            directives(&policy).contains_key("connect-src"),
            "{at}: no connect-src"
        );
        let loopback = loopback_connect_sources(&policy);
        assert!(
            loopback.is_empty(),
            "{at} lets the webview reach {loopback:?}"
        );
    }
}

/// `form-action` does not fall back to `default-src`, and a form posts
/// whatever `connect-src` says: without `form-action 'none'` a compromised
/// webview could submit a `text/plain` form to hsd on loopback. The app has
/// no form that submits.
#[test]
fn the_webview_csp_allows_no_form_submission() {
    for (at, policy) in app_csps() {
        assert_eq!(
            directives(&policy).get("form-action"),
            Some(&vec!["'none'".to_string()]),
            "{at}"
        );
    }
}

/// A Service Worker registered by a compromised main webview on the shared
/// app origin could answer the next secure-prompt window's requests. The app
/// runs no worker of any kind, and `worker-src` would otherwise fall back to
/// `script-src 'self'`.
#[test]
fn the_webview_csp_allows_no_worker() {
    for (at, policy) in app_csps() {
        assert_eq!(
            directives(&policy).get("worker-src"),
            Some(&vec!["'none'".to_string()]),
            "{at}"
        );
    }
}
