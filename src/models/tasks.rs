use chrono::{DateTime, SubsecRound, Utc};
use serde::{Deserialize, Serialize};

use super::{ColumnSection, EpicId, TmuxWindow, UrlType};
use crate::define_id_newtype;
use crate::define_str_enum;

define_id_newtype!(TaskId, task_id_tests);

// ---------------------------------------------------------------------------
// TaskStatus
// ---------------------------------------------------------------------------

// `Ord` is derived so a (TaskStatus, ColumnSection) pair can key an ordered
// set — the folded-section list needs a stable serialisation order. The
// ordering is the declaration order, which is left-to-right column order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    #[serde(alias = "ready")]
    Backlog,
    Running,
    Review,
    Done,
}

/// Status values that once existed and no longer do. A task or epic row
/// carrying one is DROPPED when rows are read from the shared store or imported
/// into the managed one: not mapped to done, not kept as a retired item. One
/// list, one predicate, shared by both paths. Spec: `sync.allium`'s
/// `RowsWithARemovedStatusAreDropped` (`config.removed_statuses`).
pub const REMOVED_STATUSES: &[&str] = &["archived"];

/// Whether `raw` is a status that was removed, as opposed to one never known.
pub fn is_removed_status(raw: &str) -> bool {
    REMOVED_STATUSES.contains(&raw)
}

impl TaskStatus {
    pub const ALL: &'static [TaskStatus] = &[
        TaskStatus::Backlog,
        TaskStatus::Running,
        TaskStatus::Review,
        TaskStatus::Done,
    ];

    /// Statuses settable through the `update_task` MCP tool. `Done` IS
    /// advertised, but only reachable through the dedicated close-only path
    /// (`MarkTaskDoneViaMcp`) — `UpdateTaskViaMcp`'s own `requires: status !=
    /// done` still refuses it on the generic multi-field rule. Kept as its
    /// own const (rather than a hand-written schema literal) so an MCP schema
    /// derived from it can't drift from this list.
    pub const MCP_UPDATABLE: &'static [TaskStatus] = &[
        TaskStatus::Backlog,
        TaskStatus::Running,
        TaskStatus::Review,
        TaskStatus::Done,
    ];

    /// The board columns that flattened mode never reaches. Kept as its own
    /// const, in the same polarity as `board-layout.allium`'s
    /// `FlattenedView.unflattened_statuses`, so the two cannot drift: a new
    /// column added to the enum flattens by default in both.
    ///
    /// Backlog is the one column read at the epic level — it is where work is
    /// planned and ordered — so it keeps its epic cards while Running, Review
    /// and Done give theirs up.
    ///
    /// Done was exempt too until task #4784: "what did we finish" is a question
    /// about tasks, and an exempt Done column answered it with an epic card
    /// whose only report was a count.
    pub const UNFLATTENED: &'static [TaskStatus] = &[TaskStatus::Backlog];

    pub const COLUMN_COUNT: usize = Self::ALL.len();

    /// Whether flattened mode leaves this column alone. See [`Self::UNFLATTENED`].
    pub fn is_unflattened(self) -> bool {
        Self::UNFLATTENED.contains(&self)
    }

    /// Advance to the next status (wraps at Done -> Done).
    pub fn next(self) -> Self {
        match self {
            TaskStatus::Backlog => TaskStatus::Running,
            TaskStatus::Running => TaskStatus::Review,
            TaskStatus::Review => TaskStatus::Done,
            TaskStatus::Done => TaskStatus::Done,
        }
    }

    /// Retreat to the previous status (wraps at Backlog -> Backlog).
    pub fn prev(self) -> Self {
        match self {
            TaskStatus::Backlog => TaskStatus::Backlog,
            TaskStatus::Running => TaskStatus::Backlog,
            TaskStatus::Review => TaskStatus::Running,
            TaskStatus::Done => TaskStatus::Review,
        }
    }

    /// Zero-based column index for kanban board layout.
    pub fn column_index(self) -> usize {
        match self {
            TaskStatus::Backlog => 0,
            TaskStatus::Running => 1,
            TaskStatus::Review => 2,
            TaskStatus::Done => 3,
        }
    }

    /// Construct from a column index; returns None if out of range.
    pub fn from_column_index(idx: usize) -> Option<Self> {
        match idx {
            0 => Some(TaskStatus::Backlog),
            1 => Some(TaskStatus::Running),
            2 => Some(TaskStatus::Review),
            3 => Some(TaskStatus::Done),
            _ => None,
        }
    }
}

define_str_enum!(TaskStatus, "status" {
    Backlog => "backlog" | "ready",
    Running => "running",
    Review => "review",
    Done => "done",
});

