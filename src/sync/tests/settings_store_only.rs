//! Task #16755, settings.allium: SettingsAreStoredPerHost — "NOTHING IS
//! COPIED IN FROM THE OLD LOCAL TABLE ... a host with no rows in the store
//! starts from the defaults", and settings are read "from these rows and from
//! nowhere else" (sync.allium: SubscribeToThisHostsSettings).

use crate::store::{SettingsStore, Store};

/// A store with no row for a key answers with the default.
#[tokio::test]
async fn a_setting_the_store_does_not_hold_reads_as_unset() {
    let store_backed = Store::open_in_memory().await.unwrap();

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
