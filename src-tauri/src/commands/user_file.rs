//! Writing a file at a path the renderer names (a native save dialog's
//! answer). The path comes from the webview, which SECURITY.md §1 treats as
//! possibly compromised, so the write refuses what a save dialog would never
//! hand back for a new or replaced document: a symbolic link (whose target
//! could be any file of the user's) and anything that is not a regular file.

use std::path::Path;

use crate::error::AppError;

/// Why a save is refused when its path is a symbolic link.
pub(crate) const SAVE_TO_SYMLINK: &str =
    "the chosen file is a symbolic link: choose a regular file or a new name";

/// Why a save is refused when its path is not a regular file.
pub(crate) const SAVE_TO_NON_FILE: &str =
    "the chosen path is not a regular file: choose a regular file or a new name";

/// Write `contents` to `path`, creating it or replacing a regular file
/// there. A symbolic link at `path` is refused and its target left
/// untouched, and so is anything that is not a regular file (a directory,
/// a FIFO, a device); nothing is truncated before those checks pass. On
/// unix the last component is opened with `O_NOFOLLOW`, so a link swapped
/// in after the check is refused by the open itself, and with `owner_only`
/// the file is 0600 whether it is new or already existed, set before
/// anything is written. What stays open (SECURITY.md §1): the path itself,
/// any regular file the user's account can write, and a hard link to one.
pub(crate) fn write_chosen_file(
    path: &Path,
    contents: &[u8],
    owner_only: bool,
) -> Result<(), AppError> {
    use std::io::Write;
    refuse_unless_regular(path)?;
    let mut file = open_regular(path, owner_only)?;
    file.set_len(0)?;
    file.write_all(contents)?;
    Ok(())
}

/// Open `path` for writing, created if missing, without truncating it, and
/// refuse what is not a regular file once open. On unix the last component
/// is not followed (`O_NOFOLLOW`), and with `owner_only` the file is 0600.
fn open_regular(path: &Path, owner_only: bool) -> Result<std::fs::File, AppError> {
    let mut opts = std::fs::OpenOptions::new();
    // No `truncate` here: the opened file is checked first.
    opts.write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW);
        if owner_only {
            opts.mode(0o600);
        }
    }
    let file = opts.open(path).map_err(|e| {
        // A link swapped in after the check: `O_NOFOLLOW` answers ELOOP.
        #[cfg(unix)]
        if e.raw_os_error() == Some(libc::ELOOP) {
            return AppError::InvalidInput(SAVE_TO_SYMLINK.into());
        }
        AppError::from(e)
    })?;
    if !file.metadata()?.is_file() {
        return Err(AppError::InvalidInput(SAVE_TO_NON_FILE.into()));
    }
    // `mode` applies to a new file only: an existing one keeps its own.
    #[cfg(unix)]
    if owner_only {
        file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = owner_only;
    Ok(file)
}

/// Refuse a path that exists and is a symbolic link or not a regular file;
/// a path that does not exist yet is fine.
fn refuse_unless_regular(path: &Path) -> Result<(), AppError> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => Err(AppError::InvalidInput(SAVE_TO_SYMLINK.into())),
        Ok(m) if !m.is_file() => Err(AppError::InvalidInput(SAVE_TO_NON_FILE.into())),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> std::path::PathBuf {
        let d =
            std::env::temp_dir().join(format!("namehold_user_file_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir(&d).unwrap();
        d
    }

    #[test]
    fn writes_a_new_file_and_replaces_a_longer_one() {
        let d = dir("plain");
        let p = d.join("f.json");
        write_chosen_file(&p, b"first, and longer", false).unwrap();
        write_chosen_file(&p, b"second", false).unwrap();
        let got = std::fs::read_to_string(&p).unwrap();
        std::fs::remove_dir_all(&d).unwrap();
        assert_eq!(got, "second");
    }

    /// A dangling link would otherwise create its target, wherever it points.
    #[cfg(unix)]
    #[test]
    fn a_dangling_symlink_is_refused_and_creates_nothing() {
        let d = dir("dangling");
        let target = d.join("nowhere.txt");
        let link = d.join("f.json");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let err = write_chosen_file(&link, b"x", true).unwrap_err();
        let created = target.exists();
        std::fs::remove_dir_all(&d).unwrap();
        assert!(err.to_string().contains(SAVE_TO_SYMLINK), "{err:?}");
        assert!(!created);
    }

    /// The open itself does not follow a link swapped in after the check.
    #[cfg(unix)]
    #[test]
    fn the_open_refuses_a_symlink_on_its_own() {
        let d = dir("swap");
        let target = d.join("target.txt");
        std::fs::write(&target, "keep").unwrap();
        let link = d.join("f.json");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let err = open_regular(&link, true).unwrap_err();
        let kept = std::fs::read_to_string(&target).unwrap();
        std::fs::remove_dir_all(&d).unwrap();
        assert!(err.to_string().contains(SAVE_TO_SYMLINK), "{err:?}");
        assert_eq!(kept, "keep");
    }

    /// A FIFO is refused before it is opened (an open would block).
    #[cfg(unix)]
    #[test]
    fn a_fifo_is_refused() {
        let d = dir("fifo");
        let p = d.join("f.json");
        let c = std::ffi::CString::new(p.to_string_lossy().as_bytes()).unwrap();
        // SAFETY: `c` is a valid NUL-terminated path for the call's duration.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let err = write_chosen_file(&p, b"x", false).unwrap_err();
        std::fs::remove_dir_all(&d).unwrap();
        assert!(err.to_string().contains(SAVE_TO_NON_FILE), "{err:?}");
    }
}
