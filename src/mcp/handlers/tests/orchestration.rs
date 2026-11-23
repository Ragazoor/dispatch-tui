//! The orchestrator switch and the ready mark over MCP
//! (`mcp-task-tools.allium`: UpdateTaskViaMcp, CreateTaskViaMcp,
//! GetTaskViaMcp, ListTasksViaMcp `ready`; `epics.allium`: UpdateEpicViaMcp,
//! GetEpicViaMcp, the orchestrator arguments).

use super::*;
use crate::models::{EpicId, ReadyMarker, StartRule, TaskId};

async fn tool(state: &Arc<McpState>, name: &str, arguments: Value) -> JsonRpcResponse {
    call(
        state,
        "tools/call",
        Some(json!({ "name": name, "arguments": arguments })),
    )
    .await
}

async fn tool_as(
    state: &Arc<McpState>,
    name: &str,
    arguments: Value,
    identity: CallerIdentity,
) -> JsonRpcResponse {
    call_as(
        state,
        "tools/call",
        Some(json!({ "name": name, "arguments": arguments })),
        identity,
    )
    .await
}

fn ok_text(resp: &JsonRpcResponse) -> String {
    assert!(!is_error(resp), "{}", error_message(resp));
    extract_response_text(resp)
}

async fn epic(state: &Arc<McpState>, title: &str, parent: Option<EpicId>) -> EpicId {
    state
        .db_write()
        .create_epic(title, "", parent)
        .await
        .unwrap()
        .id
}

async fn task_in(state: &Arc<McpState>, epic: EpicId, extra: Value) -> TaskId {
    let mut arguments = json!({ "title": "T", "repo_path": "/repo", "epic_id": epic.0 });
    arguments
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    ok_text(&tool(state, "task_create", arguments).await);
    state
        .db
        .list_all()
        .await
        .into_iter()
        .map(|t| t.id)
        .max()
        .unwrap()
}

async fn detail(state: &Arc<McpState>, task: TaskId) -> String {
    ok_text(&tool(state, "task_search", json!({ "task_id": task.0 })).await)
}

async fn epic_detail(state: &Arc<McpState>, epic: EpicId) -> String {
    ok_text(&tool(state, "epic_search", json!({ "epic_id": epic.0 })).await)
}

async fn agent_name(state: &Arc<McpState>, task: TaskId) -> String {
    let agents = state.db.list_agents().await;
    crate::models::display_agent(agents.iter().filter(|a| a.task_id == task))
        .unwrap()
        .name
        .clone()
}

// -- the ready argument --------------------------------------------------------

#[test]
fn create_and_update_advertise_a_boolean_ready() {
    for name in ["task_create", "task_update"] {
        let def = tool_def(name);
        assert_eq!(
            def["inputSchema"]["properties"]["ready"]["type"], "boolean",
            "{name}"
        );
    }
}

#[tokio::test]
async fn a_new_task_is_not_ready_unless_asked() {
    let state = test_state().await;
    let e = epic(&state, "E", None).await;
    let task = task_in(&state, e, json!({})).await;

    assert!(state.db.ready_mark(task).await.is_none());
    assert!(detail(&state, task).await.contains("\nReady: no"));
}

#[tokio::test]
async fn a_session_marks_a_new_task_ready_as_the_operator() {
    let state = test_state().await;
    let e = epic(&state, "E", None).await;
    let task = task_in(&state, e, json!({ "ready": true })).await;

    let mark = state.db.ready_mark(task).await.expect("marked");
    assert_eq!(mark.marked_by, ReadyMarker::Operator);
    assert_eq!(mark.agent_name, None);
    assert!(detail(&state, task)
        .await
        .contains("\nReady: yes (marked by the operator)"));
    let rows = ok_text(&tool(&state, "task_search", json!({})).await);
    assert!(rows.contains("T ready"), "{rows}");
}

#[tokio::test]
async fn an_agent_marks_a_task_ready_under_its_own_name_and_clears_it() {
    let state = test_state().await;
    let e = epic(&state, "E", None).await;
    let caller = task_in(&state, e, json!({})).await;
    let task = task_in(&state, e, json!({ "title": "other" })).await;
    let name = agent_name(&state, caller).await;

    let set = tool_as(
        &state,
        "task_update",
        json!({ "task_ids": [task.0], "ready": true }),
        CallerIdentity::Task(caller),
    )
    .await;
    assert!(ok_text(&set).contains("ready"), "{}", ok_text(&set));
    let mark = state.db.ready_mark(task).await.expect("marked");
    assert_eq!(mark.marked_by, ReadyMarker::Agent);
    assert_eq!(mark.agent_name.as_deref(), Some(name.as_str()));
    assert!(detail(&state, task)
        .await
        .contains(&format!("\nReady: yes (marked by agent {name})")));

    ok_text(
        &tool(
            &state,
            "task_update",
            json!({ "task_ids": [task.0], "ready": false }),
        )
        .await,
    );
    assert!(state.db.ready_mark(task).await.is_none());
}

