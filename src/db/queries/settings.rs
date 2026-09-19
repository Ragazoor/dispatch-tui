use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};

use super::super::{Database, SettingsStore};

#[async_trait::async_trait]
impl super::super::SettingsStore for Database {
    async fn get_setting_bool(&self, key: &str) -> Result<Option<bool>> {
        let key = key.to_string();
        self.db_call_read(move |conn| {
            conn.query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| {
                    let v: String = row.get(0)?;
                    Ok(v == "1")
                },
            )
            .optional()
            .context("Failed to get setting")
        })
        .await
    }

    async fn set_setting_bool(&self, key: &str, value: bool) -> Result<()> {
        let key = key.to_string();
        self.db_call(move |conn| {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = ?2",
                params![key, if value { "1" } else { "0" }],
            )?;
            Ok(())
        })
        .await
    }

    async fn get_setting_string(&self, key: &str) -> Result<Option<String>> {
        let key = key.to_string();
        self.db_call_read(move |conn| {
            conn.query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .context("Failed to get setting")
        })
        .await
    }

    async fn set_setting_string(&self, key: &str, value: &str) -> Result<()> {
        let key = key.to_string();
        let value = value.to_string();
        self.db_call(move |conn| {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = ?2",
                params![key, value],
            )?;
            Ok(())
        })
        .await
    }

    async fn save_filter_preset(
        &self,
        name: &str,
        repo_paths: &[String],
        mode: &str,
    ) -> Result<()> {
        let name = name.to_string();
        let mode = mode.to_string();
        let json = serde_json::to_string(repo_paths).context("Failed to serialize repo_paths")?;
        self.db_call(move |conn| {
            conn.execute(
                "INSERT INTO filter_presets (name, repo_paths, mode) VALUES (?1, ?2, ?3)
                 ON CONFLICT(name) DO UPDATE SET repo_paths = ?2, mode = ?3",
                params![name, json, mode],
            )?;
            Ok(())
        })
        .await
    }

    async fn delete_filter_preset(&self, name: &str) -> Result<()> {
        let name = name.to_string();
        self.db_call(move |conn| {
            conn.execute("DELETE FROM filter_presets WHERE name = ?1", params![name])?;
            Ok(())
        })
        .await
    }

    async fn list_filter_presets(&self) -> Result<Vec<(String, Vec<String>, String)>> {
        self.db_call_read(move |conn| {
            let mut stmt =
                conn.prepare("SELECT name, repo_paths, mode FROM filter_presets ORDER BY name")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            let raw: Vec<(String, String, String)> = rows
                .collect::<Result<Vec<_>, _>>()
                .context("Failed to list filter presets")?;
            raw.into_iter()
                .map(|(name, json, mode)| {
                    let paths: Vec<String> = serde_json::from_str(&json).with_context(|| {
                        format!("corrupt filter_preset JSON for preset {name:?}")
                    })?;
                    Ok((name, paths, mode))
                })
                .collect()
        })
        .await
    }

    async fn prune_repo_path_from_presets(&self, path: &str) -> Result<()> {
        let path = path.to_string();
        self.db_call(move |conn| {
            let presets: Vec<(String, String)> = {
                let mut stmt = conn
                    .prepare("SELECT name, repo_paths FROM filter_presets")
                    .context("Failed to prepare preset query")?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .context("Failed to list presets for cleanup")?;
                rows
            };
            for (name, json) in presets {
                let paths: Vec<String> = serde_json::from_str(&json)
                    .with_context(|| format!("corrupt filter_preset JSON for preset {name:?}"))?;
                let filtered: Vec<String> = paths.into_iter().filter(|p| p != &path).collect();
                if filtered.is_empty() {
                    conn.execute("DELETE FROM filter_presets WHERE name = ?1", params![name])?;
                } else {
                    let updated = serde_json::to_string(&filtered)
                        .context("Failed to serialize filtered repo_paths")?;
                    conn.execute(
                        "UPDATE filter_presets SET repo_paths = ?1 WHERE name = ?2",
                        params![updated, name],
                    )?;
                }
            }
            Ok(())
        })
        .await
    }

    // -- Managed-feed config (WP5) --

    async fn get_reviews_feed_command(&self) -> Result<Option<String>> {
        self.get_setting_string(REVIEWS_FEED_COMMAND_KEY).await
    }

    async fn set_reviews_feed_command(&self, value: Option<&str>) -> Result<()> {
        self.set_managed_feed_setting(REVIEWS_FEED_COMMAND_KEY, value)
            .await
    }

    async fn get_reviews_feed_interval_secs(&self) -> Result<Option<i64>> {
        parse_setting_i64(
            REVIEWS_FEED_INTERVAL_SECS_KEY,
            self.get_setting_string(REVIEWS_FEED_INTERVAL_SECS_KEY)
                .await?,
        )
    }

    async fn set_reviews_feed_interval_secs(&self, value: Option<i64>) -> Result<()> {
        self.set_managed_feed_setting(
            REVIEWS_FEED_INTERVAL_SECS_KEY,
            value.map(|n| n.to_string()).as_deref(),
        )
        .await
    }

    async fn get_cve_feed_command(&self) -> Result<Option<String>> {
        self.get_setting_string(CVE_FEED_COMMAND_KEY).await
    }

    async fn set_cve_feed_command(&self, value: Option<&str>) -> Result<()> {
        self.set_managed_feed_setting(CVE_FEED_COMMAND_KEY, value)
            .await
    }

    async fn get_cve_feed_interval_secs(&self) -> Result<Option<i64>> {
        parse_setting_i64(
            CVE_FEED_INTERVAL_SECS_KEY,
            self.get_setting_string(CVE_FEED_INTERVAL_SECS_KEY).await?,
        )
    }

    async fn set_cve_feed_interval_secs(&self, value: Option<i64>) -> Result<()> {
        self.set_managed_feed_setting(
            CVE_FEED_INTERVAL_SECS_KEY,
            value.map(|n| n.to_string()).as_deref(),
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// RepoConfigRead, RepoConfigStore — the `repo_paths` and `repo_base_branches` shared tables
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl super::super::RepoConfigRead for Database {
    async fn list_repo_paths(&self) -> Result<Vec<String>> {
        self.db_call_read(move |conn| {
            let mut stmt = conn
                // `id ASC` is a TIEBREAK, not decoration. `last_used` has
                // whole-second resolution, so several paths saved in the same
                // second tie — which SQLite then breaks by rowid, in practice
                // and not by promise. Leaving it implicit made the picker's
                // order something no second store could agree with, and
                // something this one was free to change at any release. Spelled
                // out, it is the order that has always been observed.
                .prepare("SELECT path FROM repo_paths ORDER BY last_used DESC, id ASC")
                .context("Failed to prepare list_repo_paths")?;
            let paths = stmt
                .query_map([], |row| row.get(0))
                .context("Failed to query repo_paths")?
                .collect::<rusqlite::Result<Vec<String>>>()
                .context("Failed to collect repo_paths")?;
            Ok(paths)
        })
        .await
    }

    async fn get_verify_command(&self, path: &str) -> Result<Option<String>> {
        let path = path.to_string();
        self.db_call_read(move |conn| {
            let result: Option<Option<String>> = conn
                .query_row(
                    "SELECT verify_command FROM repo_paths WHERE path = ?1",
                    params![path],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()
                .context("Failed to get verify_command")?;
            Ok(result.flatten())
        })
        .await
    }

    async fn list_all_base_branches(&self) -> Result<Vec<(String, String)>> {
        self.db_call_read(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT repo_path, branch FROM repo_base_branches ORDER BY last_used DESC, id DESC",
                )
                .context("Failed to prepare list_all_base_branches")?;
            let pairs = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .context("Failed to query repo_base_branches")?
                .collect::<rusqlite::Result<Vec<(String, String)>>>()
                .context("Failed to collect repo_base_branches")?;
            Ok(pairs)
        })
        .await
    }
}

#[async_trait::async_trait]
impl super::super::RepoConfigStore for Database {
    async fn save_repo_path(&self, path: &str) -> Result<()> {
        if let Some(writer) = self.shared_writer() {
            return writer.save_repo_path(path).await;
        }
        let path = path.to_string();
        self.db_call(move |conn| {
            conn.execute(
                "INSERT INTO repo_paths (path) VALUES (?1)
                 ON CONFLICT(path) DO UPDATE SET last_used = datetime('now')",
                params![path],
            )
            .context("Failed to save repo_path")?;
            Ok(())
        })
        .await
    }

    async fn delete_repo_path(&self, path: &str) -> Result<()> {
        let path = path.to_string();
        self.db_call(move |conn| {
            conn.execute("DELETE FROM repo_paths WHERE path = ?1", params![path])
                .context("Failed to delete repo_path")?;
            Ok(())
        })
        .await
    }

    async fn set_verify_command(&self, path: &str, command: Option<&str>) -> Result<()> {
        let path = path.to_string();
        let resolved: Option<String> = match command {
            Some(raw) => {
                if raw.contains('\n') || raw.contains('\r') {
                    anyhow::bail!(
                        "verify_command must not contain a newline or carriage return (use && or ; to chain steps)"
                    );
                }
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            }
            None => None,
        };
        self.db_call(move |conn| {
            match resolved {
                Some(cmd) => {
                    conn.execute(
                        "INSERT INTO repo_paths(path, verify_command) VALUES(?1, ?2)
                         ON CONFLICT(path) DO UPDATE SET verify_command = excluded.verify_command",
                        params![path, cmd],
                    )
                    .context("Failed to upsert verify_command")?;
                }
                None => {
                    conn.execute(
                        "UPDATE repo_paths SET verify_command = NULL WHERE path = ?1",
                        params![path],
                    )
                    .context("Failed to clear verify_command")?;
                }
            }
            Ok(())
        })
        .await
    }

    async fn record_base_branch(&self, repo_path: &str, branch: &str) -> Result<()> {
        let repo_path = repo_path.to_string();
        let branch = branch.to_string();
        self.db_call(move |conn| {
            conn.execute(
                "INSERT INTO repo_base_branches (repo_path, branch) VALUES (?1, ?2)
                 ON CONFLICT(repo_path, branch) DO UPDATE SET last_used = datetime('now')",
                params![repo_path, branch],
            )
            .context("Failed to record base branch")?;
            // Prune this repo's history down to the most-recently-used cap
            // (config.max_base_branches_per_repo; see dispatch.allium:
            // BranchHistoryCapped). Pruning is part of the write so callers
            // never have to manage the cap themselves.
            conn.execute(
                "DELETE FROM repo_base_branches
                 WHERE repo_path = ?1
                   AND id NOT IN (
                       SELECT id FROM repo_base_branches
                       WHERE repo_path = ?1
                       ORDER BY last_used DESC, id DESC
                       LIMIT ?2
                   )",
                params![repo_path, MAX_BASE_BRANCHES_PER_REPO],
            )
            .context("Failed to prune base branch history")?;
            Ok(())
        })
        .await
    }
}

// ---------------------------------------------------------------------------
// HostStore — this install's row in the host registry
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl super::super::HostStore for Database {
    async fn ensure_host_identity(&self) -> Result<(String, Option<String>)> {
        let generated_id = uuid::Uuid::new_v4().to_string();
        let id: String = self
            .db_call(move |conn| {
                // `DO NOTHING` is the whole guarantee: whichever process (or
                // whichever of several concurrent first-run dispatch
                // processes, see docs/conventions.md's cross-process writer
                // note) gets its INSERT applied first wins the id
                // permanently, and every other caller — this run and every
                // future one — reads that same row back rather than
                // overwriting it. See host.allium: MintHostIdentity's "mint
                // is not re-run" guidance.
                //
                // Insert and read-back share one closure because they are one
                // atomic question ("what id did this database settle on?").
                // The label read below is not part of that question, so it
                // stays outside — on the read pool rather than queued behind
                // the writer.
                conn.execute(
                    "INSERT INTO settings (key, value) VALUES (?1, ?2) \
                     ON CONFLICT(key) DO NOTHING",
                    params![HOST_ID_KEY, generated_id],
                )
                .context("Failed to mint host id")?;
                let id: String = conn
                    .query_row(
                        "SELECT value FROM settings WHERE key = ?1",
                        params![HOST_ID_KEY],
                        |row| row.get(0),
                    )
                    .context("Failed to read host id after mint")?;
                Ok(id)
            })
            .await?;

        // Mint sets `host_id` only. `host_label` is deliberately not seeded —
        // see host.allium: MintHostIdentity's "THE LABEL IS NOT MINTED AT
        // ALL" guidance — so this read answers `None` until an operator names
        // this machine via `rename_host`, which `docs/specs/startup.allium`'s
        // `HostLabelPrompt` asks for before the board's first launch draws.
        let label = self
            .get_setting_string(HOST_LABEL_KEY)
            .await
            .context("Failed to read host label after mint")?;
        Ok((id, label))
    }

    async fn rename_host(&self, label: &str) -> Result<()> {
        let trimmed = label.trim();
        if trimmed.is_empty() {
            anyhow::bail!("host label must not be empty");
        }
        self.set_setting_string(HOST_LABEL_KEY, trimmed).await
    }

    async fn user_identity(&self) -> Result<Option<String>> {
        self.get_setting_string(USER_IDENTITY_KEY)
            .await
            .context("Failed to read the stored user identity")
    }

    async fn adopt_user_identity(&self, identity: &str) -> Result<()> {
        let identity = identity.trim();
        if identity.is_empty() {
            anyhow::bail!("user identity must not be empty");
        }

        // `DO NOTHING`, not an upsert: the stored identity is written once and
        // never again (`core.allium: LocalHostOwnerIsWrittenOnce`). A blind
        // overwrite here would be the silent adoption that
        // `host.allium: RefuseAChangedUserIdentity` exists to prevent, placed
        // below the level that refuses it.
        let identity = identity.to_string();
        self.db_call(move |conn| {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) \
                 ON CONFLICT(key) DO NOTHING",
                params![USER_IDENTITY_KEY, identity],
            )
            .context("Failed to store the user identity")?;
            Ok(())
        })
        .await
    }
}

