use super::*;

/// A tempdir repo with `.worktrees/<slug>` already present, i.e. the reuse
/// path a [`DispatchScript::dispatch`] shape scripts.
fn provisioned_repo(slug: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    crate::dispatch::tests::create_worktree_dir(dir.path().join(".worktrees").join(slug));
    dir
}

// --- TaskCommand: the tmux/process half --------------------------------

#[tokio::test]
async fn dispatch_task_dispatch_agent_claims_then_provisions() {
    let repo = provisioned_repo("1-test-task");
    let mut h = harness(DispatchScript::dispatch().runner()).await;
    let task = create_task_returning(
        &**h.rt.db_write(),
        "Test Task",
        "desc",
        repo.path().to_str().unwrap(),
        None,
        models::TaskStatus::Backlog,
    )
    .await
    .unwrap();
    let id = task.id;

    h.dispatch(Command::Task(TaskCommand::DispatchAgent {
        id: task.id,
        mode: Some(models::DispatchMode::Dispatch),
    }))
    .await;
    // The arm only asked: the request is the claim
    // (`DispatchClaimExclusive` in docs/specs/dispatch.allium).
    let claimed = h.rt.database.get_task(id).await.unwrap();
    assert_eq!(claimed.status, models::TaskStatus::Running);
    assert!(claimed.worktree.is_none(), "nothing provisioned inline");

    h.serve_requests().await;

    let msg = h.next_msg().await;
    assert!(
        matches!(
            msg,
            Message::Task(crate::tui::messages::TaskMessage::Dispatched { .. })
        ),
        "Expected Dispatched, got: {msg:?}"
    );
    // The module, not the board, recorded where the agent landed.
    let provisioned = h.rt.database.get_task(id).await.unwrap();
    assert!(provisioned.worktree.is_some());
}

/// `Research` must reach the research agent. The mode travels on the command,
/// so a dispatcher arm that dropped it would launch a standard agent instead.
/// Every launcher shares one permission mode
/// (`EveryTaskAgentLaunchesInAutoMode`), so the prompt is the marker.
#[tokio::test]
async fn dispatch_task_dispatch_agent_carries_the_mode_to_the_launcher() {
    let repo = provisioned_repo("1-test-task");
    let mut h = harness(DispatchScript::dispatch().runner()).await;
    let task = create_task_returning(
        &**h.rt.db_write(),
        "Test Task",
        "desc",
        repo.path().to_str().unwrap(),
        None,
        models::TaskStatus::Backlog,
    )
    .await
    .unwrap();

    h.dispatch(Command::Task(TaskCommand::DispatchAgent {
        id: task.id,
        mode: Some(models::DispatchMode::Research),
    }))
    .await;
    h.serve_requests().await;

    assert!(matches!(
        h.next_msg().await,
        Message::Task(crate::tui::messages::TaskMessage::Dispatched { .. })
    ));
    let prompt = std::fs::read_to_string(repo.path().join(".worktrees/1-test-task/.claude-prompt"))
        .expect("dispatch should write the prompt file");
    assert!(
        prompt.contains(crate::dispatch::RESEARCH_AGENT_INTRO),
        "research mode must reach build_research_prompt: {prompt}"
    );
}

/// The trust gate's read half. The harness's default `claude_json_path`
/// points at a nonexistent file, so the check is a deterministic
/// "untrusted" — and this arm only reads, so nothing is written even
/// there.
#[tokio::test]
async fn dispatch_task_check_trust_and_dispatch_prompts_for_an_untrusted_repo() {
    let repo = tempfile::tempdir().unwrap();
    let repo_path = repo.path().to_str().unwrap().to_string();
    let mut h = quiet_harness().await;
    let task = create_task_returning(
        &**h.rt.db_write(),
        "Needs trust",
        "desc",
        &repo_path,
        None,
        models::TaskStatus::Backlog,
    )
    .await
    .unwrap();
    h.seed_board(task.clone());

    h.dispatch(Command::Task(TaskCommand::CheckTrustAndDispatch {
        id: task.id,
        repo_path,
        mode: models::DispatchMode::Dispatch,
    }))
    .await;

    let status = h.app.status_message().unwrap_or_default();
    assert!(
        status.contains("not trusted by Claude Code"),
        "an untrusted repo must reach the confirmation prompt, got: {status:?}"
    );
    assert!(
        h.mock.recorded_calls().is_empty(),
        "nothing may be provisioned before the operator confirms"
    );
}