#[tokio::test]
async fn a_ready_only_call_leaves_updated_at_alone() {
    let state = test_state().await;
    let e = epic(&state, "E", None).await;
    let task = task_in(&state, e, json!({})).await;
    let before = state.db.get_task(task).await.unwrap().updated_at;

    ok_text(
        &tool(
            &state,
            "task_update",
            json!({ "task_ids": [task.0], "ready": true }),
        )
        .await,
    );

    assert_eq!(state.db.get_task(task).await.unwrap().updated_at, before);
}

#[tokio::test]
async fn ready_on_a_task_leaving_backlog_is_refused_before_any_write() {
    let state = test_state().await;
    let e = epic(&state, "E", None).await;
    let task = task_in(&state, e, json!({})).await;

    let resp = tool(
        &state,
        "task_update",
        json!({ "task_ids": [task.0], "status": "review", "ready": true }),
    )
    .await;
    assert!(is_error(&resp));
    assert!(
        error_message(&resp).contains("backlog"),
        "{}",
        error_message(&resp)
    );
    assert_eq!(
        state.db.get_task(task).await.unwrap().status,
        crate::models::TaskStatus::Backlog
    );
    assert!(state.db.ready_mark(task).await.is_none());
}

#[tokio::test]
async fn ready_on_a_task_outside_backlog_is_refused() {
    let state = test_state().await;
    let e = epic(&state, "E", None).await;
    let task = task_in(&state, e, json!({})).await;
    ok_text(
        &tool(
            &state,
            "task_update",
            json!({ "task_ids": [task.0], "status": "review" }),
        )
        .await,
    );

    let resp = tool(
        &state,
        "task_update",
        json!({ "task_ids": [task.0], "ready": true }),
    )
    .await;
    assert!(is_error(&resp));
    assert!(state.db.ready_mark(task).await.is_none());
    assert!(!detail(&state, task).await.contains("Ready:"));
}

#[tokio::test]
async fn moving_back_to_backlog_and_marking_ready_is_one_call() {
    let state = test_state().await;
    let e = epic(&state, "E", None).await;
    let task = task_in(&state, e, json!({})).await;
    ok_text(
        &tool(
            &state,
            "task_update",
            json!({ "task_ids": [task.0], "status": "review" }),
        )
        .await,
    );

    ok_text(
        &tool(
            &state,
            "task_update",
            json!({ "task_ids": [task.0], "status": "backlog", "ready": true }),
        )
        .await,
    );
    assert!(state.db.ready_mark(task).await.is_some());
}

#[tokio::test]
async fn ready_cannot_travel_with_status_done() {
    let state = test_state().await;
    let e = epic(&state, "E", None).await;
    let task = task_in(&state, e, json!({})).await;

    let resp = tool(
        &state,
        "task_update",
        json!({ "task_ids": [task.0], "status": "done", "ready": false }),
    )
    .await;
    assert!(is_error(&resp));
    assert!(
        error_message(&resp).contains("ready"),
        "{}",
        error_message(&resp)
    );
}

// -- the orchestrator arguments ------------------------------------------------

#[test]
fn epic_update_advertises_the_orchestrator_arguments() {
    let props = &tool_def("epic_update")["inputSchema"]["properties"];
    assert_eq!(props["orchestrator"]["type"], "boolean");
    assert_eq!(
        props["start_rule"]["enum"],
        json!(["marked_only", "everything"])
    );
    assert_eq!(props["max_parallel"]["type"], "integer");
}

#[tokio::test]
async fn switching_on_takes_the_defaults_and_the_read_shows_them() {
    let state = test_state().await;
    let e = epic(&state, "E", None).await;

    let reply = ok_text(
        &tool(
            &state,
            "epic_update",
            json!({ "epic_id": e.0, "orchestrator": true }),
        )
        .await,
    );
    assert!(reply.contains("orchestrator"), "{reply}");

    let on = state.db.epic_orchestration(e).await.expect("on");
    assert_eq!(on.start_rule, StartRule::MarkedOnly);
    assert_eq!(on.max_parallel, None);
    assert!(epic_detail(&state, e)
        .await
        .contains("\nOrchestrator: on, start rule marked_only, limit 3 (default)"));
}

