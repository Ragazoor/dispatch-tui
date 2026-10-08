/// Sentinel identifier for the "no parent" option in the reparent tree picker.
pub(in crate::tui) const REPARENT_NO_PARENT_SENTINEL: &str = "__no_parent__";

use crate::models::{
    DispatchMode, Epic, EpicId, Task, TaskId, TaskStatus, TaskTag, WrapUpMode, DEFAULT_BASE_BRANCH,
};

mod fold;
mod layout;
mod state;
pub use fold::*;
pub use layout::*;
pub use state::*;

// ---------------------------------------------------------------------------
// MoveDirection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveDirection {
    Forward,
    Backward,
}

// ---------------------------------------------------------------------------
// RepoFilterMode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepoFilterMode {
    #[default]
    Include,
    Exclude,
}

impl RepoFilterMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            RepoFilterMode::Include => "include",
            RepoFilterMode::Exclude => "exclude",
        }
    }
}

impl std::str::FromStr for RepoFilterMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "include" => Ok(RepoFilterMode::Include),
            "exclude" => Ok(RepoFilterMode::Exclude),
            _ => Err(format!("unknown filter mode: {s}")),
        }
    }
}

// ---------------------------------------------------------------------------
// EditKind / EditorOutcome — tags for the pop-out editor flow
// ---------------------------------------------------------------------------

/// Identifies what the user is editing and how to finalize the edit when
/// the pop-out editor closes. One variant per existing $EDITOR call-site.
#[derive(Debug, Clone)]
/// Both entity payloads are boxed. `Task` (~470 bytes) and `Epic` (~180) each
/// dwarf `Description`, and because this type is reached through
/// `EditorMessage`/`EditorCommand` from the `Message`/`Command` bus, an inline
/// entity here is paid for by every variant of all four enums.
pub enum EditKind {
    /// Full task editor (title/description/repo_path/status/plan/tag/base_branch).
    TaskEdit(Box<Task>),
    /// Full epic editor (title/description/repo_path).
    EpicEdit(Box<Epic>),
    /// Description-only editor used during task/epic creation.
    /// `is_epic` distinguishes the epic-create flow from the task-create flow.
    Description { is_epic: bool },
}

/// Result of a pop-out editor session. `Saved` carries the final tempfile
/// contents; `Cancelled` means the editor closed without a readable result
/// (e.g. the tempfile disappeared, or the tmux window was killed while the
/// editor buffer was empty).
#[derive(Debug, Clone)]
pub enum EditorOutcome {
    Saved(String),
    Cancelled,
}

// ---------------------------------------------------------------------------
// TreeNav — directional navigation within the tree view
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub enum TreeNav {
    Up,
    Down,
    Left,
    Right,
}

/// Apply a `TreeNav` direction to a `TreeState`. Used by the reparent-epic
/// picker and the move-to-epic tree picker.
pub(in crate::tui) fn apply_tree_nav<Id: Clone + PartialEq + Eq + std::hash::Hash>(
    state: &mut tui_tree_widget::TreeState<Id>,
    nav: TreeNav,
) {
    match nav {
        TreeNav::Up => {
            state.key_up();
        }
        TreeNav::Down => {
            state.key_down();
        }
        TreeNav::Left => {
            state.key_left();
        }
        TreeNav::Right => {
            state.key_right();
        }
    }
}

// ---------------------------------------------------------------------------
// Message
// ---------------------------------------------------------------------------

/// Everything the update loop reacts to, routed to a per-domain inner enum.
///
/// Moved by value on every keystroke, async result and loop iteration, so no
/// variant may carry a domain entity inline. Guard-railed by the
/// `assert_no_entity_inline` tests at the bottom of this file.
#[derive(Debug, Clone)]
pub enum Message {
    /// System-level messages — see [`crate::tui::messages::SystemMessage`].
    System(crate::tui::messages::SystemMessage),
    /// Task-domain messages — see [`crate::tui::messages::TaskMessage`].
    Task(crate::tui::messages::TaskMessage),
    NavigateColumn(isize),
    NavigateRow(isize),
    NavigateRowFirst,
    NavigateRowLast,
    RepoPathsUpdated(Vec<String>),
    /// Full-board reload of per-repo base_branch history, keyed by repo_path
    /// (see docs/specs/dispatch.allium: surface BaseBranchPicker).
    BaseBranchesUpdated(std::collections::HashMap<String, Vec<String>>),
    ClearSelection,
    SelectAllColumn,
    /// Fold or unfold the sub-status section the cursor is in
    /// (tasks.allium: ToggleSectionCollapse).
    ToggleSectionCollapse,
    /// Fold or unfold the flattened epic group the cursor is in
    /// (tasks.allium: ToggleEpicFold).
    ToggleEpicFold,
    /// Form-input flow messages — see [`crate::tui::messages::InputMessage`].
    Input(crate::tui::messages::InputMessage),
    /// Pop-out `$EDITOR` flow messages — see
    /// [`crate::tui::messages::EditorMessage`].
    Editor(crate::tui::messages::EditorMessage),
    /// Split-pane mode messages — see [`crate::tui::messages::SplitMessage`].
    Split(crate::tui::messages::SplitMessage),
    /// Epic-domain messages — see [`crate::tui::messages::EpicMessage`].
    Epic(crate::tui::messages::EpicMessage),
    /// PR flow messages — see [`crate::tui::messages::PrMessage`].
    Pr(crate::tui::messages::PrMessage),
    /// Repo-filter overlay messages — see [`crate::tui::messages::RepoFilterMessage`].
    RepoFilter(crate::tui::messages::RepoFilterMessage),
    /// Local-first repo sync messages — see
    /// [`crate::tui::messages::RepoSyncMessage`].
    RepoSync(crate::tui::messages::RepoSyncMessage),
    /// Feed-epic refresh messages — see [`crate::tui::messages::FeedMessage`].
    Feed(crate::tui::messages::FeedMessage),
    /// Budget-indicator messages — see [`crate::tui::messages::BudgetMessage`].
    Budget(crate::tui::messages::BudgetMessage),
}

