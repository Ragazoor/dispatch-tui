//! The keybinding table: the one place every key the product answers to is
//! declared. See `docs/specs/keybindings.allium`.
//!
//! This module is a leaf (no `tui`, `db` or `mcp` dependency) so every reader
//! of the table - the key handlers, the `?` help overlay and the
//! `list_keybindings` MCP tool - can depend on it. It holds the data and the
//! pure relations over it; evaluating a [`KeyContext`] against a live board
//! (`context_holds`) belongs to the TUI.
//!
//! Every namespace has rows: the board's key dispatch, the agent-tree and
//! diff panes' dispatch and the `?` overlay all read them. Text-field editing
//! keys are rows that record no usage (`records_usage` false).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The `detail` of a keybinding usage event: the key as the user typed it,
/// modifiers dropped (`Ctrl+Left` is recorded as `Left`). Two bindings for one
/// action are told apart by this, so `j` and `Down` stay separable in recorded
/// data (`KeypressRecordsFeatureUsage` in `docs/specs/observability.allium`).
pub fn key_label(key: KeyEvent) -> String {
    match key.code {
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::BackTab => "BackTab".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Delete => "Delete".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        other => format!("{other:?}"),
    }
}

/// Whether Ctrl or Alt is held: such a press is looked up under its modified
/// name and only a row listing that name answers it
/// (`ModifiedPressMatchesOnlyItsOwnRow`).
pub fn key_has_ctrl_or_alt(key: KeyEvent) -> bool {
    key.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

/// A key as the table writes it: "Space", "Enter", "j", "L"; with Ctrl or Alt
/// held, modifier then key with a letter in uppercase ("Ctrl+D", "Alt+B",
/// "Ctrl+Left"). Shift alone is never part of the name.
pub fn key_name(key: KeyEvent) -> String {
    let base = match key.code {
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(c) if key_has_ctrl_or_alt(key) => c.to_ascii_uppercase().to_string(),
        _ => key_label(key),
    };
    let mut name = String::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        name.push_str("Ctrl+");
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        name.push_str("Alt+");
    }
    name.push_str(&base);
    name
}

/// The input mode a key is looked up in. Published names are the dotted forms
/// returned by [`KeyNamespace::name`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyNamespace {
    BoardNormal,
    BoardDetail,
    BoardSearch,
    BoardHelp,
    BoardError,
    BoardText,
    BoardRepoFilter,
    BoardPickerRepoPath,
    BoardPickerBaseBranch,
    BoardPickerTag,
    BoardPickerWrapUpMode,
    BoardPickerQuickDispatch,
    BoardPickerMoveToEpic,
    BoardPickerReparentEpic,
    BoardConfirmQuit,
    BoardConfirmDeleteTask,
    BoardConfirmBatchDelete,
    BoardConfirmDeleteEpic,
    BoardConfirmDone,
    BoardConfirmRetry,
    BoardConfirmDetachTmux,
    BoardConfirmOverrideFeedOwner,
    BoardConfirmMoveToEpic,
    BoardConfirmReparentEpic,
    BoardConfirmDeleteRepoPath,
    BoardConfirmTrustRepo,
    BoardConfirmTrustRepoQuickDispatch,
    BoardConfirmRepoSync,
    AgentTreeTree,
    AgentTreeCommits,
    AgentTreeAgents,
    AgentDiff,
    TmuxGlobal,
}

impl KeyNamespace {
    /// Every namespace, in the order the spec's enum lists them. This is the
    /// order `list_keybindings` and the help overlay group by.
    pub const ALL: [KeyNamespace; 33] = [
        Self::BoardNormal,
        Self::BoardDetail,
        Self::BoardSearch,
        Self::BoardHelp,
        Self::BoardError,
        Self::BoardText,
        Self::BoardRepoFilter,
        Self::BoardPickerRepoPath,
        Self::BoardPickerBaseBranch,
        Self::BoardPickerTag,
        Self::BoardPickerWrapUpMode,
        Self::BoardPickerQuickDispatch,
        Self::BoardPickerMoveToEpic,
        Self::BoardPickerReparentEpic,
        Self::BoardConfirmQuit,
        Self::BoardConfirmDeleteTask,
        Self::BoardConfirmBatchDelete,
        Self::BoardConfirmDeleteEpic,
        Self::BoardConfirmDone,
        Self::BoardConfirmRetry,
        Self::BoardConfirmDetachTmux,
        Self::BoardConfirmOverrideFeedOwner,
        Self::BoardConfirmMoveToEpic,
        Self::BoardConfirmReparentEpic,
        Self::BoardConfirmDeleteRepoPath,
        Self::BoardConfirmTrustRepo,
        Self::BoardConfirmTrustRepoQuickDispatch,
        Self::BoardConfirmRepoSync,
        Self::AgentTreeTree,
        Self::AgentTreeCommits,
        Self::AgentTreeAgents,
        Self::AgentDiff,
        Self::TmuxGlobal,
    ];

    /// The published dotted name.
    pub fn name(self) -> &'static str {
        match self {
            Self::BoardNormal => "board.normal",
            Self::BoardDetail => "board.detail",
            Self::BoardSearch => "board.search",
            Self::BoardHelp => "board.help",
            Self::BoardError => "board.error",
            Self::BoardText => "board.text",
            Self::BoardRepoFilter => "board.repo_filter",
            Self::BoardPickerRepoPath => "board.picker.repo_path",
            Self::BoardPickerBaseBranch => "board.picker.base_branch",
            Self::BoardPickerTag => "board.picker.tag",
            Self::BoardPickerWrapUpMode => "board.picker.wrap_up_mode",
            Self::BoardPickerQuickDispatch => "board.picker.quick_dispatch",
            Self::BoardPickerMoveToEpic => "board.picker.move_to_epic",
            Self::BoardPickerReparentEpic => "board.picker.reparent_epic",
            Self::BoardConfirmQuit => "board.confirm.quit",
            Self::BoardConfirmDeleteTask => "board.confirm.delete_task",
            Self::BoardConfirmBatchDelete => "board.confirm.batch_delete",
            Self::BoardConfirmDeleteEpic => "board.confirm.delete_epic",
            Self::BoardConfirmDone => "board.confirm.done",
            Self::BoardConfirmRetry => "board.confirm.retry",
            Self::BoardConfirmDetachTmux => "board.confirm.detach_tmux",
            Self::BoardConfirmOverrideFeedOwner => "board.confirm.override_feed_owner",
            Self::BoardConfirmMoveToEpic => "board.confirm.move_to_epic",
            Self::BoardConfirmReparentEpic => "board.confirm.reparent_epic",
            Self::BoardConfirmDeleteRepoPath => "board.confirm.delete_repo_path",
            Self::BoardConfirmTrustRepo => "board.confirm.trust_repo",
            Self::BoardConfirmTrustRepoQuickDispatch => "board.confirm.trust_repo_quick_dispatch",
            Self::BoardConfirmRepoSync => "board.confirm.repo_sync",
            Self::AgentTreeTree => "agent_tree.tree",
            Self::AgentTreeCommits => "agent_tree.commits",
            Self::AgentTreeAgents => "agent_tree.agents",
            Self::AgentDiff => "agent_diff",
            Self::TmuxGlobal => "tmux.global",
        }
    }

    pub fn parse(name: &str) -> Option<KeyNamespace> {
        Self::ALL.iter().copied().find(|n| n.name() == name)
    }

    /// Who receives a key in this namespace.
    pub fn receiver(self) -> KeyReceiver {
        if self == Self::TmuxGlobal {
            KeyReceiver::Tmux
        } else {
            KeyReceiver::Dispatch
        }
    }
}

