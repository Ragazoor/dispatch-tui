use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};

use crate::models::ReviewAgentStatus;

use super::super::Database;

#[async_trait::async_trait]
impl super::super::AlertStore for Database {
    async fn save_security_alerts(&self, alerts: &[crate::models::SecurityAlert]) -> Result<()> {
        let alerts_owned = alerts.to_vec();
        self.db_call(move |conn| save_security_alerts_impl(conn, &alerts_owned))
            .await
    }

    async fn load_security_alerts(&self) -> Result<Vec<crate::models::SecurityAlert>> {
        self.db_call(|conn| load_security_alerts_impl(conn)).await
    }

    async fn set_alert_agent(
        &self,
        repo: &str,
        number: i64,
        kind: crate::models::AlertKind,
        tmux_window: &str,
        worktree: &str,
    ) -> Result<bool> {
        let repo = repo.to_string();
        let tmux_window = tmux_window.to_string();
        let worktree = worktree.to_string();
        self.db_call(move |conn| {
            let rows = conn.execute(
                "UPDATE security_alerts SET tmux_window = ?1, worktree = ?2, agent_status = 'reviewing' WHERE repo = ?3 AND number = ?4 AND kind = ?5",
                params![tmux_window, worktree, repo, number, kind.as_db_str()],
            )?;
            Ok(rows > 0)
        })
        .await
    }

    async fn get_security_alert(
        &self,
        repo: &str,
        number: i64,
        kind: crate::models::AlertKind,
    ) -> Result<Option<crate::models::SecurityAlert>> {
        let repo = repo.to_string();
        self.db_call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT repo, number, kind, severity, title, package,
                        vulnerable_range, fixed_version, cvss_score, url,
                        created_at, state, description
                 FROM security_alerts
                 WHERE repo = ?1 AND number = ?2 AND kind = ?3",
            )?;
            let kind_str = kind.as_db_str();
            let mut rows = stmt.query(rusqlite::params![repo, number, kind_str])?;
            if let Some(row) = rows.next()? {
                return Ok(Some(parse_security_alert_row(row)?));
            }
            Ok(None)
        })
        .await
    }

    async fn alert_agent_status(
        &self,
        repo: &str,
        number: i64,
        kind: crate::models::AlertKind,
    ) -> Result<Option<ReviewAgentStatus>> {
        let repo = repo.to_string();
        self.db_call(move |conn| {
            let result: Option<Option<String>> = conn
                .query_row(
                    "SELECT agent_status FROM security_alerts WHERE repo = ?1 AND number = ?2 AND kind = ?3 AND tmux_window IS NOT NULL",
                    params![repo, number, kind.as_db_str()],
                    |row| row.get(0),
                )
                .optional()
                .context("Failed to query alert_agent_status")?;
            Ok(result
                .flatten()
                .as_deref()
                .and_then(ReviewAgentStatus::from_db_str))
        })
        .await
    }
}

// ---------------------------------------------------------------------------
// Security alert save/load helpers
// ---------------------------------------------------------------------------

fn save_security_alerts_impl(
    conn: &rusqlite::Connection,
    alerts: &[crate::models::SecurityAlert],
) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    {
        let mut stmt = tx.prepare(
            "INSERT INTO security_alerts (repo, number, kind, severity, title, package,
             vulnerable_range, fixed_version, cvss_score, url, created_at, state, description)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(repo, number, kind) DO UPDATE SET
             severity = excluded.severity, title = excluded.title,
             package = excluded.package, vulnerable_range = excluded.vulnerable_range,
             fixed_version = excluded.fixed_version, cvss_score = excluded.cvss_score,
             url = excluded.url, created_at = excluded.created_at,
             state = excluded.state, description = excluded.description",
        )?;
        for a in alerts {
            stmt.execute(params![
                a.repo,
                a.number,
                a.kind.as_db_str(),
                a.severity.as_db_str(),
                a.title,
                a.package,
                a.vulnerable_range,
                a.fixed_version,
                a.cvss_score,
                a.url,
                a.created_at.to_rfc3339(),
                a.state,
                a.description,
            ])?;
        }
    }

    // Delete stale rows
    if alerts.is_empty() {
        tx.execute("DELETE FROM security_alerts", [])?;
    } else {
        let placeholders: Vec<String> = (0..alerts.len())
            .map(|i| format!("(?{}, ?{}, ?{})", i * 3 + 1, i * 3 + 2, i * 3 + 3))
            .collect();
        let sql = format!(
            "DELETE FROM security_alerts WHERE (repo, number, kind) NOT IN (VALUES {})",
            placeholders.join(", ")
        );
        let params: Vec<Box<dyn rusqlite::types::ToSql>> = alerts
            .iter()
            .flat_map(|a| {
                vec![
                    Box::new(a.repo.clone()) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(a.number) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(a.kind.as_db_str()) as Box<dyn rusqlite::types::ToSql>,
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

fn parse_security_alert_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<crate::models::SecurityAlert> {
    use crate::models::{AlertKind, AlertSeverity, SecurityAlert};

    let repo: String = row.get(0)?;
    let number: i64 = row.get(1)?;
    let kind_str: String = row.get(2)?;
    let severity_str: String = row.get(3)?;
    let title: String = row.get(4)?;
    let package: Option<String> = row.get(5)?;
    let vulnerable_range: Option<String> = row.get(6)?;
    let fixed_version: Option<String> = row.get(7)?;
    let cvss_score: Option<f64> = row.get(8)?;
    let url: String = row.get(9)?;
    let created_at_str: String = row.get(10)?;
    let state: String = row.get(11)?;
    let description: String = row.get(12)?;

    let kind = AlertKind::from_db_str(&kind_str).unwrap_or(AlertKind::Dependabot);
    let severity = AlertSeverity::from_db_str(&severity_str).unwrap_or(AlertSeverity::Medium);
    let created_at = DateTime::parse_from_rfc3339(&created_at_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());

    Ok(SecurityAlert {
        number,
        repo,
        severity,
        kind,
        title,
        package,
        vulnerable_range,
        fixed_version,
        cvss_score,
        url,
        created_at,
        state,
        description,
    })
}

fn load_security_alerts_impl(
    conn: &rusqlite::Connection,
) -> Result<Vec<crate::models::SecurityAlert>> {
    let mut stmt = conn.prepare(
        "SELECT repo, number, kind, severity, title, package,
                vulnerable_range, fixed_version, cvss_score, url,
                created_at, state, description
         FROM security_alerts",
    )?;
    let rows = stmt.query_map([], parse_security_alert_row)?;
    let mut alerts = Vec::new();
    for row in rows {
        alerts.push(row?);
    }
    Ok(alerts)
}
