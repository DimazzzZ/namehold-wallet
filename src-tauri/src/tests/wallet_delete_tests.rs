use std::fs;
use std::path::Path;

#[test]
fn test_get_wallet_db_path() {
    let path = crate::wallet_delete::get_wallet_db_path("/home/user/.hsd");
    assert_eq!(path, Path::new("/home/user/.hsd/wallet"));
}

#[test]
fn test_get_wallet_db_path_with_trailing_slash() {
    let path = crate::wallet_delete::get_wallet_db_path("/Volumes/WD/hsd-data/");
    assert_eq!(path, Path::new("/Volumes/WD/hsd-data/wallet"));
}

#[test]
fn test_delete_wallet_folder_nonexistent() {
    let result = crate::wallet_delete::delete_wallet_folder(Path::new("/nonexistent/path"));
    assert!(result.is_err());
}

#[test]
fn test_delete_wallet_folder_success() {
    let dir = std::env::temp_dir().join("namehold_test_delete_wallet");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("test.db"), "data").unwrap();

    let result = crate::wallet_delete::delete_wallet_folder(&dir);
    assert!(result.is_ok());

    let backup = result.unwrap();
    assert!(backup.exists());
    assert!(!dir.exists());

    let _ = fs::remove_dir_all(&backup);
}

/// When the rename itself fails (here: the source lives inside a read-only
/// parent directory, so removing the source dir-entry is denied), the error is
/// mapped to `AppError::Other("Failed to rename wallet folder: ...")` rather
/// than panicking. Exercises the `map_err` failure branch.
#[cfg(unix)]
#[test]
fn test_delete_wallet_folder_rename_failure() {
    use std::os::unix::fs::PermissionsExt;

    let parent = std::env::temp_dir().join("namehold_test_delete_wallet_ro_parent");
    let _ = fs::remove_dir_all(&parent);
    fs::create_dir_all(&parent).unwrap();
    let target = parent.join("wallet");
    fs::create_dir_all(&target).unwrap();

    // Make the parent read-only: rename can't remove `target`'s dir entry.
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o555)).unwrap();

    let result = crate::wallet_delete::delete_wallet_folder(&target);

    // Restore write perms so cleanup can succeed regardless of the outcome.
    let _ = fs::set_permissions(&parent, fs::Permissions::from_mode(0o755));
    let _ = fs::remove_dir_all(&parent);

    let err = result.expect_err("rename into a read-only parent should fail");
    match err {
        crate::error::AppError::Other(m) => {
            assert!(m.contains("Failed to rename wallet folder"), "got {m}");
        }
        other => panic!("expected AppError::Other, got {other:?}"),
    }
}
