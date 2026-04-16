mod migrations;
mod queries;
#[cfg(test)]
mod tests;

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

use crate::models::{
    Epic, EpicId, SubStatus, Task, TaskId, TaskStatus, TaskTag, TaskUsage, UsageReport,
};
use crate::tui::types::TipsShowMode;

// ---------------------------------------------------------------------------
// TaskPatch — builder for selective field updates
// ---------------------------------------------------------------------------

/// Builder for partial task updates. Each field is `None` by default (= don't
/// change). For nullable columns (`plan_path`, `worktree`, `tmux_window`) we use
/// a double-Option: `None` = don't change, `Some(None)` = set NULL,
/// `Some(Some(x))` = set value.
#[derive(Debug, Default)]
pub struct TaskPatch<'a> {
    pub status: Option<TaskStatus>,
    pub plan_path: Option<Option<&'a str>>,
    pub title: Option<&'a str>,
    pub description: Option<&'a str>,
    pub repo_path: Option<&'a str>,
    pub worktree: Option<Option<&'a str>>,
    pub tmux_window: Option<Option<&'a str>>,
    pub sub_status: Option<SubStatus>,
    pub pr_url: Option<Option<&'a str>>,
    pub tag: Option<Option<TaskTag>>,
    pub sort_order: Option<Option<i64>>,
    pub base_branch: Option<&'a str>,
}

impl<'a> TaskPatch<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn status(mut self, status: TaskStatus) -> Self {
        self.status = Some(status);
        self
    }

    pub fn plan_path(mut self, plan_path: Option<&'a str>) -> Self {
        self.plan_path = Some(plan_path);
        self
    }

    pub fn title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    pub fn description(mut self, description: &'a str) -> Self {
        self.description = Some(description);
        self
    }

    pub fn repo_path(mut self, repo_path: &'a str) -> Self {
        self.repo_path = Some(repo_path);
        self
    }

    pub fn worktree(mut self, worktree: Option<&'a str>) -> Self {
        self.worktree = Some(worktree);
        self
    }

    pub fn tmux_window(mut self, tmux_window: Option<&'a str>) -> Self {
        self.tmux_window = Some(tmux_window);
        self
    }

    pub fn sub_status(mut self, sub_status: SubStatus) -> Self {
        self.sub_status = Some(sub_status);
        self
    }

    pub fn pr_url(mut self, pr_url: Option<&'a str>) -> Self {
        self.pr_url = Some(pr_url);
        self
    }

    pub fn tag(mut self, tag: Option<TaskTag>) -> Self {
        self.tag = Some(tag);
        self
    }

    pub fn sort_order(mut self, sort_order: Option<i64>) -> Self {
        self.sort_order = Some(sort_order);
        self
    }

    pub fn base_branch(mut self, base_branch: &'a str) -> Self {
        self.base_branch = Some(base_branch);
        self
    }

    pub fn has_changes(&self) -> bool {
        self.status.is_some()
            || self.plan_path.is_some()
            || self.title.is_some()
            || self.description.is_some()
            || self.repo_path.is_some()
            || self.worktree.is_some()
            || self.tmux_window.is_some()
            || self.sub_status.is_some()
            || self.pr_url.is_some()
            || self.tag.is_some()
            || self.sort_order.is_some()
            || self.base_branch.is_some()
    }
}

// ---------------------------------------------------------------------------
// EpicPatch — builder for selective epic field updates
// ---------------------------------------------------------------------------

/// Builder for partial epic updates, mirroring `TaskPatch`. Each field is
/// `None` by default (= don't change). For nullable columns (`plan_path`) we use
/// a double-Option: `None` = don't change, `Some(None)` = set NULL,
/// `Some(Some(x))` = set value.
///
/// # Why `parent_epic_id` is absent
///
/// Reparenting an epic is not supported. `parent_epic_id` is set once at
/// creation time via [`EpicCrud::create_epic`] and never changed afterward.
/// This keeps the parent chain immutable and prevents accidental cycle
/// introduction. The database enforces `CHECK (parent_epic_id != id)` (added
/// in migration v35) as a final guard against self-loops.
#[derive(Debug, Default)]
pub struct EpicPatch<'a> {
    pub title: Option<&'a str>,
    pub description: Option<&'a str>,
    pub status: Option<TaskStatus>,
    pub plan_path: Option<Option<&'a str>>,
    pub sort_order: Option<Option<i64>>,
    pub repo_path: Option<&'a str>,
    pub auto_dispatch: Option<bool>,
}

