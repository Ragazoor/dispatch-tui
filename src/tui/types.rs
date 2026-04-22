use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::{Duration, Instant};

use ratatui::widgets::ListState;

use crate::models::{
    AlertKind, AlertSeverity, DispatchMode, Epic, EpicId, EpicSubstatus, PrRef, ReviewAgentStatus,
    ReviewDecision, SecurityAlert, SubStatus, Task, TaskId, TaskStatus, TaskTag, TaskUsage,
    TipsShowMode, DEFAULT_BASE_BRANCH,
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
// ReviewBoardMode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewBoardMode {
    Reviewer,
    Author,
}

impl ReviewBoardMode {
    pub fn column_count(&self) -> usize {
        4
    }

    pub fn column_label(&self, col: usize) -> &'static str {
        match col {
            0 => "Needs Review",
            1 => "Waiting for Response",
            2 => "Changes Requested",
            3 => "Approved",
            _ => "",
        }
    }

    pub fn pr_column(&self, pr: &crate::models::ReviewPr) -> usize {
        pr.review_decision.column_index()
    }
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
// FixDispatchKey — newtype for in-flight fix agent dispatch deduplication
// ---------------------------------------------------------------------------

/// Identifies a fix agent dispatch in-flight, keyed by repo, alert number, and kind.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FixDispatchKey {
    pub repo: String,
    pub number: i64,
    pub kind: AlertKind,
}

impl FixDispatchKey {
    pub fn new(repo: String, number: i64, kind: AlertKind) -> Self {
        Self { repo, number, kind }
    }
}

// ---------------------------------------------------------------------------
// ReviewAgentHandle — execution state for a dispatched review agent
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ReviewAgentHandle {
    pub tmux_window: String,
    pub worktree: String,
    pub status: ReviewAgentStatus,
}

// ---------------------------------------------------------------------------
// FixAgentHandle — execution state for a dispatched fix agent
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct FixAgentHandle {
    pub tmux_window: String,
    pub worktree: String,
    pub status: ReviewAgentStatus,
}

// ---------------------------------------------------------------------------
// PendingDispatch — held while user selects a repo path for dispatch
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum PendingDispatch {
    Review(ReviewAgentRequest),
    Fix(FixAgentRequest),
}

impl PendingDispatch {
    pub fn github_repo(&self) -> &str {
        match self {
            PendingDispatch::Review(req) => &req.github_repo,
            PendingDispatch::Fix(req) => &req.github_repo,
        }
    }
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
    ToggleArchive,
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
    SubmitRepoPath(String),
    SubmitDispatchRepoPath(String),
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
    // Review board
    SwitchToReviewBoard,
    SwitchToTaskBoard,
    SwitchReviewBoardMode(ReviewBoardMode),
    PrsLoaded(PrListKind, Vec<crate::models::ReviewPr>),
    PrsFetchFailed(PrListKind, String),
    OpenInBrowser {
        url: String,
    },
    ToggleReviewDetail,
    RefreshReviewPrs,
    DispatchReviewAgent(ReviewAgentRequest),
    ReviewAgentDispatched {
        github_repo: String,
        number: i64,
        tmux_window: String,
        worktree: String,
    },
    ReviewAgentFailed {
        github_repo: String,
        number: i64,
        error: String,
    },
    // Repo filter
    StartRepoFilter,
    CloseRepoFilter,
    ToggleRepoFilter(String),
    ToggleAllRepoFilter,
    MoveRepoCursor(isize),
    ToggleRepoFilterMode,
    // Review repo filter
    StartReviewRepoFilter,
    CloseReviewRepoFilter,
    ToggleReviewRepoFilter(String),
    ToggleAllReviewRepoFilter,
    ToggleReviewRepoFilterMode,
    // Dispatch PR filter (My PRs tab)
    ToggleDispatchPrFilter,
    // Bot PRs (dependabot/renovate)
    RefreshBotPrs,
    BotPrsMerged(Vec<String>),
    ToggleSelectBotPr(String),
    ClearBotPrSelection,
    // Bot PR repo filter
    StartBotPrRepoFilter,
    CloseBotPrRepoFilter,
    ToggleBotPrRepoFilter(String),
    ToggleAllBotPrRepoFilter,
    ToggleBotPrRepoFilterMode,
    // Security board
    SwitchToSecurityBoard,
    SecurityAlertsLoaded(Vec<SecurityAlert>),
    SecurityAlertsFetchFailed(String),
    SecurityAlertsUnconfigured,
    RefreshSecurityAlerts,
    ToggleSecurityDetail,
    ToggleSecurityKindFilter,
    StartSecurityRepoFilter,
    CloseSecurityRepoFilter,
    ToggleSecurityRepoFilter(String),
    ToggleAllSecurityRepoFilter,
    ToggleSecurityRepoFilterMode,
    DispatchFixAgent(FixAgentRequest),
    FixAgentDispatched {
        github_repo: String,
        number: i64,
        kind: crate::models::AlertKind,
        tmux_window: String,
        worktree: String,
    },
    FixAgentFailed {
        github_repo: String,
        number: i64,
        kind: crate::models::AlertKind,
        error: String,
    },
    ReviewStatusUpdated {
        repo: String,
        number: i64,
        status: crate::models::ReviewAgentStatus,
    },
    DetachReviewAgent {
        repo: String,
        number: i64,
    },
    DetachFixAgent {
        repo: String,
        number: i64,
        kind: crate::models::AlertKind,
    },
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
    // Security board sub-mode switching
    SwitchSecurityBoardMode(SecurityBoardMode),
    // Dependabot approve/merge
    StartApproveBotPr,
    StartMergeBotPr,
    ConfirmApproveBotPr,
    ConfirmMergeBotPr,
    CancelPrOperation,
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
}

