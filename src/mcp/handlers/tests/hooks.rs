use super::*;

use crate::board_event::BoardEvent;
use crate::hooks::wire::{HookRequest, ObservedEvent};
use crate::mcp::handlers::hooks::handle_hook;
use crate::models::{HookEventKind, TaskId};

/// task #4967: a hook event that the board applies successfully must push a
/// per-task refresh, so the card's label updates without waiting on the
/// tick-driven poll (agent-health.allium: HookEventsPushALiveRefresh).
async fn assert_pushes_task_changed(kind: HookEventKind) {
    let (notify_tx, mut notify_rx) = mpsc::unbounded_channel::<BoardEvent>();
    let (state, _db) = test_state_with_overrides(
        Arc::new(MockProcessRunner::new(vec![])),
        Some(notify_tx),
        None,
    )
    .await;
    let task_id = create_task_fixture(&state).await;

    let request = HookRequest::Observe(ObservedEvent::Event {
        task_id,
        kind,
        background: crate::models::BackgroundWork::default(),
    });
    let _ = handle_hook(State(state), Json(request)).await;

    match notify_rx.recv().await {
        Some(BoardEvent::TaskChanged(id)) => assert_eq!(id, task_id),
        other => panic!("expected TaskChanged({task_id:?}), got {other:?}"),
    }
}

#[tokio::test]
async fn pre_tool_use_pushes_task_changed() {
    assert_pushes_task_changed(HookEventKind::PreToolUse).await;
}

/// Same guarantee for the Notification event, which is what drives the
/// needs_input label specifically.
#[tokio::test]
async fn notification_pushes_task_changed() {
    assert_pushes_task_changed(HookEventKind::Notification(None)).await;
}

/// A hook whose task no longer exists succeeds as a no-op
/// (agent-health.allium: MissingTaskSucceeds) and must not push a refresh for
/// a row that isn't there.
#[tokio::test]
async fn missing_task_does_not_push_task_changed() {
    let (notify_tx, mut notify_rx) = mpsc::unbounded_channel::<BoardEvent>();
    let (state, _db) = test_state_with_overrides(
        Arc::new(MockProcessRunner::new(vec![])),
        Some(notify_tx),
        None,
    )
    .await;

    let request = HookRequest::Observe(ObservedEvent::Event {
        task_id: TaskId(999_999),
        kind: HookEventKind::PreToolUse,
        background: crate::models::BackgroundWork::default(),
    });
    let _ = handle_hook(State(state), Json(request)).await;

    notify_rx.close();
    assert!(
        notify_rx.recv().await.is_none(),
        "a hook for a missing task must not push a refresh"
    );
}

/// A PreToolUse hook for a task the store holds but this board cannot decode
/// still stamps the activity (the store checks `running` itself) and does not
/// fail the agent's hook (observability.allium:
/// `ReadByIdReportsAnUndecodableTask`).
#[tokio::test]
async fn a_pre_tool_use_for_an_undecodable_task_is_still_applied() {
    use crate::hooks::wire::{HookResponse, ObserveOutcome};
    let db = Store::open_in_memory().unwrap();
    db.plant_undecodable_task(42);
    let (state, _db) = state_over(
        Arc::new(db),
        Arc::new(MockProcessRunner::new(vec![])),
        None,
        None,
        None,
    );

    // PreToolUse is the event that reads the row; dropping it would freeze
    // the agent's activity on every board.
    let request = HookRequest::Observe(ObservedEvent::Event {
        task_id: TaskId(42),
        kind: HookEventKind::PreToolUse,
        background: crate::models::BackgroundWork::default(),
    });
    let Json(resp) = handle_hook(State(state), Json(request)).await;
    assert!(
        matches!(resp, HookResponse::Observed(ObserveOutcome::Applied)),
        "{resp:?}"
    );
}

/// The undecodable-task warning is logged once per task id, not per tool call.
#[test]
fn the_undecodable_task_warning_is_given_once_per_task() {
    use crate::mcp::handlers::hooks::first_undecodable_warning;
    assert!(first_undecodable_warning(TaskId(424_242)));
    assert!(!first_undecodable_warning(TaskId(424_242)));
    assert!(first_undecodable_warning(TaskId(424_243)));
}

