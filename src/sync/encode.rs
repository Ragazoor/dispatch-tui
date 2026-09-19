//! Turning the board's own types into the shared store's rows.
//!
//! Spec: `docs/specs/sync.allium`'s `BoardWritesThroughTheStore`.
//!
//! The mirror of [`super::decode`], and the reason the two are separate files
//! rather than one is that they are not inverses of each other. Decoding turns
//! a whole row into a whole [`crate::models::Task`]. Encoding has two jobs:
//! building a whole row for a CREATE, and turning a PATCH — which says nothing
//! at all about most fields — into the module's patch, where "say nothing" has
//! to survive the trip.
//!
//! # The two absences, and why they do not collapse
//!
//! `db::TaskPatch` is a double `Option`: `None` means "do not touch this
//! field", `Some(None)` means "set it to absent", `Some(Some(v))` means "set it
//! to v". The module's patch is a single `Option`, because absence in a COLUMN
//! is a sentinel there (`""`, `0`) rather than a null — see "Why almost nothing
//! here is `Option`" in `spacetime/module/README.md`.
//!
//! So the mapping is:
//!
//! | `db::TaskPatch` | module `TaskPatch` |
//! |---|---|
//! | `None` | `None` — untouched |
//! | `Some(None)` | `Some(<sentinel>)` — cleared |
//! | `Some(Some(v))` | `Some(v)` — set |
//!
//! The middle row is the one that is easy to get wrong, and getting it wrong is
//! invisible: `None` and `Some(None)` both LOOK like absence, and collapsing
//! them turns "clear this task's worktree" into "leave this task's worktree
//! alone" — a task the board thinks was released, still holding a checkout.
//!
//! `sort_order` is the exception, on both sides. It is genuinely nullable in
//! the module too (zero is a real sort order and null means "fall back to the
//! id"), so it stays doubly optional all the way through.

use crate::db::{CreateTaskRequest, TaskPatch};
use crate::models::TaskStatus;
use crate::spacetime::bindings;

/// The module's spelling of an empty label list. Not `""`: the column holds
/// JSON, and SQLite's decoder reads `[]` for a task with no labels.
const NO_LABELS: &str = "[]";

/// Map a patch field that may be cleared.
///
/// `clear` supplies the module's sentinel for the column, so each call site
/// names it rather than this function guessing one — `""` and `0` are both
/// sentinels and the compiler is the only thing that can tell which a column
/// wants.
fn nullable<T, U>(field: Option<Option<T>>, set: impl Fn(T) -> U, clear: U) -> Option<U> {
    match field {
        None => None,
        Some(None) => Some(clear),
        Some(Some(value)) => Some(set(value)),
    }
}

/// The timestamp format both stores write.
///
/// The same string `db::queries::format_datetime_millis` produces. Spelled out
/// rather than imported because that one is `pub(super)` to `db` and widening
/// it would make a SQLite formatting detail part of the crate's surface — where
/// the thing that actually has to agree is the FORMAT, which this comment and
/// the module's `SQLITE_TIMESTAMP` both name.
pub(super) fn stamp(at: chrono::DateTime<chrono::Utc>) -> String {
    at.format("%Y-%m-%d %H:%M:%S%.3f").to_string()
}

