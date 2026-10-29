use super::*;
use crate::process::MockProcessRunner;

fn porcelain(stdout: &[u8]) -> std::process::Output {
    std::process::Output {
        status: crate::process::exit_ok(),
        stdout: stdout.to_vec(),
        stderr: vec![],
    }
}

// --- porcelain helpers (moved here from dispatch::finish) ---

#[test]
fn parse_porcelain_files_keeps_leading_space_status_codes() {
    let out = porcelain(b" M src/unrelated.rs\n?? scratch.txt\n");
    assert_eq!(
        parse_porcelain_files(&out),
        vec!["src/unrelated.rs".to_string(), "scratch.txt".to_string()]
    );
}

#[test]
fn parse_porcelain_files_is_empty_for_a_clean_tree() {
    assert!(parse_porcelain_files(&porcelain(b"")).is_empty());
}

#[test]
fn parse_porcelain_files_skips_lines_too_short_to_carry_a_path() {
    assert!(parse_porcelain_files(&porcelain(b"M\n")).is_empty());
}

#[test]
fn parse_unmerged_files_selects_only_conflict_codes() {
    let out =
        porcelain(b"UU lib.rs\nAA added.rs\nDD gone.rs\nAU theirs.rs\n M clean.rs\n?? new.rs\n");
    assert_eq!(
        parse_unmerged_files(&out),
        vec![
            "lib.rs".to_string(),
            "added.rs".to_string(),
            "gone.rs".to_string(),
            "theirs.rs".to_string(),
        ]
    );
}

#[test]
fn parse_unmerged_files_is_empty_when_only_dirty_paths_exist() {
    assert!(parse_unmerged_files(&porcelain(b" M src/a.rs\n?? b.txt\n")).is_empty());
}

// --- detect_default_branch ---

#[test]
fn detect_default_branch_returns_remote_head_when_set() {
    let runner = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        b"refs/remotes/origin/master\n",
    )]);
    assert_eq!(detect_default_branch("/repo", &runner), "master");
}

#[test]
fn detect_default_branch_keeps_a_slash_in_the_branch_name() {
    let runner = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        b"refs/remotes/origin/release/2024\n",
    )]);
    assert_eq!(detect_default_branch("/repo", &runner), "release/2024");
}

#[test]
fn dirty_files_fails_closed_when_git_status_exits_non_zero() {
    let runner =
        MockProcessRunner::new(vec![MockProcessRunner::fail("fatal: not a git repository")]);
    let err = dirty_files("/repo", &runner).unwrap_err();
    assert!(err.contains("not a git repository"), "got: {err}");
}

#[test]
fn current_branch_fails_closed_when_git_exits_non_zero() {
    let runner =
        MockProcessRunner::new(vec![MockProcessRunner::fail("fatal: not a git repository")]);
    let err = current_branch("/repo", &runner).unwrap_err();
    assert!(err.contains("not a git repository"), "got: {err}");
}

#[test]
fn detect_default_branch_falls_back_when_origin_head_missing() {
    let runner = MockProcessRunner::new(vec![MockProcessRunner::fail(
        "fatal: ref refs/remotes/origin/HEAD is not a symbolic ref",
    )]);
    assert_eq!(detect_default_branch("/repo", &runner), "main");
}

#[test]
fn detect_default_branch_falls_back_when_runner_errors() {
    let runner = MockProcessRunner::new(vec![Err(anyhow::anyhow!("git not on PATH"))]);
    assert_eq!(detect_default_branch("/repo", &runner), "main");
}

// --- has_origin_remote ---

#[test]
fn has_origin_remote_reports_a_configured_remote() {
    let runner = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        b"git@github.com:org/repo.git\n",
    )]);
    assert_eq!(has_origin_remote("/repo", &runner), Ok(true));
}

#[test]
fn has_origin_remote_reports_a_repo_without_one() {
    let runner = MockProcessRunner::new(vec![MockProcessRunner::fail(
        "error: No such remote 'origin'",
    )]);
    assert_eq!(has_origin_remote("/repo", &runner), Ok(false));
}