/// Decides what a status transition should do to `completed_at`.
///
/// `None` means "don't touch it"; `Some(now)` means "stamp this completion
/// time". Only a transition that *enters* Done stamps: a write that leaves
/// Done, or a `done -> done` write, returns `None`.
///
/// There is deliberately no clear. `completed_at` records when the task last
/// finished, not whether it is finished now, so moving a card back out of Done
/// leaves the field alone and re-entering Done overwrites it. That is the whole
/// difference from the `sort_order` completion rank this replaced, which had to
/// be cleared on the way out because it shared a field with manual ordering.
/// See `ConfirmDone` in `docs/specs/tasks.allium`.
///
/// The Done column orders on this field *descending* — no negation trick, no
/// sign to interpret (`board-layout.allium`, "Done Column Ordering").
///
/// The value is **truncated to whole milliseconds**, which is the precision the
/// storage column keeps (`stamp` in `src/sync/encode.rs`). Truncating here rather than
/// at the write is what keeps the value this returns equal to the one a later
/// read gives back: the runtime splices this result straight into the in-memory
/// board (`write_back_task_completed_at`), so an untruncated one would disagree
/// with the database until the next refresh. Milliseconds also shrink the
/// same-tick tie window for bulk actions (multi-select "confirm done", the
/// PR-poller detecting several merges in one 30s tick); a same-millisecond tie
/// is still possible and degrades gracefully to the existing id tie-break.
pub fn completed_at_for_status_transition(
    prior: TaskStatus,
    next: TaskStatus,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    if prior == TaskStatus::Done || next != TaskStatus::Done {
        return None;
    }
    Some(now.trunc_subsecs(3))
}

/// Fold one task into a running "newest completion", the key the Done column
/// orders by (`board-layout.allium`, "Done Column Ordering").
///
/// The column reads newest-first, so "newest" is the MAXIMUM. A task that is
/// not Done, or carries no `completed_at`, leaves `best` untouched. The single
/// owner of that rule. Two callers accumulate over different task sets and
/// neither restates it: `EpicPlacement::record` credits an epic's whole visible
/// subtree, and the flattened column builder groups a column's tasks by their
/// direct epic.
pub fn fold_newest_completion(best: Option<DateTime<Utc>>, task: &Task) -> Option<DateTime<Utc>> {
    if task.status != TaskStatus::Done {
        return best;
    }
    // `Option` orders `None` below `Some`, so this covers all three cases at
    // once: a missing `completed_at` loses to any real one, and two real ones
    // resolve to the later.
    best.max(task.completed_at)
}

/// Whether a status write from `prior` to `next` voids a deferred Stop
/// (`Task::stop_pending`).
///
/// The bit records "a Stop hook arrived while subagents were still live", and
/// it belongs to the turn the task was Running under. Any write that takes the
/// task out of Running ends that turn, so the bit is cleared in the same patch
/// — see `PendingStopOnlyWhileRunning` in `docs/specs/core.allium`. Leaving it
/// set would carry a Stop from the earlier turn into the next one, where the
/// drain would apply it and flip the task straight back out of Running the
/// moment a human moves the card back in.
///
/// Arriving in Running is deliberately not a clear point: only `HookStop` sets
/// the bit and it requires Running, so there is nothing to clear on the way in,
/// and the dispatch claim already clears it explicitly.
pub fn clears_pending_stop(prior: TaskStatus, next: TaskStatus) -> bool {
    prior == TaskStatus::Running && next != TaskStatus::Running
}

// ---------------------------------------------------------------------------
// SubStatus
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubStatus {
    None,
    Active,
    NeedsInput,
    Stale,
    Crashed,
    Conflict,
    AwaitingReview,
    ChangesRequested,
    Approved,
    PrClosed,
    PrUnreachable,
}

impl SubStatus {
    /// Whether a running task in this sub-status is waiting on a human rather
    /// than making progress. The one owner of that predicate: `epic_substatus`
    /// rolls it up into `Blocked(N)` for the whole epic, and `EpicPlacement`
    /// rolls it up per column.
    pub fn is_blocked(self) -> bool {
        matches!(
            self,
            SubStatus::NeedsInput | SubStatus::Stale | SubStatus::Crashed | SubStatus::Conflict
        )
    }

