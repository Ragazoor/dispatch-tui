//! Feed ingestion and retired feed items.

use anyhow::Result;

use dispatch_spacetime_module as module;

use crate::models::EpicId;
use crate::spacetime::bindings;

use super::super::encode;
use super::super::writes::ReducerOutcome;
use super::{MemoryReducerCaller, Tables, MAX_EPIC_DEPTH};

impl MemoryReducerCaller {
    /// `delete_watcher_rows`'s `retired_feed_items` twin — shared by
    /// `drop_retired_feed_items_for_epics` and `drop_closed_retired_feed_items`,
    /// which otherwise repeated this identical remove-and-notify loop.
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

    pub(super) fn apply_upsert_feed_tasks(
        &self,
        epic_id: EpicId,
        items: Vec<bindings::FeedTaskUpsertItem>,
        created_by: String,
    ) -> Result<ReducerOutcome> {
        let epic_id = epic_id.0;
        let mut tables = self.lock();
        let items = items
            .into_iter()
            .map(module::FeedTaskUpsertItem::from)
            .collect();
        Ok(self.upsert_feed_tasks_inner(&mut tables, epic_id, items, &created_by, true))
    }

    pub(super) fn apply_upsert_feed_tasks_additive(
        &self,
        epic_id: EpicId,
        items: Vec<bindings::FeedTaskUpsertItem>,
        created_by: String,
    ) -> Result<ReducerOutcome> {
        let epic_id = epic_id.0;
        let mut tables = self.lock();
        let items = items
            .into_iter()
            .map(module::FeedTaskUpsertItem::from)
            .collect();
        Ok(self.upsert_feed_tasks_inner(&mut tables, epic_id, items, &created_by, false))
    }

    pub(super) fn apply_delete_stale_subtree_feed_tasks(
        &self,
        parent_id: EpicId,
        keep_external_ids: Vec<String>,
    ) -> Result<ReducerOutcome> {
        let parent_id = parent_id.0;
        let mut tables = self.lock();
        let keep: std::collections::HashSet<&str> =
            keep_external_ids.iter().map(String::as_str).collect();
        let child_epics: std::collections::HashSet<i64> = tables
            .epics
            .values()
            .filter(|e| e.parent_epic_id == parent_id)
            .map(|e| e.id)
            .collect();
        // One pass over every task, not one pass per child epic — see
        // `retire_feed_tasks_before_epic_delete`'s matching note on why an
        // unindexed store has to merge what a real, per-epic-indexed
        // reducer gets for free.
        let stale: Vec<i64> = tables
            .tasks
            .values()
            .filter(|t| {
                child_epics.contains(&t.epic_id)
                    && !t.external_id.is_empty()
                    && !keep.contains(t.external_id.as_str())
            })
            .map(|t| t.id)
            .collect();
        for id in stale {
            self.delete_task_row(&mut tables, id);
            self.delete_task_side_effects(&mut tables, id);
        }
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_drop_closed_retired_feed_items(
        &self,
        feed_epic_id: EpicId,
        keep_external_ids: Vec<String>,
    ) -> Result<ReducerOutcome> {
        let feed_epic_id = feed_epic_id.0;
        let mut tables = self.lock();
        let keep: std::collections::HashSet<&str> =
            keep_external_ids.iter().map(String::as_str).collect();
        let stale: Vec<i64> = tables
            .retired_feed_items
            .values()
            .filter(|r| r.feed_epic_id == feed_epic_id && !keep.contains(r.external_id.as_str()))
            .map(|r| r.id)
            .collect();
        self.remove_retired_feed_item_rows(&mut tables, stale);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    pub(super) fn apply_create_repo_group_sub_epic(
        &self,
        parent_id: EpicId,
        title: String,
        created_by: String,
    ) -> Result<EpicId> {
        let parent_id = parent_id.0;
        let mut tables = self.lock();
        let existing = tables
            .epics
            .values()
            .find(|e| e.parent_epic_id == parent_id && e.title == title && e.origin == "repo-group")
            .map(|e| e.id);
        if let Some(id) = existing {
            return Ok(EpicId(id));
        }
        let now = self.now();
        Ok(EpicId(self.write_epic(
            &mut tables,
            module::Epic {
                title,
                parent_epic_id: parent_id,
                origin: "repo-group".to_string(),
                created_by,
                created_at: now.clone(),
                updated_at: now,
                ..module::blank_epic()
            },
        )))
    }

    pub(super) fn apply_create_managed_role_epic(
        &self,
        title: String,
        parent_epic_id: Option<EpicId>,
        role: String,
        feed_command: String,
        feed_interval_secs: i64,
        created_by: String,
    ) -> Result<EpicId> {
        let parent_epic_id = encode::epic_ref(parent_epic_id);
        let mut tables = self.lock();
        let existing = tables
            .epics
            .values()
            .find(|e| e.parent_epic_id == parent_epic_id && e.feed_role == role)
            .map(|e| e.id);
        if let Some(id) = existing {
            return Ok(EpicId(id));
        }
        let now = self.now();
        Ok(EpicId(self.write_epic(
            &mut tables,
            module::Epic {
                title,
                parent_epic_id,
                feed_role: role,
                feed_command,
                feed_interval_secs,
                created_by,
                created_at: now.clone(),
                updated_at: now,
                ..module::blank_epic()
            },
        )))
    }
}
