use std::collections::HashSet;

use anyhow::{Context, Result};
use chrono::{NaiveDateTime, TimeZone, Utc};
use rusqlite::{params, OptionalExtension};

use crate::models::{
    EpicId, Learning, LearningId, LearningKind, LearningScope, LearningStatus, ProjectId, TaskId,
};

use super::super::{Database, LearningFilter, LearningPatch};

const LEARNING_COLUMNS: &str =
    "id, kind, summary, detail, scope, scope_ref, tags, status, source_task_id, \
     confirmed_count, last_confirmed_at, created_at, updated_at";

fn row_to_learning(row: &rusqlite::Row<'_>) -> rusqlite::Result<Learning> {
    let kind_str: String = row.get(1)?;
    let scope_str: String = row.get(4)?;
    let status_str: String = row.get(7)?;
    let tags_json: String = row.get(6)?;
    let last_confirmed_str: Option<String> = row.get(10)?;
    let created_str: String = row.get(11)?;
    let updated_str: String = row.get(12)?;

    let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();

    let parse_dt = |s: &str| -> chrono::DateTime<Utc> {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
            .map(|ndt| Utc.from_utc_datetime(&ndt))
            .unwrap_or_else(|_| Utc::now())
    };

    Ok(Learning {
        id: LearningId(row.get::<_, i64>(0)?),
        kind: LearningKind::parse(&kind_str).unwrap_or(LearningKind::Convention),
        summary: row.get(2)?,
        detail: row.get(3)?,
        scope: LearningScope::parse(&scope_str).unwrap_or(LearningScope::User),
        scope_ref: row.get(5)?,
        tags,
        status: LearningStatus::parse(&status_str).unwrap_or(LearningStatus::Approved),
        source_task_id: row.get::<_, Option<i64>>(8)?.map(crate::models::TaskId),
        confirmed_count: row.get(9)?,
        last_confirmed_at: last_confirmed_str.as_deref().map(parse_dt),
        created_at: parse_dt(&created_str),
        updated_at: parse_dt(&updated_str),
    })
}

impl super::super::LearningStore for Database {
    #[allow(clippy::too_many_arguments)]
    fn create_learning(
        &self,
        kind: LearningKind,
        summary: &str,
        detail: Option<&str>,
        scope: LearningScope,
        scope_ref: Option<&str>,
        tags: &[String],
        source_task_id: Option<TaskId>,
    ) -> Result<LearningId> {
        let conn = self.conn()?;
        let tags_json = serde_json::to_string(tags).context("Failed to serialize tags")?;
        conn.execute(
            "INSERT INTO learnings (kind, summary, detail, scope, scope_ref, tags, status, source_task_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'approved', ?7)",
            params![
                kind.as_str(),
                summary,
                detail,
                scope.as_str(),
                scope_ref,
                tags_json,
                source_task_id.map(|t| t.0),
            ],
        )
        .context("Failed to insert learning")?;
        Ok(LearningId(conn.last_insert_rowid()))
    }

    fn get_learning(&self, id: LearningId) -> Result<Option<Learning>> {
        let conn = self.conn()?;
        conn.query_row(
            &format!("SELECT {LEARNING_COLUMNS} FROM learnings WHERE id = ?1"),
            params![id.0],
            row_to_learning,
        )
        .optional()
        .context("Failed to get learning")
    }

