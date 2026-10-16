use super::*;
use crate::models::tasks::model_tests::make_task_with;
use crate::models::{test_tmux_window, TaskTag, TaskUrl, UrlType};

/// A Review task, provisioned and live (worktree + tmux window), no url.
fn review_task(sub_status: SubStatus, tag: Option<TaskTag>) -> Task {
    let mut t = make_task_with(None, tag);
    t.status = TaskStatus::Review;
    t.sub_status = sub_status;
    t.worktree = Some("/repo/.worktrees/1-task".to_string());
    t.tmux_window = Some(test_tmux_window("task-1"));
    t
}

/// A detached Review task: worktree present, tmux window gone, no url.
fn detached_review_task() -> Task {
    let mut t = review_task(SubStatus::AwaitingReview, None);
    t.tmux_window = None;
    t
}

/// The section-header label a task's card renders under, or the empty
/// string in a column with no sections.
fn task_header_label(task: &Task) -> &'static str {
    ColumnSection::for_task(task).map_or("", |s| s.header_label())
}

fn pr_url() -> Option<TaskUrl> {
    Some(TaskUrl::new(
        "https://github.com/org/repo/pull/10",
        UrlType::Pr,
    ))
}

/// A live Review task that has a PR — the common case for every section
/// except `parked`.
fn with_pr(sub_status: SubStatus, tag: Option<TaskTag>) -> Task {
    let mut t = review_task(sub_status, tag);
    t.url = pr_url();
    t
}

// --- parked ---

#[test]
fn detached_review_task_without_a_pr_is_parked() {
    let t = detached_review_task();
    assert_eq!(ColumnSection::for_task(&t), Some(ColumnSection::Parked));
    assert_eq!(task_header_label(&t), "parked");
    assert!(
        task_column_priority(&t) > SubStatus::AwaitingReview.column_priority(),
        "parked should sort below awaiting review"
    );
}

/// The awaiting-merge removal: once a PR exists, detach state stops
/// mattering and only the review decision does.
#[test]
fn detached_review_task_with_a_pr_is_plain_awaiting_review() {
    let mut t = detached_review_task();
    t.url = pr_url();
    assert_eq!(
        ColumnSection::for_task(&t),
        Some(ColumnSection::AwaitingReview)
    );
    assert_eq!(task_header_label(&t), "awaiting review");
    assert_eq!(
        task_column_priority(&t),
        SubStatus::AwaitingReview.column_priority()
    );
}

/// Only a *pr*-typed url takes a task out of parked — an issue or
/// security-alert link is not something that can be reviewed or merged.
#[test]
fn detached_review_task_with_a_non_pr_url_is_still_parked() {
    for ty in [UrlType::Issue, UrlType::SecurityAlert, UrlType::Other] {
        let mut t = detached_review_task();
        t.url = Some(TaskUrl::new("https://example.com/1", ty));
        assert_eq!(task_header_label(&t), "parked", "url_type {ty:?}");
    }
}

#[test]
fn live_review_task_without_a_pr_is_not_parked() {
    let t = review_task(SubStatus::AwaitingReview, None);
    assert_eq!(task_header_label(&t), "awaiting review");
}

#[test]
fn running_detached_task_is_not_parked() {
    let mut t = detached_review_task();
    t.status = TaskStatus::Running;
    t.sub_status = SubStatus::Active;
    assert_eq!(task_header_label(&t), SubStatus::Active.header_label());
}

#[test]
fn unprovisioned_review_task_is_not_parked() {
    let mut t = detached_review_task();
    t.worktree = None;
    assert_eq!(task_header_label(&t), "awaiting review");
}

/// Sub-status is deliberately not part of the parked condition: parked
/// means there is no PR at all, so it dominates every sub-status that could
/// only be about one — a conflict, or either review decision somehow
/// recorded without a PR.
#[test]
fn parked_wins_over_every_pr_bearing_sub_status() {
    for ss in [
        SubStatus::Conflict,
        SubStatus::ChangesRequested,
        SubStatus::Approved,
    ] {
        let mut t = detached_review_task();
        t.sub_status = ss;
        t.tag = Some(TaskTag::PrReview);
        assert_eq!(
            ColumnSection::for_task(&t),
            Some(ColumnSection::Parked),
            "{ss:?}"
        );
        assert_eq!(task_header_label(&t), "parked", "{ss:?}");
    }
}

