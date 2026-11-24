//! Waking a live orchestrator (`agent-health.allium: WakeTheOrchestrator`,
//! `OrchestratorWakeEvents`; `orchestrators.allium: AgentPingExpired`,
//! `ReviewReminderDue`, `BoardWaitsForTheSummary`).
//!
//! The board compares an orchestrated epic's subtree with what it saw at its
//! last tick and turns what changed, and the timers that came due, into one
//! line typed into the orchestrator's window. Everything here is pure; the
//! runner in `orchestrator_host` reads the store and sends the line.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};

use crate::models::{
    DispatchRequest, DispatchRequestState, EpicId, LogEntryKind, OrchestratorLogEntry, SubStatus,
    Task, TaskId, TaskStatus, WaitingItem,
};

/// `config.orchestrator_ping_wait`.
pub(super) fn ping_wait() -> Duration {
    Duration::minutes(10)
}

/// `config.orchestrator_review_reminder_after`.
pub(super) fn review_reminder_after() -> Duration {
    Duration::hours(24)
}

/// What the board read of one orchestrated epic this tick.
pub(super) struct EpicReads<'a> {
    /// The tasks of the epic's subtree.
    pub tasks: Vec<&'a Task>,
    /// The subtree's tasks that carry a ready mark.
    pub ready: BTreeSet<TaskId>,
    /// The epic's waiting items.
    pub items: Vec<&'a WaitingItem>,
    /// The epic's log.
    pub log: &'a [OrchestratorLogEntry],
    /// Every dispatch request the board holds.
    pub requests: &'a [DispatchRequest],
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SeenTask {
    status: TaskStatus,
    sub_status: SubStatus,
    host: Option<String>,
    ready: bool,
}

/// What a tick saw of one epic's subtree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Subtree {
    tasks: BTreeMap<TaskId, SeenTask>,
    resolved_items: BTreeSet<i64>,
    failed_starts: BTreeSet<i64>,
}

/// The timers already woken for, so each wakes once per entry.
#[derive(Debug, Clone, Default)]
pub(super) struct Woken {
    pings: BTreeSet<i64>,
    reviews: BTreeSet<i64>,
}

/// This machine's task, or one no machine holds (`live_orchestrator_for`).
fn is_mine(host: &str, task_host: Option<&str>) -> bool {
    task_host.is_none_or(|h| h.is_empty() || h == host)
}

/// `started_by_orchestrator(request)`: a `started_task` entry for the task
/// written in the request's own transaction.
fn started_by_orchestrator(request: &DispatchRequest, log: &[OrchestratorLogEntry]) -> bool {
    log.iter().any(|e| {
        e.kind == LogEntryKind::StartedTask
            && e.task_id == Some(request.task_id)
            && e.at == request.requested_at
    })
}

pub(super) fn subtree(host: &str, reads: &EpicReads) -> Subtree {
    let tasks: BTreeMap<TaskId, SeenTask> = reads
        .tasks
        .iter()
        .map(|t| {
            (
                t.id,
                SeenTask {
                    status: t.status,
                    sub_status: t.sub_status,
                    host: t.host.clone(),
                    ready: reads.ready.contains(&t.id),
                },
            )
        })
        .collect();
    let failed_starts = reads
        .requests
        .iter()
        .filter(|r| {
            r.state == DispatchRequestState::Failed
                && r.host == host
                && tasks.contains_key(&r.task_id)
                && started_by_orchestrator(r, reads.log)
        })
        .map(|r| r.id.0)
        .collect();
    let resolved_items = reads
        .items
        .iter()
        .filter(|i| !i.is_open())
        .map(|i| i.id)
        .collect();
    Subtree {
        tasks,
        resolved_items,
        failed_starts,
    }
}

/// A store string typed into the window: control characters gone, one
/// line, cut short, quoted, so it cannot pass for the board's own words.
fn quoted(s: &str) -> String {
    let clean: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let line = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = line.chars().take(120).collect();
    format!("{:?}", cut)
}

const RECORD: &str = "record your one retry with orchestrator-trouble_record";

fn trouble_event(id: TaskId, sub: SubStatus) -> Option<String> {
    let id = id.0;
    Some(match sub {
        SubStatus::NeedsInput => format!(
            "task #{id}'s agent needs input: read what it asks, then answer it or pass it on"
        ),
        SubStatus::Stale => format!(
            "task #{id}'s agent is stale: if it waits for your answer or for an open item, \
             answer it or leave it; otherwise {RECORD} and ask it for status"
        ),
        SubStatus::Crashed => {
            format!("task #{id}'s agent crashed: {RECORD} and resume it with task_resume")
        }
        SubStatus::Conflict => format!(
            "task #{id}'s wrap-up hit a conflict: {RECORD} and tell its agent to resolve it and \
             wrap up again"
        ),
        _ => return None,
    })
}