// The point of the Result: a probe that could not be *run* is not a positive
// finding that there is no remote, and callers must be able to tell the two
// apart rather than have `git.rs` collapse them on their behalf.
#[test]
fn has_origin_remote_distinguishes_a_probe_that_could_not_be_run() {
    let runner = MockProcessRunner::new(vec![Err(anyhow::anyhow!("git not on PATH"))]);
    let err = has_origin_remote("/repo", &runner)
        .expect_err("a probe that cannot be run is not an answer");
    assert!(
        err.contains("git not on PATH"),
        "the failure must carry why the probe could not run, got: {err}"
    );
}

#[test]
fn has_origin_remote_invokes_remote_get_url_origin() {
    let runner = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    let _ = has_origin_remote("/some/repo", &runner);
    let calls = runner.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "git");
    assert_eq!(
        calls[0].1,
        vec![
            "-C".to_string(),
            "/some/repo".to_string(),
            "remote".to_string(),
            "get-url".to_string(),
            "origin".to_string(),
        ]
    );
}

#[test]
fn detect_default_branch_invokes_correct_git_command() {
    let runner = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        b"refs/remotes/origin/main\n",
    )]);
    let _ = detect_default_branch("/some/repo", &runner);
    let calls = runner.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "git");
    assert_eq!(
        calls[0].1,
        vec![
            "-C".to_string(),
            "/some/repo".to_string(),
            "symbolic-ref".to_string(),
            "refs/remotes/origin/HEAD".to_string(),
        ]
    );
}

// --- subprocess bounding ---
//
// These four helpers are issued on the wrap-up rebase path (finish_task), and
// every one of them can block on a repository lock — routinely, since a human
// often has the same checkout open while an agent wraps up. A bare `run`
// wedges the caller with no way out, so each must carry the bound.

#[test]
fn detect_default_branch_bounds_its_subprocess() {
    let runner = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        b"refs/remotes/origin/main\n",
    )]);
    let _ = detect_default_branch("/repo", &runner);
    assert_eq!(runner.recorded_timeouts(), vec![Some(SUBPROCESS_TIMEOUT)]);
}

#[test]
fn has_origin_remote_bounds_its_subprocess() {
    let runner = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    let _ = has_origin_remote("/repo", &runner);
    assert_eq!(runner.recorded_timeouts(), vec![Some(SUBPROCESS_TIMEOUT)]);
}

#[test]
fn current_branch_bounds_its_subprocess() {
    let runner = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"main\n")]);
    let _ = current_branch("/repo", &runner);
    assert_eq!(runner.recorded_timeouts(), vec![Some(SUBPROCESS_TIMEOUT)]);
}

#[test]
fn dirty_files_bounds_its_subprocess() {
    let runner = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"")]);
    let _ = dirty_files("/repo", &runner);
    assert_eq!(runner.recorded_timeouts(), vec![Some(SUBPROCESS_TIMEOUT)]);
}

// --- the shared `git -C <dir>` helpers ---

fn argv(runner: &MockProcessRunner) -> Vec<String> {
    let calls = runner.recorded_calls();
    assert_eq!(calls.len(), 1, "expected one call, got {calls:?}");
    assert_eq!(calls[0].0, "git");
    calls[0].1.clone()
}

#[test]
fn git_in_prefixes_the_directory_and_carries_the_bound() {
    let runner = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    let timeout = std::time::Duration::from_secs(7);
    let _ = git_in(&runner, "/repo", &["status", "--porcelain"], timeout);
    assert_eq!(argv(&runner), ["-C", "/repo", "status", "--porcelain"]);
    assert_eq!(runner.recorded_timeouts(), vec![Some(timeout)]);
}

#[test]
fn git_in_unbounded_runs_without_a_timeout() {
    let runner = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
    let _ = git_in_unbounded(&runner, "/repo", &["worktree", "list"]);
    assert_eq!(argv(&runner), ["-C", "/repo", "worktree", "list"]);
    assert_eq!(runner.recorded_timeouts(), vec![None]);
}

