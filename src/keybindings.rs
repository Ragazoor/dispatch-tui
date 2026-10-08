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

mod table;
pub use table::KEY_BINDINGS;

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
