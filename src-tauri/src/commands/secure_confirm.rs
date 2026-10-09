//! Asking the secure window to confirm rows, for command modules that may
//! not name the legacy `secure_prompt` layer themselves (CODING_STANDARDS
//! §Rust layering: a small dedicated helper module). The window shows the
//! rows verbatim, out of reach of the main webview.

use tauri::{AppHandle, Runtime};

use crate::commands::secure_prompt::{prompt_secure, SecurePromptRequest};
use crate::error::AppError;

/// Show `details` (`{ "rows": [...] }`) under `title` and `message` in a
/// `confirm` prompt; `UserRejected` unless the user confirms.
pub(crate) async fn confirm_rows<R: Runtime>(
    app: &AppHandle<R>,
    title: &str,
    message: &str,
    details: serde_json::Value,
) -> Result<(), AppError> {
    let answer = prompt_secure(
        app,
        SecurePromptRequest {
            mode: "confirm".into(),
            title: title.into(),
            message: message.into(),
            details: Some(details),
            ..Default::default()
        },
    )
    .await?;
    if !answer.confirmed {
        return Err(AppError::UserRejected);
    }
    Ok(())
}
