use super::*;
use chrono::Utc;

// --- Signal / FeedItem.signals ---

#[test]
fn signal_deserializes_kebab_case() {
    let s: Vec<Signal> = serde_json::from_str(r#"["direct-request","author-bot"]"#).unwrap();
    assert_eq!(s, vec![Signal::DirectRequest, Signal::AuthorBot]);
}

#[test]
fn feed_item_signals_default_empty_and_unknown_skipped() {
    // missing field -> empty
    let item: FeedItem = serde_json::from_str(
        r#"{"external_id":"x","title":"t","description":"","status":"backlog","tag":"pr-review"}"#,
    )
    .unwrap();
    assert!(item.signals.is_empty());
    // unknown signal value is dropped, not fatal
    let item2: FeedItem = serde_json::from_str(
            r#"{"external_id":"x","title":"t","description":"","status":"backlog","tag":"pr-review","signals":["reviewed","bogus"]}"#,
        )
        .unwrap();
    assert_eq!(item2.signals, vec![Signal::Reviewed]);
}

// --- TaskStatus ---

#[test]
fn status_roundtrip() {
    for &status in TaskStatus::ALL {
        let s = status.as_str();
        let parsed = TaskStatus::parse(s).expect("roundtrip failed");
        assert_eq!(status, parsed, "roundtrip failed for {:?}", status);
    }
}

#[test]
fn status_invalid_from_str() {
    assert!(TaskStatus::parse("").is_none());
    assert!(TaskStatus::parse("unknown").is_none());
    assert!(
        TaskStatus::parse("Backlog").is_none(),
        "should be case-sensitive"
    );
}

#[test]
fn parse_ready_maps_to_backlog() {
    assert_eq!(TaskStatus::parse("ready"), Some(TaskStatus::Backlog));
}

#[test]
fn status_next() {
    assert_eq!(TaskStatus::Backlog.next(), TaskStatus::Running);
    assert_eq!(TaskStatus::Running.next(), TaskStatus::Review);
    assert_eq!(TaskStatus::Review.next(), TaskStatus::Done);
    assert_eq!(
        TaskStatus::Done.next(),
        TaskStatus::Done,
        "Done.next() should stay Done"
    );
}

#[test]
fn status_prev() {
    assert_eq!(TaskStatus::Done.prev(), TaskStatus::Review);
    assert_eq!(TaskStatus::Review.prev(), TaskStatus::Running);
    assert_eq!(TaskStatus::Running.prev(), TaskStatus::Backlog);
    assert_eq!(
        TaskStatus::Backlog.prev(),
        TaskStatus::Backlog,
        "Backlog.prev() should stay Backlog"
    );
}

#[test]
fn status_column_index_roundtrip() {
    for &status in TaskStatus::ALL {
        let idx = status.column_index();
        let back = TaskStatus::from_column_index(idx).expect("column roundtrip failed");
        assert_eq!(status, back);
    }
}

#[test]
fn column_index_out_of_range() {
    assert!(TaskStatus::from_column_index(4).is_none());
    assert!(TaskStatus::from_column_index(999).is_none());
}

#[test]
fn column_count_matches_all_len() {
    assert_eq!(TaskStatus::COLUMN_COUNT, TaskStatus::ALL.len());
    assert_eq!(TaskStatus::COLUMN_COUNT, 4);
}

#[test]
fn task_status_display() {
    for &status in TaskStatus::ALL {
        assert_eq!(format!("{status}"), status.as_str());
    }
}

#[test]
fn task_status_from_str_roundtrip() {
    for &status in TaskStatus::ALL {
        let parsed: TaskStatus = status.as_str().parse().unwrap();
        assert_eq!(parsed, status);
    }
}

#[test]
fn task_status_from_str_error() {
    let result: Result<TaskStatus, _> = "bogus".parse();
    assert!(result.is_err());
}

/// core.allium: `enum TaskStatus { backlog | running | review | done }`.
/// The `archived` status was retired by ArchivedStatusMigration
/// (epics.allium); a stored or supplied "archived" no longer decodes.
#[test]
fn archived_is_no_longer_a_task_status() {
    assert!(TaskStatus::parse("archived").is_none());
    let result: Result<TaskStatus, _> = "archived".parse();
    assert!(result.is_err());
}

/// Every stored status is one of the four board columns: with `archived`
/// gone, `ALL` is every status there is, and each round-trips.
#[test]
fn every_task_status_is_a_board_column_and_round_trips() {
    assert_eq!(TaskStatus::ALL.len(), 4);
    for &s in TaskStatus::ALL {
        assert_eq!(TaskStatus::parse(s.as_str()), Some(s));
        assert_eq!(TaskStatus::from_column_index(s.column_index()), Some(s));
    }
}

/// Done is the tail of the progression now. It has no forward edge (there
/// is no terminal status beyond it — a task leaves the board by DeleteTask)
/// and MoveTaskBackward's prev_status(done) is review.
#[test]
fn done_is_the_last_status_and_steps_back_to_review() {
    assert_eq!(TaskStatus::Done.next(), TaskStatus::Done);
    assert_eq!(TaskStatus::Done.prev(), TaskStatus::Review);
    assert_eq!(TaskStatus::Review.next(), TaskStatus::Done);
}

// --- SubStatus ---

#[test]
fn substatus_roundtrip() {
    for &sub in SubStatus::ALL {
        let s = sub.as_str();
        let parsed: SubStatus = s
            .parse()
            .unwrap_or_else(|e| panic!("roundtrip failed for {s}: {e}"));
        assert_eq!(sub, parsed, "roundtrip failed for {s}");
    }
}

#[test]
fn substatus_as_str_is_snake_case() {
    assert_eq!(SubStatus::None.as_str(), "none");
    assert_eq!(SubStatus::Active.as_str(), "active");
    assert_eq!(SubStatus::NeedsInput.as_str(), "needs_input");
    assert_eq!(SubStatus::Stale.as_str(), "stale");
    assert_eq!(SubStatus::Crashed.as_str(), "crashed");
    assert_eq!(SubStatus::Conflict.as_str(), "conflict");
    assert_eq!(SubStatus::AwaitingReview.as_str(), "awaiting_review");
    assert_eq!(SubStatus::ChangesRequested.as_str(), "changes_requested");
    assert_eq!(SubStatus::Approved.as_str(), "approved");
    assert_eq!(SubStatus::PrClosed.as_str(), "pr_closed");
}

#[test]
fn substatus_from_str_invalid() {
    assert!("bogus".parse::<SubStatus>().is_err());
    assert!("".parse::<SubStatus>().is_err());
    assert!(
        "None".parse::<SubStatus>().is_err(),
        "should be case-sensitive"
    );
}

#[test]
fn substatus_display() {
    assert_eq!(format!("{}", SubStatus::NeedsInput), "needs_input");
    assert_eq!(format!("{}", SubStatus::AwaitingReview), "awaiting_review");
}

#[test]
fn substatus_valid_combinations() {
    // Backlog: only None
    assert!(SubStatus::None.is_valid_for(TaskStatus::Backlog));
    assert!(!SubStatus::Active.is_valid_for(TaskStatus::Backlog));
    assert!(!SubStatus::NeedsInput.is_valid_for(TaskStatus::Backlog));
    assert!(!SubStatus::AwaitingReview.is_valid_for(TaskStatus::Backlog));

    // Running: Active, NeedsInput, Stale, Crashed
    assert!(!SubStatus::None.is_valid_for(TaskStatus::Running));
    assert!(SubStatus::Active.is_valid_for(TaskStatus::Running));
    assert!(SubStatus::NeedsInput.is_valid_for(TaskStatus::Running));
    assert!(SubStatus::Stale.is_valid_for(TaskStatus::Running));
    assert!(SubStatus::Crashed.is_valid_for(TaskStatus::Running));
    assert!(!SubStatus::AwaitingReview.is_valid_for(TaskStatus::Running));

    // Review: AwaitingReview, ChangesRequested, Approved, PrClosed
    assert!(!SubStatus::None.is_valid_for(TaskStatus::Review));
    assert!(!SubStatus::Active.is_valid_for(TaskStatus::Review));
    assert!(SubStatus::AwaitingReview.is_valid_for(TaskStatus::Review));
    assert!(SubStatus::ChangesRequested.is_valid_for(TaskStatus::Review));
    assert!(SubStatus::Approved.is_valid_for(TaskStatus::Review));
    assert!(SubStatus::PrClosed.is_valid_for(TaskStatus::Review));
    assert!(!SubStatus::PrClosed.is_valid_for(TaskStatus::Running));

    // Done: only None
    assert!(SubStatus::None.is_valid_for(TaskStatus::Done));
    assert!(!SubStatus::Active.is_valid_for(TaskStatus::Done));
}

#[test]
fn substatus_default_for() {
    assert_eq!(SubStatus::default_for(TaskStatus::Backlog), SubStatus::None);
    assert_eq!(
        SubStatus::default_for(TaskStatus::Running),
        SubStatus::Active
    );
    assert_eq!(
        SubStatus::default_for(TaskStatus::Review),
        SubStatus::AwaitingReview
    );
    assert_eq!(SubStatus::default_for(TaskStatus::Done), SubStatus::None);
}

/// Asserted as a relative chain, not as literal integers: the slot numbers
/// carry gaps so the presentation layer can insert display-only overrides
/// between them, and pinning the literals makes every such insertion a
/// test edit.
#[test]
fn substatus_column_priority_matches_urgency_ordering() {
    let chain = [
        SubStatus::Conflict,
        SubStatus::PrClosed,
        SubStatus::Crashed,
        SubStatus::Stale,
        SubStatus::NeedsInput,
        SubStatus::ChangesRequested,
        SubStatus::Approved,
        SubStatus::AwaitingReview,
    ];
    for pair in chain.windows(2) {
        let (lower, higher) = (pair[0], pair[1]);
        assert!(
            lower.column_priority() < higher.column_priority(),
            "{lower:?} should sort above {higher:?}"
        );
    }

    // Shared slots.
    assert_eq!(
        SubStatus::Active.column_priority(),
        SubStatus::AwaitingReview.column_priority()
    );
    assert_eq!(
        SubStatus::None.column_priority(),
        SubStatus::AwaitingReview.column_priority()
    );
}

/// `pr_unreachable` is a Review-only attention state, exactly like
/// `pr_closed` (core.allium: SubStatus).
#[test]
fn pr_unreachable_is_valid_only_for_review() {
    assert!(SubStatus::PrUnreachable.is_valid_for(TaskStatus::Review));
    for status in [TaskStatus::Backlog, TaskStatus::Running, TaskStatus::Done] {
        assert!(
            !SubStatus::PrUnreachable.is_valid_for(status),
            "pr_unreachable must not be valid for {status:?}"
        );
    }
}

/// Sorts below `pr_closed` and above `changes_requested`: the card's review
/// state is not merely unfinished, it is unknown, which is worse than a
/// known task (board-layout.allium: Review-column section order).
#[test]
fn pr_unreachable_sorts_between_pr_closed_and_changes_requested() {
    assert!(
        SubStatus::PrClosed.column_priority() < SubStatus::PrUnreachable.column_priority(),
        "pr_closed should sort above pr_unreachable"
    );
    assert!(
        SubStatus::PrUnreachable.column_priority() < SubStatus::ChangesRequested.column_priority(),
        "pr_unreachable should sort above changes_requested"
    );
}

/// System-derived from PR polling, so the MCP tool must not offer it as a
/// value an agent can choose (mcp-task-tools.allium: UpdateTaskViaMcp).
#[test]
fn pr_unreachable_is_not_mcp_advertised() {
    assert!(!SubStatus::MCP_ADVERTISED.contains(&SubStatus::PrUnreachable));
    assert!(SubStatus::ALL.contains(&SubStatus::PrUnreachable));
}

#[test]
fn pr_unreachable_round_trips_as_snake_case() {
    assert_eq!(SubStatus::PrUnreachable.as_str(), "pr_unreachable");
    assert_eq!(
        "pr_unreachable".parse::<SubStatus>().unwrap(),
        SubStatus::PrUnreachable
    );
    assert_eq!(SubStatus::PrUnreachable.header_label(), "pr unreachable");
}

/// The Review column's ordering pivot: an approved PR is one keystroke from
/// merging, so it sorts above a PR that is merely awaiting a decision.
#[test]
fn approved_sorts_above_awaiting_review() {
    assert!(
        SubStatus::Approved.column_priority() < SubStatus::AwaitingReview.column_priority(),
        "approved should sort above awaiting review"
    );
}

#[test]
fn substatus_header_label_matches_display_text() {
    assert_eq!(SubStatus::None.header_label(), "");
    assert_eq!(SubStatus::Active.header_label(), "active");
    assert_eq!(SubStatus::NeedsInput.header_label(), "needs input");
    assert_eq!(SubStatus::Stale.header_label(), "stale");
    assert_eq!(SubStatus::Crashed.header_label(), "crashed");
    assert_eq!(SubStatus::Conflict.header_label(), "conflict");
    assert_eq!(SubStatus::AwaitingReview.header_label(), "awaiting review");
    assert_eq!(
        SubStatus::ChangesRequested.header_label(),
        "changes requested"
    );
    assert_eq!(SubStatus::Approved.header_label(), "approved");
    assert_eq!(SubStatus::PrClosed.header_label(), "pr closed");
}

// --- slugify ---

#[test]
fn slugify_normal() {
    assert_eq!(slugify("Hello World"), "hello-world");
}

#[test]
fn slugify_special_chars() {
    assert_eq!(slugify("Foo & Bar! (baz)"), "foo-bar-baz");
}

#[test]
fn slugify_empty() {
    assert_eq!(slugify(""), "task");
}

#[test]
fn slugify_only_special() {
    assert_eq!(slugify("!!!"), "task");
}

#[test]
fn slugify_collapsed_dashes() {
    assert_eq!(slugify("a---b"), "a-b");
    assert_eq!(slugify("a & & b"), "a-b");
}

#[test]
fn slugify_leading_trailing_special() {
    assert_eq!(slugify("  hello  "), "hello");
    assert_eq!(slugify("---hello---"), "hello");
}

#[test]
fn slugify_numbers() {
    assert_eq!(slugify("Task 42"), "task-42");
}

// --- Staleness ---

#[test]
fn staleness_fresh() {
    let now = Utc::now();
    let updated = now - chrono::Duration::hours(71);
    assert_eq!(Staleness::from_age(updated, now), Staleness::Fresh);
}

#[test]
fn staleness_fresh_boundary() {
    let now = Utc::now();
    // Exactly 3 days minus 1 second => still Fresh
    let updated = now - chrono::Duration::seconds(3 * 24 * 3600 - 1);
    assert_eq!(Staleness::from_age(updated, now), Staleness::Fresh);
}

#[test]
fn staleness_aging() {
    let now = Utc::now();
    let updated = now - chrono::Duration::days(3);
    assert_eq!(Staleness::from_age(updated, now), Staleness::Aging);
}

#[test]
fn staleness_aging_boundary() {
    let now = Utc::now();
    // Exactly 7 days minus 1 second => still Aging
    let updated = now - chrono::Duration::seconds(7 * 24 * 3600 - 1);
    assert_eq!(Staleness::from_age(updated, now), Staleness::Aging);
}

#[test]
fn staleness_stale() {
    let now = Utc::now();
    let updated = now - chrono::Duration::days(7);
    assert_eq!(Staleness::from_age(updated, now), Staleness::Stale);
}

#[test]
fn staleness_very_stale() {
    let now = Utc::now();
    let updated = now - chrono::Duration::days(30);
    assert_eq!(Staleness::from_age(updated, now), Staleness::Stale);
}

#[test]
fn staleness_future_is_fresh() {
    let now = Utc::now();
    let updated = now + chrono::Duration::hours(1);
    assert_eq!(Staleness::from_age(updated, now), Staleness::Fresh);
}

// --- format_age ---

#[test]
fn format_age_minutes() {
    let now = Utc::now();
    let updated = now - chrono::Duration::minutes(30);
    assert_eq!(format_age(updated, now), "<1h");
}

#[test]
fn format_age_one_hour() {
    let now = Utc::now();
    let updated = now - chrono::Duration::hours(1);
    assert_eq!(format_age(updated, now), "1h");
}

#[test]
fn format_age_hours() {
    let now = Utc::now();
    let updated = now - chrono::Duration::hours(23);
    assert_eq!(format_age(updated, now), "23h");
}

#[test]
fn format_age_one_day() {
    let now = Utc::now();
    let updated = now - chrono::Duration::hours(24);
    assert_eq!(format_age(updated, now), "1d");
}

#[test]
fn format_age_days() {
    let now = Utc::now();
    let updated = now - chrono::Duration::days(5);
    assert_eq!(format_age(updated, now), "5d");
}

#[test]
fn format_age_thirteen_days() {
    let now = Utc::now();
    let updated = now - chrono::Duration::days(13);
    assert_eq!(format_age(updated, now), "13d");
}

#[test]
fn format_age_two_weeks() {
    let now = Utc::now();
    let updated = now - chrono::Duration::days(14);
    assert_eq!(format_age(updated, now), "2w");
}

#[test]
fn format_age_three_weeks() {
    let now = Utc::now();
    let updated = now - chrono::Duration::days(21);
    assert_eq!(format_age(updated, now), "3w");
}

#[test]
fn format_age_future() {
    let now = Utc::now();
    let updated = now + chrono::Duration::hours(5);
    assert_eq!(format_age(updated, now), "<1h");
}

// --- format_detail_age ---

#[test]
fn format_detail_age_minutes() {
    let now = Utc::now();
    let updated = now - chrono::Duration::minutes(30);
    assert_eq!(format_detail_age(updated, now), "less than 1 hour");
}

#[test]
fn format_detail_age_one_hour() {
    let now = Utc::now();
    let updated = now - chrono::Duration::hours(1);
    assert_eq!(format_detail_age(updated, now), "1 hour");
}

#[test]
fn format_detail_age_hours() {
    let now = Utc::now();
    let updated = now - chrono::Duration::hours(5);
    assert_eq!(format_detail_age(updated, now), "5 hours");
}

#[test]
fn format_detail_age_one_day() {
    let now = Utc::now();
    let updated = now - chrono::Duration::hours(24);
    assert_eq!(format_detail_age(updated, now), "1 day");
}

#[test]
fn format_detail_age_days() {
    let now = Utc::now();
    let updated = now - chrono::Duration::days(10);
    assert_eq!(format_detail_age(updated, now), "10 days");
}

#[test]
fn format_detail_age_future() {
    let now = Utc::now();
    let updated = now + chrono::Duration::hours(3);
    assert_eq!(format_detail_age(updated, now), "less than 1 hour");
}

// --- DispatchMode / TaskTag ---

/// A bare Backlog task fixture: no worktree, no tmux window, no url.
/// Sibling `models` test modules build on it by overwriting fields.
pub(in crate::models) fn make_task_with(plan: Option<&str>, tag: Option<TaskTag>) -> Task {
    Task {
        plan_path: plan.map(String::from),
        tag,
        ..Default::default()
    }
}

// --- is_wrappable ---

fn wrappable_task(status: TaskStatus, worktree: Option<&str>) -> Task {
    Task {
        status,
        worktree: worktree.map(String::from),
        ..make_task_with(None, None)
    }
}

#[test]
fn is_wrappable_running_with_worktree() {
    assert!(wrappable_task(TaskStatus::Running, Some("/tmp/wt")).is_wrappable());
}

#[test]
fn is_wrappable_review_with_worktree() {
    assert!(wrappable_task(TaskStatus::Review, Some("/tmp/wt")).is_wrappable());
}

#[test]
fn is_wrappable_running_without_worktree() {
    assert!(!wrappable_task(TaskStatus::Running, None).is_wrappable());
}

#[test]
fn is_wrappable_backlog_with_worktree() {
    assert!(!wrappable_task(TaskStatus::Backlog, Some("/tmp/wt")).is_wrappable());
}

#[test]
fn dispatch_mode_with_plan_always_dispatches() {
    assert_eq!(
        DispatchMode::for_task(&make_task_with(Some("a plan"), None)),
        DispatchMode::Dispatch
    );
    assert_eq!(
        DispatchMode::for_task(&make_task_with(Some("a plan"), Some(TaskTag::Feature))),
        DispatchMode::Dispatch
    );
    assert_eq!(
        DispatchMode::for_task(&make_task_with(Some("a plan"), Some(TaskTag::PrReview))),
        DispatchMode::Dispatch
    );
    assert_eq!(
        DispatchMode::for_task(&make_task_with(Some("a plan"), Some(TaskTag::Research))),
        DispatchMode::Dispatch
    );
    assert_eq!(
        DispatchMode::for_task(&make_task_with(Some("a plan"), Some(TaskTag::Fix))),
        DispatchMode::Dispatch
    );
}

#[test]
fn task_tag_parse_roundtrip_new_tags() {
    for (tag, expected_str, expected_short) in [
        (TaskTag::PrReview, "pr-review", "pr-rev"),
        (TaskTag::Research, "research", "research"),
        (TaskTag::Fix, "fix", "fix"),
    ] {
        assert_eq!(tag.as_str(), expected_str, "as_str mismatch for {tag:?}");
        assert_eq!(
            TaskTag::parse(expected_str),
            Some(tag),
            "parse mismatch for {expected_str}"
        );
        assert_eq!(
            tag.short_label(),
            expected_short,
            "short_label mismatch for {tag:?}"
        );
        assert_eq!(
            tag.to_string(),
            expected_str,
            "Display mismatch for {tag:?}"
        );
        assert_eq!(
            expected_str.parse::<TaskTag>().unwrap(),
            tag,
            "FromStr mismatch for {expected_str}"
        );
    }
}

/// `TaskTag::ALL` backs the create_task/update_task MCP schema's tag enum
/// (dispatch.rs) — a variant added there without updating `ALL` would
/// silently under-advertise it.
#[test]
fn task_tag_all_has_every_variant() {
    assert_eq!(TaskTag::ALL.len(), 7);
}

#[test]
fn task_tag_is_review_only_for_pr_review_and_dependabot() {
    // Derived from ALL rather than listed, so an eighth tag cannot slip
    // past by being absent from both halves of a hand-written pair of
    // lists. `WrapUpViaMcp` in docs/specs/mcp-task-tools.allium spells
    // these two literals in its guard clause; a change here means a change
    // there.
    let review: Vec<TaskTag> = TaskTag::ALL
        .iter()
        .copied()
        .filter(TaskTag::is_review)
        .collect();
    assert_eq!(
        review,
        vec![TaskTag::PrReview, TaskTag::Dependabot],
        "the review set changed — update WrapUpViaMcp's guard in \
docs/specs/mcp-task-tools.allium to match"
    );
}

#[test]
fn dispatch_mode_without_plan_routes_only_research() {
    for tag in [
        None,
        Some(TaskTag::Feature),
        Some(TaskTag::Bug),
        Some(TaskTag::Chore),
        Some(TaskTag::PrReview),
        Some(TaskTag::Fix),
        Some(TaskTag::Dependabot),
    ] {
        assert_eq!(
            DispatchMode::for_task(&make_task_with(None, tag)),
            DispatchMode::Dispatch,
            "tag {tag:?} should fall through to Dispatch"
        );
    }
    assert_eq!(
        DispatchMode::for_task(&make_task_with(None, Some(TaskTag::Research))),
        DispatchMode::Research
    );
}

#[test]
fn task_tag_dependabot_serde_roundtrip() {
    let tag = TaskTag::Dependabot;
    let s = serde_json::to_string(&tag).unwrap();
    assert_eq!(s, "\"dependabot\"");
    let back: TaskTag = serde_json::from_str(&s).unwrap();
    assert_eq!(back, TaskTag::Dependabot);
}

#[test]
fn task_tag_dependabot_parse_and_labels() {
    assert_eq!(TaskTag::parse("dependabot"), Some(TaskTag::Dependabot));
    assert_eq!(TaskTag::Dependabot.as_str(), "dependabot");
    assert_eq!(TaskTag::Dependabot.short_label(), "dep");
}

#[test]
fn default_base_branch_is_main() {
    assert_eq!(DEFAULT_BASE_BRANCH, "main");
}
