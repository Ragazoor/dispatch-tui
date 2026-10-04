use super::*;

// ---------------------------------------------------------------------------
// base_branch: create_task and update_task MCP schema tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_task_with_base_branch_stores_it() {
    let state = test_state().await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "My Feature",
                "repo_path": "/repo",
                "epic_id": null,
                "base_branch": "develop",
            }
        })),
    )
    .await;

    assert!(resp.error.is_none(), "{:?}", resp.error);
    let tasks = state.db.list_all().await.unwrap();
    let task = tasks.iter().find(|t| t.title == "My Feature").unwrap();
    assert_eq!(task.base_branch, "develop");
}

/// The base branch a `create_task` call lands on when it names none, given a
/// repository that answers `probe` to `git symbolic-ref refs/remotes/origin/HEAD`.
///
/// Both cases differ only in that one response and the branch they expect, so
/// the difference is the whole of each test rather than four lines buried in
/// thirty identical ones.
async fn base_branch_created_with(probe: anyhow::Result<std::process::Output>) -> String {
    let (state, _db) =
        test_state_with_overrides(Arc::new(MockProcessRunner::new(vec![probe])), None, None).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "Default Branch Task",
                "repo_path": "/repo",
                "epic_id": null,
            }
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);

    state
        .db
        .list_all()
        .await
        .unwrap()
        .iter()
        .find(|t| t.title == "Default Branch Task")
        .unwrap()
        .base_branch
        .clone()
}

#[tokio::test]
async fn create_task_without_base_branch_detects_the_repo_default() {
    // BaseBranchIsResolvedNotAssumed (docs/specs/mcp-task-tools.allium). This
    // tool is the creation path with no human in front of it: the TUI form
    // shows its answer in a picker the user can correct, and quick dispatch
    // goes through the same service resolution. Assuming "main" here produced
    // tasks in "master" repos that could not be dispatched at all.
    let branch = base_branch_created_with(MockProcessRunner::ok_with_stdout(
        b"refs/remotes/origin/master\n",
    ))
    .await;
    assert_eq!(branch, "master");
}

#[tokio::test]
async fn create_task_without_base_branch_falls_back_to_main_when_the_repo_names_no_default() {
    // config.default_branch is the detection helper's own last resort. The
    // tool stays total: it never refuses over a branch the caller did not ask
    // about, because a repo that is temporarily unreachable must not break a
    // bulk decomposition.
    let branch = base_branch_created_with(MockProcessRunner::fail(
        "fatal: ref refs/remotes/origin/HEAD is not a symbolic ref",
    ))
    .await;
    assert_eq!(branch, "main");
}

#[tokio::test]
async fn update_task_with_base_branch_updates_it() {
    let state = test_state().await;

    let task_id = state
        .db_write()
        .create_task(CreateTaskRequest {
            title: "T",
            description: "d",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": {
                "task_id": task_id.0,
                "base_branch": "release/2.0"
            }
        })),
    )
    .await;

    assert!(resp.error.is_none(), "{:?}", resp.error);
    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(task.base_branch, "release/2.0");
}

// `dispatch_next_returns_disabled_when_auto_dispatch_off` moved to
// tests/tasks/dispatch.rs as `exit_session_does_not_chain_when_auto_dispatch_off`
// — the auto_dispatch flag is now read by the session-close chain, not by a tool.

// -- list_tasks: header-based caller identity ---------------------------------

