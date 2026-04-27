use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use ratatui::widgets::ListState;

use crate::models::{
    AlertKind, DispatchMode, Epic, EpicId, EpicSubstatus, Project, ProjectId, SubStatus, Task,
    TaskId, TaskStatus, TaskTag, TaskUsage, TipsShowMode, DEFAULT_BASE_BRANCH,
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
// ReviewAgentRequest
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ReviewAgentRequest {
    pub repo: String,
    pub github_repo: String,
    pub number: i64,
    pub head_ref: String,
    pub is_dependabot: bool,
}

// ---------------------------------------------------------------------------
// FixAgentRequest
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct FixAgentRequest {
    pub repo: String,
    pub github_repo: String,
    pub number: i64,
    pub kind: AlertKind,
    pub title: String,
    pub description: String,
    pub package: Option<String>,
    pub fixed_version: Option<String>,
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
// Message
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    TerminalResized,
    FocusChanged(bool),
    Quit,
    NavigateColumn(isize),
    NavigateRow(isize),
    MoveTask {
        id: TaskId,
        direction: MoveDirection,
    },
    ReorderItem(isize), // +1 = down, -1 = up
    DispatchTask(TaskId, DispatchMode),
    Dispatched {
        id: TaskId,
        worktree: String,
        tmux_window: String,
        switch_focus: bool,
    },
    TaskCreated {
        task: Task,
    },
    DeleteTask(TaskId),
    ToggleDetail,
    ToggleFlattened,
    TmuxOutput {
        id: TaskId,
        output: String,
        activity_ts: u64,
    },
    WindowGone(TaskId),
    RefreshTasks(Vec<Task>),
    ResumeTask(TaskId),
    Resumed {
        id: TaskId,
        tmux_window: String,
    },
    Error(String),
    DispatchFailed(TaskId),
    MarkDispatching(TaskId),
    TaskEdited(TaskEdit),
    RepoPathsUpdated(Vec<String>),
    QuickDispatch {
        repo_path: String,
        epic_id: Option<EpicId>,
    },
    StaleAgent(TaskId),
    AgentCrashed(TaskId),
    KillAndRetry(TaskId),
    RetryResume(TaskId),
    RetryFresh(TaskId),
    ArchiveTask(TaskId),
    ToggleSelect(TaskId),
    ToggleSelectEpic(EpicId),
    ClearSelection,
    SelectAllColumn,
    BatchMoveTasks {
        ids: Vec<TaskId>,
        direction: MoveDirection,
    },
    BatchArchiveTasks(Vec<TaskId>),
    BatchArchiveEpics(Vec<EpicId>),
    // Input routing messages
    DismissError,
    StartNewTask,
    CopyTask,
    CancelInput,
    ConfirmDeleteStart,
    ConfirmDeleteYes,
    CancelDelete,
    SubmitTitle(String),
    SubmitDescription(String),
    DescriptionEditorResult(String),
    /// Result from an editor popped out into a tmux window. Carries the
    /// tag identifying which edit was in flight and the editor outcome.
    EditorResult {
        kind: EditKind,
        outcome: EditorOutcome,
    },
    SubmitRepoPath(String),
    SubmitTag(Option<TaskTag>),
    SubmitBaseBranch(String),
    InputChar(char),
    InputBackspace,
    StartQuickDispatchSelection,
    SelectQuickDispatchRepo(usize),
    CancelRetry,
    StatusInfo(String),
    ToggleHelp,
    // Split mode messages
    ToggleSplitMode,
    SwapSplitPane(TaskId),
    SplitPaneOpened {
        pane_id: String,
        task_id: Option<TaskId>,
    },
    SplitPaneClosed,
    // Epic messages
    DispatchEpic(EpicId),
    EnterEpic(EpicId),
    ExitEpic,
    RefreshEpics(Vec<Epic>),
    RefreshUsage(Vec<TaskUsage>),
    EpicCreated(Epic),
    EditEpic(EpicId),
    EpicEdited(Epic),
    DeleteEpic(EpicId),
    ToggleEpicAutoDispatch(EpicId),
    ConfirmDeleteEpic,
    MoveEpicStatus(EpicId, MoveDirection),
    ArchiveEpic(EpicId),
    ConfirmArchiveEpic,
    StartNewEpic,
    SubmitEpicTitle(String),
    SubmitEpicDescription(String),
    SubmitEpicRepoPath(String),
    // Finish (rebase + cleanup)
    FinishComplete(TaskId),
    FinishFailed {
        id: TaskId,
        error: String,
        is_conflict: bool,
    },
    // PR flow
    PrCreated {
        id: TaskId,
        pr_url: String,
    },
    PrFailed {
        id: TaskId,
        error: String,
    },
    PrMerged(TaskId),
    StartMergePr(TaskId),
    ConfirmMergePr,
    CancelMergePr,
    MergePrFailed {
        id: TaskId,
        error: String,
    },
    PrReviewState {
        id: TaskId,
        review_decision: Option<crate::models::ReviewDecision>,
    },
    // Done confirmation (no cleanup, just status change)
    ConfirmDone,
    CancelDone,
    ToggleNotifications,
    TabCycle,
    OpenInBrowser {
        url: String,
    },
    // Repo filter
    StartRepoFilter,
    CloseRepoFilter,
    ToggleRepoFilter(String),
    ToggleAllRepoFilter,
    MoveRepoCursor(isize),
    ToggleRepoFilterMode,
    // Filter presets
    StartSavePreset,
    SaveFilterPreset(String),
    LoadFilterPreset(String),
    StartDeletePreset,
    DeleteFilterPreset(String),
    StartDeleteRepoPath,
    DeleteRepoPath(String),
    CancelPresetInput,
    FilterPresetsLoaded(Vec<(String, HashSet<String>, RepoFilterMode)>),
    // Wrap up (replaces finish + PR)
    StartWrapUp(TaskId),
    WrapUpRebase,
    WrapUpPr,
    CancelWrapUp,
    // Epic batch wrap-up
    StartEpicWrapUp(EpicId),
    EpicWrapUpRebase,
    EpicWrapUpPr,
    CancelEpicWrapUp,
    CancelMergeQueue,
    // Detach tmux panel (Review tasks only)
    DetachTmux(TaskId),
    BatchDetachTmux(Vec<TaskId>),
    ConfirmDetachTmux,
    // Inter-agent messaging
    MessageReceived(TaskId),
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
    // Project messages
    ProjectsUpdated(Vec<Project>),
    SelectProject(ProjectId),
    OpenProjectsPanel,
    CloseProjectsPanel,
}

// ---------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Command {
    PersistTask(Task),
    InsertTask {
        draft: TaskDraft,
        epic_id: Option<EpicId>,
    },
    DeleteTask(TaskId),
    DispatchAgent {
        task: Task,
        mode: DispatchMode,
    },
    Cleanup {
        id: TaskId,
        repo_path: String,
        worktree: String,
        tmux_window: Option<String>,
    },
    Finish {
        id: TaskId,
        repo_path: String,
        branch: String,
        base_branch: String,
        worktree: String,
        tmux_window: Option<String>,
    },
    CaptureTmux {
        id: TaskId,
        window: String,
    },
    Resume {
        task: Task,
    },
    JumpToTmux {
        window: String,
    },
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
    KillTmuxWindow {
        window: String,
    },
    /// Launch $EDITOR in a new tmux window. The `kind` decides both what
    /// to put in the initial file and what post-processing to apply when
    /// the editor closes.
    PopOutEditor(EditKind),
    /// Finalize an editor session: apply the user's edits (if any) to
    /// the database via the appropriate service. Dispatches on the
    /// [`EditKind`] to reach the right code path.
    FinalizeEditorResult {
        kind: EditKind,
        outcome: EditorOutcome,
    },
    SaveRepoPath(String),
    RefreshFromDb,
    QuickDispatch {
        draft: TaskDraft,
        epic_id: Option<EpicId>,
    },
    // Epic commands
    DispatchEpic {
        epic: Epic,
    },
    InsertEpic(EpicDraft),
    DeleteEpic(EpicId),
    PersistEpic {
        id: EpicId,
        status: Option<TaskStatus>,
        sort_order: Option<i64>,
    },
    ToggleEpicAutoDispatch {
        id: EpicId,
        auto_dispatch: bool,
    },
    RefreshEpicsFromDb,
    SendNotification {
        title: String,
        body: String,
        urgent: bool,
    },
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
    CreatePr {
        id: TaskId,
        repo_path: String,
        branch: String,
        base_branch: String,
        title: String,
        description: String,
    },
    CheckPrStatus {
        id: TaskId,
        pr_url: String,
    },
    MergePr {
        id: TaskId,
        pr_url: String,
    },
    OpenInBrowser {
        url: String,
    },
    PatchSubStatus {
        id: TaskId,
        sub_status: SubStatus,
    },
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
}

