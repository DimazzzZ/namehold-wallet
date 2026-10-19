//! The main window's capability set, run through Tauri's real ACL.
//!
//! `mock_builder` + the app's own `app_context()` resolve the same
//! capability files the shipped app does (`capabilities/*.json`), and
//! `get_ipc_response` sends a command as the `main` window's webview would.
//! The command unit tests call commands as plain functions and mock the
//! frontend plugins, so neither sees a permission refusal.

use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{get_ipc_response, mock_builder, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{WebviewUrl, WebviewWindowBuilder};

fn main_window_call(cmd: &str, body: serde_json::Value) -> Result<serde_json::Value, String> {
    let app = mock_builder()
        .build(crate::app_context())
        .expect("mock app with the real capabilities");
    let window = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
        .build()
        .expect("main window");
    get_ipc_response(
        &window,
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(body),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .map(|b| b.deserialize::<serde_json::Value>().unwrap())
    .map_err(|e| e.to_string())
}

/// The harness enforces the ACL: a permission the window has goes through.
#[test]
fn main_window_may_call_a_granted_plugin_command() {
    let version = main_window_call("plugin:app|version", serde_json::json!({}));
    assert!(version.is_ok(), "granted by core:default: {version:?}");
}

/// The bid backup holds secret nonces, so the main window must not be able to
/// write files itself: it goes through `save_bid_backup`. The app ships no fs
/// plugin, so the command is refused as an unknown plugin. Granting an `fs:`
/// permission in `capabilities/*.json` without adding the plugin back does not
/// compile (tauri-build rejects an unknown permission), so no capability file
/// can open this route by itself.
#[test]
fn main_window_has_no_fs_write_permission() {
    let target = std::env::temp_dir().join(format!("namehold_acl_{}.txt", std::process::id()));
    let refused = main_window_call(
        "plugin:fs|write_text_file",
        serde_json::json!({ "path": target.to_string_lossy(), "data": "x" }),
    );
    let why = refused.expect_err("the fs plugin must be refused for the main window");
    assert!(
        why.contains("not allowed"),
        "ACL refusal expected, got: {why}"
    );
    assert!(!target.exists());
}
