#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Task #16755, `cli.allium: CliCommandsNeedAHostFile`: a one-shot command
//! needs a host file that already holds a user identity, and never writes it.

use crate::host_file::{host_file_path, HostIdentity};

/// A host file whose owner is still null (a board never connected) is no
/// identity to act as: the command refuses, telling the operator to run
/// `dispatch tui` once, and leaves the file as it was.
#[tokio::test]
async fn a_cli_store_with_no_user_identity_refuses_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = host_file_path(dir.path());
    std::fs::write(
        &path,
        serde_json::to_vec(&HostIdentity {
            host_id: "h".to_string(),
            label: Some("laptop".to_string()),
            user_identity: None,
            credential: None,
            extra: Default::default(),
        })
        .unwrap(),
    )
    .unwrap();
    let before = std::fs::read(&path).unwrap();

    let result = super::open_cli_store(dir.path(), Some("http://127.0.0.1:1".to_string())).await;

    let err = match result {
        Ok(_) => panic!("a null user identity must be refused"),
        Err(e) => format!("{e:#}"),
    };
    assert!(err.contains("dispatch tui"), "{err}");
    assert!(!err.contains("Could not connect"), "{err}");
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

const PINNED_DB: &str = "c200298fac876590c951a7e10328c408fb4664de2bd945a55a0eff22ff8e1f77";
const OTHER_DB: &str = "c200a1ada23f68e494f28f6a791a2afe105f5a0056e01e6762e3d3618a893ccb";

/// A data directory whose host file holds a user identity and whose pin names
/// PINNED_DB.
fn pinned_data_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        host_file_path(dir.path()),
        serde_json::to_vec(&HostIdentity {
            host_id: "h".to_string(),
            label: None,
            user_identity: Some("u".to_string()),
            credential: None,
            extra: Default::default(),
        })
        .unwrap(),
    )
    .unwrap();
    crate::startup::pin_store_identity(dir.path(), PINNED_DB);
    dir
}

/// `cli.allium: CliCommandsRefuseAStoreThisInstallDoesNotUse`. The refusal
/// names the stores and comes before any connection: the address is
/// unreachable, so reaching the connection would give a different error.
#[tokio::test]
async fn a_cli_store_whose_database_is_not_the_pinned_one_is_refused_before_connecting() {
    let dir = pinned_data_dir();

    let result = super::open_cli_store_checked(
        dir.path(),
        Some("http://127.0.0.1:1".to_string()),
        super::StorePin::Checked,
        |_| Some(OTHER_DB.to_string()),
    )
    .await;

    let err = match result {
        Ok(_) => panic!("a switched store must be refused"),
        Err(e) => format!("{e:#}"),
    };
    assert!(err.contains(PINNED_DB) && err.contains(OTHER_DB), "{err}");
    assert!(err.contains("--accept-store-switch"), "{err}");
    assert!(!err.contains("Could not connect"), "{err}");
}

/// The same store, but the command is exempt (`store import`): it goes on to
/// connect, and fails only because nothing answers.
#[tokio::test]
async fn an_unchecked_cli_store_skips_the_pin() {
    let dir = pinned_data_dir();

    let result = super::open_cli_store_checked(
        dir.path(),
        Some("http://127.0.0.1:1".to_string()),
        super::StorePin::Unchecked,
        |_| Some(OTHER_DB.to_string()),
    )
    .await;

    let err = match result {
        Ok(_) => panic!("nothing listens on port 1"),
        Err(e) => format!("{e:#}"),
    };
    assert!(!err.contains("--accept-store-switch"), "{err}");
}

/// A command never pins: the pin stays what a board set.
#[tokio::test]
async fn a_cli_store_check_does_not_write_the_pin() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        host_file_path(dir.path()),
        serde_json::to_vec(&HostIdentity {
            host_id: "h".to_string(),
            label: None,
            user_identity: Some("u".to_string()),
            credential: None,
            extra: Default::default(),
        })
        .unwrap(),
    )
    .unwrap();

    let _ = super::open_cli_store_checked(
        dir.path(),
        Some("http://127.0.0.1:1".to_string()),
        super::StorePin::Checked,
        |_| Some(OTHER_DB.to_string()),
    )
    .await;

    assert_eq!(crate::startup::pinned_store_identity(dir.path()), None);
}
