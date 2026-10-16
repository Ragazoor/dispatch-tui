use super::*;
use crate::dispatch::mock_sequence::{DispatchScript, FinishRun, FINISH_TIMEOUT};

/// Drive `finish_task` under `script`, which declares the whole subprocess
/// sequence and asserts the recorded calls are exactly it — see
/// `docs/conventions.md`, "Driving a dispatch: `DispatchScript`, never a
/// hand-written queue"; the same rule holds for a finish.
fn run(script: &DispatchScript) -> FinishRun {
    script.drive_finish()
}

// finish_task is the git half only: a successful rebase + fast-forward
// issues no tmux call at all. The teardown belongs to the caller, gated on
// the task's terminal write landing (ExitSession in
// docs/specs/pr-workflow.allium).
#[test]
fn finish_task_issues_no_tmux_command() {
    let (calls, result) = run(&DispatchScript::finish().no_remote());

    result.expect("rebase + fast-forward succeeds");
    assert!(
        calls.iter().all(|(program, _)| program != "tmux"),
        "finish_task must not touch tmux: {calls:?}"
    );
}

// Pull runner returns Err (process could not be spawned) rather than a
// non-zero exit — maps to FinishError::Other via map_err.
#[test]
fn finish_task_pull_runner_error_returns_other() {
    let (_calls, result) = run(&DispatchScript::finish().pull_cannot_run());

    let err = result.unwrap_err();
    assert!(
        matches!(err, FinishError::Other(ref m) if m.contains("Failed to pull")),
        "pull runner error should map to FinishError::Other, got: {err}"
    );
}

// FF-only runner returns Err (process could not be spawned) — maps to
// FinishError::Other via map_err with "Failed to fast-forward" prefix.
#[test]
fn finish_task_ff_only_runner_error_returns_other() {
    let (_calls, result) = run(&DispatchScript::finish()
        .no_remote()
        .fast_forward_cannot_run());

    let err = result.unwrap_err();
    assert!(
        matches!(err, FinishError::Other(ref m) if m.contains("Failed to fast-forward")),
        "ff-only runner error should map to FinishError::Other, got: {err}"
    );
}

// A remote probe that could not be *run* is a git failure, not a finding
// that there is no remote — so it stops the operation rather than silently
// skipping the pull and rebasing against a base that was never refreshed.
#[test]
fn finish_task_reports_a_remote_probe_that_could_not_be_run() {
    // That nothing beyond the probe ran is pinned by `assert_matches` inside
    // `run`: the shape declares the probe as its last step, and an extra
    // call fails there.
    let (_calls, result) = run(&DispatchScript::finish().remote_probe_cannot_run());

    let err = result.unwrap_err();
    assert!(
        matches!(err, FinishError::Other(ref m) if m.contains("command not found")),
        "a probe that cannot run must carry why, got: {err}"
    );
}

// Rebase detects conflict via stdout CONFLICT marker (stderr is empty),
// and the conflicted file name is read from the worktree's own porcelain
// status (queried before the abort clears it) rather than parsed out of
// git's English rebase prose.
#[test]
fn finish_task_rebase_conflict_in_stdout_returns_rebase_conflict() {
    let (_calls, result) = run(&DispatchScript::finish()
        .no_remote()
        .rebase_conflicts_in_stdout(&["lib.rs"]));

    let err = result.unwrap_err();
    assert!(
        matches!(err, FinishError::RebaseConflict { ref files, .. } if files == &["lib.rs".to_string()]),
        "CONFLICT in stdout should map to RebaseConflict naming lib.rs, got: {err}"
    );
    assert!(
        err.to_string().contains("lib.rs"),
        "error message should name the conflicted file, got: {err}"
    );
}

// A dirty primary worktree is detected before any pull/rebase call, and
// reported as its own distinct error — not conflated with a rebase
// conflict.
#[test]
fn finish_task_dirty_primary_worktree_returns_error_before_pull() {
    // `assert_matches` inside `run` is what pins "and no pull, rebase or
    // merge followed": the shape declares the dirty check as its last step.
    let (_calls, result) =
        run(&DispatchScript::finish().dirty_primary(&["src/unrelated.rs", "scratch.txt"]));

    let err = result.unwrap_err();
    assert!(
        matches!(err, FinishError::DirtyPrimaryWorktree { ref path, ref files }
            if path == "/repo" && files == &["src/unrelated.rs".to_string(), "scratch.txt".to_string()]),
        "dirty primary worktree should be reported before any rebase attempt, got: {err}"
    );
    assert!(
        err.to_string().contains("/repo") && err.to_string().contains("uncommitted"),
        "error message should name the primary worktree and mention uncommitted changes, got: {err}"
    );
}

