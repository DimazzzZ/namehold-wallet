//! The trust-root settings (`security::PROTECTED_SETTING_KEYS`): which hsd
//! program runs, which node and explorer are trusted, where a node api-key
//! goes. The renderer writes them only through `update_protected_settings`,
//! which validates every value, asks the user in the secure window with one
//! row per change and writes only what was confirmed. These tests drive the
//! command body (`update_protected_settings_confirmed`) over a mock app,
//! answering the secure window through the `secure_prompt` test seam, and
//! prove the old route to running a renderer-chosen program is closed.

use std::path::{Path, PathBuf};

use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;

use crate::commands::node::resolve_probe_api_key;
use crate::commands::secure_prompt::{
    clear_test_answers, on_next_test_answer, push_test_answer, take_test_requests,
    SecurePromptRequest, SecurePromptResult,
};
use crate::commands::settings::{
    get_audit_log, update_protected_settings_confirmed, update_setting,
};
use crate::db;
use crate::error::AppError;
use crate::protected_settings::{
    program_file_problem, refuse_network_path, validate_hsd_path, validate_hsd_prefix,
    validate_value, SettingChange, CONFIRM_TITLE,
};
use crate::AppState;

type App = tauri::App<tauri::test::MockRuntime>;

fn migrated_conn() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    db::migrations::run(&conn).unwrap();
    conn
}

fn app_with(conn: rusqlite::Connection) -> App {
    mock_builder()
        .manage(AppState {
            db: std::sync::Mutex::new(conn),
            signer: std::sync::Mutex::new(None),
            secure_prompts: std::sync::Mutex::new(std::collections::HashMap::new()),
            hsd_children: std::sync::Mutex::new(std::collections::HashMap::new()),
            hsd_starting: std::sync::Mutex::new(std::collections::HashSet::new()),
            node_rpc_alive: std::sync::atomic::AtomicBool::new(false),
            sync_status: std::sync::Arc::new(tokio::sync::Mutex::new(
                crate::commands::sync::SyncStatus::default(),
            )),
        })
        .build(mock_context(noop_assets()))
        .expect("mock app")
}

fn raw_setting(app: &App, key: &str) -> Option<String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    db::queries::get_settings(&conn).unwrap().get(key).cloned()
}

/// The audit table as stored. `get_audit_log` redacts again on the way out, so
/// it cannot show what was written.
fn stored_audit_details(app: &App) -> Vec<String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().unwrap();
    let mut stmt = conn.prepare("SELECT detail FROM audit_log").unwrap();
    let rows = stmt.query_map([], |r| r.get(0)).unwrap();
    rows.map(Result::unwrap).collect()
}

fn changes(pairs: &[(&str, &str)]) -> Vec<SettingChange> {
    pairs
        .iter()
        .map(|(k, v)| SettingChange {
            key: k.to_string(),
            value: v.to_string(),
        })
        .collect()
}

/// Forget earlier prompts and queue the answer to the next one.
fn answer(confirmed: bool) {
    let _ = take_test_requests();
    clear_test_answers();
    push_test_answer(SecurePromptResult {
        value: None,
        confirmed,
    });
}

async fn save(app: &App, pairs: &[(&str, &str)]) -> Result<Vec<String>, AppError> {
    let state = app.state::<AppState>();
    update_protected_settings_confirmed(app.handle(), &state, &changes(pairs)).await
}

