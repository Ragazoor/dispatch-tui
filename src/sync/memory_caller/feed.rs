//! Feed ingestion and retired feed items.

use std::collections::HashSet;

use anyhow::Result;

use dispatch_spacetime_module as module;

use crate::models::EpicId;
use crate::spacetime::bindings;

use super::super::writes::ReducerOutcome;
use super::{MemoryReducerCaller, Tables, MAX_EPIC_DEPTH};

impl MemoryReducerCaller {
    /// The `retired_feed_items` delete helper — shared by
    /// `drop_retired_feed_items_for_epics` and the closed-retirement drop in
    /// `ingest_feed`, which otherwise repeated this identical remove-and-notify
    /// loop.
    pub(super) fn remove_retired_feed_item_rows(&self, tables: &mut Tables, ids: Vec<i64>) {
        for id in ids {
            if tables.retired_feed_items.remove(&id).is_some() {
                self.rows.remove_retired_feed_item(id);
            }
        }
    }

    /// Walk `epic_id`'s ancestry, itself first, for the nearest epic carrying
    /// a `feed_command`. Mirrors the module's own `nearest_feed_epic`.
    pub(super) fn nearest_feed_epic(tables: &Tables, epic_id: i64) -> Option<i64> {
        let mut next = epic_id;
        for _ in 0..MAX_EPIC_DEPTH {
            if next == 0 {
                return None;
            }
            let epic = tables.epics.get(&next)?;
            if !epic.feed_command.is_empty() {
                return Some(epic.id);
            }
            next = epic.parent_epic_id;
        }
        None
    }

    pub(super) fn is_retired(tables: &Tables, feed_epic_id: i64, external_id: &str) -> bool {
        tables
            .retired_feed_items
            .values()
            .any(|r| r.feed_epic_id == feed_epic_id && r.external_id == external_id)
    }

    /// Idempotent insert, mirroring the module's `retire_feed_item` /
    /// `core/RetiredFeedItem: UniqueRetiredFeedItemPerFeed`.
    pub(super) fn retire_feed_item(
        &self,
        tables: &mut Tables,
        feed_epic_id: i64,
        external_id: &str,
    ) {
        if !Self::is_retired(tables, feed_epic_id, external_id) {
            let id = Self::assign_id(0, &mut tables.next_retired_feed_item_id);
            let row = module::RetiredFeedItem {
                id,
                feed_epic_id,
                external_id: external_id.to_string(),
                retired_at: self.now(),
            };
            tables.retired_feed_items.insert(id, row.clone());
            self.rows.upsert_retired_feed_item(&row.into());
        }
    }

    /// `tasks.allium: DeleteTask`'s retirement clause. Mirrors the module's
    /// `retire_task_if_feed_backed`, shared by `delete_task` and
    /// `batch_delete`'s plain-task pass.
    pub(super) fn retire_task_if_feed_backed(
        &self,
        tables: &mut Tables,
        epic_id: i64,
        external_id: &str,
    ) {
        if external_id.is_empty() || epic_id == 0 {
            return;
        }
        if let Some(feed_epic_id) = Self::nearest_feed_epic(tables, epic_id) {
            self.retire_feed_item(tables, feed_epic_id, external_id);
        }
    }

    /// `epics.allium: DeleteEpic`'s retirement clause, over the WHOLE doomed
    /// subtree, before anything in it is deleted. Mirrors the module's
    /// `retire_feed_tasks_before_epic_delete`.
    pub(super) fn retire_feed_tasks_before_epic_delete(
        &self,
        tables: &mut Tables,
        doomed: &std::collections::HashSet<i64>,
    ) {
        // One pass over every task, not one pass per doomed epic id — the
        // module's own version (`retire_feed_tasks_before_epic_delete` in
        // spacetime/module/src/lib.rs) gets this for free from an indexed
        // `epic_id()` filter per epic; this store has no such index, so
        // repeating the filter per id would be a real repeated scan here.
        let tasks: Vec<(i64, String)> = tables
            .tasks
            .values()
            .filter(|t| doomed.contains(&t.epic_id) && !t.external_id.is_empty())
            .map(|t| (t.epic_id, t.external_id.clone()))
            .collect();
        let mut nearest_feed_epic_cache: std::collections::HashMap<i64, Option<i64>> =
            std::collections::HashMap::new();
        for (epic_id, external_id) in tasks {
            let feed_epic_id = *nearest_feed_epic_cache
                .entry(epic_id)
                .or_insert_with(|| Self::nearest_feed_epic(tables, epic_id));
            let Some(feed_epic_id) = feed_epic_id else {
                continue;
            };
            if doomed.contains(&feed_epic_id) {
                continue;
            }
            self.retire_feed_item(tables, feed_epic_id, &external_id);
        }
    }

