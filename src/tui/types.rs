use std::collections::{HashMap, HashSet};
use std::time::Instant;

use ratatui::widgets::ListState;

use crate::models::{
    Epic, EpicId, EpicSubstatus, Project, ProjectId, Task, TaskId, TaskStatus, TaskTag,
    TipsShowMode, WrapUpMode, DEFAULT_BASE_BRANCH,
};

// ---------------------------------------------------------------------------
// TipsOverlayState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TipsOverlayState {
    pub tips: Vec<crate::tips::Tip>,
    pub index: usize,
    /// Highest tip id that was already seen before this session started.
    /// Used to show the NEW badge on unseen tips.
    pub max_seen_id: u32,
    pub show_mode: TipsShowMode,
}

impl TipsOverlayState {
    pub fn current_tip(&self) -> Option<&crate::tips::Tip> {
        self.tips.get(self.index)
    }

    pub fn is_new(&self) -> bool {
        self.current_tip()
            .map(|t| t.id > self.max_seen_id)
            .unwrap_or(false)
    }
}

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

#[cfg(test)]
pub(crate) fn repo_filter_matches(
    filter: &HashSet<String>,
    mode: RepoFilterMode,
    repo: &str,
) -> bool {
    if filter.is_empty() {
        return true;
    }
    match mode {
        RepoFilterMode::Include => filter.contains(repo),
        RepoFilterMode::Exclude => !filter.contains(repo),
    }
}

// ---------------------------------------------------------------------------
// EditKind / EditorOutcome — tags for the pop-out editor flow
// ---------------------------------------------------------------------------

