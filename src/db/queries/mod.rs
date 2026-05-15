mod epics;
mod learnings;
mod projects;
mod settings;
mod tasks;

use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};

use crate::models::{
    Epic, EpicId, ProjectId, SubStatus, Task, TaskId, TaskStatus, TaskTag, WrapUpMode,
};

/// Process-wide counter incremented each time a row decode falls back to a
/// default value (unknown enum string, malformed JSON list, etc.). Surfaces
/// slow-bleeding migration/decoding bugs that the per-warn `tracing::warn!`
/// lines alone are too easy to miss.
static DECODE_FALLBACK_COUNT: AtomicU64 = AtomicU64::new(0);

/// Returns the current value of the process-wide decode-fallback counter.
pub fn decode_fallback_count() -> u64 {
    DECODE_FALLBACK_COUNT.load(Ordering::Relaxed)
}

fn bump_decode_fallback() -> u64 {
    DECODE_FALLBACK_COUNT.fetch_add(1, Ordering::Relaxed) + 1
}

/// Column list shared by all task SELECT queries. Pair with `row_to_task`.
pub(super) const TASK_COLUMNS: &str =
    "id, title, description, repo_path, status, worktree, tmux_window, \
     plan_path, epic_id, sub_status, pr_url, tag, sort_order, base_branch, external_id, \
     created_at, updated_at, project_id, labels, last_pre_tool_use_at, last_notification_at, \
     wrap_up_mode";

pub(super) fn row_to_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let status_str: String = row.get("status")?;
    let status = TaskStatus::parse(&status_str).unwrap_or_else(|| {
        let count = bump_decode_fallback();
        tracing::warn!(
            raw = %status_str,
            count,
            "unrecognised task status, defaulting to Backlog"
        );
        TaskStatus::Backlog
    });

    let created_str: String = row.get("created_at")?;
    let updated_str: String = row.get("updated_at")?;

    Ok(Task {
        id: TaskId(row.get("id")?),
        title: row.get("title")?,
        description: row.get("description")?,
        repo_path: row.get("repo_path")?,
        status,
        worktree: row.get("worktree")?,
        tmux_window: row.get("tmux_window")?,
        plan_path: row.get("plan_path")?,
        epic_id: row
            .get::<_, Option<i64>>("epic_id")
            .unwrap_or(None)
            .map(EpicId),
        sub_status: parse_sub_status_or_warn(row.get::<_, String>("sub_status").ok()),
        pr_url: row.get::<_, Option<String>>("pr_url").unwrap_or(None),
        tag: parse_tag_or_warn(row.get::<_, Option<String>>("tag").unwrap_or(None)),
        sort_order: row.get::<_, Option<i64>>("sort_order").unwrap_or(None),
        base_branch: row
            .get::<_, Option<String>>("base_branch")
            .unwrap_or(None)
            .unwrap_or_else(|| "main".to_string()),
        external_id: row.get::<_, Option<String>>("external_id").unwrap_or(None),
        project_id: ProjectId(row.get::<_, i64>("project_id")?),
        labels: read_json_string_vec(row, "labels"),
        created_at: parse_datetime(&created_str),
        updated_at: parse_datetime(&updated_str),
        last_pre_tool_use_at: read_optional_datetime(row, "last_pre_tool_use_at"),
        last_notification_at: read_optional_datetime(row, "last_notification_at"),
        wrap_up_mode: parse_wrap_up_mode_or_warn(
            row.get::<_, Option<String>>("wrap_up_mode").unwrap_or(None),
        ),
    })
}

pub(super) fn row_to_epic(row: &rusqlite::Row<'_>) -> rusqlite::Result<Epic> {
    let created_str: String = row.get("created_at")?;
    let updated_str: String = row.get("updated_at")?;
    let status_str: String = row.get("status")?;

    Ok(Epic {
        id: EpicId(row.get("id")?),
        title: row.get("title")?,
        description: row.get("description")?,
        repo_path: row.get("repo_path")?,
        status: TaskStatus::parse(&status_str).unwrap_or_else(|| {
            let count = bump_decode_fallback();
            tracing::warn!(
                raw = %status_str,
                count,
                "unrecognised epic status, defaulting to Backlog"
            );
            TaskStatus::Backlog
        }),
        plan_path: row.get("plan_path")?,
        sort_order: row.get::<_, Option<i64>>("sort_order").unwrap_or(None),
        auto_dispatch: row.get::<_, bool>("auto_dispatch").unwrap_or(true),
        parent_epic_id: row
            .get::<_, Option<i64>>("parent_epic_id")
            .unwrap_or(None)
            .map(EpicId),
        feed_command: row.get::<_, Option<String>>("feed_command").unwrap_or(None),
        feed_interval_secs: row
            .get::<_, Option<i64>>("feed_interval_secs")
            .unwrap_or(None),
        group_by_repo: row.get::<_, bool>("group_by_repo").unwrap_or(false),
        project_id: ProjectId(row.get::<_, i64>("project_id")?),
        created_at: parse_datetime(&created_str),
        updated_at: parse_datetime(&updated_str),
    })
}