    /// Drop every stale `retired_feed_items` row for each epic id in
    /// `doomed`. Mirrors the module's `drop_retired_feed_items_for_epics`.
    pub(super) fn drop_retired_feed_items_for_epics(
        &self,
        tables: &mut Tables,
        doomed: &std::collections::HashSet<i64>,
    ) {
        // One pass over every retired-feed-item row, not one pass per doomed
        // epic id — see `retire_feed_tasks_before_epic_delete`'s matching
        // note on why this store needs to merge what an indexed real
        // reducer gets for free.
        let stale: Vec<i64> = tables
            .retired_feed_items
            .values()
            .filter(|r| doomed.contains(&r.feed_epic_id))
            .map(|r| r.id)
            .collect();
        self.remove_retired_feed_item_rows(tables, stale);
    }

    /// Insert or update one feed item under `epic_id`. Mirrors the module's
    /// `upsert_feed_item` field for field — see that function's doc comment
    /// for which fields update on conflict and which are preserved.
    pub(super) fn upsert_feed_item(
        &self,
        tables: &mut Tables,
        epic_id: i64,
        feed_epic_id: Option<i64>,
        item: &module::FeedTaskUpsertItem,
        created_by: &str,
    ) {
        let existing = tables
            .tasks
            .values()
            .find(|t| t.epic_id == epic_id && t.external_id == item.external_id)
            .cloned();

        if existing.is_none() {
            if let Some(feed_epic_id) = feed_epic_id {
                if Self::is_retired(tables, feed_epic_id, &item.external_id) {
                    return;
                }
            }
        }

        let row = match existing {
            Some(existing) => {
                let (url, url_type) = if existing.url.is_empty() {
                    (item.url.clone(), item.url_type.clone())
                } else {
                    (existing.url.clone(), existing.url_type.clone())
                };
                module::Task {
                    title: item.title.clone(),
                    description: item.description.clone(),
                    tag: item.tag.clone(),
                    labels: item.labels.clone(),
                    sort_order: item.sort_order,
                    url,
                    url_type,
                    updated_at: self.now(),
                    ..existing
                }
            }
            None => module::Task {
                id: 0,
                title: item.title.clone(),
                description: item.description.clone(),
                repo_path: item.repo_path.clone(),
                status: item.status.clone(),
                sub_status: item.sub_status.clone(),
                base_branch: item.base_branch.clone(),
                epic_id,
                external_id: item.external_id.clone(),
                tag: item.tag.clone(),
                labels: item.labels.clone(),
                sort_order: item.sort_order,
                url: item.url.clone(),
                url_type: item.url_type.clone(),
                wrap_up_mode: item.wrap_up_mode.clone(),
                created_at: self.now(),
                updated_at: self.now(),
                owner: String::new(),
                created_by: created_by.to_string(),
                ..module::blank_task()
            },
        };
        let _ = self.write_task(tables, row);
    }

    pub(super) fn delete_stale_feed_tasks_in_epic(
        &self,
        tables: &mut Tables,
        epic_id: i64,
        keep: &std::collections::HashSet<&str>,
    ) {
        let stale: Vec<i64> = tables
            .tasks
            .values()
            .filter(|t| {
                t.epic_id == epic_id
                    && !t.external_id.is_empty()
                    && !keep.contains(t.external_id.as_str())
            })
            .map(|t| t.id)
            .collect();
        for id in stale {
            self.delete_task_row(tables, id);
            self.delete_task_side_effects(tables, id);
        }
    }

