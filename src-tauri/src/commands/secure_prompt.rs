//! Rust-owned secure prompt surface.
//!
//! This module implements the "secret never touches React" rule. Whenever the
//! backend needs to ingest a secret (a passphrase to unlock/encrypt) or egress
//! one (display a freshly-generated or decrypted mnemonic), it does so through a
//! dedicated, Rust-controlled `secure.html` webview window — NOT the React app.
//!
//! Lifecycle of a single prompt:
//!   1. A backend flow calls [`prompt_secure`] with a [`SecurePromptRequest`].
//!   2. We register the request in [`AppState::secure_prompts`] keyed by a random
//!      id, open a `secure-prompt-<id>` window pointed at `secure.html`, and
//!      `await` a oneshot channel.
//!   3. The secure window calls [`secure_prompt_fetch`] to read its request, then
//!      [`secure_prompt_submit`] with the user's answer, which fulfils the oneshot.
//!   4. The originating flow receives the [`SecurePromptResult`] and the window is
//!      closed by the backend.
//!
//! The secret value (passphrase or mnemonic) only ever flows window <-> backend.
//! It is never the return value of a React-invoked command.

use std::collections::HashMap;
use std::sync::Mutex;

use rand::RngCore;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::oneshot;

use crate::error::AppError;
use crate::AppState;

/// A request shown by the secure window. `payload` carries display-only secret
/// material (e.g. a mnemonic to reveal); it is sent window-ward only.
#[derive(Clone, Serialize)]
pub struct SecurePromptRequest {
    /// One of: `passphrase`, `passphrase_new`, `reveal`, `import`.
    pub mode: String,
    pub title: String,
    pub message: String,
    /// For `reveal`: the mnemonic to display. `None` otherwise.
    pub payload: Option<String>,
}

/// The user's answer to a secure prompt.
#[derive(Clone, Deserialize)]
pub struct SecurePromptResult {
    /// Entered secret (passphrase for `passphrase*`, mnemonic for `import`).
    pub value: Option<String>,
    /// `true` when the user confirmed; `false` on cancel / window close.
    pub confirmed: bool,
}

/// A prompt awaiting an answer. The `responder` is taken exactly once.
pub struct PendingPrompt {
    request: SecurePromptRequest,
    responder: Option<oneshot::Sender<SecurePromptResult>>,
}

/// Registry of in-flight prompts, stored on [`AppState`].
pub type SecurePromptRegistry = Mutex<HashMap<String, PendingPrompt>>;

fn random_id() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Open a secure window for `request` and block until the user answers (or
/// closes the window, which resolves to a non-confirmed result).
///
/// This MUST NOT be called while holding the `AppState::db` lock, since it
/// awaits user interaction.
pub async fn prompt_secure(
    app: &AppHandle,
    request: SecurePromptRequest,
) -> Result<SecurePromptResult, AppError> {
    let id = random_id();
    let (tx, rx) = oneshot::channel();

    {
        let state = app
            .try_state::<AppState>()
            .ok_or_else(|| AppError::Other("app state unavailable".into()))?;
        let mut map = state
            .secure_prompts
            .lock()
            .map_err(|e| AppError::Lock(e.to_string()))?;
        map.insert(
            id.clone(),
            PendingPrompt {
                request,
                responder: Some(tx),
            },
        );
    }

    let label = format!("secure-prompt-{id}");
    let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("secure.html".into()))
        .title("Namehold — Secure")
        .inner_size(480.0, 440.0)
        .resizable(false)
        .focused(true)
        .always_on_top(true)
        .build()
        .map_err(|e| AppError::Other(format!("failed to open secure window: {e}")))?;

    // If the user closes the window before submitting, resolve as cancelled so
    // the awaiting flow does not hang forever.
    {
        let app = app.clone();
        let id = id.clone();
        window.on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                if let Some(state) = app.try_state::<AppState>() {
                    if let Ok(mut map) = state.secure_prompts.lock() {
                        if let Some(mut pending) = map.remove(&id) {
                            if let Some(resp) = pending.responder.take() {
                                let _ = resp.send(SecurePromptResult {
                                    value: None,
                                    confirmed: false,
                                });
                            }
                        }
                    }
                }
            }
        });
    }

    let result = rx
        .await
        .map_err(|_| AppError::Other("secure prompt was cancelled".into()))?;

    // Close the window and clean up the registry regardless of outcome.
    let _ = window.close();
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut map) = state.secure_prompts.lock() {
            map.remove(&id);
        }
    }

    Ok(result)
}

/// Reject any caller whose window is not the exact secure window that owns this
/// prompt. This bars the React window (and any other secure window) from reading
/// a `reveal` payload (the mnemonic) or answering on another window's behalf.
/// App commands are not ACL-gated in Tauri v2, so this in-command check is the
/// real enforcement boundary.
fn assert_owning_window(window: &tauri::WebviewWindow, prompt_id: &str) -> Result<(), AppError> {
    if window.label() != format!("secure-prompt-{prompt_id}") {
        return Err(AppError::Other("forbidden".into()));
    }
    Ok(())
}

/// Served to the secure window only: returns the request it should render.
#[tauri::command]
pub async fn secure_prompt_fetch(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    prompt_id: String,
) -> Result<SecurePromptRequest, AppError> {
    assert_owning_window(&window, &prompt_id)?;
    let map = state
        .secure_prompts
        .lock()
        .map_err(|e| AppError::Lock(e.to_string()))?;
    map.get(&prompt_id)
        .map(|p| p.request.clone())
        .ok_or_else(|| AppError::NotFound("secure prompt".into()))
}

/// Served to the secure window only: delivers the user's answer.
#[tauri::command]
pub async fn secure_prompt_submit(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    prompt_id: String,
    result: SecurePromptResult,
) -> Result<(), AppError> {
    assert_owning_window(&window, &prompt_id)?;
    let mut map = state
        .secure_prompts
        .lock()
        .map_err(|e| AppError::Lock(e.to_string()))?;
    let pending = map
        .get_mut(&prompt_id)
        .ok_or_else(|| AppError::NotFound("secure prompt".into()))?;
    let responder = pending
        .responder
        .take()
        .ok_or_else(|| AppError::Other("secure prompt already answered".into()))?;
    responder
        .send(result)
        .map_err(|_| AppError::Other("secure prompt receiver dropped".into()))?;
    Ok(())
}