    pub const ALL: &'static [SubStatus] = &[
        SubStatus::None,
        SubStatus::Active,
        SubStatus::NeedsInput,
        SubStatus::Stale,
        SubStatus::Crashed,
        SubStatus::Conflict,
        SubStatus::AwaitingReview,
        SubStatus::ChangesRequested,
        SubStatus::Approved,
        SubStatus::PrClosed,
        SubStatus::PrUnreachable,
    ];

    /// Sub-statuses advertised by the `update_task` MCP tool's schema.
    /// Excludes `pr_closed` and `pr_unreachable`: both are derived from GitHub
    /// PR polling (`PollPrStatus` / `PrPollGaveUp`), not values an agent
    /// should set by hand.
    /// Advertisement-only — the handler still accepts any of the three if a
    /// caller sends it anyway, same as any other
    /// `SubStatus` valid for the effective status (mcp-task-tools.allium:
    /// `UpdateTaskViaMcp`). Kept as its own const (rather than a hand-written
    /// schema literal) so the advertised set can't silently drop a variant
    /// it should include.
    pub const MCP_ADVERTISED: &'static [SubStatus] = &[
        SubStatus::None,
        SubStatus::Active,
        SubStatus::NeedsInput,
        SubStatus::Stale,
        SubStatus::Crashed,
        SubStatus::Conflict,
        SubStatus::AwaitingReview,
        SubStatus::ChangesRequested,
        SubStatus::Approved,
    ];

    /// Check whether this sub-status is valid for the given parent status.
    pub fn is_valid_for(&self, status: TaskStatus) -> bool {
        match status {
            TaskStatus::Backlog => matches!(self, SubStatus::None),
            TaskStatus::Running => matches!(
                self,
                SubStatus::Active
                    | SubStatus::NeedsInput
                    | SubStatus::Stale
                    | SubStatus::Crashed
                    | SubStatus::Conflict
            ),
            TaskStatus::Review => matches!(
                self,
                SubStatus::AwaitingReview
                    | SubStatus::ChangesRequested
                    | SubStatus::Approved
                    | SubStatus::Conflict
                    | SubStatus::PrClosed
                    | SubStatus::PrUnreachable
            ),
            TaskStatus::Done => matches!(self, SubStatus::None),
        }
    }

    /// Return the default sub-status for a given parent status.
    pub fn default_for(status: TaskStatus) -> Self {
        match status {
            TaskStatus::Backlog => SubStatus::None,
            TaskStatus::Running => SubStatus::Active,
            TaskStatus::Review => SubStatus::AwaitingReview,
            TaskStatus::Done => SubStatus::None,
        }
    }

    /// The section this sub-status renders under, or `None` for `none` — the
    /// sub-status of the two columns that have no sections at all.
    ///
    pub const fn column_section(self) -> Option<ColumnSection> {
        match self {
            SubStatus::None => None,
            SubStatus::Active => Some(ColumnSection::Active),
            SubStatus::NeedsInput => Some(ColumnSection::NeedsInput),
            SubStatus::Stale => Some(ColumnSection::Stale),
            SubStatus::Crashed => Some(ColumnSection::Crashed),
            SubStatus::Conflict => Some(ColumnSection::Conflict),
            SubStatus::AwaitingReview => Some(ColumnSection::AwaitingReview),
            SubStatus::ChangesRequested => Some(ColumnSection::ChangesRequested),
            SubStatus::Approved => Some(ColumnSection::Approved),
            SubStatus::PrClosed => Some(ColumnSection::PrClosed),
            SubStatus::PrUnreachable => Some(ColumnSection::PrUnreachable),
        }
    }

    /// Sort priority for column grouping (lower = more urgent = top of column).
    /// Read off the section table, which owns every slot.
    pub const fn column_priority(self) -> u8 {
        super::columns::section_sort_priority(self.column_section())
    }

    /// Label for section header lines within a column, or the empty string
    /// where this sub-status names no section.
    pub const fn header_label(self) -> &'static str {
        match self.column_section() {
            Some(section) => section.header_label(),
            None => "",
        }
    }
}

define_str_enum!(SubStatus, "sub-status" {
    None => "none",
    Active => "active",
    NeedsInput => "needs_input",
    Stale => "stale",
    Crashed => "crashed",
    Conflict => "conflict",
    AwaitingReview => "awaiting_review",
    ChangesRequested => "changes_requested",
    Approved => "approved",
    PrClosed => "pr_closed",
    PrUnreachable => "pr_unreachable",
});

// ---------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------

pub const DEFAULT_QUICK_TASK_TITLE: &str = "Quick task";
pub const DEFAULT_BASE_BRANCH: &str = "main";

