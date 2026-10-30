use anyhow::Result;

use crate::models::{Epic, EpicId, Task, TaskId};
use crate::spacetime::bindings;
use crate::sync::encode;

use super::super::{EpicPatch, Store};

#[async_trait::async_trait]
impl super::super::EpicRead for Store {
    async fn get_epic(&self, id: EpicId) -> Option<Epic> {
        self.rows.epic(id)
    }

    async fn list_epics(&self) -> Vec<Epic> {
        self.rows.epics()
    }

    async fn list_sub_epics(&self, parent_id: EpicId) -> Vec<Epic> {
        self.rows.epics_with_parent(Some(parent_id))
    }

    async fn list_tasks_for_epic(&self, epic_id: EpicId) -> Vec<Task> {
        self.rows.tasks_for_epic(epic_id)
    }

    async fn list_undecodable_task_ids_for_epic(&self, epic_id: EpicId) -> Vec<TaskId> {
        self.rows.undecodable_task_ids_for_epic(epic_id)
    }

    async fn list_all_tasks_with_epic_id(&self) -> Vec<Task> {
        self.rows.tasks_with_epic()
    }
}

#[async_trait::async_trait]
impl super::super::EpicCrud for Store {
    async fn create_epic_configured(
        &self,
        title: &str,
        description: &str,
        parent_epic_id: Option<EpicId>,
        birth: &super::super::EpicBirth<'_>,
    ) -> Result<Epic> {
        // `sync.allium: CreatesRequireASettledIdentity` — an epic has no
        // `owner` at all, so `created_by` is the only way `own_creations` can
        // find it before anyone follows it, and there is no name to stamp
        // without a settled identity.
        let identity = self
            .require_identity("there is no name to stamp on a new epic; it was not created")
            .await?;
        let now = self.now();
        let row = encode::create_epic_row_with(
            title,
            description,
            parent_epic_id,
            birth.sort_order,
            birth.feed_command,
            birth.feed_interval_secs,
            (birth.workspace_id, birth.visibility),
            &identity,
            &now,
        );
        let id = self.caller.create_epic(row.clone()).await?;
        // THE ROW AS SENT, WITH THE ID FILLED IN, rather than a read-back.
        //
        // Elsewhere this codebase insists the row is the truth and re-reads it
        // — a reducer's effects are read off the subscription. A create is the one
        // case where there is nothing to drift from: every field was chosen
        // here a moment ago, including both timestamps, and the store added
        // exactly one thing. Re-reading would also mean waiting for the
        // subscription to deliver a row this board may not even be subscribed
        // to.
        crate::sync::decode::epic(&bindings::Epic { id: id.0, ..row })
            .map_err(|e| anyhow::anyhow!("the created epic could not be read back: {e}"))
    }

    async fn ensure_managed_epics(
        &self,
        reviews_command: Option<&str>,
        reviews_interval_secs: Option<i64>,
        cve_command: Option<&str>,
        cve_interval_secs: Option<i64>,
    ) -> Result<()> {
        self.caller
            .ensure_managed_epics(
                reviews_command.unwrap_or_default().to_string(),
                reviews_interval_secs.unwrap_or(0),
                cve_command.unwrap_or_default().to_string(),
                cve_interval_secs.unwrap_or(0),
            )
            .await?
            .applied()
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

    async fn set_epic_visibility(
        &self,
        id: EpicId,
        visibility: crate::models::Visibility,
    ) -> Result<()> {
        self.caller
            .set_epic_visibility(id, visibility.as_str().to_string())
            .await?
            .applied()
    }

    async fn move_epic_to_workspace(&self, id: EpicId, workspace_id: i64) -> Result<()> {
        self.caller
            .move_epic_to_workspace(id, workspace_id)
            .await?
            .applied()
    }

    async fn delete_epic(&self, id: EpicId) -> Result<()> {
        self.caller.delete_epic(id).await?.applied()
    }

    async fn set_task_epic_id(&self, task_id: TaskId, epic_id: Option<EpicId>) -> Result<()> {
        let owner = self.owner_for_epic_move(epic_id).await?;
        self.caller
            .set_task_epic(task_id, epic_id, owner)
            .await?
            .applied()
    }

    async fn regroup_epic(&self, root: EpicId) -> Result<()> {
        self.caller.regroup_epic(root).await?.applied()
    }

    async fn mark_epic_seen(&self, epic: EpicId) -> Result<()> {
        let Some(identity) = self.identity.user().await? else {
            return Ok(());
        };
        self.caller.mark_epic_seen(identity, epic).await?.applied()
    }

    async fn flatten_epic(&self, root: EpicId) -> Result<()> {
        self.caller.flatten_epic(root).await?.applied()
    }
}
