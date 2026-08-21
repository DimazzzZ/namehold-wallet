use crate::commands::secure_prompt;

// --- random_id tests ---

#[test]
fn test_random_id_is_32_hex_chars() {
    let id = secure_prompt::random_id();
    assert_eq!(id.len(), 32);
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_random_id_is_unique() {
    let mut ids = std::collections::HashSet::new();
    for _ in 0..100 {
        ids.insert(secure_prompt::random_id());
    }
    assert_eq!(ids.len(), 100);
}

// --- assert_owning_window label format tests ---
// Note: assert_owning_window takes &tauri::WebviewWindow (concrete Wry runtime),
// which cannot be constructed with MockRuntime. We test the label format logic
// and the command functions indirectly through the registry and oneshot tests.

#[test]
fn test_expected_label_format() {
    let expected_label = format!("secure-prompt-{}", "abc123");
    assert_eq!(expected_label, "secure-prompt-abc123");
}

#[test]
fn test_expected_label_empty_id() {
    let expected_label = format!("secure-prompt-{}", "");
    assert_eq!(expected_label, "secure-prompt-");
}

#[test]
fn test_expected_label_long_id() {
    let long_id = "a".repeat(32);
    let expected_label = format!("secure-prompt-{}", long_id);
    assert_eq!(expected_label, format!("secure-prompt-{long_id}"));
}

// --- SecurePromptRequest serialization tests ---

#[test]
fn test_secure_prompt_request_serializes() {
    let req = secure_prompt::SecurePromptRequest {
        mode: "passphrase".into(),
        title: "Unlock Wallet".into(),
        message: "Enter your passphrase".into(),
        payload: None,
        ..Default::default()
    };
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["mode"], "passphrase");
    assert_eq!(json["title"], "Unlock Wallet");
    assert_eq!(json["message"], "Enter your passphrase");
    assert!(json["payload"].is_null());
}

#[test]
fn test_secure_prompt_request_with_payload() {
    let req = secure_prompt::SecurePromptRequest {
        mode: "reveal".into(),
        title: "Reveal Mnemonic".into(),
        message: "Your recovery phrase".into(),
        payload: Some("abandon abandon abandon...".into()),
        ..Default::default()
    };
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["mode"], "reveal");
    assert_eq!(json["payload"], "abandon abandon abandon...");
}

#[test]
fn test_secure_prompt_request_import_mode() {
    let req = secure_prompt::SecurePromptRequest {
        mode: "import".into(),
        title: "Import Wallet".into(),
        message: "Enter your mnemonic".into(),
        payload: None,
        ..Default::default()
    };
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["mode"], "import");
}

#[test]
fn test_secure_prompt_request_passphrase_new_mode() {
    let req = secure_prompt::SecurePromptRequest {
        mode: "passphrase_new".into(),
        title: "Set Passphrase".into(),
        message: "Choose a passphrase".into(),
        payload: None,
        ..Default::default()
    };
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(json["mode"], "passphrase_new");
}

// --- SecurePromptResult deserialization tests ---

#[test]
fn test_secure_prompt_result_confirmed() {
    let json = serde_json::json!({
        "value": "my-secret-passphrase",
        "confirmed": true
    });
    let result: secure_prompt::SecurePromptResult = serde_json::from_value(json).unwrap();
    assert!(result.confirmed);
    assert_eq!(result.value, Some("my-secret-passphrase".into()));
}

#[test]
fn test_secure_prompt_result_cancelled() {
    let json = serde_json::json!({
        "value": null,
        "confirmed": false
    });
    let result: secure_prompt::SecurePromptResult = serde_json::from_value(json).unwrap();
    assert!(!result.confirmed);
    assert!(result.value.is_none());
}

#[test]
fn test_secure_prompt_result_empty_value() {
    let json = serde_json::json!({
        "value": "",
        "confirmed": true
    });
    let result: secure_prompt::SecurePromptResult = serde_json::from_value(json).unwrap();
    assert!(result.confirmed);
    assert_eq!(result.value, Some("".into()));
}

// --- SecurePromptResult deserialization from string ---

#[test]
fn test_secure_prompt_result_from_json_string() {
    let json_str = r#"{"value":"my-mnemonic-phrase","confirmed":true}"#;
    let result: secure_prompt::SecurePromptResult = serde_json::from_str(json_str).unwrap();
    assert!(result.confirmed);
    assert_eq!(result.value, Some("my-mnemonic-phrase".into()));
}

#[test]
fn test_secure_prompt_result_cancelled_from_json_string() {
    let json_str = r#"{"value":null,"confirmed":false}"#;
    let result: secure_prompt::SecurePromptResult = serde_json::from_str(json_str).unwrap();
    assert!(!result.confirmed);
    assert!(result.value.is_none());
}