/// Identifies what the user is editing and how to finalize the edit when
/// the pop-out editor closes. One variant per existing $EDITOR call-site.
#[derive(Debug, Clone)]
pub enum EditKind {
    /// Full task editor (title/description/repo_path/status/plan/tag/base_branch).
    TaskEdit(Task),
    /// Full epic editor (title/description/repo_path).
    EpicEdit(Epic),
    /// Description-only editor used during task/epic creation.
    /// `is_epic` distinguishes the epic-create flow from the task-create flow.
    Description { is_epic: bool },
    /// Edit a learning's summary, kind, tags, and detail.
    Learning(crate::models::Learning),
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
// LearningsView — list vs tree display mode for the Learnings overlay
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LearningsView {
    #[default]
    List,
    Tree,
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

// ---------------------------------------------------------------------------
// Message
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Message {
    /// System-level messages — see [`crate::tui::messages::SystemMessage`].
    System(crate::tui::messages::SystemMessage),
    /// Task-domain messages — see [`crate::tui::messages::TaskMessage`].
    Task(crate::tui::messages::TaskMessage),
    NavigateColumn(isize),
    NavigateRow(isize),
    RepoPathsUpdated(Vec<String>),
    ClearSelection,
    SelectAllColumn,
    /// Form-input flow messages — see [`crate::tui::messages::InputMessage`].
    Input(crate::tui::messages::InputMessage),
    /// Pop-out `$EDITOR` flow messages — see
    /// [`crate::tui::messages::EditorMessage`].
    Editor(crate::tui::messages::EditorMessage),
    // Split mode messages
    ToggleSplitMode,
    SwapSplitPane(TaskId),
    SplitPaneOpened {
        pane_id: String,
        task_id: Option<TaskId>,
    },
    SplitPaneClosed,
    /// Epic-domain messages — see [`crate::tui::messages::EpicMessage`].
    Epic(crate::tui::messages::EpicMessage),
    /// PR flow messages — see [`crate::tui::messages::PrMessage`].
    Pr(crate::tui::messages::PrMessage),
    // Repo filter
    StartRepoFilter,
    CloseRepoFilter,
    ToggleRepoFilter(String),
    ToggleAllRepoFilter,
    ToggleRepoFilterMode,
    MoveRepoCursor(isize),
    // Filter presets
    StartSavePreset,
    SaveFilterPreset(String),
    LoadFilterPreset(String),
    StartDeletePreset,
    DeleteFilterPreset(String),
    StartDeleteRepoPath,
    DeleteRepoPath(String),
    CancelPresetInput,
    ToggleOnlyActive,
    FilterPresetsLoaded(Vec<(String, HashSet<String>, RepoFilterMode)>),
    /// Wrap-up flow messages (rebase only — PR creation is agent-driven via the
    /// `/wrap-up` skill). See [`crate::tui::messages::WrapUpMessage`].
    WrapUp(crate::tui::messages::WrapUpMessage),
    // Tips overlay
    ShowTips {
        tips: Vec<crate::tips::Tip>,
        starting_index: usize,
        max_seen_id: u32,
        show_mode: TipsShowMode,
    },
    NextTip,
    PrevTip,
    SetTipsMode(TipsShowMode),
    CloseTips,
    /// Knowledge-base overlay messages — see [`crate::tui::messages::LearningMessage`].
    Learning(crate::tui::messages::LearningMessage),
    // Project messages
    ProjectsUpdated(Vec<Project>),
    SelectProject(ProjectId),
    FollowProject(ProjectId),
    /// Feed-epic refresh messages — see [`crate::tui::messages::FeedMessage`].
    Feed(crate::tui::messages::FeedMessage),
    // Main session messages
    SubmitMainSessionDir(String),
    MainSessionCreated(String),
}

// ---------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Command {
    /// Task-domain side-effect commands — see
    /// [`crate::tui::commands::TaskCommand`].
    Task(crate::tui::commands::TaskCommand),
    // Split mode commands
    EnterSplitMode,
    EnterSplitModeWithTask {
        task_id: TaskId,
        window: String,
    },
    ExitSplitMode {
        pane_id: String,
        restore_window: Option<String>,
    },
    SwapSplitPane {
        task_id: TaskId,
        new_window: String,
        old_pane_id: Option<String>,
        old_window: Option<String>,
    },
    FocusSplitPane {
        pane_id: String,
    },
    CheckSplitPaneExists {
        pane_id: String,
    },
    RespawnSplitPane {
        pane_id: String,
    },
    /// Pop-out `$EDITOR` flow side-effect commands — see
    /// [`crate::tui::commands::EditorCommand`].
    Editor(crate::tui::commands::EditorCommand),
    SaveRepoPath(String),
    /// Epic-domain side-effect commands — see
    /// [`crate::tui::commands::EpicCommand`].
    Epic(crate::tui::commands::EpicCommand),
    /// Feed-epic refresh side-effect commands — see
    /// [`crate::tui::commands::FeedCommand`].
    Feed(crate::tui::commands::FeedCommand),
    /// System-level side-effect commands — see
    /// [`crate::tui::commands::SystemCommand`].
    System(crate::tui::commands::SystemCommand),
    PersistSetting {
        key: String,
        value: bool,
    },
    PersistStringSetting {
        key: String,
        value: String,
    },
    PersistFilterPreset {
        name: String,
        repo_paths: Vec<String>,
        mode: RepoFilterMode,
    },
    DeleteFilterPreset(String),
    DeleteRepoPath(String),
    /// PR flow side-effect commands — see [`crate::tui::commands::PrCommand`].
    Pr(crate::tui::commands::PrCommand),
    // Tips persistence
    SaveTipsState {
        seen_up_to: u32,
        show_mode: TipsShowMode,
    },
    // Project commands
    CreateProject {
        name: String,
    },
    RenameProject {
        id: ProjectId,
        name: String,
    },
    DeleteProject {
        id: ProjectId,
    },
    /// +1 = move down (higher sort_order), -1 = move up (lower sort_order)
    ReorderProject {
        id: ProjectId,
        delta: i8,
    },
    OpenMainSession,
    /// Knowledge-base overlay side-effect commands — see
    /// [`crate::tui::commands::LearningCommand`].
    Learning(crate::tui::commands::LearningCommand),
}

// ---------------------------------------------------------------------------
// InputMode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    InputTitle,
    InputDescription,
    InputRepoPath,
    InputTag,
    ConfirmDelete,
    QuickDispatch,
    ConfirmRetry(TaskId),
    /// `Some(id)` = single-task archive (ID captured when 'x' was pressed).
    /// `None` = batch archive (uses the current multi-selection set).
    ConfirmArchive(Option<TaskId>),
    ConfirmDone(TaskId),
    ConfirmMergePr(TaskId),
    ConfirmWrapUp(TaskId),
    ConfirmDetachTmux(Vec<TaskId>),
    // Epic input modes
    InputEpicTitle,
    InputEpicDescription,
    InputEpicRepoPath,
    ConfirmDeleteEpic,
    ConfirmArchiveEpic,
    ConfirmEpicWrapUp(EpicId),
    // Overlay modes
    Help,
    RepoFilter,
    InputPresetName,
    ConfirmDeletePreset,
    ConfirmDeleteRepoPath,
    ConfirmEditTask(TaskId),
    ConfirmQuit,
    InputBaseBranch,
    InputWrapUpMode,
    // Project panel input modes
    InputProjectName {
        /// None = create new project, Some(id) = rename existing
        editing_id: Option<ProjectId>,
    },
    ConfirmDeleteProject1 {
        id: ProjectId,
    },
    ConfirmDeleteProject2 {
        id: ProjectId,
        item_count: u64,
    },
    MainSessionDir,
}