#[tokio::test]
async fn list_tasks_task_identity_scopes_to_epic_and_excludes_self() {
    let (state, db) = test_state_with_db().await;
    let eid = db.create_epic("e", "", None).await.unwrap().id;
    let me = db
        .create_task(CreateTaskRequest {
            title: "me",
            description: "",
            repo_path: "/r",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: Some(eid),
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();
    let sibling = db
        .create_task(CreateTaskRequest {
            title: "sibling",
            description: "",
            repo_path: "/r",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: Some(eid),
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();
    let _unrelated = db
        .create_task(CreateTaskRequest {
            title: "unrelated",
            description: "",
            repo_path: "/r",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    let resp = call_as(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
        CallerIdentity::Task(me),
    )
    .await;

    let text = extract_response_text(&resp);
    // sibling is in scope (same epic); me is excluded (self); unrelated is out of scope.
    assert!(
        text.contains(&format!("[{}]", sibling.0)),
        "expected sibling in:\n{text}"
    );
    assert!(
        !text.contains(&format!("[{}]", me.0)),
        "self should be excluded:\n{text}"
    );
}

#[tokio::test]
async fn list_tasks_task_identity_scopes_to_project_when_no_epic() {
    let (state, db) = test_state_with_db().await;
    let me = db
        .create_task(CreateTaskRequest {
            title: "me",
            description: "",
            repo_path: "/r",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();
    let sibling = db
        .create_task(CreateTaskRequest {
            title: "sib",
            description: "",
            repo_path: "/r",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    let resp = call_as(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
        CallerIdentity::Task(me),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains(&format!("[{}]", sibling.0)),
        "expected sibling:\n{text}"
    );
    assert!(
        !text.contains(&format!("[{}]", me.0)),
        "self excluded:\n{text}"
    );
}

#[tokio::test]
async fn list_tasks_session_identity_sees_all_tasks() {
    let (state, db) = test_state_with_db().await;
    db.create_task(CreateTaskRequest {
        title: "t1",
        description: "",
        repo_path: "/r",
        plan: None,
        status: TaskStatus::Backlog,
        base_branch: "main",
        epic_id: None,
        sort_order: None,
        tag: None,
        wrap_up_mode: None,
        auto_run_plan: false,
        phoenix: false,
    })
    .await
    .unwrap();
    db.create_task(CreateTaskRequest {
        title: "t2",
        description: "",
        repo_path: "/r",
        plan: None,
        status: TaskStatus::Backlog,
        base_branch: "main",
        epic_id: None,
        sort_order: None,
        tag: None,
        wrap_up_mode: None,
        auto_run_plan: false,
        phoenix: false,
    })
    .await
    .unwrap();

    let resp = call_as(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
        CallerIdentity::Session,
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(text.contains("t1"), "got:\n{text}");
    assert!(text.contains("t2"), "got:\n{text}");
}

#[tokio::test]
async fn list_tasks_repo_paths_filter() {
    let state = test_state().await;

    state
        .db_write()
        .create_task(CreateTaskRequest {
            title: "Repo A task",
            description: "",
            repo_path: "/repo/a",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();
    state
        .db_write()
        .create_task(CreateTaskRequest {
            title: "Repo B task",
            description: "",
            repo_path: "/repo/b",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "list_tasks",
            "arguments": { "repo_paths": ["/repo/a"] }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(text.contains("Repo A task"));
    assert!(!text.contains("Repo B task"));
}

#[tokio::test]
async fn list_tasks_includes_pr_url_in_output() {
    let state = test_state().await;

    let task_id = create_task_fixture(&state).await;
    let url = crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/42",
        crate::models::UrlType::Pr,
    );
    state
        .db_write()
        .patch_task(task_id, &crate::db::TaskPatch::new().url(Some(&url)))
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("| PR #42: https://github.com/org/repo/pull/42"),
        "PR URL should appear in output; got: {text}"
    );
}

#[tokio::test]
async fn list_tasks_includes_plan_goal_in_output() {
    let state = test_state().await;

    let plan_path = std::env::temp_dir().join("dispatch_test_plan_345.md");
    std::fs::write(
        &plan_path,
        "# My Feature — Implementation Plan\n\n**Goal:** Implement the learning enrichment.\n",
    )
    .unwrap();
    let plan_path_str = plan_path.to_string_lossy().to_string();

    state
        .db_write()
        .create_task(CreateTaskRequest {
            title: "Feature task",
            description: "desc",
            repo_path: "/repo",
            plan: Some(&plan_path_str),
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("| Goal: Implement the learning enrichment."),
        "Plan goal should appear in output; got: {text}"
    );

    let _ = std::fs::remove_file(&plan_path);
}

#[tokio::test]
async fn list_tasks_falls_back_to_description_when_no_plan() {
    let state = test_state().await;

    state
        .db_write()
        .create_task(CreateTaskRequest {
            title: "No Plan Task",
            description: "A task without a plan file",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("A task without a plan file"),
        "Description should appear as fallback; got: {text}"
    );
}

#[tokio::test]
async fn list_tasks_omits_pr_segment_when_no_pr_url() {
    let state = test_state().await;
    create_task_fixture(&state).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "list_tasks", "arguments": {} })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        !text.contains("| PR:"),
        "No PR segment should appear when pr_url is null; got: {text}"
    );
}

// -- update_task PR-finalisation nudge tests -------------------------------
//
// When the agent records a freshly-created PR via update_task (per the
// agent-driven /wrap-up flow), the response should append the same
// reflection nudge that the rebase wrap_up emits — i.e. when pr_url
// transitions from null to a value AND status is being set to review.

#[tokio::test]
async fn update_task_pr_finalisation_appends_reflection_nudge_by_default() {
    let state = test_state().await;
    let task_id = state
        .db_write()
        .create_task(CreateTaskRequest {
            title: "PR finalise",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": {
                "task_id": task_id.0,
                "url": "https://github.com/org/repo/pull/7", "url_type": "pr",
                "status": "review"
            }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        text.contains("record_learning"),
        "nudge should appear when finalising a PR via update_task; got: {text}"
    );
}

#[tokio::test]
async fn update_task_pr_finalisation_omits_nudge_when_disabled() {
    let state = test_state().await;
    state
        .db
        .set_setting_bool("learning_reflection_enabled", false)
        .await
        .unwrap();
    let task_id = state
        .db_write()
        .create_task(CreateTaskRequest {
            title: "PR finalise disabled",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": {
                "task_id": task_id.0,
                "url": "https://github.com/org/repo/pull/7", "url_type": "pr",
                "status": "review"
            }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        !text.contains("record_learning"),
        "nudge must not appear when reflection disabled; got: {text}"
    );
}

#[tokio::test]
async fn update_task_pr_set_without_status_does_not_nudge() {
    // Agent setting only pr_url (no status transition) is not a wrap-up
    // finalisation — don't nudge. This preserves current update_task UX
    // for non-wrap-up callers tweaking the URL.
    let state = test_state().await;
    let task_id = state
        .db_write()
        .create_task(CreateTaskRequest {
            title: "PR set no status",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": {
                "task_id": task_id.0,
                "url": "https://github.com/org/repo/pull/7", "url_type": "pr"
            }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        !text.contains("record_learning"),
        "nudge must not appear when status is not transitioning; got: {text}"
    );
}

#[tokio::test]
async fn update_task_status_review_without_pr_url_change_does_not_nudge() {
    // Re-confirming a task to review without setting a new pr_url is
    // not a wrap-up finalisation. No nudge.
    let state = test_state().await;
    let task_id = state
        .db_write()
        .create_task(CreateTaskRequest {
            title: "Already in review",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": {
                "task_id": task_id.0,
                "status": "review"
            }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        !text.contains("record_learning"),
        "nudge must not appear without pr_url transition; got: {text}"
    );
}

#[tokio::test]
async fn update_task_pr_url_already_set_does_not_nudge_again() {
    // The nudge should fire only on the first null->set transition.
    // Subsequent updates to pr_url (e.g. correcting the URL) must not
    // re-nudge.
    let state = test_state().await;
    let task_id = state
        .db_write()
        .create_task(CreateTaskRequest {
            title: "PR already set",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Review,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();
    let url = crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/1",
        crate::models::UrlType::Pr,
    );
    state
        .db_write()
        .patch_task(task_id, &db::TaskPatch::new().url(Some(&url)))
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": {
                "task_id": task_id.0,
                "url": "https://github.com/org/repo/pull/2", "url_type": "pr",
                "status": "review"
            }
        })),
    )
    .await;

    let text = extract_response_text(&resp);
    assert!(
        !text.contains("record_learning"),
        "nudge must not fire when pr_url was already set; got: {text}"
    );
}

// -- create_task: header-based caller identity --------------------------------

// -- epic_id is required, and never inherited --------------------------------
//
// mcp-task-tools.allium: CreateTaskViaMcp / EveryTaskNamesItsEpicOrNull. The
// ARGUMENT must be present; the VALUE may be null. Omitting it is refused for
// both caller kinds, and a Task-kind caller's own epic is never consulted.

/// Creates an epic and a Running task inside it, returning both ids. The
/// stand-in for "a dispatched agent that belongs to an epic".
async fn caller_task_in_epic(
    state: &Arc<McpState>,
) -> (crate::models::EpicId, crate::models::TaskId) {
    let epic = state
        .db_write()
        .create_epic("parent epic", "", None)
        .await
        .unwrap();
    let task = state
        .db_write()
        .create_task(CreateTaskRequest {
            title: "parent",
            description: "",
            repo_path: "/r",
            plan: None,
            status: TaskStatus::Running,
            base_branch: "main",
            epic_id: Some(epic.id),
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();
    (epic.id, task)
}

/// allow-phantom-symbol: names the test this one replaced, kept so the inversion stays traceable.
/// The inversion of the old `create_task_task_identity_inherits_epic`: a
/// dispatched agent that omits epic_id is REFUSED, and nothing is created. The
/// caller's own epic is not consulted — silent inheritance propagated an
/// epic-less caller's blank down a whole lineage.
#[tokio::test]
async fn create_task_task_identity_does_not_inherit_epic() {
    let state = test_state().await;
    let (_parent_epic, parent) = caller_task_in_epic(&state).await;

    let resp = call_as(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "title": "child", "repo_path": "/r" }
        })),
        CallerIdentity::Task(parent),
    )
    .await;

    assert!(is_error(&resp), "omitted epic_id must be refused");
    let tasks = state.db.list_all().await.unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "nothing may be created by a refused call; got {tasks:?}"
    );
    assert_eq!(tasks[0].id, parent, "only the caller's own task may exist");
}

/// Same refusal for a non-dispatched session — the rule is not caller-kind
/// specific, and nothing is created.
#[tokio::test]
async fn create_task_session_identity_requires_epic_id() {
    let state = test_state().await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "title": "t", "repo_path": "/r" }
        })),
    )
    .await;

    assert!(is_error(&resp), "omitted epic_id must be refused");
    assert!(
        state.db.list_all().await.unwrap().is_empty(),
        "nothing may be created by a refused call"
    );
}

