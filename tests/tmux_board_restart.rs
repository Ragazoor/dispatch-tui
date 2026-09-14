#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Real-tmux integration tests for retiring the previous board and starting a
//! fresh one in its place — `docs/specs/startup.allium`'s
//! `RestartTheBoardInTheExistingSession` and `RetireTheBoardWindow`.
//!
//! # Why this needs a real tmux server
//!
//! The mock tests in src/startup.rs pin the argv dispatch hands tmux. They
//! cannot see the two things this path actually depends on, both of which are
//! tmux's own semantics rather than dispatch's:
//!
//! * **A session disappears with its last window.** `retire_board_window`
//!   returns "did the session survive?" precisely because tmux discards a
//!   session whose final window closes, and the launch path takes a different
//!   branch when it did. A mock answers whatever the test queued, so a mock
//!   test of that branch is a test of the test.
//! * **Exact-name targeting.** `=` is accepted by some tmux commands and
//!   silently ignored by others — the reason tests/tmux_window_targets.rs
//!   exists. `session_exists` and `new_window_in_session_running` both rely on
//!   it, against session targets rather than the window targets that file
//!   covers.
//!
//! # Isolation
//!
//! Private `-L` socket per test with drop-guard teardown, from the shared rig
//! in tests/tmux_harness/mod.rs.

mod tmux_harness;

use dispatch_tui::startup::{
    launch_context_inside, plan_launch, retire_board_window, LaunchPlan, RetireOutcome,
    StartupAbort, BOARD_WINDOW_NAME,
};
use dispatch_tui::tmux;

use tmux_harness::{tmux_available_or_skip, TmuxServer};

/// The session dispatch supplies for itself, as these tests name it. The real
/// name is `startup::SESSION_NAME`; using a different literal here keeps the
/// assertions about *a* session rather than about that constant's value.
const SESSION: &str = "board-restart";

/// The board's window name, bound once. `BOARD_WINDOW_NAME` is a `const`, so
/// each mention would otherwise materialise a fresh temporary to borrow from.
static BOARD: dispatch_tui::models::TmuxWindow = BOARD_WINDOW_NAME;

/// Start a server holding `SESSION`, whose first window is the board's and
/// whose remaining windows are agents.
fn server_with(board: bool, agents: &[&str]) -> TmuxServer {
    let server = TmuxServer::start();
    let first = if board { BOARD.as_str() } else { "shell" };
    server.tmux_ok(&["new-session", "-d", "-s", SESSION, "-n", first]);
    for agent in agents {
        server.tmux_ok(&["new-window", "-d", "-t", SESSION, "-n", agent]);
    }
    server
}

#[test]
fn retiring_the_board_leaves_every_agent_window_running() {
    if !tmux_available_or_skip() {
        return;
    }
    let server = server_with(true, &["task-42", "task-43"]);

    let outcome = retire_board_window(SESSION, &server.runner());

    assert_eq!(
        outcome,
        RetireOutcome::SessionReady,
        "agent windows keep the session alive"
    );
    assert!(
        !server.has_window(BOARD.as_str()),
        "the board's window must actually be gone"
    );
    assert_eq!(
        server.window_names(),
        vec!["task-42".to_string(), "task-43".to_string()],
        "RestartingCostsOnlyTheBoard: the operator's running agents survive"
    );
}

#[test]
fn retiring_the_board_reports_the_session_gone_when_it_was_the_last_window() {
    if !tmux_available_or_skip() {
        return;
    }
    let server = server_with(true, &[]);

    let outcome = retire_board_window(SESSION, &server.runner());

    assert_eq!(
        outcome,
        RetireOutcome::SessionDiscarded,
        "tmux discards a session with no windows, and the launch path must notice \
         so it can recreate rather than attach to nothing"
    );
    assert!(!tmux::session_exists(SESSION, &server.runner()));
}

#[test]
fn retiring_is_a_no_op_when_the_session_has_no_board_window() {
    if !tmux_available_or_skip() {
        return;
    }
    let server = server_with(false, &["task-42"]);

    let outcome = retire_board_window(SESSION, &server.runner());

    assert_eq!(outcome, RetireOutcome::SessionReady);
    assert_eq!(
        server.window_names(),
        vec!["shell".to_string(), "task-42".to_string()],
        "RetiringAnAbsentBoardWindowSucceeds: nothing is closed and nothing fails"
    );
}

