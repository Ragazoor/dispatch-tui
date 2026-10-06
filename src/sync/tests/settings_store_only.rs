//! Task #16755, settings.allium: SettingsAreStoredPerHost — "NOTHING IS
//! COPIED IN FROM THE OLD LOCAL TABLE ... a host with no rows in the store
//! starts from the defaults", and settings are read "from these rows and from
//! nowhere else" (sync.allium: SubscribeToThisHostsSettings).

use std::sync::Arc;

use crate::db::{Database, SettingsStore};
use crate::sync::{SharedRows, SubscriptionBoardReads};

/// A store-backed handle whose own SQLite holds a setting the store does not:
/// the read answers from the (empty) store, i.e. the default, and the local
/// row is never consulted.
#[tokio::test]
async fn a_setting_only_in_the_local_table_is_not_read_when_the_store_has_none() {
    let local = Database::open_in_memory_unattached().await.unwrap();
    local
        .set_setting_string("repo_filter", "/legacy/repo")
        .await
        .unwrap();
    local.set_setting_bool("notifications", true).await.unwrap();

    let store_backed = local.with_shared_reader(Arc::new(SubscriptionBoardReads::new(Arc::new(
        SharedRows::new(),
    ))));

    assert_eq!(
        store_backed
            .get_setting_string("repo_filter")
            .await
            .unwrap(),
        None,
        "no row in the store means the default, never the local copy"
    );
    assert_eq!(
        store_backed
            .get_setting_bool("notifications")
            .await
            .unwrap(),
        None
    );
}