// ---------------------------------------------------------------------------
// TaskDraft
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TaskDraft {
    pub title: String,
    pub description: String,
    pub repo_path: String,
    pub tag: Option<TaskTag>,
    pub base_branch: String,
    pub wrap_up_mode: Option<WrapUpMode>,
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
        }
    }
}

// ---------------------------------------------------------------------------
// BoardState — tasks, epics, view mode, and related board data
// ---------------------------------------------------------------------------

pub struct BoardState {
    pub(in crate::tui) tasks: Vec<Task>,
    pub(in crate::tui) epics: Vec<Epic>,
    pub(in crate::tui) projects: Vec<Project>,
    pub(in crate::tui) view_mode: ViewMode,
    pub(in crate::tui) repo_paths: Vec<String>,
    pub(in crate::tui) split: SplitState,
    /// Flattened rendering mode: when true, epic cards are hidden and every
    /// descendant task of the current view surfaces directly in its status
    /// column. Preserved across navigation, session-scoped.
    pub(in crate::tui) flattened: bool,
}

// ---------------------------------------------------------------------------
// StatusState — transient status messages and error popups
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct StatusState {
    pub(in crate::tui) message: Option<String>,
    pub(in crate::tui) message_set_at: Option<Instant>,
    pub(in crate::tui) error_popup: Option<String>,
    /// When true, the status message survives the [`STATUS_MESSAGE_TTL`]
    /// auto-clear in `handle_tick`. Used for in-flight dispatch feedback —
    /// the message must persist for the multi-second `git fetch` window
    /// rather than vanish mid-flight.
    pub(in crate::tui) message_sticky: bool,
}

// ---------------------------------------------------------------------------
// AgentTracking — tmux output and health state for dispatched agents
// ---------------------------------------------------------------------------

/// Per-agent tmux output and health tracking for dispatched agents. Stale
/// detection is now derived from `task.last_pre_tool_use_at` by
/// `ClassifyAgentActivity` on each tick; this struct only retains state that
/// the classifier cannot reconstruct from the database — captured tmux output,
/// notification de-duplication, PR poll cadence, message-flash decay, and
/// last-error context.
#[derive(Debug, Default)]
pub struct AgentTracking {
    pub tmux_outputs: HashMap<TaskId, String>,
    pub notified_review: HashSet<TaskId>,
    pub notified_needs_input: HashSet<TaskId>,
    pub last_pr_poll: HashMap<TaskId, Instant>,
    pub message_flash: HashMap<TaskId, Instant>,
    pub last_error: HashMap<TaskId, String>,
}

