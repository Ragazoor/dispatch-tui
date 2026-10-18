//! The board side of `HookDelivery` (`docs/specs/agent-health.allium`).
//!
//! Every Claude Code hook on this machine posts here instead of opening the
//! task database itself. That is what makes the denormalised counters
//! (`live_subagents`) safe under concurrent hooks: the read and
//! its dependent write are serialised by this one process, rather than raced
//! for across as many processes as there are live agent sessions.

use std::sync::Arc;

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

use crate::hooks::wire::{
    Answer, HookRequest, HookResponse, ObserveOutcome, ObservedEvent, PaneAgent, PaneTask,
    PaneView, PaneViewRequest, Question,
};
use crate::mcp::{trajectory, BackgroundWrite, McpState};
use crate::models::TaskId;
use crate::service::ServiceError;

/// The reminder the gate blocks a task's first PR-creation attempt with.
///
/// It lives on the board rather than in the hook process because the board is
/// what decides: the hook binary is whichever `dispatch` is on `PATH`, which
/// need not be the build the board is running. Composed here, revised wording
/// takes effect with the board that serves it. See `PrLearningsGate` in
/// `docs/specs/pr-workflow.allium` for why the scope is the whole submission
/// and why it suggests no tag, and why it names `task_id` with the id.
fn pr_gate_reminder(task_id: TaskId) -> String {
    format!(
        "Before creating this PR, consult the knowledge base for the conventions that apply \
         to what you are submitting — the code in the diff as well as the PR title and body. \
         Call the dispatch `query_learnings` MCP tool with `task_id` {id} and a `query` \
         describing this change, then apply what it returns and re-run the command.",
        id = task_id.0
    )
}

/// The route handler: decodes the body itself so one the board cannot read
/// (a newer hook binary's event, say) still gets a JSON `Failed` outcome. The
/// extractor's own 422 would surface as an error in the agent's terminal.
///
/// A body that is not declared `application/json` is refused with 415 before
/// it is read: a cross-origin `text/plain` POST needs no preflight, so
/// without this a web page could forge `observe` events.
pub async fn handle_hook_body(
    State(state): State<Arc<McpState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !is_json_content_type(&headers) {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "hook requests must be application/json",
        )
            .into_response();
    }
    match serde_json::from_slice::<HookRequest>(&body) {
        Ok(request) => handle_hook(State(state), Json(request))
            .await
            .into_response(),
        Err(e) => {
            tracing::warn!("undecodable hook body: {e}");
            Json(HookResponse::Observed(ObserveOutcome::Failed {
                reason: format!("the board could not read the hook request: {e}"),
            }))
            .into_response()
        }
    }
}

/// Whether the `Content-Type` is `application/json` (parameters such as
/// `charset` ignored).
fn is_json_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|m| m.trim().eq_ignore_ascii_case("application/json"))
}

pub async fn handle_hook(
    State(state): State<Arc<McpState>>,
    Json(request): Json<HookRequest>,
) -> Json<HookResponse> {
    Json(match request {
        HookRequest::Observe(event) => HookResponse::Observed(observe(&state, event).await),
        HookRequest::Ask(question) => HookResponse::Answer(answer(&state, question).await),
    })
}

/// `BoardPaneView` (`docs/specs/agent-tree.allium`): the companion panes' read.
/// Answered from the board's own rows, changes nothing and notifies nobody. A
/// row the board does not hold yet is an answer with no task, not an error; an
/// unreadable row is one.
pub async fn handle_pane_view(
    State(state): State<Arc<McpState>>,
    Json(request): Json<PaneViewRequest>,
) -> Result<Json<PaneView>, (StatusCode, String)> {
    let task = state
        .db
        .get_task(request.task_id)
        .await
        .map(|task| PaneTask {
            worktree: task.worktree,
            base_branch: task.base_branch,
        });
    let (live_tasks, agents) =
        tokio::join!(state.db.list_live_agent_tasks(), state.db.list_agents());
    let live_agents = live_tasks
        .into_iter()
        .filter_map(|task| {
            Some(PaneAgent {
                id: task.id,
                title: task.title,
                tmux_window: task.tmux_window?.as_str().to_string(),
                agent_name: agents
                    .iter()
                    .find(|a| a.task_id == task.id && a.is_current())
                    .map(|a| a.name.clone())
                    .unwrap_or_default(),
            })
        })
        .collect();
    Ok(Json(PaneView { task, live_agents }))
}

