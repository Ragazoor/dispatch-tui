use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};

use crate::models::{CiStatus, ReviewAgentStatus, ReviewDecision, ReviewPr, Reviewer};

use super::super::Database;

#[async_trait::async_trait]
impl super::super::PrStore for Database {
    async fn save_prs(&self, kind: super::super::PrKind, prs: &[ReviewPr]) -> Result<()> {
        let prs_owned = prs.to_vec();
        self.db_call(move |conn| save_prs_to_table(conn, kind.table_name(), &prs_owned))
            .await
    }

    async fn load_prs(&self, kind: super::super::PrKind) -> Result<Vec<ReviewPr>> {
        self.db_call(move |conn| load_prs_from_table(conn, kind.table_name()))
            .await
    }

    async fn set_pr_agent(
        &self,
        kind: super::super::PrKind,
        repo: &str,
        number: i64,
        tmux_window: &str,
        worktree: &str,
    ) -> Result<bool> {
        let table = kind.table_name();
        let repo = repo.to_string();
        let tmux_window = tmux_window.to_string();
        let worktree = worktree.to_string();
        self.db_call(move |conn| {
            let rows = conn.execute(
                &format!("UPDATE {table} SET tmux_window = ?1, worktree = ?2, agent_status = 'reviewing' WHERE repo = ?3 AND number = ?4"),
                params![tmux_window, worktree, repo, number],
            )?;
            Ok(rows > 0)
        })
        .await
    }

    async fn get_review_pr(&self, repo: &str, number: i64) -> Result<Option<ReviewPr>> {
        let repo = repo.to_string();
        self.db_call(move |conn| {
            for table in &["review_prs", "my_prs"] {
                let result = load_pr_by_key(conn, table, &repo, number)?;
                if result.is_some() {
                    return Ok(result);
                }
            }
            Ok(None)
        })
        .await
    }

    async fn update_agent_status(
        &self,
        repo: &str,
        number: i64,
        status: Option<&str>,
    ) -> Result<String> {
        let repo = repo.to_string();
        let status = status.map(|s| s.to_string());
        self.db_call(move |conn| {
            for table in &["review_prs", "my_prs", "bot_prs"] {
                let affected = conn.execute(
                    &format!("UPDATE {table} SET agent_status = ?1 WHERE repo = ?2 AND number = ?3 AND tmux_window IS NOT NULL"),
                    params![status, repo, number],
                )?;
                if affected > 0 {
                    return Ok(table.to_string());
                }
            }
            let affected = conn.execute(
                "UPDATE security_alerts SET agent_status = ?1 WHERE repo = ?2 AND number = ?3 AND tmux_window IS NOT NULL",
                params![status, repo, number],
            )?;
            if affected > 0 {
                return Ok("security_alerts".to_string());
            }
            anyhow::bail!("No active agent found for {repo}#{number}");
        })
        .await
    }

    async fn pr_agent_status(
        &self,
        table: &str,
        repo: &str,
        number: i64,
    ) -> Result<Option<ReviewAgentStatus>> {
        assert!(
            matches!(table, "review_prs" | "my_prs" | "bot_prs"),
            "invalid PR table: {table}"
        );
        let table = table.to_string();
        let repo = repo.to_string();
        self.db_call(move |conn| {
            let result: Option<Option<String>> = conn
                .query_row(
                    &format!(
                        "SELECT agent_status FROM {table} WHERE repo = ?1 AND number = ?2 AND tmux_window IS NOT NULL"
                    ),
                    params![repo, number],
                    |row| row.get(0),
                )
                .optional()
                .context("Failed to query pr_agent_status")?;
            Ok(result
                .flatten()
                .as_deref()
                .and_then(ReviewAgentStatus::from_db_str))
        })
        .await
    }
}

