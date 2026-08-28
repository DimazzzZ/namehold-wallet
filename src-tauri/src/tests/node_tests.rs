use crate::commands::node;
use crate::noncustodial::network::Network;
use std::path::Path;

#[test]
fn test_pick_hsd_path_override_takes_precedence() {
    let candidates: Vec<String> = vec!["/nonexistent/hsd".into()];
    let result = node::pick_hsd_path(Some("/custom/hsd"), &candidates);
    assert_eq!(result, Some("/custom/hsd".to_string()));
}

#[test]
fn test_pick_hsd_path_override_empty_falls_through() {
    let result = node::pick_hsd_path(Some(""), &[]);
    assert_eq!(result, None);
}

#[test]
fn test_pick_hsd_path_override_whitespace_falls_through() {
    let result = node::pick_hsd_path(Some("  "), &[]);
    assert_eq!(result, None);
}

#[test]
fn test_pick_hsd_path_no_override_no_candidates() {
    let result = node::pick_hsd_path(None, &[]);
    assert_eq!(result, None);
}

#[test]
fn test_chain_paths_for_network_mainnet() {
    let paths = node::chain_paths_for_network("/data", Network::Main);
    assert_eq!(paths.len(), 3);
    assert!(paths.iter().all(|p| p.starts_with("/data/")));
    assert!(paths.iter().any(|p| p.ends_with("blocks")));
    assert!(paths.iter().any(|p| p.ends_with("chain")));
    assert!(paths.iter().any(|p| p.ends_with("tree")));
}

#[test]
fn test_chain_paths_for_network_testnet() {
    let paths = node::chain_paths_for_network("/data", Network::Testnet);
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0], Path::new("/data/testnet"));
}

#[test]
fn test_chain_paths_for_network_regtest() {
    let paths = node::chain_paths_for_network("/data", Network::Regtest);
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0], Path::new("/data/regtest"));
}

#[test]
fn test_chain_paths_for_network_simnet() {
    let paths = node::chain_paths_for_network("/data", Network::Simnet);
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0], Path::new("/data/simnet"));
}

#[test]
fn test_read_log_tail_nonexistent_file() {
    let result = node::read_log_tail(Path::new("/nonexistent/log.txt"));
    assert_eq!(result, "");
}