#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub description: String,
    pub repo_path: String,
    pub status: TaskStatus,
    pub worktree: Option<String>,
    pub tmux_window: Option<TmuxWindow>,
    /// The opaque id of the machine holding this task's worktree; `None` means
    /// no machine holds one. Coupled to `worktree`, never independently null
    /// or set — see `core/Task`'s `HostTracksWorktree` invariant in
    /// `docs/specs/core.allium`. Stamped by the dispatch write that records
    /// the worktree (`src/service/tasks/dispatch.rs`) and cleared by the write
    /// that forgets it (`clear_worktree_pointer` in `src/runtime/tasks.rs`).
    pub host: Option<String>,
    pub plan_path: Option<String>,
    pub epic_id: Option<EpicId>,
    pub sub_status: SubStatus,
    pub url: Option<crate::models::TaskUrl>,
    pub tag: Option<TaskTag>,
    pub sort_order: Option<i64>,
    /// When this task last entered Done; `None` until it first does.
    ///
    /// The Done column's ordering key, read *descending* — see
    /// `completed_at_for_status_transition` and "Done Column Ordering" in
    /// `docs/specs/board-layout.allium`. Deliberately survives a move back out
    /// of Done: it records the last completion, not the current status.
    pub completed_at: Option<DateTime<Utc>>,
    pub base_branch: String,
    pub external_id: Option<String>,
    /// Free-form badges rendered on the kanban card alongside derived
    /// indicators. Order is preserved so feed scripts can control rendering
    /// order.
    pub labels: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_pre_tool_use_at: Option<DateTime<Utc>>,
    pub last_notification_at: Option<DateTime<Utc>>,
    /// Stamped by `dispatch hook <id> peer-message` (task #4098) when this
    /// task's agent is observed calling the native `SendMessage` tool. Drives
    /// the TUI's "sent" flash — see `docs/specs/agent-health.allium`'s
    /// `HookPeerMessageSent`.
    pub last_peer_message_sent_at: Option<DateTime<Utc>>,
    /// Stamped on the *resolved target* task's row by the same hook
    /// observation. Drives the TUI's "received" flash.
    pub last_peer_message_received_at: Option<DateTime<Utc>>,
    pub wrap_up_mode: Option<WrapUpMode>,
    pub auto_run_plan: bool,
    /// A recurring task with no schedule. When this task enters Done, a fresh
    /// Backlog copy of it is created and the flag MOVES to that copy — so it
    /// fires exactly once, and the flag surviving in Done means the copy did
    /// not land (`respawn_failed`). See `PhoenixRespawn` and the `== Phoenix ==`
    /// section in `docs/specs/tasks.allium`.
    ///
    /// Deliberately not a `TaskTag`: `tag` selects the dispatch prompt and
    /// marks review work, and a recurring *chore* is the ordinary case, so the
    /// two must be able to coexist.
    pub phoenix: bool,
    /// Number of subagents currently executing for this task. Denormalised
    /// `COUNT(*)` over `task_subagents`, rewritten by every mutation and
    /// every clear point. Read by `classify_agent_activity` (live subagents
    /// outrank staleness) and by the running card label.
    pub live_subagents: i64,
    /// A `Stop` hook arrived while subagents were still live, so the
    /// Running -> Review flip was deferred. The last `SubagentStop` to drain
    /// the count performs it. See `HookStop` in `docs/specs/agent-health.allium`.
    pub stop_pending: bool,
}

impl Task {
    /// The card's generic ordering key: its `sort_order`, or its id when that
    /// is null. The one place that spells the fallback out.
    ///
    /// Every column but Done uses it. Done has its own key, `completed_at`
    /// (see `completed_at_for_status_transition`), which is what freed
    /// `sort_order` to mean feed and manual ordering and nothing else.
    pub fn sort_key(&self) -> i64 {
        self.sort_order.unwrap_or(self.id.0)
    }

    /// Whether this task has a worktree but no tmux window (agent session ended).
    pub fn is_detached(&self) -> bool {
        self.worktree.is_some()
            && self.tmux_window.is_none()
            && matches!(self.status, TaskStatus::Running | TaskStatus::Review)
    }

    /// Whether this task looks live but has nothing behind it: Running/Review
    /// with neither a worktree nor a tmux window, so there is nothing to
    /// resume. The complement of [`Self::is_detached`], which requires a
    /// worktree — the two are mutually exclusive.
    ///
    /// Reachable by a manual forward move out of Backlog, by a crash between
    /// the dispatch claim and provisioning, and by a dispatch worker that dies
    /// without reporting. See `UnprovisionedIndicator` in
    /// `docs/specs/dispatch.allium`.
    pub fn is_unprovisioned(&self) -> bool {
        self.worktree.is_none()
            && self.tmux_window.is_none()
            && matches!(self.status, TaskStatus::Running | TaskStatus::Review)
    }

    /// Whether this task has an agent window the user can reach: Running or
    /// Review with a tmux window recorded. The board's own answer, as the
    /// agent-tree pane's agents section lists it (`RefreshAgentTreeAgentList`
    /// in `docs/specs/agent-tree.allium`). `TaskRead::list_live_agent_tasks`
    /// is its SQL mirror.
    pub fn is_live_agent(&self) -> bool {
        self.tmux_window.is_some()
            && matches!(self.status, TaskStatus::Running | TaskStatus::Review)
    }