// ---------------------------------------------------------------------------
// PrWorkflowStore
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl super::super::PrWorkflowStore for Database {
    async fn insert_pr_workflow_if_absent(
        &self,
        repo: &str,
        number: i64,
        kind: crate::models::WorkflowItemKind,
    ) -> anyhow::Result<()> {
        let repo = repo.to_string();
        self.db_call(move |conn| {
            conn.execute(
                "INSERT OR IGNORE INTO pr_workflow_states (repo, number, kind, state, updated_at)
                 VALUES (?1, ?2, ?3, 'backlog', ?4)",
                params![
                    repo,
                    number,
                    kind.as_db_str(),
                    chrono::Utc::now().to_rfc3339(),
                ],
            )?;
            Ok(())
        })
        .await
    }

    async fn upsert_pr_workflow(
        &self,
        repo: &str,
        number: i64,
        kind: crate::models::WorkflowItemKind,
        state: &str,
        sub_state: Option<&str>,
    ) -> anyhow::Result<()> {
        let repo = repo.to_string();
        let state = state.to_string();
        let sub_state = sub_state.map(|s| s.to_string());
        self.db_call(move |conn| {
            conn.execute(
                "INSERT INTO pr_workflow_states (repo, number, kind, state, sub_state, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(repo, number, kind) DO UPDATE SET
                     state = excluded.state,
                     sub_state = excluded.sub_state,
                     updated_at = excluded.updated_at",
                params![
                    repo,
                    number,
                    kind.as_db_str(),
                    state,
                    sub_state,
                    chrono::Utc::now().to_rfc3339(),
                ],
            )?;
            Ok(())
        })
        .await
    }

    async fn get_pr_workflow(
        &self,
        repo: &str,
        number: i64,
        kind: crate::models::WorkflowItemKind,
    ) -> anyhow::Result<Option<super::super::PrWorkflowRow>> {
        let repo = repo.to_string();
        self.db_call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT repo, number, kind, state, sub_state, updated_at
                 FROM pr_workflow_states
                 WHERE repo = ?1 AND number = ?2 AND kind = ?3",
            )?;
            let row = stmt
                .query_row(params![repo, number, kind.as_db_str()], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                })
                .optional()?;

            let Some((repo, number, kind_str, state, sub_state, updated_str)) = row else {
                return Ok(None);
            };
            let kind = crate::models::WorkflowItemKind::from_db_str(&kind_str)
                .ok_or_else(|| anyhow::anyhow!("unknown workflow kind: {kind_str}"))?;
            let updated_at = updated_str
                .parse::<chrono::DateTime<chrono::Utc>>()
                .map_err(|e| anyhow::anyhow!("bad updated_at timestamp: {e}"))?;
            Ok(Some(super::super::PrWorkflowRow {
                repo,
                number,
                kind,
                state,
                sub_state,
                updated_at,
            }))
        })
        .await
    }

    async fn list_pr_workflows(&self) -> anyhow::Result<Vec<super::super::PrWorkflowRow>> {
        self.db_call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT repo, number, kind, state, sub_state, updated_at
                 FROM pr_workflow_states",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                })?
                .map(|r| {
                    let (repo, number, kind_str, state, sub_state, updated_str) = r?;
                    let kind = crate::models::WorkflowItemKind::from_db_str(&kind_str)
                        .ok_or_else(|| rusqlite::Error::InvalidQuery)?;
                    let updated_at = updated_str
                        .parse::<chrono::DateTime<chrono::Utc>>()
                        .map_err(|_| rusqlite::Error::InvalidQuery)?;
                    Ok(super::super::PrWorkflowRow {
                        repo,
                        number,
                        kind,
                        state,
                        sub_state,
                        updated_at,
                    })
                })
                .collect::<Result<Vec<_>, rusqlite::Error>>()?;
            Ok(rows)
        })
        .await
    }

    async fn find_pr_workflow_kind(
        &self,
        repo: &str,
        number: i64,
    ) -> anyhow::Result<Option<crate::models::WorkflowItemKind>> {
        let repo = repo.to_string();
        self.db_call(move |conn| {
            let kind_str: Option<String> = conn
                .query_row(
                    "SELECT kind FROM pr_workflow_states WHERE repo = ?1 AND number = ?2 LIMIT 1",
                    params![repo, number],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(kind_str
                .as_deref()
                .and_then(crate::models::WorkflowItemKind::from_db_str))
        })
        .await
    }

    async fn prune_done_pr_workflows(&self, older_than: chrono::Duration) -> anyhow::Result<()> {
        self.db_call(move |conn| {
            let cutoff = chrono::Utc::now() - older_than;
            conn.execute(
                "DELETE FROM pr_workflow_states
                 WHERE state = 'done' AND updated_at < ?1",
                params![cutoff.to_rfc3339()],
            )?;
            Ok(())
        })
        .await
    }
}

// ---------------------------------------------------------------------------
// Shared PR save/load helpers
// ---------------------------------------------------------------------------

