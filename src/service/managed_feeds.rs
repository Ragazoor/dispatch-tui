//! Managed feed-epic provisioning (WP5): settings in, one reducer call out.
//!
//! The managed epics are the ones the PR-review feed routing depends on,
//! matched by feed role (never by title, so user renames survive): a
//! `reviews_parent` root carrying the reviews `feed_command` with
//! `my_reviews` / `team_reviews` / `bots` sub-epics that carry none, and a
//! `cve` root carrying the CVE `feed_command`. Each subtree exists only while
//! its command is configured; clearing the command is the opt-out. See the
//! `ProvisionManagedEpics` rule in `docs/specs/epics.allium`.
//!
//! The provisioning itself is the module's `ensure_managed_epics` reducer
//! (`epics.allium: ProvisionManagedEpics`, store-split M7 X4): the whole tree
//! in one transaction, so two boards starting together cannot create
//! duplicates. What stays here is request shape: reading and writing the four
//! settings, and the feed-interval floor on the two intervals.

use anyhow::Result;

use crate::service::ServiceError;

/// The four managed-feed settings, read from the settings table and fed to
/// `EpicCrud::ensure_managed_epics`. A named struct (rather than a 4-tuple) so the two
/// `(command, interval)` pairs can't be transposed at a call site.
#[derive(Debug, Clone, Default)]
pub struct ManagedFeedSettings {
    pub reviews_command: Option<String>,
    pub reviews_interval_secs: Option<i64>,
    pub cve_command: Option<String>,
    pub cve_interval_secs: Option<i64>,
}

/// Read the four managed-feed settings from the settings table. Pure reads —
/// callable through a read-only `&dyn SettingsStore` handle, so a non-service
/// consumer can fetch them and hand them to `EpicServiceApi::provision_managed_feeds`.
pub async fn read_managed_feed_settings(
    db: &dyn crate::store::SettingsStore,
) -> Result<ManagedFeedSettings> {
    Ok(ManagedFeedSettings {
        reviews_command: db.get_reviews_feed_command().await?,
        reviews_interval_secs: db.get_reviews_feed_interval_secs().await?,
        cve_command: db.get_cve_feed_command().await?,
        cve_interval_secs: db.get_cve_feed_interval_secs().await?,
    })
}

/// Partial update for the four managed-feed settings: absent (`None`) leaves
/// a field unchanged, `Some(None)` clears it, `Some(Some(v))` sets it. A named
/// struct (rather than four positional double-Option params) so the two
/// `(command, interval)` pairs can't be transposed at a call site — mirrors
/// the read-side [`ManagedFeedSettings`].
#[derive(Debug, Clone, Default)]
pub struct ManagedFeedSettingsPatch {
    pub reviews_command: Option<Option<String>>,
    pub reviews_interval_secs: Option<Option<i64>>,
    pub cve_command: Option<Option<String>>,
    pub cve_interval_secs: Option<Option<i64>>,
}

/// Persist only the provided managed-feed settings fields, leaving any absent
/// field unchanged. Mirrors [`read_managed_feed_settings`] for the write side.
pub async fn write_managed_feed_settings(
    db: &dyn crate::store::SettingsStore,
    patch: ManagedFeedSettingsPatch,
) -> std::result::Result<(), ServiceError> {
    // Both intervals are checked before ANY of the four settings is written.
    // These are separate rows, so a mid-way rejection would leave a command
    // persisted against an interval that was refused. They are also not a
    // separate kind of cadence: provisioning copies them onto the managed
    // epics' `feed_interval_secs`, so the floor that binds an epic must bind
    // them (epics.allium: SetManagedFeedConfig). `0` was blessed here as "poll
    // every tick" before the floor existed.
    crate::service::validate_feed_interval(
        "reviews_interval_secs",
        patch.reviews_interval_secs.flatten(),
    )?;
    crate::service::validate_feed_interval("cve_interval_secs", patch.cve_interval_secs.flatten())?;

    if let Some(v) = patch.reviews_command {
        db.set_reviews_feed_command(v.as_deref()).await?;
    }
    if let Some(v) = patch.reviews_interval_secs {
        db.set_reviews_feed_interval_secs(v).await?;
    }
    if let Some(v) = patch.cve_command {
        db.set_cve_feed_command(v.as_deref()).await?;
    }
    if let Some(v) = patch.cve_interval_secs {
        db.set_cve_feed_interval_secs(v).await?;
    }
    Ok(())
}

/// Read the managed-feed settings and provision accordingly. This is the
/// startup entry point (called from `run_tui`), also exercised directly in
/// tests. A no-op when neither command is configured.
// Takes the umbrella `&dyn TaskStore` so a concrete `&Store` (startup, tests)
// coerces in. Non-service consumers go through
// `EpicServiceApi::provision_managed_feeds` instead. The `ensure_managed_epics`
// call upcasts the trait object to `&dyn EpicCrud`.
pub async fn provision_managed_feeds_from_settings(db: &dyn crate::store::TaskStore) -> Result<()> {
    let s = read_managed_feed_settings(db).await?;
    db.ensure_managed_epics(
        s.reviews_command.as_deref(),
        s.reviews_interval_secs,
        s.cve_command.as_deref(),
        s.cve_interval_secs,
    )
    .await
}

#[cfg(test)]
mod tests;