/// Build the row a `create_task` reducer inserts.
///
/// `id` is zero, which is how `#[auto_inc]` is asked to generate one. Every
/// field the request does not carry gets the module's absent sentinel rather
/// than a plausible default, because a create is not a patch: a field left out
/// here is a field the caller said nothing about, and the row has to say
/// "nothing" rather than "the empty string happens to mean this".
pub fn create_task_row(req: &CreateTaskRequest<'_>, owner: &str, now: &str) -> bindings::Task {
    bindings::Task {
        id: 0,
        title: req.title.to_string(),
        description: req.description.to_string(),
        repo_path: req.repo_path.to_string(),
        status: req.status.as_str().to_string(),
        worktree: String::new(),
        tmux_window: String::new(),
        plan_path: req.plan.unwrap_or_default().to_string(),
        epic_id: req.epic_id.map(|e| e.0).unwrap_or(0),
        sub_status: crate::models::SubStatus::default_for(req.status)
            .as_str()
            .to_string(),
        tag: req.tag.map(|t| t.as_str().to_string()).unwrap_or_default(),
        sort_order: req.sort_order,
        created_at: now.to_string(),
        updated_at: now.to_string(),
        base_branch: req.base_branch.to_string(),
        external_id: String::new(),
        labels: NO_LABELS.to_string(),
        last_pre_tool_use_at: String::new(),
        last_notification_at: String::new(),
        wrap_up_mode: req
            .wrap_up_mode
            .map(|m| m.as_str().to_string())
            .unwrap_or_default(),
        url: String::new(),
        url_type: String::new(),
        pr_learnings_gate_shown_at: String::new(),
        auto_run_plan: req.auto_run_plan,
        live_subagents: 0,
        stop_pending: false,
        stop_pending_at: String::new(),
        live_shells: 0,
        oldest_live_shell_started_at: String::new(),
        last_peer_message_sent_at: String::new(),
        last_peer_message_received_at: String::new(),
        phoenix: req.phoenix,
        host: String::new(),
        // `core.allium: OwnerTracksUserBoardTask`, enforced by the module's
        // `write_task`: a task with no epic names whose board it sits on, and a
        // task with one must not. Supplied here rather than derived in the
        // module because the module cannot know who is calling.
        owner: if req.epic_id.is_none() {
            owner.to_string()
        } else {
            String::new()
        },
        completed_at: String::new(),
    }
}

/// Translate a patch into the module's.
///
/// Every field the board may patch appears here. A field absent from
/// `db::TaskPatch` — `live_subagents`, `live_shells` and the rest of the
/// denormalised counters — is absent here too: those have dedicated writers on
/// purpose, so no handler can desync a count, and giving them a patch route
/// would undo that.
pub fn task_patch(patch: &TaskPatch<'_>) -> bindings::TaskPatch {
    bindings::TaskPatch {
        title: patch.title.map(str::to_string),
        description: patch.description.map(str::to_string),
        repo_path: patch.repo_path.map(str::to_string),
        status: patch.status.map(|s| s.as_str().to_string()),
        worktree: nullable(patch.worktree, str::to_string, String::new()),
        tmux_window: nullable(patch.tmux_window, |w| w.to_string(), String::new()),
        plan_path: nullable(patch.plan_path, str::to_string, String::new()),
        // Not patchable through `db::TaskPatch` — `set_task_epic_id` owns it,
        // because moving a task between epics has to recalculate both.
        epic_id: None,
        sub_status: patch.sub_status.map(|s| s.as_str().to_string()),
        tag: nullable(patch.tag, |t| t.as_str().to_string(), String::new()),
        sort_order: patch.sort_order,
        base_branch: patch.base_branch.map(str::to_string),
        external_id: nullable(patch.external_id, str::to_string, String::new()),
        labels: patch
            .labels
            .map(|l| serde_json::to_string(l).unwrap_or_else(|_| NO_LABELS.to_string())),
        last_pre_tool_use_at: nullable(patch.last_pre_tool_use_at, stamp, String::new()),
        last_notification_at: nullable(patch.last_notification_at, stamp, String::new()),
        wrap_up_mode: nullable(
            patch.wrap_up_mode,
            |m| m.as_str().to_string(),
            String::new(),
        ),
        // ONE FIELD ON THIS SIDE, TWO COLUMNS ON THE OTHER, so both move
        // together or neither does. A patch that set one and left the other
        // produces the `inconsistent url=.. url_type=..` row the decoder
        // refuses outright — a task that vanishes from the board with no other
        // symptom.
        url: nullable(patch.url, |u| u.url.clone(), String::new()),
        url_type: nullable(
            patch.url,
            |u| u.url_type.as_str().to_string(),
            String::new(),
        ),
        pr_learnings_gate_shown_at: None,
        auto_run_plan: patch.auto_run_plan,
        live_subagents: None,
        stop_pending: patch.stop_pending,
        stop_pending_at: None,
        live_shells: None,
        oldest_live_shell_started_at: None,
        last_peer_message_sent_at: nullable(patch.last_peer_message_sent_at, stamp, String::new()),
        last_peer_message_received_at: nullable(
            patch.last_peer_message_received_at,
            stamp,
            String::new(),
        ),
        phoenix: patch.phoenix,
        host: nullable(patch.host, str::to_string, String::new()),
        // Not patchable: `core.allium: OwnerTracksUserBoardTask` ties it to
        // `epic_id`, and a patch that could move one without the other would be
        // a task on two boards or on none.
        owner: None,
        completed_at: nullable(patch.completed_at, stamp, String::new()),
    }
}

