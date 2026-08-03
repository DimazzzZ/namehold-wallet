use std::fs;

use crate::commands::node;

#[test]
fn explicit_hsrd_binary_path_takes_precedence() {
    let result = node::pick_hsrd_path(Some(" /custom/hsrd "), &["/bin/sh".into()]);
    assert_eq!(result.as_deref(), Some("/custom/hsrd"));
}

#[test]
fn candidate_lookup_uses_first_existing_binary() {
    let result = node::pick_hsrd_path(None, &["/not/present".into(), "/bin/sh".into()]);
    assert_eq!(result.as_deref(), Some("/bin/sh"));
}

#[test]
fn version_parser_accepts_release_output() {
    assert_eq!(node::parse_hsrd_version("hsrd 0.3.4"), Some((0, 3, 4)));
    assert_eq!(
        node::parse_hsrd_version("hsrd v1.2.0-rc.1"),
        Some((1, 2, 0))
    );
    assert_eq!(node::parse_hsrd_version("not a version"), None);
}

#[test]
fn candidates_include_cargo_and_local_bin_locations() {
    let candidates = node::hsrd_candidates();
    assert!(candidates
        .iter()
        .any(|path| path.ends_with("/.cargo/bin/hsrd")));
    assert!(candidates
        .iter()
        .any(|path| path.ends_with("/.local/bin/hsrd")));
}

#[test]
fn startup_error_reads_only_the_managed_sidecar_log() {
    let dir =
        std::env::temp_dir().join(format!("namehold-hsrd-node-test-{}", rand::random::<u64>()));
    fs::create_dir_all(&dir).unwrap();
    assert!(node::node_start_error(&dir).is_none());
    fs::write(
        dir.join("namehold-hsrd.log"),
        "starting\nerror: wallet index is unavailable\n",
    )
    .unwrap();
    let message = node::node_start_error(&dir).expect("error log");
    assert!(message.contains("wallet index is unavailable"));
    fs::remove_dir_all(dir).unwrap();
}
