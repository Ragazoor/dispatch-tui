use anyhow::{Context, Result};

use super::super::{EpicId, Store};

#[async_trait::async_trait]
impl super::super::SettingsStore for Store {
    async fn get_setting_bool(&self, key: &str) -> Result<Option<bool>> {
        Ok(self.get_setting_string(key).await?.map(|v| v == "1"))
    }

    async fn set_setting_bool(&self, key: &str, value: bool) -> Result<()> {
        self.set_setting_string(key, if value { "1" } else { "0" })
            .await
    }

    async fn get_setting_string(&self, key: &str) -> Result<Option<String>> {
        Ok(self.rows.setting(key))
    }

    async fn set_setting_string(&self, key: &str, value: &str) -> Result<()> {
        refuse_identity_key(key)?;
        // Scoped by this board's own host id, not an argument —
        // `docs/specs/settings.allium` for why the scope is host rather than
        // owner.
        self.caller
            .save_setting(self.host.clone(), key.to_string(), value.to_string())
            .await?
            .applied()
    }

    // -- Managed-feed config --

    async fn get_reviews_feed_command(&self) -> Result<Option<String>> {
        self.get_setting_string(REVIEWS_FEED_COMMAND_KEY).await
    }

    async fn get_reviews_feed_interval_secs(&self) -> Result<Option<i64>> {
        parse_setting_i64(
            REVIEWS_FEED_INTERVAL_SECS_KEY,
            self.get_setting_string(REVIEWS_FEED_INTERVAL_SECS_KEY)
                .await?,
        )
    }

    async fn get_cve_feed_command(&self) -> Result<Option<String>> {
        self.get_setting_string(CVE_FEED_COMMAND_KEY).await
    }

    async fn get_cve_feed_interval_secs(&self) -> Result<Option<i64>> {
        parse_setting_i64(
            CVE_FEED_INTERVAL_SECS_KEY,
            self.get_setting_string(CVE_FEED_INTERVAL_SECS_KEY).await?,
        )
    }

    async fn set_managed_feed_settings(
        &self,
        patch: super::super::ManagedFeedSettingsPatch,
    ) -> Result<()> {
        let mut writes = Vec::new();
        let mut add = |key: &str, value: Option<String>| {
            writes.push(crate::spacetime::bindings::SettingWrite {
                key: key.to_string(),
                value,
            });
        };
        if let Some(v) = patch.reviews_command {
            add(REVIEWS_FEED_COMMAND_KEY, v);
        }
        if let Some(v) = patch.reviews_interval_secs {
            add(REVIEWS_FEED_INTERVAL_SECS_KEY, v.map(|n| n.to_string()));
        }
        if let Some(v) = patch.cve_command {
            add(CVE_FEED_COMMAND_KEY, v);
        }
        if let Some(v) = patch.cve_interval_secs {
            add(CVE_FEED_INTERVAL_SECS_KEY, v.map(|n| n.to_string()));
        }
        if writes.is_empty() {
            return Ok(());
        }
        self.caller
            .set_managed_feed_settings(self.host.clone(), writes)
            .await?
            .applied()
    }
}

// ---------------------------------------------------------------------------
// RepoConfigRead, RepoConfigStore — the `repo_paths` and `repo_base_branches` shared tables
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl super::super::RepoConfigRead for Store {
    async fn list_repo_paths(&self) -> Vec<String> {
        self.rows.repo_paths()
    }

    async fn list_repo_paths_saved_by_this_host(&self) -> Vec<String> {
        self.rows.repo_paths_saved_by(&self.host)
    }

    async fn list_unowned_repo_paths(&self) -> Vec<String> {
        self.rows.repo_paths_unowned()
    }

    async fn get_verify_command(&self, path: &str) -> Option<String> {
        self.rows.verify_command(path, &self.host)
    }

    async fn list_all_base_branches(&self) -> Vec<(String, String)> {
        self.rows.base_branches()
    }
}

/// The module's `""` means "no command", so a blank one is that.
fn normalised_verify_command(command: Option<&str>) -> String {
    match command {
        Some(c) if !c.trim().is_empty() => c.to_string(),
        _ => String::new(),
    }
}

