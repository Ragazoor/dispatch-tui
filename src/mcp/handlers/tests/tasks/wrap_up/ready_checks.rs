use super::*;

// ---------------------------------------------------------------------------
// wrap_up refuses unfinished work (WrapUpRefusesUnfinishedWork,
// docs/specs/mcp-task-tools.allium)
// ---------------------------------------------------------------------------

const WORKTREE: &str = "/repo/.worktrees/1-t";

#[tokio::test]
async fn wrap_up_refuses_a_worktree_with_uncommitted_changes_for_every_action() {
    for action in ["rebase", "pr", "done"] {
        let runner = checked_runner(" M src/lib.rs\n?? notes.md\n", None, vec![]);
        let (state, task_id) = wrappable_task_with_verify(runner.clone(), None).await;

        let resp = wrap_up_call(&state, task_id, action).await;

        assert!(is_error(&resp), "{action}: expected a refusal");
        let msg = error_message(&resp);
        assert!(
            msg.contains("src/lib.rs") && msg.contains("notes.md"),
            "{action}: the refusal must name the uncommitted paths; got: {msg}"
        );
        assert!(
            msg.contains("commit"),
            "{action}: the refusal must say to commit; got: {msg}"
        );
        assert_no_token(&state, task_id).await;
        assert_eq!(
            runner.flattened_calls(),
            vec![format!("git -C {WORKTREE} status --porcelain")],
            "{action}: nothing may run after a dirty tree is found"
        );
    }
}

#[tokio::test]
async fn wrap_up_refuses_when_the_verify_command_fails_and_never_rebases() {
    let runner = checked_runner(
        "",
        Some(MockProcessRunner::fail_with_code(
            101,
            "test result: FAILED. 1 failed",
        )),
        vec![],
    );
    let (state, task_id) = wrappable_task_with_verify(runner.clone(), Some("cargo test")).await;

    let resp = wrap_up_call(&state, task_id, "rebase").await;

    assert!(is_error(&resp), "expected a refusal");
    let msg = error_message(&resp);
    assert!(
        msg.contains("cargo test"),
        "must name the command; got: {msg}"
    );
    assert!(msg.contains("101"), "must give the exit code; got: {msg}");
    assert!(
        msg.contains("1 failed"),
        "must carry the output tail; got: {msg}"
    );
    assert_no_token(&state, task_id).await;
    assert_eq!(
        runner.recorded_calls().len(),
        3,
        "only the status check, the HEAD read and the verify run may happen; got: {:?}",
        runner.flattened_calls()
    );
}

#[tokio::test]
async fn wrap_up_refuses_pr_and_done_when_the_verify_command_fails() {
    for action in ["pr", "done"] {
        let runner = checked_runner("", Some(MockProcessRunner::fail("boom")), vec![]);
        let (state, task_id) = wrappable_task_with_verify(runner, Some("make check")).await;

        let resp = wrap_up_call(&state, task_id, action).await;

        assert!(is_error(&resp), "{action}: expected a refusal");
        assert!(error_message(&resp).contains("make check"));
        assert_no_token(&state, task_id).await;
    }
}

#[tokio::test]
async fn wrap_up_refuses_when_the_verify_command_times_out() {
    let runner = Arc::new(MockProcessRunner::new_with_delays(vec![
        (None, MockProcessRunner::ok()),
        (
            Some(crate::service::WRAP_UP_VERIFY_TIMEOUT),
            MockProcessRunner::ok(),
        ),
    ]));
    let (state, task_id) = wrappable_task_with_verify(runner, Some("cargo test")).await;

    let resp = wrap_up_call(&state, task_id, "done").await;

    assert!(is_error(&resp), "expected a refusal");
    assert!(
        error_message(&resp).contains("timed out"),
        "got: {}",
        error_message(&resp)
    );
    assert_no_token(&state, task_id).await;
}

