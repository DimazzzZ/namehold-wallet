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

/// The fs route is unreachable: the app registers no fs plugin, so the command
/// is refused as an unknown plugin. This alone does not guard against the
/// plugin coming back with a grant; see `no_fs_plugin_and_no_fs_permission`.
#[test]
fn fs_write_route_is_unreachable_without_the_plugin() {
    let target = std::env::temp_dir().join(format!("namehold_acl_{}.txt", std::process::id()));
    let refused = main_window_call(
        "plugin:fs|write_text_file",
        serde_json::json!({ "path": target.to_string_lossy(), "data": "x" }),
    );
    let why = refused.expect_err("the fs plugin must be refused for the main window");
    assert!(
        why.contains("Plugin not found"),
        "unknown-plugin refusal expected, got: {why}"
    );
    assert!(!target.exists());
}

/// Permission identifiers a capability file lists that belong to the fs plugin.
/// An entry is a string or an object with an `identifier`.
fn fs_permissions(capability_json: &str) -> Vec<String> {
    let doc: serde_json::Value = serde_json::from_str(capability_json).expect("capability JSON");
    doc["permissions"]
        .as_array()
        .expect("permissions array")
        .iter()
        .filter_map(|p| p.as_str().or_else(|| p["identifier"].as_str()))
        .filter(|id| id.starts_with("fs:"))
        .map(str::to_string)
        .collect()
}

/// The bid backup holds secret nonces, so the main window must not write files
/// itself: it goes through `save_bid_backup`. No capability grants an `fs:`
/// permission, the manifest has no direct tauri-plugin-fs dependency and the
/// app never registers the plugin. The refusal test above rests on the plugin
/// being absent, so this is the guard against it coming back.
#[test]
fn no_fs_plugin_and_no_fs_permission() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut seen = 0;
    for entry in std::fs::read_dir(root.join("capabilities")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "json") {
            seen += 1;
            let granted = fs_permissions(&std::fs::read_to_string(&path).unwrap());
            assert!(granted.is_empty(), "{path:?} grants {granted:?}");
        }
    }
    assert!(seen >= 2, "capability files not found");
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(
        !manifest.contains("tauri-plugin-fs"),
        "Cargo.toml depends on tauri-plugin-fs"
    );
    let lib = std::fs::read_to_string(root.join("src/lib.rs")).unwrap();
    assert!(
        !lib.contains("tauri_plugin_fs"),
        "lib.rs registers the fs plugin"
    );
}

#[test]
fn fs_permission_check_sees_string_and_object_grants() {
    let json = r#"{"permissions": ["core:default", "fs:allow-write-text-file",
        {"identifier": "fs:allow-write-file", "allow": []}, {"identifier": "dialog:default"}]}"#;
    assert_eq!(
        fs_permissions(json),
        vec!["fs:allow-write-text-file", "fs:allow-write-file"]
    );
    assert!(fs_permissions(r#"{"permissions": ["core:default"]}"#).is_empty());
}
