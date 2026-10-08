//! Key handling for the agent-tree pane: the keybinding table resolved
//! against the focused section, tree motions and toggles, and the jump to an
//! agent's window. See `docs/specs/keybindings.allium` for the table and the
//! `AgentTreeCompanionPane` surface for what each key does.

use crossterm::event::{KeyCode, KeyEvent};

use crate::agent_tree::list_cursor::ListCursor;
use crate::agent_tree::model::{TreeNode, TreeNodeKind};
use crate::agent_tree::state::{Focus, RenderState};
use crate::models::TmuxWindow;

/// What the event loop should do after `handle_key` has processed a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    /// Stay in the loop and redraw.
    Continue,
    /// Leave the loop, which exits the process and so closes the tmux pane.
    Exit,
    /// The set of open diffs moved. The loop reconciles the diff pane with it —
    /// splitting one when the set became non-empty and no pane exists, killing
    /// it when the set emptied. `handle_key` stays pure: every tmux call
    /// belongs to the loop.
    DiffSetChanged,
    /// Space or Enter on another agent's row: select its window. Kept out of
    /// `handle_key` for the same reason as `DiffSetChanged` — tmux belongs to
    /// the loop.
    JumpTo(TmuxWindow),
    /// Space or Enter in the commits section selected a different source
    /// (`SelectAgentTreeSource`). The loop drops the old source's tree,
    /// publishes the selection and re-queries — see [`crate::agent_tree::run::adopt_selected_source`].
    SourceChanged,
}

/// The node the widget's current selection names, if any. A selection path is
/// exactly a node's chain of `name`s below the root — which for a chain-merged
/// directory is a whole route, not one segment (see `build_tree_items` and
/// `sync_expansion_at`) — so resolving it is `TreeNode::node_at`.
///
/// Fails closed, and this is the one place that rule is stated — every key that
/// acts on the selection goes through here: `None` for an empty selection —
/// which `node_at` would otherwise resolve to the root, itself a directory —
/// and `None` for a path that resolves to nothing, i.e. a stale selection left
/// over from before a rebuild.
fn selected_node<'a>(root: &'a TreeNode, selected: &[String]) -> Option<&'a TreeNode> {
    if selected.is_empty() {
        return None;
    }
    root.node_at(selected)
}

/// Apply one key press to the view state — see `docs/specs/agent-tree.allium`'s
/// `AgentTreeCompanionPane` surface for the bindings. One-step cursor and
/// expansion keys each have a vim motion and an arrow key bound to the same
/// action; the four jump motions (`gg`, `G`, `Ctrl-D`, `Ctrl-U`) are vim-only.
///
/// Space and Enter dispatch on the selected node's kind — a directory toggles, a
/// file opens — and expand is directory-only, which is why `root` is a
/// parameter: `tui_tree_widget`'s `open()` has no leaf guard, so without the tree
/// to consult, `l` on a *file* would record a phantom open that the next `h`
/// silently consumes instead of stepping out to the parent (#3834). Collapse
/// needs no guard — on a file it always falls through to stepping out.
///
/// Pure with respect to everything but `state`, so the loop's key handling is
/// testable without a terminal, an event source, or a tmux server.
///
/// Wraps [`dispatch_key`] to record what the press did to the widget's
/// expansion state, which is what lets a manual collapse outlive a row being
/// renamed — see [`RenderState::user_collapsed`]. Every key goes through the
/// recording, including the ones that cannot move expansion: enumerating the
/// exceptions would be one more list to keep in step with the match below.
pub fn handle_key(state: &mut RenderState, root: &TreeNode, key: KeyEvent) -> KeyAction {
    let opened_before = state.tree_state.opened().clone();
    let action = dispatch_key(state, root, key);
    state.absorb_manual_expansion(&opened_before);
    action
}

/// Whether one [`crate::keybindings::KeyContext`] holds for the tree pane's
/// selection. Only the file-tree contexts exist here; a stale or empty
/// selection satisfies neither.
fn tree_context_holds(
    state: &RenderState,
    root: &TreeNode,
    context: crate::keybindings::KeyContext,
) -> bool {
    use crate::keybindings::KeyContext as C;
    let kind = selected_node(root, state.tree_state.selected()).map(|n| n.kind);
    match context {
        C::OnDirectory => kind == Some(TreeNodeKind::Directory),
        C::OnFile => kind == Some(TreeNodeKind::File),
        _ => false,
    }
}