impl Default for TaskDraft {
    fn default() -> Self {
        Self {
            title: String::new(),
            description: String::new(),
            repo_path: String::new(),
            tag: None,
            base_branch: DEFAULT_BASE_BRANCH.to_string(),
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
    pub(in crate::tui) detail_visible: bool,
    pub(in crate::tui) repo_paths: Vec<String>,
    pub(in crate::tui) usage: HashMap<TaskId, TaskUsage>,
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
}

// ---------------------------------------------------------------------------
// AgentTracking — tmux output and health state for dispatched agents
// ---------------------------------------------------------------------------

/// Per-agent tmux output and health tracking for dispatched agents.
///
/// `last_active_at` records the wall-clock [`Instant`] when each agent was last
/// known to be active (dispatched, resumed, or produced tmux output). Used for
/// stale detection — if the elapsed time exceeds `inactivity_timeout`, the task
/// is marked stale.
///
/// `prev_tmux_activity` caches the most recent tmux `window_activity` timestamp
/// so we can detect genuine new activity vs. a re-poll returning the same value.
#[derive(Debug)]
pub struct AgentTracking {
    pub tmux_outputs: HashMap<TaskId, String>,
    pub last_active_at: HashMap<TaskId, Instant>,
    pub prev_tmux_activity: HashMap<TaskId, u64>,
    pub inactivity_timeout: Duration,
    pub notified_review: HashSet<TaskId>,
    pub notified_needs_input: HashSet<TaskId>,
    pub last_pr_poll: HashMap<TaskId, Instant>,
    pub message_flash: HashMap<TaskId, Instant>,
    pub last_error: HashMap<TaskId, String>,
}

impl AgentTracking {
    pub fn new(inactivity_timeout: Duration) -> Self {
        Self {
            tmux_outputs: HashMap::new(),
            last_active_at: HashMap::new(),
            prev_tmux_activity: HashMap::new(),
            inactivity_timeout,
            notified_review: HashSet::new(),
            notified_needs_input: HashSet::new(),
            last_pr_poll: HashMap::new(),
            message_flash: HashMap::new(),
            last_error: HashMap::new(),
        }
    }

    /// Record that the agent for `id` is active right now.
    pub fn mark_active(&mut self, id: TaskId) {
        self.last_active_at.insert(id, Instant::now());
    }

    /// How long since the agent for `id` was last active, if known.
    pub fn inactive_duration(&self, id: TaskId) -> Option<Duration> {
        self.last_active_at.get(&id).map(|t| t.elapsed())
    }

    /// Remove all tracking state for a task.
    pub fn clear(&mut self, id: TaskId) {
        self.last_active_at.remove(&id);
        self.prev_tmux_activity.remove(&id);
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
    pub selected_row: usize,
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

    pub fn new_for_epic() -> Self {
        Self { selected_column: 1, ..Self::new() }
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
        if self.on_select_all { None } else { Some(self.row(col)) }
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

#[derive(Debug, Clone)]
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
}

impl Default for ViewMode {
    fn default() -> Self {
        ViewMode::Board(BoardSelection::new())
    }
}

// ---------------------------------------------------------------------------
// ColumnItem — resolves whether cursor is on a task or an epic
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum ColumnItem<'a> {
    Task(&'a Task),
    Epic(&'a Epic),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeAction {
    Rebase,
    Pr,
}

#[derive(Debug, Clone)]
pub struct MergeQueue {
    pub epic_id: EpicId,
    pub action: MergeAction,
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
    /// Compute stats for a single epic from its non-archived subtasks.
    pub fn for_epic(epic: &Epic, all_tasks: &[Task], active_merge_epic: Option<EpicId>) -> Self {
        let subtasks: Vec<&Task> = all_tasks
            .iter()
            .filter(|t| t.epic_id == Some(epic.id) && t.status != TaskStatus::Archived)
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
    use super::*;

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
