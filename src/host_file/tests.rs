#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Task #16755: the host file. Spec: `docs/specs/host.allium` (`HostFile`,
//! `HostFileIsPrivate`, `IdentityLivesInHostFile`, `HostFileIsWrittenWhole`,
//! `MintHostIdentity`, `RenameHost`, `AdoptUserIdentity`), `startup.allium`
//! (`AbortWhenTheHostIdentityStoreIsUnusable`) and `cli.allium`
//! (`CliCommandsNeedAHostFile`).

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use super::*;

fn mode_of(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn stored(dir: &Path) -> HostIdentity {
    serde_json::from_slice(&std::fs::read(host_file_path(dir)).unwrap())
        .expect("host.json must hold a parseable identity")
}

fn write_raw(dir: &Path, identity: &HostIdentity, mode: u32) {
    let path = host_file_path(dir);
    std::fs::write(&path, serde_json::to_vec(identity).unwrap()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
}

fn named(id: &str, label: &str) -> HostIdentity {
    HostIdentity {
        host_id: id.to_string(),
        label: Some(label.to_string()),
        user_identity: None,
        credential: None,
    }
}

/// Restores a path's permissions on drop, so a panicking assertion cannot
/// leave a directory `TempDir` cannot clean (docs/testing.md).
struct PermGuard(PathBuf, u32);

impl Drop for PermGuard {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(self.1));
    }
}

/// chmod `path` to `mode`, or skip when this process ignores permissions
/// (root). Refuses to skip in CI, where a skip would report green while
/// covering nothing — the `deny_access_or_skip` rule in
/// `src/dispatch/tests/agent_launch.rs`.
fn deny_or_skip(
    path: &Path,
    mode: u32,
    restore: u32,
    still_works: impl Fn() -> bool,
) -> Option<PermGuard> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    let guard = PermGuard(path.to_path_buf(), restore);
    if still_works() {
        assert!(
            std::env::var_os("CI").is_none(),
            "this process is not bound by file permissions (root?), so the \
             unusable-host-file tests cannot be staged. Refusing to skip in CI."
        );
        eprintln!("skipping: this process is not bound by file permissions");
        return None;
    }
    Some(guard)
}

// -- MintHostIdentity -------------------------------------------------------

/// A board launch with no host file is a first run: a fresh id, a null label,
/// and host.json written with the private mode.
#[test]
fn a_first_run_mints_a_host_file_with_a_null_label() {
    let dir = tempfile::tempdir().unwrap();

    let identity = resolve_for_launch(dir.path()).expect("a first run must mint an identity");

    assert!(
        !identity.host_id.trim().is_empty(),
        "a minted id is never blank"
    );
    assert_eq!(identity.label, None, "a minted host is unnamed");
    assert_eq!(identity.user_identity, None);
    assert_eq!(identity.credential, None);
    let path = host_file_path(dir.path());
    assert_eq!(path, dir.path().join("host.json"));
    assert_eq!(mode_of(&path), HOST_FILE_MODE, "HostFileIsPrivate");
    assert_eq!(
        stored(dir.path()),
        identity,
        "what was returned is what was written"
    );
}

/// MintHostIdentity's `requires: not host_file_present()`: an install whose
/// host file exists keeps the id it has.
#[test]
fn a_later_launch_reads_back_the_identity_it_minted() {
    let dir = tempfile::tempdir().unwrap();
    let first = resolve_for_launch(dir.path()).unwrap();

    let second = resolve_for_launch(dir.path()).unwrap();

    assert_eq!(
        second.host_id, first.host_id,
        "a second launch must never re-mint"
    );
}

/// A named host file is read as it is, and is not rewritten by reading it.
#[test]
fn a_launch_with_a_host_file_returns_it_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    write_raw(dir.path(), &named("host-abc", "laptop"), HOST_FILE_MODE);
    let before = std::fs::read(host_file_path(dir.path())).unwrap();

    let identity = resolve_for_launch(dir.path()).unwrap();

    assert_eq!(identity, named("host-abc", "laptop"));
    assert_eq!(std::fs::read(host_file_path(dir.path())).unwrap(), before);
}