/// What changed between two ticks' subtrees, in the scope
/// `WokenByItsSubtreeOnly` gives.
pub(super) fn changes(host: &str, prev: &Subtree, cur: &Subtree, reads: &EpicReads) -> Vec<String> {
    let mut events = Vec::new();
    for (id, now) in &cur.tasks {
        let Some(was) = prev.tasks.get(id) else {
            events.push(format!("task #{} is new in the epic", id.0));
            continue;
        };
        if was.ready != now.ready && now.status == TaskStatus::Backlog {
            events.push(if now.ready {
                format!("task #{} was marked ready", id.0)
            } else {
                format!("task #{} is no longer marked ready", id.0)
            });
        }
        let mine = is_mine(host, now.host.as_deref()) || is_mine(host, was.host.as_deref());
        if !mine {
            continue;
        }
        if was.status != now.status {
            events.push(format!(
                "task #{} moved from {} to {}",
                id.0,
                was.status.as_str(),
                now.status.as_str()
            ));
        }
        if was.sub_status != now.sub_status {
            events.extend(trouble_event(*id, now.sub_status));
        }
    }
    for request in reads
        .requests
        .iter()
        .filter(|r| cur.failed_starts.contains(&r.id.0) && !prev.failed_starts.contains(&r.id.0))
    {
        events.push(format!(
            "the start of task #{} failed (reason {}): {RECORD}, mark it ready and start it again",
            request.task_id.0,
            quoted(request.reason.as_deref().unwrap_or("none given"))
        ));
    }
    for item in reads
        .items
        .iter()
        .filter(|i| cur.resolved_items.contains(&i.id) && !prev.resolved_items.contains(&i.id))
    {
        events.push(format!("waiting item #{} was resolved", item.id));
    }
    events
}

fn has_open_item(reads: &EpicReads, task: TaskId) -> bool {
    reads
        .items
        .iter()
        .any(|i| i.task_id == Some(task) && i.is_open())
}

/// The newest entry of `kind` (and `trouble`, when given) per task.
fn newest_per_task<'a>(
    log: &'a [OrchestratorLogEntry],
    kind: LogEntryKind,
    trouble: Option<&str>,
) -> BTreeMap<TaskId, &'a OrchestratorLogEntry> {
    let mut newest: BTreeMap<TaskId, &OrchestratorLogEntry> = BTreeMap::new();
    for e in log
        .iter()
        .filter(|e| e.kind == kind && (trouble.is_none() || e.trouble.as_deref() == trouble))
    {
        if let Some(task) = e.task_id {
            let slot = newest.entry(task).or_insert(e);
            if e.at > slot.at {
                *slot = e;
            }
        }
    }
    newest
}

/// The pings and review reminders that came due at `now` and were not woken
/// for yet (`AgentPingExpired`, `ReviewReminderDue`).
pub(super) fn due(
    host: &str,
    reads: &EpicReads,
    now: DateTime<Utc>,
    woken: &mut Woken,
) -> Vec<String> {
    let mut events = Vec::new();
    let task = |id: TaskId| reads.tasks.iter().find(|t| t.id == id);
    for (id, entry) in newest_per_task(reads.log, LogEntryKind::ActedOnTrouble, Some("stale")) {
        let Some(t) = task(id) else { continue };
        if t.sub_status != SubStatus::Stale
            || !is_mine(host, t.host.as_deref())
            || entry.at + ping_wait() > now
            || has_open_item(reads, id)
            || !woken.pings.insert(entry.id)
        {
            continue;
        }
        events.push(format!(
            "task #{}'s agent did not answer your status ping: raise a waiting item",
            id.0
        ));
    }
    for (id, entry) in newest_per_task(reads.log, LogEntryKind::Review, None) {
        let Some(t) = task(id) else { continue };
        let raised_since = reads
            .items
            .iter()
            .any(|i| i.task_id == Some(id) && i.raised_at > entry.at);
        if t.status != TaskStatus::Review
            || !is_mine(host, t.host.as_deref())
            || entry.at + review_reminder_after() > now
            || raised_since
            || has_open_item(reads, id)
            || !woken.reviews.insert(entry.id)
        {
            continue;
        }
        events.push(format!(
            "task #{} has waited in review for {} hours (sub-status {}): raise a waiting item for \
             the operator, unless that sub-status shows the operator has given their review and \
             it waits on the other author",
            id.0,
            review_reminder_after().num_hours(),
            t.sub_status.as_str()
        ));
    }
    events
}