impl<'a> EpicPatch<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    pub fn description(mut self, description: &'a str) -> Self {
        self.description = Some(description);
        self
    }

    pub fn status(mut self, status: TaskStatus) -> Self {
        self.status = Some(status);
        self
    }

    pub fn plan_path(mut self, plan_path: Option<&'a str>) -> Self {
        self.plan_path = Some(plan_path);
        self
    }

    pub fn sort_order(mut self, sort_order: Option<i64>) -> Self {
        self.sort_order = Some(sort_order);
        self
    }

    pub fn repo_path(mut self, repo_path: &'a str) -> Self {
        self.repo_path = Some(repo_path);
        self
    }

    pub fn auto_dispatch(mut self, auto_dispatch: bool) -> Self {
        self.auto_dispatch = Some(auto_dispatch);
        self
    }

    pub fn has_changes(&self) -> bool {
        self.title.is_some()
            || self.description.is_some()
            || self.status.is_some()
            || self.plan_path.is_some()
            || self.sort_order.is_some()
            || self.repo_path.is_some()
            || self.auto_dispatch.is_some()
    }
}

// ---------------------------------------------------------------------------
// PrKind — selects which PR table to operate on
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrKind {
    Review,
    My,
    Bot,
}

impl PrKind {
    pub fn table_name(self) -> &'static str {
        match self {
            PrKind::Review => "review_prs",
            PrKind::My => "my_prs",
            PrKind::Bot => "bot_prs",
        }
    }
}

// ---------------------------------------------------------------------------
// Sub-traits — focused slices of the database API
// ---------------------------------------------------------------------------

/// Task CRUD, list, patch, status updates.
pub trait TaskCrud: Send + Sync {
    fn create_task(
        &self,
        title: &str,
        description: &str,
        repo_path: &str,
        plan: Option<&str>,
        status: TaskStatus,
        base_branch: &str,
    ) -> Result<TaskId>;
    fn get_task(&self, id: TaskId) -> Result<Option<Task>>;
    fn list_all(&self) -> Result<Vec<Task>>;
    fn list_by_status(&self, status: TaskStatus) -> Result<Vec<Task>>;
    /// Update status only if current status matches `expected`. Returns true if updated.
    fn update_status_if(
        &self,
        id: TaskId,
        new_status: TaskStatus,
        expected: TaskStatus,
    ) -> Result<bool>;
    fn delete_task(&self, id: TaskId) -> Result<()>;
    fn find_task_by_plan(&self, plan: &str) -> Result<Option<Task>>;
    fn patch_task(&self, id: TaskId, patch: &TaskPatch<'_>) -> Result<()>;
    fn has_other_tasks_with_worktree(&self, worktree: &str, exclude_id: TaskId) -> Result<bool>;
}

/// Epic CRUD, list, patch, recalculate status.
pub trait EpicCrud: Send + Sync {
    /// Create a new epic. `parent_epic_id` is set once here and never changed
    /// via [`EpicPatch`] (reparenting is unsupported). The DB enforces
    /// `CHECK (parent_epic_id != id)` (migration v35) to prevent self-loops,
    /// and `recalculate_epic_status` uses a visited set to guard against any
    /// cycle that might exist in older data.
    fn create_epic(
        &self,
        title: &str,
        description: &str,
        repo_path: &str,
        parent_epic_id: Option<EpicId>,
    ) -> Result<Epic>;
    fn get_epic(&self, id: EpicId) -> Result<Option<Epic>>;
    fn list_epics(&self) -> Result<Vec<Epic>>;
    /// List only root epics (no parent). Used for the main board view.
    fn list_root_epics(&self) -> Result<Vec<Epic>>;
    /// List direct children of the given epic.
    fn list_sub_epics(&self, parent_id: EpicId) -> Result<Vec<Epic>>;
    fn patch_epic(&self, id: EpicId, patch: &EpicPatch<'_>) -> Result<()>;
    fn delete_epic(&self, id: EpicId) -> Result<()>;
    fn set_task_epic_id(&self, task_id: TaskId, epic_id: Option<EpicId>) -> Result<()>;
    fn list_tasks_for_epic(&self, epic_id: EpicId) -> Result<Vec<Task>>;
    /// Fetch all tasks that have a non-null epic_id in a single query.
    /// Use instead of looping over epics and calling list_tasks_for_epic() per epic.
    fn list_all_tasks_with_epic_id(&self) -> Result<Vec<Task>>;
    /// Recalculate an epic's status from its active children (tasks + sub-epics).
    /// Propagates upward to the parent epic if one exists.
    fn recalculate_epic_status(&self, epic_id: EpicId) -> Result<()>;
}

/// Save/load PRs (all kinds) and agent tracking on PRs.
pub trait PrStore: Send + Sync {
    fn save_prs(&self, kind: PrKind, prs: &[crate::models::ReviewPr]) -> Result<()>;
    fn load_prs(&self, kind: PrKind) -> Result<Vec<crate::models::ReviewPr>>;
    fn set_pr_agent(
        &self,
        kind: PrKind,
        repo: &str,
        number: i64,
        tmux_window: &str,
        worktree: &str,
    ) -> Result<bool>; // true = row updated
    fn update_agent_status(&self, repo: &str, number: i64, status: Option<&str>) -> Result<String>;
    /// Look up a single PR by (repo, number) — checks review_prs then my_prs.
    fn get_review_pr(&self, repo: &str, number: i64) -> Result<Option<crate::models::ReviewPr>>;
}

/// Save/load security alerts and agent tracking on alerts.
pub trait AlertStore: Send + Sync {
    fn save_security_alerts(&self, alerts: &[crate::models::SecurityAlert]) -> Result<()>;
    fn load_security_alerts(&self) -> Result<Vec<crate::models::SecurityAlert>>;
    fn set_alert_agent(
        &self,
        repo: &str,
        number: i64,
        kind: crate::models::AlertKind,
        tmux_window: &str,
        worktree: &str,
    ) -> Result<bool>; // true = row updated
    /// Look up a single security alert by (repo, number, kind).
    fn get_security_alert(
        &self,
        repo: &str,
        number: i64,
        kind: crate::models::AlertKind,
    ) -> Result<Option<crate::models::SecurityAlert>>;
}

/// Settings, filter presets, repo paths, and usage tracking.
pub trait SettingsStore: Send + Sync {
    fn list_repo_paths(&self) -> Result<Vec<String>>;
    fn save_repo_path(&self, path: &str) -> Result<()>;
    fn delete_repo_path(&self, path: &str) -> Result<()>;
    fn get_setting_bool(&self, key: &str) -> Result<Option<bool>>;
    fn set_setting_bool(&self, key: &str, value: bool) -> Result<()>;
    fn get_setting_string(&self, key: &str) -> Result<Option<String>>;
    fn set_setting_string(&self, key: &str, value: &str) -> Result<()>;
    /// Seed default GitHub query strings if not already set.
    fn seed_github_query_defaults(&self) -> Result<()>;
    fn report_usage(&self, task_id: TaskId, usage: &UsageReport) -> Result<()>;
    fn get_all_usage(&self) -> Result<Vec<TaskUsage>>;
    fn save_filter_preset(&self, name: &str, repo_paths: &[String], mode: &str) -> Result<()>;
    fn delete_filter_preset(&self, name: &str) -> Result<()>;
    fn list_filter_presets(&self) -> Result<Vec<(String, Vec<String>, String)>>;
}

// ---------------------------------------------------------------------------
// TaskStore — supertrait combining all sub-traits
// ---------------------------------------------------------------------------

pub trait TaskStore: TaskCrud + EpicCrud + PrStore + AlertStore + SettingsStore {}

impl<T: TaskCrud + EpicCrud + PrStore + AlertStore + SettingsStore> TaskStore for T {}

// ---------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        // Ensure the parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create db directory: {}", parent.display()))?;
        }

        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open database at {}", path.display()))?;

