/// Sentinel identifier for the "no parent" option in the reparent tree picker.
pub(in crate::tui) const REPARENT_NO_PARENT_SENTINEL: &str = "__no_parent__";

use crate::models::{
    DispatchMode, Epic, EpicId, Task, TaskId, TaskModel, TaskStatus, TaskTag, WorkPackageId,
    WrapUpMode, DEFAULT_BASE_BRANCH,
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
// Lens — how the board is read (`board-layout.allium`, "Board Lens")
// ---------------------------------------------------------------------------

/// The board's reading mode. The only state behind the only-active filter and
/// flattening: both are derived from it, never set on their own. Tab steps it
/// forward, BackTab back. Session-scoped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lens {
    #[default]
    Everything,
    Active,
    Flat,
    FlatActive,
}

impl Lens {
    /// Whether only tasks with a live session are shown.
    pub fn only_active(self) -> bool {
        matches!(self, Lens::Active | Lens::FlatActive)
    }

    /// Whether the Running, Review and Done columns are flattened.
    pub fn flattened(self) -> bool {
        matches!(self, Lens::Flat | Lens::FlatActive)
    }

    pub fn next(self) -> Self {
        match self {
            Lens::Everything => Lens::Active,
            Lens::Active => Lens::Flat,
            Lens::Flat => Lens::FlatActive,
            Lens::FlatActive => Lens::Everything,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Lens::Everything => Lens::FlatActive,
            Lens::Active => Lens::Everything,
            Lens::Flat => Lens::Active,
            Lens::FlatActive => Lens::Flat,
        }
    }

    /// The lens with the given switches. Lets a test say "flattened" without
    /// naming which of the two flattened lenses it means.
    #[cfg(test)]
    pub fn from_switches(only_active: bool, flattened: bool) -> Self {
        match (only_active, flattened) {
            (false, false) => Lens::Everything,
            (true, false) => Lens::Active,
            (false, true) => Lens::Flat,
            (true, true) => Lens::FlatActive,
        }
    }

    /// This lens with flattening set to `on`, only-active unchanged.
    #[cfg(test)]
    pub fn with_flattened(self, on: bool) -> Self {
        Self::from_switches(self.only_active(), on)
    }