/// TheRefusalNamesTheLikelyAnswer. When the caller is Task(t) and that task has
/// an epic, the refusal names that epic id as the likely answer and says null
/// means deliberately standalone. A bare "missing field" would make the agent
/// guess; this is why the check lives in the handler, where caller identity is
/// known, and not at the deserialization boundary.
#[tokio::test]
async fn create_task_refusal_names_the_callers_epic_and_the_null_option() {
    let state = test_state().await;
    let (parent_epic, parent) = caller_task_in_epic(&state).await;

    let resp = call_as(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "title": "child", "repo_path": "/r" }
        })),
        CallerIdentity::Task(parent),
    )
    .await;

    let msg = error_message(&resp);
    assert!(
        msg.contains("epic_id"),
        "refusal must name the argument, got: {msg}"
    );
    assert!(
        msg.contains(&format!("#{}", parent_epic.0)),
        "refusal must name the caller's epic as the likely answer, got: {msg}"
    );
    assert!(
        msg.contains("null"),
        "refusal must offer null as the other answer, got: {msg}"
    );
    assert!(
        msg.to_lowercase().contains("standalone"),
        "refusal must say null means deliberately standalone, got: {msg}"
    );
}

/// The other half of TheRefusalNamesTheLikelyAnswer: with no epic to name, the
/// refusal says the argument is required and invents nothing. Any digit in the
/// message would be an id the caller did not supply and the system does not
/// know — exactly the guess this rule exists to stop.
#[tokio::test]
async fn create_task_refusal_invents_no_epic_for_an_epicless_task_caller() {
    let state = test_state().await;
    let parent = create_task_fixture(&state).await; // no epic

    let resp = call_as(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "title": "child", "repo_path": "/r" }
        })),
        CallerIdentity::Task(parent),
    )
    .await;

    assert_bare_refusal(&resp, "a caller whose own task has no epic");
}

