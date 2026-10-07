use anyhow::Result;

use crate::models::{EpicId, TaskId};

use super::super::{Database, EpicPatch};

#[async_trait::async_trait]
impl super::super::EpicRead for Database {
    async fn get_epic(&self, id: EpicId) -> Result<Option<crate::models::Epic>> {
        let reader = self.shared_reader()?;
        reader.get_epic(id).await
    }

    async fn list_epics(&self) -> Result<Vec<crate::models::Epic>> {
        let reader = self.shared_reader()?;
        reader.list_epics().await
    }

    async fn list_root_epics(&self) -> Result<Vec<crate::models::Epic>> {
        let reader = self.shared_reader()?;
        reader.list_epics_with_parent(None).await
    }

    async fn list_sub_epics(&self, parent_id: EpicId) -> Result<Vec<crate::models::Epic>> {
        let reader = self.shared_reader()?;
        reader.list_epics_with_parent(Some(parent_id)).await
    }

    async fn list_tasks_for_epic(&self, epic_id: EpicId) -> Result<Vec<crate::models::Task>> {
        let reader = self.shared_reader()?;
        reader.list_tasks_for_epic(epic_id).await
    }

    async fn list_undecodable_task_ids_for_epic(
        &self,
        epic_id: EpicId,
    ) -> Result<Vec<crate::models::TaskId>> {
        let reader = self.shared_reader()?;
        reader.list_undecodable_task_ids_for_epic(epic_id).await
    }

    async fn list_all_tasks_with_epic_id(&self) -> Result<Vec<crate::models::Task>> {
        let reader = self.shared_reader()?;
        reader.list_all_tasks_with_epic_id().await
    }
}

#[async_trait::async_trait]
impl super::super::EpicCrud for Database {
    async fn create_epic(
        &self,
        title: &str,
        description: &str,
        parent_epic_id: Option<EpicId>,
    ) -> Result<crate::models::Epic> {
        // ROUTED. `sync.allium: BoardWritesThroughTheStore`.
        let writer = self.shared_writer()?;
        writer.create_epic(title, description, parent_epic_id).await
    }

    async fn create_repo_group_sub_epic(&self, parent_id: EpicId, title: &str) -> Result<EpicId> {
        let writer = self.shared_writer()?;
        writer.create_repo_group_sub_epic(parent_id, title).await
    }

    async fn create_managed_role_epic(
        &self,
        title: &str,
        parent_epic_id: Option<EpicId>,
        role: crate::models::FeedRole,
        feed_command: Option<&str>,
        feed_interval_secs: Option<i64>,
    ) -> Result<EpicId> {
        let writer = self.shared_writer()?;
        writer
            .create_managed_role_epic(
                title,
                parent_epic_id,
                role,
                feed_command,
                feed_interval_secs,
            )
            .await
    }

    async fn patch_epic(&self, id: EpicId, patch: &EpicPatch<'_>) -> Result<()> {
        if !patch.has_changes() {
            return Ok(());
        }
        // ROUTED. `sync.allium: BoardWritesThroughTheStore`.
        let writer = self.shared_writer()?;
        writer.patch_epic(id, patch).await
    }

    async fn delete_epic(&self, id: EpicId) -> Result<()> {
        // ROUTED. `sync.allium: BoardWritesThroughTheStore`.
        let writer = self.shared_writer()?;
        writer.delete_epic(id).await
    }

    async fn set_task_epic_id(&self, task_id: TaskId, epic_id: Option<EpicId>) -> Result<()> {
        // ROUTED. `sync.allium: BoardWritesThroughTheStore`.
        let writer = self.shared_writer()?;
        writer.set_task_epic_id(task_id, epic_id).await
    }

    async fn recalculate_epic_status(&self, epic_id: EpicId) -> Result<()> {
        // ROUTED, and on a store this is the ONLY place it can run: the
        // derivation needs every child, and no board subscribes to all of them
        // (`epics.allium: EpicStatusRecalculation`).
        let writer = self.shared_writer()?;
        writer.recalculate_epic_status(epic_id).await
    }
}
