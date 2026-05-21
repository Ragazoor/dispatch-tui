use anyhow::{Context, Result};
use rusqlite::params;

use crate::models::{UsageEvent, UsageSummary};

use super::super::{Database, UsageCap, UsageQuery};
use super::parse_datetime;

#[async_trait::async_trait]
impl crate::db::UsageStore for Database {
    async fn record_usage_event_with_cap(&self, event: &UsageEvent, cap: UsageCap) -> Result<()> {
        let category = event.category.as_str().to_string();
        let action = event.action.clone();
        let detail = event.detail.clone();
        let actor = event.actor.as_str().to_string();
        self.db_call(move |conn| {
            conn.execute(
                "INSERT INTO usage_events (category, action, detail, actor)
                 VALUES (?1, ?2, ?3, ?4)",
                params![category, action, detail, actor],
            )
            .context("Failed to insert usage_event")?;

            conn.execute(
                "DELETE FROM usage_events
                 WHERE id <= (
                     SELECT id FROM usage_events
                     ORDER BY id DESC
                     LIMIT 1 OFFSET ?1
                 )",
                params![cap.0 as i64],
            )
            .context("Failed to enforce usage_events cap")?;

            Ok(())
        })
        .await
    }

    async fn query_usage(&self, q: &UsageQuery) -> Result<Vec<UsageSummary>> {
        let category = q.category.clone();
        let actor = q.actor.clone();
        let since = q.since;
        let limit = q.limit.unwrap_or(50).min(500) as i64;

        self.db_call(move |conn| {
            let mut sql = String::from(
                "SELECT category, action, detail, actor,
                        COUNT(*) AS count,
                        MAX(recorded_at) AS last_used
                 FROM usage_events
                 WHERE 1=1",
            );
            let mut param_idx = 1usize;
            let mut bind_category: Option<String> = None;
            let mut bind_actor: Option<String> = None;
            let mut bind_since: Option<String> = None;

            if let Some(ref cat) = category {
                sql.push_str(&format!(" AND category = ?{param_idx}"));
                bind_category = Some(cat.clone());
                param_idx += 1;
            }
            if let Some(ref act) = actor {
                sql.push_str(&format!(" AND actor = ?{param_idx}"));
                bind_actor = Some(act.clone());
                param_idx += 1;
            }
            if let Some(since_dt) = since {
                sql.push_str(&format!(" AND recorded_at >= ?{param_idx}"));
                bind_since = Some(since_dt.format("%Y-%m-%d %H:%M:%S").to_string());
                param_idx += 1;
            }

            sql.push_str(&format!(
                " GROUP BY category, action, detail, actor
                 ORDER BY count ASC
                 LIMIT ?{param_idx}"
            ));

            let mut stmt = conn
                .prepare(&sql)
                .context("Failed to prepare query_usage")?;

            let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            if let Some(c) = bind_category {
                params_vec.push(Box::new(c));
            }
            if let Some(a) = bind_actor {
                params_vec.push(Box::new(a));
            }
            if let Some(s) = bind_since {
                params_vec.push(Box::new(s));
            }
            params_vec.push(Box::new(limit));

            let params_refs: Vec<&dyn rusqlite::ToSql> =
                params_vec.iter().map(|b| b.as_ref()).collect();

            let rows = stmt
                .query_map(params_refs.as_slice(), |row| {
                    let category: String = row.get(0)?;
                    let action: String = row.get(1)?;
                    let detail: Option<String> = row.get(2)?;
                    let actor: String = row.get(3)?;
                    let count: i64 = row.get(4)?;
                    let last_used_str: String = row.get(5)?;
                    Ok((category, action, detail, actor, count, last_used_str))
                })
                .context("Failed to execute query_usage")?;

            let mut results = Vec::new();
            for row in rows {
                let (category, action, detail, actor, count, last_used_str) =
                    row.context("Failed to read usage_summary row")?;
                let last_used =
                    parse_datetime(&last_used_str).map_err(|e| anyhow::anyhow!("{e}"))?;
                results.push(UsageSummary {
                    category,
                    action,
                    detail,
                    actor,
                    count,
                    last_used,
                });
            }
            Ok(results)
        })
        .await
    }
}
