//! Repo configuration, subscriptions, settings and usage events.

use anyhow::Result;
use chrono::{DateTime, Utc};

use dispatch_spacetime_module as module;

use crate::models::EpicId;
use crate::spacetime::bindings;

use super::super::encode;
use super::super::writes::ReducerOutcome;
use super::{MemoryReducerCaller, Tables, POLL_SCOPE_EPIC};

impl MemoryReducerCaller {
    pub(super) fn write_repo_path(&self, tables: &mut Tables, mut row: module::RepoPath) -> i64 {
        let id = Self::assign_id(row.id, &mut tables.next_repo_path_id);
        row.id = id;
        tables.repo_paths.insert(id, row.clone());
        self.rows.upsert_repo_path(&row.into());
        id
    }

    pub(super) fn write_repo_base_branch(
        &self,
        tables: &mut Tables,
        mut row: module::RepoBaseBranch,
    ) -> i64 {
        let id = Self::assign_id(row.id, &mut tables.next_repo_base_branch_id);
        row.id = id;
        tables.repo_base_branches.insert(id, row.clone());
        self.rows.upsert_repo_base_branch(&row.into());
        id
    }

    pub(super) fn apply_save_repo_path(
        &self,
        path: String,
        last_used: DateTime<Utc>,
    ) -> Result<ReducerOutcome> {
        let last_used = encode::stamp(last_used);
        if path.trim().is_empty() {
            return Ok(ReducerOutcome::Refused("repo path is empty".into()));
        }
        let mut tables = self.lock();
        let existing = tables.repo_paths.values().find(|r| r.path == path).cloned();
        match existing {
            Some(row) => {
                self.write_repo_path(&mut tables, module::RepoPath { last_used, ..row });
            }
            None => {
                self.write_repo_path(
                    &mut tables,
                    module::RepoPath {
                        id: 0,
                        path,
                        last_used,
                        verify_command: String::new(),
                    },
                );
            }
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_delete_repo_path(&self, path: String) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let ids: Vec<i64> = tables
            .repo_paths
            .values()
            .filter(|r| r.path == path)
            .map(|r| r.id)
            .collect();
        for id in ids {
            if tables.repo_paths.remove(&id).is_some() {
                self.rows.remove_repo_path(id);
            }
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_set_verify_command(
        &self,
        path: String,
        command: String,
    ) -> Result<ReducerOutcome> {
        if command.contains('\n') || command.contains('\r') {
            return Ok(ReducerOutcome::Refused(
                "verify command must be a single line; chain steps with && or ;".into(),
            ));
        }
        let mut tables = self.lock();
        let Some(existing) = tables.repo_paths.values().find(|r| r.path == path).cloned() else {
            return Ok(ReducerOutcome::Refused(format!("no repo path {path}")));
        };
        self.write_repo_path(
            &mut tables,
            module::RepoPath {
                verify_command: command,
                ..existing
            },
        );
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_record_base_branch(
        &self,
        repo_path: String,
        branch: String,
        last_used: DateTime<Utc>,
    ) -> Result<ReducerOutcome> {
        let last_used = encode::stamp(last_used);
        let mut tables = self.lock();
        let existing = tables
            .repo_base_branches
            .values()
            .find(|r| r.repo_path == repo_path && r.branch == branch)
            .cloned();
        match existing {
            Some(row) => {
                self.write_repo_base_branch(
                    &mut tables,
                    module::RepoBaseBranch { last_used, ..row },
                );
            }
            None => {
                self.write_repo_base_branch(
                    &mut tables,
                    module::RepoBaseBranch {
                        id: 0,
                        repo_path,
                        branch,
                        last_used,
                    },
                );
            }
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_subscribe_to_epic(
        &self,
        subscriber: String,
        epic_id: EpicId,
    ) -> Result<ReducerOutcome> {
        let epic_id = epic_id.0;
        if subscriber.trim().is_empty() {
            return Ok(ReducerOutcome::Refused(
                "cannot subscribe without an identity".into(),
            ));
        }
        let mut tables = self.lock();
        if !tables.epics.contains_key(&epic_id) {
            return Ok(ReducerOutcome::Refused(format!("no epic {epic_id}")));
        }
        let id = module::subscription_id(&subscriber, epic_id);
        // DO NOTHING on a repeat, not an update — mirrors the module's own
        // `subscribe_to_epic`: every column of this row is part of its own
        // key, so there is nothing a second subscribe could refresh.
        if let std::collections::btree_map::Entry::Vacant(entry) = tables.subscriptions.entry(id) {
            let row = module::Subscription {
                id: entry.key().clone(),
                subscriber,
                epic_id,
            };
            entry.insert(row.clone());
            self.rows.upsert_subscription(&row.into());
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_unsubscribe_from_epic(
        &self,
        subscriber: String,
        epic_id: EpicId,
    ) -> Result<ReducerOutcome> {
        let epic_id = epic_id.0;
        let mut tables = self.lock();
        let id = module::subscription_id(&subscriber, epic_id);
        if !tables.subscriptions.contains_key(&id) {
            return Ok(ReducerOutcome::Refused(format!(
                "not subscribed to epic {epic_id}"
            )));
        }
        tables.subscriptions.remove(&id);
        self.rows.remove_subscription(id);
        // Mirrors the module: release the claims this person no longer covers.
        let remaining: std::collections::HashSet<i64> = tables
            .subscriptions
            .values()
            .filter(|s| s.subscriber == subscriber)
            .map(|s| s.epic_id)
            .collect();
        let parents: std::collections::HashMap<i64, i64> = tables
            .epics
            .values()
            .map(|e| (e.id, e.parent_epic_id))
            .collect();
        for released in module::epics_losing_coverage(epic_id, &remaining, &parents) {
            let stale: Vec<i64> = tables
                .poll_owners
                .values()
                .filter(|p| p.scope == POLL_SCOPE_EPIC && p.scope_id == released)
                .filter(|p| {
                    tables
                        .hosts
                        .get(&p.host)
                        .is_some_and(|h| h.owner == subscriber)
                })
                .map(|p| p.id)
                .collect();
            for row_id in stale {
                tables.poll_owners.remove(&row_id);
                self.rows.remove_poll_owner(row_id);
            }
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    /// Mirrors the module's `save_setting`: refuses an empty host, otherwise
    /// upserts on the derived `(host, key)` id.
    pub(super) fn apply_save_setting(
        &self,
        host: String,
        key: String,
        value: String,
    ) -> Result<ReducerOutcome> {
        if host.trim().is_empty() {
            return Ok(ReducerOutcome::Refused(
                "a host id must not be empty".into(),
            ));
        }
        let mut tables = self.lock();
        let id = module::host_scoped_id(&host, &key);
        let row = module::Setting {
            id: id.clone(),
            host,
            key,
            value,
        };
        tables.settings.insert(id, row.clone());
        self.rows.upsert_setting(&row.into());
        Ok(ReducerOutcome::Applied(vec![]))
    }

    /// Mirrors the module's `clear_setting`: deleting an absent key is a
    /// no-op, not a refusal (`docs/specs/settings.allium`'s `ClearSetting`).
    pub(super) fn apply_clear_setting(&self, host: String, key: String) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        let id = module::host_scoped_id(&host, &key);
        if tables.settings.remove(&id).is_some() {
            self.rows.remove_setting(id);
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    /// Mirrors the module's `record_usage_event` + `prune_usage_events`: the
    /// prune runs in the same call as the insert, so no reader ever sees the
    /// table over `cap`.
    pub(super) fn apply_record_usage_event(
        &self,
        row: bindings::UsageEvent,
        cap: i64,
    ) -> Result<ReducerOutcome> {
        if cap <= 0 {
            return Ok(ReducerOutcome::Refused(format!(
                "usage cap must be positive, got {cap}"
            )));
        }
        let mut tables = self.lock();
        let id = Self::assign_id(0, &mut tables.next_usage_event_id);
        let row = module::UsageEvent {
            id,
            ..module::UsageEvent::from(row)
        };
        tables.usage_events.insert(id, row.clone());
        self.rows.upsert_usage_event(&row.into());
        let threshold = id - cap;
        if threshold > 0 {
            let stale: Vec<i64> = tables
                .usage_events
                .range(..=threshold)
                .map(|(&id, _)| id)
                .collect();
            for stale_id in stale {
                tables.usage_events.remove(&stale_id);
                self.rows.remove_usage_event(stale_id);
            }
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }
}
