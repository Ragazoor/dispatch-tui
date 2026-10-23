use super::*;
use crate::models::test_tmux_window;
use crate::models::SubStatus;
use crate::tui::tests::{make_task, make_unprovisioned_task};

/// Every `CardIndicator` variant, paired with the border colour it claims.
///
/// Written out rather than generated so that adding a variant forces a
/// deliberate entry here — the same reason `state_border_color` matches
/// exhaustively. If this list and that match ever disagree, one of them is
/// the mistake and the test says which.
fn every_indicator() -> Vec<(CardIndicator, Option<Color>, &'static str)> {
    vec![
        // The six hard failures. This is the *membership* claim: these six
        // and no others are red. Only `Crashed` is exercised through the
        // renderer, so without this a state quietly promoted into or dropped
        // out of the set would pass every other test.
        (CardIndicator::Unprovisioned, Some(RED), "unprovisioned"),
        (
            CardIndicator::AutoDispatchFailed,
            Some(RED),
            "auto-dispatch failed",
        ),
        (CardIndicator::Conflict, Some(RED), "rebase conflict"),
        (CardIndicator::Crashed, Some(RED), "crashed"),
        (CardIndicator::RespawnFailed, Some(RED), "respawn failed"),
        (
            CardIndicator::ScheduleFailed {
                schedule_suffix: String::new(),
            },
            Some(RED),
            "schedule failed",
        ),
        // The two attention states.
        (CardIndicator::Blocked, Some(YELLOW), "blocked"),
        (
            CardIndicator::Stale {
                inactive_mins: Some(7),
            },
            Some(YELLOW),
            "stale",
        ),
        // Everything else claims nothing. `Dispatching` is the load-bearing
        // entry: it renders an amber *indicator* while claiming no border,
        // which is a judgement rather than a category and so the likeliest
        // exclusion to be "fixed" by someone who reads the glyph and assumes
        // the missing border was an oversight. It is also an absence, which is
        // what a renderer drifts into asserting by accident.
        (
            CardIndicator::Dispatching { spinner_frame: 0 },
            None,
            "dispatching",
        ),
        (
            CardIndicator::DetachedReview {
                pr_label: "PR #1".to_string(),
            },
            None,
            "detached review",
        ),
        (CardIndicator::Detached, None, "detached"),
        (CardIndicator::Running { subagents: 0 }, None, "running"),
        (
            CardIndicator::ReviewPr {
                pr_label: "PR #1".to_string(),
            },
            None,
            "review PR",
        ),
        (
            CardIndicator::DoneMerged {
                pr_label: "PR #1".to_string(),
            },
            None,
            "done merged",
        ),
        (
            CardIndicator::Idle {
                status: TaskStatus::Backlog,
                age: "1h".to_string(),
                staleness: Staleness::Fresh,
                plan_indicator: "",
                tag_suffix: String::new(),
                schedule_suffix: String::new(),
            },
            None,
            "idle",
        ),
    ]
}

// -- Phoenix marker ----------------------------------------------------
//
// board-visuals.allium's "Phoenix marker": the glyph says "this task still owes a
// respawn" in every column. In backlog that is what a phoenix task IS; in
// done it means the copy did not land.

fn phoenix_task(id: i64, status: TaskStatus) -> Task {
    let mut t = make_task(id, status);
    t.phoenix = true;
    t
}

#[test]
fn only_a_phoenix_task_carries_the_marker() {
    assert_eq!(
        phoenix_marker(&phoenix_task(1, TaskStatus::Backlog)),
        PHOENIX_GLYPH
    );
    assert_eq!(phoenix_marker(&make_task(2, TaskStatus::Backlog)), "");
}

/// The one place on the board where a glyph's rendered width and its
/// character count differ: `♻` is drawn two columns wide, so the marker
/// costs THREE cells (leading space included) rather than the two a
/// `chars().count()` budget would reserve. Under-reserve and every phoenix
/// card's title overruns its frame by a column.
#[test]
fn the_marker_reserves_its_rendered_width_not_its_character_count() {
    let recurring = phoenix_task(1, TaskStatus::Backlog);
    let ordinary = make_task(1, TaskStatus::Backlog);

    let extra = task_card_prefix_width(&recurring, false, false)
        - task_card_prefix_width(&ordinary, false, false);

    assert_eq!(
        extra, 3,
        "a space plus a two-column glyph; {PHOENIX_GLYPH:?} counts as one char but draws as two"
    );
}