/// Decode a JSON-encoded `Vec<String>` column. Tolerates NULL, missing
/// columns, and malformed JSON by defaulting to an empty vector — a
/// corrupt cell must never crash the TUI.
pub(super) fn read_json_string_vec(row: &rusqlite::Row<'_>, column: &str) -> Vec<String> {
    let raw: Option<String> = row.get::<_, Option<String>>(column).ok().flatten();
    match raw {
        Some(s) => match serde_json::from_str::<Vec<String>>(&s) {
            Ok(v) => v,
            Err(e) => {
                let count = bump_decode_fallback();
                tracing::warn!(
                    column,
                    raw = %s,
                    error = %e,
                    count,
                    "malformed JSON list, defaulting to empty"
                );
                Vec::new()
            }
        },
        None => Vec::new(),
    }
}

fn parse_sub_status_or_warn(raw: Option<String>) -> SubStatus {
    match raw {
        Some(s) => match SubStatus::parse(&s) {
            Some(v) => v,
            None => {
                let count = bump_decode_fallback();
                tracing::warn!(
                    raw = %s,
                    count,
                    "unrecognised sub_status, defaulting to None"
                );
                SubStatus::None
            }
        },
        None => SubStatus::None,
    }
}

fn parse_wrap_up_mode_or_warn(raw: Option<String>) -> Option<WrapUpMode> {
    let s = raw?;
    WrapUpMode::parse(&s).or_else(|| {
        let count = bump_decode_fallback();
        tracing::warn!(raw = %s, count, "unrecognised wrap_up_mode, dropping");
        None
    })
}

fn parse_tag_or_warn(raw: Option<String>) -> Option<TaskTag> {
    let s = raw?;
    match TaskTag::parse(&s) {
        Some(v) => Some(v),
        None => {
            let count = bump_decode_fallback();
            tracing::warn!(raw = %s, count, "unrecognised task tag, dropping");
            None
        }
    }
}

/// Serialize a `Vec<String>` for storage in a JSON-encoded column.
pub(super) fn write_json_string_vec(values: &[String]) -> Result<String> {
    serde_json::to_string(values).context("Failed to serialize string list to JSON")
}

/// Parse SQLite `datetime('now')` output: "YYYY-MM-DD HH:MM:SS"
pub(super) fn parse_datetime(s: &str) -> DateTime<Utc> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|ndt| Utc.from_utc_datetime(&ndt))
        .unwrap_or_else(Utc::now)
}

/// Format a `DateTime<Utc>` for storage in TEXT timestamp columns.
/// Pairs with [`parse_datetime`] — both use "YYYY-MM-DD HH:MM:SS".
pub(super) fn format_datetime(dt: DateTime<Utc>) -> String {
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Read a nullable TEXT timestamp column.
pub(super) fn read_optional_datetime(row: &rusqlite::Row<'_>, col: &str) -> Option<DateTime<Utc>> {
    row.get::<_, Option<String>>(col)
        .ok()
        .flatten()
        .map(|s| parse_datetime(&s))
}

pub(super) fn get_tips_state(
    conn: &rusqlite::Connection,
) -> Result<(u32, crate::models::TipsShowMode)> {
    use crate::models::TipsShowMode;
    let result = conn.query_row(
        "SELECT seen_up_to, show_mode FROM tips_state WHERE id = 1",
        [],
        |row| {
            let seen_up_to: u32 = row.get(0)?;
            let show_mode_str: String = row.get(1)?;
            Ok((seen_up_to, show_mode_str))
        },
    );

    match result {
        Ok((seen_up_to, show_mode_str)) => {
            let show_mode = show_mode_str
                .parse::<TipsShowMode>()
                .unwrap_or(TipsShowMode::Always);
            Ok((seen_up_to, show_mode))
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok((0, TipsShowMode::Always)),
        Err(e) => Err(e).context("Failed to read tips_state"),
    }
}

pub(super) fn save_tips_state(
    conn: &rusqlite::Connection,
    seen_up_to: u32,
    show_mode: crate::models::TipsShowMode,
) -> Result<()> {
    let rows = conn
        .execute(
            "UPDATE tips_state SET seen_up_to = ?1, show_mode = ?2 WHERE id = 1",
            rusqlite::params![seen_up_to, show_mode.as_str()],
        )
        .context("Failed to save tips_state")?;
    if rows != 1 {
        anyhow::bail!("save_tips_state: expected 1 row updated, got {rows}");
    }
    Ok(())
}
