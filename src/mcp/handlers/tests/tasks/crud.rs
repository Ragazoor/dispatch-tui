use super::*;
use crate::dispatch::mock_sequence::DispatchScript;
use crate::models::test_tmux_window;

// -- update_task boundary parity ---------------------------------------------
//
// These guard the `mcp_args!` generation itself (see `src/mcp/handlers/args.rs`
// for what it emits and why): that the schema is well-formed, that every name it
// advertises is one the parser accepts, and that every advertised field still
// reaches the service params.

fn update_task_schema() -> serde_json::Value {
    crate::mcp::handlers::tasks::update_task_schema()
}

/// The advertised property set is exactly the struct's field set. A schema
/// property the struct does not have would be a runtime `-32602` on the first
/// agent that believed the schema.
#[test]
fn update_task_schema_properties_match_the_args_struct() {
    let schema = update_task_schema();
    let mut advertised: Vec<&str> = schema["properties"]
        .as_object()
        .expect("properties must be an object")
        .keys()
        .map(String::as_str)
        .collect();
    advertised.sort_unstable();

    let mut declared: Vec<&str> = crate::mcp::handlers::tasks::UpdateTaskArgs::FIELD_NAMES.to_vec();
    declared.sort_unstable();

    assert_eq!(advertised, declared);
}

/// The JSON type a property really is, ignoring a nullable field's `"null"`
/// alternative. `"type"` is either a bare string or an array of them.
fn nominal_type(ty: &serde_json::Value) -> Option<&str> {
    match ty {
        serde_json::Value::String(s) => Some(s.as_str()),
        serde_json::Value::Array(members) => members
            .iter()
            .filter_map(serde_json::Value::as_str)
            .find(|m| *m != "null"),
        _ => None,
    }
}

/// Every advertised field name is one `deny_unknown_fields` accepts. This is
/// the leg that used to fail at runtime: a schema/struct name mismatch was a
/// `-32602 Invalid arguments` rather than a build or test failure.
#[test]
fn every_advertised_update_task_field_is_accepted_by_the_parser() {
    let schema = update_task_schema();
    let properties = schema["properties"].as_object().unwrap();

    // One representative valid value per JSON type the schema advertises. The
    // point is name acceptance, not value validation, so enums use their first
    // advertised member.
    for (name, spec) in properties {
        let value = match spec.get("enum").and_then(|e| e.as_array()) {
            Some(members) => members
                .iter()
                .find(|m| !m.is_null())
                .expect("an enum must advertise at least one non-null member")
                .clone(),
            // A nullable field advertises `["<type>", "null"]`. Clearing is
            // covered elsewhere; here the representative value is the
            // non-null half.
            None => match nominal_type(&spec["type"]) {
                Some("integer") => json!(1),
                Some("boolean") => json!(true),
                Some("string") => json!("x"),
                other => panic!("field {name} has unhandled schema type {other:?}"),
            },
        };
        let args = json!({ "task_id": 1, (name.as_str()): value });
        let parsed = serde_json::from_value::<crate::mcp::handlers::tasks::UpdateTaskArgs>(args);
        assert!(
            parsed.is_ok(),
            "schema advertises {name} but the args struct rejects it: {:?}",
            parsed.err()
        );
    }
}

/// `task_id` must stay the only required field — every other one is a no-op
/// when absent, so it is the only one the tool cannot work without.
/// (`tool_schemas_have_consistent_required_fields` in `tests/mod.rs` already
/// checks registry-wide that `required` names real properties.)
#[test]
fn update_task_schema_required_list_is_task_id_only() {
    assert_eq!(update_task_schema()["required"], json!(["task_id"]));
}

/// Every property carries a description — the schema is the only documentation
/// an agent sees for these fields.
#[test]
fn every_update_task_property_is_documented() {
    for (name, spec) in update_task_schema()["properties"].as_object().unwrap() {
        assert!(
            spec.get("description")
                .and_then(|d| d.as_str())
                .is_some_and(|d| !d.is_empty()),
            "field {name} has no description"
        );
    }
}