impl AgentTracking {
    pub fn new() -> Self {
        Self::default()
    }

    /// Remove all tracking state for a task.
    pub fn clear(&mut self, id: TaskId) {
        self.tmux_outputs.remove(&id);
        self.notified_review.remove(&id);
        self.notified_needs_input.remove(&id);
        self.last_pr_poll.remove(&id);
        self.message_flash.remove(&id);
        self.last_error.remove(&id);
    }
}

// ---------------------------------------------------------------------------
// InputState — current input mode and draft
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct InputState {
    pub mode: InputMode,
    pub buffer: String,
    pub task_draft: Option<TaskDraft>,
    pub epic_draft: Option<EpicDraft>,
    pub repo_cursor: usize,
    /// Tracks epic_id during quick-dispatch repo selection in epic view.
    pub pending_epic_id: Option<EpicId>,
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            mode: InputMode::Normal,
            buffer: String::new(),
            task_draft: None,
            epic_draft: None,
            repo_cursor: 0,
            pending_epic_id: None,
        }
    }
}

// ---------------------------------------------------------------------------
// ArchiveState — archive overlay state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ArchiveState {
    pub list_state: ListState,
}

// ---------------------------------------------------------------------------
// ProjectsPanelState — projects panel (leftmost hidden column)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ProjectsPanelState {
    pub list_state: ListState,
}

// ---------------------------------------------------------------------------
// SplitState — tmux split mode state
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct SplitState {
    pub(in crate::tui) active: bool,
    pub(in crate::tui) focused: bool,
    pub(in crate::tui) right_pane_id: Option<String>,
    pub(in crate::tui) pinned_task_id: Option<TaskId>,
}

impl Default for SplitState {
    fn default() -> Self {
        Self {
            active: false,
            focused: true,
            right_pane_id: None,
            pinned_task_id: None,
        }
    }
}

// ---------------------------------------------------------------------------
// SelectionState — multi-select state for batch operations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct SelectionState {
    pub tasks: HashSet<TaskId>,
    pub epics: HashSet<EpicId>,
    pub pending_done: Vec<TaskId>,
}

impl SelectionState {
    pub fn has_selection(&self) -> bool {
        !self.tasks.is_empty() || !self.epics.is_empty()
    }

    pub fn clear(&mut self) {
        self.tasks.clear();
        self.epics.clear();
    }
}

// ---------------------------------------------------------------------------
// FilterState — repo filter and presets for the task board
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct FilterState {
    pub repos: HashSet<String>,
    pub mode: RepoFilterMode,
    pub presets: Vec<(String, HashSet<String>, RepoFilterMode)>,
    pub only_active: bool,
}

impl FilterState {
    pub fn matches(&self, repo_path: &str) -> bool {
        if self.repos.is_empty() {
            return true;
        }
        match self.mode {
            RepoFilterMode::Include => self.repos.contains(repo_path),
            RepoFilterMode::Exclude => !self.repos.contains(repo_path),
        }
    }

    /// Returns false when `only_active` is set and the task has no tmux window.
    pub fn task_matches(&self, task: &crate::models::Task) -> bool {
        !self.only_active || task.tmux_window.is_some()
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
}

// ---------------------------------------------------------------------------
// BoardSelection — column + row selection state for a kanban view
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct BoardSelection {
    pub(in crate::tui) selected_column: usize,
    pub(in crate::tui) selected_row: [usize; TaskStatus::COLUMN_COUNT],
    pub(in crate::tui) on_select_all: bool,
    pub(in crate::tui) list_states: [ListState; TaskStatus::COLUMN_COUNT],
    pub(in crate::tui) anchor: Option<ColumnAnchor>,
    pub(in crate::tui) projects_row: usize,
    pub(in crate::tui) archive_row: usize,
}

impl BoardSelection {
    pub fn new() -> Self {
        Self {
            selected_column: 0,
            selected_row: [0; TaskStatus::COLUMN_COUNT],
            on_select_all: false,
            list_states: std::array::from_fn(|_| ListState::default()),
            anchor: None,
            projects_row: 0,
            archive_row: 0,
        }
    }