/// A phoenix task sitting in done is one whose respawn did not land — the
/// flag is cleared exactly when the successor is created. It is a hard
/// failure, so it takes the red border and a ⚠ line.
#[test]
fn a_phoenix_task_left_in_done_classifies_as_respawn_failed() {
    let now = Utc::now();
    let app = App::new(vec![]);
    let task = phoenix_task(1, TaskStatus::Done);

    let indicator = classify_card_indicator(&task, task.status, &app, now);

    assert_eq!(indicator, CardIndicator::RespawnFailed);
    assert_eq!(state_border_color(&indicator), Some(RED));
    let text = line_text(&render_card_indicator(indicator, &[]));
    assert!(text.contains("respawn failed"), "got {text:?}");
    assert!(
        text.contains('\u{26a0}'),
        "the red set renders a warning glyph, got {text:?}"
    );
}

/// It outranks the merged-PR card: a phoenix task completed by a PR merge
/// whose respawn then failed must say so, not report the merge.
#[test]
fn respawn_failed_outranks_done_merged() {
    let now = Utc::now();
    let app = App::new(vec![]);
    let mut task = phoenix_task(1, TaskStatus::Done);
    task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/7",
        crate::models::UrlType::Pr,
    ));

    assert_eq!(
        classify_card_indicator(&task, task.status, &app, now),
        CardIndicator::RespawnFailed
    );
}

/// A successful respawn clears the flag, so the predecessor's done card is
/// an ordinary one — and a phoenix task that has NOT reached done yet is
/// not a failure either.
#[test]
fn respawn_failed_needs_both_the_flag_and_done() {
    let now = Utc::now();
    let app = App::new(vec![]);
    for task in [
        make_task(1, TaskStatus::Done),
        phoenix_task(2, TaskStatus::Backlog),
    ] {
        assert_ne!(
            classify_card_indicator(&task, task.status, &app, now),
            CardIndicator::RespawnFailed,
            "task {} should not read as a failed respawn",
            task.id
        );
    }
}

#[test]
fn every_indicator_claims_the_border_its_severity_earns() {
    for (indicator, expected, name) in every_indicator() {
        assert_eq!(
            state_border_color(&indicator),
            expected,
            "{name} claims the wrong border colour"
        );
    }
}

#[test]
fn exactly_six_indicators_are_hard_failures() {
    // The counts are the membership claim stated a second way: the per-variant
    // test above would still pass if a variant were *added* to the red set and
    // its expectation updated in the same edit. These numbers make that edit
    // visible.
    let all = every_indicator();
    let red = all.iter().filter(|(_, c, _)| *c == Some(RED)).count();
    let amber = all.iter().filter(|(_, c, _)| *c == Some(YELLOW)).count();
    let none = all.iter().filter(|(_, c, _)| c.is_none()).count();
    assert_eq!(red, 6, "the hard-failure set must have exactly six members");
    assert_eq!(amber, 2, "the attention set must have exactly two members");
    assert_eq!(
        red + amber + none,
        all.len(),
        "a border colour outside {{red, amber, none}} has appeared"
    );
}

fn stale_task(last_pre_tool_use_at: Option<DateTime<Utc>>) -> crate::models::Task {
    let mut t = make_task(1, TaskStatus::Running);
    t.sub_status = SubStatus::Stale;
    t.worktree = Some("/repo/.worktrees/1-t".to_string());
    t.tmux_window = Some(test_tmux_window("task-1"));
    t.last_pre_tool_use_at = last_pre_tool_use_at;
    t
}

fn line_text(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

#[test]
fn stale_card_with_known_timestamp_shows_minutes() {
    let now = Utc::now();
    let task = stale_task(Some(now - chrono::Duration::minutes(7)));
    let app = App::new(vec![]);
    let indicator = classify_card_indicator(&task, task.status, &app, now);
    assert_eq!(
        indicator,
        CardIndicator::Stale {
            inactive_mins: Some(7)
        },
    );
    let text = line_text(&render_card_indicator(indicator, &[]));
    assert!(text.contains("stale · 7m"), "got {text:?}");
}

#[test]
fn running_without_worktree_classifies_unprovisioned() {
    let now = Utc::now();
    let task = make_unprovisioned_task(1, TaskStatus::Running);
    let app = App::new(vec![]);
    let indicator = classify_card_indicator(&task, task.status, &app, now);
    assert_eq!(indicator, CardIndicator::Unprovisioned);
    let text = line_text(&render_card_indicator(indicator, &[]));
    assert!(text.contains("no worktree"), "got {text:?}");
    assert!(!text.contains("running"), "got {text:?}");
}

#[test]
fn review_without_worktree_classifies_unprovisioned() {
    let now = Utc::now();
    let mut task = make_unprovisioned_task(1, TaskStatus::Review);
    task.sub_status = SubStatus::AwaitingReview;
    task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/7",
        crate::models::UrlType::Pr,
    ));
    let app = App::new(vec![]);
    assert_eq!(
        classify_card_indicator(&task, task.status, &app, now),
        CardIndicator::Unprovisioned,
    );
}

