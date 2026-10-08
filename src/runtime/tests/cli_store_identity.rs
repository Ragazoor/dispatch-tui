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
        })
        .unwrap(),
    )
    .unwrap();
    let before = std::fs::read(&path).unwrap();

    let result =
        crate::runtime::open_cli_store(dir.path(), Some("http://127.0.0.1:1".to_string())).await;

    let err = match result {
        Ok(_) => panic!("a null user identity must be refused"),
        Err(e) => format!("{e:#}"),
    };
    assert!(err.contains("dispatch tui"), "{err}");
    assert!(!err.contains("Could not connect"), "{err}");
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