#[test]
fn retiring_ignores_a_board_window_in_another_session() {
    if !tmux_available_or_skip() {
        return;
    }
    let server = server_with(false, &[]);
    server.tmux_ok(&[
        "new-session",
        "-d",
        "-s",
        "someone-elses",
        "-n",
        BOARD.as_str(),
    ]);

    let outcome = retire_board_window(SESSION, &server.runner());

    assert_eq!(outcome, RetireOutcome::SessionReady);
    assert!(
        server.has_window(BOARD.as_str()),
        "a window the operator named TUI in an unrelated session is not dispatch's board"
    );
}

#[test]
fn session_exists_does_not_prefix_match() {
    if !tmux_available_or_skip() {
        return;
    }
    let server = server_with(false, &[]);
    let runner = server.runner();

    assert!(tmux::session_exists(SESSION, &runner));
    assert!(
        !tmux::session_exists("board", &runner),
        "a prefix of the session's name must not resolve to it — the launch would \
         otherwise restart a board in a session the operator named for something else"
    );
}

#[test]
fn the_fresh_board_window_lands_in_the_named_session_and_is_selected() {
    if !tmux_available_or_skip() {
        return;
    }
    let server = server_with(false, &["task-42"]);
    // A second session whose name shares a prefix, so an inexact target would
    // have somewhere wrong to land.
    server.tmux_ok(&["new-session", "-d", "-s", "board-restart-other", "-n", "x"]);
    let runner = server.runner();

    tmux::new_window_in_session_running(SESSION, &BOARD, &["cat"], &runner)
        .expect("the board's window is created");

    assert_eq!(
        server.tmux_stdout(&[
            "list-windows",
            "-t",
            "=board-restart-other",
            "-F",
            "#{window_name}"
        ]),
        "x",
        "the prefix-sharing session must be untouched"
    );
    // Not `display-message -p -t <session>`: it accepts the target, exits zero
    // and prints nothing, which is the trap tests/tmux_window_targets.rs
    // documents. `list-windows` reports `window_active` honestly.
    let active: Vec<String> = server
        .tmux_stdout(&[
            "list-windows",
            "-t",
            "=board-restart",
            "-F",
            "#{window_active} #{window_name}",
        ])
        .lines()
        .filter_map(|l| l.strip_prefix("1 ").map(str::to_string))
        .collect();
    assert_eq!(
        active,
        vec![BOARD.as_str().to_string()],
        "the new window is the session's current one, so an attach lands on the board"
    );
}

#[test]
fn a_board_window_in_another_session_does_not_block_the_replacement() {
    if !tmux_available_or_skip() {
        return;
    }
    // The regression this pins: the duplicate-name refusal used to be
    // server-wide, so a `TUI` window anywhere — the operator having run
    // `dispatch tui` inside their own session, say — made the create bail.
    // The retire half had already killed the real board, so the launch
    // destroyed a board and started none.
    let server = server_with(true, &["task-42"]);
    server.tmux_ok(&[
        "new-session",
        "-d",
        "-s",
        "someone-elses",
        "-n",
        BOARD.as_str(),
    ]);
    let runner = server.runner();

    assert_eq!(
        retire_board_window(SESSION, &runner),
        RetireOutcome::SessionReady
    );
    tmux::new_window_in_session_running(SESSION, &BOARD, &["cat"], &runner)
        .expect("a board window in a session dispatch does not own must not refuse this");

    assert_eq!(
        server.count_windows_named(BOARD.as_str()),
        2,
        "one board window per session is the rule, not one per server"
    );
}

#[test]
fn a_board_window_already_in_this_session_refuses_a_second_one() {
    if !tmux_available_or_skip() {
        return;
    }
    let server = server_with(true, &[]);
    let runner = server.runner();

    tmux::new_window_in_session_running(SESSION, &BOARD, &["cat"], &runner)
        .expect_err("ExactlyOneBoardPerSession: a second board window in one session is refused");
    assert_eq!(server.count_windows_named(BOARD.as_str()), 1);
}

// -- The board must not refuse its own launch --------------------------------

/// The launch decision as the board's own process would reach it, composed from
/// the real tmux probe, the real predicate and the real planner.
fn plan_from(server: &TmuxServer) -> LaunchPlan {
    plan_from_pane(server, server.active_pane_id("=board-restart:").as_deref())
}

