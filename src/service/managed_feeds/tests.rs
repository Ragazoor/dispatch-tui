use super::*;
use crate::models::{Epic, FeedRole, MIN_FEED_INTERVAL_SECS};
use crate::store::{EpicCrud, EpicPatch, EpicRead, SettingsStore, Store};

const REVIEWS: &str = "/scripts/fetch-reviews.sh";
const CVE: &str = "/scripts/fetch-cve.sh";

async fn ensure(db: &Store) {
    db.ensure_managed_epics(Some(REVIEWS), Some(300), Some(CVE), Some(900))
        .await
        .unwrap();
}

fn by_role(epics: &[Epic], role: FeedRole) -> Vec<&Epic> {
    epics.iter().filter(|e| e.feed_role == role).collect()
}

#[tokio::test]
async fn ensure_managed_epics_creates_tree() {
    let db = Store::open_in_memory().unwrap();
    ensure(&db).await;
    let epics = db.list_epics().await;

    for role in [
        FeedRole::ReviewsParent,
        FeedRole::MyReviews,
        FeedRole::TeamReviews,
        FeedRole::Bots,
        FeedRole::Cve,
    ] {
        assert_eq!(
            by_role(&epics, role).len(),
            1,
            "exactly one epic for role {role}"
        );
    }

    let parent = by_role(&epics, FeedRole::ReviewsParent)[0];
    assert_eq!(parent.parent_epic_id, None, "reviews_parent is a root epic");
    assert_eq!(parent.feed_command.as_deref(), Some(REVIEWS));
    assert_eq!(parent.feed_interval_secs, Some(300));

    for role in [FeedRole::MyReviews, FeedRole::TeamReviews, FeedRole::Bots] {
        let sub = by_role(&epics, role)[0];
        assert_eq!(
            sub.parent_epic_id,
            Some(parent.id),
            "{role} parented to reviews_parent"
        );
        assert_eq!(
            sub.feed_command, None,
            "{role} sub-epic carries no feed_command"
        );
    }

    let cve = by_role(&epics, FeedRole::Cve)[0];
    assert_eq!(cve.parent_epic_id, None, "cve is a root epic");
    assert_eq!(cve.feed_command.as_deref(), Some(CVE));
    assert_eq!(cve.feed_interval_secs, Some(900));
}

#[tokio::test]
async fn ensure_is_idempotent() {
    let db = Store::open_in_memory().unwrap();
    ensure(&db).await;
    ensure(&db).await;
    let epics = db.list_epics().await;
    assert_eq!(
        epics.len(),
        5,
        "two ensures must create no duplicate managed epics"
    );
}

#[tokio::test]
async fn ensure_preserves_user_rename() {
    let db = Store::open_in_memory().unwrap();
    ensure(&db).await;
    let my_id = by_role(&db.list_epics().await, FeedRole::MyReviews)[0].id;
    db.patch_epic(my_id, &EpicPatch::new().title("My PRs"))
        .await
        .unwrap();

    ensure(&db).await;

    let epics = db.list_epics().await;
    let my = by_role(&epics, FeedRole::MyReviews);
    assert_eq!(my.len(), 1, "rename must not spawn a duplicate");
    assert_eq!(my[0].id, my_id);
    assert_eq!(my[0].title, "My PRs", "user rename is preserved");
}

/// Deleting a managed epic is the only way it goes away (task #4971), and
/// the next `ensure` re-provisions it — a fresh row, not a resurrection of
/// the old one — for as long as its command (here, the reviews root's)
/// stays configured.
#[tokio::test]
async fn ensure_recreates_a_deleted_managed_epic() {
    let db = Store::open_in_memory().unwrap();
    ensure(&db).await;
    let bots_id = by_role(&db.list_epics().await, FeedRole::Bots)[0].id;
    db.delete_epic(bots_id).await.unwrap();

    ensure(&db).await;

    let epics = db.list_epics().await;
    let bots = by_role(&epics, FeedRole::Bots);
    assert_eq!(bots.len(), 1, "the role must be re-provisioned");
    assert_ne!(
        bots[0].id, bots_id,
        "must be a fresh epic, not the deleted one"
    );
}

#[tokio::test]
async fn provision_from_settings_is_noop_without_config() {
    let db = Store::open_in_memory().unwrap();
    provision_managed_feeds_from_settings(&db).await.unwrap();
    assert!(
        db.list_epics().await.is_empty(),
        "no config -> no managed epics"
    );
}

