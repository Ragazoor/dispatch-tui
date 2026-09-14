#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Every operator-facing startup abort message renders as prose.
//!
//! # The defect this guards against
//!
//! These messages are multi-line Rust string literals joined with `\`
//! line-continuations. Lose one and the literal keeps the source indentation:
//! the operator reads "...from a pane                  inside it...". It
//! compiles, every other test passes, and nothing about the source looks wrong
//! — the run of spaces is only visible in the rendered string. Three of the six
//! messages shipped that way before this file existed, and the third survived a
//! manual pass over the other two.
//!
//! Asserting on each message's exact wording would be worse than useless: it
//! would pin prose that is meant to be edited. The invariant is narrower — no
//! message contains a run of spaces, because no sentence wants one.

#[test]
fn every_startup_abort_message_reads_as_prose() {
    use dispatch_tui::startup::StartupAbort::*;
    for abort in [
        TmuxUnavailable,
        LaunchRejected,
        BoardAlreadyInThisWindow,
        PreviousBoardNotRetired,
        SessionUnidentified,
        AgentPortUnavailable { port: 8888 },
    ] {
        let msg = abort.message();
        assert!(
            !msg.contains("  "),
            "{abort:?} message has a run of spaces — a lost line continuation: {msg:?}"
        );
        assert!(!msg.trim().is_empty(), "{abort:?} has no message");
    }
}
