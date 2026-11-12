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

/// A Stop payload's `background_tasks` list counted by kind. Subagents and
/// kinds the board does not keep are left out (`core.allium:
/// BackgroundWork`). The `shell` and `subagent` entries have the shape Claude
/// Code 2.1.322 sent in a live capture (ids and commands scrubbed); the
/// `workflow`, `monitor`, `MCP task` and `dream` type labels are the ones in
/// that binary's own label table, not from a capture.
#[test]
fn stop_counts_the_background_work_by_kind() {
    let dir = tempfile::tempdir().unwrap();
    let repo = repo_on_branch(dir.path(), "7-hotfix");
    let list = r#","background_tasks":[
        {"id":"a1","type":"subagent","status":"running","description":"d","agent_type":"general-purpose"},
        {"id":"b1","type":"shell","status":"running","description":"d","command":"sleep 25"},
        {"id":"b2","type":"shell","status":"running","description":"d","command":"cargo test"},
        {"id":"w1","type":"workflow","status":"running","description":"d","name":"review"},
        {"id":"m1","type":"monitor","status":"running","description":"d","server":"s","tool":"t"},
        {"id":"t1","type":"MCP task","status":"pending","description":"d","server":"s","tool":"t"},
        {"id":"x1","type":"dream","status":"running","description":"d"}
    ]"#;
    let input = payload(&repo, "Stop", list);
    let (code, calls) = run_script("task-status-hook", &input, Some("42"), 0, &repo, dir.path());
    assert_eq!(code, Some(0));
    assert_eq!(
        calls.trim(),
        "hook 42 stop --shells 2 --workflows 1 --monitors 1 --mcp-tasks 1"
    );
}

/// An empty list and an absent one (older Claude Code) both send a plain
/// stop, which the board reads as nothing in flight.
#[test]
fn stop_with_an_empty_or_absent_background_list_is_a_plain_stop() {
    for extra in [r#","background_tasks":[]"#, ""] {
        let dir = tempfile::tempdir().unwrap();
        let repo = repo_on_branch(dir.path(), "7-hotfix");
        let input = payload(&repo, "Stop", extra);
        let (_, calls) = run_script("task-status-hook", &input, Some("42"), 0, &repo, dir.path());
        assert_eq!(calls.trim(), "hook 42 stop", "extra: {extra:?}");
    }
}

/// A `dispatch` binary that predates the count flags rejects them. The
/// script then sends the plain stop, so the Stop is not lost.
#[test]
fn stop_falls_back_to_a_plain_stop_when_the_counts_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let repo = repo_on_branch(dir.path(), "7-hotfix");
    let input = payload(
        &repo,
        "Stop",
        r#","background_tasks":[{"id":"b1","type":"shell","status":"running","description":"d","command":"sleep 25"}]"#,
    );
    let (_, calls) = run_script("task-status-hook", &input, Some("42"), 2, &repo, dir.path());
    let lines: Vec<&str> = calls.lines().collect();
    assert_eq!(
        lines,
        [
            "hook 42 stop --shells 1 --workflows 0 --monitors 0 --mcp-tasks 0",
            "hook 42 stop"
        ]
    );
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
