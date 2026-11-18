use super::*;

// ---------------------------------------------------------------------------
// wrap_up refuses unfinished work (WrapUpRefusesUnfinishedWork,
// docs/specs/mcp-task-tools.allium)
// ---------------------------------------------------------------------------

const WORKTREE: &str = "/repo/.worktrees/1-t";

#[tokio::test]
async fn wrap_up_refuses_a_worktree_with_uncommitted_changes_for_every_action() {
    for action in ["rebase", "pr", "done"] {
        let runner = checked_runner(" M src/lib.rs\n?? notes.md\n", vec![]);
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

/// `WrapUpRefusesUnfinishedWork` / `AgentsRunTheVerifyCommand`: the board never
/// runs the verify command, whatever the action.
#[tokio::test]
async fn wrap_up_never_runs_the_verify_command() {
    for action in ["rebase", "pr", "done"] {
        let (state, task_id, runner) = verify_board(action).await;

        let resp = wrap_up_call(&state, task_id, action).await;

        assert!(!is_error(&resp), "{action}: got: {}", error_message(&resp));
        assert!(
            !runner.recorded_calls().iter().any(|(p, _)| p == "sh"),
            "{action}: the board must not run verify; got: {:?}",
            runner.flattened_calls()
        );
        let text = extract_response_text(&resp);
        assert!(
            !text.contains("passed") && !text.contains("Verified"),
            "{action}: the board verified nothing, so it must not claim to: {text}"
        );
        assert!(
            !text.contains("Re-run") && !text.contains("cargo test"),
            "{action}: no answer asks for a verify re-run (AgentsRunTheVerifyCommand): {text}"
        );
        assert!(text.contains("session_exit"), "{action}: {text}");
    }
}

#[tokio::test]
async fn wrap_up_without_a_verify_command_only_checks_the_worktree() {
    let runner = checked_runner("", vec![]);
    let (state, task_id) = wrappable_task_with_verify(runner.clone(), None).await;

    let resp = wrap_up_call(&state, task_id, "done").await;

    assert!(!is_error(&resp), "got: {}", error_message(&resp));
    assert_eq!(runner.recorded_calls().len(), 1);
}

#[tokio::test]
async fn wrap_up_refusal_names_at_most_ten_paths() {
    let status: String = (0..13).map(|i| format!(" M f{i:02}.rs\n")).collect();
    let runner = checked_runner(&status, vec![]);
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

/// A board whose repo has the verify command `cargo test`. The runner answers
/// the commit check, and for a rebase the rebase itself.
async fn verify_board(
    action: &str,
) -> (Arc<McpState>, crate::models::TaskId, Arc<MockProcessRunner>) {
    let mut responses = vec![(None, MockProcessRunner::ok_with_stdout(b""))];
    if action == "rebase" {
        responses.extend(DispatchScript::finish().no_remote().responses());
    }
    let runner = Arc::new(MockProcessRunner::new_with_delays(responses));
    let (state, task_id) = wrappable_task_with_verify(runner.clone(), Some("cargo test")).await;
    (state, task_id, runner)
}

#[tokio::test]
async fn wrap_up_rebase_with_no_branch_is_refused_before_any_command_runs() {
    // `store-split.allium` W2: the branch check comes before the commit check,
    // so a rebase that cannot work runs no command.
    let runner = checked_runner("", vec![]);
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
