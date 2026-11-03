//! Writing or reading a file at a path the renderer names (a native save or
//! open dialog's answer). The path comes from the webview, which SECURITY.md
//! §1 treats as possibly compromised, so both refuse what a dialog would
//! never hand back for a document: a symbolic link (whose target could be any
//! file of the user's) and anything that is not a regular file; a read is
//! also capped in size.

use std::path::Path;

use crate::error::AppError;

/// Why a save is refused when its path is a symbolic link.
pub(crate) const SAVE_TO_SYMLINK: &str =
    "the chosen file is a symbolic link: choose a regular file or a new name";

/// Why a save is refused when its path is not a regular file.
pub(crate) const SAVE_TO_NON_FILE: &str =
    "the chosen path is not a regular file: choose a regular file or a new name";

/// Why an import is refused when its path is a symbolic link.
pub(crate) const READ_FROM_SYMLINK: &str =
    "the chosen file is a symbolic link: choose the file itself";

/// Why an import is refused when its path is not a regular file.
pub(crate) const READ_NON_FILE: &str = "the chosen path is not a regular file";

/// The largest file an import reads: a watchlist, a name list or an account
/// history is far smaller.
pub(crate) const MAX_IMPORT_BYTES: u64 = 8 * 1024 * 1024;

/// Read the text file at `path` the user chose to import. A symbolic link and
/// anything that is not a regular file are refused (on unix the open does
/// not follow a link swapped in after the check, `O_NOFOLLOW`), as is a file
/// over [`MAX_IMPORT_BYTES`] or one that is not UTF-8 text. Callers persist
/// only rows that validate (a Handshake name) and never echo a refused row's
/// text, so a compromised webview cannot use an import to read other files.
pub(crate) fn read_chosen_file(path: &Path) -> Result<String, AppError> {
    use std::io::Read;
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => {
            return Err(AppError::InvalidInput(READ_FROM_SYMLINK.into()))
        }
        Ok(m) if !m.is_file() => return Err(AppError::InvalidInput(READ_NON_FILE.into())),
        Ok(_) => {}
        Err(e) => return Err(e.into()),
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW);
    }
    let file = opts.open(path).map_err(|e| {
        #[cfg(unix)]
        if e.raw_os_error() == Some(libc::ELOOP) {
            return AppError::InvalidInput(READ_FROM_SYMLINK.into());
        }
        AppError::from(e)
    })?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(AppError::InvalidInput(READ_NON_FILE.into()));
    }
    if meta.len() > MAX_IMPORT_BYTES {
        return Err(AppError::InvalidInput(format!(
            "the chosen file is larger than {} MiB; nothing was imported",
            MAX_IMPORT_BYTES / (1024 * 1024)
        )));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(MAX_IMPORT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_IMPORT_BYTES {
        return Err(AppError::InvalidInput(format!(
            "the chosen file is larger than {} MiB; nothing was imported",
            MAX_IMPORT_BYTES / (1024 * 1024)
        )));
    }
    String::from_utf8(bytes).map_err(|_| {
        AppError::InvalidInput("the chosen file is not text; nothing was imported".into())
    })
}

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

    /// A device opens for writing, so the post-open check is what refuses it.
    #[cfg(unix)]
    #[test]
    fn the_open_refuses_a_device_on_its_own() {
        let err = open_regular(Path::new("/dev/null"), false).unwrap_err();
        assert!(err.to_string().contains(SAVE_TO_NON_FILE), "{err:?}");
    }

    #[test]
    fn reads_a_chosen_text_file() {
        let d = dir("read");
        let p = d.join("w.csv");
        std::fs::write(&p, "name\nalice\n").unwrap();
        let got = read_chosen_file(&p).unwrap();
        std::fs::remove_dir_all(&d).unwrap();
        assert_eq!(got, "name\nalice\n");
    }

    /// An import through a link would read whatever file it points at.
    #[cfg(unix)]
    #[test]
    fn a_read_refuses_a_symlink() {
        let d = dir("readlink");
        let target = d.join("hsd.conf");
        std::fs::write(&target, "api-key: secret").unwrap();
        let link = d.join("w.csv");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let err = read_chosen_file(&link).unwrap_err();
        std::fs::remove_dir_all(&d).unwrap();
        assert!(err.to_string().contains(READ_FROM_SYMLINK), "{err:?}");
        assert!(!err.to_string().contains("secret"));
    }

    #[test]
    fn a_read_refuses_a_directory_a_huge_file_and_binary() {
        let d = dir("readbad");
        let err = read_chosen_file(&d).unwrap_err();
        assert!(err.to_string().contains(READ_NON_FILE), "{err:?}");
        let big = d.join("big.csv");
        let f = std::fs::File::create(&big).unwrap();
        f.set_len(MAX_IMPORT_BYTES + 1).unwrap();
        let err = read_chosen_file(&big).unwrap_err();
        assert!(err.to_string().contains("larger than 8 MiB"), "{err:?}");
        let bin = d.join("bin.csv");
        std::fs::write(&bin, [0xffu8, 0xfe, 0x00]).unwrap();
        let err = read_chosen_file(&bin).unwrap_err();
        std::fs::remove_dir_all(&d).unwrap();
        assert!(err.to_string().contains("not text"), "{err:?}");
    }

    /// A lookup error other than "not found" is not a free pass.
    #[test]
    fn a_path_under_a_regular_file_is_refused_by_the_check() {
        let d = dir("notdir");
        let parent = d.join("plain.txt");
        std::fs::write(&parent, "keep").unwrap();
        let res = refuse_unless_regular(&parent.join("child.json"));
        let kept = std::fs::read_to_string(&parent).unwrap();
        std::fs::remove_dir_all(&d).unwrap();
        let err = res.unwrap_err();
        assert!(!err.to_string().contains(SAVE_TO_SYMLINK), "{err:?}");
        assert!(!err.to_string().contains(SAVE_TO_NON_FILE), "{err:?}");
        assert_eq!(kept, "keep");
    }
}