/// The rows the window showed for the one prompt answered since `answer`.
fn rows_of(req: &SecurePromptRequest) -> Vec<(String, String)> {
    req.details.as_ref().unwrap()["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["label"].as_str().unwrap().to_string(),
                r["value"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn only_request() -> SecurePromptRequest {
    let mut reqs = take_test_requests();
    assert_eq!(reqs.len(), 1, "exactly one prompt");
    reqs.remove(0)
}

/// No prompt was shown; the answer queued for one is dropped unused.
fn assert_nothing_asked() {
    assert!(take_test_requests().is_empty(), "nothing must be asked");
    clear_test_answers();
}

async fn audit_details(app: &App) -> Vec<String> {
    get_audit_log(app.state(), Some(100))
        .await
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["detail"].as_str().unwrap().to_string())
        .collect()
}

/// A fresh directory of this test process.
fn temp_dir(tag: &str) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "namehold-protected-{tag}-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // The canonical form, so paths compare with what the backend stores
    // (macOS's temp dir is a link into /private).
    std::fs::canonicalize(&dir).unwrap()
}

#[cfg(unix)]
fn write_script(path: &Path, body: &str, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// One renderer write used to make any URL "the saved node", and "Test
/// connection" then sent the stored api-key there. The write is refused, so
/// the typed URL stays an endpoint that gets no key.
#[tokio::test]
async fn a_refused_url_write_leaves_the_stored_key_away_from_the_attacker() {
    let conn = migrated_conn();
    db::queries::set_setting(&conn, "node_rpc_api_key", "stored-secret").unwrap();
    let app = app_with(conn);
    let evil = "https://attacker.example:12037";

    assert!(
        update_setting(app.state(), "node_rpc_url".to_string(), evil.to_string())
            .await
            .is_err()
    );

    let settings = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().unwrap();
        db::queries::get_settings(&conn).unwrap()
    };
    assert_eq!(resolve_probe_api_key(evil, None, &settings), "");
}

// --- the confirmation --------------------------------------------------------

#[tokio::test]
async fn a_confirmed_change_is_written_after_one_row_per_change() {
    let app = app_with(migrated_conn());

    answer(true);
    let written = save(
        &app,
        &[
            ("node_rpc_url", "https://node.example.com:12037"),
            ("explorer_api_url", "https://shakeshift.com"),
        ],
    )
    .await
    .unwrap();

    let req = only_request();
    assert_eq!(req.mode, "confirm");
    assert_eq!(req.title, CONFIRM_TITLE);
    assert_eq!(
        rows_of(&req),
        vec![
            (
                "Node RPC URL".to_string(),
                "http://127.0.0.1:12037 → https://node.example.com:12037".to_string()
            ),
            (
                "Explorer URL".to_string(),
                "the network's default explorer → https://shakeshift.com".to_string()
            ),
        ]
    );
    assert_eq!(written, vec!["node_rpc_url", "explorer_api_url"]);
    assert_eq!(
        raw_setting(&app, "node_rpc_url").as_deref(),
        Some("https://node.example.com:12037")
    );
    assert_eq!(
        raw_setting(&app, "explorer_api_url").as_deref(),
        Some("https://shakeshift.com")
    );
    let log = audit_details(&app).await;
    assert_eq!(log.len(), 2, "{log:?}");
    assert!(log
        .iter()
        .any(|d| d.contains("https://node.example.com:12037")));
    assert!(log.iter().any(|d| d.contains("https://shakeshift.com")));
}

#[tokio::test]
async fn a_declined_change_writes_nothing() {
    let app = app_with(migrated_conn());

    answer(false);
    let err = save(
        &app,
        &[
            ("node_rpc_url", "https://attacker.example"),
            ("chain_source", "remote_node"),
            ("allow_remote_broadcast", "true"),
        ],
    )
    .await
    .unwrap_err();

    assert!(matches!(err, AppError::UserRejected), "{err}");
    assert_eq!(take_test_requests().len(), 1, "the user was asked");
    assert_eq!(
        raw_setting(&app, "node_rpc_url").as_deref(),
        Some("http://127.0.0.1:12037")
    );
    assert_eq!(
        raw_setting(&app, "chain_source").as_deref(),
        Some("local_node")
    );
    assert_eq!(
        raw_setting(&app, "allow_remote_broadcast").as_deref(),
        Some("false")
    );
    assert!(audit_details(&app).await.is_empty());
}

/// A save that repeats what is in effect, including a default a missing row
/// reads as, asks nothing and writes nothing.
#[tokio::test]
async fn values_equal_to_those_in_effect_ask_nothing_and_write_nothing() {
    let conn = migrated_conn();
    // Missing rows read as their defaults: local node, no remote sending.
    conn.execute(
        "DELETE FROM settings WHERE key IN ('chain_source', 'allow_remote_broadcast')",
        [],
    )
    .unwrap();
    let app = app_with(conn);

    // Queued so that a wrong prompt would answer (and write) instead of hang.
    answer(true);
    let written = save(
        &app,
        &[
            ("node_rpc_url", "http://127.0.0.1:12037"),
            ("chain_source", "local_node"),
            ("allow_remote_broadcast", "false"),
            ("node_mode", "full"),
            ("node_mode.main", "full"),
            ("node_rpc_api_key", ""),
            ("hsd_path", ""),
        ],
    )
    .await
    .unwrap();

    assert!(written.is_empty(), "{written:?}");
    assert_nothing_asked();
    assert_eq!(raw_setting(&app, "chain_source"), None);
    assert_eq!(raw_setting(&app, "node_mode"), None);
    assert_eq!(raw_setting(&app, "node_mode.main"), None);
    assert!(audit_details(&app).await.is_empty());
}

#[tokio::test]
async fn an_empty_request_asks_nothing() {
    let app = app_with(migrated_conn());

    answer(true);
    assert!(save(&app, &[]).await.unwrap().is_empty());
    assert_nothing_asked();
}

/// A network's own node mode falls back to the app-wide one, which the row
/// shows as the value it replaces.
#[tokio::test]
async fn a_network_node_mode_row_shows_the_mode_in_effect_and_the_network() {
    let conn = migrated_conn();
    db::queries::set_setting(&conn, "node_mode", "spv").unwrap();
    let app = app_with(conn);

    answer(true);
    save(&app, &[("node_mode.regtest", "full")]).await.unwrap();

    assert_eq!(
        rows_of(&only_request()),
        vec![(
            "Node mode (regtest)".to_string(),
            "SPV (headers only) → full node".to_string()
        )]
    );
    assert_eq!(
        raw_setting(&app, "node_mode.regtest").as_deref(),
        Some("full")
    );
}

// --- the api-key is never shown ----------------------------------------------

fn request_text(req: &SecurePromptRequest) -> String {
    serde_json::to_string(req).unwrap()
}

#[tokio::test]
async fn an_api_key_row_says_set_and_never_shows_the_key() {
    let app = app_with(migrated_conn());

    answer(true);
    save(&app, &[("node_rpc_api_key", "first-secret-value")])
        .await
        .unwrap();

    let req = only_request();
    assert_eq!(
        rows_of(&req),
        vec![("Node RPC API key".to_string(), "set".to_string())]
    );
    assert!(!request_text(&req).contains("first-secret-value"));
    assert_eq!(
        req.details.as_ref().unwrap()["confirmLabel"],
        "Save settings"
    );
    assert_eq!(
        raw_setting(&app, "node_rpc_api_key").as_deref(),
        Some("first-secret-value")
    );
    // The audit entry is stored redacted, as SENSITIVE_SETTING_KEYS says.
    let log = stored_audit_details(&app);
    assert_eq!(log.len(), 1);
    assert!(log[0].contains("\"***\""), "{log:?}");
    assert!(!log[0].contains("first-secret-value"), "{log:?}");
}

#[tokio::test]
async fn an_api_key_row_says_changed_or_cleared_and_shows_neither_key() {
    let conn = migrated_conn();
    db::queries::set_setting(&conn, "node_rpc_api_key", "old-secret-value").unwrap();
    db::queries::set_setting(&conn, "hsd_api_key", "legacy-secret-value").unwrap();
    let app = app_with(conn);

    answer(true);
    save(
        &app,
        &[
            ("node_rpc_api_key", "new-secret-value"),
            ("hsd_api_key", ""),
        ],
    )
    .await
    .unwrap();

    let req = only_request();
    assert_eq!(
        rows_of(&req),
        vec![
            ("Node RPC API key".to_string(), "changed".to_string()),
            ("hsd API key (legacy)".to_string(), "cleared".to_string()),
        ]
    );
    let text = request_text(&req);
    for secret in [
        "old-secret-value",
        "new-secret-value",
        "legacy-secret-value",
    ] {
        assert!(!text.contains(secret), "{secret} shown: {text}");
    }
    assert_eq!(
        raw_setting(&app, "node_rpc_api_key").as_deref(),
        Some("new-secret-value")
    );
    assert_eq!(raw_setting(&app, "hsd_api_key").as_deref(), Some(""));
}

// --- what is refused ---------------------------------------------------------

#[tokio::test]
async fn a_node_mode_of_no_network_is_refused_even_with_a_valid_value() {
    let app = app_with(migrated_conn());

    answer(true);
    let err = save(&app, &[("node_mode.attacker", "full")])
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(_)), "{err}");
    assert_nothing_asked();
    assert_eq!(raw_setting(&app, "node_mode.attacker"), None);
}

