//! Who may see and change what: this store's reimplementation of
//! `spacetime/module/src/visibility.rs` (`workspaces.allium`: the read
//! predicate, `WritesRequireSight`).
//!
//! OFF BY DEFAULT. A store built with [`MemoryReducerCaller::new`] runs as
//! the board did before workspaces existed: nobody is refused, every epic is
//! seen by everyone. Most board tests are about something else and set up no
//! workspace, so enforcing here would make each of them build one. A test of
//! the rules, and the conformance scenarios that pair this store with the
//! module, turn it on with [`MemoryReducerCaller::with_workspace_rules`].

use anyhow::Result;

use dispatch_spacetime_module as module;

use crate::models::{EpicId, WorkPackageId};

use super::super::writes::ReducerOutcome;
use super::{MemoryReducerCaller, Tables};

/// The rules in force for one call: whether they are on, and who is calling.
#[derive(Clone)]
pub(super) struct Scope {
    on: bool,
    who: String,
}

/// The most parents a walk up an epic chain follows; the module's own bound.
const WALK_LIMIT: usize = 64;

impl MemoryReducerCaller {
    /// Turn the workspace rules on: every write checks sight, a root epic or
    /// package names a workspace its caller belongs to, and an item the caller
    /// cannot see is answered as one that does not exist.
    pub fn with_workspace_rules(mut self) -> Self {
        self.workspace_rules = true;
        self
    }

    pub(super) fn scope(&self) -> Scope {
        Scope {
            on: self.workspace_rules,
            who: self.sender.clone(),
        }
    }
}

impl Scope {
    pub(super) fn on(&self) -> bool {
        self.on
    }
}

impl Tables {
    fn has_member(&self, workspace_id: i64, who: &str) -> bool {
        self.memberships
            .values()
            .any(|m| m.workspace_id == workspace_id && m.user == who)
    }

    pub(super) fn is_workspace_admin(&self, workspace_id: i64, who: &str) -> bool {
        self.memberships.values().any(|m| {
            m.workspace_id == workspace_id && m.user == who && m.role == module::ROLE_ADMIN
        })
    }

    pub(super) fn is_super_admin(&self, who: &str) -> bool {
        self.user_profiles
            .get(who)
            .is_some_and(|p| p.is_super_admin)
    }

    /// `Epic.private_holder`: the creator of the nearest private epic at or
    /// above `epic`, `None` when public all the way up.
    fn private_holder(&self, epic: &module::Epic) -> Option<String> {
        let mut current = epic.clone();
        for _ in 0..WALK_LIMIT {
            if current.visibility == module::VISIBILITY_PRIVATE {
                return Some(current.created_by);
            }
            if current.parent_epic_id == 0 {
                return None;
            }
            current = self.epics.get(&current.parent_epic_id)?.clone();
        }
        None
    }

    pub(super) fn is_private(&self, epic: &module::Epic) -> bool {
        self.private_holder(epic).is_some()
    }

    pub(super) fn epic_visible(&self, scope: &Scope, epic: &module::Epic) -> bool {
        if !scope.on {
            return true;
        }
        if !self.has_member(epic.workspace_id, &scope.who) {
            return false;
        }
        match self.private_holder(epic) {
            None => true,
            Some(holder) => !holder.is_empty() && holder == scope.who,
        }
    }

    pub(super) fn task_visible(&self, scope: &Scope, task: &module::Task) -> bool {
        if !scope.on {
            return true;
        }
        if task.epic_id == 0 {
            return !task.owner.is_empty() && task.owner == scope.who;
        }
        self.epics
            .get(&task.epic_id)
            .is_some_and(|epic| self.epic_visible(scope, epic))
    }

    pub(super) fn package_visible(&self, scope: &Scope, package: &module::WorkPackage) -> bool {
        if !scope.on {
            return true;
        }
        self.has_member(package.workspace_id, &scope.who)
            && (package.visibility == module::VISIBILITY_PUBLIC || package.created_by == scope.who)
    }

    pub(super) fn learning_visible(&self, scope: &Scope, learning: &module::Learning) -> bool {
        if !scope.on {
            return true;
        }
        let item_id = learning
            .scope_ref
            .as_deref()
            .and_then(|r| r.parse::<i64>().ok());
        let item_seen = match (learning.scope.as_str(), item_id) {
            ("epic", Some(id)) => self
                .epics
                .get(&id)
                .is_none_or(|e| self.epic_visible(scope, e)),
            ("task", Some(id)) => self
                .tasks
                .get(&id)
                .is_none_or(|t| self.task_visible(scope, t)),
            _ => true,
        };
        item_seen
            && (learning.workspace_id == 0 || self.has_member(learning.workspace_id, &scope.who))
    }

    /// The task `id`, when it exists and the caller may see it.
    pub(super) fn visible_task(&self, scope: &Scope, id: i64) -> Option<module::Task> {
        self.tasks
            .get(&id)
            .filter(|t| self.task_visible(scope, t))
            .cloned()
    }