    /// May THIS install act on the task's worktree and tmux window? True when
    /// no machine holds a worktree for it (`host` is `None` — nothing to
    /// conflict over) and true when the machine holding it is this one.
    ///
    /// `local_host_id` is `None` when this install's own id is not known yet —
    /// before `TuiRuntime::bootstrap` reads it. A caller in that state cannot
    /// claim any host-held task as its own, so only the unheld arm answers
    /// true.
    ///
    /// Mirrors `core/Task::is_locally_owned` in `docs/specs/core.allium`. On a
    /// single-machine install `host` is always `None` or `local_host_id`, so
    /// every gate built on this is a no-op — see `DispatchTask`, `ResumeTask`,
    /// `RetryResume` and `RetryFresh` in `docs/specs/dispatch.allium`.
    ///
    /// The store's claim reducers apply the same rule on their side
    /// (`validate_task_ownership` in `spacetime/module/`), so a third arm
    /// added here is owed there too.
    pub fn is_locally_owned(&self, local_host_id: Option<&str>) -> bool {
        match (&self.host, local_host_id) {
            (None, _) => true,
            (Some(host), Some(local)) => host == local,
            (Some(_), None) => false,
        }
    }

    /// Whether this task is a phoenix whose respawn did not land.
    ///
    /// `PhoenixRespawn` clears `phoenix` exactly when the successor row was
    /// created, so a phoenix task sitting in Done is one that still owes a
    /// respawn — there is no separate error column to keep in step with this.
    /// Rendered as a red `⚠ respawn failed` card indicator; see "Phoenix
    /// marker" in `docs/specs/board-visuals.allium`.
    pub fn respawn_failed(&self) -> bool {
        self.phoenix && self.status == TaskStatus::Done
    }

    /// Why this task cannot be wrapped up, or `None` when it can.
    ///
    /// The whole question lives here rather than half here and half in the
    /// service, because the two halves answer the same thing and a caller that
    /// consults only one gets an answer the other contradicts. A `bool` could
    /// not carry the two remedies apart — one asks the caller to dispatch the
    /// task, the other to retag it — so the predicate returns the reason and
    /// the service formats it.
    ///
    /// A predicate over `Task`, so it belongs on the model rather than on the
    /// dispatch adapter it used to live in — see the header of
    /// `src/models/tmux_window.rs` for why a pure predicate the service layer
    /// gates on cannot sit in an adapter.
    pub fn wrap_up_block(&self) -> Option<WrapUpBlock> {
        // The tag outranks the state. A review task is refused whatever its
        // status, so reporting "it needs a worktree" first would send the
        // caller to fix something that would not help.
        if let Some(tag) = self.tag.filter(TaskTag::is_review) {
            return Some(WrapUpBlock::ReviewTag(tag));
        }
        if self.worktree.is_none()
            || !matches!(self.status, TaskStatus::Running | TaskStatus::Review)
        {
            return Some(WrapUpBlock::NotDispatched);
        }
        None
    }

    /// Whether this task can be wrapped up at all.
    ///
    /// Defined from [`Task::wrap_up_block`] so the predicate and the reason
    /// cannot disagree: every gate, and every surface that merely wants to know
    /// whether the affordance applies, reads the same answer.
    pub fn is_wrappable(&self) -> bool {
        self.wrap_up_block().is_none()
    }
}

/// Why a task is not wrappable. Each variant carries its own remedy, which is
/// what a bare predicate could not.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WrapUpBlock {
    /// Not Running or Review, or has no worktree — the task was never
    /// dispatched, or its session is already over.
    NotDispatched,
    /// Tagged for review. A review task ends at its PR or when it is handed
    /// back, not at wrap-up — see `ReviewTasksAreNotWrappedUp` in
    /// `docs/specs/mcp-task-tools.allium`.
    ReviewTag(TaskTag),
}

// ---------------------------------------------------------------------------
// FeedItem — an item from a programmable epic feed
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedItem {
    pub external_id: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub url: String,
    /// Optional explicit type for `url`. When set, the inserted task's
    /// url_type is taken verbatim; when absent it is inferred from the URL
    /// string. Lets a feed declare types inference cannot reach (e.g.
    /// `security_alert` for Dependabot alert URLs). `#[serde(default)]`
    /// keeps wire compatibility with scripts written before this field
    /// existed. Ignored when `url` is empty.
    #[serde(default)]
    pub url_type: Option<UrlType>,
    pub status: TaskStatus,
    /// Required: feed scripts must declare which TaskTag the inserted task
    /// receives, so dispatch routes feed-derived tasks to the correct agent
    /// (e.g. `pr-review` for Dependabot PRs, `fix` for security alerts).
    pub tag: TaskTag,
    /// Free-form labels copied to `Task.labels` on insert and on conflict.
    /// `#[serde(default)]` keeps wire compatibility with scripts written
    /// before this field existed.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Ordering hint copied to `Task.sort_order` (lower sorts first). Used
    /// by the CVE feed to surface CRITICAL alerts above HIGH/MEDIUM/LOW.
    #[serde(default)]
    pub sort_order: Option<i64>,
    /// Routing signals attached by the feed script (e.g. `direct-request`,
    /// `author-bot`). Used by later WPs to route PR items into the right
    /// feed bucket. Unrecognised values are dropped with a warning rather
    /// than failing the whole item: signals are additive routing metadata,
    /// so a value introduced by a newer feed script must not break ingest
    /// on an older binary. This is a deliberate, scoped exception to the
    /// "parse failures must surface" boundary rule in docs/conventions.md —
    /// a single unknown signal should not poison an otherwise-valid item.
    #[serde(default, deserialize_with = "deserialize_lenient_signals")]
    pub signals: Vec<Signal>,
    /// Optional wrap-up mode copied to `Task.wrap_up_mode` on insert only.
    /// On conflict (a re-poll of the same `external_id`) the existing task's
    /// wrap_up_mode is preserved — like status/sub_status/repo_path — so a
    /// user's manual wrap-up choice survives feed refreshes. `#[serde(default)]`
    /// keeps wire compatibility with scripts written before this field
    /// existed: absent leaves the inserted task's wrap_up_mode NULL (decide at
    /// wrap-up time). Used by the CVE feed to default fix tasks to `pr`.
    #[serde(default)]
    pub wrap_up_mode: Option<WrapUpMode>,
}

