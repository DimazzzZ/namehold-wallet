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
