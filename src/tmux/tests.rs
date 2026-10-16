use super::*;

use crate::models::test_tmux_window;

// --- window_target scaffolding ---
//
// Every helper that takes a window *name* resolves it to a pane ID first, so
// these tests declare the fake server's windows with `with_windows` and
// assert the resolved `-t %N` target via `pane_id_of`. `with_windows` answers
// the lookup out of band, so response queues and `calls[N]` indices stay
// about the operation — see `MockProcessRunner::with_windows`.
//
// What these tests can and cannot do: they assert the argv we hand tmux, not
// what tmux does with it. The pre-fix versions pinned the vulnerable
// `-t task-42` argv and stayed green throughout. The behavioural coverage —
// that an absent name cannot reach a prefix-matched sibling — is in
// tests/tmux_window_targets.rs, against a real server.

/// The three-window topology that exposes the bug: `task-4`'s name is a
/// prefix of `task-42`'s.
const COLLIDING: [&str; 3] = ["dispatch", "task-4", "task-42"];

// --- ProcessRunner-based tests ---

use crate::process::MockProcessRunner;

// --- window_target ---
//
// These assert the lookup itself, so they queue its response positionally
// (`with_queued_window_lookup`) rather than letting the mock resolve out of
// band — the listing bytes *are* the fixture here.

/// A runner whose window lookup is answered from `responses`, for the tests
/// whose subject is resolution.
fn queued(responses: Vec<Result<Output>>) -> MockProcessRunner {
    MockProcessRunner::new(responses).with_queued_window_lookup()
}

mod keys_and_options;
mod panes;
mod window_lifecycle;
mod window_targets;