/// As the process in `pane` would reach it — the real probe, the real context
/// assembly, the real planner.
fn plan_from_pane(server: &TmuxServer, pane: Option<&str>) -> LaunchPlan {
    plan_launch(
        launch_context_inside(pane, &server.runner()),
        vec!["/bin/dispatch".to_string(), "tui".to_string()],
    )
}

#[test]
fn the_board_does_not_refuse_its_own_launch() {
    if !tmux_available_or_skip() {
        return;
    }
    // Both entry paths create the board's window already carrying the name, so
    // the board's own process runs the launch path from inside a window called
    // TUI. A name-only predicate refused it, and the operator attached to a
    // session whose board had just exited saying "This window is already the
    // dispatch board." Verified here against a real server, because that is
    // where the window's name and TMUX actually come from.
    let server = server_with(true, &[]);

    assert_eq!(
        plan_from(&server),
        LaunchPlan::DrawInThisWindow,
        "the board holds its window alone, so one pane means this IS the board, \
         and the window it would otherwise retire is its own"
    );
}

#[test]
fn a_shell_sharing_the_boards_window_is_refused() {
    if !tmux_available_or_skip() {
        return;
    }
    let server = server_with(true, &[]);
    // The split-pane agent, or the operator's own shell beside the board.
    server.tmux_ok(&["split-window", "-t", "=board-restart:", "-d"]);

    assert_eq!(
        plan_from(&server),
        LaunchPlan::Refuse(StartupAbort::BoardAlreadyInThisWindow),
        "retiring this window would close the shell issuing the command"
    );
}

#[test]
fn a_window_that_is_not_the_boards_is_never_refused() {
    if !tmux_available_or_skip() {
        return;
    }
    let server = server_with(false, &[]);
    server.tmux_ok(&["split-window", "-t", "=board-restart:", "-d"]);

    assert_eq!(
        plan_from(&server),
        LaunchPlan::ContinueHere {
            session: SESSION.to_string()
        },
        "two panes in a window that is not the board's says nothing about a board"
    );
}

#[test]
fn the_board_adopts_its_name_despite_a_tui_window_elsewhere() {
    if !tmux_available_or_skip() {
        return;
    }
    // The inside-tmux path is the only one that renames rather than creating.
    // The refusal behind it used to be server-wide, so a TUI window in any
    // other session stopped the board adopting the name — and a board without
    // the name is one the next launch in its session retires nothing of.
    let server = server_with(false, &[]);
    server.tmux_ok(&[
        "new-session",
        "-d",
        "-s",
        "someone-elses",
        "-n",
        BOARD.as_str(),
    ]);
    let runner = server.runner();
    let pane = server
        .active_pane_id("=board-restart:")
        .expect("the session's window has an active pane");

    tmux::rename_window_in_session(SESSION, &pane, &BOARD, &runner)
        .expect("a TUI window in a session dispatch does not own must not block this");

    assert_eq!(
        server.tmux_stdout(&[
            "list-windows",
            "-t",
            "=board-restart",
            "-F",
            "#{window_name}"
        ]),
        BOARD.as_str()
    );
}

#[test]
fn a_process_in_a_background_window_reads_its_own_window_not_the_active_one() {
    if !tmux_available_or_skip() {
        return;
    }
    // The defect this pins, and the reason it needs a real server: tmux's
    // `display-message -p` with no `-t` answers about the session's ACTIVE
    // window, not the caller's. A mock records argv and cannot see that. The
    // board reached the launch path from a sibling window, was told it was in
    // the window named TUI holding one pane, concluded it was the board tmux
    // had just started, and skipped the retire — so `dispatch tui` run from a
    // shell in the session left the old board running and drew a second one.
    let server = server_with(true, &["shell"]);
    let shell_pane = server
        .active_pane_id("=board-restart:shell")
        .expect("the shell window has a pane");

    // The board's window is the session's active one; the caller is not in it.
    let untargeted = tmux::current_window_context(None, &server.runner()).unwrap();
    assert_eq!(
        untargeted.window_name,
        BOARD.as_str(),
        "precondition: untargeted asks about the active window"
    );

    let mine = tmux::current_window_context(Some(&shell_pane), &server.runner()).unwrap();
    assert_eq!(mine.window_name, "shell", "targeted asks about the caller");

    assert_eq!(
        plan_from_pane(&server, Some(&shell_pane)),
        LaunchPlan::ContinueHere {
            session: SESSION.to_string()
        },
        "a launch from a sibling window must retire the board, not mistake itself for it"
    );
}