#[tokio::test]
async fn dispatch_task_quick_dispatch_stops_at_the_trust_prompt_for_an_untrusted_repo() {
    let repo = tempfile::tempdir().unwrap();
    let mut h = quiet_harness().await;

    h.dispatch(Command::Task(TaskCommand::QuickDispatch {
        draft: tui::TaskDraft {
            title: "Quick one".into(),
            description: String::new(),
            repo_path: repo.path().to_str().unwrap().into(),
            ..Default::default()
        },
        epic_id: None,
    }))
    .await;

    let status = h.app.status_message().unwrap_or_default();
    assert!(
        status.contains("not trusted by Claude Code"),
        "quick dispatch must gate on trust too, got: {status:?}"
    );
    assert!(
        h.rt.database.list_all().await.is_empty(),
        "no task row may be created before the operator confirms"
    );
}

/// A tempfile path for `claude_json_path`, pointing at a file that does not
/// exist yet — the shape `trust_at` creates on its first grant.
fn tempfile_claude_json_path() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".claude.json");
    (dir, path)
}

#[tokio::test]
async fn dispatch_task_trust_and_dispatch_grants_trust_then_dispatches() {
    let repo = provisioned_repo("1-test-task");
    let (_claude_dir, claude_json_path) = tempfile_claude_json_path();
    let mut h = harness(DispatchScript::dispatch().runner()).await;
    h.rt.claude_json_path = claude_json_path.clone();
    let task = create_task_returning(
        &**h.rt.db_write(),
        "Test Task",
        "desc",
        repo.path().to_str().unwrap(),
        None,
        models::TaskStatus::Backlog,
    )
    .await
    .unwrap();
    let repo_path = task.repo_path.clone();

    h.dispatch(Command::Task(TaskCommand::TrustAndDispatch {
        task: Box::new(task),
        mode: models::DispatchMode::Dispatch,
    }))
    .await;
    h.serve_requests().await;

    assert!(matches!(
        h.next_msg().await,
        Message::Task(crate::tui::messages::TaskMessage::Dispatched { .. })
    ));
    assert!(
        crate::dispatch::is_trusted_at(&claude_json_path, &repo_path).unwrap(),
        "the grant must be durable, not just in-memory permission to proceed"
    );
}

/// The grant's own failure — a `claude.json` that cannot even be parsed —
/// must abandon the dispatch before anything is requested or provisioned.
#[tokio::test]
async fn dispatch_task_trust_and_dispatch_abandons_on_a_trust_write_failure() {
    let (_claude_dir, claude_json_path) = tempfile_claude_json_path();
    std::fs::write(&claude_json_path, "not json").unwrap();
    let mut h = quiet_harness().await;
    h.rt.claude_json_path = claude_json_path;
    let task = create_task_returning(
        &**h.rt.db_write(),
        "Test Task",
        "desc",
        "/repo",
        None,
        models::TaskStatus::Backlog,
    )
    .await
    .unwrap();
    let id = task.id;
    // Mirrors `handle_dispatch_task`, which marks a task dispatching before
    // ever queuing `TrustAndDispatch` — without this, "not dispatching"
    // would hold trivially before the arm even runs.
    h.app.update(Message::Task(
        crate::tui::messages::TaskMessage::MarkDispatching(id),
    ));

    h.dispatch(Command::Task(TaskCommand::TrustAndDispatch {
        task: Box::new(task),
        mode: models::DispatchMode::Dispatch,
    }))
    .await;

    // Both the abandon and the error apply directly to `app`, the same way
    // `CheckTrustAndDispatch`'s untrusted branch does — this runs
    // synchronously inside `dispatch_task`, never through `msg_tx`.
    assert!(
        !h.app.is_dispatching(id),
        "an abandoned dispatch must clear the dispatching marker"
    );
    let err = h.app.error_popup().unwrap_or_default();
    assert!(err.contains("Failed to trust repo"), "got: {err:?}");
    assert!(
        h.mock.recorded_calls().is_empty(),
        "nothing may be provisioned when the trust grant itself fails"
    );
}