#[tokio::test]
async fn switching_on_with_settings_and_changing_them() {
    let state = test_state().await;
    let e = epic(&state, "E", None).await;
    ok_text(
        &tool(
            &state,
            "epic_update",
            json!({ "epic_id": e.0, "orchestrator": true, "start_rule": "everything", "max_parallel": 2 }),
        )
        .await,
    );
    assert!(epic_detail(&state, e)
        .await
        .contains("\nOrchestrator: on, start rule everything, limit 2"));

    ok_text(
        &tool(
            &state,
            "epic_update",
            json!({ "epic_id": e.0, "max_parallel": 5 }),
        )
        .await,
    );
    let on = state.db.epic_orchestration(e).await.unwrap();
    assert_eq!(
        (on.start_rule, on.max_parallel),
        (StartRule::Everything, Some(5))
    );
}

#[tokio::test]
async fn settings_on_an_epic_that_is_off_are_refused() {
    let state = test_state().await;
    let e = epic(&state, "E", None).await;
    let resp = tool(
        &state,
        "epic_update",
        json!({ "epic_id": e.0, "start_rule": "everything" }),
    )
    .await;
    assert!(is_error(&resp));
    assert!(
        error_message(&resp).contains("has no orchestrator"),
        "{}",
        error_message(&resp)
    );
}

#[tokio::test]
async fn switching_on_twice_is_refused_with_the_stores_wording() {
    let state = test_state().await;
    let e = epic(&state, "E", None).await;
    ok_text(
        &tool(
            &state,
            "epic_update",
            json!({ "epic_id": e.0, "orchestrator": true }),
        )
        .await,
    );
    let resp = tool(
        &state,
        "epic_update",
        json!({ "epic_id": e.0, "orchestrator": true }),
    )
    .await;
    assert!(
        error_message(&resp).contains("already has an orchestrator"),
        "{}",
        error_message(&resp)
    );
}

#[tokio::test]
async fn a_limit_below_one_is_refused() {
    let state = test_state().await;
    let e = epic(&state, "E", None).await;
    let resp = tool(
        &state,
        "epic_update",
        json!({ "epic_id": e.0, "orchestrator": true, "max_parallel": 0 }),
    )
    .await;
    assert!(is_error(&resp));
    assert!(state.db.epic_orchestration(e).await.is_none());
}

#[tokio::test]
async fn switching_off_removes_the_switch_and_takes_no_settings() {
    let state = test_state().await;
    let e = epic(&state, "E", None).await;
    ok_text(
        &tool(
            &state,
            "epic_update",
            json!({ "epic_id": e.0, "orchestrator": true }),
        )
        .await,
    );

    let mixed = tool(
        &state,
        "epic_update",
        json!({ "epic_id": e.0, "orchestrator": false, "max_parallel": 2 }),
    )
    .await;
    assert!(is_error(&mixed));
    assert!(state.db.epic_orchestration(e).await.is_some());

    ok_text(
        &tool(
            &state,
            "epic_update",
            json!({ "epic_id": e.0, "orchestrator": false }),
        )
        .await,
    );
    assert!(state.db.epic_orchestration(e).await.is_none());
    assert!(!epic_detail(&state, e).await.contains("Orchestrator:"));
}

#[tokio::test]
async fn the_orchestrator_arguments_go_in_a_call_of_their_own() {
    let state = test_state().await;
    let e = epic(&state, "E", None).await;
    let resp = tool(
        &state,
        "epic_update",
        json!({ "epic_id": e.0, "orchestrator": true, "title": "renamed" }),
    )
    .await;
    assert!(is_error(&resp));
    assert!(
        error_message(&resp).contains("the orchestrator arguments go in a call of their own"),
        "{}",
        error_message(&resp)
    );
    assert!(state.db.epic_orchestration(e).await.is_none());
    assert_eq!(state.db.get_epic(e).await.unwrap().title, "E");
}

#[tokio::test]
async fn a_covered_epic_and_task_name_the_epic_that_orchestrates_them() {
    let state = test_state().await;
    let root = epic(&state, "Root", None).await;
    let child = epic(&state, "Child", Some(root)).await;
    let task = task_in(&state, child, json!({})).await;
    ok_text(
        &tool(
            &state,
            "epic_update",
            json!({ "epic_id": root.0, "orchestrator": true, "start_rule": "everything" }),
        )
        .await,
    );

    assert!(epic_detail(&state, child).await.contains(&format!(
        "\nOrchestrator: covered by epic {} (Root)",
        root.0
    )));
    let text = detail(&state, task).await;
    assert!(
        text.contains(&format!("\nOrchestrated by: epic {} (Root)", root.0)),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "\nReady: no mark; epic {} starts every backlog task",
            root.0
        )),
        "{text}"
    );
}

