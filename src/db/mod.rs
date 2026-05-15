mod migrations;
mod queries;
#[cfg(test)]
mod tests;

pub use queries::decode_fallback_count;

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;

use crate::models::{
    Epic, EpicId, FeedItem, Learning, LearningId, LearningKind, LearningRetrieval, LearningScope,
    LearningStatus, LearningVerdict, Project, ProjectId, RetrievalSource, SubStatus, Task, TaskId,
    TaskStatus, TaskTag, TaskUsage, UsageReport, WrapUpMode,
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
        plain    labels:       &'a [String],
        nullable last_pre_tool_use_at: chrono::DateTime<chrono::Utc>,
        nullable last_notification_at: chrono::DateTime<chrono::Utc>,
        nullable wrap_up_mode: WrapUpMode,
    }
}

// ---------------------------------------------------------------------------
// CreateTaskRequest — input struct for the create_task DB operation
// ---------------------------------------------------------------------------

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
    pub wrap_up_mode: Option<WrapUpMode>,
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
        plain    group_by_repo:      bool,
        nullable feed_command:       &'a str,
        nullable feed_interval_secs: i64,
        plain    project_id:         ProjectId,
    }
}

// ---------------------------------------------------------------------------
// Sub-traits — focused slices of the database API
// ---------------------------------------------------------------------------

