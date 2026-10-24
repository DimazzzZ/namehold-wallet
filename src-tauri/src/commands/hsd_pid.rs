//! Which local hsd is the app's own (spec 2026-10-24 R6).
//!
//! A node the app started in an earlier session and left running for the
//! background daemon is no longer a child of this session, and hsd overwrites
//! its own command line (`process.title = 'hsd'`), so neither tells it apart
//! from a user's hsd. The app writes the child's pid and process start time to
//! `<data_dir>/namehold-hsd.pid` when it spawns it; a live process with that
//! pid and that start time is the same process, since a reused pid starts at
//! another time.

/// The OS's start time of process `pid`, as an opaque value that stays the
/// same for the process's whole life. `None` when there is no such process,
/// or on a platform where it cannot be read (the node then counts as outside,
/// never as own).
#[cfg(target_os = "macos")]
pub(crate) fn process_start_time(pid: u32) -> Option<u64> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let n = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    (n == size).then(|| info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec)
}

/// Field 22 of `/proc/<pid>/stat`, the start time in clock ticks since boot.
/// The process name (field 2) is in parentheses and may hold spaces, so the
/// fields are counted from the last `)`.
#[cfg(target_os = "linux")]
pub(crate) fn process_start_time(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_name = &stat[stat.rfind(')')? + 1..];
    // After the name: state is field 3, so starttime (22) is the 20th.
    after_name.split_whitespace().nth(19)?.parse().ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn process_start_time(_pid: u32) -> Option<u64> {
    None
}

/// The pid file's name inside a network's data directory.
pub(crate) const PID_FILE: &str = "namehold-hsd.pid";

/// Record `pid`, with its start time, as the app's own node of `data_dir`.
/// Written at spawn; a process whose start time cannot be read is not
/// recorded, so it can never be taken for own later.
pub(crate) fn write_pid_file(data_dir: &std::path::Path, pid: u32) -> std::io::Result<()> {
    let Some(start) = process_start_time(pid) else {
        return Ok(());
    };
    std::fs::write(data_dir.join(PID_FILE), format!("{pid} {start}\n"))
}

/// Forget the own node of `data_dir` (it was stopped).
pub(crate) fn remove_pid_file(data_dir: &std::path::Path) {
    let _ = std::fs::remove_file(data_dir.join(PID_FILE));
}

/// The pid of the app's own node of `data_dir`, when its pid file names a
/// live process with the recorded start time. A file that names a process
/// that is gone, a reused pid (another start time) or that cannot be read is
/// stale: it is removed and names nothing.
pub(crate) fn own_node_pid(data_dir: &std::path::Path) -> Option<u32> {
    let path = data_dir.join(PID_FILE);
    let text = std::fs::read_to_string(&path).ok()?;
    let mut fields = text.split_whitespace();
    let record = (|| {
        let pid: u32 = fields.next()?.parse().ok()?;
        let start: u64 = fields.next()?.parse().ok()?;
        Some((pid, start))
    })();
    match record {
        Some((pid, start)) if process_start_time(pid) == Some(start) => Some(pid),
        _ => {
            let _ = std::fs::remove_file(&path);
            None
        }
    }
}