    pub(super) fn upsert_feed_tasks_inner(
        &self,
        tables: &mut Tables,
        epic_id: i64,
        items: Vec<module::FeedTaskUpsertItem>,
        created_by: &str,
        delete_absent: bool,
    ) -> ReducerOutcome {
        if !tables.epics.contains_key(&epic_id) {
            return ReducerOutcome::Refused(format!(
                "epic {epic_id} not found for upsert_feed_tasks"
            ));
        }
        let feed_epic_id = Self::nearest_feed_epic(tables, epic_id);
        for item in &items {
            self.upsert_feed_item(tables, epic_id, feed_epic_id, item, created_by);
        }
        if delete_absent {
            let keep: std::collections::HashSet<&str> =
                items.iter().map(|i| i.external_id.as_str()).collect();
            self.delete_stale_feed_tasks_in_epic(tables, epic_id, &keep);
        }
        ReducerOutcome::Applied(vec![])
    }

    /// The module's own `ingest_feed` body (`module::ingest_into`), run
    /// against this double's rows through [`TablesIngest`]: the routing and
    /// reconcile decisions are the module's, not a copy of them.
    pub(super) fn apply_ingest_feed(
        &self,
        epic_id: EpicId,
        items: Vec<bindings::FeedIngestItem>,
        additive: bool,
        created_by: String,
    ) -> Result<ReducerOutcome> {
        let items: Vec<module::FeedIngestItem> = items
            .into_iter()
            .map(module::FeedIngestItem::from)
            .collect();
        let mut tables = self.lock();
        let mut store = TablesIngest {
            caller: self,
            tables: &mut tables,
        };
        Ok(
            match module::ingest_into(&mut store, epic_id.0, items, additive, &created_by) {
                Ok(()) => ReducerOutcome::Applied(vec![]),
                Err(why) => ReducerOutcome::Refused(why),
            },
        )
    }

    pub(super) fn apply_ensure_managed_epics(
        &self,
        reviews_command: String,
        reviews_interval_secs: i64,
        cve_command: String,
        cve_interval_secs: i64,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        if !reviews_command.is_empty() {
            let parent = self.ensure_role_epic(
                &mut tables,
                module::ROLE_REVIEWS_PARENT,
                0,
                Some((&reviews_command, reviews_interval_secs)),
            );
            for role in [
                module::ROLE_MY_REVIEWS,
                module::ROLE_TEAM_REVIEWS,
                module::ROLE_BOTS,
            ] {
                self.ensure_role_epic(&mut tables, role, parent, None);
            }
        }
        if !cve_command.is_empty() {
            self.ensure_role_epic(
                &mut tables,
                module::ROLE_CVE,
                0,
                Some((&cve_command, cve_interval_secs)),
            );
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    /// Mirrors the module's `ensure_role_epic`: found by `(role, parent)` and
    /// left alone but for the command and interval of a command-carrying role,
    /// else created. Its id.
    fn ensure_role_epic(
        &self,
        tables: &mut Tables,
        role: &str,
        parent_epic_id: i64,
        command: Option<(&str, i64)>,
    ) -> i64 {
        let existing = tables
            .epics
            .values()
            .find(|e| e.parent_epic_id == parent_epic_id && e.feed_role == role)
            .cloned();
        if let Some(epic) = existing {
            if let Some((cmd, interval)) = command {
                if epic.feed_command != cmd || epic.feed_interval_secs != interval {
                    let id = epic.id;
                    self.write_epic(
                        tables,
                        module::Epic {
                            feed_command: cmd.to_string(),
                            feed_interval_secs: interval,
                            updated_at: self.now(),
                            ..epic
                        },
                    );
                    return id;
                }
            }
            return epic.id;
        }
        let (feed_command, feed_interval_secs) =
            command.map(|(c, i)| (c.to_string(), i)).unwrap_or_default();
        let now = self.now();
        self.write_epic(
            tables,
            module::Epic {
                title: managed_role_title(role).to_string(),
                parent_epic_id,
                feed_role: role.to_string(),
                feed_command,
                feed_interval_secs,
                created_by: self.sender.clone(),
                created_at: now.clone(),
                updated_at: now,
                ..module::blank_epic()
            },
        )
    }
}

/// The module's `managed_role_title` (private there): the default title of a
/// freshly created managed epic.
fn managed_role_title(role: &str) -> &'static str {
    match role {
        module::ROLE_REVIEWS_PARENT => "PR Reviews",
        module::ROLE_MY_REVIEWS => "My Reviews",
        module::ROLE_TEAM_REVIEWS => "Team Reviews",
        module::ROLE_BOTS => "Bots",
        module::ROLE_CVE => "CVE",
        _ => "Reviews",
    }
}

/// [`module::IngestStore`] over this double's rows, so `ingest_feed` runs the
/// module's own body. Each method is the double's twin of the module helper
/// of the same name.
struct TablesIngest<'a> {
    caller: &'a MemoryReducerCaller,
    tables: &'a mut Tables,
}