/// A host file that will not parse mints nothing and is not overwritten: the
/// launch aborts with host_identity_unavailable.
#[test]
fn an_unparseable_host_file_is_not_overwritten_and_aborts_the_launch() {
    let dir = tempfile::tempdir().unwrap();
    let path = host_file_path(dir.path());
    std::fs::write(&path, b"{ this is not an identity").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(HOST_FILE_MODE)).unwrap();

    let result = resolve_for_launch(dir.path());

    assert_eq!(result, Err(StartupAbort::HostIdentityUnavailable));
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"{ this is not an identity",
        "a damaged host file must never be overwritten with a new identity"
    );
}

/// A host file that exists but cannot be read is treated the same way.
#[test]
fn an_unreadable_host_file_is_not_overwritten_and_aborts_the_launch() {
    let dir = tempfile::tempdir().unwrap();
    write_raw(dir.path(), &named("host-keep", "laptop"), HOST_FILE_MODE);
    let path = host_file_path(dir.path());
    let before = std::fs::read(&path).unwrap();
    let probe = path.clone();
    let Some(guard) = deny_or_skip(&path, 0o000, HOST_FILE_MODE, || {
        std::fs::read(&probe).is_ok()
    }) else {
        return;
    };

    let result = resolve_for_launch(dir.path());

    drop(guard);
    assert_eq!(result, Err(StartupAbort::HostIdentityUnavailable));
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "the unreadable file is left alone"
    );
}

/// A first run whose host file cannot be written aborts rather than run with
/// an identity nothing recorded.
#[test]
fn a_first_run_that_cannot_write_its_host_file_aborts_the_launch() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("dispatch");
    std::fs::create_dir(&data_dir).unwrap();
    let probe = data_dir.clone();
    let Some(guard) = deny_or_skip(&data_dir, 0o500, 0o700, || {
        std::fs::write(probe.join(".probe"), b"").is_ok()
    }) else {
        return;
    };

    let result = resolve_for_launch(&data_dir);

    drop(guard);
    assert_eq!(result, Err(StartupAbort::HostIdentityUnavailable));
    assert!(!host_file_path(&data_dir).exists(), "nothing was written");
}

/// The abort names the host file as what to repair, not a database.
#[test]
fn host_identity_unavailable_names_the_host_file_not_a_database() {
    let message = StartupAbort::HostIdentityUnavailable.message();

    assert!(
        message.contains("host.json"),
        "the remedy is the host file, so the message must name it: {message}"
    );
    assert!(
        !message.contains("database"),
        "no launch opens a database any more (storage.allium: \
         StoreInUseNeverOpensSqlite), so the message must not send the \
         operator to one: {message}"
    );
}

// -- RenameHost ---------------------------------------------------------------

#[test]
fn rename_host_writes_the_label_and_keeps_the_id() {
    let dir = tempfile::tempdir().unwrap();
    let minted = resolve_for_launch(dir.path()).unwrap();

    let renamed = rename_host(dir.path(), "my-laptop").unwrap();

    assert_eq!(
        renamed.host_id, minted.host_id,
        "renaming never touches the id"
    );
    assert_eq!(renamed.label.as_deref(), Some("my-laptop"));
    assert_eq!(stored(dir.path()), renamed);
    assert_eq!(
        resolve_for_launch(dir.path()).unwrap().label.as_deref(),
        Some("my-laptop")
    );
}

#[test]
fn rename_host_rejects_a_blank_label_and_leaves_the_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    write_raw(dir.path(), &named("host-abc", "laptop"), HOST_FILE_MODE);
    let before = std::fs::read(host_file_path(dir.path())).unwrap();

    assert!(rename_host(dir.path(), "   ").is_err());

    assert_eq!(std::fs::read(host_file_path(dir.path())).unwrap(), before);
}

// -- AdoptUserIdentity ----------------------------------------------------------

/// The owner and the credential the store handed out with it are written
/// together, so the file never holds one without the other.
#[test]
fn adopting_a_user_identity_writes_it_and_its_credential_together() {
    let dir = tempfile::tempdir().unwrap();
    write_raw(dir.path(), &named("host-abc", "laptop"), HOST_FILE_MODE);

    let adopted = adopt_user_identity(dir.path(), "c0ffee", "secret-token").unwrap();

    let expected = HostIdentity {
        host_id: "host-abc".to_string(),
        label: Some("laptop".to_string()),
        user_identity: Some("c0ffee".to_string()),
        credential: Some("secret-token".to_string()),
    };
    assert_eq!(adopted, expected);
    assert_eq!(stored(dir.path()), expected);
}