#[tokio::test]
async fn a_move_under_an_orchestrated_epic_names_the_switch_it_turned_off() {
    let state = test_state().await;
    let above = epic(&state, "Above", None).await;
    let moved = epic(&state, "Moved", None).await;
    for e in [above, moved] {
        ok_text(
            &tool(
                &state,
                "epic_update",
                json!({ "epic_id": e.0, "orchestrator": true }),
            )
            .await,
        );
    }

    let reply = ok_text(
        &tool(
            &state,
            "epic_update",
            json!({ "epic_id": moved.0, "parent_epic_id": above.0 }),
        )
        .await,
    );
    assert!(
        reply.contains(&format!(
            "Switched off the orchestrator of epic {} (Moved): epic {} above it has one.",
            moved.0, above.0
        )),
        "{reply}"
    );
    assert!(state.db.epic_orchestration(moved).await.is_none());
}

// -- refusals the handler owns -------------------------------------------------

#[tokio::test]
async fn an_agent_with_no_name_cannot_mark_and_nothing_is_written() {
    let state = test_state().await;
    let e = epic(&state, "E", None).await;
    let task = task_in(&state, e, json!({})).await;

    let resp = tool_as(
        &state,
        "task_update",
        json!({ "task_ids": [task.0], "title": "renamed", "ready": true }),
        CallerIdentity::Task(TaskId(9999)),
    )
    .await;
    assert!(is_error(&resp));
    assert!(
        error_message(&resp).contains("no agent name"),
        "{}",
        error_message(&resp)
    );
    assert!(state.db.ready_mark(task).await.is_none());
    assert_eq!(state.db.get_task(task).await.unwrap().title, "T");
}

/// The real task service, with `mark_task_ready` refused as the store would
/// refuse it after the fields have landed.
struct RefusesTheMark(Arc<dyn crate::service::TaskServiceApi>);

#[async_trait::async_trait]
impl crate::service::TaskServiceApiStub for RefusesTheMark {
    async fn update_task(
        &self,
        params: crate::service::UpdateTaskParams,
    ) -> Result<crate::service::UpdateTaskResult, crate::service::ServiceError> {
        self.0.update_task(params).await
    }
    async fn create_task(
        &self,
        params: crate::service::CreateTaskParams,
    ) -> Result<TaskId, crate::service::ServiceError> {
        self.0.create_task(params).await
    }
    async fn get_task(
        &self,
        id: TaskId,
    ) -> Result<crate::models::Task, crate::service::ServiceError> {
        self.0.get_task(id).await
    }
    async fn mark_task_ready(
        &self,
        _: TaskId,
        _: ReadyMarker,
        _: Option<&str>,
    ) -> Result<(), crate::service::ServiceError> {
        Err(crate::service::ServiceError::Validation(
            "only a backlog task can be marked ready".into(),
        ))
    }
}

crate::task_service_api!(service_api_stub_bridge, RefusesTheMark);

async fn state_refusing_the_mark() -> Arc<McpState> {
    let real = test_state().await;
    let (state, _) = state_over(
        real.db_write().clone(),
        Arc::new(crate::process::MockProcessRunner::new(vec![])),
        None,
        Some(Arc::new(RefusesTheMark(real.task_svc.clone()))),
        None,
    );
    state
}

#[tokio::test]
async fn a_refused_mark_after_the_fields_says_the_fields_landed() {
    let state = state_refusing_the_mark().await;
    let e = epic(&state, "E", None).await;
    let task = task_in(&state, e, json!({})).await;

    let resp = tool(
        &state,
        "task_update",
        json!({ "task_ids": [task.0], "title": "renamed", "ready": true }),
    )
    .await;
    assert!(is_error(&resp));
    let message = error_message(&resp);
    assert!(
        message.contains("the fields were written (title), but the ready mark was not"),
        "{message}"
    );
    assert_eq!(state.db.get_task(task).await.unwrap().title, "renamed");
}

#[tokio::test]
async fn a_refused_mark_after_creation_names_the_created_task() {
    let state = state_refusing_the_mark().await;
    let e = epic(&state, "E", None).await;

    let resp = tool(
        &state,
        "task_create",
        json!({ "title": "T", "repo_path": "/repo", "epic_id": e.0, "ready": true }),
    )
    .await;
    assert!(is_error(&resp));
    assert!(
        error_message(&resp).contains("was created, but not marked ready"),
        "{}",
        error_message(&resp)
    );
    assert_eq!(state.db.list_all().await.len(), 1);
}