// ---------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------

/// Side effects the runtime executes on the update loop's behalf.
///
/// A pure router: every variant wraps exactly one per-domain inner enum from
/// [`crate::tui::commands`], with no payload of its own. The migration that
/// established that shape is complete — adding a new inline variant here
/// instead of a variant on (or a new module beside) one of the inner enums
/// reintroduces the half-done split it was done to remove.
///
/// Same size constraint as [`Message`]: moved by value, so no variant reachable
/// from here may carry a domain entity inline.
#[derive(Debug, Clone)]
pub enum Command {
    /// Task-domain side-effect commands — see
    /// [`crate::tui::commands::TaskCommand`].
    Task(crate::tui::commands::TaskCommand),
    /// Split-pane mode side-effect commands — see [`crate::tui::commands::SplitCommand`].
    Split(crate::tui::commands::SplitCommand),
    /// Pop-out `$EDITOR` flow side-effect commands — see
    /// [`crate::tui::commands::EditorCommand`].
    Editor(crate::tui::commands::EditorCommand),
    /// Settings/preference-persistence side-effect commands — see
    /// [`crate::tui::commands::SettingsCommand`].
    Settings(crate::tui::commands::SettingsCommand),
    /// Epic-domain side-effect commands — see
    /// [`crate::tui::commands::EpicCommand`].
    Epic(crate::tui::commands::EpicCommand),
    /// Feed-epic refresh side-effect commands — see
    /// [`crate::tui::commands::FeedCommand`].
    Feed(crate::tui::commands::FeedCommand),
    /// System-level side-effect commands — see
    /// [`crate::tui::commands::SystemCommand`].
    System(crate::tui::commands::SystemCommand),
    /// Repo-filter overlay side-effect commands — see [`crate::tui::commands::RepoFilterCommand`].
    RepoFilter(crate::tui::commands::RepoFilterCommand),
    /// Local-first repo sync side-effect commands — see
    /// [`crate::tui::commands::RepoSyncCommand`].
    RepoSync(crate::tui::commands::RepoSyncCommand),
    /// PR flow side-effect commands — see [`crate::tui::commands::PrCommand`].
    Pr(crate::tui::commands::PrCommand),
    /// Background learning-maintenance side-effect commands — see
    /// [`crate::tui::commands::LearningCommand`].
    Learning(crate::tui::commands::LearningCommand),
    /// Usage-telemetry side-effect commands — see
    /// [`crate::tui::commands::UsageCommand`].
    Usage(crate::tui::commands::UsageCommand),
    /// Budget-indicator side-effect commands — see
    /// [`crate::tui::commands::BudgetCommand`].
    Budget(crate::tui::commands::BudgetCommand),
}