// ---------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Command {
    PersistTask(Task),
    PersistReviewAgent {
        pr_kind: crate::db::PrKind,
        github_repo: String,
        number: i64,
        tmux_window: String,
        worktree: String,
    },
    PersistFixAgent {
        github_repo: String,
        number: i64,
        kind: crate::models::AlertKind,
        tmux_window: String,
        worktree: String,
    },
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
    EditTaskInEditor(Task),
    OpenDescriptionEditor {
        is_epic: bool,
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
    EditEpicInEditor(Epic),
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
    FetchPrs(PrListKind),
    PersistPrs(PrListKind, Vec<crate::models::ReviewPr>),
    ApproveBotPr(String),
    MergeBotPr(String),
    OpenInBrowser {
        url: String,
    },
    PatchSubStatus {
        id: TaskId,
        sub_status: SubStatus,
    },
    DispatchReviewAgent(ReviewAgentRequest),
    FetchSecurityAlerts,
    PersistSecurityAlerts(Vec<SecurityAlert>),
    DispatchFixAgent(FixAgentRequest),
    EditGithubQueries(PrListKind),
    UpdateAgentStatus {
        repo: String,
        number: i64,
        status: Option<crate::models::ReviewAgentStatus>,
    },
    ReReview {
        repo: String,
        number: i64,
        tmux_window: String,
    },
    // Tips persistence
    SaveTipsState {
        seen_up_to: u32,
        show_mode: TipsShowMode,
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
    ConfirmArchive,
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
    ReviewRepoFilter,
    BotPrRepoFilter,
    SecurityRepoFilter,
    InputPresetName,
    ConfirmDeletePreset,
    ConfirmDeleteRepoPath,
    ConfirmEditTask(TaskId),
    // Dependabot approve/merge confirmation
    ConfirmApproveBotPr(String),
    ConfirmMergeBotPr(String),
    ConfirmQuit,
    // Dispatch repo path input (review/security tab fallback)
    InputDispatchRepoPath,
    InputBaseBranch,
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
    /// Holds a pending review/fix dispatch while user selects a repo path.
    pub pending_dispatch: Option<PendingDispatch>,
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
            pending_dispatch: None,
        }
    }
}

// ---------------------------------------------------------------------------
// ArchiveState — archive overlay state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ArchiveState {
    pub visible: bool,
    pub selected_row: usize,
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
// PrListKind — discriminator for the three PR lists
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrListKind {
    Review,
    Authored,
    Bot,
}