    fn list_learnings(&self, filter: LearningFilter) -> Result<Vec<Learning>> {
        let conn = self.conn()?;
        let mut conditions: Vec<String> = Vec::new();
        let mut bind: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(status) = filter.status {
            conditions.push(format!("status = ?{}", bind.len() + 1));
            bind.push(Box::new(status.as_str().to_owned()));
        }
        if let Some(scope) = filter.scope {
            conditions.push(format!("scope = ?{}", bind.len() + 1));
            bind.push(Box::new(scope.as_str().to_owned()));
        }
        if let Some(scope_ref) = filter.scope_ref {
            conditions.push(format!("scope_ref = ?{}", bind.len() + 1));
            bind.push(Box::new(scope_ref));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let limit_clause = filter
            .limit
            .map(|l| format!("LIMIT {l}"))
            .unwrap_or_default();

        let sql = format!(
            "SELECT {LEARNING_COLUMNS} FROM learnings {where_clause} ORDER BY created_at DESC {limit_clause}"
        );

        let params_refs: Vec<&dyn rusqlite::ToSql> = bind.iter().map(|b| b.as_ref()).collect();

        let mut stmt = conn
            .prepare(&sql)
            .context("Failed to prepare list_learnings")?;
        let rows = stmt
            .query_map(params_refs.as_slice(), row_to_learning)
            .context("Failed to list learnings")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect learnings")?;

        // In-memory tag filter (tags is a JSON field; OR match)
        if filter.tags.is_empty() {
            Ok(rows)
        } else {
            let tag_set: HashSet<&str> = filter.tags.iter().map(String::as_str).collect();
            Ok(rows
                .into_iter()
                .filter(|l| l.tags.iter().any(|t| tag_set.contains(t.as_str())))
                .collect())
        }
    }

    fn patch_learning(&self, id: LearningId, patch: &LearningPatch<'_>) -> Result<()> {
        if !patch.has_changes() {
            return Ok(());
        }
        let conn = self.conn()?;
        let mut sets: Vec<String> = Vec::new();
        let mut bind: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(status) = patch.status {
            sets.push(format!("status = ?{}", bind.len() + 1));
            bind.push(Box::new(status.as_str().to_owned()));
        }
        if let Some(summary) = patch.summary {
            sets.push(format!("summary = ?{}", bind.len() + 1));
            bind.push(Box::new(summary.to_owned()));
        }
        if let Some(detail) = patch.detail {
            sets.push(format!("detail = ?{}", bind.len() + 1));
            bind.push(Box::new(detail.map(str::to_owned)));
        }
        if let Some(kind) = patch.kind {
            sets.push(format!("kind = ?{}", bind.len() + 1));
            bind.push(Box::new(kind.as_str().to_owned()));
        }
        if let Some(tags) = patch.tags {
            let json = serde_json::to_string(tags).context("Failed to serialize tags")?;
            sets.push(format!("tags = ?{}", bind.len() + 1));
            bind.push(Box::new(json));
        }

        sets.push("updated_at = datetime('now')".to_string());
        bind.push(Box::new(id.0));

        let sql = format!(
            "UPDATE learnings SET {} WHERE id = ?{}",
            sets.join(", "),
            bind.len()
        );

        let params_refs: Vec<&dyn rusqlite::ToSql> = bind.iter().map(|b| b.as_ref()).collect();

        conn.execute(&sql, params_refs.as_slice())
            .context("Failed to patch learning")?;
        Ok(())
    }

    fn delete_learning(&self, id: LearningId) -> Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM learnings WHERE id = ?1", params![id.0])
            .context("Failed to delete learning")?;
        Ok(())
    }

    fn upvote_learning(&self, id: LearningId) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE learnings
             SET confirmed_count = confirmed_count + 1,
                 last_confirmed_at = datetime('now'),
                 updated_at = datetime('now')
             WHERE id = ?1",
            params![id.0],
        )
        .context("Failed to confirm learning")?;
        Ok(())
    }

    fn list_learnings_for_dispatch(
        &self,
        project_id: Option<ProjectId>,
        repo_path: &str,
        epic_id: Option<EpicId>,
    ) -> Result<Vec<Learning>> {
        let conn = self.conn()?;

        // Build the scope conditions for the dispatch union.
        // Scope priority order (used in ORDER BY CASE):
        //   procedural=0, epic=1, repo=2, project=3, user=4
        let project_ref = project_id.map(|id| id.to_string());
        let epic_ref = epic_id.map(|id| id.0.to_string());

        let mut scope_conditions: Vec<String> = vec!["scope = 'user'".to_string()];
        let mut bind: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        bind.push(Box::new(repo_path.to_owned()));
        scope_conditions.push(format!("(scope = 'repo' AND scope_ref = ?{})", bind.len()));

        if let Some(ref pref) = project_ref {
            bind.push(Box::new(pref.clone()));
            scope_conditions.push(format!(
                "(scope = 'project' AND scope_ref = ?{})",
                bind.len()
            ));
        }
        if let Some(ref eref) = epic_ref {
            bind.push(Box::new(eref.clone()));
            scope_conditions.push(format!("(scope = 'epic' AND scope_ref = ?{})", bind.len()));
        }

        let scope_filter = scope_conditions.join(" OR ");

        let sql = format!(
            "SELECT {LEARNING_COLUMNS} FROM learnings
             WHERE status = 'approved'
               AND ({scope_filter})
             ORDER BY
               CASE kind WHEN 'procedural' THEN 0 ELSE 1 END,
               CASE scope
                 WHEN 'epic'    THEN 1
                 WHEN 'repo'    THEN 2
                 WHEN 'project' THEN 3
                 WHEN 'user'    THEN 4
                 ELSE 5
               END,
               confirmed_count DESC
             LIMIT 10"
        );

        let params_refs: Vec<&dyn rusqlite::ToSql> = bind.iter().map(|b| b.as_ref()).collect();

        let mut stmt = conn
            .prepare(&sql)
            .context("Failed to prepare list_learnings_for_dispatch")?;
        let rows = stmt
            .query_map(params_refs.as_slice(), row_to_learning)
            .context("Failed to query learnings for dispatch")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect learnings for dispatch")?;
        Ok(rows)
    }
}