#[tokio::test]
async fn a_key_that_is_not_a_connection_setting_is_refused_unasked() {
    let conn = migrated_conn();
    db::queries::set_setting(&conn, "namebase_base_url", "before").unwrap();
    let app = app_with(conn);

    for key in [
        "close_to_tray",
        "namebase_base_url",
        "learnhns_base_url",
        "active_wallet_profile_id",
        "some_future_key",
    ] {
        answer(true);
        let err = save(&app, &[(key, "https://attacker.example")])
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)), "{key}: {err}");
        assert_nothing_asked();
    }
    assert_eq!(
        raw_setting(&app, "namebase_base_url").as_deref(),
        Some("before")
    );
    assert!(audit_details(&app).await.is_empty());
}

/// All or nothing: one bad value refuses the valid ones beside it.
#[tokio::test]
async fn one_invalid_value_refuses_the_whole_request_unasked() {
    let app = app_with(migrated_conn());

    answer(true);
    let err = save(
        &app,
        &[
            ("explorer_api_url", "https://fine.example"),
            ("chain_source", "attacker_node"),
        ],
    )
    .await
    .unwrap_err();

    assert!(matches!(err, AppError::InvalidInput(_)), "{err}");
    assert_nothing_asked();
    assert_eq!(raw_setting(&app, "explorer_api_url").as_deref(), Some(""));
}