#[async_trait::async_trait]
impl super::super::RepoConfigStore for Store {
    async fn save_repo_path(&self, path: &str) -> Result<()> {
        // The repository's GitHub path comes from this host's clone; `""` when
        // it has no GitHub origin (or the path is not a clone).
        let owned = path.to_string();
        let github_path = tokio::task::spawn_blocking(move || {
            crate::git::origin_github_repo(&owned, &crate::process::RealProcessRunner::default())
        })
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
        self.caller
            .save_repo_path(
                path.to_string(),
                self.clock.now(),
                self.host.clone(),
                github_path,
            )
            .await?
            .applied()
    }

    async fn adopt_repo_path(&self, path: &str) -> Result<()> {
        self.caller
            .adopt_repo_path(path.to_string(), self.host.clone())
            .await?
            .applied()
    }

    async fn delete_repo_path(&self, path: &str) -> Result<()> {
        self.caller
            .delete_repo_path(path.to_string(), self.host.clone())
            .await?
            .applied()
    }

    async fn set_verify_command(&self, path: &str, command: Option<&str>) -> Result<()> {
        // `""` clears it. The module's absent sentinel, not a command that
        // happens to be empty — see its header.
        self.caller
            .set_verify_command(
                path.to_string(),
                self.host.clone(),
                normalised_verify_command(command),
            )
            .await?
            .applied()
    }

    async fn record_base_branch(&self, repo_path: &str, branch: &str) -> Result<()> {
        self.caller
            .record_base_branch(repo_path.to_string(), branch.to_string(), self.clock.now())
            .await?
            .applied()
    }
}

