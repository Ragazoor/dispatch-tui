//! Repo configuration, subscriptions, settings and usage events.

use anyhow::Result;
use chrono::{DateTime, Utc};

use dispatch_spacetime_module as module;

use crate::models::EpicId;
use crate::spacetime::bindings;

use super::super::encode;
use super::super::writes::ReducerOutcome;
use super::{MemoryReducerCaller, Tables, POLL_SCOPE_EPIC, POLL_SCOPE_TASK};

impl MemoryReducerCaller {
    /// Mirrors the module's `release_poll_claims`: drop every epic-scoped
    /// claim on an epic in `epics` held by a host whose owner
    /// `loses_sight(epic_id, owner)`.
    pub(super) fn release_poll_claims(
        &self,
        tables: &mut Tables,
        epics: &std::collections::HashSet<i64>,
        loses_sight: impl Fn(i64, &str) -> bool,
    ) {
        let stale: Vec<i64> = tables
            .poll_owners
            .values()
            .filter(|p| p.scope == POLL_SCOPE_EPIC && epics.contains(&p.scope_id))
            .filter(|p| {
                tables
                    .hosts
                    .get(&p.host)
                    .is_some_and(|h| loses_sight(p.scope_id, &h.owner))
            })
            .map(|p| p.id)
            .collect();
        for row_id in stale {
            tables.poll_owners.remove(&row_id);
            self.rows.remove_poll_owner(row_id);
        }
    }

    /// Mirrors the module's `release_task_poll_claims` (M10): drop every
    /// task-scoped claim on a task of an epic in `epics` held by a host whose
    /// owner `loses_sight(epic_id, owner)` of the task's epic.
    pub(super) fn release_task_poll_claims(
        &self,
        tables: &mut Tables,
        epics: &std::collections::HashSet<i64>,
        loses_sight: impl Fn(i64, &str) -> bool,
    ) {
        let stale: Vec<i64> = tables
            .poll_owners
            .values()
            .filter(|p| p.scope == POLL_SCOPE_TASK)
            .filter_map(|p| {
                let epic_id = tables.tasks.get(&p.scope_id)?.epic_id;
                let owner = &tables.hosts.get(&p.host)?.owner;
                (epics.contains(&epic_id) && loses_sight(epic_id, owner)).then_some(p.id)
            })
            .collect();
        for row_id in stale {
            tables.poll_owners.remove(&row_id);
            self.rows.remove_poll_owner(row_id);
        }
    }

    /// Mirrors the module's `release_claims_without_sight`
    /// (`workspaces.allium: ClaimsFollowSight`): the one helper every
    /// loss-of-sight path calls once its change is written. Epic-scoped and
    /// task-scoped claims both.
    pub(super) fn release_claims_without_sight(
        &self,
        tables: &mut Tables,
        epics: &std::collections::HashSet<i64>,
    ) {
        let unseen: Vec<(i64, String)> = tables
            .poll_owners
            .values()
            .filter(|p| p.scope == POLL_SCOPE_EPIC && epics.contains(&p.scope_id))
            .filter_map(|p| {
                let owner = tables.hosts.get(&p.host)?.owner.clone();
                let sees = tables
                    .epics
                    .get(&p.scope_id)
                    .is_some_and(|e| module::epic_visible_to(&*tables, e, &owner));
                (!sees).then_some((p.scope_id, owner))
            })
            .collect();
        let gone: std::collections::HashSet<(i64, String)> = unseen.into_iter().collect();
        self.release_poll_claims(tables, epics, |epic_id, owner| {
            gone.contains(&(epic_id, owner.to_string()))
        });
        // The tasks' epics are judged the same way, on the store as it is now.
        let sight_lost = |tables: &Tables, epic_id: i64, owner: &str| {
            !tables
                .epics
                .get(&epic_id)
                .is_some_and(|e| module::epic_visible_to(tables, e, owner))
        };
        let task_gone: std::collections::HashSet<(i64, String)> = tables
            .poll_owners
            .values()
            .filter(|p| p.scope == POLL_SCOPE_TASK)
            .filter_map(|p| {
                let epic_id = tables.tasks.get(&p.scope_id)?.epic_id;
                let owner = tables.hosts.get(&p.host)?.owner.clone();
                (epics.contains(&epic_id) && sight_lost(tables, epic_id, &owner))
                    .then_some((epic_id, owner))
            })
            .collect();
        self.release_task_poll_claims(tables, epics, |epic_id, owner| {
            task_gone.contains(&(epic_id, owner.to_string()))
        });
    }