impl PrListKind {
    /// Settings key for the GitHub query strings.
    pub fn settings_key(self) -> &'static str {
        match self {
            Self::Review => "github_queries_review",
            Self::Authored => "github_queries_my_prs",
            Self::Bot => "github_queries_bot",
        }
    }

    /// Database table name.
    pub fn table_name(self) -> &'static str {
        self.to_pr_kind().table_name()
    }

    pub fn to_pr_kind(self) -> crate::db::PrKind {
        match self {
            Self::Review => crate::db::PrKind::Review,
            Self::Authored => crate::db::PrKind::My,
            Self::Bot => crate::db::PrKind::Bot,
        }
    }

    /// Human-readable label for log messages.
    pub fn label(self) -> &'static str {
        match self {
            Self::Review => "review",
            Self::Authored => "my",
            Self::Bot => "bot",
        }
    }
}

// ---------------------------------------------------------------------------
// PrListState — per-list state shared by review / authored / bot PR lists
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct PrListState {
    pub prs: Vec<crate::models::ReviewPr>,
    pub repos: Vec<String>,
    pub loading: bool,
    pub last_fetch: Option<Instant>,
    pub last_error: Option<String>,
    pub repo_filter: HashSet<String>,
    pub repo_filter_mode: RepoFilterMode,
}

impl PrListState {
    /// Replace the PR list and rebuild the cached distinct repos list.
    pub fn set_prs(&mut self, prs: Vec<crate::models::ReviewPr>) {
        self.repos = distinct_repos(&prs);
        self.prs = prs;
    }

    /// Return PRs filtered by repo filter. Empty filter means all PRs.
    pub fn filtered(&self) -> Vec<&crate::models::ReviewPr> {
        self.prs
            .iter()
            .filter(|pr| self.repo_matches(&pr.repo))
            .collect()
    }

    pub fn repo_matches(&self, repo: &str) -> bool {
        repo_filter_matches(&self.repo_filter, self.repo_filter_mode, repo)
    }

    /// Whether this list needs a refresh given the interval.
    pub fn needs_fetch(&self, interval: Duration) -> bool {
        self.last_fetch
            .map(|t| t.elapsed() > interval)
            .unwrap_or(true)
    }
}

// ---------------------------------------------------------------------------
// ReviewBoardState — review board data and loading state
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct ReviewBoardState {
    pub review: PrListState,
    pub authored: PrListState,
    pub detail_visible: bool,
    pub dispatch_pr_filter: bool,
    pub review_flash: HashMap<PrRef, Instant>,
    pub review_agents: HashMap<PrRef, ReviewAgentHandle>,
}

impl ReviewBoardState {
    pub fn list(&self, kind: PrListKind) -> Option<&PrListState> {
        match kind {
            PrListKind::Review => Some(&self.review),
            PrListKind::Authored => Some(&self.authored),
            PrListKind::Bot => None,
        }
    }

    pub fn list_mut(&mut self, kind: PrListKind) -> Option<&mut PrListState> {
        match kind {
            PrListKind::Review => Some(&mut self.review),
            PrListKind::Authored => Some(&mut self.authored),
            PrListKind::Bot => None,
        }
    }

    /// Insert a review agent handle and return the DB table kind for the PR.
    /// Returns `None` if the PR is not found in any tracked list.
    pub fn find_and_set_pr_agent(
        &mut self,
        github_repo: &str,
        number: i64,
        tmux_window: &str,
        worktree: &str,
    ) -> Option<crate::db::PrKind> {
        let handle = ReviewAgentHandle {
            tmux_window: tmux_window.to_string(),
            worktree: worktree.to_string(),
            status: ReviewAgentStatus::Reviewing,
        };
        let key = PrRef::new(github_repo.to_string(), number);
        self.review_agents.insert(key, handle);

        for kind in [PrListKind::Review, PrListKind::Authored] {
            if self
                .list(kind)
                .unwrap()
                .prs
                .iter()
                .any(|pr| pr.repo == github_repo && pr.number == number)
            {
                return Some(kind.to_pr_kind());
            }
        }
        None
    }
}

