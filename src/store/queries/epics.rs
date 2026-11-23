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

    async fn epic_orchestration(&self, id: EpicId) -> Option<crate::models::EpicOrchestration> {
        let row = self.rows.epic_orchestration(id)?;
        crate::sync::decode::epic_orchestration(&row)
            .map_err(|e| tracing::warn!("{e}"))
            .ok()
    }

    async fn list_epic_orchestrations(&self) -> Vec<crate::models::EpicOrchestration> {
        self.rows
            .epic_orchestrations()
            .iter()
            .filter_map(|row| {
                crate::sync::decode::epic_orchestration(row)
                    .map_err(|e| tracing::warn!("{e}"))
                    .ok()
            })
            .collect()
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
        // `owner` at all, so `created_by` is the only way the `own_epics` view
        // can find it before anyone follows it, and there is no name to stamp
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
            &identity,
            &now,
        );
        let id = self.caller.create_epic(row.clone()).await?;
        // The store rewrites `created_by` and `feed_command_set_by` on a
        // create (`identity.rs::settle_new_epic`). The board's own
        // view of the row is the truth when it has already arrived. This does
        // not wait for the subscription: a board may never be subscribed to
        // the row. Without it, the reply is the row as sent with the fields
        // the store derives filled in the way it derives them.
        if let Some(seen) = self.rows.epic(id) {
            return Ok(seen);
        }
        let feed_command_set_by = if row.feed_command.is_empty() {
            String::new()
        } else {
            row.created_by.clone()
        };
        crate::sync::decode::epic(&bindings::Epic {
            id: id.0,
            feed_command_set_by,
            ..row
        })
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

    async fn update_epic(&self, id: EpicId, patch: &EpicPatch<'_>) -> Result<()> {
        self.caller
            .update_epic(id, encode::epic_patch(patch))
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

    async fn flatten_epic(&self, root: EpicId) -> Result<()> {
        self.caller.flatten_epic(root).await?.applied()
    }

    async fn set_epic_group_by_repo(&self, id: EpicId, group_by_repo: bool) -> Result<()> {
        self.caller
            .set_epic_group_by_repo(id, group_by_repo)
            .await?
            .applied()
    }

    async fn switch_orchestrator_on(
        &self,
        id: EpicId,
        start_rule: Option<crate::models::StartRule>,
        max_parallel: Option<i64>,
    ) -> Result<()> {
        self.caller
            .switch_orchestrator_on(id, start_rule.map(|r| r.as_str().to_string()), max_parallel)
            .await?
            .applied()
    }

    async fn change_orchestrator_settings(
        &self,
        id: EpicId,
        start_rule: Option<crate::models::StartRule>,
        max_parallel: Option<i64>,
    ) -> Result<()> {
        self.caller
            .change_orchestrator_settings(
                id,
                start_rule.map(|r| r.as_str().to_string()),
                max_parallel,
            )
            .await?
            .applied()
    }

    async fn switch_orchestrator_off(&self, id: EpicId) -> Result<()> {
        self.caller.switch_orchestrator_off(id).await?.applied()
    }
}