#[tokio::test]
async fn a_key_given_twice_is_refused_unasked() {
    let app = app_with(migrated_conn());

    answer(true);
    let err = save(
        &app,
        &[
            ("node_rpc_url", "https://a.example"),
            ("node_rpc_url", "https://b.example"),
        ],
    )
    .await
    .unwrap_err();

    assert!(matches!(err, AppError::InvalidInput(m) if m.contains("twice")));
    assert_nothing_asked();
}

/// `guard_transport`'s rule holds for a confirmed change too: with a remote
/// http URL stored before URLs had to be https, no api-key, typed or supplied
/// by a data directory's hsd.conf, is set up to go there. A new remote http URL
/// never gets this far.
#[tokio::test]
async fn an_api_key_is_never_set_up_to_go_over_cleartext_to_a_remote_node() {
    let conn = migrated_conn();
    db::queries::set_setting(&conn, "node_rpc_url", "http://node.example.com:12037").unwrap();
    let app = app_with(conn);

    // A key set while the URL is remote http.
    answer(true);
    let err = save(&app, &[("node_rpc_api_key", "new-key")])
        .await
        .unwrap_err();
    assert!(
        matches!(&err, AppError::InvalidInput(m) if m.contains("plaintext")),
        "{err}"
    );
    assert_nothing_asked();

    // A data directory whose hsd.conf carries a key, with no key stored.
    let dir = temp_dir("conf-key");
    std::fs::write(dir.join("hsd.conf"), "api-key: conf-key\n").unwrap();
    answer(true);
    let err = save(&app, &[("hsd_prefix", dir.to_str().unwrap())])
        .await
        .unwrap_err();
    assert!(
        matches!(&err, AppError::InvalidInput(m) if m.contains("plaintext")),
        "{err}"
    );
    assert_nothing_asked();
    assert_eq!(
        raw_setting(&app, "node_rpc_api_key").unwrap_or_default(),
        ""
    );
    assert_eq!(raw_setting(&app, "hsd_prefix").unwrap_or_default(), "");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The row is the only guard on where a stored key goes, so it says so when a
/// new host would receive a key the user did not type in this same change.
#[tokio::test]
async fn moving_the_node_to_another_host_warns_who_receives_the_key() {
    const WARN: &str = "Your node API key will be sent to";
    async fn warnings(seed: &[(&str, &str)], pairs: &[(&str, &str)]) -> Vec<String> {
        let conn = migrated_conn();
        for (k, v) in seed {
            db::queries::set_setting(&conn, k, v).unwrap();
        }
        let app = app_with(conn);
        answer(true);
        save(&app, pairs).await.unwrap();
        rows_of(&only_request())
            .into_iter()
            .filter(|(label, _)| label == "Warning")
            .map(|(_, value)| value)
            .collect()
    }
    let key = ("node_rpc_api_key", "stored-key");
    let new_url = ("node_rpc_url", "https://node.example.com:12037");

    let warned = warnings(&[key], &[new_url]).await;
    assert_eq!(warned, [format!("{WARN} node.example.com.")]);

    let dir = temp_dir("warn-conf");
    std::fs::write(dir.join("hsd.conf"), "api-key: conf-key\n").unwrap();
    let conf = ("hsd_prefix", dir.to_str().unwrap());
    assert_eq!(
        warnings(&[conf], &[new_url]).await,
        [format!("{WARN} node.example.com.")],
        "a key read from hsd.conf goes there too"
    );
    let _ = std::fs::remove_dir_all(&dir);

    assert!(warnings(&[], &[new_url]).await.is_empty(), "no key to send");
    assert!(
        warnings(&[key], &[new_url, ("node_rpc_api_key", "typed-key")])
            .await
            .is_empty(),
        "the key typed for this host replaces the old one"
    );
    assert!(
        warnings(&[key], &[new_url, ("node_rpc_api_key", "")])
            .await
            .is_empty(),
        "the key is cleared"
    );
    assert!(
        warnings(
            &[key, ("node_rpc_url", "https://node.example.com:1")],
            &[new_url]
        )
        .await
        .is_empty(),
        "same host, another port"
    );
    assert!(
        warnings(&[key], &[("signer_session_timeout_seconds", "60")])
            .await
            .is_empty(),
        "the node does not move"
    );
}

#[tokio::test]
#[serial_test::serial(protected_settings)]
async fn an_explorer_this_app_does_not_know_is_warned_about_and_can_still_be_saved() {
    async fn warnings(pairs: &[(&str, &str)]) -> Vec<String> {
        let app = app_with(migrated_conn());
        answer(true);
        save(&app, pairs).await.unwrap();
        rows_of(&only_request())
            .into_iter()
            .filter(|(label, _)| label == "Warning")
            .map(|(_, value)| value)
            .collect()
    }
    for key in ["explorer_api_url", "explorer_fallback_url"] {
        let found = warnings(&[(key, "https://e-hnsfans.example")]).await;
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].starts_with("e-hnsfans.example is not an explorer"));
        // Near misses of a known host are what this is for.
        for lookalike in [
            "https://e.hnsfans.com.evil.example",
            "https://shakeshift.co",
            "https://x.shakeshift.com",
        ] {
            assert_eq!(warnings(&[(key, lookalike)]).await.len(), 1, "{lookalike}");
        }
        for known in [
            "https://e.hnsfans.com",
            "https://shakeshift.com/",
            "https://E.HNSFANS.COM",
            "http://127.0.0.1:8080",
        ] {
            assert!(warnings(&[(key, known)]).await.is_empty(), "{known}");
        }
    }
    let both = warnings(&[
        ("explorer_api_url", "https://a.example"),
        ("explorer_fallback_url", "https://b.example"),
    ])
    .await;
    assert_eq!(both.len(), 2, "{both:?}");
    // A node URL is not an explorer.
    assert!(warnings(&[("node_rpc_url", "https://node.example:1")])
        .await
        .is_empty());
}