fn dispatch_key(state: &mut RenderState, root: &TreeNode, key: KeyEvent) -> KeyAction {
    use crate::keybindings::{lookup, KeyNamespace, KEY_BINDINGS};
    // Any key acknowledges a notice — docs/specs/agent-tree.allium's
    // ClearAgentTreeErrorNotice. Cleared before dispatching, so a key that sets
    // a fresh one wins.
    state.notice = None;

    // The `gg` chord: its first half is pending input, not a lookup, and only
    // the completed chord is looked up. Taking the flag disarms it
    // unconditionally: a second `g` completes the chord, and any other key
    // falls through to its own row having quietly cancelled it. See
    // AgentTreeGgChordNeverExpires in docs/specs/agent-tree.allium — there is
    // no deadline, so the only thing that can end a pending chord is the next
    // key, whenever it comes.
    let Some((name, label)) = resolve_gg_chord(state, key) else {
        return KeyAction::Continue;
    };

    // The table decides which action this press runs: the pane-wide keys are
    // rows in all three sections (AgentKeysFollowFocus), the cursor keys rows of
    // the focused one. A press with no row does nothing.
    let ns = match state.focus {
        Focus::Tree => KeyNamespace::AgentTreeTree,
        Focus::Commits => KeyNamespace::AgentTreeCommits,
        Focus::Agents => KeyNamespace::AgentTreeAgents,
    };
    let Some(row) = lookup(KEY_BINDINGS, ns, &name, |c| {
        tree_context_holds(state, root, c)
    }) else {
        return KeyAction::Continue;
    };
    let action = row.action;
    match run_tree_action(state, root, ns, action, key) {
        Some(result) => {
            state
                .usage
                .push(crate::models::UsageEvent::pane_key(action, &label));
            result
        }
        None => KeyAction::Continue,
    }
}

/// The key's name and label after the `gg` chord is resolved, or `None` when
/// the press only armed the chord and there is nothing to look up yet.
fn resolve_gg_chord(state: &mut RenderState, key: KeyEvent) -> Option<(String, String)> {
    use crate::keybindings::{key_label, key_name};
    let was_pending_g = std::mem::take(&mut state.pending_g);
    let name = key_name(key);
    if name != "g" {
        return Some((name, key_label(key)));
    }
    if !was_pending_g {
        state.pending_g = true;
        return None;
    }
    Some(("gg".to_string(), "gg".to_string()))
}

/// Run one resolved action. `None` means the action had no effect (nothing to
/// jump to, or an unknown id), so the press records no usage event.
fn run_tree_action(
    state: &mut RenderState,
    root: &TreeNode,
    ns: crate::keybindings::KeyNamespace,
    action: &str,
    key: KeyEvent,
) -> Option<KeyAction> {
    let half_page = state.half_page();
    let result = match action {
        "exit_pane" => KeyAction::Exit,
        "toggle_focus" => {
            state.focus = match state.focus {
                Focus::Tree => Focus::Commits,
                Focus::Commits => Focus::Agents,
                Focus::Agents => Focus::Tree,
            };
            KeyAction::Continue
        }
        // The all-files key. Unlike Space/Enter it does NOT dispatch on the
        // selection — it acts on the whole tree, whatever the cursor is on,
        // including a directory or nothing at all.
        "toggle_all_diffs" => {
            state.toggle_all_diffs(root);
            KeyAction::DiffSetChanged
        }
        // `TreeState`'s navigation methods return whether anything changed;
        // the loop redraws unconditionally, so the answer is discarded. The
        // jump motions resolve against the identifiers of the last render —
        // the visible rows — so a collapsed directory's children are skipped
        // and nothing is expanded to reach a target. With nothing selected
        // yet they all land on the first row, as `j`/`k` do from that state.
        "navigate_row" => {
            navigate_row(state, ns, key);
            KeyAction::Continue
        }
        "navigate_row_first" => {
            navigate_to_edge(state, ns, Edge::First);
            KeyAction::Continue
        }
        "navigate_row_last" => {
            navigate_to_edge(state, ns, Edge::Last);
            KeyAction::Continue
        }
        "navigate_half_page" => {
            navigate_half_page(state, ns, key, half_page);
            KeyAction::Continue
        }
        "collapse_directory" => {
            state.tree_state.key_left();
            KeyAction::Continue
        }
        "expand_directory" => {
            state.tree_state.key_right();
            KeyAction::Continue
        }
        // No badge guard on the diff, and deliberately none. The editor this
        // replaced refused a node badged Deleted, because an editor given a
        // missing path opens a misleading empty buffer. A diff has the
        // opposite property: a deleted file's diff is exactly its former
        // contents, so deleted is the case where opening it is most useful.
        // See OpenAgentTreeFileDiff in docs/specs/agent-tree.allium.
        "toggle_diff" => {
            let selected = state.tree_state.selected();
            state.toggle_diff(selected.iter().collect());
            KeyAction::DiffSetChanged
        }
        "toggle_directory" => {
            state.tree_state.toggle_selected();
            KeyAction::Continue
        }
        "jump_to_agent" => return state.agents.jump_target().map(KeyAction::JumpTo),
        // Space or Enter in the commits section: the row under the cursor
        // becomes the source. The row already selected is a no-op, so it
        // records nothing (SelectAgentTreeSource).
        "select_source" => {
            let target = state.commits.cursor_commit().map(|c| c.id.clone());
            if target == state.selected_commit {
                return None;
            }
            state.selected_commit = target;
            KeyAction::SourceChanged
        }
        _ => return None,
    };
    Some(result)
}