    pub(super) fn visible_epic(&self, scope: &Scope, id: i64) -> Option<module::Epic> {
        self.epics
            .get(&id)
            .filter(|e| self.epic_visible(scope, e))
            .cloned()
    }

    pub(super) fn visible_package(&self, scope: &Scope, id: i64) -> Option<module::WorkPackage> {
        self.work_packages
            .get(&id)
            .filter(|p| self.package_visible(scope, p))
            .cloned()
    }

    pub(super) fn visible_learning(&self, scope: &Scope, id: i64) -> Option<module::Learning> {
        self.learnings
            .get(&id)
            .filter(|l| self.learning_visible(scope, l))
            .cloned()
    }

    /// `TaskEntersAnEpicTheCallerCanSee`.
    pub(super) fn check_task_may_enter_epic(
        &self,
        scope: &Scope,
        label: &str,
        epic_id: i64,
    ) -> Result<(), String> {
        if epic_id == 0 || self.visible_epic(scope, epic_id).is_some() {
            Ok(())
        } else {
            Err(format!("{label}: epic {epic_id} does not exist"))
        }
    }

    /// `CreateEpicInWorkspace`: the row a new epic is stored as.
    pub(super) fn settle_new_epic(
        &self,
        scope: &Scope,
        row: module::Epic,
    ) -> Result<module::Epic, String> {
        module::validate_visibility_value(&row.visibility)?;
        if !scope.on {
            return Ok(row);
        }
        let (workspace_id, parent_id) = if row.parent_epic_id != 0 {
            let Some(parent) = self.visible_epic(scope, row.parent_epic_id) else {
                return Err(format!("epic {} does not exist", row.parent_epic_id));
            };
            (parent.workspace_id, parent.id)
        } else {
            if !self.workspaces.contains_key(&row.workspace_id) {
                return Err(format!("workspace {} does not exist", row.workspace_id));
            }
            (row.workspace_id, 0)
        };
        if !self.has_member(workspace_id, &scope.who) {
            return Err(format!("workspace {workspace_id} does not exist"));
        }
        let created_by = if row.created_by.is_empty() {
            scope.who.clone()
        } else {
            row.created_by.clone()
        };
        if row.visibility == module::VISIBILITY_PRIVATE && created_by.is_empty() {
            return Err("a private epic needs a creator".into());
        }
        Ok(module::Epic {
            workspace_id,
            parent_epic_id: parent_id,
            created_by,
            ..row
        })
    }

    /// `CreateWorkPackageInWorkspace`.
    pub(super) fn check_new_package(
        &self,
        scope: &Scope,
        row: &module::WorkPackage,
    ) -> Result<(), String> {
        module::validate_visibility_value(&row.visibility)?;
        if scope.on
            && (!self.workspaces.contains_key(&row.workspace_id)
                || !self.has_member(row.workspace_id, &scope.who))
        {
            return Err(format!("workspace {} does not exist", row.workspace_id));
        }
        Ok(())
    }

    /// `EpicStaysInItsWorkspace`.
    pub(super) fn check_epic_move(
        &self,
        scope: &Scope,
        epic: &module::Epic,
        new_parent: i64,
    ) -> Result<(), String> {
        if !scope.on || new_parent == 0 {
            return Ok(());
        }
        let Some(destination) = self.visible_epic(scope, new_parent) else {
            return Err(format!("epic {new_parent} does not exist"));
        };
        if destination.workspace_id != epic.workspace_id {
            return Err("an epic never moves to another workspace".into());
        }
        Ok(())
    }

    fn item_workspace(&self, kind: &str, item_id: i64) -> Option<i64> {
        match kind {
            module::WORK_PACKAGE_ITEM_EPIC => self.epics.get(&item_id).map(|e| e.workspace_id),
            module::WORK_PACKAGE_ITEM_TASK => {
                let task = self.tasks.get(&item_id)?;
                self.epics.get(&task.epic_id).map(|e| e.workspace_id)
            }
            _ => None,
        }
    }

    /// `LinkOnlyInsideTheWorkspace`.
    pub(super) fn check_link_allowed(
        &self,
        scope: &Scope,
        package: &module::WorkPackage,
        kind: &str,
        item_id: i64,
    ) -> Result<(), String> {
        if !scope.on {
            return Ok(());
        }
        let seen = match kind {
            module::WORK_PACKAGE_ITEM_TASK => self.visible_task(scope, item_id).is_some(),
            module::WORK_PACKAGE_ITEM_EPIC => self.visible_epic(scope, item_id).is_some(),
            other => return Err(format!("unknown work package item kind {other:?}")),
        };
        if !seen {
            return Err(format!("no {kind} {item_id}"));
        }
        match self.item_workspace(kind, item_id) {
            Some(workspace) if workspace != package.workspace_id => Err(format!(
                "a work package links only to items in its own workspace; {kind} {item_id} is elsewhere"
            )),
            _ => Ok(()),
        }
    }

