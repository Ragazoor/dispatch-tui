use std::collections::HashSet;

use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};

use crate::models::{
    EpicId, Learning, LearningId, LearningKind, LearningRetrieval, LearningScope, LearningStatus,
    LearningVerdict, ProjectId, RetrievalSource, TaskId,
};

use super::super::{CreateLearningRow, Database, LearningFilter, LearningPatch};
use super::{parse_datetime, read_json_string_vec, unknown_enum, write_json_string_vec};

const LEARNING_COLUMNS: &str =
    "id, kind, summary, detail, scope, scope_ref, tags, status, source_task_id, \
     upvote_count, last_upvoted_at, created_at, updated_at";

fn row_to_learning(row: &rusqlite::Row<'_>) -> rusqlite::Result<Learning> {
    let kind_str: String = row.get(1)?;
    let scope_str: String = row.get(4)?;
    let status_str: String = row.get(7)?;
    let last_upvoted_str: Option<String> = row.get(10)?;
    let created_str: String = row.get(11)?;
    let updated_str: String = row.get(12)?;

    let tags = read_json_string_vec(row, "tags")?;

    let kind = LearningKind::parse(&kind_str)
        .ok_or_else(|| unknown_enum("learning_kind", &kind_str))?;
    let scope = LearningScope::parse(&scope_str)
        .ok_or_else(|| unknown_enum("learning_scope", &scope_str))?;
    let status = LearningStatus::parse(&status_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, e.into())
    })?;

    Ok(Learning {
        id: LearningId(row.get::<_, i64>(0)?),
        kind,
        summary: row.get(2)?,
        detail: row.get(3)?,
        scope,
        scope_ref: row.get(5)?,
        tags,
        status,
        source_task_id: row.get::<_, Option<i64>>(8)?.map(crate::models::TaskId),
        upvote_count: row.get(9)?,
        last_upvoted_at: last_upvoted_str.as_deref().map(parse_datetime).transpose()?,
        created_at: parse_datetime(&created_str)?,
        updated_at: parse_datetime(&updated_str)?,
    })
}

#[async_trait::async_trait]
impl super::super::LearningStore for Database {
    async fn create_learning(&self, row: CreateLearningRow<'_>) -> Result<LearningId> {
        let kind = row.kind;
        let summary = row.summary.to_owned();
        let detail = row.detail.map(str::to_owned);
        let scope = row.scope;
        let scope_ref = row.scope_ref.map(str::to_owned);
        let tags = row.tags.to_vec();
        let source_task_id = row.source_task_id;
        let embedding = row.embedding.map(|b| b.to_vec());
        self.db_call(move |conn| {
            let tags_json = write_json_string_vec(&tags)?;
            conn.execute(
                "INSERT INTO learnings (kind, summary, detail, scope, scope_ref, tags, status, source_task_id, embedding)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'approved', ?7, ?8)",
                params![
                    kind.as_str(),
                    summary,
                    detail,
                    scope.as_str(),
                    scope_ref,
                    tags_json,
                    source_task_id.map(|t| t.0),
                    embedding,
                ],
            )
            .context("Failed to insert learning")?;
            Ok(LearningId(conn.last_insert_rowid()))
        })
        .await
    }

    async fn get_learning(&self, id: LearningId) -> Result<Option<Learning>> {
        self.db_call(move |conn| {
            conn.query_row(
                &format!("SELECT {LEARNING_COLUMNS} FROM learnings WHERE id = ?1"),
                params![id.0],
                row_to_learning,
            )
            .optional()
            .context("Failed to get learning")
        })
        .await
    }

    async fn list_learnings(&self, filter: LearningFilter) -> Result<Vec<Learning>> {
        self.db_call(move |conn| {
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

            if filter.tags.is_empty() {
                Ok(rows)
            } else {
                let tag_set: HashSet<String> = filter.tags.iter().cloned().collect();
                Ok(rows
                    .into_iter()
                    .filter(|l| l.tags.iter().any(|t| tag_set.contains(t)))
                    .collect())
            }
        })
        .await
    }