#[tokio::test]
async fn wrap_up_runs_verify_in_the_worktree_after_the_commit_check_and_before_the_rebase() {
    let script = DispatchScript::finish().no_remote();
    let runner = checked_runner("", Some(MockProcessRunner::ok()), script.responses());
    let (state, task_id) = wrappable_task_with_verify(runner.clone(), Some("cargo test")).await;

    let resp = wrap_up_call(&state, task_id, "rebase").await;

    assert!(!is_error(&resp), "got: {}", error_message(&resp));
    let calls = runner.recorded_calls();
    assert_eq!(
        calls[0].1,
        vec!["-C", WORKTREE, "status", "--porcelain"],
        "the commit check runs first"
    );
    assert_eq!(
        calls[1].1,
        vec!["-C", WORKTREE, "rev-parse", "HEAD"],
        "HEAD is read next, to key the outcome"
    );
    let (program, args) = &calls[2];
    assert_eq!(program, "sh");
    assert!(
        args.iter().any(|a| a == WORKTREE) && args.iter().any(|a| a == "cargo test"),
        "verify must run the stored command in the worktree; got: {args:?}"
    );
    assert!(
        calls[3..]
            .iter()
            .any(|(_, a)| a.contains(&"rebase".to_string())),
        "the rebase comes after verify; got: {:?}",
        runner.flattened_calls()
    );
}

#[tokio::test]
async fn wrap_up_verified_success_says_so_and_has_no_reverify_instruction() {
    for action in ["rebase", "pr", "done"] {
        let after = if action == "rebase" {
            DispatchScript::finish().no_remote().responses()
        } else {
            vec![]
        };
        let runner = checked_runner("", Some(MockProcessRunner::ok()), after);
        let (state, task_id) = wrappable_task_with_verify(runner, Some("cargo test")).await;

        let resp = wrap_up_call(&state, task_id, action).await;

        assert!(!is_error(&resp), "{action}: got: {}", error_message(&resp));
        let text = extract_response_text(&resp);
        assert!(
            !text.contains("Verify before"),
            "{action}: the tool ran verify, so it must not ask the agent to: {text}"
        );
        assert!(
            text.contains("`cargo test` passed"),
            "{action}: the response must report the verification: {text}"
        );
        assert!(text.contains("exit_session"), "{action}: {text}");
    }
}

#[tokio::test]
async fn wrap_up_without_a_verify_command_only_checks_the_worktree() {
    let runner = checked_runner("", None, vec![]);
    let (state, task_id) = wrappable_task_with_verify(runner.clone(), None).await;

    let resp = wrap_up_call(&state, task_id, "done").await;

    assert!(!is_error(&resp), "got: {}", error_message(&resp));
    assert_eq!(runner.recorded_calls().len(), 1);
    assert!(!extract_response_text(&resp).contains("passed"));
}

#[tokio::test]
async fn wrap_up_refusal_names_at_most_ten_paths() {
    let status: String = (0..13).map(|i| format!(" M f{i:02}.rs\n")).collect();
    let runner = checked_runner(&status, None, vec![]);
    let (state, task_id) = wrappable_task_with_verify(runner, None).await;

    let resp = wrap_up_call(&state, task_id, "done").await;

    assert!(is_error(&resp));
    let msg = error_message(&resp);
    assert!(
        msg.contains("f09.rs") && !msg.contains("f10.rs"),
        "got: {msg}"
    );
    assert!(msg.contains("and 3 more"), "got: {msg}");
}

#[tokio::test]
async fn wrap_up_rebase_with_no_branch_is_refused_before_the_verify_run() {
    // `store-split.allium` W2: the branch check comes before the commit check
    // and the verify run, so a rebase that cannot work costs no verify run.
    let runner = checked_runner("", Some(MockProcessRunner::ok_with_stdout(b"")), vec![]);
    let (state, db) = make_state_with_runner(runner.clone()).await;
    let task_id = create_wrappable_task(&db).await;
    db.save_repo_path("/repo").await.unwrap();
    db.set_verify_command("/repo", Some("cargo test"))
        .await
        .unwrap();
    // A worktree path with no final component names no branch.
    db.patch_task(task_id, &store::TaskPatch::new().worktree(Some("/")))
        .await
        .unwrap();

    let resp = wrap_up_call(&state, task_id, "rebase").await;

    assert!(is_error(&resp), "expected a refusal");
    let msg = error_message(&resp);
    assert!(
        msg.contains("Cannot derive branch"),
        "must say no branch can be derived; got: {msg}"
    );
    assert_no_token(&state, task_id).await;
    assert!(
        runner.recorded_calls().is_empty(),
        "no command may run before the branch check; got: {:?}",
        runner.flattened_calls()
    );
}