#[test]
fn urls_must_be_http_or_https_with_a_host() {
    for key in ["node_rpc_url", "explorer_api_url", "explorer_fallback_url"] {
        for bad in [
            "ftp://node.example",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "not a url",
            "node.example.com:12037",
            "http://",
            "https://:12037",
            "https://node.example\n.attacker.example",
            "https://node.exam\tple",
            "https://node.example/a b",
            "https://n\u{43e}de.example", // a Cyrillic look-alike letter
        ] {
            assert!(
                matches!(validate_value(key, bad), Err(AppError::InvalidInput(_))),
                "{key} = {bad} must be refused"
            );
        }
        // The row must name the host the app connects to.
        for (bad, why) in [
            (
                "https://127.0.0.1:12037@evil.example",
                "user name or password",
            ),
            ("https://127.0.0.1%2eevil.example/", "spell its host"),
            ("https://%6Eode.example/", "spell its host"),
            ("http://2130706433/", "spell its host"),
            ("http://0x7f.1/", "spell its host"),
            ("http://127.1/", "spell its host"),
            ("http://[0:0:0:0:0:0:0:1]/", "spell its host"),
            ("https:/node.example", "spell its host"),
            ("https:node.example", "spell its host"),
            (
                "https://node.example:pw@node.example",
                "user name or password",
            ),
            ("https://node.example@node.example", "user name or password"),
            // Plain http is for this machine only.
            ("http://node.example:12037", "https://"),
            ("http://10.0.0.5:12037", "https://"),
            ("http://localhost.evil.example:12037", "https://"),
        ] {
            let e = validate_value(key, bad).expect_err(bad);
            assert!(
                matches!(&e, AppError::InvalidInput(m) if m.contains(why)),
                "{key} = {bad}: {e}"
            );
        }
        for good in [
            "HTTPS://Node.Example.com:12037/",
            "http://localhost:12037",
            "http://[::1]:12037",
            "https://[2001:db8::1]:12037/path",
            "https://xn--nde-3ma.example/",
        ] {
            assert_eq!(validate_value(key, good).unwrap(), good);
        }
        assert_eq!(
            validate_value(key, "  https://node.example.com:12037 ").unwrap(),
            "https://node.example.com:12037"
        );
        assert_eq!(
            validate_value(key, "http://127.0.0.1:14037").unwrap(),
            "http://127.0.0.1:14037"
        );
        assert_eq!(validate_value(key, "").unwrap(), "");
    }
}

#[test]
fn choices_take_only_the_values_the_backend_reads() {
    for (key, good, bad) in [
        ("chain_source", "remote_node", "remote"),
        ("allow_remote_broadcast", "true", "yes"),
        ("node_mode", "spv", "light"),
        ("node_mode.testnet", "full", "FULL"),
    ] {
        assert_eq!(validate_value(key, good).unwrap(), good);
        assert!(
            matches!(validate_value(key, bad), Err(AppError::InvalidInput(_))),
            "{key} = {bad}"
        );
        assert!(
            matches!(validate_value(key, ""), Err(AppError::InvalidInput(_))),
            "{key} empty"
        );
    }
}

#[test]
fn the_session_timeout_is_a_whole_number_of_seconds_the_session_uses_as_typed() {
    let key = "signer_session_timeout_seconds";
    assert_eq!(validate_value(key, "60").unwrap(), "60");
    assert_eq!(validate_value(key, "3600").unwrap(), "3600");
    assert_eq!(validate_value(key, "").unwrap(), "");
    for bad in [
        "0",
        "59",
        "3601",
        "-5",
        "1.5",
        "abc",
        "18446744073709551616",
    ] {
        assert!(
            matches!(validate_value(key, bad), Err(AppError::InvalidInput(_))),
            "{bad}"
        );
    }
}