/// The mapping leg: sending every advertised field in one call must report
/// every corresponding params field as updated. A field present in the struct
/// and the schema but dropped on the way to `UpdateTaskParams` would be
/// silently ignored — the failure mode this whole boundary exists to prevent.
#[tokio::test]
async fn update_task_maps_every_advertised_field_to_params() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;
    let epic = state.db_write().create_epic("E", "", None).await.unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": {
                "task_id": task_id.0,
                "status": "running",
                "plan_path": "/plans/p.md",
                "title": "new title",
                "description": "new description",
                "repo_path": "/repo",
                "sort_order": 3,
                "url": "https://github.com/org/repo/pull/1",
                "url_type": "pr",
                "tag": "bug",
                "sub_status": "active",
                "epic_id": epic.id.0,
                "base_branch": "develop",
                "wrap_up_mode": "pr",
                "auto_run_plan": true,
                "phoenix": true,
            }
        })),
    )
    .await;

    let text = resp.result.as_ref().unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();

    // Only the `[manual]` fields are exempt, and the exemption set is derived
    // from the macro rather than hand-listed — so declaring a new `[manual]`
    // field cannot quietly widen what this test forgives.
    let manual = crate::mcp::handlers::tasks::UpdateTaskArgs::manual_fields();
    for field in crate::mcp::handlers::tasks::UpdateTaskArgs::FIELD_NAMES {
        if manual.contains(field) {
            continue;
        }
        assert!(
            text.contains(field),
            "update_task did not report {field} as updated: {text}"
        );
    }

    // `url` is `[manual]` but the handler does map it, so assert it explicitly
    // rather than letting the exemption above cover for a regression there.
    // (`url_type` has no params field of its own — it folds into `url`.)
    assert!(
        text.contains("url"),
        "update_task did not report url as updated: {text}"
    );
}

// -- update_task tests -------------------------------------------------------

#[tokio::test]
async fn update_task_valid() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "running" }
        })),
    )
    .await;
    assert!(resp.result.is_some());
    assert!(resp.error.is_none());

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(task.status, crate::models::TaskStatus::Running);
}

#[tokio::test]
async fn update_task_invalid_status() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "bogus" }
        })),
    )
    .await;
    assert_error(&resp, "unknown variant `bogus`");
}

/// mcp-task-tools.allium: UpdateTaskViaMcp no longer names `archived` at
/// all — it is not a TaskStatus any more (core.allium), so the value fails
/// to decode exactly like any other unknown status, and the task is unchanged.
#[tokio::test]
async fn update_task_rejects_archived_as_an_unknown_status() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;
    let before = state.db.get_task(task_id).await.unwrap().unwrap().status;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "archived" }
        })),
    )
    .await;
    assert_error(&resp, "unknown variant `archived`");

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(task.status, before);
}

// -- update_task(status="done") — MarkTaskDoneViaMcp ------------------------
//
// A dedicated close-only path (mcp-task-tools.allium: MarkTaskDoneViaMcp),
// distinct from ExitSessionViaMcp: no exit token, reachable for a session
// caller or a dispatched agent acting on a task other than its own, never for
// a dispatched agent closing itself. See dispatch.rs's ChainFixture-based
// tests for the tmux-teardown and epic-auto-dispatch-chain parity with
// exit_session.

#[tokio::test]
async fn update_task_done_marks_task_done_for_session_caller() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "done" }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "expected success, got: {:?}",
        resp.error
    );
    assert_eq!(
        extract_response_text(&resp),
        format!("Task #{} marked done.", task_id.0)
    );

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(task.status, crate::models::TaskStatus::Done);
    assert_eq!(
        task.sub_status,
        crate::models::SubStatus::default_for(crate::models::TaskStatus::Done)
    );
    assert!(
        task.completed_at.is_some(),
        "entering done should stamp the completion time"
    );
}

/// Re-closing an already-done task is a no-op for its completion time:
/// `completed_at_for_status_transition` treats done -> done as a no-op for
/// every caller of `TaskService::close_session`, this path included. A task
/// that was already finished did not finish again.
#[tokio::test]
async fn update_task_done_reclose_keeps_existing_completed_at() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;

    call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "done" }
        })),
    )
    .await;
    let first_completed_at = state
        .db
        .get_task(task_id)
        .await
        .unwrap()
        .unwrap()
        .completed_at;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "done" }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "re-closing an already-done task must succeed"
    );

    let second_completed_at = state
        .db
        .get_task(task_id)
        .await
        .unwrap()
        .unwrap()
        .completed_at;
    assert_eq!(
        first_completed_at, second_completed_at,
        "re-closing an already-done task must not re-date it"
    );
}

#[tokio::test]
async fn update_task_done_rejects_when_combined_with_other_fields() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "done", "title": "new title" }
        })),
    )
    .await;
    assert_error(&resp, "must be the only field set");
    assert_error(&resp, "title");

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_ne!(task.status, crate::models::TaskStatus::Done);
    assert_eq!(
        task.title, "Test Task",
        "a rejected close must not mutate the task"
    );
}