/// Task CRUD, list, patch, status updates.
#[async_trait::async_trait]
pub trait TaskCrud: Send + Sync {
    async fn create_task(&self, req: CreateTaskRequest<'_>) -> Result<TaskId>;
    async fn get_task(&self, id: TaskId) -> Result<Option<Task>>;
    async fn list_all(&self) -> Result<Vec<Task>>;
    async fn list_by_status(&self, status: TaskStatus) -> Result<Vec<Task>>;
    /// Update status only if current status matches `expected`. Returns true if updated.
    async fn update_status_if(
        &self,
        id: TaskId,
        new_status: TaskStatus,
        expected: TaskStatus,
    ) -> Result<bool>;
    async fn delete_task(&self, id: TaskId) -> Result<()>;
    async fn find_task_by_plan(&self, plan: &str) -> Result<Option<Task>>;
    async fn patch_task(&self, id: TaskId, patch: &TaskPatch<'_>) -> Result<()>;
    async fn has_other_tasks_with_worktree(
        &self,
        worktree: &str,
        exclude_id: TaskId,
    ) -> Result<bool>;
    async fn report_usage(&self, task_id: TaskId, usage: &UsageReport) -> Result<()>;
    async fn get_all_usage(&self) -> Result<Vec<TaskUsage>>;
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
    async fn upsert_feed_tasks(
        &self,
        epic_id: EpicId,
        items: &[FeedItem],
        repo_paths: &[String],
        base_branches: &[String],
    ) -> Result<()>;
}

/// Epic CRUD, list, patch, recalculate status.
#[async_trait::async_trait]
pub trait EpicCrud: Send + Sync {
    /// Create a new epic. `parent_epic_id` is set once here and never changed
    /// via [`EpicPatch`] (reparenting is unsupported). The DB enforces
    /// `CHECK (parent_epic_id != id)` (migration v35) to prevent self-loops,
    /// and `recalculate_epic_status` uses a visited set to guard against any
    /// cycle that might exist in older data.
    async fn create_epic(
        &self,
        title: &str,
        description: &str,
        repo_path: &str,
        parent_epic_id: Option<EpicId>,
        project_id: ProjectId,
    ) -> Result<Epic>;
    async fn get_epic(&self, id: EpicId) -> Result<Option<Epic>>;
    async fn list_epics(&self) -> Result<Vec<Epic>>;
    /// List only root epics (no parent). Used for the main board view.
    async fn list_root_epics(&self) -> Result<Vec<Epic>>;
    /// List direct children of the given epic.
    async fn list_sub_epics(&self, parent_id: EpicId) -> Result<Vec<Epic>>;
    async fn patch_epic(&self, id: EpicId, patch: &EpicPatch<'_>) -> Result<()>;
    async fn delete_epic(&self, id: EpicId) -> Result<()>;
    async fn set_task_epic_id(&self, task_id: TaskId, epic_id: Option<EpicId>) -> Result<()>;
    async fn list_tasks_for_epic(&self, epic_id: EpicId) -> Result<Vec<Task>>;
    /// Fetch all tasks that have a non-null epic_id in a single query.
    /// Use instead of looping over epics and calling list_tasks_for_epic() per epic.
    async fn list_all_tasks_with_epic_id(&self) -> Result<Vec<Task>>;
    /// Recalculate an epic's status from its active children (tasks + sub-epics).
    /// Propagates upward to the parent epic if one exists.
    async fn recalculate_epic_status(&self, epic_id: EpicId) -> Result<()>;
}

/// Settings, filter presets, repo paths, and usage tracking.
#[async_trait::async_trait]
pub trait SettingsStore: Send + Sync {
    async fn list_repo_paths(&self) -> Result<Vec<String>>;
    async fn save_repo_path(&self, path: &str) -> Result<()>;
    async fn delete_repo_path(&self, path: &str) -> Result<()>;
    async fn get_setting_bool(&self, key: &str) -> Result<Option<bool>>;
    async fn set_setting_bool(&self, key: &str, value: bool) -> Result<()>;
    async fn get_setting_string(&self, key: &str) -> Result<Option<String>>;
    async fn set_setting_string(&self, key: &str, value: &str) -> Result<()>;
    async fn save_filter_preset(&self, name: &str, repo_paths: &[String], mode: &str)
        -> Result<()>;
    async fn delete_filter_preset(&self, name: &str) -> Result<()>;
    async fn list_filter_presets(&self) -> Result<Vec<(String, Vec<String>, String)>>;
    async fn get_tips_state(&self) -> Result<(u32, crate::models::TipsShowMode)>;
    async fn save_tips_state(
        &self,
        seen_up_to: u32,
        show_mode: crate::models::TipsShowMode,
    ) -> Result<()>;
    async fn get_verify_command(&self, path: &str) -> Result<Option<String>>;
    /// Set the verify command for a known repo path.
    ///
    /// If `command` is `Some(cmd)` and the path does not exist in `repo_paths`, a new
    /// row is inserted (with `last_used = now()`), equivalent to calling `save_repo_path`
    /// first. If `command` is `None`, the column is cleared to NULL; unknown paths are
    /// silently ignored (no row created).
    ///
    /// `command` must not contain a newline (`\n`) or carriage return (`\r`); returns
    /// an error if it does. Empty or whitespace-only commands are treated as `None`.
    async fn set_verify_command(&self, path: &str, command: Option<&str>) -> Result<()>;
}

// ---------------------------------------------------------------------------
// TaskAndEpicStore — composite for consumers that need tasks + epics only
// ---------------------------------------------------------------------------

pub trait TaskAndEpicStore: TaskCrud + EpicCrud {}

impl<T: TaskCrud + EpicCrud> TaskAndEpicStore for T {}

// ---------------------------------------------------------------------------
// ProjectCrud — CRUD for the projects table
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
pub trait ProjectCrud: Send + Sync {
    async fn create_project(&self, name: &str, sort_order: i64) -> Result<Project>;
    async fn list_projects(&self) -> Result<Vec<Project>>;
    async fn get_default_project(&self) -> Result<Project>;
    async fn rename_project(&self, id: ProjectId, name: &str) -> Result<()>;
    /// Move all tasks and epics from `from` to `to`, then delete the `from` project.
    /// The entire operation runs in a single transaction.
    async fn delete_project_and_move_items(
        &self,
        id: ProjectId,
        default_id: ProjectId,
    ) -> Result<()>;
    async fn reorder_project(&self, id: ProjectId, new_sort_order: i64) -> Result<()>;
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
// CreateLearningRow — DB-layer params for inserting a learning row
// ---------------------------------------------------------------------------

pub struct CreateLearningRow<'a> {
    pub kind: LearningKind,
    pub summary: &'a str,
    pub detail: Option<&'a str>,
    pub scope: LearningScope,
    pub scope_ref: Option<&'a str>,
    pub tags: &'a [String],
    pub source_task_id: Option<TaskId>,
}

// ---------------------------------------------------------------------------
// LearningStore — narrow sub-trait for the learnings table
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
pub trait LearningStore: Send + Sync {
    async fn create_learning(&self, row: CreateLearningRow<'_>) -> Result<LearningId>;

    async fn get_learning(&self, id: LearningId) -> Result<Option<Learning>>;

    async fn list_learnings(&self, filter: LearningFilter) -> Result<Vec<Learning>>;

    async fn patch_learning(&self, id: LearningId, patch: &LearningPatch<'_>) -> Result<()>;

    async fn delete_learning(&self, id: LearningId) -> Result<()>;

    /// Atomically increments `upvote_count` and updates `last_upvoted_at` and `updated_at`.
    async fn upvote_learning(&self, id: LearningId) -> Result<()>;

    /// Returns approved learnings for the given task context, unioning user + project + repo + epic
    /// scopes. Task-scoped learnings are excluded (they surface via explicit query only).
    /// Ordered by scope priority (procedural > epic > repo > project > user), then upvote_count DESC.
    async fn list_learnings_for_dispatch(
        &self,
        project_id: Option<ProjectId>,
        repo_path: &str,
        epic_id: Option<EpicId>,
    ) -> Result<Vec<Learning>>;
}

// ---------------------------------------------------------------------------
// LearningRetrievalStore — narrow sub-trait for retrievals + verdicts
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
pub trait LearningRetrievalStore: Send + Sync {
    /// Insert a row into `learning_retrievals` recording that `learning_id` was
    /// surfaced to `task_id` via the given source.
    async fn record_retrieval(
        &self,
        task_id: TaskId,
        learning_id: LearningId,
        source: RetrievalSource,
    ) -> Result<()>;

    /// Return all retrievals recorded for `task_id`, ordered by id ascending.
    async fn list_retrievals_for_task(&self, task_id: TaskId) -> Result<Vec<LearningRetrieval>>;

    /// Apply a batch of verdicts atomically. Each verdict is recorded in
    /// `learning_verdicts`; in addition, `Helped` bumps the learning's
    /// `upvote_count` and `Wrong` flips an approved learning to
    /// `needs_review`. `Unused` only records a row.
    async fn apply_verdicts_tx(
        &self,
        task_id: TaskId,
        verdicts: &[(LearningId, LearningVerdict)],
    ) -> Result<()>;

    /// Count of learnings currently in the `needs_review` state.
    async fn count_learnings_needs_review(&self) -> Result<i64>;
}

// ---------------------------------------------------------------------------
// TaskStore — supertrait combining all sub-traits
// ---------------------------------------------------------------------------

pub trait TaskStore:
    TaskAndEpicStore + SettingsStore + ProjectCrud + LearningStore + LearningRetrievalStore
{
}

impl<
        T: TaskAndEpicStore + SettingsStore + ProjectCrud + LearningStore + LearningRetrievalStore,
    > TaskStore for T
{
}

// ---------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------

/// Newtype that adapts `anyhow::Error` (which itself does not implement
/// [`std::error::Error`]) into the boxed-error slot that
/// [`tokio_rusqlite::Error::Other`] expects. Round-trips through `Box<dyn
/// StdError>` so [`Database::db_call`] can recover the original error.
#[derive(Debug)]
struct AnyhowErr(anyhow::Error);

impl std::fmt::Display for AnyhowErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for AnyhowErr {}

/// Async-only storage backing for the [`Database`].
///
/// Wraps a single [`tokio_rusqlite::Connection`] — a dedicated worker thread
/// owning a `rusqlite::Connection` that all async store impls dispatch to via
/// [`Database::db_call`]. There is no sync connection or `Mutex` anymore;
/// schema init and migrations also run on the worker thread via the same
/// closure mechanism.
pub struct Database {
    conn: tokio_rusqlite::Connection,
}

impl Database {
    pub async fn open(path: &Path) -> Result<Self> {
        // Ensure the parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create db directory: {}", parent.display()))?;
        }

        let conn = tokio_rusqlite::Connection::open(path)
            .await
            .with_context(|| format!("Failed to open database at {}", path.display()))?;

        Self::init_schema(&conn).await?;

        Ok(Database { conn })
    }

    pub async fn open_in_memory() -> Result<Self> {
        let conn = tokio_rusqlite::Connection::open_in_memory()
            .await
            .context("Failed to open in-memory database")?;
        Self::init_schema(&conn).await?;
        Ok(Database { conn })
    }

    /// Run a synchronous closure against the underlying SQLite database from an
    /// async context, returning its result without blocking the Tokio worker.
    ///
    /// The closure receives a `&mut rusqlite::Connection` (the dedicated thread
    /// owned by [`tokio_rusqlite::Connection`]). It must be `Send + 'static`,
    /// so any borrowed parameters need to be cloned to owned values before
    /// being moved in.
    ///
    /// Errors returned from the closure are wrapped in
    /// [`tokio_rusqlite::Error::Other`] and surfaced as `anyhow::Error`.
    pub async fn db_call<R, F>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&mut Connection) -> Result<R> + Send + 'static,
        R: Send + 'static,
    {
        match self
            .conn
            .call(move |c| f(c).map_err(|e| tokio_rusqlite::Error::Other(Box::new(AnyhowErr(e)))))
            .await
        {
            Ok(value) => Ok(value),
            Err(tokio_rusqlite::Error::Other(other)) => match other.downcast::<AnyhowErr>() {
                Ok(boxed) => Err(boxed.0),
                Err(other) => Err(anyhow::anyhow!(other.to_string())),
            },
            Err(e) => Err(anyhow::Error::from(e)),
        }
    }

    async fn init_schema(conn: &tokio_rusqlite::Connection) -> Result<()> {
        conn.call(|c| {
            init_schema_sync(c).map_err(|e| tokio_rusqlite::Error::Other(Box::new(AnyhowErr(e))))
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Other(other) => match other.downcast::<AnyhowErr>() {
                Ok(boxed) => boxed.0,
                Err(other) => anyhow::anyhow!(other.to_string()),
            },
            other => anyhow::Error::from(other),
        })
    }
}

fn init_schema_sync(conn: &Connection) -> Result<()> {
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
            id             INTEGER PRIMARY KEY,
            path           TEXT NOT NULL UNIQUE,
            last_used      TEXT NOT NULL DEFAULT (datetime('now')),
            verify_command TEXT
        );",
    )
    .context("Failed to create schema")?;

    // Versioned migrations using PRAGMA user_version
    let current_version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

    for &(version, migrate_fn) in migrations::MIGRATIONS {
        if current_version < version {
            migrate_fn(conn)?;
            conn.pragma_update(None, "user_version", version)
                .with_context(|| format!("Failed to update schema version to {version}"))?;
        }
    }

    Ok(())
}
