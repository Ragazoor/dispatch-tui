//! Turning the shared store's rows into the board's own types.
//!
//! Spec: `docs/specs/sync.allium`'s `BoardReadsFromTheSubscription`, and
//! `spacetime/module/README.md`'s "Why almost nothing here is `Option`" for the
//! sentinels this file undoes.
//!
//! # The second decoder problem
//!
//! `src/db/queries/mod.rs::row_to_task` already turns a stored row into a
//! [`models::Task`]. This is a second one, over a different row type, and two
//! decoders for one domain type is a thing that drifts. Three things hold them
//! together:
//!
//! - **The parsers are shared, not re-implemented.** Status, sub-status, tag,
//!   wrap-up mode, tmux window, feed role and origin all go through the same
//!   `models` constructors SQLite's decoder uses, and timestamps through the
//!   same [`crate::db::parse_datetime`]. What is written here is the field
//!   mapping and the sentinel undo — nothing that parses.
//! - **The soft-fail policy is copied deliberately, field by field.** A
//!   malformed `tmux_window` is dropped with a warning and an unknown status is
//!   refused, exactly as in SQLite's decoder, because a row that renders one
//!   way from disk and another way from the store is worse than either.
//! - **`tests::decode` drives a real SQLite board through the dump and back.**
//!   That is the strongest of the three: the assertion is not that this decoder
//!   is right in the abstract, but that it reproduces the very row SQLite read.
//!
//! # Why absence is refused rather than defaulted
//!
//! An unknown enum fails the row. The caller drops it, so a task written by a
//! newer binary is missing from the board rather than sitting on it with a
//! plausible wrong status — the same bargain `collect_decodable` already makes
//! for SQLite's bulk reads.

use chrono::{DateTime, Utc};

use crate::models::{
    Epic, EpicId, EpicOrigin, FeedRole, Learning, LearningId, LearningKind, LearningRetrieval,
    LearningScope, LearningStatus, RetrievalSource, SubStatus, Task, TaskId, TaskStatus, TaskTag,
    TaskUrl, TmuxWindow, UrlType, WrapUpMode,
};
use crate::spacetime::bindings;

use super::rows::{
    HostRow, PollOwnerRow, RepoBaseBranchRow, RepoPathRow, RetiredFeedItemRow, UsageEventRow,
};

/// Why one row could not become a domain value.
///
/// One type with a message rather than a taxonomy: every caller does the same
/// thing with it — drop the row and log — and the message is what a person
/// needs to work out which row and which column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError {
    message: String,
}

