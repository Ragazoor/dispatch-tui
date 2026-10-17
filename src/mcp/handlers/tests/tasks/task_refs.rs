//! mcp-task-tools.allium: TaskRefResolvesToTaskId; agents.allium:
//! AgentNameResolvesToTask, AgentIdentity.

use super::*;

#[tokio::test]
async fn a_numeric_string_is_still_an_id() {
    let state = test_state().await;
    let id = state
        .db_write()
        .create_task(CreateTaskRequest::fixture("T", "/repo"))
        .await
        .unwrap();
    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "get_task", "arguments": {"task_id": id.0.to_string()}})),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);
}

#[tokio::test]
async fn an_unknown_word_is_not_found_like_an_unknown_id() {
    let state = test_state().await;
    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "get_task", "arguments": {"task_id": "nope"}})),
    )
    .await;
    // A tool failure, like an unknown id: an `isError` result, no protocol code.
    assert!(resp.error.is_none(), "{:?}", resp.error);
    assert_eq!(resp.result.as_ref().unwrap()["isError"], json!(true));
}

#[tokio::test]
async fn omitted_or_null_arguments_mean_no_arguments() {
    let state = test_state().await;
    for params in [
        json!({"name": "list_tasks"}),
        json!({"name": "list_tasks", "arguments": null}),
    ] {
        let resp = call(&state, "tools/call", Some(params.clone())).await;
        assert!(resp.error.is_none(), "{params}: {:?}", resp.error);
        assert!(
            resp.result.as_ref().unwrap()["isError"].as_bool() != Some(true),
            "{params}: {}",
            extract_response_text(&resp)
        );
    }
}

#[tokio::test]
async fn limit_accepts_a_numeric_string_and_rejects_zero() {
    let state = test_state().await;
    let ok = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_tasks", "arguments": {"limit": "5"}})),
    )
    .await;
    assert_ne!(ok.result.as_ref().unwrap()["isError"].as_bool(), Some(true));
    let zero = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_tasks", "arguments": {"limit": 0}})),
    )
    .await;
    let rejected = zero.error.is_some() || zero.result.as_ref().unwrap()["isError"] == json!(true);
    assert!(rejected, "limit 0 must be refused");
}

/// A task with a current agent, by way of a dispatch claim. Returns the id and
/// the agent's name.
async fn task_with_agent(state: &Arc<McpState>) -> (crate::models::TaskId, String) {
    let id = state
        .db_write()
        .create_task(CreateTaskRequest::fixture("T", "/repo"))
        .await
        .unwrap();
    state
        .task_svc
        .request_dispatch(id, crate::models::DispatchOrigin::Mcp, None)
        .await
        .unwrap();
    let name = state.db.current_agent(id).await.unwrap().name;
    (id, name)
}

#[tokio::test]
async fn get_task_accepts_an_agent_name_in_place_of_the_id() {
    let state = test_state().await;
    let (id, name) = task_with_agent(&state).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "get_task", "arguments": {"task_id": name}})),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);
    let text = extract_response_text(&resp);
    assert!(text.starts_with(&format!("Task {}:", id.0)), "{text}");
}

#[tokio::test]
async fn update_tasks_accepts_agent_names_in_task_ids() {
    let state = test_state().await;
    let (id, name) = task_with_agent(&state).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({
            "name": "update_tasks",
            "arguments": {"task_ids": [name], "title": "by agent name"}
        })),
    )
    .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);
    assert_eq!(state.db.get_task(id).await.unwrap().title, "by agent name");
}

#[tokio::test]
async fn get_task_shows_the_current_agent_and_that_it_is_not_live() {
    let state = test_state().await;
    let (_, name) = task_with_agent(&state).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "get_task", "arguments": {"task_id": name}})),
    )
    .await;
    let text = extract_response_text(&resp);
    // The claim creates the agent before the host has made a window for it.
    assert!(text.contains(&format!("Agent: {name} (paused)")), "{text}");
}

#[tokio::test]
async fn list_tasks_shows_the_current_agent_beside_the_title() {
    let state = test_state().await;
    let (id, name) = task_with_agent(&state).await;

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "list_tasks", "arguments": {}})),
    )
    .await;
    let text = extract_response_text(&resp);
    let line = text
        .lines()
        .find(|l| l.starts_with(&format!("- [{}]", id.0)))
        .unwrap_or_else(|| panic!("no row for the task in {text}"));
    assert!(line.contains(&format!(" agent:{name} (paused)")), "{line}");
}

#[tokio::test]
async fn a_task_with_no_agent_shows_none() {
    let state = test_state().await;
    let id = state
        .db_write()
        .create_task(CreateTaskRequest::fixture("T", "/repo"))
        .await
        .unwrap();

    let resp = call(
        &state,
        "tools/call",
        Some(json!({"name": "get_task", "arguments": {"task_id": id.0}})),
    )
    .await;
    assert!(!extract_response_text(&resp).contains("Agent:"));
}

/// agent-tree.allium: AgentRowShowsAgentName. The pane view carries the current
/// agent's name. A task with no current agent is no live agent at all: since
/// agents A4 the window is the agent's, so there is nothing to list.
#[tokio::test]
async fn the_pane_view_carries_the_current_agents_name() {
    use axum::extract::State;
    use axum::Json;

    let state = test_state().await;
    let (named, name) = task_with_agent(&state).await;
    let nameless = state
        .db_write()
        .create_task(CreateTaskRequest::fixture("N", "/repo"))
        .await
        .unwrap();
    let window = crate::models::test_tmux_window(&format!("task-{}", named.0));
    state
        .db_write()
        .patch_task(
            named,
            &crate::store::TaskPatch::new()
                .status(crate::models::TaskStatus::Running)
                .tmux_window(Some(&window)),
        )
        .await
        .unwrap();
    // Without an agent a window cannot be recorded at all.
    let refused = state
        .db_write()
        .patch_task(
            nameless,
            &crate::store::TaskPatch::new()
                .status(crate::models::TaskStatus::Running)
                .tmux_window(Some(&window)),
        )
        .await;
    assert!(refused.is_err(), "a window needs an agent to hold it");

    let Json(view) = crate::mcp::handlers::handle_pane_view(
        State(state.clone()),
        Json(crate::hooks::wire::PaneViewRequest { task_id: named }),
    )
    .await
    .unwrap();

    let by_id = |id| view.live_agents.iter().find(|a| a.id == id).unwrap();
    assert_eq!(by_id(named).agent_name, name);
    assert!(view.live_agents.iter().all(|a| a.id != nameless));
}