#[test]
fn git_checked_passes_a_zero_exit_through() {
    let runner = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"out")]);
    let output = git_checked(&runner, "/repo", &["log"], SUBPROCESS_TIMEOUT)
        .unwrap_or_else(|_| panic!("a zero exit is a success"));
    assert_eq!(output.stdout, b"out");
}

#[test]
fn git_checked_hands_back_a_non_zero_exit_with_its_output() {
    let runner = MockProcessRunner::new(vec![MockProcessRunner::fail("fatal: nope")]);
    match git_checked(&runner, "/repo", &["log"], SUBPROCESS_TIMEOUT) {
        Err(GitFailure::Exit(output)) => {
            assert_eq!(crate::process::stderr_str(&output), "fatal: nope")
        }
        other => panic!("expected Exit, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn git_checked_reports_a_spawn_failure_apart_from_an_exit() {
    let runner = MockProcessRunner::new(vec![Err(anyhow::anyhow!("no git on PATH"))]);
    match git_checked(&runner, "/repo", &["log"], SUBPROCESS_TIMEOUT) {
        Err(GitFailure::Spawn(e)) => assert!(e.to_string().contains("no git on PATH")),
        other => panic!("expected Spawn, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn run_git_returns_stdout_on_success() {
    let runner = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"abc\n")]);
    let out = run_git(&runner, "/repo", &["rev-parse", "HEAD"], SUBPROCESS_TIMEOUT).unwrap();
    assert_eq!(out, "abc\n");
    assert_eq!(argv(&runner), ["-C", "/repo", "rev-parse", "HEAD"]);
}

#[test]
fn run_git_names_gits_first_stderr_line_on_failure() {
    let runner = MockProcessRunner::new(vec![MockProcessRunner::fail(
        "\n  fatal: bad revision  \nhint: more\n",
    )]);
    let err = run_git(&runner, "/repo", &["log"], SUBPROCESS_TIMEOUT).unwrap_err();
    assert_eq!(err.to_string(), "git: fatal: bad revision");
}

#[test]
fn run_git_names_a_spawn_failure() {
    let runner = MockProcessRunner::new(vec![Err(anyhow::anyhow!("timed out"))]);
    let err = run_git(&runner, "/repo", &["log"], SUBPROCESS_TIMEOUT).unwrap_err();
    assert_eq!(format!("{err:#}"), "could not run git: timed out");
}

#[test]
fn git_error_falls_back_when_stderr_is_empty() {
    let output = std::process::Output {
        status: crate::process::exit_fail(),
        stdout: vec![],
        stderr: vec![],
    };
    assert_eq!(git_error(&output).to_string(), "git: git failed");
}

#[test]
fn abort_issues_the_operations_abort_and_ignores_its_outcome() {
    let runner = MockProcessRunner::new(vec![MockProcessRunner::fail("no rebase")]);
    let timeout = std::time::Duration::from_secs(3);
    abort(&runner, "/wt", "rebase", timeout);
    assert_eq!(argv(&runner), ["-C", "/wt", "rebase", "--abort"]);
    assert_eq!(runner.recorded_timeouts(), vec![Some(timeout)]);
}

#[test]
fn prune_worktrees_issues_a_bounded_prune_and_ignores_its_outcome() {
    let runner = MockProcessRunner::new(vec![Err(anyhow::anyhow!("spawn failed"))]);
    let timeout = std::time::Duration::from_secs(3);
    prune_worktrees(&runner, "/repo", timeout);
    assert_eq!(argv(&runner), ["-C", "/repo", "worktree", "prune"]);
    assert_eq!(runner.recorded_timeouts(), vec![Some(timeout)]);
}

#[test]
fn git_failure_detail_is_the_spawn_error_or_gits_stderr() {
    let spawn = GitFailure::Spawn(anyhow::anyhow!("no git on PATH"));
    assert_eq!(spawn.detail(), "no git on PATH");
    let exit = GitFailure::Exit(std::process::Output {
        status: crate::process::exit_fail(),
        stdout: vec![],
        stderr: b"  fatal: refused\n".to_vec(),
    });
    assert_eq!(exit.detail(), "fatal: refused");
}

#[test]
fn abort_after_conflict_reads_the_conflicts_before_aborting() {
    let runner = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"UU src/a.rs\n M src/b.rs\n"),
        MockProcessRunner::ok(),
    ]);
    let timeout = std::time::Duration::from_secs(3);
    let files = abort_after_conflict(&runner, "/wt", "merge", timeout);
    assert_eq!(files, ["src/a.rs"]);
    assert_eq!(
        runner.flattened_calls(),
        ["git -C /wt status --porcelain", "git -C /wt merge --abort"]
    );
    assert_eq!(runner.recorded_timeouts(), vec![Some(timeout); 2]);
}