/// Family names accepted by `list_keybindings` as a filter: a prefix standing
/// for every `board.confirm.*` / `board.picker.*` namespace.
pub const KEY_FAMILIES: [&str; 2] = ["board.confirm", "board.picker"];

/// The namespaces a name selects: one namespace, or every namespace of a
/// family. `None` for a name that is neither.
pub fn namespaces_matching(name: &str) -> Option<Vec<KeyNamespace>> {
    if let Some(ns) = KeyNamespace::parse(name) {
        return Some(vec![ns]);
    }
    if KEY_FAMILIES.contains(&name) {
        let prefix = format!("{name}.");
        return Some(
            KeyNamespace::ALL
                .iter()
                .copied()
                .filter(|n| n.name().starts_with(&prefix))
                .collect(),
        );
    }
    None
}

/// Who receives a key in a namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyReceiver {
    Dispatch,
    Tmux,
}

impl KeyReceiver {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dispatch => "dispatch",
            Self::Tmux => "tmux",
        }
    }
}

/// The condition a context-guarded row applies under. The literals for one key
/// are defined to be mutually exclusive (see `KeyContext` in the spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyContext {
    SearchActive,
    InsideEpicView,
    EpicViewNoSearch,
    TopLevel,
    WithSelection,
    OnColumnSelectAll,
    OnFoldedSection,
    OnFoldedEpicGroup,
    OnEpicCard,
    OnTaskCard,
    OnFlattenedTaskCard,
    OnUnflattenedTaskCard,
    OffEpicCard,
    OnDirectory,
    OnFile,
    TaskOnOtherMachine,
    TaskPinnedInSplit,
    TaskWindowSplitOpen,
    TaskWithWindow,
    BacklogTask,
    StuckTask,
    TaskDispatching,
    TaskWithWorktree,
    TaskWithoutWorktree,
    DetailZoomed,
}

impl KeyContext {
    /// The plain words shown to readers; the literal is never shown.
    pub fn words(self) -> &'static str {
        match self {
            Self::SearchActive => "with a search active",
            Self::InsideEpicView => "inside an epic view",
            Self::EpicViewNoSearch => "inside an epic view, no search active",
            Self::TopLevel => "on the top-level board",
            Self::WithSelection => {
                "on the top-level board, no search active, with cards selected or the cursor on a select-all row"
            }
            Self::OnColumnSelectAll => "on a column's select-all row",
            Self::OnFoldedSection => "on a folded section header",
            Self::OnFoldedEpicGroup => "on a folded epic group",
            Self::OnEpicCard => "on an epic card",
            Self::OnTaskCard => "on a task card",
            Self::OnFlattenedTaskCard => "on a task card in a flattened column",
            Self::OnUnflattenedTaskCard => "on a task card in a column that is not flattened",
            Self::OffEpicCard => "anywhere but an epic card",
            Self::OnDirectory => "on a directory",
            Self::OnFile => "on a file",
            Self::TaskOnOtherMachine => "on a task held by another machine",
            Self::TaskPinnedInSplit => "on the task shown in the split pane",
            Self::TaskWindowSplitOpen => "on a task with a window, split view open",
            Self::TaskWithWindow => "on a task with an agent window",
            Self::BacklogTask => "on a Backlog task with no window",
            Self::StuckTask => "on a stale, crashed or unprovisioned task with no window",
            Self::TaskDispatching => "on a task whose dispatch is still starting",
            Self::TaskWithWorktree => "on a task with a worktree but no window",
            Self::TaskWithoutWorktree => "on a task with neither window nor worktree",
            Self::DetailZoomed => "with the detail zoomed",
        }
    }
}

/// Which axis a context talks about. Contexts on different axes can always
/// hold together; contexts on one axis overlap only where the table below says.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ContextAxis {
    /// Which view is showing, and whether a search is active.
    View,
    /// Where the cursor is on the board.
    Cursor,
    /// The rung of Space's activation ladder (a task card refined further).
    Ladder,
    /// A pane's cursor row: directory or file.
    PaneRow,
    /// The task-detail zoom flag.
    Zoom,
}

impl KeyContext {
    fn axis(self) -> ContextAxis {
        use KeyContext::*;
        match self {
            SearchActive | InsideEpicView | EpicViewNoSearch | TopLevel | WithSelection => {
                ContextAxis::View
            }
            OnColumnSelectAll
            | OnFoldedSection
            | OnFoldedEpicGroup
            | OnEpicCard
            | OnTaskCard
            | OnFlattenedTaskCard
            | OnUnflattenedTaskCard
            | OffEpicCard => ContextAxis::Cursor,
            TaskOnOtherMachine | TaskPinnedInSplit | TaskWindowSplitOpen | TaskWithWindow
            | BacklogTask | StuckTask | TaskDispatching | TaskWithWorktree
            | TaskWithoutWorktree => ContextAxis::Ladder,
            OnDirectory | OnFile => ContextAxis::PaneRow,
            DetailZoomed => ContextAxis::Zoom,
        }
    }
}