// --- the hsd program and its data directory ----------------------------------

#[cfg(unix)]
#[test]
fn an_hsd_program_must_be_an_absolute_path_to_an_executable_file() {
    let dir = temp_dir("hsd-path");
    let program = dir.join("hsd");
    write_script(&program, "#!/bin/sh\necho 8.0.0\n", 0o755);
    let plain = dir.join("not-executable");
    write_script(&plain, "#!/bin/sh\necho 8.0.0\n", 0o644);

    let refused = |value: &str, why: &str| {
        let e = validate_hsd_path(value).expect_err(value);
        assert!(
            matches!(&e, AppError::InvalidInput(m) if m.contains(why)),
            "{value}: {e}"
        );
    };
    refused("hsd", "full path");
    refused("bin/hsd", "full path");
    refused(dir.join("missing").to_str().unwrap(), "cannot be found");
    refused(dir.to_str().unwrap(), "not a file");
    refused(plain.to_str().unwrap(), "not executable");

    assert_eq!(
        validate_hsd_path(program.to_str().unwrap()).unwrap(),
        program.to_str().unwrap()
    );
    assert_eq!(validate_hsd_path("").unwrap(), "", "empty = auto-detect");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A link is shown and stored as the file it points at, so what the user
/// confirms is what runs.
#[cfg(unix)]
#[tokio::test]
async fn an_hsd_program_given_by_a_link_is_confirmed_and_stored_as_its_target() {
    let dir = temp_dir("hsd-link");
    let target = dir.join("hsd-8.0.0");
    write_script(&target, "#!/bin/sh\necho 8.0.0\n", 0o755);
    let link = dir.join("hsd");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let app = app_with(migrated_conn());

    answer(true);
    save(&app, &[("hsd_path", link.to_str().unwrap())])
        .await
        .unwrap();

    let target = target.to_str().unwrap();
    assert_eq!(
        rows_of(&only_request()),
        vec![("hsd program".to_string(), format!("auto-detect → {target}"))]
    );
    assert_eq!(raw_setting(&app, "hsd_path").as_deref(), Some(target));

    // The link again: the program in effect is unchanged, so nothing is asked.
    answer(true);
    assert!(save(&app, &[("hsd_path", link.to_str().unwrap())])
        .await
        .unwrap()
        .is_empty());
    assert_nothing_asked();
    let _ = std::fs::remove_dir_all(&dir);
}

/// The path is confirmed once and the file runs every time, so nobody but its
/// owner may be able to change it.
#[cfg(unix)]
#[test]
fn an_hsd_program_others_can_change_is_refused() {
    let dir = temp_dir("hsd-writable");
    for (mode, why) in [(0o775, "other users"), (0o757, "other users")] {
        let program = dir.join(format!("hsd-{mode:o}"));
        write_script(&program, "#!/bin/sh\n", mode);
        let e = validate_hsd_path(program.to_str().unwrap()).expect_err("writable");
        assert!(
            matches!(&e, AppError::InvalidInput(m) if m.contains(why)),
            "{mode:o}: {e}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(
        program_file_problem(0o755, 501, 501),
        None,
        "my own install"
    );
    assert_eq!(
        program_file_problem(0o555, 0, 501),
        None,
        "a system install"
    );
    assert_eq!(
        program_file_problem(0o755, 502, 501),
        Some("belongs to another user")
    );
    assert_eq!(
        program_file_problem(0o644, 501, 501),
        Some("is not executable")
    );
    // A real system file, as package managers install them.
    assert!(validate_hsd_path("/bin/sh").is_ok());
}

#[test]
fn a_path_that_could_look_like_another_in_the_window_is_refused() {
    let dir = temp_dir("hsd-unshowable");
    let base = dir.to_str().unwrap();
    for hidden in [
        "\u{202E}", "\n", "\u{200B}", "\u{2066}", "\u{FEFF}", "\u{7}",
    ] {
        let value = format!("{base}/a{hidden}b");
        for e in [
            validate_hsd_prefix(&value).expect_err("prefix"),
            validate_hsd_path(&value).expect_err("program"),
        ] {
            assert!(
                matches!(&e, AppError::InvalidInput(m) if m.contains("invisible")),
                "{value:?}: {e}"
            );
        }
    }
    #[cfg(unix)]
    {
        // The window shows the file a link leads to, so that name counts too.
        let target = dir.join("h\u{202E}sd");
        write_script(&target, "#!/bin/sh\n", 0o755);
        let link = dir.join("hsd-link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let e = validate_hsd_path(link.to_str().unwrap()).expect_err("link target");
        assert!(
            matches!(&e, AppError::InvalidInput(m) if m.contains("invisible")),
            "{e}"
        );
    }
    let cyrillic = format!("{base}/\u{434}\u{430}\u{43d}\u{43d}\u{44b}\u{435}");
    assert_eq!(validate_hsd_prefix(&cyrillic).unwrap(), cyrillic);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_hsd_data_directory_is_an_absolute_path_that_is_not_a_file() {
    let dir = temp_dir("hsd-prefix");
    let file = dir.join("hsd.conf");
    std::fs::write(&file, "").unwrap();

    for (value, why) in [
        ("hsd-data", "full path"),
        ("~/.hsd", "full path"),
        (file.to_str().unwrap(), "not a directory"),
    ] {
        let e = validate_hsd_prefix(value).expect_err(value);
        assert!(
            matches!(&e, AppError::InvalidInput(m) if m.contains(why)),
            "{value}: {e}"
        );
    }
    assert_eq!(
        validate_hsd_prefix(dir.to_str().unwrap()).unwrap(),
        dir.to_str().unwrap()
    );
    let later = dir.join("not-yet");
    assert_eq!(
        validate_hsd_prefix(later.to_str().unwrap()).unwrap(),
        later.to_str().unwrap(),
        "start_hsd creates a missing directory"
    );
    let e = validate_hsd_prefix("//server/share").expect_err("a share");
    assert!(
        matches!(&e, AppError::InvalidInput(m) if m.contains("network path")),
        "{e}"
    );
    assert_eq!(
        validate_hsd_prefix("").unwrap(),
        "",
        "empty = hsd's default"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A Windows network path would let a remote share supply the program or
/// its data directory. Checked on the text, so every platform runs this.
#[test]
fn a_windows_network_path_is_refused() {
    for bad in [
        r"\\server\share\hsd.exe",
        r"\\?\UNC\server\share\hsd.exe",
        r"\\?\unc\server\share\hsd.exe",
        "//server/share/hsd.exe",
        r"/\server\share\hsd.exe",
        "//?/UNC/server/share/hsd.exe",
    ] {
        let e = refuse_network_path(bad).expect_err(bad);
        assert!(
            matches!(&e, AppError::InvalidInput(m) if m.contains("network path")),
            "{bad}: {e}"
        );
    }
    for ok in [
        r"\\?\C:\hsd\hsd.exe",
        r"C:\hsd\hsd.exe",
        "/usr/local/bin/hsd",
    ] {
        assert!(refuse_network_path(ok).is_ok(), "{ok}");
    }
}

// --- the write ---------------------------------------------------------------

/// What the user confirmed is the change from the values shown; if those
/// change while the window is open, nothing is written.
#[tokio::test]
async fn a_change_under_the_open_window_writes_nothing() {
    let app = app_with(migrated_conn());
    let handle = app.handle().clone();

    answer(true);
    on_next_test_answer(move || {
        let state = handle.state::<AppState>();
        let conn = state.db.lock().unwrap();
        db::queries::set_setting(&conn, "node_rpc_url", "https://other.example").unwrap();
    });
    let err = save(&app, &[("node_rpc_url", "https://node.example.com")])
        .await
        .unwrap_err();

    assert!(
        matches!(&err, AppError::InvalidInput(m) if m.contains("changed while")),
        "{err}"
    );
    assert_eq!(
        raw_setting(&app, "node_rpc_url").as_deref(),
        Some("https://other.example")
    );
    assert!(audit_details(&app).await.is_empty());
}

/// The changes are written in one transaction: a write that fails part-way
/// leaves every setting as it was.
#[tokio::test]
async fn a_write_that_fails_part_way_leaves_every_setting_as_it_was() {
    let conn = migrated_conn();
    conn.execute_batch(
        "CREATE TRIGGER refuse_fallback BEFORE INSERT ON settings
         WHEN NEW.key = 'explorer_fallback_url'
         BEGIN SELECT RAISE(ABORT, 'refused by the test'); END;",
    )
    .unwrap();
    let app = app_with(conn);

    answer(true);
    let err = save(
        &app,
        &[
            ("node_rpc_url", "https://node.example.com"),
            ("explorer_fallback_url", "https://fallback.example"),
        ],
    )
    .await
    .unwrap_err();

    assert!(matches!(err, AppError::Db(_)), "{err}");
    assert_eq!(
        raw_setting(&app, "node_rpc_url").as_deref(),
        Some("http://127.0.0.1:12037")
    );
    assert!(audit_details(&app).await.is_empty());
}

// --- the hsd program the renderer once chose (node-02) ----------------------

/// A data directory of this test and a stub "hsd" that records every run in
/// `ran` before it answers `--version`.
#[cfg(unix)]
fn marker_hsd(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
    let dir = temp_dir(tag);
    let program = dir.join("hsd");
    let marker = dir.join("ran");
    write_script(
        &program,
        &format!(
            "#!/bin/sh\necho \"$@\" >> '{}'\necho 8.0.0\n",
            marker.to_str().unwrap()
        ),
        0o755,
    );
    (dir, program, marker)
}

/// A node that cannot answer and a data directory of the test's own, so
/// `node_status` touches no real node.
fn status_conn(dir: &Path) -> rusqlite::Connection {
    let conn = migrated_conn();
    db::queries::set_setting(&conn, "node_rpc_url", "http://127.0.0.1:1").unwrap();
    db::queries::set_setting(&conn, "hsd_prefix", dir.to_str().unwrap()).unwrap();
    conn
}

async fn node_status_of(app: &App) -> serde_json::Value {
    crate::commands::node::node_status(app.state())
        .await
        .unwrap()
}

/// The renderer's old route: write `hsd_path`, and the 3-second
/// `node_status` poll runs `<hsd_path> --version`. The write is refused, so
/// the poll runs no program the renderer named.
#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(hsd_home)]
async fn node_status_runs_no_program_the_renderer_named() {
    let (dir, program, marker) = marker_hsd("renderer-named");
    let app = app_with(status_conn(&dir));

    let err = update_setting(
        app.state(),
        "hsd_path".to_string(),
        program.to_str().unwrap().to_string(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, AppError::InvalidInput(_)), "{err}");
    assert_eq!(raw_setting(&app, "hsd_path"), None);

    let status = node_status_of(&app).await;
    assert_ne!(status["binary"], program.to_str().unwrap());
    assert!(!marker.exists(), "the renderer's program must not run");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(hsd_home)]
async fn node_status_runs_no_program_whose_confirmation_was_declined() {
    let (dir, program, marker) = marker_hsd("declined");
    let app = app_with(status_conn(&dir));

    answer(false);
    let err = save(&app, &[("hsd_path", program.to_str().unwrap())])
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::UserRejected), "{err}");
    assert_eq!(raw_setting(&app, "hsd_path"), None);

    node_status_of(&app).await;
    assert!(!marker.exists(), "a declined program must not run");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The positive control for the two tests above: the stub does record a run,
/// and the program the user confirmed is the one `node_status` runs.
#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(hsd_home)]
async fn node_status_runs_the_program_the_user_confirmed() {
    let (dir, program, marker) = marker_hsd("confirmed");
    let app = app_with(status_conn(&dir));

    answer(true);
    save(&app, &[("hsd_path", program.to_str().unwrap())])
        .await
        .unwrap();

    let status = node_status_of(&app).await;
    assert_eq!(status["binary"], program.to_str().unwrap());
    assert_eq!(status["binary_found"], true);
    assert_eq!(status["version"], "8.0.0");
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap().trim(),
        "--version"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A stored path that breaks the rule it would be confirmed under (here a
/// bare name, which `Command` would look up on PATH) is refused at use: not
/// run, reported as not found, with the reason.
#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(hsd_home)]
async fn node_status_refuses_a_stored_program_that_breaks_the_rule() {
    let dir = temp_dir("stored-bare");
    let conn = status_conn(&dir);
    db::queries::set_setting(&conn, "hsd_path", "sh").unwrap();
    let app = app_with(conn);

    let status = node_status_of(&app).await;
    assert_eq!(status["binary"], "sh");
    assert_eq!(status["binary_found"], false);
    assert!(status["version"].is_null());
    assert!(
        status["last_error"].as_str().unwrap().contains("full path"),
        "{status}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(hsd_home)]
async fn start_hsd_refuses_a_stored_program_that_breaks_the_rule() {
    let (dir, _program, marker) = marker_hsd("start-refused");
    let conn = status_conn(&dir);
    // A file without an execute bit: refused by the rule, so never spawned.
    let plain = dir.join("plain-hsd");
    write_script(
        &plain,
        &format!("#!/bin/sh\necho \"$@\" >> '{}'\n", marker.to_str().unwrap()),
        0o644,
    );
    db::queries::set_setting(&conn, "hsd_path", plain.to_str().unwrap()).unwrap();
    db::queries::insert_wallet_profile(
        &conn,
        "p1",
        "Primary",
        "watch_only_xpub",
        "mainnet",
        "xpub_placeholder",
        0,
        true,
    )
    .unwrap();
    db::queries::set_active_profile(&conn, "p1").unwrap();
    let app = app_with(conn);

    let err = crate::commands::node::start_hsd(app.state())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not executable"), "{err}");
    assert!(app
        .state::<AppState>()
        .hsd_children
        .lock()
        .unwrap()
        .is_empty());
    assert!(!marker.exists());
    let _ = std::fs::remove_dir_all(&dir);
}
