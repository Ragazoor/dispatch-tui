mod migrations;
mod queries;
#[cfg(test)]
mod tests;

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

use crate::models::{
    Epic, EpicId, FeedItem, Learning, LearningId, LearningKind, LearningScope, LearningStatus,
    Project, ProjectId, SubStatus, Task, TaskId, TaskStatus, TaskTag, TaskUsage, TipsShowMode,
    UsageReport,
};

// ---------------------------------------------------------------------------
// patch_struct! — declarative macro for selective-update builder structs
// ---------------------------------------------------------------------------

/// Generates a lifetime-parameterised builder struct for partial DB updates.
///
/// Each field is wrapped in `Option<…>` (default `None` = don't touch).
/// Two field kinds:
/// - `plain    field: Type` — `Option<Type>` storage; setter takes `Type`.
/// - `nullable field: Type` — `Option<Option<Type>>` storage (double-Option);
///   setter takes `Option<Type>` (allows NULL vs value distinction).
///
/// Also generates `new()` (alias for `Default::default()`) and
/// `has_changes()` (true if any field is `Some`).
macro_rules! patch_struct {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident < $lt:lifetime > {
            $( $kind:ident $field:ident : $ty:ty ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Default)]
        $vis struct $name<$lt> {
            $( pub $field: patch_struct!(@field_type $kind $ty), )*
        }

        impl<$lt> $name<$lt> {
            pub fn new() -> Self { Self::default() }

            $( patch_struct!(@setter $kind $field $ty); )*

            pub fn has_changes(&self) -> bool {
                false $(|| self.$field.is_some())*
            }
        }
    };

    (@field_type plain    $ty:ty) => { Option<$ty> };
    (@field_type nullable $ty:ty) => { Option<Option<$ty>> };

    (@setter plain    $field:ident $ty:ty) => {
        pub fn $field(mut self, v: $ty) -> Self { self.$field = Some(v); self }
    };
    (@setter nullable $field:ident $ty:ty) => {
        pub fn $field(mut self, v: Option<$ty>) -> Self { self.$field = Some(v); self }
    };
}

// ---------------------------------------------------------------------------
// TaskPatch — builder for selective field updates
// ---------------------------------------------------------------------------

patch_struct! {
    /// Builder for selective task field updates.
    pub struct TaskPatch<'a> {
        plain    status:       TaskStatus,
        nullable plan_path:    &'a str,
        plain    title:        &'a str,
        plain    description:  &'a str,
        plain    repo_path:    &'a str,
        nullable worktree:     &'a str,
        nullable tmux_window:  &'a str,
        plain    sub_status:   SubStatus,
        nullable pr_url:       &'a str,
        nullable tag:          TaskTag,
        nullable sort_order:   i64,
        plain    base_branch:  &'a str,
        nullable external_id:  &'a str,
        plain    project_id:   ProjectId,
    }
}

// ---------------------------------------------------------------------------
// CreateTaskRequest — input struct for the create_task DB operation
// ---------------------------------------------------------------------------

/// All parameters needed to insert a new task row.
#[derive(Debug)]
pub struct CreateTaskRequest<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub repo_path: &'a str,
    pub plan: Option<&'a str>,
    pub status: TaskStatus,
    pub base_branch: &'a str,
    pub epic_id: Option<EpicId>,
    pub sort_order: Option<i64>,
    pub tag: Option<TaskTag>,
    pub project_id: ProjectId,
}

// ---------------------------------------------------------------------------
// EpicPatch — builder for selective epic field updates
// ---------------------------------------------------------------------------