// ---------------------------------------------------------------------------
// HostStore — this install's row in the host registry
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl super::super::HostStore for Store {
    fn host_id(&self) -> &str {
        &self.host
    }

    async fn ensure_host_identity(&self) -> Result<(String, Option<String>)> {
        // The identity is READ from the host file, never minted here: minting
        // is the board launch's `host_file::resolve_for_launch`, and a
        // one-shot command with no host file must fail rather than mint
        // (`cli.allium: CliCommandsNeedAHostFile`).
        let identity = host_file_call(&self.host_file_dir, crate::host_file::read_for_cli).await?;
        Ok((identity.host_id, identity.label))
    }

    async fn rename_host(&self, label: &str) -> Result<()> {
        let trimmed = label.trim();
        if trimmed.is_empty() {
            anyhow::bail!("host label must not be empty");
        }
        let dir = &self.host_file_dir;
        let label = trimmed.to_string();
        host_file_call(dir, move |dir| crate::host_file::rename_host(dir, &label)).await?;
        Ok(())
    }

    async fn user_identity(&self) -> Result<Option<String>> {
        let dir = &self.host_file_dir;
        Ok(host_file_call(dir, crate::host_file::read_for_cli)
            .await?
            .user_identity)
    }

    async fn adopt_user_identity_with_credential(
        &self,
        identity: &str,
        credential: &str,
    ) -> Result<()> {
        let dir = &self.host_file_dir;
        let (identity, credential) = (identity.to_string(), credential.to_string());
        host_file_call(dir, move |dir| {
            crate::host_file::adopt_user_identity(dir, &identity, &credential)
        })
        .await?;
        Ok(())
    }

    async fn adopt_user_identity(&self, identity: &str) -> Result<()> {
        let identity = identity.trim();
        if identity.is_empty() {
            anyhow::bail!("user identity must not be empty");
        }
        let dir = &self.host_file_dir;
        let identity = identity.to_string();
        host_file_call(dir, move |dir| {
            crate::host_file::adopt_user_identity_once(dir, &identity)
        })
        .await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// IdentityCredentialStore — the local secret that proves the shared identity
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl super::super::IdentityCredentialStore for Store {
    async fn user_identity_token(&self) -> Result<Option<String>> {
        let dir = &self.host_file_dir;
        Ok(host_file_call(dir, crate::host_file::read_for_cli)
            .await?
            .credential)
    }

    async fn set_user_identity_token(&self, token: &str) -> Result<()> {
        let token = token.trim();
        if token.is_empty() {
            // Refused rather than stored, because an identity with no
            // credential is one this install cannot prove again — it would
            // survive exactly until the next connection and then present as a
            // conflict, which is the worst of both answers.
            anyhow::bail!("user identity credential must not be empty");
        }
        let dir = &self.host_file_dir;
        let token = token.to_string();
        host_file_call(dir, move |dir| {
            crate::host_file::set_credential(dir, &token)
        })
        .await?;
        Ok(())
    }
}

/// Run a host-file operation off the async runtime: it is small, synchronous
/// file I/O, and a whole-file replacement fsyncs.
async fn host_file_call<T: Send + 'static>(
    data_dir: &std::path::Path,
    op: impl FnOnce(&std::path::Path) -> Result<T> + Send + 'static,
) -> Result<T> {
    let data_dir = data_dir.to_path_buf();
    tokio::task::spawn_blocking(move || op(&data_dir))
        .await
        .context("host file task panicked")?
}

// ---------------------------------------------------------------------------
// Host identity keys (docs/specs/host.allium)
// ---------------------------------------------------------------------------
//
// These name the identity fields so `refuse_identity_key` can keep them out of
// the shared settings table. The identity itself lives in the host file
// (`host.allium: IdentityLivesInHostFile`), never in a setting row.

/// The name of this install's Host id (`core/Host.id`).
pub(crate) const HOST_ID_KEY: &str = "host_id";

/// The name of this install's operator-chosen Host label (`core/Host.label`).
pub(crate) const HOST_LABEL_KEY: &str = "host_label";

/// The name of this install's UserIdentity (`core/Host.owner` for the local
/// row).
pub(crate) const USER_IDENTITY_KEY: &str = "user_identity";

/// Refuse a write to `SettingsStore`'s generic key/value accessors when `key`
/// is one of the four identity/credential keys.
///
/// The choke point for `docs/specs/settings.allium`'s Excludes, enforced here
/// rather than left to caller discipline: `HostStore`/`IdentityCredentialStore`
/// keep identity in the host file and never call it, so this only ever fires on a
/// caller mistake — but the alternative is a future caller passing
/// `USER_IDENTITY_TOKEN_KEY` to `set_setting_string` and silently routing a
/// credential into the shared store.
fn refuse_identity_key(key: &str) -> Result<()> {
    if matches!(
        key,
        HOST_ID_KEY | HOST_LABEL_KEY | USER_IDENTITY_KEY | "user_identity_token"
    ) {
        anyhow::bail!(
            "{key:?} is a host identity/credential key, not a generic setting — \
             use HostStore/IdentityCredentialStore instead"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Managed-feed config keys
// ---------------------------------------------------------------------------

const REVIEWS_FEED_COMMAND_KEY: &str = "reviews_feed_command";
const REVIEWS_FEED_INTERVAL_SECS_KEY: &str = "reviews_feed_interval_secs";
const CVE_FEED_COMMAND_KEY: &str = "cve_feed_command";
const CVE_FEED_INTERVAL_SECS_KEY: &str = "cve_feed_interval_secs";

/// Parse a settings value stored as a decimal string into an `i64`. A stored
/// non-integer is a corruption we surface rather than silently treat as unset.
fn parse_setting_i64(key: &str, raw: Option<String>) -> Result<Option<i64>> {
    match raw {
        Some(s) => s
            .parse::<i64>()
            .map(Some)
            .with_context(|| format!("setting {key:?} is not a valid integer: {s:?}")),
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// SubscriptionStore — the `subscriptions` shared table
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl super::super::SubscriptionStore for Store {
    async fn subscribed_epics(&self, subscriber: &str) -> Result<Vec<EpicId>> {
        Ok(self.rows.subscribed_epics(subscriber))
    }

    async fn subscribe_to_epic(&self, subscriber: &str, epic_id: EpicId) -> Result<()> {
        if subscriber.trim().is_empty() {
            // `sync.allium: SubscribeToEpic` requires an identity. An install
            // that has never connected has none, and a subscription with an
            // empty subscriber is a row that belongs to nobody — which would
            // then be sent to everybody by a store that matches on it.
            anyhow::bail!("cannot subscribe without a user identity");
        }
        self.caller
            .subscribe_to_epic(subscriber.to_string(), epic_id)
            .await?
            .applied()
    }

    async fn unsubscribe_from_epic(&self, subscriber: &str, epic_id: EpicId) -> Result<bool> {
        // `sync.allium: UnsubscribeFromEpic` makes an epic that was not
        // followed a refusal rather than a no-op, and the store says so; it is
        // reported as `false` rather than as an error.
        Ok(self
            .caller
            .unsubscribe_from_epic(subscriber.to_string(), epic_id)
            .await?
            .won())
    }
}
