//! A SQLite VFS that counts the bytes SQLite reads, so a long migration can
//! show how far it has got.
//!
//! A single statement such as `DROP TABLE` on a 9 GB table runs as one VDBE
//! opcode, so SQLite's progress handler never fires during it. What does move
//! is reading: dropping a table visits every one of its pages. This VFS is the
//! process's default VFS with `xRead` wrapped to add up the bytes read; the
//! migration connection opens through it ([`VFS_NAME`]) and the startup screen
//! compares [`bytes_read`] against the file's size.
//!
//! Every other method is the default VFS's own. The only ones that read the
//! VFS's `pAppData` (unix's locking-style finder, Windows' app data) run inside
//! `xOpen`, which is forwarded with the default VFS itself, so copying the rest
//! of the struct is sound.

use rusqlite::ffi;
use std::ffi::{c_int, c_void};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// The name the metered VFS is registered under.
pub const VFS_NAME: &str = "namehold-read-meter";
const VFS_NAME_C: &std::ffi::CStr = c"namehold-read-meter";

static BYTES_READ: AtomicU64 = AtomicU64::new(0);

/// Bytes read through the metered VFS since the process started.
pub fn bytes_read() -> u64 {
    BYTES_READ.load(Ordering::Relaxed)
}

/// An open file's method table: a copy of the real one with `xRead` and
/// `xClose` replaced, followed by the real table they forward to.
#[repr(C)]
struct MeteredMethods {
    methods: ffi::sqlite3_io_methods,
    real: *const ffi::sqlite3_io_methods,
}

unsafe extern "C" fn metered_read(
    file: *mut ffi::sqlite3_file,
    buf: *mut c_void,
    amount: c_int,
    offset: ffi::sqlite3_int64,
) -> c_int {
    // SAFETY: `metered_open` installed a `MeteredMethods` as this file's
    // method table, and `methods` is its first field.
    let real = unsafe { (*((*file).pMethods as *const MeteredMethods)).real };
    let Some(read) = (unsafe { (*real).xRead }) else {
        return ffi::SQLITE_IOERR_READ;
    };
    let rc = unsafe { read(file, buf, amount, offset) };
    if rc == ffi::SQLITE_OK || rc == ffi::SQLITE_IOERR_SHORT_READ {
        BYTES_READ.fetch_add(u64::try_from(amount).unwrap_or(0), Ordering::Relaxed);
    }
    rc
}

unsafe extern "C" fn metered_close(file: *mut ffi::sqlite3_file) -> c_int {
    // SAFETY: as in `metered_read`. The box was leaked by `metered_open` and is
    // reclaimed exactly once, here: SQLite closes a file once and then clears
    // its method table.
    let metered = unsafe { (*file).pMethods as *mut MeteredMethods };
    let real = unsafe { (*metered).real };
    unsafe { (*file).pMethods = real };
    drop(unsafe { Box::from_raw(metered) });
    match unsafe { (*real).xClose } {
        Some(close) => unsafe { close(file) },
        None => ffi::SQLITE_OK,
    }
}

unsafe extern "C" fn metered_open(
    vfs: *mut ffi::sqlite3_vfs,
    name: ffi::sqlite3_filename,
    file: *mut ffi::sqlite3_file,
    flags: c_int,
    out_flags: *mut c_int,
) -> c_int {
    // SAFETY: `register` stored the default VFS in `pAppData`, and SQLite
    // hands `xOpen` a buffer of the default VFS's `szOsFile`, which this VFS
    // copied unchanged.
    let real_vfs = unsafe { (*vfs).pAppData as *mut ffi::sqlite3_vfs };
    let Some(open) = (unsafe { (*real_vfs).xOpen }) else {
        return ffi::SQLITE_CANTOPEN;
    };
    let rc = unsafe { open(real_vfs, name, file, flags, out_flags) };
    // SQLite calls xClose whenever the method table is set, even after a
    // failed open, so wrap it whenever it is set.
    let real = unsafe { (*file).pMethods };
    if !real.is_null() {
        let mut methods = unsafe { *real };
        methods.xRead = Some(metered_read);
        methods.xClose = Some(metered_close);
        let metered = Box::into_raw(Box::new(MeteredMethods { methods, real }));
        unsafe { (*file).pMethods = metered as *const ffi::sqlite3_io_methods };
    }
    rc
}

/// Register the metered VFS (once per process). It is not made the default:
/// only a connection that names [`VFS_NAME`] uses it.
pub fn register() -> Result<(), rusqlite::Error> {
    static RC: OnceLock<c_int> = OnceLock::new();
    let rc = *RC.get_or_init(|| {
        // SAFETY: `sqlite3_vfs_find` initialises SQLite and returns the
        // default VFS, which lives for the whole process. The copy is leaked
        // on purpose: a registered VFS must outlive every connection.
        unsafe {
            let real = ffi::sqlite3_vfs_find(std::ptr::null());
            if real.is_null() {
                return ffi::SQLITE_ERROR;
            }
            let mut vfs = *real;
            vfs.pNext = std::ptr::null_mut();
            vfs.zName = VFS_NAME_C.as_ptr();
            vfs.pAppData = real as *mut c_void;
            vfs.xOpen = Some(metered_open);
            ffi::sqlite3_vfs_register(Box::into_raw(Box::new(vfs)), 0)
        }
    });
    if rc == ffi::SQLITE_OK {
        Ok(())
    } else {
        Err(rusqlite::Error::SqliteFailure(
            ffi::Error::new(rc),
            Some("could not register the read-metering VFS".into()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, OpenFlags};

    fn metered(path: &std::path::Path) -> Connection {
        register().unwrap();
        Connection::open_with_flags_and_vfs(path, OpenFlags::default(), VFS_NAME).unwrap()
    }

    #[test]
    fn counts_what_a_drop_reads_and_keeps_the_file_usable() {
        let dir = std::env::temp_dir().join(format!("namehold_read_meter_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.db");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "PRAGMA journal_mode = WAL;
                 CREATE TABLE big (id INTEGER PRIMARY KEY, v TEXT);
                 CREATE TABLE keep (v TEXT);
                 INSERT INTO keep VALUES ('still here');",
            )
            .unwrap();
            let tx = c.unchecked_transaction().unwrap();
            for i in 0..20_000 {
                tx.execute("INSERT INTO big (v) VALUES (?1)", [format!("{i:0>200}")])
                    .unwrap();
            }
            tx.commit().unwrap();
            c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
        }
        let file_size = std::fs::metadata(&path).unwrap().len();

        let c = metered(&path);
        let before = bytes_read();
        c.execute_batch("DROP TABLE big;").unwrap();
        let read = bytes_read() - before;
        // Dropping visits every page of the table: most of the file.
        assert!(
            read > file_size / 2,
            "read {read} of a {file_size}-byte file"
        );

        let kept: String = c.query_row("SELECT v FROM keep", [], |r| r.get(0)).unwrap();
        assert_eq!(kept, "still here");
        drop(c);

        // The default VFS still opens the file the metered one wrote.
        let plain = Connection::open(&path).unwrap();
        let tables: i64 = plain
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'big'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 0);
        drop(plain);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn registering_twice_is_fine() {
        register().unwrap();
        register().unwrap();
    }
}
