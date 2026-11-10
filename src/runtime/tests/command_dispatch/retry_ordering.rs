use super::*;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

// RetryResume and RetryFresh (docs/specs/dispatch.allium) start a teardown and
// then a launch. The teardown must finish before the launch looks at tmux or
// the worktree, so the command drain waits for it.

/// A tmux that answers `list-windows` from state and takes a while to kill a
/// window, like a real window with a process that is slow to die.
struct SlowKillTmux {
    inner: Arc<MockProcessRunner>,
    alive: AtomicBool,
}

impl SlowKillTmux {
    fn call(&self, program: &str, args: &[&str]) -> Option<anyhow::Result<std::process::Output>> {
        if args.contains(&"list-windows") {
            let stdout: &[u8] = if self.alive.load(AtomicOrdering::SeqCst) {
                b"task-1\n"
            } else {
                b""
            };
            return Some(MockProcessRunner::ok_with_stdout(stdout));
        }
        if args.contains(&"kill-window") {
            // allow-test-sleep: models a slow kill; nothing measures the time, the assertion is on call order.
            std::thread::sleep(Duration::from_millis(300));
            self.alive.store(false, AtomicOrdering::SeqCst);
        }
        let _ = program;
        None
    }
}

impl ProcessRunner for SlowKillTmux {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<std::process::Output> {
        self.call(program, args)
            .unwrap_or_else(|| self.inner.run(program, args))
    }

    fn run_with_timeout(
        &self,
        program: &str,
        args: &[&str],
        timeout: Duration,
    ) -> anyhow::Result<std::process::Output> {
        self.call(program, args)
            .unwrap_or_else(|| self.inner.run_with_timeout(program, args, timeout))
    }
}

#[tokio::test]
async fn retry_resume_launches_only_after_the_old_window_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path().to_string_lossy().into_owned();
    let db = test_db().await;
    let (tx, mut msgs) = mpsc::unbounded_channel();
    let inner = Arc::new(MockProcessRunner::new(
        (0..60).map(|_| MockProcessRunner::ok()).collect(),
    ));
    let runner = Arc::new(SlowKillTmux {
        inner: Arc::clone(&inner),
        alive: AtomicBool::new(true),
    });
    let rt = make_runtime(db.clone(), tx, runner.clone()).await;
    let mut app = App::new(db.list_all().await);
    let task = seed(&rt, "Stale", models::TaskStatus::Running).await;

    // What handle_retry_resume emits for a task whose window is still alive.
    drain(
        &rt,
        &mut app,
        Command::Task(TaskCommand::KillTmuxWindow {
            window: test_tmux_window("task-1"),
        }),
    )
    .await;
    drain(
        &rt,
        &mut app,
        Command::Task(TaskCommand::Resume {
            id: task.id,
            worktree: Some(worktree),
            message: None,
        }),
    )
    .await;

    // Resume reports through the message channel once its launch settles.
    tokio::time::timeout(TEST_TIMEOUT, msgs.recv())
        .await
        .expect("resume must report")
        .expect("sender alive");
    let calls = inner.flattened_calls();
    assert!(
        calls.iter().any(|c| c.contains("new-window")),
        "a retry-resume must launch a new window, not reattach to the dying one; calls: {calls:?}"
    );
}

#[tokio::test]
async fn a_cleanup_is_finished_before_the_next_command_starts() {
    let db = test_db().await;
    let (tx, _msgs) = mpsc::unbounded_channel();
    let inner = Arc::new(MockProcessRunner::new(
        (0..60).map(|_| MockProcessRunner::ok()).collect(),
    ));
    let runner = Arc::new(SlowKillTmux {
        inner: Arc::clone(&inner),
        alive: AtomicBool::new(true),
    });
    let rt = make_runtime(db.clone(), tx, runner).await;
    let mut app = App::new(db.list_all().await);

    commands::dispatch(
        Command::Task(TaskCommand::Cleanup {
            id: models::TaskId(1),
            repo_path: "/repo".into(),
            worktree: Some("/repo/.worktrees/1-x".into()),
            tmux_window: Some(test_tmux_window("task-1")),
            follow_up: crate::tui::commands::CleanupFollowUp::ClearPointer,
            guard: None,
        }),
        &mut app,
        &rt,
    )
    .await;

    // RetryFresh's re-dispatch is the next command: by now the teardown is done.
    let calls = inner.flattened_calls();
    assert!(
        calls.iter().any(|c| c.contains("worktree remove")),
        "the worktree removal must be over when the cleanup command returns; calls: {calls:?}"
    );
}
