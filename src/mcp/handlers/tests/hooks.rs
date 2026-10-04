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
