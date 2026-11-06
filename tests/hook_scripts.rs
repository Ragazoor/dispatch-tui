#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Behaviour of the plugin hook scripts themselves, run under bash with a fake
//! `dispatch` first on `PATH` that records its arguments.
//!
//! `HookDelivery` in `docs/specs/agent-health.allium`: a hook acts on the task
//! its agent was launched for (`DISPATCH_TASK_ID`), not on whatever branch the
//! agent's cwd happens to be on.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[path = "common/script_command.rs"]
mod script_command;

fn script(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("plugin/hooks/scripts")
        .join(name)
}

/// A bin dir holding a fake `dispatch` that appends its arguments to a log
/// and exits with `exit_code`.
fn fake_dispatch(dir: &Path, exit_code: i32) -> (PathBuf, PathBuf) {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = dir.join("calls.log");
    let body = format!(
        "#!/usr/bin/env bash\necho \"$*\" >> '{}'\nexit {exit_code}\n",
        log.display()
    );
    let path = bin.join("dispatch");
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    (bin, log)
}

/// A git repo on branch `branch`.
fn repo_on_branch(dir: &Path, branch: &str) -> PathBuf {
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let init = ["init", "-q", "-b", branch];
    let commit = [
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        "x",
    ];
    for args in [&init[..], &commit[..]] {
        let st = Command::new("git")
            .args(args)
            .current_dir(&repo)
            .status()
            .unwrap();
        assert!(st.success());
    }
    repo
}

/// Run `name` with `input` on stdin. `task_id` sets `DISPATCH_TASK_ID`
/// (otherwise it is removed). Returns the exit code and the fake's call log.
fn run_script(
    name: &str,
    input: &str,
    task_id: Option<&str>,
    dispatch_exit: i32,
    repo: &Path,
    dir: &Path,
) -> (Option<i32>, String) {
    let (bin, log) = fake_dispatch(dir, dispatch_exit);
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let mut cmd = script_command::plugin_script_command(&script(name), &path);
    cmd.current_dir(repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(id) = task_id {
        cmd.env("DISPATCH_TASK_ID", id);
    }
    let mut child = cmd.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let status = child.wait().unwrap();
    let calls = std::fs::read_to_string(&log).unwrap_or_default();
    (status.code(), calls)
}

fn payload(cwd: &Path, event: &str, extra: &str) -> String {
    format!(
        r#"{{"cwd":"{}","hook_event_name":"{event}"{extra}}}"#,
        cwd.display()
    )
}

#[test]
fn stop_resolves_the_launch_task_not_the_branch() {
    let dir = tempfile::tempdir().unwrap();
    let repo = repo_on_branch(dir.path(), "7-hotfix");
    let input = payload(&repo, "Stop", "");
    let (code, calls) = run_script("task-status-hook", &input, Some("42"), 0, &repo, dir.path());
    assert_eq!(code, Some(0));
    assert_eq!(calls.trim(), "hook 42 stop");
}

#[test]
fn stop_falls_back_to_the_branch_without_the_env() {
    let dir = tempfile::tempdir().unwrap();
    let repo = repo_on_branch(dir.path(), "7-hotfix");
    let input = payload(&repo, "Stop", "");
    let (_, calls) = run_script("task-status-hook", &input, None, 0, &repo, dir.path());
    assert_eq!(calls.trim(), "hook 7 stop");
}

#[test]
fn session_start_clears_the_launch_task_not_the_branch() {
    let dir = tempfile::tempdir().unwrap();
    let repo = repo_on_branch(dir.path(), "7-hotfix");
    let input = payload(&repo, "SessionStart", "");
    let (_, calls) = run_script("task-status-hook", &input, Some("42"), 0, &repo, dir.path());
    assert_eq!(calls.trim(), "hook-subagent 42 clear");
}

#[test]
fn pr_gate_spends_the_launch_tasks_reminder() {
    let dir = tempfile::tempdir().unwrap();
    let repo = repo_on_branch(dir.path(), "7-hotfix");
    let cmd = ["gh pr", "create"].join(" ");
    let extra = format!(r#","tool_name":"Bash","tool_input":{{"command":"{cmd}"}}"#);
    let input = payload(&repo, "PreToolUse", &extra);
    let (_, calls) = run_script(
        "pr-learnings-hook",
        &input,
        Some("42"),
        0,
        &repo,
        dir.path(),
    );
    assert_eq!(calls.trim(), "pr-gate 42");
}

#[test]
fn a_failing_board_call_does_not_skip_the_ask_user_question_notification() {
    let dir = tempfile::tempdir().unwrap();
    let repo = repo_on_branch(dir.path(), "42-x");
    let input = payload(&repo, "PreToolUse", r#","tool_name":"AskUserQuestion""#);
    let (code, calls) = run_script("task-status-hook", &input, Some("42"), 1, &repo, dir.path());
    assert_eq!(code, Some(0));
    assert!(
        calls.contains("hook 42 notification --kind elicitation_dialog"),
        "got: {calls}"
    );
}

#[test]
fn scripts_do_not_need_gnu_grep() {
    let src = std::fs::read_to_string(script("task-status-hook")).unwrap()
        + &std::fs::read_to_string(script("pr-learnings-hook")).unwrap();
    assert!(!src.contains("grep -oP"), "grep -P is GNU-only");
}
