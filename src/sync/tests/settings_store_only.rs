//! Task #16755, settings.allium: SettingsAreStoredPerHost — "NOTHING IS
//! COPIED IN FROM THE OLD LOCAL TABLE ... a host with no rows in the store
//! starts from the defaults", and settings are read "from these rows and from
//! nowhere else" (sync.allium: SubscribeToThisHostsSettings). Task #36865 took
//! the local table away altogether, so a handle with no store has nowhere to
//! answer from or to write to.

use std::sync::Arc;

use crate::store::{SettingsStore, Store};
use crate::sync::{SharedRows, SubscriptionBoardReads};

/// A handle with no store attached refuses a settings write and a settings
/// read, rather than keeping the value somewhere nothing else reads.
#[tokio::test]
async fn a_handle_with_no_store_refuses_settings_rather_than_keeping_them_locally() {
    let local = Store::unattached();

    let write = local
        .set_setting_string("repo_filter", "/legacy/repo")
        .await;
    assert!(write.is_err(), "a write has no store to land in");
    let read = local.get_setting_string("repo_filter").await;
    assert!(read.is_err(), "a read has no store to answer from");
}

/// A store-backed handle with no row for a key answers with the default.
#[tokio::test]
async fn a_setting_the_store_does_not_hold_reads_as_unset() {
    let store_backed = Store::unattached().with_shared_reader(Arc::new(
        SubscriptionBoardReads::new(Arc::new(SharedRows::new())),
    ));

    assert_eq!(
        store_backed
            .get_setting_string("repo_filter")
            .await
            .unwrap(),
        None,
        "no row in the store means the default"
    );
    assert_eq!(
        store_backed
            .get_setting_bool("notifications")
            .await
            .unwrap(),
        None
    );
}
