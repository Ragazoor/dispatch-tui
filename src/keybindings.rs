//! The keybinding table: the one place every key the product answers to is
//! declared. See `docs/specs/keybindings.allium`.
//!
//! This module is a leaf (no `tui`, `db` or `mcp` dependency) so every reader
//! of the table - the key handlers, the `?` help overlay and the
//! `keybinding_search` MCP tool - can depend on it. It holds the data and the
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
        KeyCode::F(n) => format!("F{n}"),
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

/// A character after a Ctrl/Alt modifier, as the table writes it: uppercase
/// when the letter has a one-character uppercase form ("D", "É"), else as
/// typed. `is_key_name` accepts exactly this form.
pub(crate) fn modified_char(c: char) -> String {
    let mut upper = c.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(u), None) => u.to_string(),
        _ => c.to_string(),
    }
}

/// A key as the table writes it: "Space", "Enter", "j", "L"; with Ctrl or Alt
/// held, modifier then key with a letter in uppercase ("Ctrl+D", "Alt+B",
/// "Ctrl+Left"). Shift alone is never part of the name.
pub fn key_name(key: KeyEvent) -> String {
    let base = match key.code {
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(c) if key_has_ctrl_or_alt(key) => modified_char(c),
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
    BoardDescriptionEditor,
    BoardFilter,
    BoardFilterSearch,
    BoardPickerRepoPath,
    BoardPickerBaseBranch,
    BoardPickerTag,
    BoardPickerWrapUpMode,
    BoardPickerModel,
    BoardPickerQuickDispatch,
    BoardPickerMoveToEpic,
    BoardPickerReparentEpic,
    BoardPickerWorkPackage,
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
    BoardConfirmDeleteWorkPackage,
    AgentTreeTree,
    AgentTreeCommits,
    AgentTreeAgents,
    AgentDiff,
    TmuxGlobal,
}

impl KeyNamespace {
    /// Every namespace, in the order the spec's enum lists them. This is the
    /// order `keybinding_search` and the help overlay group by.
    pub const ALL: [KeyNamespace; 37] = [
        Self::BoardNormal,
        Self::BoardDetail,
        Self::BoardSearch,
        Self::BoardHelp,
        Self::BoardError,
        Self::BoardText,
        Self::BoardDescriptionEditor,
        Self::BoardFilter,
        Self::BoardFilterSearch,
        Self::BoardPickerRepoPath,
        Self::BoardPickerBaseBranch,
        Self::BoardPickerTag,
        Self::BoardPickerWrapUpMode,
        Self::BoardPickerModel,
        Self::BoardPickerQuickDispatch,
        Self::BoardPickerMoveToEpic,
        Self::BoardPickerReparentEpic,
        Self::BoardPickerWorkPackage,
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
        Self::BoardConfirmDeleteWorkPackage,
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
            Self::BoardDescriptionEditor => "board.description_editor",
            Self::BoardFilter => "board.filter",
            Self::BoardFilterSearch => "board.filter_search",
            Self::BoardPickerRepoPath => "board.picker.repo_path",
            Self::BoardPickerBaseBranch => "board.picker.base_branch",
            Self::BoardPickerTag => "board.picker.tag",
            Self::BoardPickerWrapUpMode => "board.picker.wrap_up_mode",
            Self::BoardPickerModel => "board.picker.model",
            Self::BoardPickerQuickDispatch => "board.picker.quick_dispatch",
            Self::BoardPickerMoveToEpic => "board.picker.move_to_epic",
            Self::BoardPickerReparentEpic => "board.picker.reparent_epic",
            Self::BoardPickerWorkPackage => "board.picker.work_package",
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
            Self::BoardConfirmDeleteWorkPackage => "board.confirm.delete_work_package",
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

/// Family names accepted by `keybinding_search` as a filter: a prefix standing
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
    InsideNestedView,
    NestedViewNoSearch,
    TopLevel,
    WithSelection,
    OnColumnSelectAll,
    OnFoldedSection,
    OnFoldedEpicGroup,
    OnEpicCard,
    OnWorkPackageCard,
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
    /// Every context, in the spec's order.
    pub const ALL: [KeyContext; 27] = [
        Self::SearchActive,
        Self::InsideEpicView,
        Self::InsideNestedView,
        Self::NestedViewNoSearch,
        Self::TopLevel,
        Self::WithSelection,
        Self::OnColumnSelectAll,
        Self::OnFoldedSection,
        Self::OnFoldedEpicGroup,
        Self::OnEpicCard,
        Self::OnWorkPackageCard,
        Self::OnTaskCard,
        Self::OnFlattenedTaskCard,
        Self::OnUnflattenedTaskCard,
        Self::OffEpicCard,
        Self::OnDirectory,
        Self::OnFile,
        Self::TaskOnOtherMachine,
        Self::TaskPinnedInSplit,
        Self::TaskWindowSplitOpen,
        Self::TaskWithWindow,
        Self::BacklogTask,
        Self::StuckTask,
        Self::TaskDispatching,
        Self::TaskWithWorktree,
        Self::TaskWithoutWorktree,
        Self::DetailZoomed,
    ];

    /// The spec's literal ("on_epic_card"): how the override file and
    /// keybinding_search name a context.
    pub fn literal(self) -> &'static str {
        match self {
            Self::SearchActive => "search_active",
            Self::InsideEpicView => "inside_epic_view",
            Self::InsideNestedView => "inside_nested_view",
            Self::NestedViewNoSearch => "nested_view_no_search",
            Self::TopLevel => "top_level",
            Self::WithSelection => "with_selection",
            Self::OnColumnSelectAll => "on_column_select_all",
            Self::OnFoldedSection => "on_folded_section",
            Self::OnFoldedEpicGroup => "on_folded_epic_group",
            Self::OnEpicCard => "on_epic_card",
            Self::OnWorkPackageCard => "on_work_package_card",
            Self::OnTaskCard => "on_task_card",
            Self::OnFlattenedTaskCard => "on_flattened_task_card",
            Self::OnUnflattenedTaskCard => "on_unflattened_task_card",
            Self::OffEpicCard => "off_epic_card",
            Self::OnDirectory => "on_directory",
            Self::OnFile => "on_file",
            Self::TaskOnOtherMachine => "task_on_other_machine",
            Self::TaskPinnedInSplit => "task_pinned_in_split",
            Self::TaskWindowSplitOpen => "task_window_split_open",
            Self::TaskWithWindow => "task_with_window",
            Self::BacklogTask => "backlog_task",
            Self::StuckTask => "stuck_task",
            Self::TaskDispatching => "task_dispatching",
            Self::TaskWithWorktree => "task_with_worktree",
            Self::TaskWithoutWorktree => "task_without_worktree",
            Self::DetailZoomed => "detail_zoomed",
        }
    }

    /// The context a literal names.
    pub fn parse(literal: &str) -> Option<KeyContext> {
        Self::ALL.iter().copied().find(|c| c.literal() == literal)
    }

    /// The plain words shown to readers; the literal is never shown.
    pub fn words(self) -> &'static str {
        match self {
            Self::SearchActive => "with a search active",
            Self::InsideEpicView => "inside an epic view",
            Self::InsideNestedView => "inside an epic or work package view",
            Self::NestedViewNoSearch => "inside an epic or work package view, no search active",
            Self::TopLevel => "on the top-level board",
            Self::WithSelection => {
                "on the top-level board, no search active, with cards selected or the cursor on a select-all row"
            }
            Self::OnColumnSelectAll => "on a column's select-all row",
            Self::OnFoldedSection => "on a folded section header",
            Self::OnFoldedEpicGroup => "on a folded epic group",
            Self::OnEpicCard => "on an epic card",
            Self::OnWorkPackageCard => "on a work package card",
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
            SearchActive | InsideEpicView | InsideNestedView | NestedViewNoSearch | TopLevel
            | WithSelection => ContextAxis::View,
            OnColumnSelectAll
            | OnFoldedSection
            | OnFoldedEpicGroup
            | OnEpicCard
            | OnWorkPackageCard
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
/// The loader's clash check and the drift gate read this one relation.
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
            // An epic view is one kind of nested view: InsideEpicView holds
            // only where InsideNestedView does.
            pair(SearchActive, InsideEpicView)
                || pair(SearchActive, InsideNestedView)
                || pair(SearchActive, TopLevel)
                || pair(InsideEpicView, InsideNestedView)
                || pair(InsideEpicView, NestedViewNoSearch)
                || pair(InsideNestedView, NestedViewNoSearch)
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
                OnWorkPackageCard,
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

mod overrides;
mod table;
pub use overrides::{
    check_overrides, is_key_name, load_key_table, read_override_entries, set_key_overrides,
    tmux_key_name, KeyChange, KeyLoadError, KeyOverrideProblem, KeySetError, KeySetOutcome,
    KeyTableLoad, KeyTableReader, KEY_OVERRIDE_FILE_NAME, NAMED_KEYS,
};
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

/// Whether a key name is a chord: exactly two characters pressed one after
/// the other ("gg"). A two-character name that is a named key ("Up", "F1" to
/// "F9") is that key, never a chord.
pub fn is_chord(key: &str) -> bool {
    key.chars().count() == 2 && !overrides::NAMED_KEYS.contains(&key)
}

/// Whether a press named `key` is the first half of a chord: some row of
/// `ns` in `table` lists a chord that starts with it. Such a press is
/// pending input, not a lookup (`KeypressRunsItsRowsAction`).
pub fn starts_chord(table: &[KeyBinding], ns: KeyNamespace, key: &str) -> Option<char> {
    let mut chars = key.chars();
    let (Some(first), None) = (chars.next(), chars.next()) else {
        return None;
    };
    table
        .iter()
        .filter(|b| b.namespace == ns)
        .flat_map(|b| b.keys.iter())
        .any(|k| is_chord(k) && k.starts_with(first))
        .then_some(first)
}

/// The chord a pending `first` and a press named `second` form, when a row of
/// `ns` in `table` lists it. `None` abandons the chord: the press is then
/// looked up on its own.
pub fn completed_chord(
    table: &[KeyBinding],
    ns: KeyNamespace,
    first: char,
    second: &str,
) -> Option<String> {
    let chord = format!("{first}{second}");
    (is_chord(&chord)
        && table
            .iter()
            .any(|b| b.namespace == ns && b.keys.contains(&chord.as_str())))
    .then_some(chord)
}

/// The key's name and label after any pending chord is resolved, or `None`
/// when the press only armed a chord and there is nothing to look up yet.
/// Taking the pending half disarms it unconditionally: a press that completes
/// a chord some row of `ns` lists is looked up as that chord, and any other
/// press is looked up on its own, having quietly cancelled it (or arms a
/// chord of its own). How long a pending half may wait is the caller's
/// business (the board's times out, the panes' never do).
pub fn resolve_chord(
    pending: &mut Option<char>,
    table: &[KeyBinding],
    ns: KeyNamespace,
    key: KeyEvent,
) -> Option<(String, String)> {
    let name = key_name(key);
    if let Some(first) = pending.take() {
        if let Some(chord) = completed_chord(table, ns, first, &name) {
            return Some((chord.clone(), chord));
        }
    }
    if let Some(first) = starts_chord(table, ns, &name) {
        *pending = Some(first);
        return None;
    }
    Some((name, key_label(key)))
}

/// The `?` overlay's border title, built from `board.help`'s
/// `scroll_help_down`, `scroll_help_up` and `close_help` rows so rebinding
/// any of them changes it (`OverlayTitleNamesItsOwnKeys` in
/// `docs/specs/keybindings.allium`).
pub fn help_overlay_title(table: &[KeyBinding]) -> String {
    let keys = |actions: &[&str], sep: &str| {
        actions
            .iter()
            .filter_map(|action| {
                table
                    .iter()
                    .find(|b| b.namespace == KeyNamespace::BoardHelp && b.action == *action)
            })
            .flat_map(|b| b.keys.iter().copied())
            .collect::<Vec<_>>()
            .join(sep)
    };
    format!(
        " Help \u{2014} {} scroll, {} close ",
        keys(&["scroll_help_down", "scroll_help_up"], "/"),
        keys(&["close_help"], " or ")
    )
}

/// The first key of `namespace`'s `action` row in `table`, as the table
/// writes it, for a hint built from the table (`PaneTitlesNameTheirOwnKeys`
/// in `docs/specs/keybindings.allium`). Empty when no such row exists, which
/// the drift tests rule out.
pub fn first_key(
    table: &'static [KeyBinding],
    namespace: KeyNamespace,
    action: &str,
) -> &'static str {
    bindings_in(table, namespace)
        .find(|b| b.action == action)
        .and_then(|b| b.keys.first().copied())
        .unwrap_or_default()
}

/// The key a hint shows for `namespace`'s `action` row in `table` (the row
/// guarded by `context` where the action has several): `shown` while the
/// row lists it, otherwise the row's first key, in the hints' short form
/// (`Del`, `↑`, `↓`). So a hint that shows one of a row's keys keeps its
/// text under the default table and names the user's key after a remap
/// (`HintsNameOnlyTableKeys` in `docs/specs/keybindings.allium`). Empty when
/// no such row exists, which the drift test reads as a failure.
pub fn hint_key(
    table: &'static [KeyBinding],
    namespace: KeyNamespace,
    context: Option<KeyContext>,
    action: &str,
    shown: &str,
) -> &'static str {
    #[cfg(test)]
    note_hint_row(namespace, action);
    let rows = || bindings_in(table, namespace).filter(|b| b.action == action);
    let row = rows()
        .find(|b| b.context == context)
        .or_else(|| rows().next());
    row.map_or("", |b| row_key(b, shown))
}

/// The key a hint shows for `row`: `shown` while the row lists it, else its
/// first key, in the hints' short form.
fn row_key(row: &'static KeyBinding, shown: &str) -> &'static str {
    let key = row
        .keys
        .iter()
        .copied()
        .find(|k| *k == shown)
        .or_else(|| row.keys.first().copied())
        .unwrap_or_default();
    short_key_form(key)
}

/// A hint naming several rows' keys at once, `/`-joined: `[j/k]` is
/// `hint_keys(table, ns, &[("…_down", "j"), ("…_up", "k")])`. Each part is
/// [`hint_key`] of the context-free row (or the first) of its action.
pub fn hint_keys(
    table: &'static [KeyBinding],
    namespace: KeyNamespace,
    parts: &[(&str, &str)],
) -> String {
    parts
        .iter()
        .map(|(action, shown)| hint_key(table, namespace, None, action, shown))
        .collect::<Vec<_>>()
        .join("/")
}

/// A hint naming several keys of one row, `/`-joined (`[Space/Enter]`): those
/// of `shown` the row lists, or the row's first key when it lists none.
pub fn hint_row_keys(
    table: &'static [KeyBinding],
    namespace: KeyNamespace,
    action: &str,
    shown: &[&str],
) -> String {
    #[cfg(test)]
    note_hint_row(namespace, action);
    let row = bindings_in(table, namespace).find(|b| b.action == action);
    let listed: Vec<&str> = row.map_or_else(Vec::new, |b| {
        shown
            .iter()
            .filter_map(|k| b.keys.iter().copied().find(|key| *key == *k))
            .collect()
    });
    if listed.is_empty() {
        return row.map_or("", |b| row_key(b, "")).to_string();
    }
    listed
        .into_iter()
        .map(short_key_form)
        .collect::<Vec<_>>()
        .join("/")
}

/// The `[y/n]` of a confirmation prompt in `namespace`: the key of its
/// `_yes` row, then the key of its `_no` row. A catch-all no row has no key
/// of its own and is drawn as `n` (`N` in the prompts whose no is the
/// default: taking over a feed, trusting a repo), unless another row of the
/// namespace lists that key: then the first of Esc and q no row lists
/// (`HintsNameOnlyTableKeys`).
pub fn yes_no_hint(table: &'static [KeyBinding], namespace: KeyNamespace) -> String {
    use KeyNamespace as N;
    let no = match namespace {
        N::BoardConfirmOverrideFeedOwner
        | N::BoardConfirmTrustRepo
        | N::BoardConfirmTrustRepoQuickDispatch => "N",
        _ => "n",
    };
    let row = |suffix: &str| {
        let row = bindings_in(table, namespace).find(|b| b.action.ends_with(suffix));
        #[cfg(test)]
        if let Some(b) = row {
            note_hint_row(namespace, b.action);
        }
        row
    };
    let yes = row("_yes").map_or("", |b| row_key(b, "y"));
    let no = match row("_no") {
        Some(b) if b.keys == [ANY_OTHER_KEY] => catch_all_keys(table, namespace, &[no]),
        Some(b) => row_key(b, no).to_string(),
        None => String::new(),
    };
    format!("[{yes}/{no}]")
}

/// The keys a hint draws for a catch-all row, which has no key of its own:
/// those of `shown` no row of `namespace` lists, `/`-joined, or else the
/// first of Esc and q no row lists (`HintsNameOnlyTableKeys`).
pub fn catch_all_keys(
    table: &'static [KeyBinding],
    namespace: KeyNamespace,
    shown: &[&str],
) -> String {
    let listed = |k: &str| bindings_in(table, namespace).any(|b| b.keys.contains(&k));
    let free: Vec<&str> = shown.iter().copied().filter(|k| !listed(k)).collect();
    if !free.is_empty() {
        return free.join("/");
    }
    ["Esc", "q"]
        .into_iter()
        .find(|k| !listed(k))
        .unwrap_or_default()
        .to_string()
}

/// `label` with `key` bracketed inside it where the label holds that
/// character (`[r]ebase`, `pr-re[v]iew`), else `[key] label`: the compact
/// form a picker prompt writes its value keys in, kept when the key is
/// rebound to one the label does not contain.
pub fn embedded_hint(key: &str, label: &str) -> String {
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if let Some(at) = label.find(c) {
            let (head, tail) = label.split_at(at);
            return format!("{head}[{key}]{}", &tail[c.len_utf8()..]);
        }
    }
    format!("[{key}] {label}")
}

#[cfg(test)]
thread_local! {
    /// The rows hint builders asked for while a test records them
    /// ([`record_hint_rows`]): which action each drawn hint stands for.
    static HINT_ROWS: std::cell::RefCell<Option<Vec<(KeyNamespace, String)>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn note_hint_row(namespace: KeyNamespace, action: &str) {
    HINT_ROWS.with(|rows| {
        if let Some(rows) = rows.borrow_mut().as_mut() {
            rows.push((namespace, action.to_string()));
        }
    });
}

/// Run `f` and return the rows ([`hint_key`], [`hint_row_keys`] and what
/// builds on them) it asked for a hint, so a test can rebind exactly the rows
/// a frame's hints stand for.
#[cfg(test)]
pub(crate) fn record_hint_rows<T>(f: impl FnOnce() -> T) -> (T, Vec<(KeyNamespace, String)>) {
    HINT_ROWS.with(|rows| *rows.borrow_mut() = Some(Vec::new()));
    let out = f();
    let asked = HINT_ROWS
        .with(|rows| rows.borrow_mut().take())
        .unwrap_or_default();
    (out, asked)
}

/// The keys a hint draws in a short form, and that form.
pub const SHORT_KEY_FORMS: [(&str, &str); 5] = [
    ("Delete", "Del"),
    ("Up", "\u{2191}"),
    ("Down", "\u{2193}"),
    ("Left", "\u{2190}"),
    ("Right", "\u{2192}"),
];

/// The short form a hint draws a key in ([`SHORT_KEY_FORMS`]).
pub fn short_key_form(key: &'static str) -> &'static str {
    SHORT_KEY_FORMS
        .iter()
        .find(|(long, _)| *long == key)
        .map_or(key, |(_, short)| short)
}

/// Every key of `namespace`'s `action` rows in `table`, in table order:
/// what a hint that names a row's keys shows (OverlayTitleNamesItsOwnKeys,
/// PaneStartupScreenIsOutsideUsage).
pub fn keys_of(
    table: &'static [KeyBinding],
    namespace: KeyNamespace,
    action: &str,
) -> Vec<&'static str> {
    #[cfg(test)]
    note_hint_row(namespace, action);
    bindings_in(table, namespace)
        .filter(|b| b.action == action)
        .flat_map(|b| b.keys.iter().copied())
        .collect()
}

/// The rows of one namespace of `table`, in table order. Every reader passes
/// the table its process loaded (`LoadKeyTable`), so what it shows or answers
/// is the user's keys where an override was accepted.
pub fn bindings_in(
    table: &'static [KeyBinding],
    namespace: KeyNamespace,
) -> impl Iterator<Item = &'static KeyBinding> {
    table.iter().filter(move |b| b.namespace == namespace)
}

/// The rows of one namespace of the DEFAULT table, for tests that ask about
/// the shipped keys. Test-only, so no reader can show the defaults by mistake.
#[cfg(test)]
pub(crate) fn default_bindings_in(
    namespace: KeyNamespace,
) -> impl Iterator<Item = &'static KeyBinding> {
    bindings_in(KEY_BINDINGS, namespace)
}