/// Compute a sorted, deduplicated list of repo names from a slice of review PRs.
fn distinct_repos(prs: &[crate::models::ReviewPr]) -> Vec<String> {
    prs.iter()
        .map(|pr| pr.repo.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
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
}

impl BoardSelection {
    pub fn new() -> Self {
        Self {
            selected_column: 0,
            selected_row: [0; TaskStatus::COLUMN_COUNT],
            on_select_all: false,
            list_states: std::array::from_fn(|_| ListState::default()),
        }
    }

    pub fn column(&self) -> usize {
        self.selected_column
    }

    pub fn row(&self, col: usize) -> usize {
        self.selected_row[col]
    }

    pub fn set_column(&mut self, col: usize) {
        self.selected_column = col;
    }

    pub fn set_row(&mut self, col: usize, row: usize) {
        self.selected_row[col] = row;
    }

    /// List item index for `ListState` scrolling.
    /// Returns `None` when the cursor is on the select-all toggle (header),
    /// since no list item should be selected in that case.
    pub fn list_state_index(&self, col: usize) -> Option<usize> {
        if self.on_select_all {
            None
        } else {
            Some(self.selected_row[col])
        }
    }
}

impl Default for BoardSelection {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// ReviewBoardSelection — column + row selection state for review board
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ReviewBoardSelection {
    pub(in crate::tui) selected_column: usize,
    pub(in crate::tui) selected_row: [usize; ReviewDecision::COLUMN_COUNT],
    pub(in crate::tui) list_states: [ListState; ReviewDecision::COLUMN_COUNT],
}

impl ReviewBoardSelection {
    pub fn new() -> Self {
        Self {
            selected_column: 0,
            selected_row: [0; ReviewDecision::COLUMN_COUNT],
            list_states: std::array::from_fn(|_| ListState::default()),
        }
    }

    pub fn column(&self) -> usize {
        self.selected_column
    }

    pub fn row(&self, col: usize) -> usize {
        self.selected_row[col]
    }

    pub fn set_column(&mut self, col: usize) {
        self.selected_column = col;
    }

    pub fn set_row(&mut self, col: usize, row: usize) {
        self.selected_row[col] = row;
    }

    pub fn list_state_index(&self, col: usize) -> Option<usize> {
        Some(self.selected_row[col])
    }
}

impl Default for ReviewBoardSelection {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// SecurityBoardSelection — column + row selection state for security board
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SecurityBoardSelection {
    pub(in crate::tui) selected_column: usize,
    pub(in crate::tui) selected_row: [usize; AlertSeverity::COLUMN_COUNT],
    pub(in crate::tui) list_states: [ListState; AlertSeverity::COLUMN_COUNT],
}

impl SecurityBoardSelection {
    pub fn new() -> Self {
        Self {
            selected_column: 0,
            selected_row: [0; AlertSeverity::COLUMN_COUNT],
            list_states: std::array::from_fn(|_| ListState::default()),
        }
    }

    pub fn column(&self) -> usize {
        self.selected_column
    }

    pub fn row(&self, col: usize) -> usize {
        self.selected_row[col]
    }

    pub fn set_column(&mut self, col: usize) {
        self.selected_column = col;
    }

    pub fn set_row(&mut self, col: usize, row: usize) {
        self.selected_row[col] = row;
    }

    pub fn list_state_index(&self, col: usize) -> Option<usize> {
        Some(self.selected_row[col])
    }
}

impl Default for SecurityBoardSelection {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// SecurityBoardMode — sub-view selector for the Security Board
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SecurityBoardMode {
    #[default]
    Dependabot,
    Alerts,
}

// ---------------------------------------------------------------------------
// DependabotBoardState — state for the Dependabot PR sub-view
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct DependabotBoardState {
    pub prs: PrListState,
    pub selected_prs: HashSet<String>,
    pub detail_visible: bool,
}

// ---------------------------------------------------------------------------
// SecurityBoardState — security board data and loading state
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct SecurityBoardState {
    pub alerts: Vec<SecurityAlert>,
    pub repos: Vec<String>,
    pub loading: bool,
    pub unconfigured: bool,
    pub last_fetch: Option<Instant>,
    pub last_error: Option<String>,
    pub detail_visible: bool,
    pub repo_filter: HashSet<String>,
    pub repo_filter_mode: RepoFilterMode,
    pub kind_filter: Option<AlertKind>,
    pub review_flash: HashMap<PrRef, Instant>,
    pub dependabot: DependabotBoardState,
    pub fix_agents: HashMap<FixDispatchKey, FixAgentHandle>,
}

impl SecurityBoardState {
    /// Set alerts and rebuild the cached distinct repos list.
    pub fn set_alerts(&mut self, alerts: Vec<SecurityAlert>) {
        self.repos = {
            let mut set = BTreeSet::new();
            for a in &alerts {
                set.insert(a.repo.clone());
            }
            set.into_iter().collect()
        };
        self.alerts = alerts;
    }

    /// Return alerts filtered by repo filter and kind filter.
    pub fn filtered_alerts(&self) -> Vec<&SecurityAlert> {
        self.alerts
            .iter()
            .filter(|a| self.repo_matches(&a.repo))
            .filter(|a| self.kind_filter.is_none() || self.kind_filter == Some(a.kind))
            .collect()
    }

    pub fn repo_matches(&self, repo: &str) -> bool {
        repo_filter_matches(&self.repo_filter, self.repo_filter_mode, repo)
    }

    /// Whether alerts need a refresh given the interval.
    pub fn needs_fetch(&self, interval: Duration) -> bool {
        self.last_fetch
            .map(|t| t.elapsed() > interval)
            .unwrap_or(true)
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
    ReviewBoard {
        mode: ReviewBoardMode,
        selection: ReviewBoardSelection,
        saved_board: BoardSelection,
    },
    SecurityBoard {
        mode: SecurityBoardMode,
        selection: SecurityBoardSelection,
        dependabot_selection: ReviewBoardSelection,
        saved_board: BoardSelection,
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
    use crate::models::{CiStatus, ReviewDecision, ReviewPr};
    use chrono::Utc;

    fn make_pr(number: i64, repo: &str) -> ReviewPr {
        ReviewPr {
            number,
            title: format!("PR {number}"),
            author: "alice".to_string(),
            repo: repo.to_string(),
            url: format!("https://github.com/{repo}/pull/{number}"),
            is_draft: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            additions: 10,
            deletions: 5,
            review_decision: ReviewDecision::ReviewRequired,
            labels: vec![],
            body: String::new(),
            head_ref: String::new(),
            ci_status: CiStatus::None,
            reviewers: vec![],
        }
    }

    // -- PrListState::set_prs --

    #[test]
    fn pr_list_state_set_prs_stores_prs_and_computes_repos() {
        let mut state = PrListState::default();
        state.set_prs(vec![make_pr(1, "org/beta"), make_pr(2, "org/alpha")]);
        assert_eq!(state.prs.len(), 2);
        // repos should be sorted and deduplicated
        assert_eq!(state.repos, vec!["org/alpha", "org/beta"]);
    }

    // -- PrListState::filtered --

    #[test]
    fn pr_list_state_filtered_returns_all_when_no_filter() {
        let mut state = PrListState::default();
        state.set_prs(vec![make_pr(1, "org/a"), make_pr(2, "org/b")]);
        assert_eq!(state.filtered().len(), 2);
    }

    #[test]
    fn pr_list_state_filtered_include_mode() {
        let mut state = PrListState::default();
        state.set_prs(vec![make_pr(1, "org/a"), make_pr(2, "org/b")]);
        state.repo_filter.insert("org/a".to_string());
        state.repo_filter_mode = RepoFilterMode::Include;
        let result = state.filtered();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].repo, "org/a");
    }

    #[test]
    fn pr_list_state_filtered_exclude_mode() {
        let mut state = PrListState::default();
        state.set_prs(vec![make_pr(1, "org/a"), make_pr(2, "org/b")]);
        state.repo_filter.insert("org/a".to_string());
        state.repo_filter_mode = RepoFilterMode::Exclude;
        let result = state.filtered();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].repo, "org/b");
    }

    // -- PrListState::repo_matches --

    #[test]
    fn pr_list_state_repo_matches_empty_filter_matches_all() {
        let state = PrListState::default();
        assert!(state.repo_matches("anything"));
    }

    #[test]
    fn pr_list_state_repo_matches_include_mode() {
        let mut state = PrListState::default();
        state.repo_filter.insert("org/a".to_string());
        state.repo_filter_mode = RepoFilterMode::Include;
        assert!(state.repo_matches("org/a"));
        assert!(!state.repo_matches("org/b"));
    }

    #[test]
    fn pr_list_state_repo_matches_exclude_mode() {
        let mut state = PrListState::default();
        state.repo_filter.insert("org/a".to_string());
        state.repo_filter_mode = RepoFilterMode::Exclude;
        assert!(!state.repo_matches("org/a"));
        assert!(state.repo_matches("org/b"));
    }

    // -- PrListState::needs_fetch --

    #[test]
    fn pr_list_state_needs_fetch_true_when_never_fetched() {
        let state = PrListState::default();
        assert!(state.needs_fetch(Duration::from_secs(60)));
    }

    #[test]
    fn pr_list_state_needs_fetch_false_when_recently_fetched() {
        let state = PrListState {
            last_fetch: Some(Instant::now()),
            ..Default::default()
        };
        assert!(!state.needs_fetch(Duration::from_secs(60)));
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

    // -- PrListKind --

    #[test]
    fn pr_list_kind_settings_key() {
        assert_eq!(PrListKind::Review.settings_key(), "github_queries_review");
        assert_eq!(PrListKind::Authored.settings_key(), "github_queries_my_prs");
        assert_eq!(PrListKind::Bot.settings_key(), "github_queries_bot");
    }

    #[test]
    fn pr_list_kind_table_name() {
        assert_eq!(PrListKind::Review.table_name(), "review_prs");
        assert_eq!(PrListKind::Authored.table_name(), "my_prs");
        assert_eq!(PrListKind::Bot.table_name(), "bot_prs");
    }

    #[test]
    fn pr_list_kind_label() {
        assert_eq!(PrListKind::Review.label(), "review");
        assert_eq!(PrListKind::Authored.label(), "my");
        assert_eq!(PrListKind::Bot.label(), "bot");
    }

    // -- ReviewBoardState::list / list_mut --

    #[test]
    fn review_board_state_list_returns_correct_list() {
        let mut state = ReviewBoardState::default();
        state.review.set_prs(vec![make_pr(1, "org/a")]);
        state
            .authored
            .set_prs(vec![make_pr(2, "org/b"), make_pr(3, "org/c")]);

        assert_eq!(state.list(PrListKind::Review).unwrap().prs.len(), 1);
        assert_eq!(state.list(PrListKind::Authored).unwrap().prs.len(), 2);
    }

    #[test]
    fn review_board_state_list_mut_mutates_correct_list() {
        let mut state = ReviewBoardState::default();
        state.list_mut(PrListKind::Review).unwrap().loading = true;
        assert!(state.review.loading);
        assert!(!state.authored.loading);
    }

    // -- ReviewBoardState::find_and_set_pr_agent --

    #[test]
    fn find_and_set_pr_agent_sets_fields_in_review_list() {
        let mut state = ReviewBoardState::default();
        state.review.set_prs(vec![make_pr(42, "org/app")]);

        let kind = state.find_and_set_pr_agent("org/app", 42, "win-42", "/tmp/wt");
        assert_eq!(kind, Some(crate::db::PrKind::Review));
        let key = PrRef::new("org/app".to_string(), 42);
        let handle = state.review_agents.get(&key).unwrap();
        assert_eq!(handle.tmux_window, "win-42");
        assert_eq!(handle.worktree, "/tmp/wt");
        assert_eq!(handle.status, crate::models::ReviewAgentStatus::Reviewing);
    }

    #[test]
    fn find_and_set_pr_agent_sets_fields_in_authored_list() {
        let mut state = ReviewBoardState::default();
        state.authored.set_prs(vec![make_pr(99, "org/lib")]);

        let kind = state.find_and_set_pr_agent("org/lib", 99, "win-99", "/tmp/wt2");
        assert_eq!(kind, Some(crate::db::PrKind::My));
        let key = PrRef::new("org/lib".to_string(), 99);
        let handle = state.review_agents.get(&key).unwrap();
        assert_eq!(handle.tmux_window, "win-99");
    }

    #[test]
    fn find_and_set_pr_agent_returns_none_when_not_found() {
        let mut state = ReviewBoardState::default();
        let kind = state.find_and_set_pr_agent("org/unknown", 1, "win", "/wt");
        assert_eq!(kind, None);
    }

    #[test]
    fn review_board_list_bot_returns_none() {
        let state = ReviewBoardState::default();
        assert!(state.list(PrListKind::Bot).is_none());
    }

    #[test]
    fn review_board_list_mut_bot_returns_none() {
        let mut state = ReviewBoardState::default();
        assert!(state.list_mut(PrListKind::Bot).is_none());
    }

    #[test]
    fn review_board_list_review_returns_some() {
        let state = ReviewBoardState::default();
        assert!(state.list(PrListKind::Review).is_some());
    }

    #[test]
    fn review_board_list_authored_returns_some() {
        let state = ReviewBoardState::default();
        assert!(state.list(PrListKind::Authored).is_some());
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