    /// This lens with only-active set to `on`, flattening unchanged.
    #[cfg(test)]
    pub fn with_only_active(self, on: bool) -> Self {
        Self::from_switches(on, self.flattened())
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
    /// Repo-filter overlay messages — see [`crate::tui::messages::BoardFilterMessage`].
    BoardFilter(crate::tui::messages::BoardFilterMessage),
    /// Feed-epic refresh messages — see [`crate::tui::messages::FeedMessage`].
    Feed(crate::tui::messages::FeedMessage),
    /// Budget-indicator messages — see [`crate::tui::messages::BudgetMessage`].
    Budget(crate::tui::messages::BudgetMessage),
    /// Work-package messages — see
    /// [`crate::tui::messages::WorkPackageMessage`].
    WorkPackage(crate::tui::messages::WorkPackageMessage),
    /// Admin-screen messages — see [`crate::tui::messages::AdminMessage`].
    Admin(crate::tui::messages::AdminMessage),
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
    /// Work-package side-effect commands — see
    /// [`crate::tui::commands::WorkPackageCommand`].
    WorkPackage(crate::tui::commands::WorkPackageCommand),
    /// People side-effect commands — see
    /// [`crate::tui::commands::PeopleCommand`].
    People(crate::tui::commands::PeopleCommand),
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
    BoardFilter,
    /// Typing the Epics tab's `/` title search inside the board filter.
    BoardFilterSearch,
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
    /// The wrap-up picker. phoenix has no step of its own — it is armed at
    /// [`InputMode::InputTag`] (CreateTask: PhoenixArming, in
    /// `docs/specs/tasks.allium`).
    InputWrapUpMode,
    /// The creation form's last step: the model picker (CreateTask:
    /// ModelPicker). Answering it creates the task.
    InputModel,
    /// The work-package link picker (`w` on a task or epic card —
    /// `work-packages.allium: WorkPackageLinkPicker`). `cursor` indexes the
    /// board's packages in order, then the final "new package" row.
    LinkWorkPackage {
        item: crate::models::WorkPackageItem,
        cursor: usize,
    },
    /// The picker's "new package" title input. Carries the item the new
    /// package will be linked to, and returns to the picker either way.
    InputWorkPackageTitle(crate::models::WorkPackageItem),
    /// `e` on a package card: the one-line rename input.
    RenameWorkPackage(crate::models::WorkPackageId),
    /// `u`: the one-line input for the person's own display name
    /// (`workspaces.allium: SetOwnNameFromBoard`).
    InputDisplayName,
    /// `x` on a package card: delete the package (its links only).
    ConfirmDeleteWorkPackage(crate::models::WorkPackageId),
    /// The admin screen (`admin-surfaces.allium: OpenAdminScreen`). Its state
    /// is `InputState::admin`.
    Admin,
    /// A one-line input opened from the admin screen
    /// (`InputState::admin`'s `prompt` says which). Esc returns to the screen.
    InputAdminText,
    /// A y/n confirmation opened from the admin screen
    /// (`InputState::admin`'s `confirm` says which).
    ConfirmAdmin,
    /// The last step of the new-epic and new-package forms: workspace (when
    /// the person has several) and visibility
    /// (`workspaces.allium: NameWorkspaceAndVisibilityFromBoard`). The state
    /// is `InputState::placement`.
    InputPlacement,
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
    /// `None` launches on the default model (`core.allium: TaskModel`).
    pub model: Option<TaskModel>,
    pub phoenix: bool,
    /// The package view the task was drafted in: the new task is linked to it
    /// once created (`work-packages.allium: CreateTaskInPackageView`). Set
    /// when the task is submitted, so it names the view at that moment.
    pub work_package: Option<WorkPackageId>,
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
            model: None,
            phoenix: false,
            work_package: None,
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
    pub model: Option<crate::models::TaskModel>,
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
    /// The workspace a root epic is created in; `0` until chosen (the runtime
    /// then uses the person's only workspace). A sub-epic takes its parent's.
    pub workspace_id: i64,
    pub visibility: crate::models::Visibility,
}

// ---------------------------------------------------------------------------
// Placement — the workspace and visibility step ending a create form
// ---------------------------------------------------------------------------

/// What the placement step creates once answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlacementFor {
    /// The epic in `InputState::epic_draft`.
    Epic,
    /// A package, linked to `item`, from the link picker.
    Package {
        title: String,
        item: crate::models::WorkPackageItem,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub target: PlacementFor,
    pub visibility: crate::models::Visibility,
    /// Index into `InputState::workspaces`.
    pub workspace: usize,
    /// False for a sub-epic, which takes its parent's workspace.
    pub names_workspace: bool,
}

// ---------------------------------------------------------------------------
// The admin screen (`docs/specs/admin-surfaces.allium`)
// ---------------------------------------------------------------------------

/// The four panes of the admin screen, in Tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AdminPane {
    #[default]
    Workspaces,
    Members,
    SuperAdmins,
    Unseen,
}

/// What the one-line input opened from the screen is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdminPrompt {
    NewWorkspace,
    RenameWorkspace(i64),
    AddMember(i64),
    GrantSuperAdmin,
}

/// What the y/n confirmation opened from the screen would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdminConfirm {
    DeleteWorkspace {
        id: i64,
        name: String,
        members: usize,
    },
    RemoveMember {
        membership_id: i64,
        person: String,
        workspace: String,
    },
    RevokeSuperAdmin {
        identity: String,
    },
    DeleteUnseen {
        kind: String,
        id: i64,
    },
}

/// The admin screen's state: the snapshot it last read, where the highlight
/// is in each pane, and the prompt or confirmation open over it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdminScreen {
    pub snapshot: crate::models::AdminSnapshot,
    pub pane: AdminPane,
    pub workspace: usize,
    pub member: usize,
    pub super_admin: usize,
    pub unseen: usize,
    pub prompt: Option<AdminPrompt>,
    pub confirm: Option<AdminConfirm>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