// ---------------------------------------------------------------------------
// IdentityCredentialStore — the local secret that proves the shared identity
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl super::super::IdentityCredentialStore for Database {
    async fn user_identity_token(&self) -> Result<Option<String>> {
        self.get_setting_string(USER_IDENTITY_TOKEN_KEY)
            .await
            .context("Failed to read the stored user identity credential")
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
        self.set_setting_string(USER_IDENTITY_TOKEN_KEY, token)
            .await
    }
}

/// Per-repo cap on remembered base branches (config.max_base_branches_per_repo
/// in docs/specs/dispatch.allium). `record_base_branch` prunes beyond this on
/// every write; see the `BranchHistoryCapped` invariant.
const MAX_BASE_BRANCHES_PER_REPO: i64 = 10;

// ---------------------------------------------------------------------------
// Host identity keys (docs/specs/host.allium)
// ---------------------------------------------------------------------------

/// `settings` key holding this install's Host id (`core/Host.id`). Minted once
/// by `ensure_host_identity` and never rewritten.
///
/// A `macro_rules!` rather than only a `const` because the key also has to
/// appear inside a compile-time SQL string — `LOCALLY_OWNED_PREDICATE` in
/// `src/db/queries/mod.rs` builds its subquery with `concat!`, which accepts
/// literals and macro expansions but not a `const` item. Same bridge, same
/// reason, as `src/claude_paths.rs`; see "Two things that must agree" in
/// `docs/conventions.md`. Spelled inline in either place, a rename would yield
/// a statement that silently matches nothing rather than a compile error.
macro_rules! host_id_key {
    () => {
        "host_id"
    };
}
pub(crate) use host_id_key;