#[tokio::test]
async fn dispatch_task_trust_and_quick_dispatch_grants_trust_then_dispatches() {
    let dir = tempfile::tempdir().unwrap();
    let repo_path = dir.path().to_str().unwrap().to_string();
    crate::dispatch::tests::create_worktree_dir(dir.path().join(".worktrees/1-quick-one"));
    let (_claude_dir, claude_json_path) = tempfile_claude_json_path();
    let mut h = harness(
        DispatchScript::dispatch()
            .detecting_default_branch("main")
            .runner(),
    )
    .await;
    h.rt.claude_json_path = claude_json_path.clone();

    h.dispatch(Command::Task(TaskCommand::TrustAndQuickDispatch {
        draft: tui::TaskDraft {
            title: "Quick one".into(),
            description: String::new(),
            repo_path: repo_path.clone(),
            ..Default::default()
        },
        epic_id: None,
    }))
    .await;
    h.serve_requests().await;

    // `Created` applies to `app` directly, the same way `MarkDispatching`
    // does — only the eventual dispatch outcome travels over `msg_tx`.
    assert_eq!(
        h.app.tasks().len(),
        1,
        "quick dispatch must still create the task row on the granted path"
    );
    assert_eq!(h.app.tasks()[0].title, "Quick one");
    assert!(
        crate::dispatch::is_trusted_at(&claude_json_path, &repo_path).unwrap(),
        "the grant must be durable"
    );
    assert!(matches!(
        h.next_msg().await,
        Message::Task(crate::tui::messages::TaskMessage::Dispatched { .. })
    ));
}

/// The trusted-repo halves of `CheckTrustAndDispatch`/`QuickDispatch`: the
/// untrusted-repo tests above cover the read failing to find trust, so this
/// pair covers it finding trust and letting the dispatch through instead.
#[tokio::test]
async fn dispatch_task_check_trust_and_dispatch_proceeds_when_already_trusted() {
    let repo = provisioned_repo("1-test-task");
    let repo_path = repo.path().to_str().unwrap().to_string();
    let (_claude_dir, claude_json_path) = tempfile_claude_json_path();
    crate::dispatch::trust_at(&claude_json_path, &repo_path).unwrap();
    let mut h = harness(DispatchScript::dispatch().runner()).await;
    h.rt.claude_json_path = claude_json_path;
    let task = create_task_returning(
        &**h.rt.db_write(),
        "Test Task",
        "desc",
        &repo_path,
        None,
        models::TaskStatus::Backlog,
    )
    .await
    .unwrap();
    h.seed_board(task.clone());

    let extra = h
        .dispatch(Command::Task(TaskCommand::CheckTrustAndDispatch {
            id: task.id,
            repo_path,
            mode: models::DispatchMode::Dispatch,
        }))
        .await;
    for cmd in extra {
        drain(&h.rt, &mut h.app, cmd).await;
    }
    h.serve_requests().await;

    assert!(
        matches!(
            h.next_msg().await,
            Message::Task(crate::tui::messages::TaskMessage::Dispatched { .. })
        ),
        "an already-trusted repo must reach the real dispatcher, not a prompt"
    );
}

#[tokio::test]
async fn dispatch_task_quick_dispatch_proceeds_when_already_trusted() {
    let dir = tempfile::tempdir().unwrap();
    let repo_path = dir.path().to_str().unwrap().to_string();
    crate::dispatch::tests::create_worktree_dir(dir.path().join(".worktrees/1-quick-one"));
    let (_claude_dir, claude_json_path) = tempfile_claude_json_path();
    crate::dispatch::trust_at(&claude_json_path, &repo_path).unwrap();
    let mut h = harness(
        DispatchScript::dispatch()
            .detecting_default_branch("main")
            .runner(),
    )
    .await;
    h.rt.claude_json_path = claude_json_path;

    h.dispatch(Command::Task(TaskCommand::QuickDispatch {
        draft: tui::TaskDraft {
            title: "Quick one".into(),
            description: String::new(),
            repo_path,
            ..Default::default()
        },
        epic_id: None,
    }))
    .await;
    h.serve_requests().await;

    assert_eq!(
        h.app.tasks().len(),
        1,
        "an already-trusted repo must proceed straight to task creation, not a prompt"
    );
    assert!(matches!(
        h.next_msg().await,
        Message::Task(crate::tui::messages::TaskMessage::Dispatched { .. })
    ));
}
