//! Auto-update commands (desktop only).
//!
//! Backed by `tauri-plugin-updater`, which checks the static `latest.json`
//! published on the GitHub Releases page (see `tauri.conf.json` →
//! `plugins.updater.endpoints`). The update bundles are Ed25519-signed at
//! release time; the plugin verifies the signature against the `pubkey` in
//! `tauri.conf.json` before installing — this is mandatory and cannot be
//! disabled.
//!
//! Flow: the frontend calls `check_for_update` (stashes the pending `Update`
//! in managed state and returns its metadata), then `install_update` (downloads
//! + installs, streaming progress over an IPC `Channel`). Restart is left to
//! the caller (`@tauri-apps/plugin-process`'s `relaunch()` on macOS/Linux; on
//! Windows the installer exits the app automatically).
//!
//! This module is `#[cfg(desktop)]` because the updater plugin is desktop-only.

// Module doc contains prose paragraphs whose second lines clippy misreads as
// unindented markdown list continuations. Silence the lint.
#![allow(clippy::doc_lazy_continuation)]

#[cfg(desktop)]
pub mod app_updates {
    use serde::Serialize;
    use std::sync::Mutex;
    use tauri::ipc::Channel;
    use tauri::{AppHandle, State};
    use tauri_plugin_updater::{Update, UpdaterExt};

    /// Errors surfaced to the frontend. Serialized as a plain string so the JS
    /// side gets a readable message (matching how the rest of the app maps
    /// command errors).
    #[derive(Debug, thiserror::Error)]
    pub enum Error {
        #[error(transparent)]
        Updater(#[from] tauri_plugin_updater::Error),
        #[error("no pending update — call check_for_update first")]
        NoPendingUpdate,
    }

    impl Serialize for Error {
        fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            serializer.serialize_str(self.to_string().as_str())
        }
    }

    type Result<T> = std::result::Result<T, Error>;

    /// Progress events streamed to the frontend during download+install.
    #[derive(Clone, Serialize)]
    #[serde(tag = "event", content = "data")]
    pub enum DownloadEvent {
        #[serde(rename_all = "camelCase")]
        Started {
            content_length: Option<u64>,
        },
        #[serde(rename_all = "camelCase")]
        Progress {
            chunk_length: usize,
        },
        Finished,
    }

    /// Metadata about an available update, returned by `check_for_update`.
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct UpdateMetadata {
        version: String,
        current_version: String,
        notes: Option<String>,
        date: Option<String>,
    }

    /// Holds the `Update` returned by `check` until the user chooses to install
    /// it, so `install_update` doesn't have to re-check. Managed in app state.
    pub struct PendingUpdate(pub Mutex<Option<Update>>);

    /// Check the configured endpoint for a newer release. Returns `None` when
    /// the app is already up to date. On success, the pending `Update` is
    /// stashed for a subsequent `install_update` call.
    #[tauri::command]
    pub async fn check_for_update(
        app: AppHandle,
        pending: State<'_, PendingUpdate>,
    ) -> Result<Option<UpdateMetadata>> {
        let update = app.updater()?.check().await?;
        let metadata = update.as_ref().map(update_metadata);
        *pending.0.lock().unwrap() = update;
        Ok(metadata)
    }

    /// Download + install the update stashed by `check_for_update`, streaming
    /// progress over `on_event`. Errors with `NoPendingUpdate` if no update was
    /// checked first. Does NOT restart the app — the caller decides when.
    #[tauri::command]
    pub async fn install_update(
        pending: State<'_, PendingUpdate>,
        on_event: Channel<DownloadEvent>,
    ) -> Result<()> {
        let Some(update) = pending.0.lock().unwrap().take() else {
            return Err(Error::NoPendingUpdate);
        };

        let mut started = false;
        update
            .download_and_install(
                |chunk_length, content_length| {
                    if !started {
                        let _ = on_event.send(DownloadEvent::Started { content_length });
                        started = true;
                    }
                    let _ = on_event.send(DownloadEvent::Progress { chunk_length });
                },
                || {
                    let _ = on_event.send(DownloadEvent::Finished);
                },
            )
            .await?;

        Ok(())
    }

    /// The running app's own version (from the bundle), for display in the UI.
    #[tauri::command]
    pub fn current_version(app: AppHandle) -> String {
        app.package_info().version.to_string()
    }

    /// Map a plugin `Update` to the wire metadata. Split out for unit testing
    /// the field mapping without needing a live `Update` (which can't be
    /// constructed in tests).
    fn update_metadata(update: &Update) -> UpdateMetadata {
        UpdateMetadata {
            version: update.version.clone(),
            current_version: update.current_version.clone(),
            notes: update.body.clone(),
            date: update.date.map(|d| d.to_string()),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn no_pending_update_error_message_is_actionable() {
            // The frontend shows this string verbatim, so it must point the
            // user/dev at the required check_for_update call.
            let msg = Error::NoPendingUpdate.to_string();
            assert!(msg.contains("check_for_update"), "got: {msg}");
        }

        #[test]
        fn download_event_serializes_tagged() {
            // The JS Channel handler switches on `event`/`data`, so the
            // adjacently-tagged shape is part of the contract.
            let started = serde_json::to_value(DownloadEvent::Started {
                content_length: Some(1024),
            })
            .unwrap();
            assert_eq!(started["event"], "Started");
            assert_eq!(started["data"]["contentLength"], 1024);

            let progress =
                serde_json::to_value(DownloadEvent::Progress { chunk_length: 256 }).unwrap();
            assert_eq!(progress["event"], "Progress");
            assert_eq!(progress["data"]["chunkLength"], 256);

            let finished = serde_json::to_value(DownloadEvent::Finished).unwrap();
            assert_eq!(finished["event"], "Finished");
        }

        #[test]
        fn update_metadata_serializes_camel_case() {
            // Guard the wire shape the TS `UpdateMetadata` type depends on.
            let meta = UpdateMetadata {
                version: "0.5.0".into(),
                current_version: "0.4.0".into(),
                notes: Some("fixes".into()),
                date: None,
            };
            let v = serde_json::to_value(&meta).unwrap();
            assert_eq!(v["version"], "0.5.0");
            assert_eq!(v["currentVersion"], "0.4.0");
            assert_eq!(v["notes"], "fixes");
            assert!(v["date"].is_null());
        }
    }
}