    /// The module's `check_subtree_all_seen`.
    pub(super) fn check_subtree_all_seen(
        &self,
        scope: &Scope,
        id: i64,
        doomed: &std::collections::HashSet<i64>,
    ) -> Result<(), String> {
        if !scope.on {
            return Ok(());
        }
        let all_seen = doomed.iter().all(|e| {
            self.epics
                .get(e)
                .is_none_or(|e| self.epic_visible(scope, e))
        });
        if all_seen {
            Ok(())
        } else {
            Err(format!("epic {id}: cannot delete right now"))
        }
    }
}

fn refused(why: impl Into<String>) -> Result<ReducerOutcome> {
    Ok(ReducerOutcome::Refused(why.into()))
}

impl MemoryReducerCaller {
    /// Whether the caller may change an item's visibility: its creator, an
    /// admin of its workspace, or a super-admin.
    fn may_set_visibility(&self, tables: &Tables, created_by: &str, workspace_id: i64) -> bool {
        created_by == self.sender
            || tables.is_workspace_admin(workspace_id, &self.sender)
            || tables.is_super_admin(&self.sender)
    }

    /// Mirrors the module's `set_epic_visibility`.
    pub(super) fn apply_set_epic_visibility(
        &self,
        id: EpicId,
        visibility: String,
    ) -> Result<ReducerOutcome> {
        if let Err(why) = module::validate_visibility_value(&visibility) {
            return refused(why);
        }
        let scope = self.scope();
        let mut tables = self.lock();
        let Some(mut epic) = tables.visible_epic(&scope, id.0) else {
            return refused(format!("epic {} does not exist", id.0));
        };
        if scope.on() && !self.may_set_visibility(&tables, &epic.created_by, epic.workspace_id) {
            return refused("only the creator or an admin may change an epic's visibility");
        }
        if visibility == module::VISIBILITY_PRIVATE && epic.created_by.is_empty() {
            return refused("a private epic needs a creator");
        }
        epic.visibility = visibility;
        epic.updated_at = self.now();
        self.write_epic(&mut tables, epic);
        Ok(ReducerOutcome::Applied(vec![]))
    }

    /// Mirrors the module's `set_work_package_visibility`.
    pub(super) fn apply_set_work_package_visibility(
        &self,
        id: WorkPackageId,
        visibility: String,
    ) -> Result<ReducerOutcome> {
        if let Err(why) = module::validate_visibility_value(&visibility) {
            return refused(why);
        }
        let scope = self.scope();
        let mut tables = self.lock();
        let Some(mut package) = tables.visible_package(&scope, id.0) else {
            return refused(format!("no work package {id}"));
        };
        if scope.on()
            && !self.may_set_visibility(&tables, &package.created_by, package.workspace_id)
        {
            return refused("only the creator or an admin may change a package's visibility");
        }
        package.visibility = visibility;
        package.updated_at = self.now();
        tables.work_packages.insert(id.0, package.clone());
        self.rows.upsert_work_package(&package.into());
        Ok(ReducerOutcome::Applied(vec![]))
    }

    /// Mirrors the module's `super_admin_delete_unseen_item`.
    pub(super) fn apply_super_admin_delete_unseen_item(
        &self,
        kind: String,
        item_id: i64,
    ) -> Result<ReducerOutcome> {
        let mut tables = self.lock();
        if !tables.is_super_admin(&self.sender) {
            return refused("only a super-admin may do this");
        }
        match kind.as_str() {
            module::WORK_PACKAGE_ITEM_EPIC => {
                let Some(epic) = tables.epics.get(&item_id).cloned() else {
                    return refused(format!("no epic {item_id}"));
                };
                if !tables.is_private(&epic) {
                    return refused("only a private item is deleted this way");
                }
                let doomed = Self::collect_epic_subtree_ids(&tables, item_id);
                if let Some(why) = Self::subtree_too_deep(&tables, item_id) {
                    return refused(why);
                }
                if let Some(undone) = Self::first_undone_task_in(&tables, &doomed) {
                    return refused(format!(
                        "epic {item_id}: cannot delete while task {undone} in its subtree is not done"
                    ));
                }
                self.delete_epic_subtree_with_feed_cleanup(&mut tables, item_id, &doomed);
                self.recalculate_epic_chain(&mut tables, epic.parent_epic_id);
                Ok(ReducerOutcome::Applied(vec![]))
            }
            "work_package" => {
                let Some(package) = tables.work_packages.get(&item_id).cloned() else {
                    return refused(format!("no work package {item_id}"));
                };
                if package.visibility != module::VISIBILITY_PRIVATE {
                    return refused("only a private item is deleted this way");
                }
                self.delete_package_rows(&mut tables, WorkPackageId(item_id));
                Ok(ReducerOutcome::Applied(vec![]))
            }
            other => refused(format!(
                "an unseen item is an epic or a work_package, not {other:?}"
            )),
        }
    }
}