impl DecodeError {
    fn new(table: &str, id: i64, detail: impl std::fmt::Display) -> Self {
        Self {
            message: format!("{table} row {id}: {detail}"),
        }
    }
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DecodeError {}

type Decoded<T> = Result<T, DecodeError>;

// ---------------------------------------------------------------------------
// Sentinels
// ---------------------------------------------------------------------------

/// `""` is absence. See `SharedTable::sentinel_columns`.
pub(super) fn text(raw: &str) -> Option<&str> {
    (!raw.is_empty()).then_some(raw)
}

/// `0` is absence, for a column holding a foreign key or an interval.
///
/// Safe because every id this is applied to is generated and SpacetimeDB's
/// `auto_inc` starts at 1 — zero is the value that ASKS for an id, so no real
/// row ever holds it.
fn id(raw: i64) -> Option<i64> {
    (raw != 0).then_some(raw)
}

/// An optional timestamp column, sentinel-encoded as `""`.
fn timestamp(table: &str, row_id: i64, column: &str, raw: &str) -> Decoded<Option<DateTime<Utc>>> {
    match text(raw) {
        None => Ok(None),
        Some(raw) => crate::db::parse_datetime(raw)
            .map(Some)
            .map_err(|e| DecodeError::new(table, row_id, format!("{column}: {e}"))),
    }
}

/// A required timestamp column.
fn required_timestamp(table: &str, row_id: i64, column: &str, raw: &str) -> Decoded<DateTime<Utc>> {
    crate::db::parse_datetime(raw)
        .map_err(|e| DecodeError::new(table, row_id, format!("{column}: {e}")))
}

// ---------------------------------------------------------------------------
// Tasks
// ---------------------------------------------------------------------------

/// The store's `tasks` row as the board's [`Task`].
pub fn task(row: &bindings::Task) -> Decoded<Task> {
    const T: &str = "tasks";
    let refuse = |detail: String| DecodeError::new(T, row.id, detail);

    let status = TaskStatus::parse(&row.status)
        .ok_or_else(|| refuse(format!("unknown status {:?}", row.status)))?;
    let sub_status = SubStatus::parse(&row.sub_status)
        .ok_or_else(|| refuse(format!("unknown sub_status {:?}", row.sub_status)))?;
    let tag = match text(&row.tag) {
        None => None,
        Some(raw) => {
            Some(TaskTag::parse(raw).ok_or_else(|| refuse(format!("unknown tag {raw:?}")))?)
        }
    };
    let wrap_up_mode = match text(&row.wrap_up_mode) {
        None => None,
        Some(raw) => Some(
            WrapUpMode::parse(raw)
                .ok_or_else(|| refuse(format!("unknown wrap_up_mode {raw:?}")))?,
        ),
    };

    Ok(Task {
        id: TaskId(row.id),
        title: row.title.clone(),
        description: row.description.clone(),
        repo_path: row.repo_path.clone(),
        status,
        worktree: text(&row.worktree).map(str::to_owned),
        tmux_window: tmux_window(&row.tmux_window),
        host: text(&row.host).map(str::to_owned),
        plan_path: text(&row.plan_path).map(str::to_owned),
        epic_id: id(row.epic_id).map(EpicId),
        sub_status,
        url: task_url(row).map_err(refuse)?,
        tag,
        // NOT sentinel-encoded, and the one column in this table that is not.
        // `sort_order` is a real integer whose zero is meaningful — it is the
        // top of a column — so it stayed an `Option` in the module and nothing
        // subscribes by it. See the Phase 4 note in the migration plan.
        sort_order: row.sort_order,
        base_branch: row.base_branch.clone(),
        external_id: text(&row.external_id).map(str::to_owned),
        labels: labels(T, row.id, &row.labels)?,
        created_at: required_timestamp(T, row.id, "created_at", &row.created_at)?,
        updated_at: required_timestamp(T, row.id, "updated_at", &row.updated_at)?,
        last_pre_tool_use_at: timestamp(
            T,
            row.id,
            "last_pre_tool_use_at",
            &row.last_pre_tool_use_at,
        )?,
        last_notification_at: timestamp(
            T,
            row.id,
            "last_notification_at",
            &row.last_notification_at,
        )?,
        last_peer_message_sent_at: timestamp(
            T,
            row.id,
            "last_peer_message_sent_at",
            &row.last_peer_message_sent_at,
        )?,
        last_peer_message_received_at: timestamp(
            T,
            row.id,
            "last_peer_message_received_at",
            &row.last_peer_message_received_at,
        )?,
        wrap_up_mode,
        auto_run_plan: row.auto_run_plan,
        phoenix: row.phoenix,
        live_subagents: row.live_subagents,
        stop_pending: row.stop_pending,
        completed_at: timestamp(T, row.id, "completed_at", &row.completed_at)?,
    })
}

/// `url` and `url_type` are one value or neither.
///
/// A url without a type (or the reverse) is a row the application cannot
/// produce, so it fails rather than being coerced to `None` — the same call
/// SQLite's `read_task_url` makes, for the same reason.
fn task_url(row: &bindings::Task) -> Result<Option<TaskUrl>, String> {
    match (text(&row.url), text(&row.url_type)) {
        (None, None) => Ok(None),
        (Some(url), Some(raw)) => {
            let url_type =
                UrlType::parse(raw).ok_or_else(|| format!("unknown url_type {raw:?}"))?;
            Ok(Some(TaskUrl::new(url.to_owned(), url_type)))
        }
        (url, url_type) => Err(format!("inconsistent url={url:?} url_type={url_type:?}")),
    }
}

/// A malformed window name is dropped, not refused.
///
/// The card is still worth drawing without it; refusing would take the whole
/// task off the board over a cosmetic field. SQLite's `read_tmux_window` makes
/// the same call.
fn tmux_window(raw: &str) -> Option<TmuxWindow> {
    let raw = text(raw)?;
    match TmuxWindow::from_owned(raw.to_owned()) {
        Ok(window) => Some(window),
        Err(raw) => {
            // Counted, not only logged. `db::decode_fallback_count` is the
            // process-wide gauge for "this board is quietly dropping data", and
            // a soft-fail that skipped it would leave that number reading zero
            // on the board's primary read path.
            let count = crate::db::bump_decode_fallback();
            tracing::warn!(
                count,
                raw,
                "ignoring malformed tmux_window from the shared store"
            );
            None
        }
    }
}

/// Labels travel as the same JSON array SQLite stores, so one encoding serves
/// both stores and a dump moves between them untouched.
fn labels(table: &str, row_id: i64, raw: &str) -> Decoded<Vec<String>> {
    let Some(raw) = text(raw) else {
        return Ok(Vec::new());
    };
    serde_json::from_str::<Vec<String>>(raw)
        .map_err(|e| DecodeError::new(table, row_id, format!("labels: invalid JSON: {e}")))
}

// ---------------------------------------------------------------------------
// Epics
// ---------------------------------------------------------------------------

/// The store's `epics` row as the board's [`Epic`].
pub fn epic(row: &bindings::Epic) -> Decoded<Epic> {
    const T: &str = "epics";

    let status = TaskStatus::parse(&row.status)
        .ok_or_else(|| DecodeError::new(T, row.id, format!("unknown status {:?}", row.status)))?;

    Ok(Epic {
        id: EpicId(row.id),
        title: row.title.clone(),
        description: row.description.clone(),
        status,
        plan_path: text(&row.plan_path).map(str::to_owned),
        sort_order: row.sort_order,
        auto_dispatch: row.auto_dispatch,
        parent_epic_id: id(row.parent_epic_id).map(EpicId),
        feed_command: text(&row.feed_command).map(str::to_owned),
        feed_interval_secs: id(row.feed_interval_secs),
        group_by_repo: row.group_by_repo,
        feed_append_only: row.feed_append_only,
        // Soft-failed to a default rather than refused, matching SQLite's
        // `parse_feed_role`/`parse_epic_origin`: a role written by a newer
        // binary must not take the epic and every task under it off the board.
        feed_role: FeedRole::parse(&row.feed_role).unwrap_or_else(|| {
            let count = crate::db::bump_decode_fallback();
            tracing::warn!(count, value = %row.feed_role, "unknown feed_role from the shared store; defaulting to none");
            FeedRole::None
        }),
        origin: EpicOrigin::parse(&row.origin).unwrap_or_else(|| {
            let count = crate::db::bump_decode_fallback();
            tracing::warn!(count, value = %row.origin, "unknown epic origin from the shared store; defaulting to manual");
            EpicOrigin::Manual
        }),
        created_at: required_timestamp(T, row.id, "created_at", &row.created_at)?,
        updated_at: required_timestamp(T, row.id, "updated_at", &row.updated_at)?,
        completed_at: timestamp(T, row.id, "completed_at", &row.completed_at)?,
    })
}

// ---------------------------------------------------------------------------
// The tables whose rows have no domain type of their own
// ---------------------------------------------------------------------------
//
// Decoded here rather than where they are stored, so that undoing a sentinel
// happens in ONE file. `SharedTable::sentinel_columns` is the declaration; this
// module is the only consumer of it on the read path, and a column moving off
// `""` should mean one place to change.
//
// None of the three can fail: they carry no enum, no timestamp and no foreign
// key, so there is nothing to refuse.

/// The store's `repo_paths` row.
pub fn repo_path(row: &bindings::RepoPath) -> RepoPathRow {
    RepoPathRow {
        id: row.id,
        path: row.path.clone(),
        last_used: row.last_used.clone(),
        verify_command: text(&row.verify_command).map(str::to_owned),
    }
}

/// The store's `repo_base_branches` row.
pub fn repo_base_branch(row: &bindings::RepoBaseBranch) -> RepoBaseBranchRow {
    RepoBaseBranchRow {
        id: row.id,
        repo_path: row.repo_path.clone(),
        branch: row.branch.clone(),
        last_used: row.last_used.clone(),
    }
}

/// The store's `hosts` row: one machine on this board.
pub fn host(row: &bindings::Host) -> HostRow {
    HostRow {
        id: row.id.clone(),
        label: text(&row.label).map(str::to_owned),
        owner: text(&row.owner).map(str::to_owned),
    }
}

/// The store's `poll_owners` row: which host may run recurring background
/// polling for a task or an epic (`core.allium: PollOwner`).
pub fn poll_owner(row: &bindings::PollOwner) -> PollOwnerRow {
    PollOwnerRow {
        id: row.id,
        scope: row.scope.clone(),
        scope_id: row.scope_id,
        host: row.host.clone(),
    }
}

// ---------------------------------------------------------------------------
// Learnings and retrievals (Phase 10, task #4914)
// ---------------------------------------------------------------------------

/// The store's `learnings` row as the board's [`Learning`].
///
/// No sentinel-undo here, unlike every table above: `detail`, `scope_ref`,
/// `source_task_id` and `last_upvoted_at` are genuine `Option`s on the module
/// side too, because nothing ever subscribes by filtering on one of them —
/// see the module's own doc comment on `Learning`.
pub fn learning(row: &bindings::Learning) -> Decoded<Learning> {
    const T: &str = "learnings";

    let kind = LearningKind::parse(&row.kind)
        .ok_or_else(|| DecodeError::new(T, row.id, format!("unknown kind {:?}", row.kind)))?;
    let scope = LearningScope::parse(&row.scope)
        .ok_or_else(|| DecodeError::new(T, row.id, format!("unknown scope {:?}", row.scope)))?;
    let status = LearningStatus::parse(&row.status).map_err(|e| DecodeError::new(T, row.id, e))?;
    let tags: Vec<String> = serde_json::from_str(&row.tags)
        .map_err(|e| DecodeError::new(T, row.id, format!("malformed tags {:?}: {e}", row.tags)))?;

    Ok(Learning {
        id: LearningId(row.id),
        kind,
        summary: row.summary.clone(),
        detail: row.detail.clone(),
        scope,
        scope_ref: row.scope_ref.clone(),
        tags,
        status,
        source_task_id: row.source_task_id.map(TaskId),
        upvote_count: row.upvote_count,
        last_upvoted_at: row
            .last_upvoted_at
            .as_deref()
            .map(|s| required_timestamp(T, row.id, "last_upvoted_at", s))
            .transpose()?,
        created_at: required_timestamp(T, row.id, "created_at", &row.created_at)?,
        updated_at: required_timestamp(T, row.id, "updated_at", &row.updated_at)?,
    })
}

/// The store's `learning_retrievals` row as the board's [`LearningRetrieval`].
pub fn learning_retrieval(row: &bindings::LearningRetrieval) -> Decoded<LearningRetrieval> {
    const T: &str = "learning_retrievals";

    let source = RetrievalSource::parse(&row.source).map_err(|e| DecodeError::new(T, row.id, e))?;

    Ok(LearningRetrieval {
        id: row.id,
        task_id: TaskId(row.task_id),
        learning_id: LearningId(row.learning_id),
        source,
        retrieved_at: required_timestamp(T, row.id, "retrieved_at", &row.retrieved_at)?,
    })
}

// ---------------------------------------------------------------------------
// Usage events (Phase 11, task #4915)
// ---------------------------------------------------------------------------

/// The store's `usage_events` row.
///
/// No enum validation on `category`/`actor`: `UsageStore::query_usage`
/// returns them as bare strings (`models::UsageSummary`), the same shape the
/// SQL path already answers with — parsing into `UsageCategory`/`UsageActor`
/// is a concern of callers of `query_usage`, not of this decode step.
pub fn usage_event(row: &bindings::UsageEvent) -> Decoded<UsageEventRow> {
    const T: &str = "usage_events";

    Ok(UsageEventRow {
        id: row.id,
        recorded_at: required_timestamp(T, row.id, "recorded_at", &row.recorded_at)?,
        category: row.category.clone(),
        action: row.action.clone(),
        detail: row.detail.clone(),
        actor: row.actor.clone(),
    })
}

// ---------------------------------------------------------------------------
// Retired feed items (task #4971)
// ---------------------------------------------------------------------------

/// The store's `retired_feed_items` row. Infallible: every column is a plain
/// required scalar, and `retired_at` is audit only (`core.allium:
/// RetiredFeedItem`) — nothing decides anything from it, so it is kept as the
/// raw string rather than parsed into a timestamp.
pub fn retired_feed_item(row: &bindings::RetiredFeedItem) -> RetiredFeedItemRow {
    RetiredFeedItemRow {
        id: row.id,
        feed_epic_id: EpicId(row.feed_epic_id),
        external_id: row.external_id.clone(),
        retired_at: row.retired_at.clone(),
    }
}