/// A closed-without-merge PR still *has* a url, so it keeps its own
/// "pr closed" section rather than falling into parked when the agent
/// session ends.
#[test]
fn detached_pr_closed_task_is_not_parked() {
    let mut t = detached_review_task();
    t.sub_status = SubStatus::PrClosed;
    t.url = pr_url();
    assert_eq!(task_header_label(&t), SubStatus::PrClosed.header_label());
}

// --- review decisions the user made themselves ---

/// The two sub-statuses that record a review decision, and the section each
/// becomes on a task that reviews someone else's PR. One table so a third
/// decision is one row, not another pair of cloned tests.
const BY_ME: [(SubStatus, ColumnSection, &str); 2] = [
    (
        SubStatus::ChangesRequested,
        ColumnSection::ChangesRequestedByMe,
        "changes requested by me",
    ),
    (
        SubStatus::Approved,
        ColumnSection::ApprovedByMe,
        "approved by me",
    ),
];

#[test]
fn a_review_tagged_decision_is_by_me() {
    for (ss, section, label) in BY_ME {
        for tag in [
            TaskTag::PrReview,
            TaskTag::Dependabot,
            TaskTag::ReleasePlease,
        ] {
            let t = with_pr(ss, Some(tag));
            assert_eq!(ColumnSection::for_task(&t), Some(section), "{ss:?}/{tag:?}");
            assert_eq!(task_header_label(&t), label, "{ss:?}/{tag:?}");
            assert!(
                task_column_priority(&t) > task_column_priority(&detached_review_task()),
                "{ss:?}/{tag:?} should sort below parked"
            );
        }
    }
}

#[test]
fn a_non_review_tagged_decision_keeps_the_model_label() {
    for (ss, _, _) in BY_ME {
        for tag in [None, Some(TaskTag::Feature), Some(TaskTag::Bug)] {
            let t = with_pr(ss, tag);
            assert_eq!(
                ColumnSection::for_task(&t),
                ss.column_section(),
                "{ss:?}/{tag:?}"
            );
            assert_eq!(task_header_label(&t), ss.header_label(), "{ss:?}/{tag:?}");
            assert_eq!(
                task_column_priority(&t),
                ss.column_priority(),
                "{ss:?}/{tag:?}"
            );
        }
    }
}

/// The override is scoped to the two review *decisions*: a review-tagged
/// task in any other sub-status keeps the model's own label.
#[test]
fn review_tag_does_not_affect_other_sub_statuses() {
    for &ss in SubStatus::ALL {
        if BY_ME.iter().any(|&(decision, _, _)| decision == ss) {
            continue;
        }
        let t = with_pr(ss, Some(TaskTag::PrReview));
        assert_eq!(task_header_label(&t), ss.header_label(), "{ss:?}");
        assert_eq!(task_column_priority(&t), ss.column_priority(), "{ss:?}");
    }
}

// --- ordering ---

/// The full Review-column section order, asserted end to end.
#[test]
fn review_section_order_is_urgent_first() {
    let sections = [
        with_pr(SubStatus::Conflict, None),
        with_pr(SubStatus::PrClosed, None),
        with_pr(SubStatus::ChangesRequested, None),
        with_pr(SubStatus::Approved, None),
        with_pr(SubStatus::AwaitingReview, None),
        detached_review_task(),
        with_pr(SubStatus::ChangesRequested, Some(TaskTag::PrReview)),
        with_pr(SubStatus::Approved, Some(TaskTag::PrReview)),
    ];

    for pair in sections.windows(2) {
        let (above, below) = (&pair[0], &pair[1]);
        assert!(
            task_column_priority(above) < task_column_priority(below),
            "{:?} should sort above {:?}",
            task_header_label(above),
            task_header_label(below)
        );
    }
}