/// `url_type` has no params-level slot without a `url` alongside it (it is a
/// `[manual]` field only ever consumed inside the non-empty-`url` branch), so
/// checking field-exclusivity against `UpdateTaskParams` would let this
/// combination through unnoticed. This exercises the fix: the raw-args check
/// in `other_update_task_fields_set` catches it directly.
#[tokio::test]
async fn update_task_done_rejects_when_combined_with_bare_url_type() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "done", "url_type": "pr" }
        })),
    )
    .await;
    assert_error(&resp, "must be the only field set");
    assert_error(&resp, "url_type");

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_ne!(task.status, crate::models::TaskStatus::Done);
}

/// The done-only check runs before the url/url_type pairing validation, so a
/// `status="done"` call combined with `url` (missing its required `url_type`)
/// still gets MarkTaskDoneViaMcp's own close-only error naming `url` — not
/// the sibling rule's "url_type is required when url is set" — and the task
/// is untouched either way.
#[tokio::test]
async fn update_task_done_combined_with_url_reports_the_close_only_error_first() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": {
                "task_id": task_id.0,
                "status": "done",
                "url": "https://github.com/acme/repo/pull/1"
            }
        })),
    )
    .await;
    assert_error(&resp, "must be the only field set");
    assert_error(&resp, "url");

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_ne!(task.status, crate::models::TaskStatus::Done);
}

#[tokio::test]
async fn update_task_done_rejects_dispatched_agent_closing_its_own_task() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;

    let resp = call_as(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "done" }
        })),
        CallerIdentity::Task(task_id),
    )
    .await;
    assert_error(&resp, "Cannot mark your own task done");

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_ne!(task.status, crate::models::TaskStatus::Done);
}

#[tokio::test]
async fn update_task_done_allows_dispatched_agent_closing_a_different_task() {
    let state = test_state().await;
    let caller_task = create_task_fixture(&state).await;
    let other_task = create_task_fixture(&state).await;

    let resp = call_as(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": other_task.0, "status": "done" }
        })),
        CallerIdentity::Task(caller_task),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "expected success, got: {:?}",
        resp.error
    );

    let task = state.db.get_task(other_task).await.unwrap().unwrap();
    assert_eq!(task.status, crate::models::TaskStatus::Done);
}

#[tokio::test]
async fn update_task_done_unknown_task_errors() {
    let state = test_state().await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": 9999, "status": "done" }
        })),
    )
    .await;
    assert_error(&resp, "Task 9999 not found");
}

#[tokio::test]
async fn update_task_done_reachable_from_backlog_with_no_window_or_epic() {
    let state = test_state().await;
    // create_task_fixture leaves the task in Backlog with no worktree/epic —
    // the dedicated close path has no precondition on either.
    let task_id = create_task_fixture(&state).await;
    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(task.status, crate::models::TaskStatus::Backlog);
    assert!(task.tmux_window.is_none());
    assert!(task.epic_id.is_none());

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_task",
            "arguments": { "task_id": task_id.0, "status": "done" }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "expected success, got: {:?}",
        resp.error
    );
    assert_eq!(
        state.db.get_task(task_id).await.unwrap().unwrap().status,
        crate::models::TaskStatus::Done
    );
}

#[tokio::test]
async fn update_task_still_allows_other_statuses() {
    let state = test_state().await;
    let task_id = create_task_fixture(&state).await;

    for status in &["running", "review", "ready", "backlog"] {
        let resp = call(
            &state,
            "tools/call",
            Some(json!({
                "name": "update_task",
                "arguments": { "task_id": task_id.0, "status": status }
            })),
        )
        .await;
        assert!(
            resp.error.is_none(),
            "status={status} should be allowed, got: {:?}",
            resp.error
        );
    }
}

/// The done-status rejection above is one-directional: it blocks moving a task
/// INTO done, but not out of it. So every destination this tool accepts is
/// reachable from a done task, and none of them may be refused by the gate.
/// What leaving done does to the completion fields is the other half: nothing
/// at all. `completed_at` records the LAST completion and survives the move
/// (tasks.allium, ConfirmDone), and `sort_order` is manual ordering no status
/// write touches — so a task's hand-set position survives a round trip.
/// Spec: `UpdateTaskViaMcp` in docs/specs/mcp-task-tools.allium.
#[tokio::test]
async fn update_task_done_rejection_does_not_block_leaving_done() {
    let state = test_state().await;
    let finished = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();

    for status in &["backlog", "running", "review"] {
        let task_id = create_task_fixture(&state).await;

        state
            .db_write()
            .patch_task(
                task_id,
                &db::TaskPatch::new()
                    .status(TaskStatus::Done)
                    .completed_at(Some(finished))
                    .sort_order(Some(7)),
            )
            .await
            .unwrap();

        let resp = call(
            &state,
            "tools/call",
            Some(json!({
                "name": "update_task",
                "arguments": { "task_id": task_id.0, "status": status }
            })),
        )
        .await;
        assert!(!is_error(&resp), "status={status}: {:?}", resp);

        let task = state.db.get_task(task_id).await.unwrap().unwrap();
        assert_eq!(
            task.completed_at,
            Some(finished),
            "leaving done -> {status} must keep completed_at"
        );
        assert_eq!(
            task.sort_order,
            Some(7),
            "leaving done -> {status} must keep the manual sort_order"
        );
    }
}