pub(crate) const HOST_ID_KEY: &str = host_id_key!();

/// `settings` key holding this install's operator-chosen Host label
/// (`core/Host.label`). Absent until `rename_host` writes it; its absence is
/// what `docs/specs/startup.allium`'s `PromptForHostLabelWhenUnnamed` reads as
/// "this machine has not been named".
pub(crate) const HOST_LABEL_KEY: &str = "host_label";

/// `settings` key holding this install's UserIdentity — `core/Host.owner` for
/// the local row.
///
/// Beside the host id rather than anywhere else because the two are the same
/// kind of fact about this install, asked at different times: the id is minted
/// offline on first run, and this is learned from a shared store on first
/// connect. Absent means "this install has never connected", which is a real
/// and lasting state rather than a bounded window (`core.allium: Host.owner`).
///
/// There is exactly one place an install remembers who it is, and this is it.
/// See `LocalHostOwnerIsWrittenOnce`: a second copy would be a second thing to
/// keep in step, and the way it fails is a machine owned by a person the
/// install no longer believes it is.
pub(crate) const USER_IDENTITY_KEY: &str = "user_identity";

/// `settings` key holding the credential that proves [`USER_IDENTITY_KEY`].
///
/// **Local, secret and deliberately not domain.** It is not in `SharedTable`,
/// never travels in a snapshot, and has no counterpart in the SpacetimeDB
/// module — a credential in a shared store is a credential everybody on the
/// board can use. `docs/specs/sync.allium` does not mention it for the same
/// reason it does not mention sockets: it is how the identity is proven, not
/// what the identity means.
///
/// Losing it is not a small thing. The store cannot recognise this install
/// without it, so it issues a NEW identity, and that is precisely the conflict
/// `host.allium: RefuseAChangedUserIdentity` refuses.
pub(crate) const USER_IDENTITY_TOKEN_KEY: &str = "user_identity_token";

