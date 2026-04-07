use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::{Duration, Instant};

use ratatui::widgets::ListState;

use crate::models::{
    AlertKind, AlertSeverity, DispatchMode, Epic, EpicId, EpicSubstatus, PrRef, ReviewDecision,
    SecurityAlert, SubStatus, Task, TaskId, TaskStatus, TaskTag, TaskUsage,
};

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
    Dependabot,
}

impl ReviewBoardMode {
    pub fn column_count(&self) -> usize {
        4
    }

    pub fn column_label(&self, col: usize) -> &'static str {
        match self {
            Self::Reviewer | Self::Author => match col {
                0 => "Needs Review",
                1 => "Waiting for Response",
                2 => "Changes Requested",
                3 => "Approved",
                _ => "",
            },
            Self::Dependabot => match col {
                0 => "CI Passing",
                1 => "CI Failing",
                2 => "CI Pending",
                3 => "Approved",
                _ => "",
            },
        }
    }

    pub fn pr_column(&self, pr: &crate::models::ReviewPr) -> usize {
        match self {
            Self::Reviewer | Self::Author => pr.review_decision.column_index(),
            Self::Dependabot => {
                if pr.review_decision == crate::models::ReviewDecision::Approved {
                    3
                } else {
                    pr.ci_status.column_index()
                }
            }
        }
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
    pub title: String,
    pub body: String,
    pub head_ref: String,
    pub is_dependabot: bool,
}

// ---------------------------------------------------------------------------
// PendingDispatch — held while user selects a repo path for dispatch
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum PendingDispatch {
    Review(ReviewAgentRequest),
    Fix {
        repo: String,
        number: i64,
        kind: AlertKind,
        title: String,
        description: String,
        package: Option<String>,
        fixed_version: Option<String>,
    },
}

