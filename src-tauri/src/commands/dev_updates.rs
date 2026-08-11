//! Dev-only helper for driving the in-app update UI without a real release.
//!
//! The real updater plugin verifies Ed25519-signed artifacts fetched from
//! GitHub Releases; that flow can't be spoofed client-side, and dev builds
//! aren't signed. To let developers still see the "update available"
//! experience (banner + Settings card), this module exposes a single command
//! that returns the metadata of the last GitHub release for the project.
//! The frontend then seeds `useAppUpdate` with that metadata and simulates
//! the download+install phases visually.
//!
//! The command is gated with `#[cfg(all(debug_assertions, not(test)))]` and
//! only registered in dev builds (see `lib.rs`), so it does not exist at all
//! in release binaries. The fetch happens Rust-side because the app's CSP
//! doesn't allow direct calls to `api.github.com` from the webview.

#![cfg(all(debug_assertions, not(test)))]

use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// Subset of the GitHub Releases API response we care about.
#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    body: Option<String>,
    published_at: Option<String>,
}

/// Wire shape returned to the frontend. Camel-cased to match the shape the
/// TS side expects for `UpdateMetadata`-adjacent data.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevReleaseMeta {
    pub version: String,
    pub notes: Option<String>,
    pub date: Option<String>,
}

/// Fetch the latest GitHub release for the project so the dev simulate button
/// can seed realistic version + notes. Errors flow through `AppError::Http`
/// so the frontend can fall back to a synthetic version when offline.
#[tauri::command]
pub async fn fetch_latest_release_meta() -> Result<DevReleaseMeta, AppError> {
    // GitHub requires a User-Agent, otherwise it returns 403. The `Accept`
    // header pins us to the v3 JSON shape we deserialize below.
    let client = reqwest::Client::builder()
        .user_agent("namehold-wallet-dev")
        .build()?;

    let release: GhRelease = client
        .get("https://api.github.com/repos/DimazzzZ/namehold-wallet/releases/latest")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    // Release tags conventionally carry a leading "v" (e.g. "v0.5.0"); strip
    // it so the version compares cleanly against `current_version`, which is
    // returned bare.
    let version = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name)
        .to_string();

    Ok(DevReleaseMeta {
        version,
        notes: release.body,
        date: release.published_at,
    })
}
