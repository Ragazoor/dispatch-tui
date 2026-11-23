//! The board's two tmux prefix bindings, the jump back to the board and the
//! agent-tree toggle: the keys come from the tmux.global rows of the key table
//! the board loaded (`TmuxPrefixKeys` in `docs/specs/keybindings.allium`).

use crate::keybindings::{tmux_key_name, KeyBinding, KeyNamespace};
use crate::process::ProcessRunner;
use crate::tmux;

/// Default key (after the tmux prefix) that toggles a companion agent-tree
/// pane's visibility in whichever agent window it's pressed in. Matches
/// config.agent_tree_toggle_key in docs/specs/agent-tree.allium.
pub(crate) const AGENT_TREE_TOGGLE_KEY: &str = "e";

/// Default tmux key bound (after the prefix) to jump back to the board window.
pub(crate) const JUMP_BACK_KEY: &str = "Space";

/// Command bound to the agent-tree toggle key. `#{window_name}` is expanded
/// by tmux itself, before invoking the shell, to the name of whichever window
/// was focused when the key was pressed — so `dispatch toggle-agent-tree-pane`
/// is handed the target window without this process ever having to ask tmux
/// which window is focused. `-b` backgrounds the shell job so the keypress
/// doesn't block the tmux client.
pub(crate) const AGENT_TREE_TOGGLE_COMMAND: &str = concat!(
    "run-shell -b \"",
    crate::process::dispatch_program!(),
    " toggle-agent-tree-pane '#{window_name}'\""
);

/// Command bound to the jump-back key.
///
/// `=` anchors the target to an exact name match. tmux otherwise resolves a
/// `-t <name>` by prefix, so a window whose name merely starts with the
/// board's could absorb this jump. Unlike every other window target in the
/// codebase this one cannot go through `tmux::window_target`: the binding is
/// a string tmux executes later, and a pane ID captured now would be stale by
/// then. The sigil does work for `select-window` specifically (verified
/// against tmux 3.5a; it does not for `send-keys` or `set-option -w`).
pub fn jump_back_command() -> String {
    format!("select-window -t ={}", crate::startup::BOARD_WINDOW_NAME)
}

/// The keys the board binds after the tmux prefix, in tmux's spelling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixKeys {
    pub jump_back: String,
    pub agent_tree_toggle: String,
}

impl Default for PrefixKeys {
    fn default() -> Self {
        Self {
            jump_back: JUMP_BACK_KEY.to_string(),
            agent_tree_toggle: AGENT_TREE_TOGGLE_KEY.to_string(),
        }
    }
}

impl PrefixKeys {
    /// The keys of `table`'s jump_to_dispatch and toggle_agent_tree rows. A
    /// row whose key is not a tmux key keeps its default.
    pub fn from_table(table: &[KeyBinding]) -> Self {
        Self {
            jump_back: row_key(table, "jump_to_dispatch", JUMP_BACK_KEY),
            agent_tree_toggle: row_key(table, "toggle_agent_tree", AGENT_TREE_TOGGLE_KEY),
        }
    }
}

fn row_key(table: &[KeyBinding], action: &str, default: &str) -> String {
    let key = table
        .iter()
        .find(|b| b.namespace == KeyNamespace::TmuxGlobal && b.action == action)
        .and_then(|b| b.keys.first());
    match key.and_then(|k| tmux_key_name(k)) {
        Some(tmux_key) => tmux_key,
        None => {
            tracing::warn!(
                action,
                ?key,
                "not a tmux key; binding the default {default}"
            );
            default.to_string()
        }
    }
}

/// Whether `command`, as `list-keys` prints it, is one the board binds.
fn is_board_command(command: &str) -> bool {
    command == jump_back_command() || command == AGENT_TREE_TOGGLE_COMMAND
}

/// Bind `keys`, first removing every prefix binding that runs one of the
/// board's commands under another key, which a killed board left behind
/// (`NoStalePrefixKeySurvivesAStart`). A key tmux refuses falls back to its
/// default. Returns the keys bound, which are the ones to unbind, and an
/// error for the board to show when a key fell back. Best-effort: the board
/// starts whatever tmux answers.
pub fn bind_prefix_keys(
    keys: &PrefixKeys,
    runner: &dyn ProcessRunner,
) -> (PrefixKeys, Option<String>) {
    match tmux::prefix_bindings(runner) {
        Ok(bindings) => {
            for stale in bindings.iter().filter(|b| is_board_command(&b.command)) {
                let _ = tmux::unbind_key(&stale.key, runner);
            }
        }
        Err(e) => tracing::warn!("could not list tmux prefix bindings: {e:#}"),
    }
    let mut refused = Vec::new();
    let jump_back = bind_or_default(
        &keys.jump_back,
        JUMP_BACK_KEY,
        &jump_back_command(),
        runner,
        &mut refused,
    );
    let agent_tree_toggle = bind_or_default(
        &keys.agent_tree_toggle,
        AGENT_TREE_TOGGLE_KEY,
        AGENT_TREE_TOGGLE_COMMAND,
        runner,
        &mut refused,
    );
    let error = (!refused.is_empty()).then(|| {
        format!(
            "tmux refused the prefix key{} {}; the default is bound instead.",
            if refused.len() > 1 { "s" } else { "" },
            refused.join(", ")
        )
    });
    (
        PrefixKeys {
            jump_back,
            agent_tree_toggle,
        },
        error,
    )
}

/// Bind `key` to `command`, or `default` when tmux refuses a `key` that is
/// not the default. Returns the key it bound.
fn bind_or_default(
    key: &str,
    default: &str,
    command: &str,
    runner: &dyn ProcessRunner,
    refused: &mut Vec<String>,
) -> String {
    match tmux::bind_key(key, command, runner) {
        Ok(()) => key.to_string(),
        Err(e) if key != default => {
            tracing::warn!("tmux refused prefix key {key}: {e:#}; binding {default}");
            refused.push(format!("\"{key}\""));
            let _ = tmux::bind_key(default, command, runner);
            default.to_string()
        }
        Err(e) => {
            tracing::warn!("could not bind prefix key {key}: {e:#}");
            key.to_string()
        }
    }
}

/// Unbind the keys [`bind_prefix_keys`] bound (`TheBoardUnbindsWhatItBound`).
pub fn unbind_prefix_keys(keys: &PrefixKeys, runner: &dyn ProcessRunner) {
    let _ = tmux::unbind_key(&keys.jump_back, runner);
    let _ = tmux::unbind_key(&keys.agent_tree_toggle, runner);
}