/// The board restarted and adopted a live window.
pub(super) fn restart_event() -> String {
    "the board restarted: check the board for what changed while it was away".to_string()
}

/// `BoardWaitsForTheSummary`'s ask.
pub(super) fn summary_event(epic: EpicId) -> String {
    format!(
        "epic #{} is done: record your closing summary with orchestrator-summary_record",
        epic.0
    )
}

/// The one line typed into the window.
pub(super) fn wake_line(epic: EpicId, events: &[String]) -> String {
    let mut line = format!("[dispatch] Epic #{}: {}.", epic.0, events.join("; "));
    if events.iter().any(|e| e.contains(RECORD)) {
        line.push_str(" If a retry record is refused, raise a waiting item instead.");
    }
    line
}

/// What the board does about a finished epic's live session this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SummaryStep {
    /// Ask for the closing summary and keep the window.
    Ask,
    /// Asked; keep the window until a turn completes or the wait passes.
    Wait,
    /// End the session.
    End,
}

/// When the board asked, by its own clock, and the newest log entry id of
/// the epic then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SummaryAsk {
    pub at: DateTime<Utc>,
    pub log_then: i64,
}

/// The newest entry id of `log`, `0` when it is empty.
pub(super) fn newest_entry(log: &[OrchestratorLogEntry]) -> i64 {
    log.iter().map(|e| e.id).max().unwrap_or(0)
}