fn save_prs_to_table(conn: &rusqlite::Connection, table: &str, prs: &[ReviewPr]) -> Result<()> {
    assert!(
        matches!(table, "review_prs" | "my_prs" | "bot_prs"),
        "invalid PR table: {table}"
    );
    let tx = conn.unchecked_transaction()?;

    // Upsert all PRs — ON CONFLICT preserves tmux_window and worktree
    {
        let mut stmt = tx.prepare(&format!(
            "INSERT INTO {table} (repo, number, title, author, url, is_draft,
             created_at, updated_at, additions, deletions, review_decision, labels,
             body, head_ref, ci_status, reviewers)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
             ON CONFLICT(repo, number) DO UPDATE SET
             title = excluded.title, author = excluded.author, url = excluded.url,
             is_draft = excluded.is_draft, created_at = excluded.created_at,
             updated_at = excluded.updated_at, additions = excluded.additions,
             deletions = excluded.deletions, review_decision = excluded.review_decision,
             labels = excluded.labels, body = excluded.body, head_ref = excluded.head_ref,
             ci_status = excluded.ci_status, reviewers = excluded.reviewers"
        ))?;
        for pr in prs {
            let labels_json =
                serde_json::to_string(&pr.labels).context("Failed to serialize labels")?;
            let reviewers_json = serde_json::to_string(
                &pr.reviewers
                    .iter()
                    .map(|r| {
                        serde_json::json!({
                            "login": r.login,
                            "decision": r.decision.map(|d| d.as_db_str())
                        })
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap_or_default();
            stmt.execute(params![
                pr.repo,
                pr.number,
                pr.title,
                pr.author,
                pr.url,
                pr.is_draft,
                pr.created_at.to_rfc3339(),
                pr.updated_at.to_rfc3339(),
                pr.additions,
                pr.deletions,
                pr.review_decision.as_db_str(),
                labels_json,
                pr.body,
                pr.head_ref,
                pr.ci_status.as_db_str(),
                reviewers_json,
            ])?;
        }
    }

    // Delete stale rows not in the fresh set
    if prs.is_empty() {
        tx.execute(&format!("DELETE FROM {table}"), [])?;
    } else {
        let placeholders: Vec<String> = (0..prs.len())
            .map(|i| format!("(?{}, ?{})", i * 2 + 1, i * 2 + 2))
            .collect();
        let sql = format!(
            "DELETE FROM {table} WHERE (repo, number) NOT IN (VALUES {})",
            placeholders.join(", ")
        );
        let params: Vec<Box<dyn rusqlite::types::ToSql>> = prs
            .iter()
            .flat_map(|pr| {
                vec![
                    Box::new(pr.repo.clone()) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(pr.number) as Box<dyn rusqlite::types::ToSql>,
                ]
            })
            .collect();
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        tx.execute(&sql, param_refs.as_slice())?;
    }

    tx.commit()?;
    Ok(())
}

fn parse_review_pr_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReviewPr> {
    let repo: String = row.get(0)?;
    let number: i64 = row.get(1)?;
    let title: String = row.get(2)?;
    let author: String = row.get(3)?;
    let url: String = row.get(4)?;
    let is_draft: bool = row.get(5)?;
    let created_at_str: String = row.get(6)?;
    let updated_at_str: String = row.get(7)?;
    let additions: i64 = row.get(8)?;
    let deletions: i64 = row.get(9)?;
    let decision_str: String = row.get(10)?;
    let labels_json: String = row.get(11)?;
    let body: String = row.get(12)?;
    let head_ref: String = row.get(13)?;
    let ci_status_str: String = row.get(14)?;
    let reviewers_json: String = row.get(15)?;
    let created_at = DateTime::parse_from_rfc3339(&created_at_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let review_decision =
        ReviewDecision::from_db_str(&decision_str).unwrap_or(ReviewDecision::ReviewRequired);
    let labels: Vec<String> = serde_json::from_str(&labels_json).unwrap_or_default();
    let ci_status = CiStatus::from_db_str(&ci_status_str);
    let reviewers: Vec<Reviewer> = serde_json::from_str::<Vec<serde_json::Value>>(&reviewers_json)
        .unwrap_or_default()
        .iter()
        .map(|v| Reviewer {
            login: v["login"].as_str().unwrap_or("").to_string(),
            decision: v["decision"].as_str().and_then(ReviewDecision::from_db_str),
        })
        .collect();

    Ok(ReviewPr {
        number,
        title,
        author,
        repo,
        url,
        is_draft,
        created_at,
        updated_at,
        additions,
        deletions,
        review_decision,
        labels,
        body,
        head_ref,
        ci_status,
        reviewers,
    })
}

fn load_prs_from_table(conn: &rusqlite::Connection, table: &str) -> Result<Vec<ReviewPr>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT repo, number, title, author, url, is_draft,
                created_at, updated_at, additions, deletions,
                review_decision, labels, body, head_ref, ci_status, reviewers
         FROM {table}"
    ))?;
    let rows = stmt.query_map([], parse_review_pr_row)?;
    let mut prs = Vec::new();
    for row in rows {
        prs.push(row?);
    }
    Ok(prs)
}

fn load_pr_by_key(
    conn: &rusqlite::Connection,
    table: &str,
    repo: &str,
    number: i64,
) -> Result<Option<ReviewPr>> {
    assert!(
        matches!(table, "review_prs" | "my_prs" | "bot_prs"),
        "invalid PR table: {table}"
    );
    let mut stmt = conn.prepare(&format!(
        "SELECT repo, number, title, author, url, is_draft,
                created_at, updated_at, additions, deletions,
                review_decision, labels, body, head_ref, ci_status, reviewers
         FROM {table}
         WHERE repo = ?1 AND number = ?2"
    ))?;
    let mut rows = stmt.query(rusqlite::params![repo, number])?;
    if let Some(row) = rows.next()? {
        return Ok(Some(parse_review_pr_row(row)?));
    }
    Ok(None)
}