/// The opposite of a motion's action id: `_down` and `_up`, `_left` and
/// `_right`, `_expand` and `_collapse`, `_forward` and `_backward`,
/// `_next_` and `_previous_`. `None` for an id with no opposite.
#[cfg(test)]
pub(crate) fn opposite_action(action: &str) -> Option<String> {
    let suffixes = [
        ("_down", "_up"),
        ("_left", "_right"),
        ("_expand", "_collapse"),
        ("_forward", "_backward"),
    ];
    for (a, b) in suffixes {
        if let Some(stem) = action.strip_suffix(a) {
            return Some(format!("{stem}{b}"));
        }
        if let Some(stem) = action.strip_suffix(b) {
            return Some(format!("{stem}{a}"));
        }
    }
    if action.contains("_next_") {
        return Some(action.replacen("_next_", "_previous_", 1));
    }
    if action.contains("_previous_") {
        return Some(action.replacen("_previous_", "_next_", 1));
    }
    None
}

/// The value families whose rows pick one value each: the tag, wrap-up-mode
/// and model letters and the repo digits.
#[cfg(test)]
pub(crate) const VALUE_FAMILIES: [&str; 4] = [
    "tag_picker_select_",
    "wrap_up_mode_picker_select_",
    "model_picker_select_",
    "repo_filter_toggle_repo_",
];

