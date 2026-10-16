use super::*;

/// Answers every call with success, except the verify run (`sh`), which
/// reports that it started and then blocks until the test releases it.
/// `git rev-parse HEAD` answers `head`, which a test moves to model a commit.
struct GatedVerifyRunner {
    started: std::sync::Mutex<std::sync::mpsc::Sender<()>>,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    verify_runs: std::sync::atomic::AtomicUsize,
    head: std::sync::Mutex<String>,
    sccache: bool,
    last_script: std::sync::Mutex<String>,
}

impl ProcessRunner for GatedVerifyRunner {
    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<std::process::Output> {
        if program == "sh" {
            self.verify_runs
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            *self.last_script.lock().unwrap() = args.get(1).unwrap_or(&"").to_string();
            let _ = self.started.lock().unwrap().send(());
            let _ = self.release.lock().unwrap().recv();
        }
        if args.contains(&"rev-parse") {
            return MockProcessRunner::ok_with_stdout(self.head.lock().unwrap().as_bytes());
        }
        MockProcessRunner::ok()
    }

    fn sccache_available(&self) -> bool {
        self.sccache
    }
}

struct Gated {
    runner: Arc<GatedVerifyRunner>,
    state: Arc<McpState>,
    task_id: crate::models::TaskId,
    started: std::sync::mpsc::Receiver<()>,
    release: std::sync::mpsc::Sender<()>,
}

