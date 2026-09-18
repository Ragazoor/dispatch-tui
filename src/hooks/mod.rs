//! The Claude Code hook side of `HookDelivery` (`docs/specs/agent-health.allium`).
//!
//! Every hook fires as its own short-lived process, many times a second
//! across every live agent session. Such a process does exactly two things
//! here: turn its arguments into a [`wire::HookRequest`], and hand that to the
//! running board. It opens no store of its own — that is the whole point of
//! this module, and the entry-point tests in `tests/hooks.rs` hold the line so
//! a new hook kind cannot quietly reopen one.

pub mod wire;

use std::time::Duration;

use anyhow::{Context, Result};
use http_body_util::BodyExt;
use hyper_util::rt::TokioIo;

use crate::models::{HookEventKind, NotificationKind};
use wire::{Answer, HookRequest, HookResponse, ObserveOutcome, ObservedEvent, Question, HOOK_PATH};

/// How long a hook waits on the board before giving up on it.
///
/// A hook runs inside the agent's own tool call, which does not proceed until
/// the hook returns, so an unbounded wait would let one stalled board hang
/// every session on the machine. Before hooks went over HTTP the equivalent
/// bound was SQLite's own busy timeout; this is what replaces it. The budget
/// covers the whole exchange — connect, request and response — because a
/// board that has accepted the connection but cannot answer is the case worth
/// bounding, and it is generous relative to the work (a loopback round trip
/// and one small row write) so that only a genuinely stuck board hits it.
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(10);

/// The port a board with no port of its own claims, and the one a hook
/// assumes when nothing told it otherwise.
///
/// Declared beside the client that consumes it so the CLI surface states the
/// board-address contract once rather than per subcommand.
#[derive(Debug, Clone, Copy, clap::Args)]
pub struct BoardAddress {
    /// Port the running board serves its hook endpoint on. Matches the port
    /// `dispatch tui` was started with, which the board records in the
    /// settings its sessions are launched with.
    #[arg(long, env = "DISPATCH_PORT", default_value_t = crate::DEFAULT_PORT)]
    pub port: u16,
}

/// Deliver an observed event and report the outcome the way a hook must: a
/// delivery the board answered succeeds, whatever it answered; a board that
/// is not there fails, and the event is gone.
pub async fn deliver(port: u16, event: ObservedEvent) -> Result<()> {
    let task_id = event.task_id();
    match send(port, &HookRequest::Observe(event)).await? {
        HookResponse::Observed(ObserveOutcome::Applied) => Ok(()),
        // A task that no longer exists still succeeds. The hook fires from a
        // session whose task may since have been archived or deleted, and a
        // non-zero exit there would surface in the agent's own terminal for
        // something it cannot act on. It is named rather than skipped in
        // silence: reaching this arm means the board looked the row up before
        // it could decide anything, which is the half of `MissingTaskSucceeds`
        // that reports. The events that decide inside their own write never
        // produce this answer, so they stay quiet without a branch here.
        HookResponse::Observed(ObserveOutcome::TaskNotFound) => {
            eprintln!("Task {task_id} not found, skipping");
            Ok(())
        }
        HookResponse::Observed(ObserveOutcome::Failed { reason }) => Err(anyhow::anyhow!(
            "the dispatch board rejected the hook: {reason}"
        )),
        other => Err(unexpected_answer(other)),
    }
}

fn unexpected_answer(response: HookResponse) -> anyhow::Error {
    anyhow::anyhow!("the dispatch board answered with {response:?}, which does not fit the request")
}

/// POST the request to the board and decode its answer.
///
/// Every error is "no board answered": unreachable, too slow, or answering
/// something this version cannot read. An answer the board *did* give travels
/// back intact for the caller to interpret, because the two kinds of hook act
/// on those answers differently.
async fn send(port: u16, request: &HookRequest) -> Result<HookResponse> {
    tokio::time::timeout(DELIVERY_TIMEOUT, exchange(port, request))
        .await
        .unwrap_or_else(|_| {
            Err(anyhow::anyhow!(
                "the dispatch board on port {port} did not answer within {DELIVERY_TIMEOUT:?}"
            ))
        })
        .with_context(|| {
            format!(
                "could not reach the dispatch board on port {port} — \
                 the hook event was dropped. Is the board running?"
            )
        })
}