impl PendingDispatch {
    pub fn github_repo(&self) -> &str {
        match self {
            PendingDispatch::Review(req) => &req.github_repo,
            PendingDispatch::Fix { repo, .. } => repo,
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

// ---------------------------------------------------------------------------
// Message
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
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
    ToggleReviewBoardMode,
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
    ToggleSelectBotPr(String),
    SelectAllBotPrColumn,
    ClearBotPrSelection,
    StartBatchApprove,
    StartBatchMerge,
    ConfirmBatchApprove,
    ConfirmBatchMerge,
    CancelBatchOperation,
    // Security board
    SwitchToSecurityBoard,
    SecurityAlertsLoaded(Vec<SecurityAlert>),
    SecurityAlertsFetchFailed(String),
    RefreshSecurityAlerts,
    ToggleSecurityDetail,
    ToggleSecurityKindFilter,
    StartSecurityRepoFilter,
    CloseSecurityRepoFilter,
    ToggleSecurityRepoFilter(String),
    ToggleAllSecurityRepoFilter,
    ToggleSecurityRepoFilterMode,
    DispatchFixAgent {
        repo: String,
        number: i64,
        kind: crate::models::AlertKind,
        title: String,
        description: String,
        package: Option<String>,
        fixed_version: Option<String>,
    },
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
}

// ---------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Command {
    PersistTask(Task),
    PersistReviewAgent {
        table: String,
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
    CheckSplitPaneExists {
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
    BatchApprovePrs(Vec<String>),
    BatchMergePrs(Vec<String>),
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
    DispatchFixAgent {
        repo: String,
        github_repo: String,
        number: i64,
        kind: crate::models::AlertKind,
        title: String,
        description: String,
        package: Option<String>,
        fixed_version: Option<String>,
    },
    EditGithubQueries(ReviewBoardMode),
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
    SecurityRepoFilter,
    InputPresetName,
    ConfirmDeletePreset,
    ConfirmDeleteRepoPath,
    ConfirmEditTask(TaskId),
    // Dependabot batch operations
    ConfirmBatchApprove(Vec<String>),
    ConfirmBatchMerge(Vec<String>),
    ConfirmQuit,
    // Dispatch repo path input (review/security tab fallback)
    InputDispatchRepoPath,
}

// ---------------------------------------------------------------------------
// TaskDraft
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct TaskDraft {
    pub title: String,
    pub description: String,
    pub repo_path: String,
    pub tag: Option<TaskTag>,
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

#[derive(Debug)]
pub struct AgentTracking {
    pub tmux_outputs: HashMap<TaskId, String>,
    pub last_output_change: HashMap<TaskId, Instant>,
    pub last_activity: HashMap<TaskId, u64>,
    pub inactivity_timeout: Duration,
    pub notified_review: HashSet<TaskId>,
    pub notified_needs_input: HashSet<TaskId>,
    pub last_pr_poll: HashMap<TaskId, Instant>,
    pub message_flash: HashMap<TaskId, Instant>,
}

impl AgentTracking {
    pub fn new(inactivity_timeout: Duration) -> Self {
        Self {
            tmux_outputs: HashMap::new(),
            last_output_change: HashMap::new(),
            last_activity: HashMap::new(),
            inactivity_timeout,
            notified_review: HashSet::new(),
            notified_needs_input: HashSet::new(),
            last_pr_poll: HashMap::new(),
            message_flash: HashMap::new(),
        }
    }

    /// Remove all tracking state for a task.
    pub fn clear(&mut self, id: TaskId) {
        self.last_output_change.remove(&id);
        self.last_activity.remove(&id);
        self.tmux_outputs.remove(&id);
        self.notified_review.remove(&id);
        self.notified_needs_input.remove(&id);
        self.last_pr_poll.remove(&id);
        self.message_flash.remove(&id);
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

#[derive(Debug, Default)]
pub struct SplitState {
    pub(in crate::tui) active: bool,
    pub(in crate::tui) right_pane_id: Option<String>,
    pub(in crate::tui) pinned_task_id: Option<TaskId>,
}

// ---------------------------------------------------------------------------
// SelectionState — multi-select state for batch operations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct SelectionState {
    pub tasks: HashSet<TaskId>,
    pub epics: HashSet<EpicId>,
    pub bot_prs: HashSet<String>,
    pub pending_done: Vec<TaskId>,
}

impl SelectionState {
    pub fn has_selection(&self) -> bool {
        !self.tasks.is_empty() || !self.epics.is_empty()
    }

    pub fn has_bot_pr_selection(&self) -> bool {
        !self.bot_prs.is_empty()
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
        match self {
            Self::Review => "review_prs",
            Self::Authored => "my_prs",
            Self::Bot => "bot_prs",
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
    /// Replace the PR list, preserving agent fields (tmux_window, worktree,
    /// agent_status) from the previous list when the new PR lacks them.
    /// Also rebuilds the cached distinct repos list.
    pub fn set_prs(&mut self, mut prs: Vec<crate::models::ReviewPr>) {
        for new_pr in prs.iter_mut() {
            if let Some(old_pr) = self
                .prs
                .iter()
                .find(|p| p.repo == new_pr.repo && p.number == new_pr.number)
            {
                if new_pr.tmux_window.is_none() {
                    new_pr.tmux_window = old_pr.tmux_window.clone();
                }
                if new_pr.worktree.is_none() {
                    new_pr.worktree = old_pr.worktree.clone();
                }
                if new_pr.agent_status.is_none() {
                    new_pr.agent_status = old_pr.agent_status;
                }
            }
        }
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
        if self.repo_filter.is_empty() {
            return true;
        }
        match self.repo_filter_mode {
            RepoFilterMode::Include => self.repo_filter.contains(repo),
            RepoFilterMode::Exclude => !self.repo_filter.contains(repo),
        }
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
    pub bot: PrListState,
    pub detail_visible: bool,
    pub dispatch_pr_filter: bool,
    pub review_flash: HashMap<PrRef, Instant>,
}

impl ReviewBoardState {
    pub fn list(&self, kind: PrListKind) -> &PrListState {
        match kind {
            PrListKind::Review => &self.review,
            PrListKind::Authored => &self.authored,
            PrListKind::Bot => &self.bot,
        }
    }

    pub fn list_mut(&mut self, kind: PrListKind) -> &mut PrListState {
        match kind {
            PrListKind::Review => &mut self.review,
            PrListKind::Authored => &mut self.authored,
            PrListKind::Bot => &mut self.bot,
        }
    }

    /// Find a PR by github_repo + number across all review lists, set its agent
    /// fields, and return the DB table name where the PR lives.
    pub fn find_and_set_pr_agent(
        &mut self,
        github_repo: &str,
        number: i64,
        tmux_window: &str,
        worktree: &str,
    ) -> String {
        for kind in [PrListKind::Review, PrListKind::Authored, PrListKind::Bot] {
            for pr in self.list_mut(kind).prs.iter_mut() {
                if pr.repo == github_repo && pr.number == number {
                    pr.tmux_window = Some(tmux_window.to_string());
                    pr.worktree = Some(worktree.to_string());
                    pr.agent_status = Some(crate::models::ReviewAgentStatus::Reviewing);
                    return kind.table_name().to_string();
                }
            }
        }
        "review_prs".to_string()
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
// SecurityBoardState — security board data and loading state
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct SecurityBoardState {
    pub alerts: Vec<SecurityAlert>,
    pub repos: Vec<String>,
    pub loading: bool,
    pub last_fetch: Option<Instant>,
    pub last_error: Option<String>,
    pub detail_visible: bool,
    pub repo_filter: HashSet<String>,
    pub repo_filter_mode: RepoFilterMode,
    pub kind_filter: Option<AlertKind>,
    pub review_flash: HashMap<PrRef, Instant>,
}

impl SecurityBoardState {
    /// Set alerts and rebuild the cached distinct repos list.
    /// Preserves agent fields (tmux_window, worktree) from old alerts.
    pub fn set_alerts(&mut self, mut alerts: Vec<SecurityAlert>) {
        for new_alert in alerts.iter_mut() {
            if let Some(old_alert) = self.alerts.iter().find(|a| {
                a.repo == new_alert.repo && a.number == new_alert.number && a.kind == new_alert.kind
            }) {
                if new_alert.tmux_window.is_none() {
                    new_alert.tmux_window = old_alert.tmux_window.clone();
                }
                if new_alert.worktree.is_none() {
                    new_alert.worktree = old_alert.worktree.clone();
                }
                if new_alert.agent_status.is_none() {
                    new_alert.agent_status = old_alert.agent_status;
                }
            }
        }
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
        if self.repo_filter.is_empty() {
            return true;
        }
        match self.repo_filter_mode {
            RepoFilterMode::Include => self.repo_filter.contains(repo),
            RepoFilterMode::Exclude => !self.repo_filter.contains(repo),
        }
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
        saved_board: BoardSelection,
    },
    ReviewBoard {
        mode: ReviewBoardMode,
        selection: ReviewBoardSelection,
        saved_board: BoardSelection,
    },
    SecurityBoard {
        selection: SecurityBoardSelection,
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
            tmux_window: None,
            worktree: None,
            agent_status: None,
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

    #[test]
    fn pr_list_state_set_prs_preserves_agent_fields() {
        let mut state = PrListState::default();

        let mut old = make_pr(1, "org/app");
        old.tmux_window = Some("win-1".to_string());
        old.worktree = Some("/tmp/wt".to_string());
        old.agent_status = Some(crate::models::ReviewAgentStatus::Reviewing);
        state.set_prs(vec![old]);

        // Simulate a refresh: new PR has no agent fields
        let fresh = make_pr(1, "org/app");
        assert!(fresh.tmux_window.is_none());
        state.set_prs(vec![fresh]);

        assert_eq!(state.prs[0].tmux_window.as_deref(), Some("win-1"));
        assert_eq!(state.prs[0].worktree.as_deref(), Some("/tmp/wt"));
        assert_eq!(
            state.prs[0].agent_status,
            Some(crate::models::ReviewAgentStatus::Reviewing)
        );
    }

    #[test]
    fn pr_list_state_set_prs_does_not_overwrite_new_agent_fields() {
        let mut state = PrListState::default();

        let mut old = make_pr(1, "org/app");
        old.tmux_window = Some("old-win".to_string());
        state.set_prs(vec![old]);

        let mut fresh = make_pr(1, "org/app");
        fresh.tmux_window = Some("new-win".to_string());
        state.set_prs(vec![fresh]);

        assert_eq!(
            state.prs[0].tmux_window.as_deref(),
            Some("new-win"),
            "new non-None agent fields should not be overwritten by old values"
        );
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
        let mut state = PrListState::default();
        state.last_fetch = Some(Instant::now());
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
        state.authored.set_prs(vec![make_pr(2, "org/b"), make_pr(3, "org/c")]);
        state.bot.set_prs(vec![make_pr(4, "org/d"), make_pr(5, "org/e"), make_pr(6, "org/f")]);

        assert_eq!(state.list(PrListKind::Review).prs.len(), 1);
        assert_eq!(state.list(PrListKind::Authored).prs.len(), 2);
        assert_eq!(state.list(PrListKind::Bot).prs.len(), 3);
    }

    #[test]
    fn review_board_state_list_mut_mutates_correct_list() {
        let mut state = ReviewBoardState::default();
        state.list_mut(PrListKind::Review).loading = true;
        assert!(state.review.loading);
        assert!(!state.authored.loading);
        assert!(!state.bot.loading);
    }

    // -- ReviewBoardState::find_and_set_pr_agent --

    #[test]
    fn find_and_set_pr_agent_sets_fields_in_review_list() {
        let mut state = ReviewBoardState::default();
        state.review.set_prs(vec![make_pr(42, "org/app")]);

        let table = state.find_and_set_pr_agent("org/app", 42, "win-42", "/tmp/wt");
        assert_eq!(table, "review_prs");
        assert_eq!(state.review.prs[0].tmux_window.as_deref(), Some("win-42"));
        assert_eq!(state.review.prs[0].worktree.as_deref(), Some("/tmp/wt"));
        assert_eq!(
            state.review.prs[0].agent_status,
            Some(crate::models::ReviewAgentStatus::Reviewing)
        );
    }

    #[test]
    fn find_and_set_pr_agent_sets_fields_in_authored_list() {
        let mut state = ReviewBoardState::default();
        state.authored.set_prs(vec![make_pr(99, "org/lib")]);

        let table = state.find_and_set_pr_agent("org/lib", 99, "win-99", "/tmp/wt2");
        assert_eq!(table, "my_prs");
        assert_eq!(state.authored.prs[0].tmux_window.as_deref(), Some("win-99"));
    }

    #[test]
    fn find_and_set_pr_agent_sets_fields_in_bot_list() {
        let mut state = ReviewBoardState::default();
        state.bot.set_prs(vec![make_pr(7, "org/infra")]);

        let table = state.find_and_set_pr_agent("org/infra", 7, "win-7", "/tmp/wt3");
        assert_eq!(table, "bot_prs");
        assert_eq!(state.bot.prs[0].tmux_window.as_deref(), Some("win-7"));
    }

    #[test]
    fn find_and_set_pr_agent_defaults_to_review_prs_when_not_found() {
        let mut state = ReviewBoardState::default();
        let table = state.find_and_set_pr_agent("org/unknown", 1, "win", "/wt");
        assert_eq!(table, "review_prs");
    }
}