// ---------------------------------------------------------------------------
// InputMode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    /// Live task-search bar (title or task id). The board filters in place as
    /// the user types.
    SearchTasks,
    InputTitle,
    InputDescription,
    InputRepoPath,
    /// Tag picker. Also where phoenix is armed: `p` sets the flag and re-opens
    /// this same step with `p` dropped from the accepted set, so the real tag
    /// is still picked (CreateTask: PhoenixArming, in `docs/specs/tasks.allium`).
    InputTag,
    /// Single-task permanent delete (`tasks.allium: DeleteTask`,
    /// `DeleteKeyRouting`). The id is captured when 'x' was pressed, so a
    /// cursor drift before 'y' cannot redirect the delete to a different card.
    ConfirmDeleteTask(TaskId),
    QuickDispatch,
    ConfirmRetry(TaskId),
    /// Batch permanent delete (`tasks.allium: BatchDelete`) — a multi-selection
    /// of tasks (all Done) and/or epics (whole subtree done). Reads the
    /// current `select.tasks`/`select.epics` at confirm time, which is why the
    /// variant carries no payload.
    ConfirmBatchDelete,
    /// Review → Done confirmation. The tasks awaiting confirmation live in
    /// `select.pending_done` (one entry for a single move, N for a batch),
    /// which is why the variant carries no payload.
    ConfirmDone,
    ConfirmDetachTmux(Vec<TaskId>),
    // Epic input modes
    InputEpicTitle,
    InputEpicDescription,
    /// Single-epic permanent delete (`epics.allium: ConfirmDeleteEpic`),
    /// guarded on the epic's whole subtree being done.
    ConfirmDeleteEpic,
    /// Shown after `EditEpic` applies a `feed_command` change that conflicts
    /// with an existing `core/PollOwner` claim (`epics.allium: EditEpic`,
    /// `feeds.allium: OverrideFeedOwner`). The edit has ALREADY been applied
    /// by the time this shows — accepting only decides ownership.
    ConfirmOverrideFeedOwner {
        epic_id: EpicId,
        other_host: String,
    },
    ReparentEpic(EpicId),
    ConfirmReparentEpic {
        epic_id: EpicId,
        new_parent: Option<EpicId>,
    },
    // Move-task-to-epic tree picker (the `m` key on a task card)
    MoveTaskToEpic(TaskId),
    ConfirmMoveTaskToEpic {
        task_id: TaskId,
        new_epic: Option<EpicId>,
    },
    // Overlay modes
    Help,
    RepoFilter,
    ConfirmDeleteRepoPath,
    ConfirmQuit,
    ConfirmTrustRepo {
        task_id: TaskId,
        mode: DispatchMode,
    },
    /// Quick-dispatch's equivalent of `ConfirmTrustRepo`: entered when
    /// `TaskCommand::QuickDispatch`'s trust check finds the repo untrusted.
    /// No `Task`/`TaskId` exists yet at this point, so the pending draft is
    /// carried directly instead.
    ConfirmTrustRepoQuickDispatch {
        draft: TaskDraft,
        epic_id: Option<EpicId>,
    },
    InputBaseBranch,
    /// The creation form's last step. phoenix has no step of its own — it is
    /// armed at [`InputMode::InputTag`] (CreateTask: PhoenixArming, in
    /// `docs/specs/tasks.allium`).
    InputWrapUpMode,
    /// Sync confirmation for one repository (docs/specs/repo-sync.allium:
    /// surface RepoSyncConfirmation). Carries only the repo path: the
    /// measurement itself is re-read from `App.repo_sync` at confirm time, so a
    /// refresh that lands while the prompt is open cannot be acted on stale.
    ConfirmRepoSync {
        repo_path: String,
    },
}

impl InputMode {
    /// The repo-picker modes whose filtered list depends on the query, so any
    /// query edit must reset the list cursor to 0 (per RepoPathPicker in
    /// dispatch.allium). `InputBaseBranch` shares this cursor-reset-on-type
    /// contract (per BaseBranchPicker) even though its candidate list is a
    /// per-repo branch history rather than the global repo-path set — see
    /// `handle_move_repo_cursor` and the Enter-selection branch in
    /// `submit_text_input`, which special-case it for candidate lookup.
    pub fn is_repo_picker(&self) -> bool {
        matches!(
            self,
            InputMode::InputRepoPath | InputMode::QuickDispatch | InputMode::InputBaseBranch
        )
    }
}

// ---------------------------------------------------------------------------
// TaskDraft
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskDraft {
    pub title: String,
    pub description: String,
    pub repo_path: String,
    pub tag: Option<TaskTag>,
    pub base_branch: String,
    pub wrap_up_mode: Option<WrapUpMode>,
    pub phoenix: bool,
}

impl Default for TaskDraft {
    fn default() -> Self {
        Self {
            title: String::new(),
            description: String::new(),
            repo_path: String::new(),
            tag: None,
            base_branch: DEFAULT_BASE_BRANCH.to_string(),
            wrap_up_mode: None,
            phoenix: false,
        }
    }
}

// ---------------------------------------------------------------------------
// TaskEdit — bundled fields for Message::TaskEdited
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TaskEdit {
    pub id: TaskId,
    pub title: String,
    pub description: String,
    pub repo_path: String,
    pub status: TaskStatus,
    pub plan_path: Option<String>,
    pub tag: Option<TaskTag>,
    pub base_branch: Option<String>,
    pub wrap_up_mode: Option<crate::models::WrapUpMode>,
    /// Resolved post-edit url value (not a delta): `Some` to set, `None` to
    /// clear or leave absent. Applied directly to the in-memory task snapshot.
    pub url: Option<crate::models::TaskUrl>,
    pub phoenix: bool,
}

// ---------------------------------------------------------------------------
// EpicDraft — fields collected during epic creation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct EpicDraft {
    pub title: String,
    pub description: String,
    pub parent_epic_id: Option<EpicId>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