impl Gated {
    fn runs(&self) -> usize {
        self.runner
            .verify_runs
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Block until a verify run has started.
    async fn wait_started(self) -> Self {
        tokio::task::spawn_blocking(move || {
            self.started.recv().unwrap();
            self
        })
        .await
        .unwrap()
    }
}

/// A board whose verify run blocks until released, and whose `wrap_up` waits
/// only 50ms for it instead of `wrap_up_verify_wait`.
async fn gated_board(sccache: bool) -> Gated {
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let runner = Arc::new(GatedVerifyRunner {
        started: std::sync::Mutex::new(started_tx),
        release: std::sync::Mutex::new(release_rx),
        verify_runs: std::sync::atomic::AtomicUsize::new(0),
        head: std::sync::Mutex::new(HEAD_A.to_string()),
        sccache,
        last_script: std::sync::Mutex::new(String::new()),
    });
    let db: Arc<dyn store::TaskStore> = Arc::new(Store::open_in_memory().unwrap());
    let svc = crate::service::TaskService::new(db.clone(), runner.clone())
        .with_verify_wait(std::time::Duration::from_millis(50));
    let (state, db) = state_over(db, runner.clone(), None, Some(Arc::new(svc)), None);
    let task_id = create_wrappable_task(&db).await;
    db.save_repo_path("/repo").await.unwrap();
    db.set_verify_command("/repo", Some("cargo test"))
        .await
        .unwrap();
    Gated {
        runner,
        state,
        task_id,
        started: started_rx,
        release: release_tx,
    }
}

/// mcp-task-tools.allium: WrapUpRefusesUnfinishedWork, "Verify outcome
/// memory". A call that gives up on a slow verify is refused with "still
/// running"; the run carries on, and the retry reuses it rather than starting
/// another.
#[tokio::test]
async fn a_retry_after_a_slow_verify_reuses_that_run_instead_of_starting_another() {
    let g = gated_board(false).await;
    let state = g.state.clone();
    let task_id = g.task_id;
    let first = tokio::spawn({
        let state = state.clone();
        async move { wrap_up_call(&state, task_id, "done").await }
    });
    let g = g.wait_started().await;

    let first = first.await.unwrap();
    assert!(is_error(&first), "the slow verify must not hold the call");
    assert!(
        error_message(&first).contains("still running"),
        "got: {}",
        error_message(&first)
    );
    assert_no_token(&state, task_id).await;

    // The retry joins the run still going; let it finish while the retry waits.
    g.release.send(()).unwrap();
    let mut second = wrap_up_call(&state, task_id, "done").await;
    // The 50ms wait may end just before the run does; the outcome is recorded
    // either way, so one more call reads it.
    if is_error(&second) {
        second = wrap_up_call(&state, task_id, "done").await;
    }

    assert!(!is_error(&second), "got: {}", error_message(&second));
    assert!(extract_response_text(&second).contains("`cargo test` passed"));
    assert_eq!(g.runs(), 1, "the retry must reuse the run, not repeat it");
}

/// A commit since the run started is a different key: the retry is neither
/// joined to the old run nor allowed to start a second beside it.
#[tokio::test]
async fn a_commit_since_the_running_verify_started_is_not_joined_and_starts_no_second_run() {
    let g = gated_board(false).await;
    let state = g.state.clone();
    let task_id = g.task_id;
    let first = tokio::spawn({
        let state = state.clone();
        async move { wrap_up_call(&state, task_id, "done").await }
    });
    let g = g.wait_started().await;
    let _ = first.await.unwrap();

    *g.runner.head.lock().unwrap() = HEAD_B.to_string();
    let second = wrap_up_call(&state, task_id, "done").await;

    assert!(is_error(&second));
    assert!(
        error_message(&second).contains("still running"),
        "got: {}",
        error_message(&second)
    );
    assert_eq!(g.runs(), 1, "no second run beside the one going");
    assert_no_token(&state, task_id).await;

    // Once the old run ends, the new commit is verified on its own.
    g.release.send(()).unwrap();
    g.release.send(()).unwrap();
    let mut third = wrap_up_call(&state, task_id, "done").await;
    for _ in 0..50 {
        if !is_error(&third) {
            break;
        }
        third = wrap_up_call(&state, task_id, "done").await;
    }
    assert!(!is_error(&third), "got: {}", error_message(&third));
    assert_eq!(g.runs(), 2, "the new commit gets its own run");
}

#[tokio::test]
async fn a_recorded_pass_is_reused_on_the_same_commit_without_running_verify_again() {
    let runner = checked_runner(
        "",
        Some(MockProcessRunner::ok()),
        vec![
            // The second call: commit check, then HEAD; no verify run.
            (None, MockProcessRunner::ok_with_stdout(b"")),
            (None, MockProcessRunner::ok_with_stdout(HEAD_A.as_bytes())),
        ],
    );
    let (state, task_id) = wrappable_task_with_verify(runner.clone(), Some("cargo test")).await;

    let first = wrap_up_call(&state, task_id, "done").await;
    let second = wrap_up_call(&state, task_id, "done").await;

    assert!(!is_error(&first), "got: {}", error_message(&first));
    assert!(!is_error(&second), "got: {}", error_message(&second));
    assert!(extract_response_text(&second).contains("`cargo test` passed"));
    let sh = runner
        .recorded_calls()
        .iter()
        .filter(|(p, _)| p == "sh")
        .count();
    assert_eq!(sh, 1, "calls: {:?}", runner.flattened_calls());
}

#[tokio::test]
async fn a_recorded_failure_is_returned_again_without_running_verify_again() {
    let runner = checked_runner(
        "",
        Some(MockProcessRunner::fail_with_code(101, "1 failed")),
        vec![
            (None, MockProcessRunner::ok_with_stdout(b"")),
            (None, MockProcessRunner::ok_with_stdout(HEAD_A.as_bytes())),
        ],
    );
    let (state, task_id) = wrappable_task_with_verify(runner.clone(), Some("cargo test")).await;

    let _first = wrap_up_call(&state, task_id, "done").await;
    let second = wrap_up_call(&state, task_id, "done").await;

    assert!(is_error(&second));
    let msg = error_message(&second);
    assert!(
        msg.contains("101") && msg.contains("1 failed") && msg.contains("cargo test"),
        "the refusal must repeat the original; got: {msg}"
    );
    assert_no_token(&state, task_id).await;
    let sh = runner
        .recorded_calls()
        .iter()
        .filter(|(p, _)| p == "sh")
        .count();
    assert_eq!(sh, 1, "calls: {:?}", runner.flattened_calls());
}

#[tokio::test]
async fn a_new_commit_is_verified_afresh_even_after_a_recorded_pass() {
    let runner = checked_runner(
        "",
        Some(MockProcessRunner::ok()),
        vec![
            (None, MockProcessRunner::ok_with_stdout(b"")),
            (None, MockProcessRunner::ok_with_stdout(HEAD_B.as_bytes())),
            (None, MockProcessRunner::ok()),
        ],
    );
    let (state, task_id) = wrappable_task_with_verify(runner.clone(), Some("cargo test")).await;

    let _first = wrap_up_call(&state, task_id, "done").await;
    let second = wrap_up_call(&state, task_id, "done").await;

    assert!(!is_error(&second), "got: {}", error_message(&second));
    let sh = runner
        .recorded_calls()
        .iter()
        .filter(|(p, _)| p == "sh")
        .count();
    assert_eq!(sh, 2, "a different commit must not reuse the pass");
}

/// mcp-task-tools.allium: the verify run gets `RUSTC_WRAPPER=sccache` when
/// sccache is on the board's PATH, and a value already set is kept.
#[tokio::test]
async fn the_verify_run_gets_sccache_when_it_is_available_and_not_otherwise() {
    for (available, expected) in [(true, true), (false, false)] {
        let g = gated_board(available).await;
        g.release.send(()).unwrap();
        let resp = wrap_up_call(&g.state, g.task_id, "done").await;
        assert!(!is_error(&resp), "got: {}", error_message(&resp));
        let script = g.runner.last_script.lock().unwrap().clone();
        assert_eq!(
            script.contains("RUSTC_WRAPPER"),
            expected,
            "sccache available={available}; script: {script}"
        );
        if expected {
            assert!(
                script.contains(":=sccache"),
                "an existing value must be kept; script: {script}"
            );
        }
    }
}