#[test]
fn abort_after_conflict_still_aborts_when_status_cannot_run() {
    let runner = MockProcessRunner::new(vec![
        Err(anyhow::anyhow!("spawn failed")),
        MockProcessRunner::ok(),
    ]);
    let files = abort_after_conflict(&runner, "/wt", "rebase", SUBPROCESS_TIMEOUT);
    assert!(files.is_empty());
    assert_eq!(runner.flattened_calls()[1], "git -C /wt rebase --abort");
}

// --- ahead_behind (moved here from the removed repo_sync module) ---

const REPO: &str = "/repo";
const BASE: &str = "main";

#[test]
fn ahead_behind_parses_left_right_counts() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"3\t1\n")]);
    assert_eq!(
        ahead_behind(REPO, BASE, &mock),
        Some(AheadBehind {
            ahead: 3,
            behind: 1
        })
    );
}

// A repo that is genuinely level is *measured*, so it yields a zero pair —
// distinct from the unmeasurable None below.
#[test]
fn ahead_behind_reports_a_level_repo_as_measured_zeroes() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"0\t0\n")]);
    let counts = ahead_behind(REPO, BASE, &mock).expect("a level repo is still measurable");
    assert_eq!(
        counts,
        AheadBehind {
            ahead: 0,
            behind: 0
        }
    );
}

// UnmeasurableIsNotInSync: no origin ref to compare against yields no
// counts at all, never a zero pair.
#[test]
fn ahead_behind_yields_nothing_when_origin_ref_does_not_resolve() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail(
        "fatal: ambiguous argument 'main...origin/main': unknown revision",
    )]);
    assert_eq!(ahead_behind(REPO, BASE, &mock), None);
}

#[test]
fn ahead_behind_yields_nothing_for_unparseable_output() {
    for stdout in [
        &b"banana\n"[..],
        &b"3\n"[..],
        &b"3\tmany\n"[..],
        &b""[..],
        &b"3\t1\t9\n"[..],
    ] {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(stdout)]);
        assert_eq!(
            ahead_behind(REPO, BASE, &mock),
            None,
            "unparseable output {:?} must not be reported as counts",
            String::from_utf8_lossy(stdout)
        );
    }
}

#[test]
fn ahead_behind_yields_nothing_when_the_runner_errors() {
    let mock = MockProcessRunner::new(vec![Err(anyhow::anyhow!("git not on PATH"))]);
    assert_eq!(ahead_behind(REPO, BASE, &mock), None);
}

#[test]
fn ahead_behind_invokes_rev_list_left_right_against_origin() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"0\t0\n")]);
    let _ = ahead_behind("/some/repo", "master", &mock);
    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "git");
    assert_eq!(
        calls[0].1,
        [
            "-C",
            "/some/repo",
            "rev-list",
            "--count",
            "--left-right",
            "master...origin/master",
        ]
    );
}

// Every subprocess this engine issues is bounded. rev-list walks history and
// can take a lock, so an unbounded one wedges whatever called it — the TUI's
// drift poll, or a dispatch that is waiting to provision a worktree.
#[test]
fn ahead_behind_bounds_the_rev_list_with_the_subprocess_timeout() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"0\t0\n")]);
    let _ = ahead_behind(REPO, BASE, &mock);
    assert_eq!(
        mock.recorded_timeouts(),
        vec![Some(SUBPROCESS_TIMEOUT)],
        "rev-list must be bounded like every other subprocess here"
    );
}