impl FeedItem {
    /// The `UrlType` this item's `url` resolves to, or `None` when it carries
    /// no url at all.
    ///
    /// The one place the precedence lives: an explicit [`FeedItem::url_type`]
    /// wins, otherwise the type is inferred from the URL string, and an empty
    /// url has no type (there is nothing to type). Feed ingest writes the
    /// `url`/`url_type` column pair from this, and
    /// [`crate::feed::parse_feed_items`] validates against it — so the type a
    /// review-tag check rejects on is exactly the type the row would have got.
    pub fn resolved_url_type(&self) -> Option<UrlType> {
        if self.url.is_empty() {
            return None;
        }
        Some(self.url_type.unwrap_or_else(|| UrlType::infer(&self.url)))
    }

    /// `Err` with a message naming this item when it breaks a cross-field
    /// rule of the feed wire format.
    ///
    /// Today there is one such rule: a review-tagged item must name a pull
    /// request (`AReviewTaggedFeedItemNamesItsPr` in `docs/specs/feeds.allium`).
    /// The dependabot runbook omits its author check on the strength of a feed
    /// having filtered by bot author, and that omission is only sound if a
    /// feed-created review task actually names the PR the feed listed.
    ///
    /// Cross-field, so it cannot live in the `Deserialize` impl beside the
    /// strict `tag` rule — serde's field attributes cannot see a sibling
    /// field, and a shadow struct would duplicate every field. The single
    /// decode point calls this instead.
    pub fn validate(&self) -> Result<(), String> {
        // Guarded before resolving, so a non-review item never pays the infer
        // scan — every CVE and fix item goes through here too.
        if !self.tag.is_review() {
            return Ok(());
        }
        let resolved = self.resolved_url_type();
        if resolved == Some(UrlType::Pr) {
            return Ok(());
        }
        Err(format!(
            "feed item {:?} has tag {} but names no pull request: url {:?} types as {}. \
A review-tagged item must carry a pr url — see AReviewTaggedFeedItemNamesItsPr in \
docs/specs/feeds.allium.",
            self.external_id,
            self.tag,
            self.url,
            resolved.map_or("nothing (empty url)", |t| t.as_str()),
        ))
    }
}

// ---------------------------------------------------------------------------
// Signal — routing hints a feed script attaches to a FeedItem
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Signal {
    DirectRequest,
    TeamRequest,
    Reviewed,
    Commented,
    AuthorBot,
    AuthorMe,
    OrgReview,
    /// The user's latest opinionated review on the PR is an approval. Read by
    /// `excluded_from_reviews` (which pairs it with the request signals), not
    /// by `route`.
    Approved,
}

/// Deserialize `FeedItem.signals`, dropping any entry that is not a recognised
/// `Signal` (logging each at `warn`). See the field doc for why this is lenient
/// rather than surfacing the error like the rest of the feed-JSON boundary.
fn deserialize_lenient_signals<'de, D>(deserializer: D) -> Result<Vec<Signal>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Vec::<serde_json::Value>::deserialize(deserializer)?;
    let mut signals = Vec::with_capacity(raw.len());
    for value in raw {
        // Deserialize from a borrow so `value` stays available for the warn.
        match Signal::deserialize(&value) {
            Ok(sig) => signals.push(sig),
            Err(_) => tracing::warn!(value = %value, "dropping unrecognised feed signal"),
        }
    }
    Ok(signals)
}