/// The refusal a caller with no epic to name gets: it says what is required and
/// invents nothing. Any digit would be an id the caller did not supply and the
/// system does not know — exactly the guess this rule exists to stop, which is
/// why "names no id" is asserted as "carries no digit at all".
fn assert_bare_refusal(resp: &JsonRpcResponse, whose: &str) {
    let msg = error_message(resp);
    assert!(
        msg.contains("epic_id") && msg.to_lowercase().contains("required"),
        "refusal must say epic_id is required, got: {msg}"
    );
    assert!(
        !msg.chars().any(|c| c.is_ascii_digit()),
        "refusal must not name any id for {whose}, got: {msg}"
    );
}

/// A Session caller has no epic to be named either — it is told the argument is
/// required and nothing more (CallerIdentityDependsOnTheLaunch: a misconfigured
/// agent loses the refusal's help, not the refusal).
#[tokio::test]
async fn create_task_refusal_invents_no_epic_for_a_session_caller() {
    let state = test_state().await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "title": "t", "repo_path": "/r" }
        })),
    )
    .await;

    assert_bare_refusal(&resp, "a session caller");
}

/// The effective epic is exactly the argument: an explicit null produces a
/// standalone task even though the caller's own task has an epic.
#[tokio::test]
async fn create_task_explicit_null_epic_creates_a_standalone_task() {
    let state = test_state().await;
    let (_parent_epic, parent) = caller_task_in_epic(&state).await;

    let resp = call_as(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "title": "t", "repo_path": "/r", "epic_id": null }
        })),
        CallerIdentity::Task(parent),
    )
    .await;
    let new_id = extract_created_task_id(&resp);
    let t = state.db.get_task(new_id).await.unwrap().unwrap();
    assert_eq!(t.epic_id, None);
}