    /// Main board starts at Backlog (nav col 1), to the right of the Projects edge column.
    pub fn new_for_board() -> Self {
        Self {
            selected_column: 1,
            ..Self::new()
        }
    }

    pub fn new_for_epic() -> Self {
        Self {
            selected_column: 1,
            ..Self::new()
        }
    }

    pub fn column(&self) -> usize {
        self.selected_column
    }

    /// Row cursor for the given navigation column.
    /// nav col 0 → projects_row, nav col 1–4 → selected_row[nav_col-1], nav col 5 → archive_row.
    pub fn row(&self, col: usize) -> usize {
        match col {
            0 => self.projects_row,
            1..=4 => self.selected_row[col - 1],
            5 => self.archive_row,
            _ => 0,
        }
    }

    pub fn set_column(&mut self, col: usize) {
        self.selected_column = col;
    }

    pub fn set_row(&mut self, col: usize, row: usize) {
        match col {
            0 => self.projects_row = row,
            1..=4 => self.selected_row[col - 1] = row,
            5 => self.archive_row = row,
            _ => {}
        }
    }

    /// List item index for `ListState` scrolling.
    /// Returns `None` when the cursor is on the select-all toggle (header),
    /// since no list item should be selected in that case.
    pub fn list_state_index(&self, col: usize) -> Option<usize> {
        if self.on_select_all {
            None
        } else {
            Some(self.row(col))
        }
    }
}

impl Default for BoardSelection {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// ViewMode — board vs epic view with preserved selection state
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ViewMode {
    Board(BoardSelection),
    Epic {
        epic_id: EpicId,
        selection: BoardSelection,
        /// The view to restore when exiting this epic.
        /// For a root epic entered from the board, this is `ViewMode::Board(...)`.
        /// For a nested sub-epic, this is `ViewMode::Epic { ... }` of the parent.
        parent: Box<ViewMode>,
    },
    TaskDetail {
        task_id: TaskId,
        scroll: u16,
        zoomed: bool,
        /// Scroll limit — updated by the renderer each frame from the actual wrapped line count.
        /// Do not treat this as authoritative input state; it is renderer-managed.
        max_scroll: u16,
        previous: Box<ViewMode>,
    },
    Learnings {
        selected: usize,
        learnings: Vec<crate::models::Learning>,
        view: LearningsView,
        // RefCell allows render_stateful_widget to borrow_mut without &mut App
        tree_state: std::cell::RefCell<tui_tree_widget::TreeState<String>>,
        previous: Box<ViewMode>,
    },
}

impl Clone for ViewMode {
    fn clone(&self) -> Self {
        match self {
            ViewMode::Board(sel) => ViewMode::Board(sel.clone()),
            ViewMode::Epic {
                epic_id,
                selection,
                parent,
            } => ViewMode::Epic {
                epic_id: *epic_id,
                selection: selection.clone(),
                parent: parent.clone(),
            },
            ViewMode::TaskDetail {
                task_id,
                scroll,
                zoomed,
                max_scroll,
                previous,
            } => ViewMode::TaskDetail {
                task_id: *task_id,
                scroll: *scroll,
                zoomed: *zoomed,
                max_scroll: *max_scroll,
                previous: previous.clone(),
            },
            ViewMode::Learnings {
                selected,
                learnings,
                view,
                previous,
                // TreeState does not implement Clone — create a fresh one on clone.
                tree_state: _,
            } => ViewMode::Learnings {
                selected: *selected,
                learnings: learnings.clone(),
                view: *view,
                tree_state: std::cell::RefCell::new(tui_tree_widget::TreeState::default()),
                previous: previous.clone(),
            },
        }
    }
}

impl ViewMode {
    pub(in crate::tui) fn selection(&self) -> &BoardSelection {
        match self {
            ViewMode::Board(sel) => sel,
            ViewMode::Epic { selection, .. } => selection,
            ViewMode::TaskDetail { previous, .. } => previous.selection(),
            ViewMode::Learnings { previous, .. } => previous.selection(),
        }
    }