/// The remapped table of `RemappedTableDrivesDispatch`
/// (`docs/specs/keybindings.allium`): every pair of opposite rows (one
/// namespace, one context) trades keys, the rows of every value family pass
/// their keys on to the next row of the family, and every other namespace
/// but tmux.global has one row moved to a key it did not list. Under it a handler that
/// reads the pressed key to pick a direction or a value goes the wrong way.
#[cfg(test)]
pub(crate) fn remapped_table() -> &'static [KeyBinding] {
    let mut rows: Vec<KeyBinding> = KEY_BINDINGS.to_vec();
    for row in &mut rows {
        let Some(other) = opposite_action(row.action) else {
            continue;
        };
        let partner = KEY_BINDINGS.iter().find(|b| {
            b.namespace == row.namespace && b.context == row.context && b.action == other
        });
        if let Some(partner) = partner {
            row.keys = partner.keys;
        }
    }
    for ns in KeyNamespace::ALL {
        for prefix in VALUE_FAMILIES {
            let family: Vec<usize> = (0..rows.len())
                .filter(|&i| rows[i].namespace == ns && rows[i].action.starts_with(prefix))
                .collect();
            for (n, &i) in family.iter().enumerate() {
                rows[i].keys = KEY_BINDINGS[family[(n + 1) % family.len()]].keys;
            }
        }
    }
    // Every other namespace an override may name gets one row rebound to a
    // key it did not list. A modified key, so a text namespace may take it.
    for ns in KeyNamespace::ALL {
        if ns == KeyNamespace::TmuxGlobal
            || (0..rows.len()).any(|i| rows[i].namespace == ns && rows[i] != KEY_BINDINGS[i])
        {
            continue;
        }
        let listed = |k: &&str| rows.iter().any(|b| b.namespace == ns && b.keys.contains(k));
        let Some(key) = REBIND_KEYS.iter().find(|k| !listed(k)) else {
            continue;
        };
        if let Some(row) = rows
            .iter_mut()
            .find(|b| b.namespace == ns && b.context.is_none() && b.keys != [ANY_OTHER_KEY])
        {
            row.keys = std::slice::from_ref(key);
        }
    }
    Box::leak(rows.into_boxed_slice())
}