/// A placeholder Task for building fixtures with struct-update syntax
/// (`Task { field: value, ..Default::default() }`). `id`, `title`,
/// `description`, and `repo_path` are meaningless placeholders — callers that
/// care about them should always set them explicitly.
impl Default for Task {
    fn default() -> Self {
        let now = Utc::now();
        Task {
            id: TaskId(0),
            title: String::new(),
            description: String::new(),
            repo_path: "/repo".to_string(),
            status: TaskStatus::Backlog,
            worktree: None,
            tmux_window: None,
            host: None,
            plan_path: None,
            epic_id: None,
            sub_status: SubStatus::None,
            url: None,
            tag: None,
            sort_order: None,
            completed_at: None,
            base_branch: DEFAULT_BASE_BRANCH.to_string(),
            external_id: None,
            labels: Vec::new(),
            created_at: now,
            updated_at: now,
            last_pre_tool_use_at: None,
            last_notification_at: None,
            last_peer_message_sent_at: None,
            last_peer_message_received_at: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
            live_subagents: 0,
            stop_pending: false,
        }
    }
}

// ---------------------------------------------------------------------------
// DispatchMode
// ---------------------------------------------------------------------------

/// Determines how a backlog task should be dispatched. Most tasks route to
/// `Dispatch`, which produces the unified prompt skeleton (with-plan or
/// no-plan variant). The `research` tag is the only one with a dedicated
/// agent — its prompt instructs the agent to make no code changes while it
/// presents findings to the user. That is an instruction, not an enforced
/// permission boundary (`EveryTaskAgentLaunchesInAutoMode` in
/// `docs/specs/dispatch.allium`). Other tags (`pr_review`, `fix`,
/// `dependabot`) are kanban labels and route through the unified
/// `Dispatch` path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchMode {
    Dispatch,
    Research,
}

impl DispatchMode {
    pub fn label(self) -> &'static str {
        match self {
            DispatchMode::Dispatch => "Dispatch",
            DispatchMode::Research => "Research",
        }
    }

    /// Select the dispatch mode for a task: tasks with a plan always go
    /// through the unified `Dispatch` path; otherwise only the `research`
    /// tag routes to its dedicated agent.
    pub fn for_task(task: &Task) -> Self {
        if task.plan_path.is_some() {
            DispatchMode::Dispatch
        } else {
            match task.tag {
                Some(TaskTag::Research) => DispatchMode::Research,
                _ => DispatchMode::Dispatch,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// TaskTag
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskTag {
    Bug,
    Feature,
    Chore,
    #[serde(rename = "pr-review")]
    PrReview,
    Research,
    Fix,
    Dependabot,
}

impl TaskTag {
    /// Every variant, in kanban tag-picker order.
    ///
    /// Derive a SUBSET from this — `ALL.iter().copied().filter(..)` — rather
    /// than writing the variants out again. A hand-written list lets a new
    /// variant slip past by being absent from it, which no compiler error
    /// catches: `task_tag_is_review_only_for_pr_review_and_dependabot` and the
    /// review-tag coverage in `crate::feed::parse_feed_items`'s tests both
    /// depend on that.
    ///
    /// Note the shape: an associated const, not a method. `define_str_enum!`
    /// generates the string quartet and not this, so it is hand-maintained per
    /// enum — searching for an `all()` function finds nothing and invites the
    /// duplicate list above.
    pub const ALL: &'static [TaskTag] = &[
        TaskTag::Bug,
        TaskTag::Feature,
        TaskTag::Chore,
        TaskTag::PrReview,
        TaskTag::Research,
        TaskTag::Fix,
        TaskTag::Dependabot,
    ];

    pub fn short_label(&self) -> &'static str {
        match self {
            TaskTag::Bug => "bug",
            TaskTag::Feature => "feat",
            TaskTag::Chore => "chore",
            TaskTag::PrReview => "pr-rev",
            TaskTag::Research => "research",
            TaskTag::Fix => "fix",
            TaskTag::Dependabot => "dep",
        }
    }

    /// Whether this tag routes to a PR-review agent (PR review or
    /// Dependabot). Review tasks skip the plan/implement flow and, when they
    /// carry a PR URL, base their worktree on the PR's branch.
    ///
    /// Also read by `ColumnSection::for_task` to mean "this task reviews
    /// someone else's PR", which is what makes a review decision on it the
    /// user's own. A tag added here that routes to the review agent but
    /// authors its own PR would mislabel both by-me sections.
    pub fn is_review(&self) -> bool {
        matches!(self, TaskTag::PrReview | TaskTag::Dependabot)
    }

    // A third reader of `tag` lives outside this type, and is a VETO rather
    // than a selector — which is why it is not a predicate here.
    // `is_cve_task` routes a task to the CVE runbook on its epic's ancestry,
    // never on a tag, but lets `PrReview`, `Dependabot` and `Research` claim
    // it back first. Its exclusion list belongs to the prompt's design-step
    // rule rather than to this type, and the two must stay identical, so
    // there is nothing to hoist here. Noted because searching `TaskTag` is how
    // the other two readers are found. See "Tag system" in
    // `docs/conventions.md`.
}

define_str_enum!(TaskTag, "tag" {
    Bug => "bug",
    Feature => "feature",
    Chore => "chore",
    PrReview => "pr-review",
    Research => "research",
    Fix => "fix",
    Dependabot => "dependabot",
});

// ---------------------------------------------------------------------------
// WrapUpMode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WrapUpMode {
    Rebase,
    Pr,
    Done,
}

