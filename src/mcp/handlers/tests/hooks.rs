use super::*;

use crate::hooks::wire::{HookRequest, ObservedEvent};
use crate::mcp::handlers::hooks::handle_hook;
use crate::mcp::McpEvent;
use crate::models::HookEventKind;

/// task #4967: a hook event that the board applies successfully must push a
/// per-task refresh, so the card's label updates without waiting on the
/// tick-driven poll (agent-health.allium: HookEventsPushALiveRefresh).
async fn assert_pushes_task_changed(kind: HookEventKind) {
    let (notify_tx, mut notify_rx) = mpsc::unbounded_channel::<McpEvent>();
    let (state, _db) = test_state_with_overrides(
        Arc::new(MockProcessRunner::new(vec![])),
        Some(notify_tx),
        None,
    )
    .await;
    let task_id = create_task_fixture(&state).await;

    let request = HookRequest::Observe(ObservedEvent::Event {
        task_id: task_id.0,
        kind,
    });
    let _ = handle_hook(State(state), Json(request)).await;

    match notify_rx.recv().await {
        Some(McpEvent::TaskChanged(id)) => assert_eq!(id, task_id),
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
    let (notify_tx, mut notify_rx) = mpsc::unbounded_channel::<McpEvent>();
    let (state, _db) = test_state_with_overrides(
        Arc::new(MockProcessRunner::new(vec![])),
        Some(notify_tx),
        None,
    )
    .await;

    let request = HookRequest::Observe(ObservedEvent::Event {
        task_id: 999_999,
        kind: HookEventKind::PreToolUse,
    });
    let _ = handle_hook(State(state), Json(request)).await;

    notify_rx.close();
    assert!(
        notify_rx.recv().await.is_none(),
        "a hook for a missing task must not push a refresh"
    );
}

/// A pane keypress delivered over the hook endpoint is recorded as a
/// keybinding usage event under the row's action and the key, and does not
/// push a task refresh (`PanesRecordUsageLikeTheBoard`).
#[tokio::test]
async fn a_pane_keypress_is_recorded_as_keybinding_usage() {
    let (notify_tx, mut notify_rx) = mpsc::unbounded_channel::<McpEvent>();
    let (state, db) = test_state_with_overrides(
        Arc::new(MockProcessRunner::new(vec![])),
        Some(notify_tx),
        None,
    )
    .await;
    let request = HookRequest::Observe(ObservedEvent::PaneKey {
        task_id: 1,
        action: "navigate_half_page".to_string(),
        key: "d".to_string(),
    });
    let Json(answer) = handle_hook(State(state), Json(request)).await;
    assert_eq!(
        answer,
        crate::hooks::wire::HookResponse::Observed(crate::hooks::wire::ObserveOutcome::Applied)
    );
    let rows = db
        .query_usage(&crate::db::UsageQuery::default())
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].category, "keybinding");
    assert_eq!(rows[0].action, "navigate_half_page");
    assert!(notify_rx.try_recv().is_err(), "no refresh for usage");
}
