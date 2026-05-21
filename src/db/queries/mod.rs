mod epics;
mod learnings;
mod projects;
mod settings;
mod tasks;
mod usage;

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};

use crate::models::{
    Epic, EpicId, ProjectId, SubStatus, Task, TaskId, TaskStatus, TaskTag, WrapUpMode,
};

/// Build a `FromSqlConversionFailure` error for an unrecognised enum string.
pub(super) fn unknown_enum(field: &'static str, raw: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        format!("unrecognised {field} value: {raw:?}").into(),
    )
}

/// Column list shared by all task SELECT queries. Pair with `row_to_task`.
pub(super) const TASK_COLUMNS: &str =
    "id, title, description, repo_path, status, worktree, tmux_window, \
     plan_path, epic_id, sub_status, pr_url, tag, sort_order, base_branch, external_id, \
     created_at, updated_at, project_id, labels, last_pre_tool_use_at, last_notification_at, \
     wrap_up_mode";

pub(super) fn row_to_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let status_str: String = row.get("status")?;
    let status =
        TaskStatus::parse(&status_str).ok_or_else(|| unknown_enum("task_status", &status_str))?;

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
        epic_id: row.get::<_, Option<i64>>("epic_id")?.map(EpicId),
        sub_status: parse_sub_status(&row.get::<_, String>("sub_status")?)?,
        pr_url: row.get("pr_url")?,
        tag: parse_tag(row.get("tag")?)?,
        sort_order: row.get("sort_order")?,
        base_branch: row.get("base_branch")?,
        external_id: row.get("external_id")?,
        project_id: ProjectId(row.get::<_, i64>("project_id")?),
        labels: read_json_string_vec(row, "labels")?,
        created_at: parse_datetime(&created_str)?,
        updated_at: parse_datetime(&updated_str)?,
        last_pre_tool_use_at: read_optional_datetime(row, "last_pre_tool_use_at")?,
        last_notification_at: read_optional_datetime(row, "last_notification_at")?,
        wrap_up_mode: parse_wrap_up_mode(row.get("wrap_up_mode")?)?,
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
        status: TaskStatus::parse(&status_str)
            .ok_or_else(|| unknown_enum("epic_status", &status_str))?,
        plan_path: row.get("plan_path")?,
        sort_order: row.get("sort_order")?,
        auto_dispatch: row.get("auto_dispatch")?,
        parent_epic_id: row.get::<_, Option<i64>>("parent_epic_id")?.map(EpicId),
        feed_command: row.get("feed_command")?,
        feed_interval_secs: row.get("feed_interval_secs")?,
        group_by_repo: row.get::<_, bool>("group_by_repo")?,
        project_id: ProjectId(row.get::<_, i64>("project_id")?),
        created_at: parse_datetime(&created_str)?,
        updated_at: parse_datetime(&updated_str)?,
    })
}

/// Decode a JSON-encoded `Vec<String>` column. Returns an error for malformed
/// JSON so corrupt cells surface immediately rather than silently becoming empty.
pub(super) fn read_json_string_vec(
    row: &rusqlite::Row<'_>,
    column: &str,
) -> rusqlite::Result<Vec<String>> {
    let raw: Option<String> = row.get::<_, Option<String>>(column)?;
    match raw {
        None => Ok(Vec::new()),
        Some(s) => serde_json::from_str::<Vec<String>>(&s).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                format!("invalid JSON in column {column:?}: {e}").into(),
            )
        }),
    }
}

fn parse_sub_status(raw: &str) -> rusqlite::Result<SubStatus> {
    SubStatus::parse(raw).ok_or_else(|| unknown_enum("sub_status", raw))
}

fn parse_wrap_up_mode(raw: Option<String>) -> rusqlite::Result<Option<WrapUpMode>> {
    match raw {
        None => Ok(None),
        Some(s) => WrapUpMode::parse(&s)
            .map(Some)
            .ok_or_else(|| unknown_enum("wrap_up_mode", &s)),
    }
}

fn parse_tag(raw: Option<String>) -> rusqlite::Result<Option<TaskTag>> {
    match raw {
        None => Ok(None),
        Some(s) => TaskTag::parse(&s)
            .map(Some)
            .ok_or_else(|| unknown_enum("task_tag", &s)),
    }
}

/// Serialize a `Vec<String>` for storage in a JSON-encoded column.
pub(super) fn write_json_string_vec(values: &[String]) -> Result<String> {
    serde_json::to_string(values).context("Failed to serialize string list to JSON")
}

/// Parse SQLite `datetime('now')` output: "YYYY-MM-DD HH:MM:SS"
pub(super) fn parse_datetime(s: &str) -> rusqlite::Result<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .map(|ndt| Utc.from_utc_datetime(&ndt))
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                format!("invalid datetime {s:?}: {e}").into(),
            )
        })
}

/// Format a `DateTime<Utc>` for storage in TEXT timestamp columns.
/// Pairs with [`parse_datetime`] — both use "YYYY-MM-DD HH:MM:SS".
pub(super) fn format_datetime(dt: DateTime<Utc>) -> String {
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Read a nullable TEXT timestamp column.
pub(super) fn read_optional_datetime(
    row: &rusqlite::Row<'_>,
    col: &str,
) -> rusqlite::Result<Option<DateTime<Utc>>> {
    let s: Option<String> = row.get::<_, Option<String>>(col)?;
    match s {
        None => Ok(None),
        Some(s) => parse_datetime(&s).map(Some),
    }
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
            let show_mode = show_mode_str.parse::<TipsShowMode>().map_err(|e| {
                anyhow::anyhow!("unrecognised tips show_mode {:?}: {}", show_mode_str, e)
            })?;
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