#[tokio::test]
async fn create_task_unknown_caller_identity_returns_error() {
    let state = test_state().await;
    let resp = call_as(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "title": "t", "repo_path": "/r", "epic_id": null }
        })),
        CallerIdentity::Task(crate::models::TaskId(99999)),
    )
    .await;
    assert!(is_error(&resp));
    let msg = error_message(&resp);
    assert!(msg.to_lowercase().contains("caller"), "got {msg}");
}

#[tokio::test]
async fn get_task_shows_wrap_up_mode_when_set() {
    let state = test_state().await;
    let task_id = state
        .db_write()
        .create_task(CreateTaskRequest {
            title: "T",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: Some(crate::models::WrapUpMode::Rebase),
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "get_task", "arguments": { "task_id": task_id.0 } })),
    )
    .await;
    let text = extract_response_text(&resp);
    // The exact label matters, not just the words: get_task returns prose, and
    // the /wrap-up skill reads the mode off this line by name. A rename here
    // silently sends the skill back to asking a question it was told to skip.
    assert!(
        text.contains("Wrap-up mode: rebase"),
        "expected 'Wrap-up mode: rebase' in output, got: {text}"
    );
}

#[tokio::test]
async fn get_task_shows_verify_command_when_configured() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;
    state.db_write().save_repo_path("/repo").await.unwrap();
    state
        .db_write()
        .set_verify_command("/repo", Some("cargo test"))
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "get_task", "arguments": { "task_id": task_id.0 } })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        text.contains("Verify command: cargo test"),
        "expected 'Verify command: cargo test' in output, got: {text}"
    );
}

#[tokio::test]
async fn get_task_omits_verify_command_when_unconfigured() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "get_task", "arguments": { "task_id": task_id.0 } })),
    )
    .await;
    let text = extract_response_text(&resp);
    assert!(
        !text.contains("Verify command"),
        "expected no 'Verify command' line in output, got: {text}"
    );
}

// -- phoenix ---------------------------------------------------------------

#[tokio::test]
async fn create_task_accepts_phoenix() {
    let state = test_state().await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "Weekly dep audit",
                "repo_path": "/repo",
                "epic_id": null,
                "phoenix": true
            }
        })),
    )
    .await;

    let id = extract_created_task_id(&resp);
    let task = state.db.get_task(id).await.unwrap().unwrap();
    assert!(task.phoenix);
}

#[tokio::test]
async fn create_task_defaults_phoenix_to_false() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;
    assert!(!state.db.get_task(task_id).await.unwrap().unwrap().phoenix);
}

#[tokio::test]
async fn update_task_sets_and_clears_phoenix() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;

    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "phoenix": true }
        })),
    )
    .await;
    assert!(state.db.get_task(task_id).await.unwrap().unwrap().phoenix);

    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "phoenix": false }
        })),
    )
    .await;
    assert!(!state.db.get_task(task_id).await.unwrap().unwrap().phoenix);
}

/// Omitting the field is a no-op, not a clear — the same nullable-boolean
/// semantics `auto_run_plan` has.
#[tokio::test]
async fn update_task_omitting_phoenix_leaves_it_alone() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;
    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "phoenix": true }
        })),
    )
    .await;

    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "title": "renamed" }
        })),
    )
    .await;

    assert!(state.db.get_task(task_id).await.unwrap().unwrap().phoenix);
}

/// So a dispatched agent wrapping up knows it is finishing THIS run of a
/// recurring task, not the task itself.
#[tokio::test]
async fn get_task_shows_phoenix_when_set_and_omits_it_otherwise() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;

    let plain = extract_response_text(
        &call(
            &state,
            "tools/call",
            Some(json!({ "name": "get_task", "arguments": { "task_id": task_id.0 } })),
        )
        .await,
    );
    assert!(
        !plain.contains("Phoenix"),
        "an ordinary task shows no Phoenix line, got: {plain}"
    );

    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "phoenix": true }
        })),
    )
    .await;
    let recurring = extract_response_text(
        &call(
            &state,
            "tools/call",
            Some(json!({ "name": "get_task", "arguments": { "task_id": task_id.0 } })),
        )
        .await,
    );
    assert!(
        recurring.contains("Phoenix"),
        "expected a Phoenix line, got: {recurring}"
    );
}
