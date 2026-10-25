//! Tests for `commands::local_node`: recognising the app's own hsd across
//! sessions by its pid and process start time (spec 2026-10-24 R6).

use crate::commands::local_node::{own_node_pid, process_start_time, write_pid_file, PID_FILE};

/// The start time is read from the OS and does not change while the process
/// lives, so a second read of the same process gives the same value.
#[cfg(unix)]
#[test]
fn a_live_process_has_one_start_time() {
    let me = std::process::id();
    let first = process_start_time(me).expect("this process has a start time");
    assert_eq!(process_start_time(me), Some(first));
}

/// A process that has exited and been reaped has no start time.
#[cfg(unix)]
#[test]
fn an_exited_process_has_no_start_time() {
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    assert_eq!(process_start_time(pid), None);
}

fn data_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("namehold_hsd_pid_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The pid file of a live process names that process.
#[cfg(unix)]
#[test]
fn a_pid_file_of_a_live_process_names_it() {
    let dir = data_dir("live");
    let me = std::process::id();
    write_pid_file(&dir, me).unwrap();
    assert_eq!(own_node_pid(&dir), Some(me));
    assert!(dir.join(PID_FILE).exists(), "a live record stays");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A pid file whose start time is not the live process's (the pid was reused
/// by another process), whose process is gone, or that cannot be read names
/// nothing. Reading it leaves it in place: a status read never changes a data
/// directory.
#[cfg(unix)]
#[test]
fn a_stale_pid_file_names_nothing_and_is_left_alone() {
    let me = std::process::id();
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let dead = child.id();
    child.wait().unwrap();
    for (tag, text) in [
        ("reused", format!("{me} 1\n")),
        ("dead", format!("{dead} 1\n")),
        ("garbage", "not a pid file".to_string()),
    ] {
        let dir = data_dir(tag);
        std::fs::write(dir.join(PID_FILE), text).unwrap();
        assert_eq!(own_node_pid(&dir), None, "{tag}");
        assert!(dir.join(PID_FILE).exists(), "{tag}: a read leaves it");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// No pid file, no own node.
#[test]
fn no_pid_file_names_nothing() {
    let dir = data_dir("none");
    assert_eq!(own_node_pid(&dir), None);
    let _ = std::fs::remove_dir_all(&dir);
}
