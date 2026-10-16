//! Plain-data inputs to the store's writes and queries: the selective-update
//! patches (`TaskPatch`, `EpicPatch`, `LearningPatch`), the create rows and
//! the list filters.
//!
//! They live here rather than beside the `store` traits that take them because
//! `sync` encodes them for the shared store's reducers, and `sync` sits below
//! `store` (`tests/module_layering.rs`).

use crate::models::{
    EpicId, LearningKind, LearningScope, LearningStatus, SubStatus, TaskId, TaskStatus, TaskTag,
    WrapUpMode,
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
    ///
    /// `live_subagents` is deliberately **absent**: it is a count owned
    /// exclusively by the subagent writes
    /// ([`subagent_start`](crate::store::TaskCrud::subagent_start) and its
    /// siblings). Leaving it out of the patch surface makes "no handler can
    /// desync the count" a compile-time property rather than a convention.
    /// `stop_pending` is patch-driven and stays — but for *clearing* only: the
    /// sole writer that may set it true is
    /// [`try_record_stop`](crate::store::TaskCrud::try_record_stop)'s defer
    /// branch, which stamps `stop_pending_at` in the same statement. Setting it true through a
    /// patch would leave that timestamp stale or absent, and
    /// `record_user_prompt_submit` decides against it.
    pub struct TaskPatch<'a> {
        plain    status:       TaskStatus,
        nullable plan_path:    &'a str,
        plain    title:        &'a str,
        plain    description:  &'a str,
        plain    repo_path:    &'a str,
        nullable worktree:     &'a str,
        nullable tmux_window:  &'a crate::models::TmuxWindow,
        nullable host:         &'a str,
        plain    sub_status:   SubStatus,
        nullable url:          &'a crate::models::TaskUrl,
        nullable tag:          TaskTag,
        nullable sort_order:   i64,
        nullable completed_at: chrono::DateTime<chrono::Utc>,
        plain    base_branch:  &'a str,
        nullable external_id:  &'a str,
        plain    labels:       &'a [String],
        nullable last_pre_tool_use_at: chrono::DateTime<chrono::Utc>,
        nullable last_notification_at: chrono::DateTime<chrono::Utc>,
        nullable last_peer_message_sent_at: chrono::DateTime<chrono::Utc>,
        nullable last_peer_message_received_at: chrono::DateTime<chrono::Utc>,
        nullable wrap_up_mode: WrapUpMode,
        plain    auto_run_plan: bool,
        plain    phoenix:       bool,
        plain    stop_pending: bool,
        plain    alias:        &'a str,
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
    pub wrap_up_mode: Option<WrapUpMode>,
    pub auto_run_plan: bool,
    pub phoenix: bool,
}

// ---------------------------------------------------------------------------
// EpicPatch — builder for selective epic field updates
// ---------------------------------------------------------------------------

patch_struct! {
    /// Builder for selective epic field updates.
    pub struct EpicPatch<'a> {
        plain    title:              &'a str,
        plain    description:        &'a str,
        plain    status:             TaskStatus,
        nullable plan_path:          &'a str,
        nullable sort_order:         i64,
        nullable completed_at:       chrono::DateTime<chrono::Utc>,
        plain    auto_dispatch:      bool,
        plain    group_by_repo:      bool,
        plain    feed_append_only:   bool,
        plain    feed_role:          crate::models::FeedRole,
        plain    origin:             crate::models::EpicOrigin,
        nullable feed_command:       &'a str,
        nullable feed_interval_secs: i64,
        nullable parent_epic_id:     EpicId,
    }
}

// ---------------------------------------------------------------------------
// LearningPatch — builder for partial learning updates
// ---------------------------------------------------------------------------

patch_struct! {
    /// Builder for selective learning field updates.
    ///
    /// Deliberately narrow: `embedding` is the only field with a production
    /// writer (the startup backfill in `runtime::bootstrap`), and `status` is
    /// written by tests seeding archived/rejected rows. There is no field-editing
    /// path for a learning's content — `detail`/`kind`/`tags` setters were
    /// removed with the TUI overlay, which keeps a stale embedding
    /// unrepresentable (see `docs/specs/learnings.allium`: EmbeddingConsistency).
    pub struct LearningPatch<'a> {
        plain    status:    LearningStatus,
        plain    summary:   &'a str,
        plain    embedding: &'a [u8],
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
    pub embedding: Option<&'a [u8]>,
}

#[derive(Debug, Default)]
pub struct UsageQuery {
    pub category: Option<String>,
    pub actor: Option<String>,
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: Option<usize>,
}