#[test]
fn test_read_log_tail_empty_file() {
    let dir = std::env::temp_dir().join("namehold_test_log_tail");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("empty.log");
    std::fs::write(&path, "").unwrap();
    let result = node::read_log_tail(&path);
    assert_eq!(result, "");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_read_log_tail_short_content() {
    let dir = std::env::temp_dir().join("namehold_test_log_tail_short");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("short.log");
    std::fs::write(&path, "line1\nline2\nline3").unwrap();
    let result = node::read_log_tail(&path);
    assert!(result.contains("line1"));
    assert!(result.contains("line3"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_node_start_error_no_log() {
    let result = node::node_start_error("/nonexistent/dir");
    assert!(result.is_none());
}

#[test]
fn test_node_start_error_log_without_error() {
    let dir = std::env::temp_dir().join("namehold_test_node_start_ok");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("namehold-hsd.log"),
        "hsd v6.0.0\nstarting...\nlistening on port 12037",
    )
    .unwrap();
    let result = node::node_start_error(dir.to_str().unwrap());
    assert!(result.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_node_start_error_log_with_generic_error() {
    let dir = std::env::temp_dir().join("namehold_test_node_start_err");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("namehold-hsd.log"),
        "Error: port 12037 already in use",
    )
    .unwrap();
    let result = node::node_start_error(dir.to_str().unwrap());
    assert!(result.is_some());
    let (msg, is_index_mismatch) = result.unwrap();
    assert!(msg.contains("hsd failed to start"));
    assert!(!is_index_mismatch);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_node_start_error_log_with_index_mismatch() {
    let dir = std::env::temp_dir().join("namehold_test_node_start_idx");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("namehold-hsd.log"),
        "Error: Cannot retroactively enable tx indexing",
    )
    .unwrap();
    let result = node::node_start_error(dir.to_str().unwrap());
    assert!(result.is_some());
    let (msg, is_index_mismatch) = result.unwrap();
    assert!(msg.contains("indexes don't match"));
    assert!(is_index_mismatch);
    let _ = std::fs::remove_dir_all(&dir);
}

// --- hsd_candidates tests ---

#[test]
fn test_hsd_candidates_returns_default_paths() {
    let candidates = node::hsd_candidates();
    // Should always include the two default paths
    assert!(candidates.contains(&"/opt/homebrew/bin/hsd".to_string()));
    assert!(candidates.contains(&"/usr/local/bin/hsd".to_string()));
    // Should have at least 2 candidates
    assert!(candidates.len() >= 2);
}

#[test]
fn test_hsd_candidates_includes_home_paths() {
    // If HOME is set, should include home-based paths
    if std::env::var("HOME").is_ok() {
        let candidates = node::hsd_candidates();
        let home = std::env::var("HOME").unwrap();
        assert!(candidates
            .iter()
            .any(|c| c.contains(&format!("{home}/.npm-global/bin/hsd"))));
        assert!(candidates
            .iter()
            .any(|c| c.contains(&format!("{home}/.npm/bin/hsd"))));
        assert!(candidates
            .iter()
            .any(|c| c.contains(&format!("{home}/.local/bin/hsd"))));
    }
}

// --- find_hsd_binary tests ---

#[test]
fn test_find_hsd_binary_with_override() {
    let result = node::find_hsd_binary(Some("/custom/hsd"));
    assert_eq!(result, "/custom/hsd");
}

#[test]
fn test_find_hsd_binary_with_empty_override() {
    // Empty override should fall through to candidates/which
    let result = node::find_hsd_binary(Some(""));
    // Should return either a candidate path or "hsd" as fallback
    assert!(!result.is_empty());
}

#[test]
fn test_find_hsd_binary_with_none() {
    // None override should use candidates/which
    let result = node::find_hsd_binary(None);
    // Should return either a candidate path or "hsd" as fallback
    assert!(!result.is_empty());
}

// --- find_hsd_binary additional tests ---

#[test]
fn test_find_hsd_binary_override_whitespace_falls_through() {
    let result = node::find_hsd_binary(Some("  "));
    assert!(!result.is_empty());
}

#[test]
fn test_find_hsd_binary_override_with_path() {
    let result = node::find_hsd_binary(Some("/usr/local/bin/hsd"));
    assert_eq!(result, "/usr/local/bin/hsd");
}

// --- hsd_candidates edge cases ---

#[test]
fn test_hsd_candidates_does_not_contain_duplicates() {
    let candidates = node::hsd_candidates();
    let mut sorted = candidates.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        candidates.len(),
        sorted.len(),
        "candidates should not contain duplicates"
    );
}

// --- parse_hsd_version tests ---

#[test]
fn test_parse_hsd_version_plain() {
    assert_eq!(node::parse_hsd_version("8.0.0"), Some((8, 0, 0)));
}

#[test]
fn test_parse_hsd_version_v_prefix() {
    assert_eq!(node::parse_hsd_version("v8.0.0"), Some((8, 0, 0)));
}

#[test]
fn test_parse_hsd_version_capital_v_prefix() {
    assert_eq!(node::parse_hsd_version("V8.0.0"), Some((8, 0, 0)));
}

#[test]
fn test_parse_hsd_version_prerelease_suffix() {
    assert_eq!(node::parse_hsd_version("8.0.0-rc.1"), Some((8, 0, 0)));
}

#[test]
fn test_parse_hsd_version_build_suffix() {
    assert_eq!(node::parse_hsd_version("8.0.0+build.5"), Some((8, 0, 0)));
}

#[test]
fn test_parse_hsd_version_whitespace_trimmed() {
    assert_eq!(node::parse_hsd_version("  8.0.0  \n"), Some((8, 0, 0)));
}

#[test]
fn test_parse_hsd_version_missing_patch_defaults_zero() {
    assert_eq!(node::parse_hsd_version("8.1"), Some((8, 1, 0)));
}

#[test]
fn test_parse_hsd_version_major_only_defaults_zero() {
    assert_eq!(node::parse_hsd_version("8"), Some((8, 0, 0)));
}

#[test]
fn test_parse_hsd_version_older_version() {
    assert_eq!(node::parse_hsd_version("2.5.2"), Some((2, 5, 2)));
}

#[test]
fn test_parse_hsd_version_garbage() {
    assert_eq!(node::parse_hsd_version("not a version"), None);
}

#[test]
fn test_parse_hsd_version_empty() {
    assert_eq!(node::parse_hsd_version(""), None);
}

#[test]
fn test_parse_hsd_version_whitespace_only() {
    assert_eq!(node::parse_hsd_version("   "), None);
}

#[test]
fn test_parse_hsd_version_lone_v_no_digits() {
    assert_eq!(node::parse_hsd_version("v"), None);
}

#[test]
fn test_parse_hsd_version_trailing_dot() {
    // "8." leaves an empty minor segment, which fails to parse as u32.
    assert_eq!(node::parse_hsd_version("8."), None);
}

// --- HSD_MIN_VERSION comparison tests ---

#[test]
fn test_min_version_is_8_0_0() {
    assert_eq!(node::HSD_MIN_VERSION, (8, 0, 0));
}

#[test]
fn test_version_below_min_is_rejected() {
    let found = node::parse_hsd_version("7.9.9").unwrap();
    assert!(found < node::HSD_MIN_VERSION);
}

#[test]
fn test_version_major_below_min_is_rejected() {
    let found = node::parse_hsd_version("2.5.2").unwrap();
    assert!(found < node::HSD_MIN_VERSION);
}

#[test]
fn test_version_equal_min_is_accepted() {
    let found = node::parse_hsd_version("8.0.0").unwrap();
    assert!(found >= node::HSD_MIN_VERSION);
}

#[test]
fn test_version_above_min_is_accepted() {
    let found = node::parse_hsd_version("9.0.0").unwrap();
    assert!(found >= node::HSD_MIN_VERSION);
}

#[test]
fn test_version_patch_above_min_is_accepted() {
    let found = node::parse_hsd_version("8.0.1").unwrap();
    assert!(found >= node::HSD_MIN_VERSION);
}

#[test]
fn test_version_prerelease_of_min_is_accepted() {
    // parse drops the prerelease tag, so 8.0.0-rc.1 parses to exactly 8.0.0.
    let found = node::parse_hsd_version("8.0.0-rc.1").unwrap();
    assert!(found >= node::HSD_MIN_VERSION);
}

// --- hsd_candidates: nvm-managed node discovery loop -------------------------

#[test]
fn test_hsd_candidates_discovers_nvm_managed_hsd() {
    // The nvm branch (`~/.nvm/versions/node/<ver>/bin/hsd`) only runs when that
    // directory tree exists AND contains an hsd binary — otherwise the loop is
    // skipped. Build the tree under a throwaway HOME so the discovery loop is
    // exercised deterministically without depending on the dev's real nvm setup.
    let home = std::env::temp_dir().join("namehold_nvm_discovery_test");
    let _ = std::fs::remove_dir_all(&home);
    let bin = home.join(".nvm/versions/node/v20.11.0/bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("hsd"), b"#!/bin/sh\n").unwrap();

    // hsd_candidates reads $HOME; override it for the duration of the call.
    let prev = std::env::var("HOME").ok();
    std::env::set_var("HOME", &home);
    let candidates = node::hsd_candidates();
    match prev {
        Some(h) => std::env::set_var("HOME", h),
        None => std::env::remove_var("HOME"),
    }

    let expected = bin.join("hsd").to_string_lossy().to_string();
    assert!(
        candidates.contains(&expected),
        "nvm-managed hsd not discovered: {candidates:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

// --- find_hsd_binary: which/PATH fallback (no matching candidate) ------------

#[test]
fn test_find_hsd_binary_none_override_returns_nonempty() {
    // With no override and (typically) no installed hsd on a CI box, this walks
    // candidates → `which hsd` → the bare "hsd" fallback. Whichever branch wins,
    // the result is always a non-empty binary name/path.
    let result = node::find_hsd_binary(None);
    assert!(!result.is_empty());
}

// --- get_hsd_version: spawns `<binary> --version` ----------------------------

#[test]
fn test_get_hsd_version_none_for_missing_binary() {
    // A path that can't be spawned → `Command::output()` errors → None (the
    // `.ok()?` short-circuit). Covers the version-probe helper's failure arm.
    assert!(node::get_hsd_version("/nonexistent/definitely/not/hsd").is_none());
}

#[cfg(unix)]
#[test]
fn test_get_hsd_version_reads_stdout_of_runnable_binary() {
    use std::os::unix::fs::PermissionsExt;
    // A runnable stub that prints a version string exercises the success arm
    // (status.success() && non-empty stdout → Some(version)).
    let dir = std::env::temp_dir().join("namehold_get_version_ok");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let stub = dir.join("hsd");
    std::fs::write(&stub, "#!/bin/sh\necho \"8.3.1\"\n").unwrap();
    let mut perms = std::fs::metadata(&stub).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&stub, perms).unwrap();

    let v = node::get_hsd_version(&stub.to_string_lossy());
    assert_eq!(v.as_deref(), Some("8.3.1"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn test_get_hsd_version_none_when_exit_nonzero() {
    use std::os::unix::fs::PermissionsExt;
    // Runs but exits non-zero → None (the !status.success() arm).
    let dir = std::env::temp_dir().join("namehold_get_version_fail");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let stub = dir.join("hsd");
    std::fs::write(&stub, "#!/bin/sh\nexit 1\n").unwrap();
    let mut perms = std::fs::metadata(&stub).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&stub, perms).unwrap();

    assert!(node::get_hsd_version(&stub.to_string_lossy()).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

// --- parse_hsd_version: additional edge cases --------------------------------

#[test]
fn test_parse_hsd_version_prerelease_and_build_suffix() {
    assert_eq!(node::parse_hsd_version("8.0.0-rc.1+build.5"), Some((8, 0, 0)));
}

#[test]
fn test_parse_hsd_version_v_prefix_with_prerelease() {
    assert_eq!(node::parse_hsd_version("v8.1.2-beta"), Some((8, 1, 2)));
}

// --- read_log_tail: only the last 8 lines are kept ---------------------------

#[test]
fn test_read_log_tail_keeps_only_last_eight_lines() {
    let dir = std::env::temp_dir().join("namehold_log_tail_eight");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let body: String = (0..20).map(|i| format!("line {i}\n")).collect();
    std::fs::write(dir.join("hsd.log"), body).unwrap();

    let tail = node::read_log_tail(&dir.join("hsd.log"));
    assert!(tail.contains("line 19"));
    assert!(tail.contains("line 12"));
    // Line 11 and earlier are dropped (only the last 8 are retained).
    assert!(!tail.contains("line 11"));
    assert!(!tail.contains("line 0\n"));
    let _ = std::fs::remove_dir_all(&dir);
}

// --- (reserve section for future tests) ---