#[tokio::test]
async fn provision_from_settings_creates_tree() {
    let db = Store::open_in_memory().unwrap();
    db.set_reviews_feed_command(Some(REVIEWS)).await.unwrap();
    db.set_reviews_feed_interval_secs(Some(300)).await.unwrap();
    db.set_cve_feed_command(Some(CVE)).await.unwrap();
    provision_managed_feeds_from_settings(&db).await.unwrap();

    let epics = db.list_epics().await;
    assert_eq!(
        epics.len(),
        5,
        "settings-driven provisioning builds the tree"
    );
    let parent = by_role(&epics, FeedRole::ReviewsParent)[0];
    assert_eq!(parent.feed_command.as_deref(), Some(REVIEWS));
}

#[tokio::test]
async fn write_managed_feed_settings_updates_only_provided_fields() {
    let db = Store::open_in_memory().unwrap();
    db.set_cve_feed_command(Some("/existing.sh")).await.unwrap();

    write_managed_feed_settings(
        &db,
        ManagedFeedSettingsPatch {
            reviews_command: Some(Some(REVIEWS.to_string())),
            reviews_interval_secs: Some(Some(300)),
            cve_command: None,
            cve_interval_secs: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(
        db.get_reviews_feed_command().await.unwrap().as_deref(),
        Some(REVIEWS)
    );
    assert_eq!(
        db.get_reviews_feed_interval_secs().await.unwrap(),
        Some(300)
    );
    assert_eq!(
        db.get_cve_feed_command().await.unwrap().as_deref(),
        Some("/existing.sh"),
        "absent field must be left unchanged"
    );
}

#[tokio::test]
async fn write_managed_feed_settings_clears_on_explicit_none() {
    let db = Store::open_in_memory().unwrap();
    db.set_reviews_feed_command(Some(REVIEWS)).await.unwrap();

    write_managed_feed_settings(
        &db,
        ManagedFeedSettingsPatch {
            reviews_command: Some(None),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert!(
        db.get_reviews_feed_command().await.unwrap().is_none(),
        "Some(None) must clear the setting"
    );
}

// --- the feed-cadence floor (core.allium: "Interval literals", CLAIM 2) ---

/// These settings are not a separate kind of cadence: they are copied onto
/// the managed epics' `feed_interval_secs`, so a value the floor refuses on
/// an epic must not be reachable by writing it here instead. `0` used to be
/// blessed here as "poll every tick".
#[tokio::test]
async fn write_managed_feed_settings_rejects_a_sub_floor_interval() {
    for bad in [0, -5, MIN_FEED_INTERVAL_SECS - 1] {
        let db = Store::open_in_memory().unwrap();

        let reviews = write_managed_feed_settings(
            &db,
            ManagedFeedSettingsPatch {
                reviews_interval_secs: Some(Some(bad)),
                ..Default::default()
            },
        )
        .await;
        assert!(
            matches!(reviews, Err(ServiceError::Validation(_))),
            "reviews_interval_secs = {bad} should be rejected, got {reviews:?}"
        );

        let cve = write_managed_feed_settings(
            &db,
            ManagedFeedSettingsPatch {
                cve_interval_secs: Some(Some(bad)),
                ..Default::default()
            },
        )
        .await;
        assert!(
            matches!(cve, Err(ServiceError::Validation(_))),
            "cve_interval_secs = {bad} should be rejected, got {cve:?}"
        );
    }
}

/// A rejected interval must write nothing at all — not the command it
/// arrived alongside, and not the other feed's interval.
#[tokio::test]
async fn write_managed_feed_settings_rejecting_an_interval_writes_nothing() {
    let db = Store::open_in_memory().unwrap();

    let err = write_managed_feed_settings(
        &db,
        ManagedFeedSettingsPatch {
            reviews_command: Some(Some(REVIEWS.to_string())),
            reviews_interval_secs: Some(Some(10)),
            cve_command: None,
            cve_interval_secs: Some(Some(300)),
        },
    )
    .await;
    assert!(matches!(err, Err(ServiceError::Validation(_))), "{err:?}");

    assert!(
        db.get_reviews_feed_command().await.unwrap().is_none(),
        "the command must not survive a rejected interval in the same call"
    );
    assert!(
        db.get_cve_feed_interval_secs().await.unwrap().is_none(),
        "the other feed's valid interval must not be written either"
    );
}

#[tokio::test]
async fn write_managed_feed_settings_accepts_the_floor_and_clearing() {
    let db = Store::open_in_memory().unwrap();

    write_managed_feed_settings(
        &db,
        ManagedFeedSettingsPatch {
            reviews_interval_secs: Some(Some(MIN_FEED_INTERVAL_SECS)),
            cve_interval_secs: Some(None),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(
        db.get_reviews_feed_interval_secs().await.unwrap(),
        Some(MIN_FEED_INTERVAL_SECS)
    );
    // Clearing means "inherit the default", which itself clears the floor.
    assert_eq!(db.get_cve_feed_interval_secs().await.unwrap(), None);
}
