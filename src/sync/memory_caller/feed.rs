//! Feed ingestion and retired feed items.

use anyhow::Result;

use dispatch_spacetime_module as module;

use crate::models::EpicId;
use crate::spacetime::bindings;

use super::super::encode;
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

    /// Mirrors the module's `ingest_feed` (`spacetime/module/src/feed_ingest.rs`),
    /// phase for phase: see there for why each step is where it is.
    pub(super) fn apply_ingest_feed(
        &self,
        epic_id: EpicId,
        items: Vec<bindings::FeedIngestItem>,
        additive: bool,
        created_by: String,
    ) -> Result<ReducerOutcome> {
        let epic_id = epic_id.0;
        let items: Vec<module::FeedIngestItem> = items
            .into_iter()
            .map(module::FeedIngestItem::from)
            .collect();
        let mut tables = self.lock();
        let Some(epic) = tables.epics.get(&epic_id).cloned() else {
            return Ok(ReducerOutcome::Refused(format!(
                "epic {epic_id} not found for ingest_feed"
            )));
        };
        let result = match epic.feed_role.as_str() {
            module::ROLE_REVIEWS_PARENT => {
                self.ingest_role_routed(&mut tables, &epic, items, additive, &created_by)
            }
            module::ROLE_MY_REVIEWS | module::ROLE_TEAM_REVIEWS | module::ROLE_BOTS => {
                Err(module::ROLE_SUB_EPIC_FEED_REFUSED.to_string())
            }
            _ if epic.group_by_repo => {
                self.ingest_grouped(&mut tables, &epic, items, additive, &created_by)
            }
            _ => self.ingest_flat(&mut tables, &epic, items, additive, &created_by),
        };
        Ok(match result {
            Ok(()) => ReducerOutcome::Applied(vec![]),
            Err(why) => ReducerOutcome::Refused(why),
        })
    }

    fn drop_closed_retired(
        &self,
        tables: &mut Tables,
        feed_epic_id: i64,
        keep: &std::collections::HashSet<&str>,
    ) {
        let stale: Vec<i64> = tables
            .retired_feed_items
            .values()
            .filter(|r| r.feed_epic_id == feed_epic_id && !keep.contains(r.external_id.as_str()))
            .map(|r| r.id)
            .collect();
        self.remove_retired_feed_item_rows(tables, stale);
    }

    fn ingest_flat(
        &self,
        tables: &mut Tables,
        epic: &module::Epic,
        items: Vec<module::FeedIngestItem>,
        additive: bool,
        created_by: &str,
    ) -> Result<(), String> {
        let has_repo_group = tables
            .epics
            .values()
            .any(|e| e.parent_epic_id == epic.id && e.origin == module::REPO_GROUP_ORIGIN);
        if has_repo_group {
            self.flatten_epic_rows(tables, epic.id)?;
        }
        if !additive {
            let keep: std::collections::HashSet<&str> =
                items.iter().map(|i| i.item.external_id.as_str()).collect();
            self.drop_closed_retired(tables, epic.id, &keep);
        }
        let upserts = items.into_iter().map(|i| i.item).collect();
        if let ReducerOutcome::Refused(why) =
            self.upsert_feed_tasks_inner(tables, epic.id, upserts, created_by, !additive)
        {
            return Err(why);
        }
        self.recalculate_epic_chain(tables, epic.id);
        Ok(())
    }

    fn ingest_grouped(
        &self,
        tables: &mut Tables,
        epic: &module::Epic,
        items: Vec<module::FeedIngestItem>,
        additive: bool,
        created_by: &str,
    ) -> Result<(), String> {
        if !additive {
            let keep: std::collections::HashSet<&str> =
                items.iter().map(|i| i.item.external_id.as_str()).collect();
            self.drop_closed_retired(tables, epic.id, &keep);
        }
        let mut groups: Vec<(String, Vec<module::FeedTaskUpsertItem>)> = Vec::new();
        for entry in items {
            let name = module::repo_name_from_url(&entry.item.url);
            match groups.iter_mut().find(|(n, _)| *n == name) {
                Some((_, group)) => group.push(entry.item),
                None => groups.push((name, vec![entry.item])),
            }
        }
        let subs: Vec<module::Epic> = tables
            .epics
            .values()
            .filter(|e| e.parent_epic_id == epic.id)
            .cloned()
            .collect();
        let group_names: std::collections::HashSet<&str> =
            groups.iter().map(|(n, _)| n.as_str()).collect();
        let candidates: Vec<&str> = groups
            .iter()
            .filter(|(name, _)| !subs.iter().any(|s| s.title == *name))
            .flat_map(|(_, group)| group.iter().map(|i| i.external_id.as_str()))
            .collect();
        let dropped = Self::retired_without_task(tables, epic.id, &candidates);

        for (name, group) in &groups {
            let existing = subs.iter().find(|s| s.title == *name).map(|s| s.id);
            let group: Vec<module::FeedTaskUpsertItem> = match existing {
                Some(_) => group.clone(),
                None => group
                    .iter()
                    .filter(|i| !dropped.contains(i.external_id.as_str()))
                    .cloned()
                    .collect(),
            };
            if group.is_empty() {
                continue;
            }
            let sub_id = match existing {
                Some(id) => id,
                None => {
                    let now = self.now();
                    self.write_epic(
                        tables,
                        module::Epic {
                            title: name.clone(),
                            parent_epic_id: epic.id,
                            created_by: self.sender.clone(),
                            created_at: now.clone(),
                            updated_at: now,
                            ..module::blank_epic()
                        },
                    )
                }
            };
            if let ReducerOutcome::Refused(why) =
                self.upsert_feed_tasks_inner(tables, sub_id, group, created_by, !additive)
            {
                return Err(why);
            }
            self.recalculate_epic_chain(tables, sub_id);
        }

        if !additive {
            for sub in subs
                .iter()
                .filter(|s| !group_names.contains(s.title.as_str()))
            {
                if let ReducerOutcome::Refused(why) =
                    self.upsert_feed_tasks_inner(tables, sub.id, Vec::new(), created_by, true)
                {
                    return Err(why);
                }
                self.recalculate_epic_chain(tables, sub.id);
            }
            if let ReducerOutcome::Refused(why) =
                self.upsert_feed_tasks_inner(tables, epic.id, Vec::new(), created_by, true)
            {
                return Err(why);
            }
            self.recalculate_epic_chain(tables, epic.id);
        }
        Ok(())
    }

    /// Mirrors the module's `retired_without_task`.
    fn retired_without_task<'a>(
        tables: &Tables,
        feed_epic_id: i64,
        candidates: &[&'a str],
    ) -> std::collections::HashSet<&'a str> {
        if candidates.is_empty() {
            return std::collections::HashSet::new();
        }
        let subtree = Self::collect_epic_subtree_ids(tables, feed_epic_id);
        let survivors: std::collections::HashSet<&str> = tables
            .tasks
            .values()
            .filter(|t| subtree.contains(&t.epic_id) && !t.external_id.is_empty())
            .map(|t| t.external_id.as_str())
            .collect();
        candidates
            .iter()
            .copied()
            .filter(|c| Self::is_retired(tables, feed_epic_id, c) && !survivors.contains(*c))
            .collect()
    }

    fn ensure_role_sub_epic(
        &self,
        tables: &mut Tables,
        parent: i64,
        role: &str,
        title: &str,
    ) -> i64 {
        let existing = tables
            .epics
            .values()
            .find(|e| e.parent_epic_id == parent && e.feed_role == role)
            .map(|e| e.id);
        if let Some(id) = existing {
            return id;
        }
        let now = self.now();
        self.write_epic(
            tables,
            module::Epic {
                title: title.to_string(),
                parent_epic_id: parent,
                feed_role: role.to_string(),
                created_by: self.sender.clone(),
                created_at: now.clone(),
                updated_at: now,
                ..module::blank_epic()
            },
        )
    }

    fn ingest_role_routed(
        &self,
        tables: &mut Tables,
        parent: &module::Epic,
        items: Vec<module::FeedIngestItem>,
        additive: bool,
        created_by: &str,
    ) -> Result<(), String> {
        use std::collections::{HashMap, HashSet};
        let my =
            self.ensure_role_sub_epic(tables, parent.id, module::ROLE_MY_REVIEWS, "My Reviews");
        let team =
            self.ensure_role_sub_epic(tables, parent.id, module::ROLE_TEAM_REVIEWS, "Team Reviews");
        let bots = self.ensure_role_sub_epic(tables, parent.id, module::ROLE_BOTS, "Bots");
        let roles = [my, team, bots];

        let mut existing: HashMap<String, module::Task> = HashMap::new();
        let mut pre_existing_repo_groups: Vec<i64> = Vec::new();
        let remember =
            |tables: &Tables, existing: &mut HashMap<String, module::Task>, epic: i64| {
                for task in tables.tasks.values().filter(|t| t.epic_id == epic) {
                    if !task.external_id.is_empty() {
                        existing.insert(task.external_id.clone(), task.clone());
                    }
                }
            };
        remember(tables, &mut existing, parent.id);
        for &sub in &roles {
            let children: Vec<i64> = tables
                .epics
                .values()
                .filter(|e| e.parent_epic_id == sub)
                .map(|e| e.id)
                .collect();
            for child in children {
                remember(tables, &mut existing, child);
                pre_existing_repo_groups.push(child);
            }
        }
        for &sub in &roles {
            remember(tables, &mut existing, sub);
        }

        if !additive {
            let keep: HashSet<&str> = items.iter().map(|i| i.item.external_id.as_str()).collect();
            self.drop_closed_retired(tables, parent.id, &keep);
        }

        let items: Vec<module::FeedIngestItem> = items
            .into_iter()
            .filter(|i| !module::excluded_from_reviews(&i.signals))
            .collect();
        let candidates: Vec<&str> = items.iter().map(|i| i.item.external_id.as_str()).collect();
        let dropped = Self::retired_without_task(tables, parent.id, &candidates);

        let mut groups: HashMap<i64, Vec<module::FeedTaskUpsertItem>> = HashMap::new();
        let mut all_external_ids: HashSet<String> = HashSet::new();
        let mut repo_group_cache: HashMap<(i64, String), i64> = HashMap::new();
        for entry in &items {
            let item = &entry.item;
            if dropped.contains(item.external_id.as_str()) {
                continue;
            }
            let role_target = match module::route_signals(&entry.signals) {
                module::ROLE_TEAM_REVIEWS => team,
                module::ROLE_BOTS => bots,
                _ => my,
            };
            let groups_by_repo = tables
                .epics
                .get(&role_target)
                .is_some_and(|e| module::can_auto_group(e.group_by_repo, &e.feed_command));
            let target = if groups_by_repo {
                let name = module::repo_name_from_url(&item.url);
                match repo_group_cache.get(&(role_target, name.clone())) {
                    Some(&id) => id,
                    None => {
                        let id = self.find_or_create_repo_group(
                            tables,
                            role_target,
                            &name,
                            self.sender.clone(),
                        );
                        repo_group_cache.insert((role_target, name), id);
                        id
                    }
                }
            } else {
                role_target
            };

            all_external_ids.insert(item.external_id.clone());

            if let Some(task) = existing.get(&item.external_id) {
                if task.epic_id != target {
                    let moved = module::Task {
                        epic_id: target,
                        updated_at: self.now(),
                        ..task.clone()
                    };
                    tables.tasks.insert(moved.id, moved.clone());
                    self.rows.upsert_task(&moved.into());
                }
            }
            groups.entry(target).or_default().push(item.clone());
        }

        for (sub, group) in groups {
            if let ReducerOutcome::Refused(why) =
                self.upsert_feed_tasks_inner(tables, sub, group, created_by, !additive)
            {
                return Err(why);
            }
        }
        if !additive {
            let keep: HashSet<&str> = all_external_ids.iter().map(String::as_str).collect();
            self.delete_stale_in_children(tables, parent.id, &keep);
            for &sub in &roles {
                self.delete_stale_in_children(tables, sub, &keep);
            }
            self.delete_stale_feed_tasks_in_epic(tables, parent.id, &HashSet::new());
        }

        let repo_groups: HashSet<i64> = repo_group_cache
            .into_values()
            .chain(pre_existing_repo_groups)
            .collect();
        for id in repo_groups {
            self.recalculate_epic_chain(tables, id);
        }
        for &sub in &roles {
            self.recalculate_epic_chain(tables, sub);
        }
        self.recalculate_epic_chain(tables, parent.id);
        Ok(())
    }

    fn delete_stale_in_children(
        &self,
        tables: &mut Tables,
        parent_id: i64,
        keep: &std::collections::HashSet<&str>,
    ) {
        let children: Vec<i64> = tables
            .epics
            .values()
            .filter(|e| e.parent_epic_id == parent_id)
            .map(|e| e.id)
            .collect();
        for child in children {
            self.delete_stale_feed_tasks_in_epic(tables, child, keep);
        }
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

    pub(super) fn apply_create_repo_group_sub_epic(
        &self,
        parent_id: EpicId,
        title: String,
        created_by: String,
    ) -> Result<EpicId> {
        let mut tables = self.lock();
        Ok(EpicId(self.find_or_create_repo_group(
            &mut tables,
            parent_id.0,
            &title,
            created_by,
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