/// `core.allium: LocalHostOwnerIsWrittenOnce`: a second adoption with a
/// different owner leaves the stored owner alone; the credential is refreshed.
#[test]
fn adopting_never_overwrites_an_existing_owner() {
    let dir = tempfile::tempdir().unwrap();
    let mut owned = named("host-abc", "laptop");
    owned.user_identity = Some("first-owner".to_string());
    owned.credential = Some("old-token".to_string());
    write_raw(dir.path(), &owned, HOST_FILE_MODE);

    let adopted = adopt_user_identity(dir.path(), "someone-else", "new-token").unwrap();

    assert_eq!(adopted.user_identity.as_deref(), Some("first-owner"));
    assert_eq!(adopted.credential.as_deref(), Some("new-token"));
    assert_eq!(stored(dir.path()), adopted);
}

// -- HostFileIsWrittenWhole / HostFileIsPrivate --------------------------------

/// Every write replaces the file (a new inode, as a temp-file-and-rename
/// does), leaves no temporary file behind, and lands with the private mode —
/// even over a file someone loosened to 0644.
#[test]
fn every_write_replaces_the_whole_file_with_the_private_mode() {
    let dir = tempfile::tempdir().unwrap();
    write_raw(dir.path(), &named("host-abc", "laptop"), 0o644);
    let path = host_file_path(dir.path());
    let inode_before = std::fs::metadata(&path).unwrap().ino();

    rename_host(dir.path(), "desktop").unwrap();

    assert_ne!(
        std::fs::metadata(&path).unwrap().ino(),
        inode_before,
        "a whole-file replacement renames a new file over the old one rather \
         than rewriting it in place"
    );
    assert_eq!(mode_of(&path), HOST_FILE_MODE, "HostFileIsPrivate");
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(
        names,
        vec!["host.json".to_string()],
        "no temporary file is left behind"
    );

    adopt_user_identity(dir.path(), "c0ffee", "secret-token").unwrap();
    assert_eq!(
        mode_of(&path),
        HOST_FILE_MODE,
        "the credential write is private too"
    );
}

/// The file holds identity only — never a setting (settings.allium:
/// SettingsAreStoredPerHost).
#[test]
fn the_host_file_holds_identity_fields_only() {
    let dir = tempfile::tempdir().unwrap();
    resolve_for_launch(dir.path()).unwrap();
    rename_host(dir.path(), "laptop").unwrap();
    adopt_user_identity(dir.path(), "c0ffee", "secret-token").unwrap();

    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(host_file_path(dir.path())).unwrap()).unwrap();
    let keys: Vec<&String> = json.as_object().expect("an object").keys().collect();
    assert_eq!(
        keys.len(),
        4,
        "host id, label, user identity, credential and nothing else: {keys:?}"
    );
}

// -- CliCommandsNeedAHostFile ------------------------------------------------------

/// A one-shot command with no host file fails telling the operator to run
/// `dispatch tui`, and mints nothing.
#[test]
fn a_cli_read_with_no_host_file_fails_and_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();

    let err = read_for_cli(dir.path()).expect_err("no host file means no identity");

    assert!(
        format!("{err:#}").contains("dispatch tui"),
        "the failure must say one `dispatch tui` creates the identity: {err:#}"
    );
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "nothing is created"
    );
}

#[test]
fn a_cli_read_returns_the_stored_identity_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    write_raw(dir.path(), &named("host-abc", "laptop"), HOST_FILE_MODE);
    let path = host_file_path(dir.path());
    let before = (
        std::fs::read(&path).unwrap(),
        std::fs::metadata(&path).unwrap().ino(),
    );

    let identity = read_for_cli(dir.path()).unwrap();

    assert_eq!(identity, named("host-abc", "laptop"));
    assert_eq!(
        (
            std::fs::read(&path).unwrap(),
            std::fs::metadata(&path).unwrap().ino()
        ),
        before,
        "a one-shot command never writes the host file"
    );
}