/// Whether two contexts can hold at once for the same key
/// (ContextsDoNotOverlapForAKey). Contexts on different axes can co-occur,
/// except that a pane row never combines with a board context. Within one
/// axis only the pairs listed here overlap; every other pair is exclusive.
pub fn contexts_overlap(a: KeyContext, b: KeyContext) -> bool {
    use KeyContext::*;
    if a == b {
        return true;
    }
    let (xa, xb) = (a.axis(), b.axis());
    if xa != xb {
        let pane = |x| x == ContextAxis::PaneRow;
        if pane(xa) || pane(xb) {
            return false;
        }
        // A ladder rung is a task card, so only a cursor context that can be
        // a task card holds with it.
        let cursor_with_ladder = |c: KeyContext| {
            matches!(
                c,
                OnTaskCard | OnFlattenedTaskCard | OnUnflattenedTaskCard | OffEpicCard
            )
        };
        return match (xa, xb) {
            (ContextAxis::Cursor, ContextAxis::Ladder) => cursor_with_ladder(a),
            (ContextAxis::Ladder, ContextAxis::Cursor) => cursor_with_ladder(b),
            _ => true,
        };
    }
    let pair = |x: KeyContext, y: KeyContext| (a == x && b == y) || (a == y && b == x);
    match xa {
        ContextAxis::View => {
            pair(SearchActive, InsideEpicView)
                || pair(SearchActive, TopLevel)
                || pair(InsideEpicView, EpicViewNoSearch)
                || pair(TopLevel, WithSelection)
        }
        // OffEpicCard is every cursor position except an epic card; a task
        // card is on exactly one of the flattened / unflattened contexts.
        ContextAxis::Cursor => {
            [
                OnColumnSelectAll,
                OnFoldedSection,
                OnFoldedEpicGroup,
                OnTaskCard,
                OnFlattenedTaskCard,
                OnUnflattenedTaskCard,
            ]
            .iter()
            .any(|c| pair(OffEpicCard, *c))
                || [OnFlattenedTaskCard, OnUnflattenedTaskCard]
                    .iter()
                    .any(|c| pair(OnTaskCard, *c))
        }
        // The rungs are one evaluated order, so no two hold together.
        ContextAxis::Ladder | ContextAxis::PaneRow | ContextAxis::Zoom => false,
    }
}

/// The reserved catch-all key: matches a press no other applicable row lists.
pub const ANY_OTHER_KEY: &str = "any other key";

/// One row of the keybinding table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyBinding {
    pub namespace: KeyNamespace,
    /// Every key that runs this row's action, as typed ("j", "Enter",
    /// "Space", "L", "gg").
    pub keys: &'static [&'static str],
    pub context: Option<KeyContext>,
    /// Stable action id; reuses the feature-usage action names.
    pub action: &'static str,
    pub description: &'static str,
    pub note: Option<&'static str>,
    /// Whether a press that takes effect records feature usage. False for the
    /// text-editing keys, which share text entry's exemption.
    pub records_usage: bool,
}

impl KeyBinding {
    pub fn receiver(&self) -> KeyReceiver {
        self.namespace.receiver()
    }
}

use KeyContext as C;
use KeyNamespace as N;

const fn row(
    namespace: KeyNamespace,
    keys: &'static [&'static str],
    context: Option<KeyContext>,
    action: &'static str,
    description: &'static str,
    note: Option<&'static str>,
) -> KeyBinding {
    KeyBinding {
        namespace,
        keys,
        context,
        action,
        description,
        note,
        records_usage: true,
    }
}

/// A row that runs its action but records no usage (text editing).
const fn unrecorded(
    namespace: KeyNamespace,
    keys: &'static [&'static str],
    action: &'static str,
    description: &'static str,
) -> KeyBinding {
    KeyBinding {
        namespace,
        keys,
        context: None,
        action,
        description,
        note: None,
        records_usage: false,
    }
}

const fn r(
    namespace: KeyNamespace,
    keys: &'static [&'static str],
    action: &'static str,
    description: &'static str,
) -> KeyBinding {
    row(namespace, keys, None, action, description, None)
}

const fn normal(
    keys: &'static [&'static str],
    context: Option<KeyContext>,
    action: &'static str,
    description: &'static str,
) -> KeyBinding {
    row(N::BoardNormal, keys, context, action, description, None)
}