    pub(in crate::tui) fn selection_mut(&mut self) -> &mut BoardSelection {
        match self {
            ViewMode::Board(sel) => sel,
            ViewMode::Epic { selection, .. } => selection,
            ViewMode::TaskDetail { previous, .. } => previous.selection_mut(),
            ViewMode::Learnings { previous, .. } => previous.selection_mut(),
        }
    }
}

impl Default for ViewMode {
    fn default() -> Self {
        ViewMode::Board(BoardSelection::new_for_board())
    }
}

// ---------------------------------------------------------------------------
// ColumnItem — resolves whether cursor is on a task or an epic
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum ColumnItem<'a> {
    Task(&'a Task),
    Epic(&'a Epic),
    /// Non-selectable group header in flat view. Carries the epic so the renderer
    /// can read its title without an extra lookup.
    EpicHeader(&'a Epic),
    /// Non-selectable substatus section header, pre-built by the flat-view path of
    /// `column_items_for_status_with_stats`. Only produced in flat view for
    /// Running and Review columns. The renderer must not also inject its own
    /// substatus header for the same group transition.
    SubstatusLabel(&'static str),
}

impl ColumnItem<'_> {
    /// Returns `true` for `Task` and `Epic` items that can hold the cursor.
    /// `EpicHeader` and `SubstatusLabel` are decorative and non-selectable.
    pub fn is_selectable(&self) -> bool {
        matches!(self, ColumnItem::Task(_) | ColumnItem::Epic(_))
    }
}

// ---------------------------------------------------------------------------
// ColumnAnchor — identity of the currently-selected task-board item
// ---------------------------------------------------------------------------

/// Identifies which item the cursor is anchored to across column refreshes.
/// Task and Epic IDs come from separate SQLite sequences and can overlap,
/// so we use a discriminated enum rather than a bare i64.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnAnchor {
    Task(crate::models::TaskId),
    Epic(crate::models::EpicId),
}

// ---------------------------------------------------------------------------
// ColumnLayout — pre-computed column items for one render frame
// ---------------------------------------------------------------------------

/// Pre-computed column items for one render frame.
/// Built once at the top of `render()` to avoid recomputing per widget.
pub struct ColumnLayout<'a> {
    columns: [Vec<ColumnItem<'a>>; TaskStatus::COLUMN_COUNT],
}

impl<'a> ColumnLayout<'a> {
    pub fn build(app: &'a super::App, stats: &EpicStatsMap) -> Self {
        let columns = std::array::from_fn(|i| {
            let status = TaskStatus::ALL[i];
            app.column_items_for_status_with_stats(status, Some(stats))
        });
        ColumnLayout { columns }
    }

    pub fn get(&self, status: TaskStatus) -> &[ColumnItem<'a>] {
        &self.columns[status.column_index()]
    }

    pub fn count(&self, status: TaskStatus) -> usize {
        self.columns[status.column_index()].len()
    }
}

// ---------------------------------------------------------------------------
// EpicDraft — fields collected during epic creation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct EpicDraft {
    pub title: String,
    pub description: String,
    pub repo_path: String,
    pub parent_epic_id: Option<EpicId>,
}

// ---------------------------------------------------------------------------
// MergeQueue — state for batch epic wrap-up
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct MergeQueue {
    pub epic_id: EpicId,
    pub task_ids: Vec<TaskId>,
    pub completed: usize,
    pub current: Option<TaskId>,
    pub failed: Option<TaskId>,
}

// ---------------------------------------------------------------------------
// SubtaskStats — pre-computed per-epic subtask status counts
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SubtaskStats {
    pub backlog: usize,
    pub running: usize,
    pub review: usize,
    pub done: usize,
    pub total: usize,
    pub substatus: EpicSubstatus,
}

impl SubtaskStats {
    /// Compute stats for a single epic from its non-archived subtasks,
    /// including tasks owned by any descendant sub-epics.
    pub fn for_epic(
        epic: &Epic,
        all_tasks: &[Task],
        all_epics: &[Epic],
        active_merge_epic: Option<EpicId>,
    ) -> Self {
        let epic_ids = crate::models::descendant_epic_ids(epic.id, all_epics);
        let subtasks: Vec<&Task> = all_tasks
            .iter()
            .filter(|t| {
                matches!(t.epic_id, Some(eid) if epic_ids.contains(&eid))
                    && t.status != TaskStatus::Archived
            })
            .collect();

        let mut backlog = 0;
        let mut running = 0;
        let mut review = 0;
        let mut done = 0;
        for t in &subtasks {
            match t.status {
                TaskStatus::Backlog => backlog += 1,
                TaskStatus::Running => running += 1,
                TaskStatus::Review => review += 1,
                TaskStatus::Done => done += 1,
                TaskStatus::Archived => {}
            }
        }

        // epic_substatus needs owned tasks — collect only the refs we already have
        let owned: Vec<Task> = subtasks.iter().map(|t| (*t).clone()).collect();
        let substatus = crate::models::epic_substatus(epic, &owned, active_merge_epic);

        SubtaskStats {
            backlog,
            running,
            review,
            done,
            total: backlog + running + review + done,
            substatus,
        }
    }
}

/// Pre-computed subtask stats for all epics, keyed by EpicId.
pub type EpicStatsMap = HashMap<EpicId, SubtaskStats>;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::models::{ProjectId, SubStatus, TaskId};
    use chrono::Utc;

