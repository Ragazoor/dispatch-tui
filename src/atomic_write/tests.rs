use super::*;
use std::os::unix::fs::PermissionsExt;

fn mode_of(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

#[test]
fn keep_mode_preserves_the_old_files_mode() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f");
    std::fs::write(&path, "old").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    write_bytes(&path, Mode::KeepOr { default: 0o600 }, b"new").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
    assert_eq!(mode_of(&path), 0o640);
}

#[test]
fn keep_mode_uses_the_default_for_a_new_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f");
    write_bytes(&path, Mode::KeepOr { default: 0o600 }, b"new").unwrap();
    assert_eq!(mode_of(&path), 0o600);
}

#[test]
fn fixed_mode_overrides_the_old_files_mode() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f");
    std::fs::write(&path, "old").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    write_bytes(&path, Mode::Fixed(0o600), b"new").unwrap();
    assert_eq!(mode_of(&path), 0o600);
}

#[test]
fn a_failing_writer_leaves_the_original_and_no_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f");
    std::fs::write(&path, "old").unwrap();
    let result = write_with(&path, Mode::Fixed(0o600), |w| {
        w.write_all(b"partial").unwrap();
        anyhow::bail!("boom")
    });
    assert!(result.is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
    let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(entries.len(), 1, "temp file left behind");
}

#[test]
fn a_symlinked_target_stays_a_symlink_and_the_real_file_is_updated() {
    let dir = tempfile::tempdir().unwrap();
    let other = dir.path().join("other");
    std::fs::create_dir(&other).unwrap();
    let real = other.join("real");
    let link = dir.path().join("link");
    std::fs::write(&real, "old").unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    write_bytes(&link, Mode::KeepOr { default: 0o600 }, b"new").unwrap();
    assert_eq!(std::fs::read_to_string(&real).unwrap(), "new");
    assert!(std::fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
}