#[tokio::test]
async fn update_task_missing_args() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({ "name": "update_task", "arguments": {} })),
    )
    .await;
    assert!(is_error(&resp));
}

#[tokio::test]
async fn get_task_found() {
    let state = test_state().await;
    let task_id = state
        .db_write()
        .create_task(CreateTaskRequest {
            title: "My Task",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: crate::models::TaskStatus::Backlog,
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
            "name": "get_task",
            "arguments": { "task_id": task_id.0 }
        })),
    )
    .await;
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("My Task"));
}

#[tokio::test]
async fn get_task_not_found() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "get_task",
            "arguments": { "task_id": 9999 }
        })),
    )
    .await;
    assert!(is_error(&resp));
    assert!(error_message(&resp).contains("not found"));
}

#[tokio::test]
async fn create_task_minimal() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "New Task",
                "repo_path": "/my/repo",
                "epic_id": null,
            }
        })),
    )
    .await;
    assert!(resp.error.is_none());
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("created"));

    // Verify task was created in DB
    let tasks = state.db.list_all().await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "New Task");
    assert_eq!(tasks[0].status, TaskStatus::Backlog);
    assert!(tasks[0].plan_path.is_none());
}

#[tokio::test]
async fn create_task_with_plan_stays_backlog() {
    let dir = tempfile::tempdir().unwrap();
    let plan_file = dir.path().join("plan.md");
    std::fs::write(&plan_file, "# Plan").unwrap();

    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "Planned Task",
                "repo_path": "/my/repo",
                "epic_id": null,
                "plan_path": plan_file.to_string_lossy(),
            }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    let tasks = state.db.list_all().await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].status, TaskStatus::Backlog);
    let stored = tasks[0].plan_path.as_deref().unwrap();
    assert!(
        std::path::Path::new(stored).is_absolute(),
        "plan path should be absolute, got: {stored}"
    );
    assert_eq!(
        stored,
        std::fs::canonicalize(&plan_file).unwrap().to_string_lossy()
    );
}

#[tokio::test]
async fn create_task_with_description() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": {
                "title": "Described Task",
                "repo_path": "/repo",
                "epic_id": null,
                "description": "Some details",
            }
        })),
    )
    .await;
    assert!(resp.error.is_none());

    let tasks = state.db.list_all().await.unwrap();
    assert_eq!(tasks[0].description, "Some details");
}

#[tokio::test]
async fn create_task_missing_title() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "repo_path": "/repo", "epic_id": null }
        })),
    )
    .await;
    assert!(is_error(&resp));
}

// An unknown argument name is a typo or a stale/removed field — without
// `deny_unknown_fields` it is silently dropped instead of surfacing as an error.
#[tokio::test]
async fn create_task_unknown_field_returns_error() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "create_task",
            "arguments": { "title": "t", "repo_path": "/repo", "epic_id": null, "bogus_field": "x" }
        })),
    )
    .await;
    assert_error(&resp, "bogus_field");
}

// -- String task_id coercion (Claude Code sends integers as strings) ------

#[tokio::test]
async fn update_task_accepts_string_task_id() {
    let state = test_state().await;
    let task_id = state
        .db_write()
        .create_task(CreateTaskRequest {
            title: "Test",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: crate::models::TaskStatus::Backlog,
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
            "arguments": { "task_id": task_id.0.to_string(), "status": "running" }
        })),
    )
    .await;
    assert!(
        resp.error.is_none(),
        "update_task should accept string task_id, got: {:?}",
        resp.error
    );

    let task = state.db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(task.status, crate::models::TaskStatus::Running);
}

pub(super) fn extract_created_task_id(resp: &JsonRpcResponse) -> crate::models::TaskId {
    let result = resp.result.as_ref().expect("expected ok response");
    let text = result["content"][0]["text"].as_str().expect("text field");
    // "Task <id> created"
    let id_str = text
        .strip_prefix("Task ")
        .and_then(|s| s.strip_suffix(" created"))
        .expect("expected 'Task <id> created'");
    crate::models::TaskId(id_str.parse().expect("numeric id"))
}

mod base_branch;
mod mock_service;
mod sub_status;