/// `@guarantee DispatchingOutranksIt` — the claim writes Running before the
/// worktree exists, so every in-flight dispatch passes through the
/// unprovisioned state. It must keep rendering "dispatching…"; otherwise
/// every normal dispatch renders as broken while it is in flight.
#[test]
fn dispatching_outranks_unprovisioned() {
    let now = Utc::now();
    let task = make_unprovisioned_task(1, TaskStatus::Running);
    let mut app = App::new(vec![task.clone()]);
    app.mark_dispatching(task.id);
    let indicator = classify_card_indicator(&task, task.status, &app, now);
    assert!(
        matches!(indicator, CardIndicator::Dispatching { .. }),
        "got {indicator:?}",
    );
}

/// The epic auto-dispatch chain claims its next subtask inside the MCP
/// handler and never enters `app.dispatching`, so the map alone would let
/// every chained subtask render as broken for its whole provisioning
/// window. A fresh claim stamp keeps it on the ordinary running card.
#[test]
fn freshly_claimed_task_not_in_dispatching_map_still_shows_running() {
    let now = Utc::now();
    let mut task = make_unprovisioned_task(1, TaskStatus::Running);
    task.last_pre_tool_use_at = Some(now - chrono::Duration::seconds(5));
    let app = App::new(vec![task.clone()]);
    assert!(
        !app.is_dispatching(task.id),
        "not in the map, by construction"
    );
    assert_eq!(
        classify_card_indicator(&task, task.status, &app, now),
        CardIndicator::Running { subagents: 0 },
    );
}

/// Once the claim ages past the request timeout (store-split.allium:
/// config.dispatch_request_timeout, 60s), "slow" becomes "dead" and the
/// card flips to the warning.
#[test]
fn stale_claim_flips_to_unprovisioned() {
    let now = Utc::now();
    let mut task = make_unprovisioned_task(1, TaskStatus::Running);
    task.last_pre_tool_use_at = Some(
        now - chrono::Duration::from_std(crate::models::DISPATCH_REQUEST_TIMEOUT).unwrap()
            - chrono::Duration::seconds(1),
    );
    let app = App::new(vec![task.clone()]);
    assert_eq!(
        classify_card_indicator(&task, task.status, &app, now),
        CardIndicator::Unprovisioned,
    );
}

/// `@guarantee NotShownWhenProvisioned` — a worktree with no window is
/// `detached` (resumable), the opposite of unprovisioned.
#[test]
fn running_with_worktree_no_window_stays_detached() {
    let now = Utc::now();
    let mut task = make_task(1, TaskStatus::Running);
    task.tmux_window = None;
    let app = App::new(vec![]);
    assert_eq!(
        classify_card_indicator(&task, task.status, &app, now),
        CardIndicator::Detached,
    );
}

#[test]
fn running_with_worktree_and_window_stays_running() {
    let now = Utc::now();
    let task = make_task(1, TaskStatus::Running);
    let app = App::new(vec![]);
    assert_eq!(
        classify_card_indicator(&task, task.status, &app, now),
        CardIndicator::Running { subagents: 0 },
    );
}

fn label_of(indicator: CardIndicator) -> String {
    render_card_indicator(indicator, &[])
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect()
}

#[test]
fn running_card_shows_subagent_count() {
    let text = label_of(CardIndicator::Running { subagents: 3 });
    assert!(text.contains("running \u{00b7} 3 agents"), "got: {text:?}");
}

#[test]
fn running_card_uses_the_singular_for_one_subagent() {
    let text = label_of(CardIndicator::Running { subagents: 1 });
    assert!(text.contains("running \u{00b7} 1 agent"), "got: {text:?}");
    assert!(!text.contains("1 agents"), "got: {text:?}");
}

#[test]
fn running_card_omits_the_suffix_at_zero() {
    let text = label_of(CardIndicator::Running { subagents: 0 });
    assert!(
        !text.contains("agent"),
        "zero subagents must render no suffix; got: {text:?}"
    );
}

#[test]
fn stale_card_without_timestamp_omits_minutes() {
    let now = Utc::now();
    let task = stale_task(None);
    let app = App::new(vec![]);
    let indicator = classify_card_indicator(&task, task.status, &app, now);
    assert_eq!(
        indicator,
        CardIndicator::Stale {
            inactive_mins: None
        },
    );
    let text = line_text(&render_card_indicator(indicator, &[]));
    assert!(text.contains("stale"), "got {text:?}");
    assert!(!text.contains('m'), "got {text:?}");
}

// board-visuals.allium: CardLeadsWithTheAgentName. Line 1 leads with the
// current agent's name, and with `#<id>` only when the task has no agent.