impl WrapUpMode {
    pub const ALL: &'static [WrapUpMode] = &[WrapUpMode::Rebase, WrapUpMode::Pr, WrapUpMode::Done];
}

define_str_enum!(WrapUpMode, "wrap-up mode" {
    Rebase => "rebase",
    Pr => "pr",
    Done => "done",
});

// ---------------------------------------------------------------------------
// DispatchResult
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DispatchResult {
    pub worktree_path: String,
    pub tmux_window: TmuxWindow,
    /// Whether `worktree_path` already existed and was reused, rather than
    /// being created by this dispatch.
    ///
    /// Only provisioning can answer this — it measures the directory before
    /// acting, and afterwards the directory exists either way — so the answer
    /// is carried here rather than re-derived downstream. See
    /// rule-guidance.DispatchTaskViaMcp in docs/specs/mcp-task-tools.allium
    /// for why a caller is told.
    pub reused_worktree: bool,
}

// ---------------------------------------------------------------------------
// ResumeResult
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ResumeResult {
    pub tmux_window: TmuxWindow,
}

// ---------------------------------------------------------------------------
// slugify
// ---------------------------------------------------------------------------

/// Convert an arbitrary string into a URL/filesystem-safe slug.
/// - Lowercased
/// - Non-alphanumeric characters replaced with `-`
/// - Consecutive dashes collapsed to one
/// - Leading/trailing dashes trimmed
/// - Returns `"task"` if the result would be empty
pub fn slugify(input: &str) -> String {
    let lower = input.to_lowercase();
    let mut slug = String::with_capacity(lower.len());
    let mut last_was_dash = false;

    for ch in lower.chars() {
        if ch.is_alphanumeric() {
            slug.push(ch);
            last_was_dash = false;
        } else {
            if !last_was_dash && !slug.is_empty() {
                slug.push('-');
                last_was_dash = true;
            }
        }
    }

    // Trim trailing dash
    let slug = slug.trim_end_matches('-').to_string();

    if slug.is_empty() {
        "task".to_string()
    } else {
        slug
    }
}

// ---------------------------------------------------------------------------
// Staleness
// ---------------------------------------------------------------------------

/// Tasks updated within this many hours are considered fresh.
const FRESH_THRESHOLD_HOURS: i64 = 3 * 24; // 3 days
/// Tasks updated within this many hours are aging (not yet stale).
const AGING_THRESHOLD_HOURS: i64 = 7 * 24; // 7 days
/// Days threshold above which format_age switches to weeks.
const WEEKS_THRESHOLD_DAYS: i64 = 14;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Staleness {
    Fresh,
    Aging,
    Stale,
}

impl Staleness {
    /// Determine staleness tier from the age of `timestamp` relative to `now`.
    pub fn from_age(timestamp: DateTime<Utc>, now: DateTime<Utc>) -> Self {
        let age = now.signed_duration_since(timestamp);
        let hours = age.num_hours().max(0);
        if hours < FRESH_THRESHOLD_HOURS {
            Staleness::Fresh
        } else if hours < AGING_THRESHOLD_HOURS {
            Staleness::Aging
        } else {
            Staleness::Stale
        }
    }
}

// ---------------------------------------------------------------------------
// format_age
// ---------------------------------------------------------------------------

/// Format the age of `updated_at` relative to `now` as a compact label.
/// Returns strings like "<1h", "3h", "2d", "3w".
pub fn format_age(updated_at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let age = now.signed_duration_since(updated_at);
    let hours = age.num_hours().max(0);

    if hours < 1 {
        "<1h".to_string()
    } else if hours < 24 {
        format!("{hours}h")
    } else {
        let days = hours / 24;
        if days < WEEKS_THRESHOLD_DAYS {
            format!("{days}d")
        } else {
            format!("{}w", days / 7)
        }
    }
}

// ---------------------------------------------------------------------------
// format_detail_age
// ---------------------------------------------------------------------------

/// Format age for the detail panel — slightly more verbose than card labels.
/// Returns strings like "less than 1 hour", "1 hour", "5 hours", "1 day", "3 days".
pub fn format_detail_age(updated_at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let age = now.signed_duration_since(updated_at);
    let total_hours = age.num_hours().max(0);

    if total_hours < 1 {
        "less than 1 hour".to_string()
    } else if total_hours == 1 {
        "1 hour".to_string()
    } else if total_hours < 24 {
        format!("{total_hours} hours")
    } else {
        let days = total_hours / 24;
        if days == 1 {
            "1 day".to_string()
        } else {
            format!("{days} days")
        }
    }
}

#[cfg(test)]
pub(in crate::models) mod model_tests;

#[cfg(test)]
mod property_tests;

#[cfg(test)]
mod tests;