    /// Mirrors the module's `on_sight_lost` (task #98992, B1): the one helper
    /// an epic-level loss-of-sight path calls. Claims and marks together.
    pub(super) fn on_sight_lost(
        &self,
        tables: &mut Tables,
        epics: &std::collections::HashSet<i64>,
    ) {
        self.release_claims_without_sight(tables, epics);
        let tasks: std::collections::HashSet<i64> = tables
            .tasks
            .values()
            .filter(|t| epics.contains(&t.epic_id))
            .map(|t| t.id)
            .collect();
        self.drop_marks_of_those_who_cannot_see(tables, &tasks);
    }

    /// Mirrors the module's `on_task_sight_lost`: a single task whose own
    /// sight changed (a move into a private epic, a detach).
    pub(super) fn on_task_sight_lost(&self, tables: &mut Tables, task_id: i64) {
        let Some(task) = tables.tasks.get(&task_id).cloned() else {
            return;
        };
        let stale: Vec<i64> = tables
            .poll_owners
            .values()
            .filter(|p| p.scope == POLL_SCOPE_TASK && p.scope_id == task_id)
            .filter(|p| {
                tables
                    .hosts
                    .get(&p.host)
                    .is_some_and(|h| !module::task_visible_to(&*tables, &task, &h.owner))
            })
            .map(|p| p.id)
            .collect();
        for row_id in stale {
            tables.poll_owners.remove(&row_id);
            self.rows.remove_poll_owner(row_id);
        }
        self.drop_marks_of_those_who_cannot_see(
            tables,
            &std::collections::HashSet::from([task_id]),
        );
    }