/// Each key of every row of `ns` that `remapped` moved, with the default
/// key it stands in for: the key at the same place in the row's default list
/// (cycling when the new list is longer). Under the default table that key
/// runs the same row, so the two presses must have the same effect.
#[cfg(test)]
pub(crate) fn moved_keys(
    remapped: &'static [KeyBinding],
    ns: KeyNamespace,
) -> impl Iterator<Item = (&'static KeyBinding, &'static str, &'static str)> {
    remapped
        .iter()
        .zip(KEY_BINDINGS.iter())
        .filter(move |(row, default)| row.namespace == ns && row.keys != default.keys)
        .flat_map(|(row, default)| {
            row.keys
                .iter()
                .enumerate()
                .map(move |(n, key)| (row, *key, default.keys[n % default.keys.len()]))
        })
}

/// The pair or family a moved row belongs to, for asking whether a remap
/// test saw any of its rows take a visible effect: the value family's
/// prefix, the earlier of a motion and its opposite, or the action itself.
#[cfg(test)]
pub(crate) fn remap_group(action: &str) -> String {
    if let Some(family) = VALUE_FAMILIES.iter().find(|f| action.starts_with(**f)) {
        return (*family).to_string();
    }
    match opposite_action(action) {
        Some(other) => std::cmp::min(action.to_string(), other),
        None => action.to_string(),
    }
}

