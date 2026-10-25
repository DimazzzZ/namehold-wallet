//! The app's local hsd nodes, one per network (spec 2026-10-24): where each
//! network's chain lives, and which running node is the app's own (R6, R7).
//!
//! A node the app started in an earlier session and left running for the
//! background daemon is no longer a child of this session, and hsd overwrites
//! its own command line (`process.title = 'hsd'`), so neither tells it apart
//! from a user's hsd. The app writes the child's pid and process start time to
//! `<data_dir>/namehold-hsd.pid` when it spawns it; a live process with that
//! pid and that start time is the same process, since a reused pid starts at
//! another time.

use crate::db;
use crate::error::AppError;
use crate::noncustodial::network::Network;
use crate::AppState;

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
/// stale and names nothing. Reading never touches the file: a status read
/// must not change a data directory, so a stale record is replaced by the
/// next start and removed by the next stop or quit of that network's node.
pub(crate) fn own_node_pid(data_dir: &std::path::Path) -> Option<u32> {
    let text = std::fs::read_to_string(data_dir.join(PID_FILE)).ok()?;
    let mut fields = text.split_whitespace();
    let record = (|| {
        let pid: u32 = fields.next()?.parse().ok()?;
        let start: u64 = fields.next()?.parse().ok()?;
        Some((pid, start))
    })();
    match record {
        Some((pid, start)) if process_start_time(pid) == Some(start) => Some(pid),
        _ => None,
    }
}

/// Ask process `pid` to exit (SIGTERM, so hsd closes its chain cleanly).
/// Called only for a pid `own_node_pid` vouched for.
#[cfg(unix)]
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn terminate(pid: u32) {
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
}

/// No own-node record is ever written off macOS and Linux, so there is no
/// pid to stop.
#[cfg(not(unix))]
pub(crate) fn terminate(_pid: u32) {}

/// Network-scoped data dir. hsd isolates non-mainnet chains *inside* a single
/// prefix (`<prefix>/regtest`, `<prefix>/testnet`), while mainnet writes
/// `blocks/chain/tree` at the prefix root — so one shared prefix lets a mainnet
/// chain at the root sit beside a regtest subdir, the exact overlap that let a
/// mainnet chain end up driving a regtest wallet.
///
/// The fix gives each network its own data-dir *root*: mainnet keeps the base
/// unchanged (so existing mainnet data is never moved), and every other network
/// gets `<base>/<network>`. That is the directory the log, RPC, and migration
/// treat as the chain root. Note this is the *data dir*, not the hsd
/// `--prefix`: `start_hsd` passes hsd the un-scoped base as `--prefix` and lets
/// `--network` create exactly this `<base>/<network>` subdir — so hsd's chain
/// root coincides with this data dir instead of nesting a second time.
pub(crate) fn resolve_data_dir_for_network(
    state: &AppState,
    network: Network,
) -> Result<String, AppError> {
    let base = resolve_configured_base(state)?;
    Ok(network_scoped_data_dir(&base, network))
}

/// Pure path rule: mainnet → `base` unchanged; any other network → its own
/// top-level root `base/<network>`. Kept pure (no IO, no state) so the layout
/// is unit-testable without a filesystem.
pub(crate) fn network_scoped_data_dir(base: &str, network: Network) -> String {
    match network {
        Network::Main => base.to_string(),
        other => std::path::Path::new(base)
            .join(other.as_str())
            .to_string_lossy()
            .into_owned(),
    }
}

/// The configured, un-scoped base prefix (`hsd_prefix` setting, else `~/.hsd`).
/// Split out from [`resolve_data_dir_for_network`] so the migration can reason
/// about the base independent of the network scoping.
pub(crate) fn resolve_configured_base(state: &AppState) -> Result<String, AppError> {
    let configured = {
        let db = state.db.lock().map_err(|e| AppError::Lock(e.to_string()))?;
        db::queries::get_settings(&db)?
            .get("hsd_prefix")
            .cloned()
            .unwrap_or_default()
    };
    let configured = configured.trim();
    if !configured.is_empty() {
        return Ok(configured.to_string());
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    Ok(format!("{home}/.hsd"))
}

/// The pid of `network`'s own node an earlier session left, if any (R6).
pub(crate) fn own_node_pid_of(state: &AppState, network: Network) -> Option<u32> {
    // A data dir that cannot be resolved (the settings cannot be read) holds
    // no record we could read: no own node is known there.
    let data_dir = resolve_data_dir_for_network(state, network).ok()?;
    own_node_pid(std::path::Path::new(&data_dir))
}

/// Why Stop refuses a node it did not start (spec 2026-10-24 R7).
pub(crate) const OUTSIDE_NODE_NOT_STOPPED: &str =
    "This node was not started by Namehold, so Namehold does not stop it.";

/// Which own node a network has (spec 2026-10-24 R6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnNode {
    /// A live child of this session.
    Child,
    /// A node an earlier session started and left, named by its pid file.
    Left(u32),
}

/// The one rule for "`network`'s own node", read by start, status, the tray,
/// stop and quit alike: a live child of this session (a child that has
/// exited is reaped and does not count), else the node a pid file names.
pub(crate) fn own_node(state: &AppState, network: Network) -> Option<OwnNode> {
    {
        let mut children = match state.hsd_children.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(child) = children.get_mut(&network) {
            if matches!(child.try_wait(), Ok(None)) {
                return Some(OwnNode::Child);
            }
            children.remove(&network);
        }
    }
    own_node_pid_of(state, network).map(OwnNode::Left)
}

/// Stop `network`'s own node, whichever it is, and forget its record: kill
/// this session's child, or SIGTERM the node an earlier session left.
/// `false` when the network has no own node; a node Namehold did not start
/// is never touched (R7).
pub(crate) fn stop_own_node(state: &AppState, network: Network) -> bool {
    let child = match state.hsd_children.lock() {
        Ok(mut g) => g.remove(&network),
        Err(poisoned) => poisoned.into_inner().remove(&network),
    };
    // Best-effort: with no data dir to resolve there is no record to read or
    // remove, and a child of this session is still stopped.
    let data_dir = resolve_data_dir_for_network(state, network).ok();
    let data_dir = data_dir.as_deref().map(std::path::Path::new);
    let stopped = if let Some(mut child) = child {
        let _ = child.kill();
        let _ = child.wait();
        true
    } else if let Some(pid) = data_dir.and_then(own_node_pid) {
        terminate(pid);
        true
    } else {
        false
    };
    if let Some(dir) = data_dir {
        remove_pid_file(dir);
    }
    stopped
}
