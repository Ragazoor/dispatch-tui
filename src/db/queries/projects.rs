use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};

use crate::models::{Project, ProjectId};

use super::super::Database;

impl super::super::ProjectCrud for Database {
    fn create_project(&self, name: &str, sort_order: i64) -> Result<Project> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO projects (name, sort_order, is_default) VALUES (?1, ?2, 0)",
            params![name, sort_order],
        )
        .context("Failed to create project")?;
        let id = ProjectId(conn.last_insert_rowid());
        Ok(Project {
            id,
            name: name.to_string(),
            sort_order,
            is_default: false,
        })
    }

    fn list_projects(&self) -> Result<Vec<Project>> {
        let conn = self.conn()?;
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
    }

    fn get_default_project(&self) -> Result<Project> {
        let conn = self.conn()?;
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
    }

    fn rename_project(&self, id: ProjectId, name: &str) -> Result<()> {
        let conn = self.conn()?;
        let rows = conn
            .execute(
                "UPDATE projects SET name = ?1 WHERE id = ?2",
                params![name, id.0],
            )
            .context("Failed to rename project")?;
        if rows == 0 {
            return Err(anyhow::anyhow!("Project {id} not found"));
        }
        Ok(())
    }

    fn delete_project_and_move_items(&self, id: ProjectId, default_id: ProjectId) -> Result<()> {
        let conn = self.conn()?;
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
    }

    fn reorder_project(&self, id: ProjectId, new_sort_order: i64) -> Result<()> {
        let conn = self.conn()?;
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
    }
}