/// The default table with one row's keys replaced.
#[cfg(test)]
pub(crate) fn table_with_keys(
    ns: KeyNamespace,
    action: &str,
    keys: &'static [&'static str],
) -> &'static [KeyBinding] {
    let rows: Vec<KeyBinding> = KEY_BINDINGS
        .iter()
        .map(|b| {
            let mut b = *b;
            if b.namespace == ns && b.action == action {
                b.keys = keys;
            }
            b
        })
        .collect();
    Box::leak(rows.into_boxed_slice())
}

/// The table `reader` loads from a data directory whose override file holds
/// `json`, as a process builds it at start (`LoadKeyTable`). Panics when
/// the load refused anything, so a test of a reader cannot pass on a table
/// that quietly kept its defaults.
#[cfg(test)]
pub(crate) fn loaded_table(reader: KeyTableReader, json: &str) -> KeyTableLoad {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join(KEY_OVERRIDE_FILE_NAME), json).expect("write overrides");
    let load = load_key_table(dir.path(), reader);
    assert!(load.errors.is_empty(), "{:?}", load.errors);
    assert_ne!(load.table, KEY_BINDINGS, "the override changed nothing");
    load
}

/// The keys [`remapped_table`] rebinds a row to in a namespace whose rows it
/// does not otherwise move.
#[cfg(test)]
const REBIND_KEYS: [&str; 3] = ["Ctrl+T", "Ctrl+Y", "Ctrl+O"];

#[cfg(test)]
mod tests;