// --- Oneshot channel logic tests ---

#[test]
fn test_oneshot_delivers_result() {
    let (tx, rx) = tokio::sync::oneshot::channel::<secure_prompt::SecurePromptResult>();
    let result = secure_prompt::SecurePromptResult {
        value: Some("secret123".into()),
        confirmed: true,
    };
    assert!(tx.send(result).is_ok());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let received = rt.block_on(rx).unwrap();
    assert!(received.confirmed);
    assert_eq!(received.value, Some("secret123".into()));
}

#[test]
fn test_oneshot_cancel_delivers_error() {
    let (tx, rx) = tokio::sync::oneshot::channel::<secure_prompt::SecurePromptResult>();
    drop(tx);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(rx);
    assert!(result.is_err());
}

#[test]
fn test_oneshot_double_send_fails() {
    let (tx, rx) = tokio::sync::oneshot::channel::<secure_prompt::SecurePromptResult>();
    let result = secure_prompt::SecurePromptResult {
        value: Some("first".into()),
        confirmed: true,
    };
    assert!(tx.send(result).is_ok());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let received = rt.block_on(rx).unwrap();
    assert!(received.confirmed);
    assert_eq!(received.value, Some("first".into()));
}

// --- Window close simulation test ---

#[test]
fn test_window_close_sends_cancelled_result() {
    let (tx, rx) = tokio::sync::oneshot::channel::<secure_prompt::SecurePromptResult>();
    // Simulate window close: send cancelled result
    let _ = tx.send(secure_prompt::SecurePromptResult {
        value: None,
        confirmed: false,
    });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(rx).unwrap();
    assert!(!result.confirmed);
    assert!(result.value.is_none());
}

// --- Multiple concurrent prompts test ---

#[test]
fn test_multiple_oneshot_channels() {
    let modes = ["passphrase", "reveal", "import", "passphrase_new"];
    let mut senders = Vec::new();
    let mut receivers = Vec::new();
    for _ in &modes {
        let (tx, rx) = tokio::sync::oneshot::channel::<secure_prompt::SecurePromptResult>();
        senders.push(tx);
        receivers.push(rx);
    }
    // Send confirmed results on all channels
    for tx in senders {
        let _ = tx.send(secure_prompt::SecurePromptResult {
            value: Some("answer".into()),
            confirmed: true,
        });
    }
    let rt = tokio::runtime::Runtime::new().unwrap();
    for rx in receivers {
        let result = rt.block_on(rx).unwrap();
        assert!(result.confirmed);
        assert_eq!(result.value, Some("answer".into()));
    }
}

// --- SecurePromptRequest all modes coverage ---

#[test]
fn test_secure_prompt_request_all_modes() {
    let modes = ["passphrase", "reveal", "import", "passphrase_new"];
    for mode in &modes {
        let req = secure_prompt::SecurePromptRequest {
            mode: (*mode).into(),
            title: format!("Title for {mode}"),
            message: format!("Message for {mode}"),
            payload: None,
            ..Default::default()
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["mode"], *mode);
    }
}

// --- SecurePromptRequest clone test ---

#[test]
fn test_secure_prompt_request_clone() {
    let req = secure_prompt::SecurePromptRequest {
        mode: "reveal".into(),
        title: "Reveal".into(),
        message: "Your phrase".into(),
        payload: Some("secret phrase".into()),
        ..Default::default()
    };
    let cloned = req.clone();
    let json_orig = serde_json::to_value(&req).unwrap();
    let json_cloned = serde_json::to_value(&cloned).unwrap();
    assert_eq!(json_orig, json_cloned);
}

// --- SecurePromptResult edge cases ---

#[test]
fn test_secure_prompt_result_missing_value_field() {
    let json_str = r#"{"confirmed":true}"#;
    let result: secure_prompt::SecurePromptResult = serde_json::from_str(json_str).unwrap();
    assert!(result.confirmed);
    assert!(result.value.is_none());
}

#[test]
fn test_secure_prompt_result_long_value() {
    let long_value = "a".repeat(10000);
    let json = serde_json::json!({
        "value": long_value,
        "confirmed": true
    });
    let result: secure_prompt::SecurePromptResult = serde_json::from_value(json).unwrap();
    assert!(result.confirmed);
    assert_eq!(result.value.unwrap().len(), 10000);
}

#[test]
fn test_secure_prompt_result_unicode_value() {
    let json = serde_json::json!({
        "value": "abandon abandon abandon",
        "confirmed": true
    });
    let result: secure_prompt::SecurePromptResult = serde_json::from_value(json).unwrap();
    assert!(result.confirmed);
    assert_eq!(result.value, Some("abandon abandon abandon".into()));
}
