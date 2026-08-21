use crate::commands::secure_wallet;

#[test]
fn test_validate_network_mainnet() {
    let (s, n) = secure_wallet::validate_network("mainnet").unwrap();
    assert_eq!(s, "mainnet");
    assert_eq!(n, crate::noncustodial::network::Network::Main);
}

#[test]
fn test_validate_network_testnet() {
    let (s, n) = secure_wallet::validate_network("testnet").unwrap();
    assert_eq!(s, "testnet");
    assert_eq!(n, crate::noncustodial::network::Network::Testnet);
}

#[test]
fn test_validate_network_regtest() {
    let (s, n) = secure_wallet::validate_network("regtest").unwrap();
    assert_eq!(s, "regtest");
    assert_eq!(n, crate::noncustodial::network::Network::Regtest);
}

#[test]
fn test_validate_network_rejects_unknown() {
    let err = secure_wallet::validate_network("invalid").unwrap_err();
    assert!(err.to_string().contains("unsupported network"));
}

#[test]
fn test_fingerprint_is_deterministic() {
    let fp1 = secure_wallet::fingerprint("xpub6AHA9hZDN11k2ijHMeS5QqHx2KP9aMBRhTDqANMnwVtdG2Jj4e3uJ4f9Nx8c5K9fQ8K9fQ8K9fQ8K9fQ8K9fQ8K9fQ8K9fQ8K9fQ8K9fQ8K9fQ");
    let fp2 = secure_wallet::fingerprint("xpub6AHA9hZDN11k2ijHMeS5QqHx2KP9aMBRhTDqANMnwVtdG2Jj4e3uJ4f9Nx8c5K9fQ8K9fQ8K9fQ8K9fQ8K9fQ8K9fQ8K9fQ8K9fQ8K9fQ8K9fQ");
    assert_eq!(fp1, fp2);
    assert_eq!(fp1.len(), 16); // 8 bytes = 16 hex chars
}

#[test]
fn test_fingerprint_differs_for_different_xpubs() {
    let fp1 = secure_wallet::fingerprint("xpub6AHA9hZDN11k2ijHMeS5QqHx2KP9aMBRhTDqANMnwVtdG2Jj4e3uJ4f9Nx8c5K9fQ8K9fQ8K9fQ8K9fQ8K9fQ8K9fQ8K9fQ8K9fQ8K9fQ8K9fQ");
    let fp2 = secure_wallet::fingerprint("xpub6BHA9hZDN11k2ijHMeS5QqHx2KP9aMBRhTDqANMnwVtdG2Jj4e3uJ4f9Nx8c5K9fQ8K9fQ8K9fQ8K9fQ8K9fQ8K9fQ8K9fQ8K9fQ8K9fQ8K9fQ");
    assert_ne!(fp1, fp2);
}

#[test]
fn test_gap_limit_defaults_to_20() {
    let settings = std::collections::HashMap::new();
    assert_eq!(secure_wallet::gap_limit(&settings), 20);
}

#[test]
fn test_gap_limit_from_settings() {
    let mut settings = std::collections::HashMap::new();
    settings.insert("address_gap_limit".to_string(), "50".to_string());
    assert_eq!(secure_wallet::gap_limit(&settings), 50);
}

#[test]
fn test_gap_limit_rejects_zero() {
    let mut settings = std::collections::HashMap::new();
    settings.insert("address_gap_limit".to_string(), "0".to_string());
    assert_eq!(secure_wallet::gap_limit(&settings), 20);
}

#[test]
fn test_gap_limit_rejects_negative() {
    let mut settings = std::collections::HashMap::new();
    settings.insert("address_gap_limit".to_string(), "-5".to_string());
    assert_eq!(secure_wallet::gap_limit(&settings), 20);
}

#[test]
fn test_gap_limit_rejects_non_numeric() {
    let mut settings = std::collections::HashMap::new();
    settings.insert("address_gap_limit".to_string(), "abc".to_string());
    assert_eq!(secure_wallet::gap_limit(&settings), 20);
}

#[test]
fn test_session_ttl_ms_defaults_to_900_seconds() {
    let settings = std::collections::HashMap::new();
    assert_eq!(secure_wallet::session_ttl_ms(&settings), 900_000);
}

#[test]
fn test_session_ttl_ms_from_settings() {
    let mut settings = std::collections::HashMap::new();
    settings.insert(
        "signer_session_timeout_seconds".to_string(),
        "3600".to_string(),
    );
    assert_eq!(secure_wallet::session_ttl_ms(&settings), 3_600_000);
}

#[test]
fn test_session_ttl_ms_rejects_zero() {
    let mut settings = std::collections::HashMap::new();
    settings.insert(
        "signer_session_timeout_seconds".to_string(),
        "0".to_string(),
    );
    assert_eq!(secure_wallet::session_ttl_ms(&settings), 900_000);
}

#[test]
fn test_resolve_secret_key_empty_uses_device_key() {
    let (key, kdf) = secure_wallet::resolve_secret_key("");
    assert_eq!(kdf, "none");
    assert_eq!(key, "namehold::no-passphrase::v1");
}

#[test]
fn test_resolve_secret_key_non_empty_uses_argon2id() {
    let (key, kdf) = secure_wallet::resolve_secret_key("my-passphrase");
    assert_eq!(kdf, "argon2id");
    assert_eq!(key, "my-passphrase");
}

#[test]
fn test_random_id_is_32_hex_chars() {
    let id = secure_wallet::random_id();
    assert_eq!(id.len(), 32);
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_random_id_is_unique() {
    let a = secure_wallet::random_id();
    let b = secure_wallet::random_id();
    assert_ne!(a, b);
}
