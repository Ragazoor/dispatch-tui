#![allow(clippy::unwrap_used, clippy::expect_used)]
#![allow(clippy::disallowed_methods)] // tests: a hung child fails the run, not a board
//! Real-tmux tests of an epic orchestrator's session (`orchestrators.allium:
//! StartOrchestratorSession`, `EndOrchestratorSession`): the window it runs
//! in, its own directory, its caller identity and its resume. The stub
//! `claude` reports its cwd and argv; see tests/tmux_harness/mod.rs.

mod tmux_harness;

use std::path::{Path, PathBuf};

use dispatch_tui::dispatch;
use dispatch_tui::models::{EpicId, TmuxWindow};

use tmux_harness::{await_stub_line, canonical, stub_lines, tmux_available_or_skip, TmuxServer};

const NAME: &str = "zlata";
/// One line: the stub records its argv on one line.
const PROMPT: &str = "You are zlata, the orchestrator of epic 7.";

struct Fixture {
    server: TmuxServer,
    #[allow(dead_code)]
    tmp: tempfile::TempDir,
    home: PathBuf,
    claude_json: PathBuf,
    dir: PathBuf,
    window: TmuxWindow,
}

fn setup_or_skip() -> Option<Fixture> {
    if !tmux_available_or_skip() {
        return None;
    }
    let server = TmuxServer::start();
    server.tmux_ok(&["new-session", "-d", "-s", "t", "-n", "board"]);
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let claude_json = home.join(".claude.json");
    std::fs::write(
        &claude_json,
        r#"{"mcpServers":{"dispatch":{"type":"http","url":"http://127.0.0.1:9/mcp","headersHelper":"dispatch caller-headers"}}}"#,
    )
    .unwrap();
    let dir = dispatch::orchestrator_dir(&tmp.path().join("data"), EpicId(7));
    Some(Fixture {
        server,
        tmp,
        home,
        claude_json,
        dir,
        window: TmuxWindow::for_orchestrator(EpicId(7)),
    })
}

fn launch(f: &Fixture) {
    launch_with(f, true);
}

fn launch_with(f: &Fixture, may_resume: bool) {
    dispatch::launch_orchestrator(
        &dispatch::OrchestratorLaunch {
            name: NAME,
            window: &f.window,
            dir: &f.dir,
            prompt: PROMPT,
            claude_json: &f.claude_json,
            may_resume,
        },
        &f.server.runner(),
    )
    .expect("launch");
}

fn dir_str(p: &Path) -> String {
    canonical(&p.to_string_lossy())
}

#[test]
fn the_session_runs_in_its_own_window_and_directory_under_its_name() {
    let Some(f) = setup_or_skip() else { return };

    launch(&f);

    assert!(f.server.has_window("orch-7"));
    let line = await_stub_line(&f.server, |l| l.name == "claude").expect("claude started");
    assert_eq!(canonical(&line.cwd), dir_str(&f.dir));
    assert!(
        line.args.contains(&format!("--name {NAME}")),
        "{}",
        line.args
    );
    assert!(line.args.contains("--plugin-dir"), "{}", line.args);
    assert!(
        line.args.ends_with(&format!("-- {PROMPT}")),
        "{}",
        line.args
    );
    assert!(!line.args.contains("--continue"), "{}", line.args);
    assert!(
        !f.dir.join(".claude-prompt").exists(),
        "the prompt file is read and removed"
    );
}

#[test]
fn the_session_carries_its_own_caller_identity_and_hook_name() {
    let Some(f) = setup_or_skip() else { return };

    launch(&f);

    let config = f.dir.join("dispatch-mcp.json");
    let line = await_stub_line(&f.server, |l| l.name == "claude").expect("claude started");
    assert!(
        line.args
            .contains(&format!("--mcp-config {}", config.display())),
        "{}",
        line.args
    );
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    let entry = &written["mcpServers"]["dispatch"];
    assert_eq!(entry["headers"]["X-Caller-Orchestrator"], NAME);
    assert!(entry.get("headersHelper").is_none());
    let pane = f.server.pane_ids("orch-7")[0].clone();
    let start = f.server.pane_start_command(&pane);
    assert!(
        start.contains(&format!("DISPATCH_ORCHESTRATOR={NAME}")),
        "{start}"
    );
    assert!(start.contains("-u DISPATCH_TASK_ID"), "{start}");
}

#[test]
fn the_board_trusts_the_directory_it_made() {
    let Some(f) = setup_or_skip() else { return };

    launch(&f);

    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&f.claude_json).unwrap()).unwrap();
    let key = f.dir.to_string_lossy().into_owned();
    assert_eq!(
        json["projects"][key.as_str()]["hasTrustDialogAccepted"],
        true
    );
}

#[test]
fn a_kept_conversation_is_continued() {
    let Some(f) = setup_or_skip() else { return };
    let key: String = f
        .dir
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let projects = f.home.join(".claude").join("projects").join(key);
    std::fs::create_dir_all(&projects).unwrap();
    std::fs::write(projects.join("a.jsonl"), "{}\n").unwrap();

    launch(&f);

    let line = await_stub_line(&f.server, |l| l.name == "claude").expect("claude started");
    assert!(line.args.contains(" --continue"), "{}", line.args);
}

#[test]
fn a_start_after_a_death_does_not_continue() {
    let Some(f) = setup_or_skip() else { return };
    let key: String = f
        .dir
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let projects = f.home.join(".claude").join("projects").join(key);
    std::fs::create_dir_all(&projects).unwrap();
    std::fs::write(projects.join("a.jsonl"), "{}\n").unwrap();

    launch_with(&f, false);

    let line = await_stub_line(&f.server, |l| l.name == "claude").expect("claude started");
    assert!(!line.args.contains("--continue"), "{}", line.args);
}

#[test]
fn a_launch_replaces_a_window_left_with_the_same_name() {
    let Some(f) = setup_or_skip() else { return };
    f.server.tmux_ok(&["new-window", "-d", "-n", "orch-7"]);

    launch(&f);

    assert_eq!(f.server.count_windows_named("orch-7"), 1);
    assert!(await_stub_line(&f.server, |l| l.name == "claude").is_some());
}

#[test]
fn ending_closes_the_window() {
    let Some(f) = setup_or_skip() else { return };
    launch(&f);
    assert!(await_stub_line(&f.server, |l| l.name == "claude").is_some());

    dispatch::end_orchestrator(&f.window, &f.server.runner()).expect("end");

    assert!(!f.server.has_window("orch-7"));
    assert!(f.server.has_window("board"));
    assert_eq!(
        stub_lines(&f.server)
            .iter()
            .filter(|l| l.name == "claude")
            .count(),
        1
    );
}