// --- subprocess bounding ---

// The pull is the only network call on the finish path. Unbounded, an origin
// that accepts the connection and then stalls hangs wrap_up forever: no exit
// token is ever minted, so the agent cannot close its session at all.
#[test]
fn finish_task_bounds_the_pull() {
    let (_calls, result) = run(&DispatchScript::finish().pull_times_out(FINISH_TIMEOUT));

    let err = result.unwrap_err();
    assert!(
        matches!(err, FinishError::Other(ref m) if m.contains("Failed to pull") && m.contains("timed out")),
        "a stalled pull must surface as a timed-out pull, got: {err}"
    );
}

// `git rebase` takes the worktree index lock, which another git process in the
// same checkout can hold indefinitely.
#[test]
fn finish_task_bounds_the_rebase() {
    let (_calls, result) = run(&DispatchScript::finish()
        .no_remote()
        .rebase_times_out(FINISH_TIMEOUT));

    let err = result.unwrap_err();
    assert!(
        matches!(err, FinishError::Other(ref m) if m.contains("Failed to run git rebase") && m.contains("timed out")),
        "a rebase blocked on the index lock must surface as a timeout, got: {err}"
    );
}

// Same for the fast-forward, which takes the repo root's index lock.
#[test]
fn finish_task_bounds_the_fast_forward() {
    let (_calls, result) = run(&DispatchScript::finish()
        .no_remote()
        .fast_forward_times_out(FINISH_TIMEOUT));

    let err = result.unwrap_err();
    assert!(
        matches!(err, FinishError::Other(ref m) if m.contains("Failed to fast-forward") && m.contains("timed out")),
        "a blocked fast-forward must surface as a timeout, got: {err}"
    );
}

// The test that pins the convention rather than three instances of it: every
// subprocess reachable on a successful finish carries the bound, including the
// three preflight reads reached through `crate::git`. A future unbounded call
// added anywhere on this path fails here.
//
// The preflight reads (rev-parse, status --porcelain, remote get-url) go
// through `crate::git` helpers bounded in Task 2 with the production
// `SUBPROCESS_TIMEOUT` constant, not the injected `FINISH_TIMEOUT` — only the
// pull/rebase/merge calls `finish_task` issues directly carry the context's
// `timeout` field. So the recorded timeouts are a mix of two different
// `Some(_)` durations, not all equal to one value. What matters here is that
// none of them is `None` — i.e. nothing on the path is unbounded.
#[test]
fn finish_task_bounds_every_subprocess_it_runs() {
    let script = DispatchScript::finish();
    let mock = script.runner();

    finish_task(&script.finish_context(), &mock).expect("rebase + fast-forward succeeds");

    script.assert_matches(&mock.recorded_calls());
    let timeouts = mock.recorded_timeouts();
    assert_eq!(
        timeouts.len(),
        6,
        "expected 6 subprocesses on this path (3 preflight + pull + rebase + merge), got {}: {:?}",
        timeouts.len(),
        mock.recorded_calls()
    );
    assert!(
        timeouts.iter().all(|t| t.is_some()),
        "every subprocess on the finish path must be bounded, got: {timeouts:?}"
    );
}

// The happy path above never reaches the conflict branch, so the abort and the
// porcelain read that precedes it need their own gate. Both are best-effort,
// so a timeout there degrades exactly as any other failure does — but neither
// may hang, which is what an unbounded call on a lock-taking abort would do.
// Timeouts are a mix of two durations for the reason given on
// `finish_task_bounds_every_subprocess_it_runs` above.
#[test]
fn finish_task_bounds_the_conflict_abort_path() {
    let script = DispatchScript::finish()
        .no_remote()
        .rebase_conflicts_in_stdout(&["lib.rs"]);
    let mock = script.runner();

    let err = finish_task(&script.finish_context(), &mock).unwrap_err();
    script.assert_matches(&mock.recorded_calls());
    assert!(
        matches!(err, FinishError::RebaseConflict { .. }),
        "expected a rebase conflict, got: {err}"
    );

    let timeouts = mock.recorded_timeouts();
    assert_eq!(
        timeouts.len(),
        6,
        "expected 6 subprocesses on this path (3 preflight + rebase + status read + abort), got {}: {:?}",
        timeouts.len(),
        mock.recorded_calls()
    );
    assert!(
        timeouts.iter().all(|t| t.is_some()),
        "the conflict read and the abort must be bounded too, got: {timeouts:?}"
    );
}