// ---------------------------------------------------------------------------
// Managed-feed config keys (WP5)
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

impl Database {
    /// Upsert a managed-feed settings key when `value` is `Some`, or delete the
    /// row when `None` so a subsequent get returns `None`.
    async fn set_managed_feed_setting(&self, key: &'static str, value: Option<&str>) -> Result<()> {
        match value {
            Some(v) => self.set_setting_string(key, v).await,
            None => {
                self.db_call(move |conn| {
                    conn.execute("DELETE FROM settings WHERE key = ?1", params![key])
                        .context("Failed to delete setting")?;
                    Ok(())
                })
                .await
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SubscriptionStore — the `subscriptions` shared table
// ---------------------------------------------------------------------------

/// The `subscriptions` row id: `<subscriber>/<epic_id>`.
///
/// Derived rather than generated, and the same shape the SpacetimeDB module
/// uses, so the two stores agree on which rows are the same row. Uniqueness
/// that matters is over the PAIR, and a single-column primary key over the
/// derived pair is how a store that indexes one column at a time expresses it
/// (`core.allium: SubscriptionIsUniquePerSubscriberAndEpic`).
fn subscription_id(subscriber: &str, epic_id: i64) -> String {
    format!("{subscriber}/{epic_id}")
}

#[async_trait::async_trait]
impl super::super::SubscriptionStore for Database {
    async fn subscribed_epics(&self, subscriber: &str) -> Result<Vec<i64>> {
        let subscriber = subscriber.to_string();
        self.db_call_read(move |conn| {
            let mut stmt = conn
                .prepare("SELECT epic_id FROM subscriptions WHERE subscriber = ?1 ORDER BY epic_id")
                .context("Failed to prepare the subscription read")?;
            let ids = stmt
                .query_map(params![subscriber], |row| row.get::<_, i64>(0))
                .context("Failed to query subscriptions")?
                .collect::<rusqlite::Result<Vec<i64>>>()
                .context("Failed to decode a subscription row")?;
            Ok(ids)
        })
        .await
    }

    async fn subscribe_to_epic(&self, subscriber: &str, epic_id: i64) -> Result<()> {
        if subscriber.trim().is_empty() {
            // `sync.allium: SubscribeToEpic` requires an identity. An install
            // that has never connected has none, and a subscription with an
            // empty subscriber is a row that belongs to nobody — which would
            // then be sent to everybody by a store that matches on it.
            anyhow::bail!("cannot subscribe without a user identity");
        }
        let id = subscription_id(subscriber, epic_id);
        let subscriber = subscriber.to_string();
        self.db_call(move |conn| {
            // DO NOTHING, not UPDATE: subscribing twice is subscribing. There
            // is nothing to refresh on a row whose every column is part of its
            // own key.
            conn.execute(
                "INSERT INTO subscriptions (id, epic_id, subscriber) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(id) DO NOTHING",
                params![id, epic_id, subscriber],
            )
            .context("Failed to subscribe to epic")?;
            Ok(())
        })
        .await
    }

    async fn unsubscribe_from_epic(&self, subscriber: &str, epic_id: i64) -> Result<bool> {
        let id = subscription_id(subscriber, epic_id);
        self.db_call(move |conn| {
            let removed = conn
                .execute("DELETE FROM subscriptions WHERE id = ?1", params![id])
                .context("Failed to unsubscribe from epic")?;
            Ok(removed > 0)
        })
        .await
    }
}