    fn make_test_epic(id: i64, parent: Option<i64>) -> Epic {
        Epic {
            id: EpicId(id),
            title: format!("Epic {id}"),
            description: String::new(),
            repo_path: "/repo".to_string(),
            status: TaskStatus::Running,
            plan_path: None,
            sort_order: None,
            auto_dispatch: false,
            parent_epic_id: parent.map(EpicId),
            feed_command: None,
            feed_interval_secs: None,
            group_by_repo: false,
            project_id: ProjectId(1),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_test_task(id: i64, status: TaskStatus, epic: Option<i64>) -> Task {
        Task {
            id: TaskId(id),
            title: format!("Task {id}"),
            description: String::new(),
            repo_path: "/repo".to_string(),
            status,
            sub_status: SubStatus::None,
            worktree: None,
            tmux_window: None,
            plan_path: None,
            epic_id: epic.map(EpicId),
            pr_url: None,
            tag: None,
            sort_order: None,
            base_branch: "main".to_string(),
            external_id: None,
            labels: Vec::new(),
            project_id: ProjectId(1),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_pre_tool_use_at: None,
            last_notification_at: None,
            wrap_up_mode: None,
        }
    }

    // -- SubtaskStats --

    #[test]
    fn subtask_stats_counts_direct_tasks_only_without_nested_epics() {
        let epics = vec![make_test_epic(1, None)];
        let tasks = vec![
            make_test_task(1, TaskStatus::Running, Some(1)),
            make_test_task(2, TaskStatus::Done, Some(1)),
        ];
        let stats = SubtaskStats::for_epic(&epics[0], &tasks, &epics, None);
        assert_eq!(stats.running, 1);
        assert_eq!(stats.done, 1);
        assert_eq!(stats.total, 2);
    }

    #[test]
    fn subtask_stats_includes_tasks_from_nested_sub_epics() {
        let epics = vec![make_test_epic(1, None), make_test_epic(2, Some(1))];
        let tasks = vec![
            make_test_task(1, TaskStatus::Backlog, Some(1)),
            make_test_task(2, TaskStatus::Running, Some(2)),
            make_test_task(3, TaskStatus::Done, Some(2)),
        ];
        let stats = SubtaskStats::for_epic(&epics[0], &tasks, &epics, None);
        assert_eq!(stats.backlog, 1);
        assert_eq!(stats.running, 1);
        assert_eq!(stats.done, 1);
        assert_eq!(stats.total, 3);
    }

    #[test]
    fn subtask_stats_includes_tasks_from_deeply_nested_epics() {
        let epics = vec![
            make_test_epic(1, None),
            make_test_epic(2, Some(1)),
            make_test_epic(3, Some(2)),
        ];
        let tasks = vec![make_test_task(1, TaskStatus::Running, Some(3))];
        let stats = SubtaskStats::for_epic(&epics[0], &tasks, &epics, None);
        assert_eq!(stats.running, 1);
        assert_eq!(stats.total, 1);
    }

    #[test]
    fn subtask_stats_excludes_archived_tasks_from_nested_epics() {
        let epics = vec![make_test_epic(1, None), make_test_epic(2, Some(1))];
        let tasks = vec![
            make_test_task(1, TaskStatus::Running, Some(1)),
            make_test_task(2, TaskStatus::Archived, Some(2)),
        ];
        let stats = SubtaskStats::for_epic(&epics[0], &tasks, &epics, None);
        assert_eq!(stats.running, 1);
        assert_eq!(stats.total, 1);
    }

    // -- RepoFilterMode --

    #[test]
    fn repo_filter_mode_as_str() {
        assert_eq!(RepoFilterMode::Include.as_str(), "include");
        assert_eq!(RepoFilterMode::Exclude.as_str(), "exclude");
    }

    #[test]
    fn repo_filter_mode_from_str_roundtrip() {
        for mode in [RepoFilterMode::Include, RepoFilterMode::Exclude] {
            let s = mode.as_str();
            let parsed: RepoFilterMode = s.parse().unwrap();
            assert_eq!(parsed, mode);
        }
    }

    #[test]
    fn repo_filter_mode_from_str_invalid() {
        assert!("bogus".parse::<RepoFilterMode>().is_err());
        assert!("".parse::<RepoFilterMode>().is_err());
        assert!("Include".parse::<RepoFilterMode>().is_err());
    }

    #[test]
    fn repo_filter_mode_default_is_include() {
        assert_eq!(RepoFilterMode::default(), RepoFilterMode::Include);
    }

    // -- repo_filter_matches --

    #[test]
    fn repo_filter_matches_empty_filter_matches_any_repo() {
        let filter = HashSet::new();
        assert!(repo_filter_matches(
            &filter,
            RepoFilterMode::Include,
            "org/any"
        ));
        assert!(repo_filter_matches(
            &filter,
            RepoFilterMode::Exclude,
            "org/any"
        ));
    }

    #[test]
    fn repo_filter_matches_include_mode() {
        let filter: HashSet<String> = ["org/a".to_string()].into();
        assert!(repo_filter_matches(
            &filter,
            RepoFilterMode::Include,
            "org/a"
        ));
        assert!(!repo_filter_matches(
            &filter,
            RepoFilterMode::Include,
            "org/b"
        ));
    }

    #[test]
    fn repo_filter_matches_exclude_mode() {
        let filter: HashSet<String> = ["org/a".to_string()].into();
        assert!(!repo_filter_matches(
            &filter,
            RepoFilterMode::Exclude,
            "org/a"
        ));
        assert!(repo_filter_matches(
            &filter,
            RepoFilterMode::Exclude,
            "org/b"
        ));
    }
}