    fn drop_marks_of_those_who_cannot_see(
        &self,
        tables: &mut Tables,
        tasks: &std::collections::HashSet<i64>,
    ) {
        if tasks.is_empty() {
            return;
        }
        self.drop_marks_where(tables, |tables, task_id, subscriber| {
            tasks.contains(&task_id)
                && tables
                    .tasks
                    .get(&task_id)
                    .is_some_and(|t| !module::task_visible_to(tables, t, subscriber))
        });
    }

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
        host: String,
        github_path: String,
    ) -> Result<ReducerOutcome> {
        let last_used = encode::stamp(last_used);
        if path.trim().is_empty() {
            return Ok(ReducerOutcome::Refused("repo path is empty".into()));
        }
        let mut tables = self.lock();
        if let Err(why) = self.acting_for(&tables, &host) {
            return Ok(ReducerOutcome::Refused(why));
        }
        // The module's single read of the table.
        let rows: Vec<module::RepoPath> = tables.repo_paths.values().cloned().collect();
        let own = rows
            .iter()
            .find(|r| r.path == path && r.host == host)
            .or_else(|| rows.iter().find(|r| r.path == path && r.host.is_empty()))
            .cloned();
        let row = match own {
            Some(row) => {
                let github_path = if github_path.is_empty() {
                    row.github_path.clone()
                } else {
                    github_path
                };
                let repository =
                    module::repository_verify_command(&rows, row.id, &path, &github_path);
                let verify_command =
                    module::resaved_verify_command(&row, &github_path, &repository);
                module::RepoPath {
                    last_used,
                    host,
                    github_path,
                    verify_command,
                    ..row
                }
            }
            None => {
                let verify_command =
                    module::repository_verify_command(&rows, 0, &path, &github_path);
                module::RepoPath {
                    id: 0,
                    path,
                    last_used,
                    verify_command,
                    host,
                    github_path,
                }
            }
        };
        let id = self.write_repo_path(&mut tables, row.clone());
        let row = module::RepoPath { id, ..row };
        for sibling in module::rows_without_the_repository_command(&rows, &row) {
            self.write_repo_path(
                &mut tables,
                module::RepoPath {
                    verify_command: row.verify_command.clone(),
                    ..sibling.clone()
                },
            );
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_adopt_repo_path(
        &self,
        path: String,
        host: String,
    ) -> Result<ReducerOutcome> {
        if host.is_empty() {
            return Ok(ReducerOutcome::Refused("host is empty".into()));
        }
        let mut tables = self.lock();
        if let Err(why) = self.acting_for(&tables, &host) {
            return Ok(ReducerOutcome::Refused(why));
        }
        let owns = tables
            .repo_paths
            .values()
            .any(|r| r.path == path && r.host == host);
        let existing = tables
            .repo_paths
            .values()
            .find(|r| r.path == path && r.host.is_empty())
            .cloned();
        if let Some(row) = existing {
            if owns {
                if tables.repo_paths.remove(&row.id).is_some() {
                    self.rows.remove_repo_path(row.id);
                }
            } else {
                self.write_repo_path(&mut tables, module::RepoPath { host, ..row });
            }
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_delete_repo_path(
        &self,
        path: String,
        host: String,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        if let Err(why) = self.acting_for(&tables, &host) {
            return Ok(ReducerOutcome::Refused(why));
        }
        let rows: Vec<module::RepoPath> = tables
            .repo_paths
            .values()
            .filter(|r| r.path == path)
            .cloned()
            .collect();
        let mine: Vec<i64> = rows
            .iter()
            .filter(|r| r.host == host || r.host.is_empty())
            .map(|r| r.id)
            .collect();
        if mine.is_empty() {
            return Ok(match rows.first() {
                Some(other) => ReducerOutcome::Refused(format!(
                    "repo path {path} is owned by host {}",
                    other.host
                )),
                None => ReducerOutcome::Applied(vec![]),
            });
        }
        for id in mine {
            if tables.repo_paths.remove(&id).is_some() {
                self.rows.remove_repo_path(id);
            }
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_set_verify_command(
        &self,
        path: String,
        host: String,
        command: String,
    ) -> Result<ReducerOutcome> {
        if command.contains('\n') || command.contains('\r') {
            return Ok(ReducerOutcome::Refused(
                "verify command must be a single line; chain steps with && or ;".into(),
            ));
        }
        let mut tables = self.lock();
        if let Err(why) = self.acting_for(&tables, &host) {
            return Ok(ReducerOutcome::Refused(why));
        }
        let rows: Vec<module::RepoPath> = tables.repo_paths.values().cloned().collect();
        let Some(named) = rows
            .iter()
            .find(|r| r.path == path && r.host == host)
            .or_else(|| rows.iter().find(|r| r.path == path && r.host.is_empty()))
        else {
            return Ok(ReducerOutcome::Refused(format!(
                "no repo path {path} for host {host}"
            )));
        };
        let repository = module::rows_of_the_repository_of(&rows, named);
        for row in repository {
            self.write_repo_path(
                &mut tables,
                module::RepoPath {
                    verify_command: command.clone(),
                    ..row.clone()
                },
            );
        }
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
                        repo_path: repo_path.clone(),
                        branch,
                        last_used,
                    },
                );
            }
        }
        // The module's own cap rule (dispatch.allium: BranchHistoryCapped).
        let stale =
            module::base_branches_beyond_cap(tables.repo_base_branches.values(), &repo_path);
        for id in stale {
            tables.repo_base_branches.remove(&id);
            self.rows.remove_repo_base_branch(id);
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
        let scope = self.scope();
        if let Err(why) = Tables::require_own_identity(&scope, &subscriber) {
            return Ok(ReducerOutcome::Refused(why));
        }
        let mut tables = self.lock();
        if tables.visible_epic(&scope, epic_id).is_none() {
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
        let scope = self.scope();
        if let Err(why) = Tables::require_own_identity(&scope, &subscriber) {
            return Ok(ReducerOutcome::Refused(why));
        }
        let mut tables = self.lock();
        let id = module::subscription_id(&subscriber, epic_id);
        if !tables.subscriptions.contains_key(&id) {
            // The module's sight check: no row and no sight reads as missing.
            if tables.visible_epic(&scope, epic_id).is_none() {
                return Ok(ReducerOutcome::Refused(format!("no epic {epic_id}")));
            }
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
        let released: std::collections::HashSet<i64> =
            module::epics_losing_coverage(epic_id, &remaining, &parents)
                .into_iter()
                .collect();
        self.release_poll_claims(&mut tables, &released, |_, owner| owner == subscriber);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    /// Mirrors the module's `save_setting`: refuses an empty host and one the
    /// sender does not own (D11), otherwise
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
        if let Err(why) = self.acting_for(&tables, &host) {
            return Ok(ReducerOutcome::Refused(why));
        }
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

    /// Mirrors the module's `clear_setting`: refuses a host the sender does not
    /// own (D11); deleting an absent key is a no-op, not a refusal (`docs/specs/settings.allium`'s `ClearSetting`).
    pub(super) fn apply_clear_setting(&self, host: String, key: String) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        if let Err(why) = self.acting_for(&tables, &host) {
            return Ok(ReducerOutcome::Refused(why));
        }
        let id = module::host_scoped_id(&host, &key);
        if tables.settings.remove(&id).is_some() {
            self.rows.remove_setting(id);
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    /// Mirrors the module's `set_managed_feed_settings`: judges every write
    /// first, then applies them all.
    pub(super) fn apply_set_managed_feed_settings(
        &self,
        host: String,
        writes: Vec<bindings::SettingWrite>,
    ) -> Result<ReducerOutcome> {
        if host.trim().is_empty() {
            return Ok(ReducerOutcome::Refused(
                "a host id must not be empty".into(),
            ));
        }
        let mut tables = self.lock();
        if let Err(why) = self.acting_for(&tables, &host) {
            return Ok(ReducerOutcome::Refused(why));
        }
        let writes: Vec<module::SettingWrite> = writes
            .into_iter()
            .map(|w| module::SettingWrite {
                key: w.key,
                value: w.value,
            })
            .collect();
        if let Err(why) = module::check_managed_feed_writes(&writes) {
            return Ok(ReducerOutcome::Refused(why));
        }
        for write in writes {
            let id = module::host_scoped_id(&host, &write.key);
            match write.value {
                Some(value) => {
                    let row = module::Setting {
                        id: id.clone(),
                        host: host.clone(),
                        key: write.key,
                        value,
                    };
                    tables.settings.insert(id, row.clone());
                    self.rows.upsert_setting(&row.into());
                }
                None => {
                    if tables.settings.remove(&id).is_some() {
                        self.rows.remove_setting(id);
                    }
                }
            }
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    /// Mirrors the module's `own_usage_summary` procedure: this caller's
    /// events, grouped.
    pub(super) fn apply_own_usage_summary(
        &self,
        category: Option<String>,
        actor: Option<String>,
        since: Option<String>,
        limit: i64,
    ) -> Vec<bindings::UsageSummaryRow> {
        let tables = self.lock();
        let filter = module::UsageFilter {
            category,
            actor,
            since,
            limit,
        };
        module::usage_summary_for(&*tables, &self.sender, &filter)
            .into_iter()
            .map(|r| bindings::UsageSummaryRow {
                category: r.category,
                action: r.action,
                detail: r.detail,
                actor: r.actor,
                count: r.count,
                last_used: r.last_used,
            })
            .collect()
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
            recorded_by: self.sender.clone(),
            ..module::UsageEvent::from(row)
        };
        tables.usage_events.insert(id, row);
        let threshold = id - cap;
        if threshold > 0 {
            let stale: Vec<i64> = tables
                .usage_events
                .range(..=threshold)
                .map(|(&id, _)| id)
                .collect();
            for stale_id in stale {
                tables.usage_events.remove(&stale_id);
            }
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }
}