        Self::init_schema(&conn)?;

        Ok(Database {
            conn: Mutex::new(conn),
        })
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("Failed to open in-memory database")?;
        Self::init_schema(&conn)?;
        Ok(Database {
            conn: Mutex::new(conn),
        })
    }

    fn init_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=5000;",
        )
        .context("Failed to set PRAGMAs")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tasks (
                id          INTEGER PRIMARY KEY,
                title       TEXT NOT NULL,
                description TEXT NOT NULL,
                repo_path   TEXT NOT NULL,
                status      TEXT NOT NULL DEFAULT 'backlog',
                worktree    TEXT,
                tmux_window TEXT,
                plan_path   TEXT,
                tag         TEXT,
                created_at  TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS repo_paths (
                id        INTEGER PRIMARY KEY,
                path      TEXT NOT NULL UNIQUE,
                last_used TEXT NOT NULL DEFAULT (datetime('now'))
            );",
        )
        .context("Failed to create schema")?;

        // Versioned migrations using PRAGMA user_version
        let current_version: i64 =
            conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

        for &(version, migrate_fn) in migrations::MIGRATIONS {
            if current_version < version {
                migrate_fn(conn)?;
                conn.pragma_update(None, "user_version", version)
                    .with_context(|| format!("Failed to update schema version to {version}"))?;
            }
        }

        Ok(())
    }

    fn conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db lock poisoned"))
    }

    pub fn get_tips_state(&self) -> Result<(u32, TipsShowMode)> {
        let conn = self.conn()?;
        queries::get_tips_state(&conn)
    }

    pub fn save_tips_state(&self, seen_up_to: u32, show_mode: TipsShowMode) -> Result<()> {
        let conn = self.conn()?;
        queries::save_tips_state(&conn, seen_up_to, show_mode)
    }
}
