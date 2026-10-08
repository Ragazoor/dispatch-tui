use anyhow::Result;

use crate::models::{Epic, EpicId, Task, TaskId};
use crate::spacetime::bindings;
use crate::sync::encode;

use super::super::{EpicPatch, Store};

#[async_trait::async_trait]
impl super::super::EpicRead for Store {
    async fn get_epic(&self, id: EpicId) -> Result<Option<Epic>> {
        Ok(self.rows.epic(id))
    }

    async fn list_epics(&self) -> Result<Vec<Epic>> {
        Ok(self.rows.epics())
    }

    async fn list_root_epics(&self) -> Result<Vec<Epic>> {
        Ok(self.rows.epics_with_parent(None))
    }

    async fn list_sub_epics(&self, parent_id: EpicId) -> Result<Vec<Epic>> {
        Ok(self.rows.epics_with_parent(Some(parent_id)))
    }

    async fn list_tasks_for_epic(&self, epic_id: EpicId) -> Result<Vec<Task>> {
        Ok(self.rows.tasks_for_epic(epic_id))
    }

    async fn list_undecodable_task_ids_for_epic(&self, epic_id: EpicId) -> Result<Vec<TaskId>> {
        Ok(self.rows.undecodable_task_ids_for_epic(epic_id))
    }

    async fn list_all_tasks_with_epic_id(&self) -> Result<Vec<Task>> {
        Ok(self.rows.tasks_with_epic())
    }
}

#[async_trait::async_trait]
impl super::super::EpicCrud for Store {
    async fn create_epic(
        &self,
        title: &str,
        description: &str,
        parent_epic_id: Option<EpicId>,
    ) -> Result<Epic> {
        // `sync.allium: CreatesRequireASettledIdentity` — an epic has no
        // `owner` at all, so `created_by` is the only way `own_creations` can
        // find it before anyone follows it, and there is no name to stamp
        // without a settled identity.
        let identity = self
            .require_identity("there is no name to stamp on a new epic; it was not created")
            .await?;
        let now = self.now();
        let row = encode::create_epic_row(title, description, parent_epic_id, &identity, &now);
        let id = self.caller.create_epic(row.clone()).await?;
        // THE ROW AS SENT, WITH THE ID FILLED IN, rather than a read-back.
        //
        // Elsewhere this codebase insists the row is the truth and re-reads it
        // — `claim_next_backlog_task` says so explicitly. A create is the one
        // case where there is nothing to drift from: every field was chosen
        // here a moment ago, including both timestamps, and the store added
        // exactly one thing. Re-reading would also mean waiting for the
        // subscription to deliver a row this board may not even be subscribed
        // to.
        crate::sync::decode::epic(&bindings::Epic { id: id.0, ..row })
            .map_err(|e| anyhow::anyhow!("the created epic could not be read back: {e}"))
    }

    async fn create_repo_group_sub_epic(&self, parent_id: EpicId, title: &str) -> Result<EpicId> {
        // `sync.allium: CreatesRequireASettledIdentity`. Required even on the
        // FOUND arm, not only the created one: the module cannot tell this
        // caller which arm it took, so both need the same identity to stamp
        // — see `create_repo_group_sub_epic`'s doc comment in the module for
        // why `created_by` is what makes either answer readable back at all.
        let identity = self
            .require_identity("there is no name to stamp on a repo-group epic; it was not created")
            .await?;
        self.caller
            .create_repo_group_sub_epic(parent_id, title.to_string(), identity)
            .await
    }

    async fn create_managed_role_epic(
        &self,
        title: &str,
        parent_epic_id: Option<EpicId>,
        role: crate::models::FeedRole,
        feed_command: Option<&str>,
        feed_interval_secs: Option<i64>,
    ) -> Result<EpicId> {
        let identity = self
            .require_identity(
                "there is no name to stamp on a managed-role epic; it was not created",
            )
            .await?;
        self.caller
            .create_managed_role_epic(
                title.to_string(),
                parent_epic_id,
                role.as_str().to_string(),
                feed_command.unwrap_or_default().to_string(),
                feed_interval_secs.unwrap_or(0),
                identity,
            )
            .await
    }

    async fn patch_epic(&self, id: EpicId, patch: &EpicPatch<'_>) -> Result<()> {
        if !patch.has_changes() {
            return Ok(());
        }
        self.caller
            .patch_epic(id, encode::epic_patch(patch))
            .await?
            .applied()
    }

    async fn delete_epic(&self, id: EpicId) -> Result<()> {
        self.caller.delete_epic(id).await?.applied()
    }

    async fn set_task_epic_id(&self, task_id: TaskId, epic_id: Option<EpicId>) -> Result<()> {
        // A task leaving its epic lands on a user board and needs an owner;
        // one joining an epic gives its owner up. The store enforces both arms
        // (`core.allium: OwnerTracksUserBoardTask`), so the only job here is to
        // supply the name it may need.
        let owner = match epic_id {
            Some(_) => String::new(),
            None => {
                self.require_identity(
                    "a task cannot be moved out of its epic onto a user board; nothing was changed",
                )
                .await?
            }
        };
        self.caller
            .set_task_epic(task_id, epic_id, owner)
            .await?
            .applied()
    }

    /// On a store this is the ONLY place it can run: the derivation needs
    /// every child, and no board subscribes to all of them
    /// (`epics.allium: EpicStatusRecalculation`).
    async fn recalculate_epic_status(&self, epic_id: EpicId) -> Result<()> {
        self.caller
            .recalculate_epic_status(epic_id)
            .await?
            .applied()
    }
}