async fn observe(state: &McpState, event: ObservedEvent) -> ObserveOutcome {
    // A pane keypress is usage, not a change to the task: it is recorded and
    // nothing is re-read or refreshed.
    if let ObservedEvent::PaneKey { action, key, .. } = &event {
        crate::service::record_usage_event_logged(
            state.db.as_ref(),
            &crate::models::UsageEvent::pane_key(action, key),
        )
        .await;
        return ObserveOutcome::Applied;
    }
    let task_id = event.task_id();
    match apply(state, task_id, event).await {
        Ok(()) => {
            // agent-health.allium: HookEventsPushALiveRefresh. Scoped to this
            // one task (a single row re-read), not a full-board refresh, so
            // the cost scales with this task's own hook volume rather than
            // with board size.
            state.notify_task_changed(task_id);
            ObserveOutcome::Applied
        }
        // A hook fires from a session whose task may since have been
        // deleted. That is not a failure the agent can act on, so it
        // travels back as an outcome and the hook exits cleanly.
        Err(ServiceError::NotFound(_)) => ObserveOutcome::TaskNotFound,
        Err(e) => {
            tracing::warn!(task_id = task_id.0, "hook event failed: {e}");
            ObserveOutcome::Failed {
                reason: e.to_string(),
            }
        }
    }
}

async fn apply(
    state: &McpState,
    task_id: TaskId,
    event: ObservedEvent,
) -> Result<(), ServiceError> {
    match event {
        ObservedEvent::Event { kind, .. } => state.task_svc.record_hook_event(task_id, kind).await,
        ObservedEvent::Subagent {
            agent_id,
            session_id,
            stop,
            ..
        } => {
            let event = ObservedEvent::subagent_event(agent_id, session_id, stop);
            state.task_svc.record_subagent_event(task_id, event).await
        }
        ObservedEvent::SubagentClear { .. } => {
            state.task_svc.clear_subagents_no_drain(task_id).await
        }
        ObservedEvent::PeerMessage { target, body, .. } => {
            state
                .task_svc
                .record_peer_message_sent(task_id, &target)
                .await?;
            append_peer_message_trajectory(state, task_id, &target, &body);
            Ok(())
        }
        // Answered in `observe`, before any task is touched.
        ObservedEvent::PaneKey { .. } => Ok(()),
    }
}

async fn answer(state: &McpState, question: Question) -> Answer {
    let task_id = question.task_id();
    match question {
        // The gate answers the agent's tool call rather than observing it, so
        // its verdict is the response rather than a side effect. It is on the
        // same endpoint as the observers for the reason they are: it is a
        // read-modify-write on one task row fired from a hook, and running it
        // in its own process is what made it a race.
        Question::PrGate { .. } => match state.task_svc.mark_pr_learnings_gate_shown(task_id).await
        {
            Ok(first_time) => Answer::PrGate {
                reminder: first_time.then(|| pr_gate_reminder(task_id)),
            },
            Err(e) => {
                tracing::warn!(task_id = task_id.0, "pr gate failed: {e}");
                Answer::Failed {
                    reason: e.to_string(),
                }
            }
        },
    }
}

/// The `SendMessage` trajectory entry, appended off the response path the way
/// every other trajectory write is. It used to be written by the hook process
/// into the data dir beside the database; the hook has no data dir any more,
/// so the board writes it into its own.
fn append_peer_message_trajectory(state: &McpState, task_id: TaskId, target: &str, body: &str) {
    let entry = trajectory::TrajectoryEntry {
        timestamp: chrono::Utc::now(),
        task_id: task_id.0,
        method: "SendMessage".to_string(),
        args: serde_json::json!({"target": target, "body": body}),
        result: serde_json::json!({"observed": true}),
        duration_ms: 0,
    };
    let data_dir = state.data_dir.clone();
    let done = state.test_hooks.bg_write_done_tx.clone();
    tokio::spawn(async move {
        trajectory::append_entry(&data_dir, &entry).await;
        if let Some(tx) = done {
            let _ = tx.send(BackgroundWrite::Trajectory);
        }
    });
}