patch_struct! {
    /// Builder for selective epic field updates.
    ///
    /// # Why `parent_epic_id` is absent
    ///
    /// Reparenting an epic is not supported. `parent_epic_id` is set once at
    /// creation time via [`EpicCrud::create_epic`] and never changed afterward.
    /// This keeps the parent chain immutable and prevents accidental cycle
    /// introduction. The database enforces `CHECK (parent_epic_id != id)` (added
    /// in migration v35) as a final guard against self-loops.
    pub struct EpicPatch<'a> {
        plain    title:              &'a str,
        plain    description:        &'a str,
        plain    status:             TaskStatus,
        nullable plan_path:          &'a str,
        nullable sort_order:         i64,
        plain    repo_path:          &'a str,
        plain    auto_dispatch:      bool,
        nullable feed_command:       &'a str,
        nullable feed_interval_secs: i64,
        plain    project_id:         ProjectId,
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
    fn create_task(&self, req: CreateTaskRequest<'_>) -> Result<TaskId>;
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
    fn report_usage(&self, task_id: TaskId, usage: &UsageReport) -> Result<()>;
    fn get_all_usage(&self) -> Result<Vec<TaskUsage>>;
    /// Upsert tasks from a feed. Inserts new tasks; on conflict (epic_id, external_id)
    /// updates title and description only — status and other user-managed fields are preserved.
    ///
    /// `repo_paths` is a parallel slice: `repo_paths[i]` is the resolved local path for
    /// `items[i]`. Pass `""` when the path could not be resolved — dispatch will be blocked
    /// until the user sets it via the task editor.
    ///
    /// `base_branches` is a parallel slice: `base_branches[i]` is the base branch the
    /// inserted task should use for its worktree. The caller is expected to resolve the
    /// repo's default branch (typically via [`crate::git::detect_default_branch`]),
    /// falling back to `"main"` when no path is known.
    fn upsert_feed_tasks(
        &self,
        epic_id: EpicId,
        items: &[FeedItem],
        repo_paths: &[String],
        base_branches: &[String],
    ) -> Result<()>;
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
        project_id: ProjectId,
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
    /// Returns the agent status for a single PR if an agent is active, without loading all rows.
    fn pr_agent_status(
        &self,
        table: &str,
        repo: &str,
        number: i64,
    ) -> Result<Option<crate::models::ReviewAgentStatus>>;
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
    /// Returns the agent status for a single alert if an agent is active, without loading all rows.
    fn alert_agent_status(
        &self,
        repo: &str,
        number: i64,
        kind: crate::models::AlertKind,
    ) -> Result<Option<crate::models::ReviewAgentStatus>>;
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
    fn save_filter_preset(&self, name: &str, repo_paths: &[String], mode: &str) -> Result<()>;
    fn delete_filter_preset(&self, name: &str) -> Result<()>;
    fn list_filter_presets(&self) -> Result<Vec<(String, Vec<String>, String)>>;
    fn get_tips_state(&self) -> Result<(u32, crate::models::TipsShowMode)>;
    fn save_tips_state(
        &self,
        seen_up_to: u32,
        show_mode: crate::models::TipsShowMode,
    ) -> Result<()>;
}

// ---------------------------------------------------------------------------
// TaskAndEpicStore — composite for consumers that need tasks + epics only
// ---------------------------------------------------------------------------

pub trait TaskAndEpicStore: TaskCrud + EpicCrud {}

impl<T: TaskCrud + EpicCrud> TaskAndEpicStore for T {}

// ---------------------------------------------------------------------------
// PrWorkflowRow — a row from the pr_workflow_states table
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PrWorkflowRow {
    pub repo: String,
    pub number: i64,
    pub kind: crate::models::WorkflowItemKind,
    pub state: String,
    pub sub_state: Option<String>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

// ---------------------------------------------------------------------------
// PrWorkflowStore — CRUD for the pr_workflow_states table
// ---------------------------------------------------------------------------

pub trait PrWorkflowStore: Send + Sync {
    /// INSERT OR IGNORE — never overwrites an existing row (preserves workflow state).
    fn insert_pr_workflow_if_absent(
        &self,
        repo: &str,
        number: i64,
        kind: crate::models::WorkflowItemKind,
    ) -> anyhow::Result<()>;

    /// INSERT OR REPLACE — always sets state and sub_state.
    fn upsert_pr_workflow(
        &self,
        repo: &str,
        number: i64,
        kind: crate::models::WorkflowItemKind,
        state: &str,
        sub_state: Option<&str>,
    ) -> anyhow::Result<()>;

    fn get_pr_workflow(
        &self,
        repo: &str,
        number: i64,
        kind: crate::models::WorkflowItemKind,
    ) -> anyhow::Result<Option<PrWorkflowRow>>;

    fn list_pr_workflows(&self) -> anyhow::Result<Vec<PrWorkflowRow>>;

    /// Return the kind of the first workflow row for (repo, number), or None if absent.
    fn find_pr_workflow_kind(
        &self,
        repo: &str,
        number: i64,
    ) -> anyhow::Result<Option<crate::models::WorkflowItemKind>>;

    /// Delete rows where state = 'done' AND updated_at < (now - older_than).
    fn prune_done_pr_workflows(&self, older_than: chrono::Duration) -> anyhow::Result<()>;
}

// ---------------------------------------------------------------------------
// ProjectCrud — CRUD for the projects table
// ---------------------------------------------------------------------------

pub trait ProjectCrud: Send + Sync {
    fn create_project(&self, name: &str, sort_order: i64) -> Result<Project>;
    fn list_projects(&self) -> Result<Vec<Project>>;
    fn get_default_project(&self) -> Result<Project>;
    fn rename_project(&self, id: ProjectId, name: &str) -> Result<()>;
    /// Move all tasks and epics from `from` to `to`, then delete the `from` project.
    /// The entire operation runs in a single transaction.
    fn delete_project_and_move_items(&self, id: ProjectId, default_id: ProjectId) -> Result<()>;
    fn reorder_project(&self, id: ProjectId, new_sort_order: i64) -> Result<()>;
}

// ---------------------------------------------------------------------------
// LearningPatch — builder for partial learning updates
// ---------------------------------------------------------------------------

patch_struct! {
    /// Builder for selective learning field updates.
    pub struct LearningPatch<'a> {
        plain    status:  LearningStatus,
        plain    summary: &'a str,
        nullable detail:  &'a str,
        plain    kind:    LearningKind,
        plain    tags:    &'a [String],
    }
}

// ---------------------------------------------------------------------------
// LearningFilter — optional filter for list_learnings
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct LearningFilter {
    pub status: Option<LearningStatus>,
    pub scope: Option<LearningScope>,
    pub scope_ref: Option<String>,
    /// Return only learnings whose tags intersect this set (OR match).
    pub tags: Vec<String>,
    pub limit: Option<usize>,
}

// ---------------------------------------------------------------------------
// LearningStore — narrow sub-trait for the learnings table
// ---------------------------------------------------------------------------

pub trait LearningStore: Send + Sync {
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
    ) -> Result<LearningId>;

    fn get_learning(&self, id: LearningId) -> Result<Option<Learning>>;

    fn list_learnings(&self, filter: LearningFilter) -> Result<Vec<Learning>>;

    fn patch_learning(&self, id: LearningId, patch: &LearningPatch<'_>) -> Result<()>;

    fn delete_learning(&self, id: LearningId) -> Result<()>;

    /// Atomically increments `confirmed_count` and updates `last_confirmed_at` and `updated_at`.
    fn confirm_learning(&self, id: LearningId) -> Result<()>;

    /// Returns approved learnings for the given task context, unioning user + project + repo + epic
    /// scopes. Task-scoped learnings are excluded (they surface via explicit query only).
    /// Ordered by scope priority (procedural > epic > repo > project > user), then confirmed_count DESC.
    fn list_learnings_for_dispatch(
        &self,
        project_id: Option<ProjectId>,
        repo_path: &str,
        epic_id: Option<EpicId>,
    ) -> Result<Vec<Learning>>;
}

// ---------------------------------------------------------------------------
// TaskStore — supertrait combining all sub-traits
// ---------------------------------------------------------------------------

pub trait TaskStore:
    TaskAndEpicStore
    + PrStore
    + AlertStore
    + SettingsStore
    + PrWorkflowStore
    + ProjectCrud
    + LearningStore
{
}

impl<
        T: TaskAndEpicStore
            + PrStore
            + AlertStore
            + SettingsStore
            + PrWorkflowStore
            + ProjectCrud
            + LearningStore,
    > TaskStore for T
{
}

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