fn title_line_text(task: &Task) -> String {
    line_text(&card_title_line(
        task,
        "a title",
        false,
        false,
        Color::Reset,
        (false, false),
    ))
}

#[test]
fn a_card_with_a_current_agent_leads_with_its_name_and_not_the_id() {
    let mut task = make_task(98332, TaskStatus::Running);
    task.agent_name = Some("paivi".into());

    let text = title_line_text(&task);

    assert!(text.contains(" paivi a title"), "{text:?}");
    assert!(!text.contains("98332"), "{text:?}");
}

#[test]
fn a_card_without_a_current_agent_leads_with_the_id() {
    let task = make_task(98332, TaskStatus::Backlog);

    let text = title_line_text(&task);

    assert!(text.contains(" #98332 a title"), "{text:?}");
}

/// The title's budget is what the lead leaves: a five-letter name costs the
/// same as `#` plus a four-digit id.
#[test]
fn the_title_budget_follows_the_lead_word_not_the_id() {
    let mut named = make_task(1, TaskStatus::Running);
    named.agent_name = Some("paivi".into());
    let numbered = make_task(1234, TaskStatus::Running);

    assert_eq!(
        task_card_prefix_width(&named, false, false),
        task_card_prefix_width(&numbered, false, false)
    );
}

// -- Schedule marker ---------------------------------------------------
//
// board-visuals.allium's "Schedule marker": a pending scheduled run shows on
// the metadata line, in local time; a failed scheduled launch is a hard
// failure.

fn scheduled_task(next_due: Option<DateTime<Utc>>) -> Task {
    Task {
        schedule: Some("0 9 * * 1".into()),
        next_due,
        ..make_task(1, TaskStatus::Backlog)
    }
}

#[test]
fn a_pending_run_reads_as_the_local_due_time() {
    let now = Utc.with_ymd_and_hms(2026, 10, 21, 10, 0, 0).unwrap();
    let due = Utc.with_ymd_and_hms(2026, 10, 26, 9, 0, 0).unwrap();
    let cet = chrono::FixedOffset::east_opt(3600).unwrap();
    assert_eq!(
        schedule_suffix(Some(due), now, &Utc),
        " \u{00b7} \u{25f7} Oct 26 09:00"
    );
    assert_eq!(
        schedule_suffix(Some(due), now, &cet),
        " \u{00b7} \u{25f7} Oct 26 10:00"
    );
}

#[test]
fn a_passed_due_time_reads_due() {
    let now = Utc.with_ymd_and_hms(2026, 10, 21, 10, 0, 0).unwrap();
    let due = Utc.with_ymd_and_hms(2026, 10, 19, 9, 0, 0).unwrap();
    assert_eq!(
        schedule_suffix(Some(due), now, &Utc),
        " \u{00b7} \u{25f7} due"
    );
}

#[test]
fn no_pending_run_shows_nothing() {
    assert_eq!(schedule_suffix(None, Utc::now(), &Utc), "");
}

#[test]
fn a_scheduled_backlog_card_carries_the_marker_on_its_idle_line() {
    let now = Utc::now();
    let app = App::new(vec![]);
    let task = scheduled_task(Some(now + chrono::Duration::days(3)));
    let text = label_of(classify_card_indicator(&task, task.status, &app, now));
    assert!(text.contains('\u{25f7}'), "got {text:?}");
}

#[test]
fn a_failed_scheduled_launch_in_backlog_is_a_hard_failure() {
    let now = Utc::now();
    let app = App::new(vec![]);
    let task = Task {
        schedule_error: Some("repo missing".into()),
        ..scheduled_task(None)
    };
    let indicator = classify_card_indicator(&task, task.status, &app, now);
    assert_eq!(
        indicator,
        CardIndicator::ScheduleFailed {
            schedule_suffix: String::new()
        }
    );
    let text = label_of(indicator);
    assert!(text.contains("\u{26a0} schedule failed"), "got {text:?}");

    // Out of backlog the failure is no longer waiting on anyone.
    let moved_on = Task {
        status: TaskStatus::Done,
        ..task
    };
    assert!(!matches!(
        classify_card_indicator(&moved_on, moved_on.status, &app, now),
        CardIndicator::ScheduleFailed { .. }
    ));
}

/// A phoenix task re-armed after a failed launch says both.
#[test]
fn a_rearmed_failed_phoenix_shows_the_failure_and_the_next_try() {
    let now = Utc::now();
    let app = App::new(vec![]);
    let task = Task {
        phoenix: true,
        schedule_error: Some("repo missing".into()),
        ..scheduled_task(Some(now + chrono::Duration::days(3)))
    };
    let text = label_of(classify_card_indicator(&task, task.status, &app, now));
    assert!(
        text.contains("\u{26a0} schedule failed \u{00b7} \u{25f7} "),
        "got {text:?}"
    );
}
