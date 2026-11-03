//! The daemon's SIGTERM path, against the real binary and a temporary home: the
//! pid file is gone once it exits, so logout or shutdown leaves no stale pid for
//! the OS to hand to another process.
#![cfg(unix)]

use std::time::{Duration, Instant};

#[test]
fn sigterm_removes_the_pid_file() {
    let home = std::env::temp_dir().join(format!("namehold-syncd-it-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let pid_file = home.join(".namehold").join("syncd.pid");

    let mut daemon = std::process::Command::new(env!("CARGO_BIN_EXE_namehold-syncd"))
        .env("HOME", &home)
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !pid_file.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let started = pid_file.exists();

    unsafe {
        libc::kill(daemon.id() as libc::pid_t, libc::SIGTERM);
    }
    let status = daemon.wait().unwrap();
    let left_behind = pid_file.exists();
    let _ = std::fs::remove_dir_all(&home);

    assert!(started, "the daemon writes its pid file");
    assert!(!left_behind, "exited with {status:?}, pid file still there");
}