/// The session ends once a summary entry newer than the ask is in the log,
/// or the wait has passed. A turn that ends without one, such as a turn
/// already running when the ask was typed, does not end it.
pub(super) fn summary_step(
    ask: Option<SummaryAsk>,
    log: &[OrchestratorLogEntry],
    now: DateTime<Utc>,
) -> SummaryStep {
    let Some(ask) = ask else {
        return SummaryStep::Ask;
    };
    let recorded = log
        .iter()
        .any(|e| e.kind == LogEntryKind::Summary && e.id > ask.log_then);
    if recorded || now >= ask.at + ping_wait() {
        SummaryStep::End
    } else {
        SummaryStep::Wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{DispatchMode, DispatchOrigin, DispatchRequestId, TaskBuilder};

    const HOST: &str = "host-a";

    fn at(min: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap() + Duration::minutes(min)
    }

    fn task(id: i64, status: TaskStatus, sub: SubStatus, host: Option<&str>) -> Task {
        let mut t = TaskBuilder::new(id)
            .title(format!("t{id}"))
            .epic(Some(1))
            .status(status)
            .sub_status(sub)
            .build();
        t.host = host.map(String::from);
        t
    }

    fn entry(
        id: i64,
        kind: LogEntryKind,
        task: i64,
        trouble: Option<&str>,
        when: DateTime<Utc>,
    ) -> OrchestratorLogEntry {
        OrchestratorLogEntry {
            id,
            epic_id: EpicId(1),
            host: HOST.into(),
            at: when,
            kind,
            task_id: Some(TaskId(task)),
            text: "x".into(),
            question: None,
            trouble: trouble.map(String::from),
        }
    }

    fn item(
        id: i64,
        task: i64,
        raised: DateTime<Utc>,
        resolved: Option<DateTime<Utc>>,
    ) -> WaitingItem {
        WaitingItem {
            id,
            epic_id: EpicId(1),
            host: HOST.into(),
            task_id: Some(TaskId(task)),
            about: "may I?".into(),
            tried: "t".into(),
            recommendation: "r".into(),
            raised_at: raised,
            resolved_at: resolved,
        }
    }

    fn failed_request(id: i64, task: i64, requested: DateTime<Utc>) -> DispatchRequest {
        DispatchRequest {
            id: DispatchRequestId(id),
            task_id: TaskId(task),
            host: HOST.into(),
            requested_by: "me".into(),
            mode: DispatchMode::Dispatch,
            origin: DispatchOrigin::Mcp,
            state: DispatchRequestState::Failed,
            requested_at: requested,
            claimed_at: None,
            finished_at: None,
            reason: Some("worktree exists".into()),
            worktree: None,
            tmux_window: None,
            reused_worktree: false,
            deadline: requested,
        }
    }

    fn reads<'a>(
        tasks: &'a [Task],
        items: &'a [WaitingItem],
        log: &'a [OrchestratorLogEntry],
        requests: &'a [DispatchRequest],
        ready: &[i64],
    ) -> EpicReads<'a> {
        EpicReads {
            tasks: tasks.iter().collect(),
            ready: ready.iter().map(|i| TaskId(*i)).collect(),
            items: items.iter().collect(),
            log,
            requests,
        }
    }

    fn diff(before: &EpicReads, after: &EpicReads) -> Vec<String> {
        changes(HOST, &subtree(HOST, before), &subtree(HOST, after), after)
    }

    #[test]
    fn nothing_changed_wakes_nothing() {
        let tasks = [task(10, TaskStatus::Running, SubStatus::Active, Some(HOST))];
        let r = reads(&tasks, &[], &[], &[], &[]);
        assert!(diff(&r, &r).is_empty());
    }

    #[test]
    fn a_new_task_and_a_ready_mark_wake_it_for_the_whole_subtree() {
        let before = [task(
            10,
            TaskStatus::Backlog,
            SubStatus::None,
            Some("host-b"),
        )];
        let after = [
            before[0].clone(),
            task(11, TaskStatus::Backlog, SubStatus::None, None),
        ];
        let events = diff(
            &reads(&before, &[], &[], &[], &[]),
            &reads(&after, &[], &[], &[], &[10]),
        );
        assert_eq!(
            events,
            vec!["task #10 was marked ready", "task #11 is new in the epic"]
        );
    }

    #[test]
    fn status_and_trouble_wake_it_for_this_machines_tasks_only() {
        let before = [
            task(10, TaskStatus::Running, SubStatus::Active, Some(HOST)),
            task(11, TaskStatus::Running, SubStatus::Active, Some("host-b")),
            task(12, TaskStatus::Running, SubStatus::Active, Some(HOST)),
        ];
        let after = [
            task(10, TaskStatus::Running, SubStatus::Crashed, Some(HOST)),
            task(
                11,
                TaskStatus::Review,
                SubStatus::AwaitingReview,
                Some("host-b"),
            ),
            task(
                12,
                TaskStatus::Review,
                SubStatus::AwaitingReview,
                Some(HOST),
            ),
        ];
        let events = diff(
            &reads(&before, &[], &[], &[], &[]),
            &reads(&after, &[], &[], &[], &[]),
        );
        assert_eq!(events.len(), 2, "{events:?}");
        assert!(
            events[0].starts_with("task #10's agent crashed"),
            "{events:?}"
        );
        assert!(
            events[0].contains("orchestrator-trouble_record"),
            "{events:?}"
        );
        assert_eq!(events[1], "task #12 moved from running to review");
    }

    #[test]
    fn a_failed_start_the_orchestrator_asked_for_wakes_it_once() {
        let tasks = [task(10, TaskStatus::Backlog, SubStatus::None, None)];
        let log = [entry(1, LogEntryKind::StartedTask, 10, None, at(0))];
        let mine = [failed_request(7, 10, at(0))];
        let someone_elses = [failed_request(8, 10, at(5))];

        let woke = diff(
            &reads(&tasks, &[], &log, &[], &[]),
            &reads(&tasks, &[], &log, &mine, &[]),
        );
        assert_eq!(woke.len(), 1, "{woke:?}");
        assert!(
            woke[0].contains("the start of task #10 failed (reason \"worktree exists\")"),
            "{woke:?}"
        );
        assert!(diff(
            &reads(&tasks, &[], &log, &mine, &[]),
            &reads(&tasks, &[], &log, &mine, &[])
        )
        .is_empty());
        assert!(diff(
            &reads(&tasks, &[], &log, &[], &[]),
            &reads(&tasks, &[], &log, &someone_elses, &[])
        )
        .is_empty());
    }

    #[test]
    fn a_resolved_item_wakes_it() {
        let tasks = [task(10, TaskStatus::Running, SubStatus::Active, Some(HOST))];
        let open = [item(3, 10, at(0), None)];
        let resolved = [item(3, 10, at(0), Some(at(1)))];
        let events = diff(
            &reads(&tasks, &open, &[], &[], &[]),
            &reads(&tasks, &resolved, &[], &[], &[]),
        );
        assert_eq!(events, vec!["waiting item #3 was resolved"]);
    }

    #[test]
    fn an_unanswered_ping_comes_due_once_after_the_wait() {
        let tasks = [task(10, TaskStatus::Running, SubStatus::Stale, Some(HOST))];
        let log = [entry(
            1,
            LogEntryKind::ActedOnTrouble,
            10,
            Some("stale"),
            at(0),
        )];
        let r = reads(&tasks, &[], &log, &[], &[]);
        let mut woken = Woken::default();

        assert!(due(HOST, &r, at(9), &mut woken).is_empty());
        let events = due(HOST, &r, at(10), &mut woken);
        assert_eq!(events.len(), 1, "{events:?}");
        assert!(events[0].contains("did not answer your status ping"));
        assert!(
            due(HOST, &r, at(11), &mut woken).is_empty(),
            "once per entry"
        );
    }

    #[test]
    fn a_ping_skips_a_task_with_an_open_item_or_that_works_again() {
        let stale = [task(10, TaskStatus::Running, SubStatus::Stale, Some(HOST))];
        let working = [task(10, TaskStatus::Running, SubStatus::Active, Some(HOST))];
        let log = [entry(
            1,
            LogEntryKind::ActedOnTrouble,
            10,
            Some("stale"),
            at(0),
        )];
        let open = [item(3, 10, at(1), None)];

        assert!(due(
            HOST,
            &reads(&stale, &open, &log, &[], &[]),
            at(20),
            &mut Woken::default()
        )
        .is_empty());
        assert!(due(
            HOST,
            &reads(&working, &[], &log, &[], &[]),
            at(20),
            &mut Woken::default()
        )
        .is_empty());
    }

    #[test]
    fn a_review_reminder_comes_due_once_per_stay() {
        let tasks = [task(
            10,
            TaskStatus::Review,
            SubStatus::AwaitingReview,
            Some(HOST),
        )];
        let log = [entry(1, LogEntryKind::Review, 10, None, at(0))];
        let day = 24 * 60;

        let r = reads(&tasks, &[], &log, &[], &[]);
        assert!(due(HOST, &r, at(day - 1), &mut Woken::default()).is_empty());
        let events = due(HOST, &r, at(day), &mut Woken::default());
        assert_eq!(events.len(), 1, "{events:?}");
        assert!(
            events[0].contains("has waited in review for 24 hours (sub-status awaiting_review)"),
            "{events:?}"
        );

        // An item raised during this stay, even one already resolved, is the
        // stay's reminder: a restarted board does not remind again.
        let raised = [item(3, 10, at(day), Some(at(day + 5)))];
        let r = reads(&tasks, &raised, &log, &[], &[]);
        assert!(due(HOST, &r, at(2 * day), &mut Woken::default()).is_empty());
    }

    #[test]
    fn a_reason_is_quoted_cleaned_and_cut() {
        let typed = quoted("bad\u{1b}[2J\nreason \"x\"");
        assert_eq!(typed, "\"bad [2J reason \\\"x\\\"\"");
        assert!(quoted(&"y".repeat(500)).len() < 130);
    }

    #[test]
    fn the_wake_line_is_one_line_and_says_what_a_refused_record_means() {
        let line = wake_line(
            EpicId(4),
            &[trouble_event(TaskId(9), SubStatus::Crashed).unwrap()],
        );
        assert!(
            line.starts_with("[dispatch] Epic #4: task #9's agent crashed"),
            "{line}"
        );
        assert!(
            line.ends_with("If a retry record is refused, raise a waiting item instead."),
            "{line}"
        );
        assert!(!line.contains('\n'));
        let plain = wake_line(EpicId(4), &["task #9 was marked ready".into()]);
        assert_eq!(plain, "[dispatch] Epic #4: task #9 was marked ready.");
    }

    #[test]
    fn the_summary_ask_waits_for_the_summary_or_the_ping_wait() {
        let earlier = [entry(4, LogEntryKind::Summary, 0, None, at(-60))];
        let recorded = [
            entry(4, LogEntryKind::Summary, 0, None, at(-60)),
            entry(9, LogEntryKind::Summary, 0, None, at(2)),
        ];
        let asked = Some(SummaryAsk {
            at: at(0),
            log_then: 4,
        });
        assert_eq!(summary_step(None, &[], at(0)), SummaryStep::Ask);
        assert_eq!(summary_step(asked, &earlier, at(5)), SummaryStep::Wait);
        assert_eq!(summary_step(asked, &recorded, at(5)), SummaryStep::End);
        assert_eq!(summary_step(asked, &earlier, at(10)), SummaryStep::End);
    }
}