async fn exchange(port: u16, request: &HookRequest) -> Result<HookResponse> {
    let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .with_context(|| format!("connecting to 127.0.0.1:{port}"))?;
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
    // The connection task drives the exchange; it ends when the response is
    // complete, which is why its result is not the one we report.
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let body = serde_json::to_vec(request)?;
    let http_request = hyper::Request::builder()
        .method(hyper::Method::POST)
        .uri(HOOK_PATH)
        .header(hyper::header::HOST, "127.0.0.1")
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .body(http_body_util::Full::new(hyper::body::Bytes::from(body)))?;

    let response = sender.send_request(http_request).await?;
    let status = response.status();
    let bytes = response.into_body().collect().await?.to_bytes();
    if !status.is_success() {
        anyhow::bail!(
            "the board answered {status}: {}",
            String::from_utf8_lossy(&bytes)
        );
    }
    serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "decoding the board's answer: {}",
            String::from_utf8_lossy(&bytes)
        )
    })
}

/// `dispatch hook <id> <kind> [--kind <notification_kind>]`.
pub async fn run_event(
    port: u16,
    id: i64,
    kind: &str,
    notification_kind: Option<&str>,
) -> Result<()> {
    // The notification subtype (from `--kind`) is only meaningful for the
    // `notification` event; build it directly instead of parsing then
    // overwriting. An absent or unrecognised value stays `None`, which the
    // board maps to the raise/`needs_input` path for backward compatibility.
    let parsed = if kind == "notification" {
        HookEventKind::Notification(notification_kind.and_then(NotificationKind::parse))
    } else {
        HookEventKind::parse(kind).ok_or_else(|| {
            anyhow::anyhow!(
                "Invalid hook kind: {kind}. Valid: pre_tool_use, notification, stop, user_prompt_submit"
            )
        })?
    };
    deliver(
        port,
        ObservedEvent::Event {
            task_id: id,
            kind: parsed,
        },
    )
    .await
}

/// `dispatch hook-subagent <id> <action>`'s action, parsed at the boundary by
/// clap so `--help` enumerates the valid values and an unrecognised one is
/// rejected before anything is delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum SubagentAction {
    /// SubagentStart
    Start,
    /// SubagentStop
    Stop,
    /// SessionStart — drop every entry for the task without draining a
    /// deferred Stop.
    Clear,
}

/// `dispatch hook-subagent <id> start|stop|clear`.
pub async fn run_subagent(
    port: u16,
    id: i64,
    action: SubagentAction,
    agent_id: Option<String>,
    session_id: Option<String>,
) -> Result<()> {
    // `clear` (SessionStart) is deliberately the *non-draining* clear. A new,
    // resumed or cleared session means the previous turn is over, so a Stop
    // deferred by that turn is stale and must be voided rather than applied:
    // resume in particular keeps the task Running on purpose (see
    // `handle_retry_resume`), and draining here would strand it in Review
    // with a live agent and no UserPromptSubmit coming. The draining clear is
    // reached only from detach, whose rule owns no status of its own, and
    // never travels over the wire at all. See `ClearSubagentsOnSessionStart`
    // in `docs/specs/agent-health.allium`.
    if action == SubagentAction::Clear {
        return deliver(port, ObservedEvent::SubagentClear { task_id: id }).await;
    }
    // A start/stop with no agent_id/session_id carries no information — the
    // shell hook already guards this, but a bare CLI call must not deliver a
    // half-formed event. It is dropped here, before any delivery is
    // attempted, so it neither reaches the board nor fails when none is
    // running.
    let (Some(agent_id), Some(session_id)) = (agent_id, session_id) else {
        return Ok(());
    };
    deliver(
        port,
        ObservedEvent::Subagent {
            task_id: id,
            agent_id,
            session_id,
            stop: action == SubagentAction::Stop,
        },
    )
    .await
}

/// `dispatch hook-shell <id> <action>`'s action. Deliberately has no `clear`:
/// a backgrounded shell has no SessionStart-driven clear, only session
/// fencing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ShellAction {
    /// A Bash call with `run_in_background: true`
    Start,
    /// KillBash/TaskStop, or a BashOutput/TaskOutput signalling completion
    Stop,
}

