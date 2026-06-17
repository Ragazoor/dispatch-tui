//! Managed-feed config side-effect commands.

#[derive(Debug, Clone)]
pub enum ManagedFeedCommand {
    /// Persist the four managed-feed settings. `None` clears a setting.
    PersistConfig {
        reviews_command: Option<String>,
        reviews_interval_secs: Option<i64>,
        cve_command: Option<String>,
        cve_interval_secs: Option<i64>,
    },
    /// Re-fire `provision_managed_feeds_from_settings`, then refresh epics into
    /// the board so a newly-enabled feed's tree appears without a restart.
    ProvisionAndRefresh,
}
