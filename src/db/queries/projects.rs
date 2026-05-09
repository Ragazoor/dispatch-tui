use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};

use crate::models::{Project, ProjectId};

use super::super::Database;

#[async_trait::async_trait]
impl super::super::ProjectCrud for Database {
    async fn create_project(&self, name: &str, sort_order: i64) -> Result<Project> {
        let name_owned = name.to_string();
        let name_for_row = name_owned.clone();
        self.db_call(move |conn| {
            conn.execute(
                "INSERT INTO projects (name, sort_order, is_default) VALUES (?1, ?2, 0)",
                params![name_owned, sort_order],
            )
            .context("Failed to create project")?;
            let id = ProjectId(conn.last_insert_rowid());
            Ok(Project {
                id,
                name: name_for_row,
                sort_order,
                is_default: false,
            })
        })
        .await
    }

    async fn list_projects(&self) -> Result<Vec<Project>> {
        self.db_call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, sort_order, is_default FROM projects \
                     ORDER BY sort_order ASC, id ASC",
                )
                .context("Failed to prepare list_projects")?;
            let projects = stmt
                .query_map([], |row| {
                    Ok(Project {
                        id: ProjectId(row.get::<_, i64>(0)?),
                        name: row.get(1)?,
                        sort_order: row.get(2)?,
                        is_default: row.get::<_, i64>(3)? != 0,
                    })
                })
                .context("Failed to query projects")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("Failed to collect projects")?;
            Ok(projects)
        })
        .await
    }

    async fn get_default_project(&self) -> Result<Project> {
        self.db_call(move |conn| {
            conn.query_row(
                "SELECT id, name, sort_order, is_default FROM projects WHERE is_default = 1",
                [],
                |row| {
                    Ok(Project {
                        id: ProjectId(row.get::<_, i64>(0)?),
                        name: row.get(1)?,
                        sort_order: row.get(2)?,
                        is_default: true,
                    })
                },
            )
            .context("Failed to get default project")
        })
        .await
    }

    async fn rename_project(&self, id: ProjectId, name: &str) -> Result<()> {
        let name_owned = name.to_string();
        self.db_call(move |conn| {
            let rows = conn
                .execute(
                    "UPDATE projects SET name = ?1 WHERE id = ?2",
                    params![name_owned, id.0],
                )
                .context("Failed to rename project")?;
            if rows == 0 {
                return Err(anyhow::anyhow!("Project {id} not found"));
            }
            Ok(())
        })
        .await
    }

    async fn delete_project_and_move_items(
        &self,
        id: ProjectId,
        default_id: ProjectId,
    ) -> Result<()> {
        self.db_call(move |conn| {
            // Guard: refuse to delete the default project
            let is_default: bool = conn
                .query_row(
                    "SELECT is_default FROM projects WHERE id = ?1",
                    params![id.0],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .context("Failed to check project")?
                .map(|v| v != 0)
                .unwrap_or(false);
            if is_default {
                return Err(anyhow::anyhow!("Cannot delete the default project"));
            }
            // Move items and delete the project atomically
            conn.execute_batch(&format!(
                "BEGIN;
                UPDATE tasks SET project_id = {default_id} WHERE project_id = {id};
                UPDATE epics SET project_id = {default_id} WHERE project_id = {id};
                DELETE FROM projects WHERE id = {id} AND is_default = 0;
                COMMIT;"
            ))
            .context("Failed to delete project and move items")?;
            Ok(())
        })
        .await
    }

    async fn reorder_project(&self, id: ProjectId, new_sort_order: i64) -> Result<()> {
        self.db_call(move |conn| {
            let rows = conn
                .execute(
                    "UPDATE projects SET sort_order = ?1 WHERE id = ?2",
                    params![new_sort_order, id.0],
                )
                .context("Failed to reorder project")?;
            if rows == 0 {
                return Err(anyhow::anyhow!("Project {id} not found"));
            }
            Ok(())
        })
        .await
    }
}