/// `dispatch hook-shell <id> start|stop`.
pub async fn run_shell(
    port: u16,
    id: i64,
    action: ShellAction,
    shell_id: Option<String>,
    session_id: Option<String>,
) -> Result<()> {
    // Same information-free guard as `run_subagent`.
    let (Some(shell_id), Some(session_id)) = (shell_id, session_id) else {
        return Ok(());
    };
    deliver(
        port,
        ObservedEvent::Shell {
            task_id: id,
            shell_id,
            session_id,
            stop: action == ShellAction::Stop,
        },
    )
    .await
}

/// `dispatch hook-peer-message <id> --target <to> --body <body>`: an observed
/// native `SendMessage` tool call (task #4098). The board stamps the sender's
/// (and, when resolvable, the target's) row and appends the sender's
/// trajectory entry — the only audit record a native `SendMessage` gets,
/// since it never reaches dispatch's own MCP server.
pub async fn run_peer_message(port: u16, id: i64, target: String, body: String) -> Result<()> {
    deliver(
        port,
        ObservedEvent::PeerMessage {
            task_id: id,
            target,
            body,
        },
    )
    .await
}

/// What the PR gate decided. Returned rather than acted on, so the exit code
/// stays the caller's to choose — see `run_pr_gate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateVerdict {
    /// First attempt for this task: show the reminder, block the tool call.
    Block(String),
    /// Let the attempt through.
    Allow,
}

/// `dispatch pr-gate <id>`: the one hook whose answer the tool call waits on.
///
/// Unlike the observers it cannot simply drop its delivery when no board
/// answers, because its job *is* to answer. It **fails open** — every outcome
/// short of a first-attempt verdict returns [`GateVerdict::Allow`], including
/// an unreachable board, a board that could not decide, and an answer this
/// version cannot read. An unreachable board still propagates its error to
/// the caller, so the operator hears about it; what it must not do is stand
/// between an agent and its submission. The gate is a one-time reminder
/// rather than enforcement, which is what makes that trade the right way
/// round. See `PrLearningsGate` in `docs/specs/pr-workflow.allium`.
pub async fn run_pr_gate(port: u16, id: i64) -> Result<GateVerdict> {
    match send(port, &HookRequest::Ask(Question::PrGate { task_id: id })).await? {
        HookResponse::Answer(Answer::PrGate { reminder }) => Ok(match reminder {
            Some(text) => GateVerdict::Block(text),
            None => GateVerdict::Allow,
        }),
        HookResponse::Answer(Answer::Failed { reason }) => Err(anyhow::anyhow!(
            "the dispatch board could not decide the PR gate: {reason}"
        )),
        other => Err(unexpected_answer(other)),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// The payload must survive a round-trip through JSON with its subtype
    /// intact: the board decides `needs_input` from it, and a subtype lost in
    /// transit would look exactly like an older Claude Code that never sent
    /// one.
    #[test]
    fn a_notification_subtype_survives_the_wire() {
        let request = HookRequest::Observe(ObservedEvent::Event {
            task_id: 7,
            kind: HookEventKind::Notification(Some(NotificationKind::AuthSuccess)),
        });
        let json = serde_json::to_string(&request).unwrap();
        assert_eq!(
            serde_json::from_str::<HookRequest>(&json).unwrap(),
            request,
            "json was: {json}"
        );
    }

    /// An observation and a question are different deliveries on the wire, so
    /// the board cannot mistake one for the other.
    #[test]
    fn an_observation_and_a_question_are_distinct_on_the_wire() {
        let observed = serde_json::to_string(&HookRequest::Observe(ObservedEvent::SubagentClear {
            task_id: 1,
        }))
        .unwrap();
        let asked =
            serde_json::to_string(&HookRequest::Ask(Question::PrGate { task_id: 1 })).unwrap();
        assert_ne!(observed, asked);
    }

    #[test]
    fn every_observation_reports_its_task() {
        assert_eq!(
            ObservedEvent::PeerMessage {
                task_id: 42,
                target: "task-1".into(),
                body: "x".into(),
            }
            .task_id(),
            42
        );
    }
}
