use rusqlite::Connection;
use std::path::Path;

/// Create the wallet's data directory (`~/.namehold`) readable by its owner
/// only (0700 on unix), and tighten one that already exists. It holds the
/// database (encrypted seeds, the account xpubs, sealed bids' values and
/// nonces, the node's api-key), the daemon's pid and log: nothing in it is
/// for another account on the machine.
pub fn secure_data_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir)?;
    Ok(())
}

/// Make `path` readable and writable by its owner only (0600 on unix), when
/// it exists.
fn owner_only(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            r => r?,
        }
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Open the wallet database, owner-only. The file is created 0600 before
/// SQLite first opens it (SQLite gives its `-wal` and `-shm` files the main
/// file's mode), and an existing database and its journal files are
/// tightened to 0600. `secure_delete` zero-fills deleted content, so a
/// deleted wallet's encrypted seed or a spent bid's nonce does not linger in
/// the file's free pages.
pub fn open(path: &Path) -> Result<Connection, rusqlite::Error> {
    open_via(path, None)
}

/// [`open`] through the read-metering VFS, for the startup migration whose
/// progress is shown ([`crate::db::read_meter`]).
pub fn open_metered(path: &Path) -> Result<Connection, rusqlite::Error> {
    crate::db::read_meter::register()?;
    open_via(path, Some(crate::db::read_meter::VFS_NAME))
}

fn open_via(path: &Path, vfs: Option<&str>) -> Result<Connection, rusqlite::Error> {
    let io = |e: std::io::Error| {
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
            Some(format!("{}: {e}", path.display())),
        )
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        if !path.exists() && path.parent().is_some_and(|p| p.is_dir()) {
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(path)
                .map_err(io)?;
        }
    }
    for p in [
        path.to_path_buf(),
        path.with_extension("db-wal"),
        path.with_extension("db-shm"),
    ] {
        owner_only(&p).map_err(io)?;
    }
    let conn = match vfs {
        Some(vfs) => {
            Connection::open_with_flags_and_vfs(path, rusqlite::OpenFlags::default(), vfs)?
        }
        None => Connection::open(path)?,
    };
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;
        PRAGMA busy_timeout = 5000;
        PRAGMA secure_delete = ON;
        ",
    )?;
    Ok(conn)
}

/// Push everything in the write-ahead log into the database file and empty
/// the log, so content deleted a moment ago (zero-filled in the file by
/// `secure_delete`) does not stay readable in the log's older frames. Run
/// after deleting a wallet. Best effort: a reader holding an old snapshot
/// can keep the log from being emptied, and then the next checkpoint does it.
pub fn checkpoint_truncate(conn: &Connection) {
    // Why ignored: a busy checkpoint is not an error for the delete that
    // already committed; SQLite checkpoints again on its own.
    let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
}

/// [`open`] plus migrations: the connection a background job (sync step,
/// daemon, scan) opens for itself, hardened like the app's main one.
pub fn open_migrated(db_path: &str) -> Result<Connection, crate::error::AppError> {
    let conn = open(Path::new(db_path))?;
    crate::db::migrations::run(&conn)?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn mode(p: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("namehold_conn_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// A fresh data dir is 0700 and a fresh database (and the WAL SQLite
    /// creates beside it) 0600, whatever the process umask.
    #[cfg(unix)]
    #[test]
    fn a_new_data_dir_and_database_are_owner_only() {
        let dir = scratch("new").join(".namehold");
        secure_data_dir(&dir).unwrap();
        assert_eq!(mode(&dir), 0o700);
        let db = dir.join("portfolio.db");
        let conn = open(&db).unwrap();
        conn.execute_batch("CREATE TABLE t (x); INSERT INTO t VALUES (1);")
            .unwrap();
        assert_eq!(mode(&db), 0o600);
        let wal = db.with_extension("db-wal");
        assert!(wal.exists(), "WAL mode writes a -wal file");
        assert_eq!(mode(&wal), 0o600);
        drop(conn);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    /// An install from before the fix (0755 dir, 0644 files) is tightened.
    #[cfg(unix)]
    #[test]
    fn an_existing_world_readable_data_dir_and_database_are_tightened() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("old").join(".namehold");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let db = dir.join("portfolio.db");
        drop(Connection::open(&db).unwrap());
        std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o644)).unwrap();
        let wal = db.with_extension("db-wal");
        std::fs::write(&wal, b"").unwrap();
        std::fs::set_permissions(&wal, std::fs::Permissions::from_mode(0o644)).unwrap();

        secure_data_dir(&dir).unwrap();
        drop(open(&db).unwrap());
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&db), 0o600);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn secure_delete_is_on() {
        let dir = scratch("sd");
        secure_data_dir(&dir).unwrap();
        let conn = open(&dir.join("t.db")).unwrap();
        let on: i64 = conn
            .pragma_query_value(None, "secure_delete", |r| r.get(0))
            .unwrap();
        assert_eq!(on, 1);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A deleted secret leaves no copy in the database file or its WAL once
    /// the delete is checkpointed.
    #[test]
    fn a_deleted_secret_is_not_left_in_the_file_or_its_log() {
        let dir = scratch("residue");
        secure_data_dir(&dir).unwrap();
        let db = dir.join("t.db");
        let conn = open(&db).unwrap();
        let marker = "SEEDBLOBMARKER".repeat(40);
        conn.execute_batch("CREATE TABLE s (v TEXT);").unwrap();
        conn.execute("INSERT INTO s VALUES (?1)", [&marker])
            .unwrap();
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .unwrap();
        conn.execute("DELETE FROM s", []).unwrap();
        checkpoint_truncate(&conn);
        let mut bytes = std::fs::read(&db).unwrap();
        if let Ok(w) = std::fs::read(db.with_extension("db-wal")) {
            bytes.extend(w);
        }
        let needle = b"SEEDBLOBMARKER";
        assert!(
            !bytes.windows(needle.len()).any(|w| w == needle),
            "the deleted secret is still readable in the file or its log"
        );
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_creates_sqlite_file_with_wal() {
        let dir = std::env::temp_dir().join("namehold_test_conn");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.db");
        let _ = std::fs::remove_file(&path);

        let conn = open(&path).expect("open should succeed");
        let mode: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
        let fk: i64 = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert_eq!(fk, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