/// Which end of a section a jump motion lands on.
enum Edge {
    First,
    Last,
}

/// `j`/`k` and the arrow keys: one row in the focused section.
// `TreeState`'s navigation methods return whether anything changed; the loop
// redraws unconditionally, so the answer is discarded. The jump motions
// resolve against the identifiers of the last render — the visible rows — so
// a collapsed directory's children are skipped and nothing is expanded to
// reach a target. With nothing selected yet they all land on the first row,
// as `j`/`k` do from that state.
fn navigate_row(state: &mut RenderState, ns: crate::keybindings::KeyNamespace, key: KeyEvent) {
    use crate::keybindings::KeyNamespace;
    let down = matches!(key.code, KeyCode::Char('j') | KeyCode::Down);
    match (ns, down) {
        (KeyNamespace::AgentTreeAgents, true) => state.agents.down(),
        (KeyNamespace::AgentTreeAgents, false) => state.agents.up(),
        (KeyNamespace::AgentTreeCommits, true) => state.commits.down(),
        (KeyNamespace::AgentTreeCommits, false) => state.commits.up(),
        (_, true) => {
            state.tree_state.key_down();
        }
        (_, false) => {
            state.tree_state.key_up();
        }
    }
}

/// `gg` and `G`: jump to the first or last row of the focused section.
fn navigate_to_edge(state: &mut RenderState, ns: crate::keybindings::KeyNamespace, edge: Edge) {
    use crate::keybindings::KeyNamespace;
    match (ns, edge) {
        (KeyNamespace::AgentTreeAgents, Edge::First) => state.agents.top(),
        (KeyNamespace::AgentTreeAgents, Edge::Last) => state.agents.bottom(),
        (KeyNamespace::AgentTreeCommits, Edge::First) => state.commits.top(),
        (KeyNamespace::AgentTreeCommits, Edge::Last) => state.commits.bottom(),
        (_, Edge::First) => {
            state.tree_state.select_first();
        }
        (_, Edge::Last) => {
            state.tree_state.select_last();
        }
    }
}

/// `d`/`u`: half a page in the focused section.
fn navigate_half_page(
    state: &mut RenderState,
    ns: crate::keybindings::KeyNamespace,
    key: KeyEvent,
    half_page: usize,
) {
    use crate::keybindings::KeyNamespace;
    let down = matches!(key.code, KeyCode::Char('d' | 'D'));
    match (ns, down) {
        (KeyNamespace::AgentTreeAgents, true) => state.agents.half_page_down(),
        (KeyNamespace::AgentTreeAgents, false) => state.agents.half_page_up(),
        (KeyNamespace::AgentTreeCommits, true) => state.commits.half_page_down(),
        (KeyNamespace::AgentTreeCommits, false) => state.commits.half_page_up(),
        (_, true) => {
            state
                .tree_state
                .select_relative(|current| current.map_or(0, |c| c.saturating_add(half_page)));
        }
        (_, false) => {
            state
                .tree_state
                .select_relative(|current| current.map_or(0, |c| c.saturating_sub(half_page)));
        }
    }
}
