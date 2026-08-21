use crate::error::AppError;

#[test]
fn test_app_error_display_and_serialize() {
    let err = AppError::Lock("poisoned".to_string());
    assert_eq!(err.to_string(), "Lock error: poisoned");
    let json = serde_json::to_value(&err).unwrap();
    assert_eq!(json, serde_json::json!("Lock error: poisoned"));

    let err = AppError::NotFound("asset 42".to_string());
    assert_eq!(err.to_string(), "Not found: asset 42");
    let json = serde_json::to_value(&err).unwrap();
    assert_eq!(json, serde_json::json!("Not found: asset 42"));

    let err = AppError::InvalidInput("negative amount".to_string());
    assert_eq!(err.to_string(), "Invalid input: negative amount");
    let json = serde_json::to_value(&err).unwrap();
    assert_eq!(json, serde_json::json!("Invalid input: negative amount"));

    let err = AppError::WalletLocked;
    assert_eq!(err.to_string(), "Wallet locked");
    let json = serde_json::to_value(&err).unwrap();
    assert_eq!(json, serde_json::json!("Wallet locked"));

    let err = AppError::Other("something went wrong".to_string());
    assert_eq!(err.to_string(), "something went wrong");
    let json = serde_json::to_value(&err).unwrap();
    assert_eq!(json, serde_json::json!("something went wrong"));

    let err = AppError::Crypto("decryption failed".to_string());
    assert_eq!(err.to_string(), "Crypto error: decryption failed");
    let json = serde_json::to_value(&err).unwrap();
    assert_eq!(json, serde_json::json!("Crypto error: decryption failed"));

    let err = AppError::Rpc("connection refused".to_string());
    assert_eq!(err.to_string(), "Node RPC error: connection refused");
    let json = serde_json::to_value(&err).unwrap();
    assert_eq!(
        json,
        serde_json::json!("Node RPC error: connection refused")
    );
}

#[test]
fn test_app_error_db_variant() {
    let err = AppError::Db(rusqlite::Error::InvalidParameterName("foo".into()));
    assert!(err.to_string().contains("Database error:"));
    let json = serde_json::to_value(&err).unwrap();
    assert!(json.as_str().unwrap().contains("Database error:"));
}

#[test]
fn test_app_error_json_variant() {
    let err = AppError::Json(serde_json::from_str::<()>("invalid").unwrap_err());
    assert!(err.to_string().contains("JSON error:"));
    let json = serde_json::to_value(&err).unwrap();
    assert!(json.as_str().unwrap().contains("JSON error:"));
}

#[test]
fn test_app_error_io_variant() {
    let err = AppError::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "denied",
    ));
    assert!(err.to_string().contains("IO error:"));
    let json = serde_json::to_value(&err).unwrap();
    assert!(json.as_str().unwrap().contains("IO error:"));
}