/// The table. There is no second list of keys anywhere in the product.
pub static KEY_BINDINGS: &[KeyBinding] = &[
    // ---- board.normal: navigation ----
    normal(
        &["h", "Left", "l", "Right"],
        None,
        "navigate_column",
        "Move to the previous column (h, Left) or the next column (l, Right)",
    ),
    normal(
        &["j", "Down", "k", "Up"],
        None,
        "navigate_row",
        "Move to the next card (j, Down) or the previous card (k, Up) in the column",
    ),
    normal(
        &["[", "gg"],
        None,
        "navigate_row_first",
        "Jump to the first row of the column",
    ),
    row(
        N::BoardNormal,
        &["]", "G"],
        None,
        "navigate_row_last",
        "Jump to the last row of the column",
        Some("Last row of the column; does not enter an epic, use Space"),
    ),
    // ---- board.normal: view level ----
    normal(
        &["Esc"],
        Some(C::SearchActive),
        "clear_search",
        "Clear the active search query",
    ),
    normal(
        &["q"],
        Some(C::InsideEpicView),
        "exit_epic",
        "Leave the epic view and return to the board",
    ),
    normal(
        &["Esc"],
        Some(C::EpicViewNoSearch),
        "exit_epic",
        "Leave the epic view and return to the board",
    ),
    normal(
        &["Q"],
        Some(C::InsideEpicView),
        "exit_all_epics",
        "Leave every epic view and return to the board",
    ),
    normal(&["q"], Some(C::TopLevel), "quit", "Quit dispatch"),
    normal(
        &["Esc"],
        Some(C::WithSelection),
        "clear_selection",
        "Clear the current selection",
    ),
    // ---- board.normal: cursor level ----
    normal(
        &["Enter"],
        Some(C::OnColumnSelectAll),
        "select_all",
        "Toggle select-all for the column",
    ),
    normal(
        &["a"],
        None,
        "select_all",
        "Select every card in the column, or deselect them all when the whole column is already selected",
    ),
    normal(
        &["Enter", "Space"],
        Some(C::OnFoldedSection),
        "toggle_section_collapse",
        "Unfold the folded section",
    ),
    normal(
        &["z"],
        None,
        "toggle_section_collapse",
        "Fold the sub-status section the cursor is in; its header shows how many cards are hidden. Only the Running and Review columns have sections. Folds are remembered across restarts",
    ),
    normal(
        &["Enter", "Space"],
        Some(C::OnFoldedEpicGroup),
        "toggle_epic_fold",
        "Unfold the folded epic group",
    ),
    normal(
        &["Z"],
        None,
        "toggle_epic_fold",
        "Fold or unfold the epic group the cursor is in",
    ),
    normal(
        &["Enter"],
        Some(C::OnUnflattenedTaskCard),
        "open_task_detail",
        "Open the task detail panel",
    ),
    normal(
        &["i"],
        Some(C::OnTaskCard),
        "open_task_detail",
        "Open the task detail panel",
    ),
    normal(
        &["Enter"],
        Some(C::OnFlattenedTaskCard),
        "jump_to_task_epic",
        "Jump to the epic the task belongs to; does nothing for a task with no epic",
    ),
    normal(
        &["Enter"],
        Some(C::OnEpicCard),
        "jump_to_deepest_epic",
        "Jump to the deepest epic holding the work that puts this epic in the column",
    ),
    normal(
        &["Space"],
        Some(C::OnEpicCard),
        "enter_epic",
        "Enter the epic and show its subtasks",
    ),
    // ---- board.normal: Space on a task card (activation ladder) ----
    normal(
        &["Space"],
        Some(C::TaskOnOtherMachine),
        "activate_unavailable",
        "Report that another machine holds this task's worktree",
    ),
    normal(
        &["Space"],
        Some(C::TaskPinnedInSplit),
        "jump_to_tmux",
        "Focus the split pane the task is pinned in",
    ),
    normal(
        &["Space"],
        Some(C::TaskWindowSplitOpen),
        "swap_split_pane",
        "Swap the task's agent window into the split pane; the board keeps focus",
    ),
    normal(
        &["Space"],
        Some(C::TaskWithWindow),
        "jump_to_tmux",
        "Jump to the task's agent tmux window",
    ),
    normal(
        &["Space"],
        Some(C::BacklogTask),
        "dispatch_task",
        "Dispatch the Backlog task into a new agent window",
    ),
    normal(
        &["Space"],
        Some(C::StuckTask),
        "open_retry_dialog",
        "Open the kill-and-retry dialog for a stale, crashed or unprovisioned task",
    ),
    normal(
        &["Space"],
        Some(C::TaskDispatching),
        "activate_unavailable",
        "Report that the dispatch is still in progress",
    ),
    normal(
        &["Space"],
        Some(C::TaskWithWorktree),
        "resume_task",
        "Resume the task in its existing worktree",
    ),
    normal(
        &["Space"],
        Some(C::TaskWithoutWorktree),
        "activate_unavailable",
        "Report that there is no worktree to resume; move the task to Backlog and re-dispatch",
    ),
    // ---- board.normal: moving and reordering ----
    normal(
        &["L"],
        Some(C::OnEpicCard),
        "move_task_forward",
        "Move the epic's status forward",
    ),
    normal(
        &["L"],
        Some(C::OffEpicCard),
        "move_task_forward",
        "Move the selected task or tasks to the next status",
    ),
    normal(
        &["H"],
        Some(C::OnEpicCard),
        "move_task_backward",
        "Move the epic's status backward",
    ),
    normal(
        &["H"],
        Some(C::OffEpicCard),
        "move_task_backward",
        "Move the selected task or tasks to the previous status",
    ),
    normal(
        &["J"],
        None,
        "reorder_task_down",
        "Move the card down in the column; in an epic view this sets the dispatch order",
    ),
    normal(
        &["K"],
        None,
        "reorder_task_up",
        "Move the card up in the column; in an epic view this sets the dispatch order",
    ),
    normal(
        &["m"],
        Some(C::OnEpicCard),
        "reparent_epic",
        "Reparent the epic via the tree picker",
    ),
    normal(
        &["m"],
        Some(C::OnTaskCard),
        "move_task_to_epic",
        "Move the task to another epic, or detach it, via the tree picker",
    ),
    // ---- board.normal: creating and editing ----
    normal(&["n"], None, "create_task", "Create a new task"),
    normal(&["c"], None, "copy_task", "Copy the selected task"),
    normal(&["E"], None, "create_epic", "Create a new epic"),
    normal(
        &["e"],
        None,
        "edit_task",
        "Edit the selected task or epic in the editor (opens in a separate tmux window)",
    ),
    normal(
        &["x"],
        None,
        "delete_task",
        "Move the task to Done (with confirmation); on a task already in Done, delete it instead. On an epic, delete it and its whole subtask subtree, guarded on every task in it being Done. In a multi-selection of tasks: all Done means delete, otherwise the not-yet-Done tasks move to Done",
    ),
    normal(&["v"], None, "toggle_select", "Toggle selection of the card"),
    normal(
        &["D"],
        None,
        "quick_dispatch",
        "Quick dispatch: pick a repo (immediate with one repo) and dispatch a new task; inside an epic view the task becomes a subtask of that epic",
    ),
    normal(
        &["T"],
        None,
        "detach_tmux",
        "Detach the tmux panel of every selected task that has a live window (batch), after a confirmation",
    ),
    normal(
        &["r"],
        None,
        "refresh_feed",
        "Refresh a feed epic: the selected epic card if it has a feed command, otherwise the feed epic you are inside. Does nothing elsewhere",
    ),
    normal(
        &["p"],
        None,
        "open_pr_url",
        "Open the selected task's URL (its pull request, once one is set) in a browser; reports `No URL set` when there is none",
    ),
    normal(
        &["o"],
        None,
        "open_repo_sync_prompt",
        "Sync the selected task's repository with origin on its default branch (merge what it is behind by, push what it is ahead by) after a confirmation. Offered only while the status bar's drift segment is lit; otherwise does nothing",
    ),
    // ---- board.normal: epic view settings ----
    normal(
        &["U"],
        Some(C::InsideEpicView),
        "toggle_auto_dispatch",
        "Toggle auto-dispatch for the epic you are inside: chain the next backlog subtask when one finishes",
    ),
    normal(
        &["R"],
        Some(C::InsideEpicView),
        "toggle_group_by_repo",
        "Toggle group-by-repo for the epic you are inside",
    ),
    // ---- board.normal: filters, views, overlays ----
    normal(
        &["/"],
        None,
        "search_tasks",
        "Search the board with a live bar. A card matches when the query fuzzy-matches its title or is a digit prefix of its id (38 matches #38, #380; a leading # is optional). Epic cards match on their own title or id, or when a descendant the board would still show matches. Enter keeps the query, Esc in the bar restores the previous one, Esc on the board clears it",
    ),
    normal(&["f"], None, "filter_repos", "Filter the board by repo path"),
    normal(
        &["A"],
        None,
        "filter_active",
        "Toggle the filter that shows only tasks with an active tmux session",
    ),
    normal(
        &["F"],
        None,
        "toggle_flattened",
        "Toggle the flat view: in the Running, Review and Done columns show every task as a plain card instead of grouping subtasks under their epic. Backlog is never flattened",
    ),
    normal(
        &["N"],
        None,
        "toggle_notifications",
        "Toggle the notification panel",
    ),
    normal(
        &["s"],
        None,
        "toggle_split_mode",
        "Toggle split view: the board side by side with an agent pane. With the pane open, Space swaps the selected task into it",
    ),
    normal(&["?"], None, "toggle_help", "Toggle the help overlay"),
    // ---- board.detail ----
    r(
        N::BoardDetail,
        &["q", "Esc", "Enter"],
        "close_detail",
        "Close the task detail panel",
    ),
    r(
        N::BoardDetail,
        &["j", "Down", "k", "Up"],
        "scroll_detail",
        "Scroll the detail down (j, Down) or up (k, Up)",
    ),
    r(
        N::BoardDetail,
        &["z"],
        "zoom_detail",
        "Zoom the detail to fill the screen, or back",
    ),
    // ---- board.search ----
    r(
        N::BoardSearch,
        &["Esc"],
        "search_cancel",
        "Leave the search bar and restore the query that was active before it opened",
    ),
    r(
        N::BoardSearch,
        &["Enter"],
        "search_commit",
        "Leave the search bar and keep the query",
    ),
    unrecorded(
        N::BoardSearch,
        &["Backspace"],
        "text_backspace",
        "Delete the last character of the query",
    ),
    // ---- board.help ----
    r(
        N::BoardHelp,
        &["?", "Esc"],
        "close_help",
        "Close the help overlay",
    ),
    r(
        N::BoardHelp,
        &["j", "Down", "k", "Up"],
        "scroll_help",
        "Scroll the overlay down (j, Down) or up (k, Up)",
    ),
    // ---- board.error ----
    r(
        N::BoardError,
        &[ANY_OTHER_KEY],
        "dismiss_error",
        "Dismiss the error popup; the key that did it is not otherwise acted on",
    ),
    // ---- board.text ----
    r(
        N::BoardText,
        &["Esc"],
        "cancel_input",
        "Cancel the input and discard what was typed",
    ),
    r(
        N::BoardText,
        &["Enter"],
        "submit_input",
        "Submit what was typed and move to the next step",
    ),
    unrecorded(
        N::BoardText,
        &["Backspace"],
        "text_backspace",
        "Delete the character before the caret",
    ),
    unrecorded(
        N::BoardText,
        &["Delete"],
        "text_delete_forward",
        "Delete the character after the caret",
    ),
    unrecorded(
        N::BoardText,
        &["Left"],
        "text_cursor_left",
        "Move the caret one character left",
    ),
    unrecorded(
        N::BoardText,
        &["Right"],
        "text_cursor_right",
        "Move the caret one character right",
    ),
    unrecorded(
        N::BoardText,
        &["Home"],
        "text_cursor_home",
        "Move the caret to the start of the field",
    ),
    unrecorded(
        N::BoardText,
        &["End"],
        "text_cursor_end",
        "Move the caret to the end of the field",
    ),
    unrecorded(
        N::BoardText,
        &["Ctrl+Left", "Alt+Left", "Alt+B"],
        "text_cursor_word_left",
        "Move the caret one word left",
    ),
    unrecorded(
        N::BoardText,
        &["Ctrl+Right", "Alt+Right", "Alt+F"],
        "text_cursor_word_right",
        "Move the caret one word right",
    ),
    // ---- board.repo_filter ----
    r(
        N::BoardRepoFilter,
        &["Enter", "Esc", "q"],
        "repo_filter_close",
        "Close the repo filter",
    ),
    r(
        N::BoardRepoFilter,
        &["j", "Down", "k", "Up"],
        "repo_filter_move_cursor",
        "Move the cursor down (j, Down) or up (k, Up) the repo list",
    ),
    r(
        N::BoardRepoFilter,
        &["Space", "1", "2", "3", "4", "5", "6", "7", "8", "9"],
        "repo_filter_toggle_repo",
        "Toggle the repo under the cursor (Space) or the repo with that number (1-9) in the filter",
    ),
    r(
        N::BoardRepoFilter,
        &["Tab"],
        "repo_filter_toggle_mode",
        "Switch the filter between including and excluding the toggled repos",
    ),
    r(
        N::BoardRepoFilter,
        &["Backspace", "Delete"],
        "repo_filter_delete_repo_path",
        "Delete the repo path under the cursor from the saved list, after a confirmation",
    ),
    // ---- BoardPickerRepoPath ----
    r(
        N::BoardPickerRepoPath,
        &["Down", "Up"],
        "picker_move_cursor",
        "Move the cursor down or up the candidate list; every printable key filters it",
    ),
    r(
        N::BoardPickerRepoPath,
        &["Esc"],
        "cancel_input",
        "Cancel the task creation",
    ),
    r(
        N::BoardPickerRepoPath,
        &["Enter"],
        "submit_input",
        "Pick the path under the cursor, or the typed path when it is new",
    ),
    unrecorded(
        N::BoardPickerRepoPath,
        &["Backspace"],
        "text_backspace",
        "Delete the character before the caret",
    ),
    unrecorded(
        N::BoardPickerRepoPath,
        &["Delete"],
        "text_delete_forward",
        "Delete the character after the caret",
    ),
    unrecorded(
        N::BoardPickerRepoPath,
        &["Left"],
        "text_cursor_left",
        "Move the caret one character left",
    ),
    unrecorded(
        N::BoardPickerRepoPath,
        &["Right"],
        "text_cursor_right",
        "Move the caret one character right",
    ),
    unrecorded(
        N::BoardPickerRepoPath,
        &["Home"],
        "text_cursor_home",
        "Move the caret to the start of the field",
    ),
    unrecorded(
        N::BoardPickerRepoPath,
        &["End"],
        "text_cursor_end",
        "Move the caret to the end of the field",
    ),
    unrecorded(
        N::BoardPickerRepoPath,
        &["Ctrl+Left", "Alt+Left", "Alt+B"],
        "text_cursor_word_left",
        "Move the caret one word left",
    ),
    unrecorded(
        N::BoardPickerRepoPath,
        &["Ctrl+Right", "Alt+Right", "Alt+F"],
        "text_cursor_word_right",
        "Move the caret one word right",
    ),
    // ---- BoardPickerBaseBranch ----
    r(
        N::BoardPickerBaseBranch,
        &["Down", "Up"],
        "picker_move_cursor",
        "Move the cursor down or up the candidate list; every printable key filters it",
    ),
    r(
        N::BoardPickerBaseBranch,
        &["Esc"],
        "cancel_input",
        "Cancel the task creation",
    ),
    r(
        N::BoardPickerBaseBranch,
        &["Enter"],
        "submit_input",
        "Pick the branch under the cursor, or the typed branch when it is new",
    ),
    unrecorded(
        N::BoardPickerBaseBranch,
        &["Backspace"],
        "text_backspace",
        "Delete the character before the caret",
    ),
    unrecorded(
        N::BoardPickerBaseBranch,
        &["Delete"],
        "text_delete_forward",
        "Delete the character after the caret",
    ),
    unrecorded(
        N::BoardPickerBaseBranch,
        &["Left"],
        "text_cursor_left",
        "Move the caret one character left",
    ),
    unrecorded(
        N::BoardPickerBaseBranch,
        &["Right"],
        "text_cursor_right",
        "Move the caret one character right",
    ),
    unrecorded(
        N::BoardPickerBaseBranch,
        &["Home"],
        "text_cursor_home",
        "Move the caret to the start of the field",
    ),
    unrecorded(
        N::BoardPickerBaseBranch,
        &["End"],
        "text_cursor_end",
        "Move the caret to the end of the field",
    ),
    unrecorded(
        N::BoardPickerBaseBranch,
        &["Ctrl+Left", "Alt+Left", "Alt+B"],
        "text_cursor_word_left",
        "Move the caret one word left",
    ),
    unrecorded(
        N::BoardPickerBaseBranch,
        &["Ctrl+Right", "Alt+Right", "Alt+F"],
        "text_cursor_word_right",
        "Move the caret one word right",
    ),
    // ---- board.picker.tag ----
    r(
        N::BoardPickerTag,
        &["b", "f", "c", "v", "r", "x"],
        "tag_picker_select",
        "Pick the tag whose name holds the letter: [b]ug, [f]eature, [c]hore, pr re[v]iew, [r]esearch, fi[x]",
    ),
    r(
        N::BoardPickerTag,
        &["p"],
        "phoenix_arm",
        "Arm phoenix for the task, then pick its tag as usual; a second press does nothing",
    ),
    r(
        N::BoardPickerTag,
        &["Enter"],
        "tag_picker_default",
        "Continue with no tag",
    ),
    r(
        N::BoardPickerTag,
        &["Esc"],
        "tag_picker_cancel",
        "Cancel the task creation",
    ),
    // ---- board.picker.wrap_up_mode ----
    r(
        N::BoardPickerWrapUpMode,
        &["r", "p", "d"],
        "wrap_up_mode_picker_select",
        "Pick how the task wraps up: [r]ebase, [p]r or [d]one",
    ),
    r(
        N::BoardPickerWrapUpMode,
        &["Enter"],
        "wrap_up_mode_picker_default",
        "Continue with the default wrap-up mode",
    ),
    r(
        N::BoardPickerWrapUpMode,
        &["Esc"],
        "wrap_up_mode_picker_cancel",
        "Cancel the task creation",
    ),
    // ---- board.picker.quick_dispatch ----
    r(
        N::BoardPickerQuickDispatch,
        &["Down", "Up"],
        "quick_dispatch_move_cursor",
        "Move the cursor down or up the repo list; every printable key filters it",
    ),
    r(
        N::BoardPickerQuickDispatch,
        &["Enter"],
        "quick_dispatch_select",
        "Dispatch a new task in the repo under the cursor",
    ),
    r(
        N::BoardPickerQuickDispatch,
        &["Esc"],
        "quick_dispatch_cancel",
        "Cancel the quick dispatch",
    ),
    unrecorded(
        N::BoardPickerQuickDispatch,
        &["Backspace"],
        "text_backspace",
        "Delete the character before the caret",
    ),
    unrecorded(
        N::BoardPickerQuickDispatch,
        &["Delete"],
        "text_delete_forward",
        "Delete the character after the caret",
    ),
    unrecorded(
        N::BoardPickerQuickDispatch,
        &["Left"],
        "text_cursor_left",
        "Move the caret one character left",
    ),
    unrecorded(
        N::BoardPickerQuickDispatch,
        &["Right"],
        "text_cursor_right",
        "Move the caret one character right",
    ),
    unrecorded(
        N::BoardPickerQuickDispatch,
        &["Home"],
        "text_cursor_home",
        "Move the caret to the start of the field",
    ),
    unrecorded(
        N::BoardPickerQuickDispatch,
        &["End"],
        "text_cursor_end",
        "Move the caret to the end of the field",
    ),
    unrecorded(
        N::BoardPickerQuickDispatch,
        &["Ctrl+Left", "Alt+Left", "Alt+B"],
        "text_cursor_word_left",
        "Move the caret one word left",
    ),
    unrecorded(
        N::BoardPickerQuickDispatch,
        &["Ctrl+Right", "Alt+Right", "Alt+F"],
        "text_cursor_word_right",
        "Move the caret one word right",
    ),
    // ---- board.picker.move_to_epic ----
    r(
        N::BoardPickerMoveToEpic,
        &["j", "Down", "k", "Up", "l", "Right", "Space", "h", "Left"],
        "move_to_epic_picker_navigate",
        "Move through the epic tree: down (j, Down), up (k, Up), expand or descend (l, Right, Space), collapse or ascend (h, Left)",
    ),
    r(
        N::BoardPickerMoveToEpic,
        &["Enter"],
        "move_to_epic_picker_confirm",
        "Choose the highlighted epic and ask for confirmation",
    ),
    r(
        N::BoardPickerMoveToEpic,
        &["Esc", "q"],
        "move_to_epic_picker_cancel",
        "Close the picker without moving the task",
    ),
    // ---- board.picker.reparent_epic ----
    r(
        N::BoardPickerReparentEpic,
        &["j", "Down", "k", "Up", "l", "Right", "Space", "h", "Left"],
        "reparent_picker_navigate",
        "Move through the epic tree: down (j, Down), up (k, Up), expand or descend (l, Right, Space), collapse or ascend (h, Left)",
    ),
    r(
        N::BoardPickerReparentEpic,
        &["Enter"],
        "reparent_picker_confirm",
        "Choose the highlighted epic as the new parent and ask for confirmation",
    ),
    r(
        N::BoardPickerReparentEpic,
        &["Esc", "q"],
        "reparent_picker_cancel",
        "Close the picker without reparenting the epic",
    ),
    // ---- BoardConfirmQuit ----
    r(
        N::BoardConfirmQuit,
        &["y", "Y"],
        "confirm_quit_yes",
        "Quit dispatch",
    ),
    r(
        N::BoardConfirmQuit,
        &[ANY_OTHER_KEY],
        "confirm_quit_no",
        "Stay in dispatch",
    ),
    // ---- BoardConfirmDeleteTask ----
    r(
        N::BoardConfirmDeleteTask,
        &["y", "Y"],
        "confirm_delete_yes",
        "Permanently delete the task",
    ),
    r(
        N::BoardConfirmDeleteTask,
        &[ANY_OTHER_KEY],
        "confirm_delete_no",
        "Keep the task",
    ),
    // ---- BoardConfirmBatchDelete ----
    r(
        N::BoardConfirmBatchDelete,
        &["y", "Y"],
        "confirm_delete_yes",
        "Permanently delete the selected items",
    ),
    r(
        N::BoardConfirmBatchDelete,
        &[ANY_OTHER_KEY],
        "confirm_delete_no",
        "Keep the selected items",
    ),
    // ---- BoardConfirmDeleteEpic ----
    r(
        N::BoardConfirmDeleteEpic,
        &["y", "Y"],
        "confirm_delete_epic_yes",
        "Permanently delete the epic and its whole subtree",
    ),
    r(
        N::BoardConfirmDeleteEpic,
        &[ANY_OTHER_KEY],
        "confirm_delete_epic_no",
        "Keep the epic",
    ),
    // ---- BoardConfirmDone ----
    r(
        N::BoardConfirmDone,
        &["y", "Y"],
        "confirm_done_yes",
        "Move the task or tasks to Done",
    ),
    r(
        N::BoardConfirmDone,
        &[ANY_OTHER_KEY],
        "confirm_done_no",
        "Leave the task or tasks where they are",
    ),
    // ---- BoardConfirmRetry ----
    r(
        N::BoardConfirmRetry,
        &["r"],
        "confirm_retry_resume",
        "Resume the task in its existing worktree",
    ),
    r(
        N::BoardConfirmRetry,
        &["f"],
        "confirm_retry_fresh",
        "Kill the agent and start the task again from Backlog",
    ),
    r(
        N::BoardConfirmRetry,
        &["Esc"],
        "confirm_retry_no",
        "Close the dialog and leave the task as it is",
    ),
    // ---- BoardConfirmDetachTmux ----
    r(
        N::BoardConfirmDetachTmux,
        &["y", "Y"],
        "confirm_detach_tmux_yes",
        "Detach the tmux panel of the selected task or tasks",
    ),
    r(
        N::BoardConfirmDetachTmux,
        &[ANY_OTHER_KEY],
        "confirm_detach_tmux_no",
        "Keep the tmux panel",
    ),
    // ---- BoardConfirmOverrideFeedOwner ----
    r(
        N::BoardConfirmOverrideFeedOwner,
        &["y", "Y"],
        "confirm_override_feed_owner_yes",
        "Take over polling of the feed from the other machine",
    ),
    r(
        N::BoardConfirmOverrideFeedOwner,
        &[ANY_OTHER_KEY],
        "confirm_override_feed_owner_no",
        "Leave the other machine polling the feed",
    ),
    // ---- BoardConfirmMoveToEpic ----
    r(
        N::BoardConfirmMoveToEpic,
        &["y"],
        "confirm_move_task_to_epic_yes",
        "Confirm: move the task",
    ),
    r(
        N::BoardConfirmMoveToEpic,
        &["n"],
        "confirm_move_task_to_epic_no",
        "Go back to the picker",
    ),
    r(
        N::BoardConfirmMoveToEpic,
        &["Esc", "q"],
        "confirm_move_task_to_epic_cancel_all",
        "Cancel entirely, without going back to the picker",
    ),
    // ---- BoardConfirmReparentEpic ----
    r(
        N::BoardConfirmReparentEpic,
        &["y"],
        "confirm_reparent_epic_yes",
        "Confirm: reparent the epic",
    ),
    r(
        N::BoardConfirmReparentEpic,
        &["n"],
        "confirm_reparent_epic_no",
        "Go back to the picker",
    ),
    r(
        N::BoardConfirmReparentEpic,
        &["Esc", "q"],
        "confirm_reparent_epic_cancel_all",
        "Cancel entirely, without going back to the picker",
    ),
    // ---- BoardConfirmDeleteRepoPath ----
    r(
        N::BoardConfirmDeleteRepoPath,
        &["y", "Y"],
        "confirm_delete_repo_path_yes",
        "Delete the repo path from the saved list",
    ),
    r(
        N::BoardConfirmDeleteRepoPath,
        &[ANY_OTHER_KEY],
        "confirm_delete_repo_path_no",
        "Keep the repo path",
    ),
    // ---- BoardConfirmTrustRepo ----
    r(
        N::BoardConfirmTrustRepo,
        &["y", "Y"],
        "confirm_trust_repo_yes",
        "Trust the repo and dispatch the task",
    ),
    r(
        N::BoardConfirmTrustRepo,
        &[ANY_OTHER_KEY],
        "confirm_trust_repo_no",
        "Do not dispatch",
    ),
    // ---- BoardConfirmTrustRepoQuickDispatch ----
    r(
        N::BoardConfirmTrustRepoQuickDispatch,
        &["y", "Y"],
        "confirm_trust_repo_quick_dispatch_yes",
        "Trust the repo and quick-dispatch the task",
    ),
    r(
        N::BoardConfirmTrustRepoQuickDispatch,
        &[ANY_OTHER_KEY],
        "confirm_trust_repo_quick_dispatch_no",
        "Do not dispatch",
    ),
    // ---- BoardConfirmRepoSync ----
    r(
        N::BoardConfirmRepoSync,
        &["y", "Y"],
        "confirm_repo_sync_yes",
        "Sync the repository with origin: merge what it is behind by, push what it is ahead by",
    ),
    r(
        N::BoardConfirmRepoSync,
        &[ANY_OTHER_KEY],
        "confirm_repo_sync_no",
        "Leave the repository untouched",
    ),
    row(
        N::AgentTreeTree,
        &["q", "Ctrl+C"],
        None,
        "exit_pane",
        "Close the agent-tree pane",
        None,
    ),
    row(
        N::AgentTreeTree,
        &["Tab"],
        None,
        "toggle_focus",
        "Move the keys through the file tree, the Commits section and the Agents section below it (their titles read `Commits (Tab)` and `Agents (Tab)`); each section keeps its own cursor, so Tab back returns you where you were",
        None,
    ),
    row(
        N::AgentTreeTree,
        &["a"],
        None,
        "toggle_all_diffs",
        "Open the diff of every changed file, or close them all when any is open",
        None,
    ),
    row(
        N::AgentTreeTree,
        &["j", "Down", "k", "Up"],
        None,
        "navigate_row",
        "Move to the next row (j, Down) or the previous row (k, Up)",
        None,
    ),
    row(
        N::AgentTreeTree,
        &["gg"],
        None,
        "navigate_row_first",
        "Jump to the first visible row. A two-key chord with no timeout, unlike the board's gg: a lone g waits as long as you like, and any other key cancels it and then does its own job",
        None,
    ),
    row(
        N::AgentTreeTree,
        &["G"],
        None,
        "navigate_row_last",
        "Jump to the last visible row",
        None,
    ),
    row(
        N::AgentTreeTree,
        &["Ctrl+D", "Ctrl+U"],
        None,
        "navigate_half_page",
        "Move the cursor half a pane-height down (Ctrl+D) or up (Ctrl+U)",
        None,
    ),
    row(
        N::AgentTreeTree,
        &["h", "Left"],
        None,
        "collapse_directory",
        "Collapse the directory, or step out to the parent",
        None,
    ),
    row(
        N::AgentTreeTree,
        &["l", "Right"],
        Some(C::OnDirectory),
        "expand_directory",
        "Expand the selected directory (a file has nothing to expand, so the key does nothing there)",
        None,
    ),
    row(
        N::AgentTreeTree,
        &["Space", "Enter"],
        Some(C::OnFile),
        "toggle_diff",
        "Show or hide the file's diff in the pane below. Every badge opens, deleted included: a deleted file's diff is exactly its former contents",
        None,
    ),
    row(
        N::AgentTreeTree,
        &["Space", "Enter"],
        Some(C::OnDirectory),
        "toggle_directory",
        "Expand or collapse the directory",
        None,
    ),
    row(
        N::AgentTreeCommits,
        &["q", "Ctrl+C"],
        None,
        "exit_pane",
        "Close the agent-tree pane",
        None,
    ),
    row(
        N::AgentTreeCommits,
        &["Tab"],
        None,
        "toggle_focus",
        "Move the keys on to the Agents section; Tab cycles the file tree, Commits and Agents in turn",
        None,
    ),
    row(
        N::AgentTreeCommits,
        &["a"],
        None,
        "toggle_all_diffs",
        "Open the diff of every changed file, or close them all when any is open",
        None,
    ),
    row(
        N::AgentTreeCommits,
        &["j", "Down", "k", "Up"],
        None,
        "navigate_row",
        "Move to the next row (j, Down) or the previous row (k, Up)",
        None,
    ),
    row(
        N::AgentTreeCommits,
        &["gg"],
        None,
        "navigate_row_first",
        "Jump to the first row (unstaged work); the chord never times out",
        None,
    ),
    row(
        N::AgentTreeCommits,
        &["G"],
        None,
        "navigate_row_last",
        "Jump to the last row",
        None,
    ),
    row(
        N::AgentTreeCommits,
        &["Ctrl+D", "Ctrl+U"],
        None,
        "navigate_half_page",
        "Move half a page down (Ctrl+D) or up (Ctrl+U)",
        None,
    ),
    row(
        N::AgentTreeCommits,
        &["Space", "Enter"],
        None,
        "select_source",
        "Show the row under the cursor, unstaged work or one commit, in the tree and the diff pane. Does nothing on the row already shown",
        None,
    ),
    row(
        N::AgentTreeAgents,
        &["q", "Ctrl+C"],
        None,
        "exit_pane",
        "Close the agent-tree pane",
        None,
    ),
    row(
        N::AgentTreeAgents,
        &["Tab"],
        None,
        "toggle_focus",
        "Switch focus: file tree, Commits, Agents, and back to the tree",
        None,
    ),
    row(
        N::AgentTreeAgents,
        &["a"],
        None,
        "toggle_all_diffs",
        "Open the diff of every changed file, or close them all when any is open",
        None,
    ),
    row(
        N::AgentTreeAgents,
        &["j", "Down", "k", "Up"],
        None,
        "navigate_row",
        "Move to the next agent (j, Down) or the previous agent (k, Up)",
        None,
    ),
    row(
        N::AgentTreeAgents,
        &["gg"],
        None,
        "navigate_row_first",
        "Jump to the first agent; the chord never times out",
        None,
    ),
    row(
        N::AgentTreeAgents,
        &["G"],
        None,
        "navigate_row_last",
        "Jump to the last agent",
        None,
    ),
    row(
        N::AgentTreeAgents,
        &["Ctrl+D", "Ctrl+U"],
        None,
        "navigate_half_page",
        "Move half a page down (Ctrl+D) or up (Ctrl+U)",
        None,
    ),
    row(
        N::AgentTreeAgents,
        &["Space", "Enter"],
        None,
        "jump_to_agent",
        "Switch tmux to the agent's window. Does nothing on your own task or with no agent under the cursor",
        None,
    ),
    row(
        N::AgentDiff,
        &["q", "Ctrl+C"],
        None,
        "exit_pane",
        "Close the pane; your open files stay open and the next toggle in the tree brings it back",
        None,
    ),
    row(
        N::AgentDiff,
        &["j", "Down", "k", "Up"],
        None,
        "navigate_row",
        "Scroll down a line (j, Down) or up a line (k, Up)",
        None,
    ),
    row(
        N::AgentDiff,
        &["gg"],
        None,
        "navigate_row_first",
        "Jump to the top of the diff; the chord never times out",
        None,
    ),
    row(
        N::AgentDiff,
        &["G"],
        None,
        "navigate_row_last",
        "Jump to the bottom of the diff",
        None,
    ),
    row(
        N::AgentDiff,
        &["Ctrl+D", "Ctrl+U"],
        None,
        "navigate_half_page",
        "Scroll half a page down (Ctrl+D) or up (Ctrl+U)",
        None,
    ),
    // ---- tmux.global (catalogue only; tmux handles these) ----
    row(
        N::TmuxGlobal,
        &["Prefix+Space"],
        None,
        "jump_to_dispatch",
        "Jump back from an agent's window to the dispatch board. Press your tmux prefix, then Space",
        None,
    ),
    row(
        N::TmuxGlobal,
        &["Prefix+e"],
        None,
        "toggle_agent_tree",
        "Show or hide the agent-tree companion pane in whichever agent window you press it in; a no-op in windows that are not agent windows. Bound while the board runs and unbound when it exits",
        None,
    ),
];