    async fn patch_learning(&self, id: LearningId, patch: &LearningPatch<'_>) -> Result<()> {
        if !patch.has_changes() {
            return Ok(());
        }
        let status = patch.status;
        let summary = patch.summary.map(|s| s.to_owned());
        let detail = patch.detail.map(|d| d.map(str::to_owned));
        let kind = patch.kind;
        let tags = patch.tags.map(|t| t.to_vec());
        let embedding = patch.embedding.map(|b| b.to_vec());

        self.db_call(move |conn| {
            let mut sets: Vec<String> = Vec::new();
            let mut bind: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

            if let Some(status) = status {
                sets.push(format!("status = ?{}", bind.len() + 1));
                bind.push(Box::new(status.as_str().to_owned()));
            }
            if let Some(summary) = summary {
                sets.push(format!("summary = ?{}", bind.len() + 1));
                bind.push(Box::new(summary));
            }
            if let Some(detail) = detail {
                sets.push(format!("detail = ?{}", bind.len() + 1));
                bind.push(Box::new(detail));
            }
            if let Some(kind) = kind {
                sets.push(format!("kind = ?{}", bind.len() + 1));
                bind.push(Box::new(kind.as_str().to_owned()));
            }
            if let Some(tags) = tags {
                sets.push(format!("tags = ?{}", bind.len() + 1));
                bind.push(Box::new(write_json_string_vec(&tags)?));
            }
            if let Some(embedding) = embedding {
                sets.push(format!("embedding = ?{}", bind.len() + 1));
                bind.push(Box::new(embedding));
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
        })
        .await
    }

    async fn delete_learning(&self, id: LearningId) -> Result<()> {
        self.db_call(move |conn| {
            conn.execute("DELETE FROM learnings WHERE id = ?1", params![id.0])
                .context("Failed to delete learning")?;
            Ok(())
        })
        .await
    }

    async fn upvote_learning(&self, id: LearningId) -> Result<()> {
        self.db_call(move |conn| {
            let changed = conn
                .execute(
                    "UPDATE learnings
                     SET upvote_count = upvote_count + 1,
                         last_upvoted_at = datetime('now'),
                         updated_at = datetime('now')
                     WHERE id = ?1 AND status = 'approved'",
                    params![id.0],
                )
                .context("Failed to upvote learning")?;
            if changed == 0 {
                anyhow::bail!("learning {id:?} not found or not approved");
            }
            Ok(())
        })
        .await
    }

    async fn list_learnings_for_dispatch(
        &self,
        project_id: Option<ProjectId>,
        repo_path: &str,
        epic_id: Option<EpicId>,
    ) -> Result<Vec<Learning>> {
        let repo_path = repo_path.to_owned();
        self.db_call(move |conn| {
            let project_ref = project_id.map(|id| id.to_string());
            let epic_ref = epic_id.map(|id| id.0.to_string());

            let mut scope_conditions: Vec<String> = vec!["scope = 'user'".to_string()];
            let mut bind: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

            bind.push(Box::new(repo_path));
            scope_conditions.push(format!("(scope = 'repo' AND scope_ref = ?{})", bind.len()));

            if let Some(pref) = project_ref {
                bind.push(Box::new(pref));
                scope_conditions.push(format!(
                    "(scope = 'project' AND scope_ref = ?{})",
                    bind.len()
                ));
            }
            if let Some(eref) = epic_ref {
                bind.push(Box::new(eref));
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
                   upvote_count DESC
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
        })
        .await
    }

    async fn list_all_approved_non_task_learnings(&self) -> Result<Vec<(Learning, Vec<u8>)>> {
        self.db_call(move |conn| {
            let sql = format!(
                "SELECT {LEARNING_COLUMNS}, embedding FROM learnings \
                 WHERE status = 'approved' AND scope != 'task' AND embedding IS NOT NULL \
                 ORDER BY id"
            );
            let mut stmt = conn
                .prepare(&sql)
                .context("Failed to prepare list_all_approved_non_task_learnings")?;
            let rows = stmt
                .query_map([], |row| {
                    let learning = row_to_learning(row)?;
                    // embedding is at index 13 (after the 13 LEARNING_COLUMNS); NOT NULL guaranteed by SQL
                    let embedding: Vec<u8> = row.get(13)?;
                    Ok((learning, embedding))
                })
                .context("Failed to query approved non-task learnings")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("Failed to collect approved non-task learnings")?;
            Ok(rows)
        })
        .await
    }

    async fn list_learnings_missing_embedding(&self) -> Result<Vec<Learning>> {
        self.db_call(move |conn| {
            let sql = format!(
                "SELECT {LEARNING_COLUMNS} FROM learnings \
                 WHERE embedding IS NULL AND status = 'approved' AND scope != 'task' \
                 ORDER BY id"
            );
            let mut stmt = conn
                .prepare(&sql)
                .context("Failed to prepare list_learnings_missing_embedding")?;
            let rows = stmt
                .query_map([], row_to_learning)
                .context("Failed to query learnings missing embedding")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("Failed to collect learnings missing embedding")?;
            Ok(rows)
        })
        .await
    }
}

#[async_trait::async_trait]
impl super::super::LearningRetrievalStore for Database {
    async fn record_retrieval(
        &self,
        task_id: TaskId,
        learning_id: LearningId,
        source: RetrievalSource,
    ) -> Result<()> {
        self.db_call(move |conn| {
            conn.execute(
                "INSERT INTO learning_retrievals (task_id, learning_id, source)
                 VALUES (?1, ?2, ?3)",
                params![task_id.0, learning_id.0, source.as_str()],
            )
            .context("Failed to insert learning retrieval")?;
            Ok(())
        })
        .await
    }

    async fn list_retrievals_for_task(&self, task_id: TaskId) -> Result<Vec<LearningRetrieval>> {
        self.db_call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, task_id, learning_id, source, retrieved_at
                     FROM learning_retrievals
                     WHERE task_id = ?1
                     ORDER BY id",
                )
                .context("Failed to prepare list_retrievals_for_task")?;
            let rows = stmt
                .query_map(params![task_id.0], |row| {
                    let source_str: String = row.get(3)?;
                    let source = RetrievalSource::parse(&source_str).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            3,
                            rusqlite::types::Type::Text,
                            e.into(),
                        )
                    })?;
                    let retrieved_str: String = row.get(4)?;
                    Ok(LearningRetrieval {
                        id: row.get(0)?,
                        task_id: TaskId(row.get(1)?),
                        learning_id: LearningId(row.get(2)?),
                        source,
                        retrieved_at: parse_datetime(&retrieved_str)?,
                    })
                })
                .context("Failed to query learning retrievals")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("Failed to collect learning retrievals")?;
            Ok(rows)
        })
        .await
    }

    async fn apply_verdicts_tx(
        &self,
        task_id: TaskId,
        verdicts: &[(LearningId, LearningVerdict)],
    ) -> Result<()> {
        let verdicts = verdicts.to_vec();
        self.db_call(move |conn| {
            let tx = conn
                .unchecked_transaction()
                .context("Failed to begin verdict transaction")?;
            for (lid, verdict) in &verdicts {
                tx.execute(
                    "INSERT INTO learning_verdicts (task_id, learning_id, verdict)
                     VALUES (?1, ?2, ?3)",
                    params![task_id.0, lid.0, verdict.as_str()],
                )
                .context("Failed to insert learning verdict")?;
                match verdict {
                    LearningVerdict::Helped => {
                        tx.execute(
                            "UPDATE learnings
                             SET upvote_count = upvote_count + 1,
                                 last_upvoted_at = datetime('now'),
                                 updated_at = datetime('now')
                             WHERE id = ?1",
                            params![lid.0],
                        )
                        .context("Failed to bump upvote_count for helped verdict")?;
                    }
                    LearningVerdict::Wrong => {
                        tx.execute(
                            "UPDATE learnings
                             SET status = 'needs_review',
                                 updated_at = datetime('now')
                             WHERE id = ?1 AND status = 'approved'",
                            params![lid.0],
                        )
                        .context("Failed to flag learning as needs_review")?;
                    }
                    LearningVerdict::Unused => {}
                }
            }
            tx.commit()
                .context("Failed to commit verdict transaction")?;
            Ok(())
        })
        .await
    }

    async fn count_learnings_needs_review(&self) -> Result<i64> {
        self.db_call(move |conn| {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM learnings WHERE status = 'needs_review'",
                    [],
                    |row| row.get(0),
                )
                .context("Failed to count needs_review learnings")?;
            Ok(n)
        })
        .await
    }
}