/// A pane keypress delivered over the hook endpoint is recorded as a
/// keybinding usage event under the row's action and the key, and does not
/// push a task refresh (`PanesRecordUsageLikeTheBoard`).
#[tokio::test]
async fn a_pane_keypress_is_recorded_as_keybinding_usage() {
    let (notify_tx, mut notify_rx) = mpsc::unbounded_channel::<BoardEvent>();
    let (state, db) = test_state_with_overrides(
        Arc::new(MockProcessRunner::new(vec![])),
        Some(notify_tx),
        None,
    )
    .await;
    let request = HookRequest::Observe(ObservedEvent::PaneKey {
        task_id: TaskId(1),
        action: "navigate_half_page_down".to_string(),
        key: "d".to_string(),
    });
    let Json(answer) = handle_hook(State(state), Json(request)).await;
    assert_eq!(
        answer,
        crate::hooks::wire::HookResponse::Observed(crate::hooks::wire::ObserveOutcome::Applied)
    );
    let rows = db
        .query_usage(&crate::store::UsageQuery::default())
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].category, "keybinding");
    assert_eq!(rows[0].action, "navigate_half_page_down");
    assert!(notify_rx.try_recv().is_err(), "no refresh for usage");
}

/// A body the board cannot decode (an event a newer hook binary added) is a
/// JSON `Failed` outcome, not an HTTP 422 the agent would see.
#[tokio::test]
async fn undecodable_hook_body_answers_a_failed_outcome() {
    use crate::hooks::wire::{HookResponse, ObserveOutcome};
    let state = test_state().await;
    let body = axum::body::Bytes::from_static(
        br#"{"hook":"observe","event":"from_the_future","task_id":1}"#,
    );
    let resp = crate::mcp::handlers::handle_hook_body(State(state), json_headers(), body).await;
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let parsed: HookResponse = serde_json::from_slice(&bytes).unwrap();
    assert!(
        matches!(
            parsed,
            HookResponse::Observed(ObserveOutcome::Failed { .. })
        ),
        "{parsed:?}"
    );
}

fn json_headers() -> axum::http::HeaderMap {
    let mut h = axum::http::HeaderMap::new();
    h.insert(
        axum::http::header::CONTENT_TYPE,
        "application/json; charset=utf-8".parse().unwrap(),
    );
    h
}

/// A cross-origin `text/plain` POST needs no preflight, so the hook route
/// refuses anything not declared JSON — and never applies the event.
#[tokio::test]
async fn non_json_hook_post_is_refused() {
    let state = test_state().await;
    let body = axum::body::Bytes::from_static(
        br#"{"hook":"observe","event":"event","task_id":1,"kind":"pre_tool_use"}"#,
    );
    for headers in [axum::http::HeaderMap::new(), {
        let mut h = axum::http::HeaderMap::new();
        h.insert(
            axum::http::header::CONTENT_TYPE,
            "text/plain".parse().unwrap(),
        );
        h
    }] {
        let resp =
            crate::mcp::handlers::handle_hook_body(State(state.clone()), headers, body.clone())
                .await;
        assert_eq!(
            resp.status(),
            axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
    }
}

// -- An orchestrator's hooks (agent-health.allium: OrchestratorHooks) --------

async fn orchestrated_state() -> (Arc<McpState>, crate::models::Orchestrator) {
    let (state, _db) =
        test_state_with_overrides(Arc::new(MockProcessRunner::new(vec![])), None, None).await;
    let epic = state.db_write().create_epic("E", "", None).await.unwrap();
    state
        .db_write()
        .switch_orchestrator_on(epic.id, None, None)
        .await
        .unwrap();
    let host = state.db.host_id().to_string();
    state.db.ensure_orchestrator(epic.id, &host).await.unwrap();
    let row = state.db.list_orchestrators().await.remove(0);
    (state, row)
}

/// OrchestratorTurnClearsTheRestartFlag: the Stop of an orchestrator's
/// session is its completed turn.
#[tokio::test]
async fn an_orchestrators_stop_records_its_turn() {
    use crate::hooks::wire::{HookResponse, ObserveOutcome, OrchestratorEvent};
    let (state, row) = orchestrated_state().await;

    let request = HookRequest::ObserveOrchestrator(OrchestratorEvent::Stop {
        name: row.name.clone(),
    });
    let Json(answer) = handle_hook(State(state.clone()), Json(request)).await;

    assert_eq!(answer, HookResponse::Observed(ObserveOutcome::Applied));
    let after = state.db.list_orchestrators().await.remove(0);
    assert!(after.last_turn_at.is_some());
}

/// DeliveredAsATaskHookIs: a name no orchestrator holds succeeds quietly.
#[tokio::test]
async fn a_stop_for_an_unknown_orchestrator_succeeds() {
    use crate::hooks::wire::{HookResponse, ObserveOutcome, OrchestratorEvent};
    let (state, _row) = orchestrated_state().await;

    let request = HookRequest::ObserveOrchestrator(OrchestratorEvent::Stop {
        name: "nobody".into(),
    });
    let Json(answer) = handle_hook(State(state), Json(request)).await;

    assert_eq!(answer, HookResponse::Observed(ObserveOutcome::Applied));
}