/// The row a key runs: among `table`'s rows of `ns` whose context holds, the
/// one listing `key`, else the namespace's catch-all (`ANY_OTHER_KEY`) row.
/// `KeypressRunsItsRowsAction` in `docs/specs/keybindings.allium`.
pub fn lookup<'a>(
    table: &'a [KeyBinding],
    ns: KeyNamespace,
    key: &str,
    holds: impl Fn(KeyContext) -> bool,
) -> Option<&'a KeyBinding> {
    let applicable = || {
        table
            .iter()
            .filter(|b| b.namespace == ns && b.context.is_none_or(&holds))
    };
    // A modified press (Ctrl/Alt held) is answered only by a row listing its
    // modified name, never by the catch-all.
    let modified = key.starts_with("Ctrl+") || key.starts_with("Alt+");
    applicable().find(|b| b.keys.contains(&key)).or_else(|| {
        if modified {
            None
        } else {
            applicable().find(|b| b.keys.contains(&ANY_OTHER_KEY))
        }
    })
}

/// The `?` overlay's border title, built from `board.help`'s `scroll_help` and
/// `close_help` rows so rebinding either changes it
/// (`OverlayTitleNamesItsOwnKeys` in `docs/specs/keybindings.allium`).
pub fn help_overlay_title(table: &[KeyBinding]) -> String {
    let keys = |action: &str, sep: &str| {
        table
            .iter()
            .find(|b| b.namespace == KeyNamespace::BoardHelp && b.action == action)
            .map(|b| b.keys.join(sep))
            .unwrap_or_default()
    };
    format!(
        " Help \u{2014} {} scroll, {} close ",
        keys("scroll_help", "/"),
        keys("close_help", " or ")
    )
}

/// The rows of one namespace, in table order.
pub fn bindings_in(namespace: KeyNamespace) -> impl Iterator<Item = &'static KeyBinding> {
    KEY_BINDINGS
        .iter()
        .filter(move |b| b.namespace == namespace)
}

#[cfg(test)]
mod tests;