impl module::IngestStore for TablesIngest<'_> {
    fn epic(&self, id: i64) -> Option<module::Epic> {
        self.tables.epics.get(&id).cloned()
    }

    fn child_epics(&self, parent: i64) -> Vec<module::Epic> {
        self.tables
            .epics
            .values()
            .filter(|e| e.parent_epic_id == parent)
            .cloned()
            .collect()
    }

    fn tasks_in_epic(&self, epic_id: i64) -> Vec<module::Task> {
        self.tables
            .tasks
            .values()
            .filter(|t| t.epic_id == epic_id)
            .cloned()
            .collect()
    }

    fn subtree_ids(&self, root: i64) -> HashSet<i64> {
        MemoryReducerCaller::collect_epic_subtree_ids(self.tables, root)
    }

    fn is_retired(&self, feed_epic_id: i64, external_id: &str) -> bool {
        MemoryReducerCaller::is_retired(self.tables, feed_epic_id, external_id)
    }

    fn identity(&self) -> String {
        self.caller.sender.clone()
    }

    fn now(&self) -> String {
        self.caller.now()
    }

    fn insert_epic(&mut self, row: module::Epic) -> i64 {
        self.caller.write_epic(self.tables, row)
    }

    fn update_task(&mut self, row: module::Task) {
        self.tables.tasks.insert(row.id, row.clone());
        self.caller.rows.upsert_task(&row.into());
    }

    fn find_or_create_repo_group(&mut self, root: i64, title: &str, created_by: String) -> i64 {
        self.caller
            .find_or_create_repo_group(self.tables, root, title, created_by)
    }

    fn flatten_epic(&mut self, root: i64) -> Result<(), String> {
        self.caller.flatten_epic_rows(self.tables, root)
    }

    fn drop_closed_retired(&mut self, feed_epic_id: i64, keep: &HashSet<&str>) {
        let stale: Vec<i64> = self
            .tables
            .retired_feed_items
            .values()
            .filter(|r| r.feed_epic_id == feed_epic_id && !keep.contains(r.external_id.as_str()))
            .map(|r| r.id)
            .collect();
        self.caller
            .remove_retired_feed_item_rows(self.tables, stale);
    }

    fn upsert_feed_tasks(
        &mut self,
        epic_id: i64,
        items: Vec<module::FeedTaskUpsertItem>,
        created_by: &str,
        delete_absent: bool,
    ) -> Result<(), String> {
        match self.caller.upsert_feed_tasks_inner(
            self.tables,
            epic_id,
            items,
            created_by,
            delete_absent,
        ) {
            ReducerOutcome::Refused(why) => Err(why),
            _ => Ok(()),
        }
    }

    fn delete_stale_feed_tasks_in_epic(&mut self, epic_id: i64, keep: &HashSet<&str>) {
        self.caller
            .delete_stale_feed_tasks_in_epic(self.tables, epic_id, keep);
    }

    fn recalculate_epic_chain(&mut self, epic_id: i64) {
        self.caller.recalculate_epic_chain(self.tables, epic_id);
    }
}
