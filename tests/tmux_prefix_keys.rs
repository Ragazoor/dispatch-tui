#![allow(clippy::unwrap_used, clippy::expect_used)]
#![allow(clippy::disallowed_methods)] // tests: a hung child fails the run, not a board
//! Real-tmux tests for the board's prefix bindings (`TmuxPrefixKeys` in
//! `docs/specs/keybindings.allium`): the keys the table names are keys tmux
//! binds, a start removes the bindings a killed board left under other keys,
//! and the unbind removes what was bound. tmux prints a binding's key and
//! command in its own syntax, which a mock cannot answer for.
//!
//! # Isolation
//!
//! Private `-L` socket per test, from the shared rig in tests/tmux_harness/mod.rs.

mod tmux_harness;

use dispatch_tui::keybindings::{tmux_key_name, NAMED_KEYS};
use dispatch_tui::runtime::{bind_prefix_keys, unbind_prefix_keys, PrefixKeys};
use dispatch_tui::tmux::{prefix_bindings, PrefixBinding};

use tmux_harness::{tmux_available_or_skip, TmuxServer};

fn server() -> TmuxServer {
    let server = TmuxServer::start();
    server.tmux_ok(&["new-session", "-d", "-s", "prefix-keys"]);
    server
}

fn bindings(server: &TmuxServer) -> Vec<PrefixBinding> {
    prefix_bindings(&server.runner()).expect("list-keys")
}

fn command_of(server: &TmuxServer, key: &str) -> Option<String> {
    bindings(server)
        .into_iter()
        .find(|b| b.key == key)
        .map(|b| b.command)
}

#[test]
fn every_tmux_key_the_table_can_name_is_one_tmux_binds_and_unbinds() {
    if !tmux_available_or_skip() {
        return;
    }
    let server = server();
    let mut keys: Vec<String> = NAMED_KEYS.iter().map(|k| format!("Prefix+{k}")).collect();
    for k in [
        "e",
        "E",
        "#",
        ";",
        "\"",
        "~",
        "Ctrl+X",
        "Alt+B",
        "Ctrl+Alt+K",
        "Ctrl+Up",
    ] {
        keys.push(format!("Prefix+{k}"));
    }
    for key in keys {
        let tmux_key = tmux_key_name(&key).expect(&key);
        let marker = format!("display-message {}", key.len());
        server.tmux_ok(&["bind-key", &tmux_key, &marker]);
        assert!(
            bindings(&server)
                .iter()
                .any(|b| b.key == tmux_key && b.command == marker),
            "{key} as {tmux_key}: {:?}",
            bindings(&server)
        );
        server.tmux_ok(&["unbind-key", &tmux_key]);
        assert!(
            !bindings(&server).iter().any(|b| b.command == marker),
            "{key} as {tmux_key} is still bound"
        );
    }
}

// `NoStalePrefixKeySurvivesAStart`, `TheBoardUnbindsWhatItBound`: a board
// killed on the default keys, then a board started after the file moved both.
#[test]
fn a_start_moves_the_bindings_a_killed_board_left_and_the_exit_removes_them() {
    if !tmux_available_or_skip() {
        return;
    }
    let server = server();
    let runner = server.runner();
    server.tmux_ok(&["bind-key", "z", "select-window -t =other"]);
    bind_prefix_keys(&PrefixKeys::default(), &runner);
    let jump = command_of(&server, "Space").expect("Space bound");
    let toggle = command_of(&server, "e").expect("e bound");

    let moved = PrefixKeys {
        jump_back: tmux_key_name("Prefix+;").unwrap(),
        agent_tree_toggle: tmux_key_name("Prefix+Ctrl+T").unwrap(),
    };
    bind_prefix_keys(&moved, &runner);
    assert_eq!(command_of(&server, "Space"), None, "the old jump key");
    assert_eq!(command_of(&server, "e"), None, "the old toggle key");
    assert_eq!(command_of(&server, &moved.jump_back), Some(jump.clone()));
    assert_eq!(command_of(&server, "C-t"), Some(toggle.clone()));
    assert_eq!(
        command_of(&server, "z").as_deref(),
        Some("select-window -t =other"),
        "a binding that is not the board's stays"
    );

    unbind_prefix_keys(&moved, &runner);
    let left = bindings(&server);
    assert!(
        !left
            .iter()
            .any(|b| b.command == jump || b.command == toggle),
        "{left:?}"
    );
}
