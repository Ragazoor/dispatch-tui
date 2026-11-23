//! What the task and epic tools read and decide about the orchestrator switch
//! and the ready mark (`orchestrators.allium: OrchestratorMcpTools`).

use crate::mcp::identity::CallerIdentity;
use crate::mcp::McpState;
use crate::models::{
    Epic, EpicId, EpicOrchestration, ReadyMark, ReadyMarker, StartRule, TaskId, TaskStatus,
};
use crate::service::ServiceError;

/// How far up an epic chain [`covering`] climbs before it gives up on a
/// chain that does not end.
const MAX_CHAIN: usize = 64;

/// `orchestrators.allium`'s `covering(epic)`: the switch on `epic` or on its
/// nearest ancestor that has one, with that epic.
pub(super) async fn covering(
    state: &McpState,
    epic: Option<EpicId>,
) -> Option<(Epic, EpicOrchestration)> {
    let mut next = epic;
    for _ in 0..MAX_CHAIN {
        let found = state.epic_svc.get_epic(next?).await.ok()?;
        if let Some(on) = state.db.epic_orchestration(found.id).await {
            return Some((found, on));
        }
        next = found.parent_epic_id;
    }
    None
}

/// Who a ready mark set by `identity` names (`UpdateTaskViaMcp`, "Who marked
/// it"): the operator for a session, an orchestrator by its own name, else
/// the agent of the caller's task.
pub(super) async fn ready_marker(
    state: &McpState,
    identity: &CallerIdentity,
) -> Result<(ReadyMarker, Option<String>), ServiceError> {
    match identity {
        CallerIdentity::Session => Ok((ReadyMarker::Operator, None)),
        CallerIdentity::Orchestrator(name) => {
            if state.db.orchestrator_named(name).await.is_some() {
                Ok((ReadyMarker::Agent, Some(name.clone())))
            } else {
                Err(ServiceError::Validation(format!(
                    "no orchestrator is named {name}; an orchestrator's ready mark names it"
                )))
            }
        }
        CallerIdentity::Task(task) => {
            let agents = state.db.list_agents().await;
            match crate::models::display_agent(agents.iter().filter(|a| a.task_id == *task)) {
                Some(agent) => Ok((ReadyMarker::Agent, Some(agent.name.clone()))),
                None => Err(ServiceError::Validation(format!(
                    "your task {} has no agent name to mark with; an agent's ready mark names the agent",
                    task.0
                ))),
            }
        }
    }
}

/// The refusal `ready: true` gets when the task will not be in backlog,
/// worded as the store words it.
pub(super) fn not_backlog_refusal() -> ServiceError {
    ServiceError::Validation("only a backlog task can be marked ready".into())
}

/// Mark `task` ready as `marker`, or clear its mark when `marker` is `None`.
pub(super) async fn write_ready(
    state: &McpState,
    task: TaskId,
    marker: Option<&(ReadyMarker, Option<String>)>,
) -> Result<(), ServiceError> {
    match marker {
        Some((by, name)) => {
            state
                .task_svc
                .mark_task_ready(task, *by, name.as_deref())
                .await
        }
        None => state.task_svc.clear_ready_mark(task).await,
    }
}

/// `GetTaskViaMcp`'s `Ready` and `Orchestrated by` lines, each with its
/// leading newline. Ready is said of a backlog task only.
pub(super) fn task_lines(
    status: TaskStatus,
    mark: Option<&ReadyMark>,
    covered_by: Option<&(Epic, EpicOrchestration)>,
) -> String {
    let mut text = String::new();
    if status == TaskStatus::Backlog {
        let line = match (mark, covered_by) {
            (Some(m), _) => match (&m.marked_by, m.agent_name.as_deref()) {
                (ReadyMarker::Agent, Some(name)) => format!("yes (marked by agent {name})"),
                (ReadyMarker::Agent, None) => "yes (marked by an agent)".to_string(),
                (ReadyMarker::Operator, _) => "yes (marked by the operator)".to_string(),
            },
            (None, Some((epic, on))) if on.start_rule == StartRule::Everything => {
                format!("no mark; epic {} starts every backlog task", epic.id.0)
            }
            (None, _) => "no".to_string(),
        };
        text.push_str(&format!("\nReady: {line}"));
    }
    if let Some((epic, _)) = covered_by {
        text.push_str(&format!(
            "\nOrchestrated by: epic {} ({})",
            epic.id.0, epic.title
        ));
    }
    text
}

/// `GetEpicViaMcp`'s Orchestrator line, with its leading newline, or nothing
/// when no orchestrator covers `epic`.
pub(super) async fn epic_line(state: &McpState, epic: &Epic) -> String {
    if let Some(on) = state.db.epic_orchestration(epic.id).await {
        let default = if on.max_parallel.is_none() {
            " (default)"
        } else {
            ""
        };
        return format!(
            "\nOrchestrator: on, start rule {}, limit {}{default}",
            on.start_rule,
            on.limit()
        );
    }
    match covering(state, epic.parent_epic_id).await {
        Some((above, _)) => format!(
            "\nOrchestrator: covered by epic {} ({})",
            above.id.0, above.title
        ),
        None => String::new(),
    }
}
