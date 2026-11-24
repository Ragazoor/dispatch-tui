//! The orchestrator switch on an epic and the ready mark on a backlog task
//! (`core.allium: EpicOrchestration`, `ReadyMark`).

use chrono::{DateTime, Utc};

use crate::define_str_enum;

use super::{EpicId, TaskId};

/// `core.allium: LogEntryKind`: what an orchestrator's log entry records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogEntryKind {
    StartedTask,
    Answered,
    PassedOn,
    ActedOnTrouble,
    Session,
    Summary,
    Review,
}

define_str_enum!(LogEntryKind, "log entry kind" {
    StartedTask => "started_task",
    Answered => "answered",
    PassedOn => "passed_on",
    ActedOnTrouble => "acted_on_trouble",
    Session => "session",
    Summary => "summary",
    Review => "review",
});

/// `core.allium: OrchestratorLogEntry`: one line of an orchestrated epic's
/// log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchestratorLogEntry {
    pub id: i64,
    pub epic_id: EpicId,
    pub host: String,
    pub at: DateTime<Utc>,
    pub kind: LogEntryKind,
    pub task_id: Option<TaskId>,
    pub text: String,
    pub question: Option<String>,
    pub trouble: Option<String>,
}

/// `core.allium: EpicNote`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpicNote {
    pub id: i64,
    pub epic_id: EpicId,
    pub text: String,
    pub done: bool,
    pub written_at: DateTime<Utc>,
}

/// `core.allium: WaitingItem`: something that waits for the operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitingItem {
    pub id: i64,
    pub epic_id: EpicId,
    pub host: String,
    pub task_id: Option<TaskId>,
    pub about: String,
    pub tried: String,
    pub recommendation: String,
    pub raised_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

impl WaitingItem {
    pub fn is_open(&self) -> bool {
        self.resolved_at.is_none()
    }
}

/// `core.allium: StartRule`: which backlog tasks an orchestrator may start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartRule {
    MarkedOnly,
    Everything,
}

define_str_enum!(StartRule, "start rule" {
    MarkedOnly => "marked_only",
    Everything => "everything",
});

impl StartRule {
    pub const ALL: &'static [StartRule] = &[StartRule::MarkedOnly, StartRule::Everything];
}

/// `core.allium: ReadyMarker`: who set a ready mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadyMarker {
    Operator,
    Agent,
}

define_str_enum!(ReadyMarker, "ready marker" {
    Operator => "operator",
    Agent => "agent",
});

/// `core.allium: config.orchestrator_max_parallel`, the limit of an epic that
/// sets none: the store's, which refuses a start past it.
pub const DEFAULT_ORCHESTRATOR_LIMIT: i64 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpicOrchestration {
    pub epic_id: EpicId,
    pub start_rule: StartRule,
    /// `None` follows [`DEFAULT_ORCHESTRATOR_LIMIT`].
    pub max_parallel: Option<i64>,
    pub switched_on_at: DateTime<Utc>,
}

impl EpicOrchestration {
    pub fn limit(&self) -> i64 {
        self.max_parallel.unwrap_or(DEFAULT_ORCHESTRATOR_LIMIT)
    }
}

/// `core.allium: Orchestrator`: an orchestrated epic's orchestrator on one
/// machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orchestrator {
    pub id: i64,
    pub epic_id: EpicId,
    pub host: String,
    pub name: String,
    pub tmux_window: Option<super::TmuxWindow>,
    pub started_at: Option<DateTime<Utc>>,
    pub restarted_once: bool,
    pub session_died: bool,
    pub last_turn_at: Option<DateTime<Utc>>,
}

impl Orchestrator {
    pub fn is_live(&self) -> bool {
        self.tmux_window.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyMark {
    pub task_id: TaskId,
    pub marked_by: ReadyMarker,
    /// The marking agent's name when `marked_by` is [`ReadyMarker::Agent`].
    pub agent_name: Option<String>,
    pub marked_at: DateTime<Utc>,
}

/// `orchestrators.allium: config.orchestrator_log_shown`: how many log
/// entries a read and the orchestrator's prompt show, newest first.
pub const ORCHESTRATOR_LOG_SHOWN: usize = 10;

/// One log entry as every read renders it.
pub fn log_line(e: &OrchestratorLogEntry) -> String {
    let mut line = format!(
        "#{} {} {} {}",
        e.id,
        e.at.format("%Y-%m-%dT%H:%M:%SZ"),
        e.host,
        e.kind
    );
    if let Some(task) = e.task_id {
        line.push_str(&format!(" task #{}", task.0));
    }
    if let Some(trouble) = &e.trouble {
        line.push_str(&format!(" ({trouble})"));
    }
    if let Some(q) = &e.question {
        line.push_str(&format!(" — asked: {q}"));
    }
    line.push_str(&format!(" — {}", e.text));
    line
}

/// One open item as the epic read and the prompt render it.
pub fn item_line(i: &WaitingItem) -> String {
    let about = match i.task_id {
        Some(task) => format!("task #{}: {}", task.0, i.about),
        None => format!("the epic: {}", i.about),
    };
    format!(
        "#{} {about} — tried: {} — recommends: {} (raised {}, {})",
        i.id,
        i.tried,
        i.recommendation,
        i.raised_at.format("%Y-%m-%dT%H:%M:%SZ"),
        i.host
    )
}

/// One note as the epic read and the prompt render it.
pub fn note_line(n: &EpicNote) -> String {
    let done = if n.done { " [done]" } else { "" };
    format!("#{}{done} {}", n.id, n.text)
}

/// The notes of `epic`, open ones first, each group by id.
pub fn notes_in_order(notes: &[EpicNote], epic: EpicId) -> Vec<&EpicNote> {
    let mut mine: Vec<&EpicNote> = notes.iter().filter(|n| n.epic_id == epic).collect();
    mine.sort_by_key(|n| (n.done, n.id));
    mine
}

/// The open items of `epic`, oldest first.
pub fn open_items(items: &[WaitingItem], epic: EpicId) -> Vec<&WaitingItem> {
    let mut open: Vec<&WaitingItem> = items
        .iter()
        .filter(|i| i.epic_id == epic && i.is_open())
        .collect();
    open.sort_by_key(|i| i.id);
    open
}

/// The log entries that pass `keep`, newest first.
pub fn newest_first(
    log: &[OrchestratorLogEntry],
    keep: impl Fn(&OrchestratorLogEntry) -> bool,
) -> Vec<&OrchestratorLogEntry> {
    let mut kept: Vec<&OrchestratorLogEntry> = log.iter().filter(|e| keep(e)).collect();
    kept.sort_by_key(|e| std::cmp::Reverse(e.id));
    kept
}
