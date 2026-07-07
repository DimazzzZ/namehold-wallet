use crate::commands::node;
use crate::noncustodial::network::Network;
use tauri::Manager;

// --- pick_hsd_path tests ---

#[test]
fn test_pick_hsd_path_override_wins() {
    let result = node::pick_hsd_path(Some("/custom/hsd"), &[]);
    assert_eq!(result, Some("/custom/hsd".to_string()));
}

#[test]
fn test_pick_hsd_path_override_trimmed() {
    let result = node::pick_hsd_path(Some("  /custom/hsd  "), &[]);
    assert_eq!(result, Some("/custom/hsd".to_string()));
}

#[test]
fn test_pick_hsd_path_empty_override_skipped() {
    let result = node::pick_hsd_path(Some(""), &["/nonexistent".to_string()]);
    assert_eq!(result, None);
}

#[test]
fn test_pick_hsd_path_whitespace_override_skipped() {
    let result = node::pick_hsd_path(Some("   "), &[]);
    assert_eq!(result, None);
}

#[test]
fn test_pick_hsd_path_no_override_no_candidates() {
    let result = node::pick_hsd_path(None, &[]);
    assert_eq!(result, None);
}

#[test]
fn test_pick_hsd_path_no_match() {
    let result = node::pick_hsd_path(None, &["/nonexistent/path/hsd".to_string()]);
    assert_eq!(result, None);
}

#[test]
fn test_pick_hsd_path_matches_existing_candidate() {
    // /bin/sh exists on all unix systems
    let result = node::pick_hsd_path(None, &["/nonexistent".to_string(), "/bin/sh".to_string()]);
    assert_eq!(result, Some("/bin/sh".to_string()));
}

#[test]
fn test_pick_hsd_path_first_match_wins() {
    let result = node::pick_hsd_path(
        None,
        &["/bin/sh".to_string(), "/bin/ls".to_string()],
    );
    assert_eq!(result, Some("/bin/sh".to_string()));
}

// --- chain_paths_for_network tests ---

#[test]
fn test_chain_paths_mainnet() {
    let paths = node::chain_paths_for_network("/data", Network::Main);
    assert_eq!(paths.len(), 3);
    assert_eq!(paths[0], std::path::Path::new("/data/blocks"));
    assert_eq!(paths[1], std::path::Path::new("/data/chain"));
    assert_eq!(paths[2], std::path::Path::new("/data/tree"));
}

#[test]
fn test_chain_paths_testnet() {
    let paths = node::chain_paths_for_network("/data", Network::Testnet);
    assert_eq!(paths, vec![std::path::Path::new("/data/testnet")]);
}

#[test]
fn test_chain_paths_regtest() {
    let paths = node::chain_paths_for_network("/data", Network::Regtest);
    assert_eq!(paths, vec![std::path::Path::new("/data/regtest")]);
}

#[test]
fn test_chain_paths_simnet() {
    let paths = node::chain_paths_for_network("/data", Network::Simnet);
    assert_eq!(paths, vec![std::path::Path::new("/data/simnet")]);
}

// --- node_start_error tests ---

#[test]
fn test_node_start_error_no_log() {
    let result = node::node_start_error("/nonexistent_dir_12345");
    assert!(result.is_none());
}

#[test]
fn test_node_start_error_empty_log() {
    let dir = std::env::temp_dir().join("namehold_test_empty_log");
    let _ = std::fs::create_dir_all(&dir);
    let log_path = dir.join("namehold-hsd.log");
    std::fs::write(&log_path, "").unwrap();
    let result = node::node_start_error(dir.to_str().unwrap());
    assert!(result.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_node_start_error_clean_log() {
    let dir = std::env::temp_dir().join("namehold_test_clean_log");
    let _ = std::fs::create_dir_all(&dir);
    let log_path = dir.join("namehold-hsd.log");
    std::fs::write(&log_path, "hsd started successfully\nlistening on port 12038\n").unwrap();
    let result = node::node_start_error(dir.to_str().unwrap());
    assert!(result.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_node_start_error_generic_error() {
    let dir = std::env::temp_dir().join("namehold_test_generic_err");
    let _ = std::fs::create_dir_all(&dir);
    let log_path = dir.join("namehold-hsd.log");
    std::fs::write(&log_path, "Error: port 12038 already in use\n").unwrap();
    let result = node::node_start_error(dir.to_str().unwrap());
    assert!(result.is_some());
    let (msg, mismatch) = result.unwrap();
    assert!(!mismatch);
    assert!(msg.contains("hsd failed to start"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_node_start_error_index_mismatch() {
    let dir = std::env::temp_dir().join("namehold_test_idx_mismatch");
    let _ = std::fs::create_dir_all(&dir);
    let log_path = dir.join("namehold-hsd.log");
    std::fs::write(
        &log_path,
        "Error: Cannot retroactively enable indexing on an existing chain\n",
    )
    .unwrap();
    let result = node::node_start_error(dir.to_str().unwrap());
    assert!(result.is_some());
    let (msg, mismatch) = result.unwrap();
    assert!(mismatch);
    assert!(msg.contains("indexes don't match"));
    let _ = std::fs::remove_dir_all(&dir);
}

// --- read_log_tail tests ---

#[test]
fn test_read_log_tail_nonexistent() {
    let result = node::read_log_tail(std::path::Path::new("/nonexistent_12345/log"));
    assert_eq!(result, "");
}

#[test]
fn test_read_log_tail_with_content() {
    let dir = std::env::temp_dir().join("namehold_test_log_tail");
    let _ = std::fs::create_dir_all(&dir);
    let log_path = dir.join("test.log");
    std::fs::write(&log_path, "line1\nline2\nline3\n").unwrap();
    let result = node::read_log_tail(&log_path);
    assert!(result.contains("line1"));
    assert!(result.contains("line3"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_read_log_tail_empty_file() {
    let dir = std::env::temp_dir().join("namehold_test_log_empty");
    let _ = std::fs::create_dir_all(&dir);
    let log_path = dir.join("test.log");
    std::fs::write(&log_path, "").unwrap();
    let result = node::read_log_tail(&log_path);
    assert_eq!(result, "");
    let _ = std::fs::remove_dir_all(&dir);
}

// Note: resolve_data_dir, active_profile_network, is_running, configured_hsd_path
// are private fn — tested indirectly via the node_status command test below.

// --- node_status command test (covers resolve_data_dir, active_profile_network,
//     is_running, configured_hsd_path, find_hsd_binary, get_hsd_version, probe_node) ---

#[tokio::test]
async fn test_node_status_command() {
    let state = crate::tests::command_helpers::create_test_state();
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app");
    // node_status will fail to probe the node (no hsd running) but should return
    // a valid JSON with the expected shape.
    let result = crate::commands::node::node_status(app.state()).await;
    assert!(result.is_ok());
    let val = result.unwrap();
    assert!(val.get("binary").is_some());
    assert!(val.get("data_dir").is_some());
    assert!(val.get("network").is_some());
    assert!(val.get("process_alive").is_some());
    assert!(val.get("connected").is_some());
    // No node running → not connected
    assert_eq!(val["connected"], serde_json::json!(false));
    assert_eq!(val["process_alive"], serde_json::json!(false));
    assert_eq!(val["read_source"], serde_json::json!("explorer"));
}