/// The status a create with no explicit one lands in.
///
/// Here rather than inlined so the create path and the tests name the same
/// thing.
pub const DEFAULT_CREATE_STATUS: TaskStatus = TaskStatus::Backlog;

// ---------------------------------------------------------------------------
// Epics
// ---------------------------------------------------------------------------

/// Build the row a `create_epic` reducer inserts.
///
/// Backlog, not auto-dispatching, and manual: `epics.allium: CreateEpic`. Those
/// three are the epic's birth state rather than defaults the caller forgot, and
/// spelling them here keeps them out of the module — the store should not have
/// an opinion about what a new epic looks like.
pub fn create_epic_row(
    title: &str,
    description: &str,
    parent_epic_id: Option<crate::models::EpicId>,
    now: &str,
) -> bindings::Epic {
    bindings::Epic {
        id: 0,
        title: title.to_string(),
        description: description.to_string(),
        status: TaskStatus::Backlog.as_str().to_string(),
        plan_path: String::new(),
        sort_order: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
        auto_dispatch: false,
        parent_epic_id: parent_epic_id.map(|e| e.0).unwrap_or(0),
        feed_command: String::new(),
        feed_interval_secs: 0,
        group_by_repo: false,
        feed_role: crate::models::FeedRole::None.as_str().to_string(),
        origin: crate::models::EpicOrigin::Manual.as_str().to_string(),
        feed_append_only: false,
        completed_at: String::new(),
    }
}

/// Translate an epic patch into the module's.
pub fn epic_patch(patch: &crate::db::EpicPatch<'_>) -> bindings::EpicPatch {
    bindings::EpicPatch {
        title: patch.title.map(str::to_string),
        description: patch.description.map(str::to_string),
        status: patch.status.map(|s| s.as_str().to_string()),
        plan_path: nullable(patch.plan_path, str::to_string, String::new()),
        sort_order: patch.sort_order,
        auto_dispatch: patch.auto_dispatch,
        // `0` is the absent parent, not epic zero — `#[auto_inc]` never hands
        // out an id of zero, which is what makes the sentinel unreachable as a
        // real value.
        parent_epic_id: nullable(patch.parent_epic_id, |e| e.0, 0),
        feed_command: nullable(patch.feed_command, str::to_string, String::new()),
        feed_interval_secs: nullable(patch.feed_interval_secs, |v| v, 0),
        group_by_repo: patch.group_by_repo,
        feed_role: patch.feed_role.map(|r| r.as_str().to_string()),
        origin: patch.origin.map(|o| o.as_str().to_string()),
        feed_append_only: patch.feed_append_only,
        completed_at: nullable(patch.completed_at, stamp, String::new()),
    }
}

// ---------------------------------------------------------------------------
// Todos
// ---------------------------------------------------------------------------

/// Build the row a `create_todo` reducer inserts.
///
/// `sort_order` is zero, which is a REAL sort order here rather than a
/// sentinel: `todos.sort_order` is a plain integer in both stores, unlike the
/// nullable one on tasks and epics.
pub fn create_todo_row(row: &crate::db::CreateTodoRow<'_>, now: &str) -> bindings::Todo {
    bindings::Todo {
        id: 0,
        title: row.title.to_string(),
        done: false,
        sort_order: 0,
        created_at: now.to_string(),
        task_id: row.task_id.unwrap_or(0),
        epic_id: row.epic_id.unwrap_or(0),
        parent_id: 0,
        // `""` for an install that has never connected. A todo written then is
        // invisible to every subscription until something fills it — see
        // `todo.allium`'s open question, which this does not answer.
        owner: row.owner.unwrap_or_default().to_string(),
    }
}

/// Translate a todo patch into the module's.
pub fn todo_patch(patch: &crate::db::TodoPatch<'_>) -> bindings::TodoPatch {
    bindings::TodoPatch {
        title: patch.title.map(str::to_string),
        done: patch.done,
        sort_order: patch.sort_order,
        task_id: nullable(patch.task_id, |v| v, 0),
        epic_id: nullable(patch.epic_id, |v| v, 0),
        parent_id: nullable(patch.parent_id, |v| v, 0),
        // Not patchable: a todo's owner is stamped once at creation
        // (`todo.allium: Todo.owner`) and moving one to somebody else's
        // checklist is not an operation this board has.
        owner: None,
    }
}
