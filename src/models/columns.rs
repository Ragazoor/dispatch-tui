//! [`ColumnSection`] — the identity of one sub-status section within a board
//! column, and the single table every section label and sort slot is read from.

use serde::Deserialize;

use super::{SubStatus, Task, TaskStatus};
use crate::define_str_enum;

// ---------------------------------------------------------------------------
// ColumnSection
// ---------------------------------------------------------------------------

/// The identity of one sub-status section within a board column — the thing a
/// section header names, and the thing a user's decision to collapse a section
/// is recorded against.
///
/// Deliberately not the same set as [`SubStatus`]. Three values are derived
/// from the task row at render time and never persisted (see [`Self::for_task`]),
/// and `Stale` holds two sub-statuses at once. Giving a section an identity of
/// its own is what makes its header label stable: grouping by priority slot
/// alone lets `Stale` and `StaleShell` take turns naming the same section.
///
/// See "Column Sections" in `docs/specs/core.allium`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnSection {
    Conflict,
    PrClosed,
    PrUnreachable,
    Crashed,
    Stale,
    NeedsInput,
    ChangesRequested,
    Approved,
    Active,
    AwaitingReview,
    Parked,
    ChangesRequestedByMe,
    ApprovedByMe,
}

impl ColumnSection {
    /// Every section, in render order: the first entry sits at the top of its
    /// column. Sections belonging to different columns never interleave, so
    /// `Active` (Running) and `AwaitingReview` (Review) sharing a slot costs
    /// nothing. `column_section_tests` pins the order against
    /// [`Self::column_priority`], so a variant inserted in the wrong place
    /// fails rather than rendering under the neighbouring header.
    pub const ALL: &'static [ColumnSection] = &[
        ColumnSection::Conflict,
        ColumnSection::PrClosed,
        ColumnSection::PrUnreachable,
        ColumnSection::Crashed,
        ColumnSection::Stale,
        ColumnSection::NeedsInput,
        ColumnSection::ChangesRequested,
        ColumnSection::Approved,
        ColumnSection::Active,
        ColumnSection::AwaitingReview,
        ColumnSection::Parked,
        ColumnSection::ChangesRequestedByMe,
        ColumnSection::ApprovedByMe,
    ];

    /// The section a task's card sits in, or `None` in a column that has no
    /// sections (Backlog and Done, whose cards all hold `SubStatus::None`).
    ///
    /// A derived section wins over the task's own sub-status. `Parked` is
    /// tested first: it means there is no PR at all, which dominates either
    /// review decision that was somehow recorded without one.
    pub fn for_task(task: &Task) -> Option<Self> {
        let has_pr = task.url.as_ref().is_some_and(|u| u.is_pr());
        if task.status == TaskStatus::Review && task.is_detached() && !has_pr {
            return Some(Self::Parked);
        }
        if task.tag.is_some_and(|t| t.is_review()) {
            // The two sub-statuses that record a review *decision*. On a task
            // that reviews someone else's PR, that decision was the user's own.
            match task.sub_status {
                SubStatus::ChangesRequested => return Some(Self::ChangesRequestedByMe),
                SubStatus::Approved => return Some(Self::ApprovedByMe),
                _ => {}
            }
        }
        task.sub_status.column_section()
    }

    /// Sort priority for column grouping (lower = more urgent = top of column).
    pub const fn column_priority(self) -> u8 {
        self.properties().priority
    }

    /// Label for the section header line within a column.
    pub const fn header_label(self) -> &'static str {
        self.properties().header_label
    }

    /// Per-variant display properties in a single match — the one table every
    /// section label and sort slot comes from. `SubStatus` and `EpicSubstatus`
    /// delegate here rather than keeping parallel tables that can drift.
    const fn properties(self) -> ColumnSectionProperties {
        match self {
            Self::Conflict => ColumnSectionProperties {
                priority: PRIORITY_URGENT,
                header_label: "conflict",
            },
            Self::PrClosed => ColumnSectionProperties {
                priority: PRIORITY_PR_CLOSED,
                header_label: "pr closed",
            },
            // Sorts below PrClosed and above ChangesRequested: an unreadable PR
            // leaves the card's review state unknown, which needs the user more
            // than a known outstanding task does.
            Self::PrUnreachable => ColumnSectionProperties {
                priority: PRIORITY_PR_UNREACHABLE,
                header_label: "pr unreachable",
            },
            Self::Crashed => ColumnSectionProperties {
                priority: PRIORITY_CRASHED,
                header_label: "crashed",
            },
            // Holds both `Stale` and `StaleShell`: the two say "this task looks
            // idle" for a different structural reason (no tool-use timestamp
            // vs. a shell that has been live unusually long), and the user acts
            // on either the same way.
            Self::Stale => ColumnSectionProperties {
                priority: PRIORITY_STALE,
                header_label: "stale",
            },
            Self::NeedsInput => ColumnSectionProperties {
                priority: PRIORITY_NEEDS_INPUT,
                header_label: "needs input",
            },
            Self::ChangesRequested => ColumnSectionProperties {
                priority: PRIORITY_CHANGES_REQUESTED,
                header_label: "changes requested",
            },
            // An approved PR is one keystroke from merging, so it outranks a PR
            // that is merely awaiting a decision and needs nothing from anyone.
            Self::Approved => ColumnSectionProperties {
                priority: PRIORITY_APPROVED,
                header_label: "approved",
            },
            // Active and AwaitingReview share a sort slot: neither signals
            // urgency the way Conflict/Crashed/Stale do, and they belong to
            // different columns so the tie is never observable.
            Self::Active => ColumnSectionProperties {
                priority: PRIORITY_ACTIVE_SLOT,
                header_label: "active",
            },
            Self::AwaitingReview => ColumnSectionProperties {
                priority: PRIORITY_ACTIVE_SLOT,
                header_label: "awaiting review",
            },
            Self::Parked => ColumnSectionProperties {
                priority: PRIORITY_PARKED,
                header_label: "parked",
            },
            Self::ChangesRequestedByMe => ColumnSectionProperties {
                priority: PRIORITY_CHANGES_REQUESTED_BY_ME,
                header_label: "changes requested by me",
            },
            Self::ApprovedByMe => ColumnSectionProperties {
                priority: PRIORITY_APPROVED_BY_ME,
                header_label: "approved by me",
            },
        }
    }
}

define_str_enum!(ColumnSection, "column-section" {
    Conflict => "conflict",
    PrClosed => "pr_closed",
    PrUnreachable => "pr_unreachable",
    Crashed => "crashed",
    Stale => "stale",
    NeedsInput => "needs_input",
    ChangesRequested => "changes_requested",
    Approved => "approved",
    Active => "active",
    AwaitingReview => "awaiting_review",
    Parked => "parked",
    ChangesRequestedByMe => "changes_requested_by_me",
    ApprovedByMe => "approved_by_me",
});

/// Per-variant properties returned by [`ColumnSection::properties`].
struct ColumnSectionProperties {
    priority: u8,
    header_label: &'static str,
}

// Column-priority sort slots (lower = more urgent = top of column). Gaps are
// intentional: they leave room to insert a new tier without renumbering the
// slots around it and without colliding with a named slot here.
const PRIORITY_URGENT: u8 = 0;
// PrClosed sorts right after Conflict (Review-only; never coexists with the
// Running-only tiers below, but still gets its own number so it doesn't
// silently share a header group with any of them).
const PRIORITY_PR_CLOSED: u8 = 5;
// Review-only, like PrClosed, and sorts directly below it.
const PRIORITY_PR_UNREACHABLE: u8 = 7;
const PRIORITY_CRASHED: u8 = 10;
const PRIORITY_STALE: u8 = 20;
const PRIORITY_NEEDS_INPUT: u8 = 30;
const PRIORITY_CHANGES_REQUESTED: u8 = 40;
const PRIORITY_APPROVED: u8 = 45;
const PRIORITY_ACTIVE_SLOT: u8 = 50;

// The three derived sections. All sit under every named sub-status slot — none
// of them is waiting on the user — and all are derived from the slot above
// rather than hardcoded, so inserting a new tier can't silently desync them.
const PRIORITY_PARKED: u8 = PRIORITY_ACTIVE_SLOT + 1;
const PRIORITY_CHANGES_REQUESTED_BY_ME: u8 = PRIORITY_PARKED + 1;
const PRIORITY_APPROVED_BY_ME: u8 = PRIORITY_CHANGES_REQUESTED_BY_ME + 1;

/// The sort slot for a card in a column that has no sections. Shares the
/// active slot's number, which it did before sections owned the table. Never
/// observable as a tie: `SubStatus::None` is only valid for Backlog, Done and
/// Archived, none of which holds a sectioned card.
const PRIORITY_NO_SECTION: u8 = PRIORITY_ACTIVE_SLOT;

/// Column sort priority for an optional section — the one home for "what does
/// a card with no section sort as".
///
/// Every sort that orders cards by section goes through here, so the answer
/// cannot be spelled differently in the model and in the board.
pub const fn section_sort_priority(section: Option<ColumnSection>) -> u8 {
    match section {
        Some(section) => section.column_priority(),
        None => PRIORITY_NO_SECTION,
    }
}

/// Column sort priority for a task: its section's slot, or the no-section slot
/// in a column that has none.
///
/// Test-only. The board reaches the same answer inline, off the section it has
/// already resolved for grouping; this spelling survives for the ordering
/// assertions below, which are about `ColumnSection::for_task` rather than
/// about any one caller. Its last production caller was the column builder for
/// the abandoned 8-column layout (see `core.allium`, "Board Columns").
#[cfg(test)]
pub fn task_column_priority(task: &Task) -> u8 {
    section_sort_priority(ColumnSection::for_task(task))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod column_section_for_task_tests {
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
            for tag in [TaskTag::PrReview, TaskTag::Dependabot] {
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
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod column_section_tests {
    use super::*;
    use crate::models::EpicSubstatus;

    /// Every (status, sub_status) pair the model admits names exactly one
    /// section, except `none` — the sub-status of the columns that have no
    /// sections at all.
    ///
    /// This total-coverage claim used to be made a second time against the
    /// 8-column VisualColumn table, which no render path ever built (see
    /// `core.allium`, "Board Columns"). That table is gone; this is the one
    /// home for the claim, so it walks Archived too rather than just the four
    /// board columns.
    #[test]
    fn every_valid_sub_status_names_one_section() {
        for &status in TaskStatus::ALL_INCLUDING_ARCHIVED {
            for &ss in SubStatus::ALL {
                if !ss.is_valid_for(status) {
                    continue;
                }
                let section = ss.column_section();
                if ss == SubStatus::None {
                    assert_eq!(section, None, "{ss:?}/{status:?}");
                } else {
                    assert!(section.is_some(), "{ss:?}/{status:?} has no section");
                }
            }
        }
    }

    /// The header label is the section's own, not the first card's. Today's
    /// grouping puts stale and shell-stale in one group but lets either name
    /// it, so the same section can read "stale" or "shell stale".
    #[test]
    fn stale_and_shell_stale_are_one_section_named_stale() {
        assert_eq!(
            SubStatus::Stale.column_section(),
            Some(ColumnSection::Stale)
        );
        assert_eq!(
            SubStatus::StaleShell.column_section(),
            Some(ColumnSection::Stale)
        );
        assert_eq!(ColumnSection::Stale.header_label(), "stale");
    }

    /// Declaration order is render order, so `ALL` must be sorted by the
    /// priority the sort actually uses. A variant inserted in the wrong place
    /// would render under a header it does not belong to.
    #[test]
    fn all_is_ordered_by_ascending_priority() {
        for pair in ColumnSection::ALL.windows(2) {
            assert!(
                pair[0].column_priority() <= pair[1].column_priority(),
                "{:?} is listed above {:?} but sorts below it",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn all_holds_every_variant_once() {
        for &s in ColumnSection::ALL {
            assert_eq!(
                ColumnSection::ALL.iter().filter(|&&x| x == s).count(),
                1,
                "{s:?}"
            );
        }
    }

    /// Pinned so the consolidation cannot reorder the board. These are the
    /// numbers `SubStatus::column_priority` and the derived sections returned
    /// before `ColumnSection` owned the table.
    #[test]
    fn section_priorities_are_todays_numbers() {
        let expected = [
            (ColumnSection::Conflict, 0u8),
            (ColumnSection::PrClosed, 5),
            (ColumnSection::PrUnreachable, 7),
            (ColumnSection::Crashed, 10),
            (ColumnSection::Stale, 20),
            (ColumnSection::NeedsInput, 30),
            (ColumnSection::ChangesRequested, 40),
            (ColumnSection::Approved, 45),
            (ColumnSection::Active, 50),
            (ColumnSection::AwaitingReview, 50),
            (ColumnSection::Parked, 51),
            (ColumnSection::ChangesRequestedByMe, 52),
            (ColumnSection::ApprovedByMe, 53),
        ];
        for (section, priority) in expected {
            assert_eq!(section.column_priority(), priority, "{section:?}");
        }
    }

    /// Sections are persisted by name, so every variant must survive the round
    /// trip. A variant that does not is a fold the user cannot keep.
    #[test]
    fn every_section_round_trips_through_its_string_form() {
        for &s in ColumnSection::ALL {
            let text = s.as_str();
            assert_eq!(text.parse::<ColumnSection>().unwrap(), s, "{text}");
        }
    }

    /// Epic cards group under the same headers their tasks do, so an epic and
    /// a task in the same state sit together rather than the epic floating
    /// above every header.
    #[test]
    fn epic_substatuses_map_onto_task_sections() {
        assert_eq!(
            EpicSubstatus::Blocked(2).column_section(),
            Some(ColumnSection::NeedsInput)
        );
        assert_eq!(
            EpicSubstatus::Active.column_section(),
            Some(ColumnSection::Active)
        );
        assert_eq!(
            EpicSubstatus::InReview.column_section(),
            Some(ColumnSection::AwaitingReview)
        );
        for s in [
            EpicSubstatus::Unplanned,
            EpicSubstatus::Planned,
            EpicSubstatus::Done,
        ] {
            assert_eq!(s.column_section(), None, "{s:?}");
        }
    }

    /// The label and priority a card renders under come from its section, so
    /// the two old accessors must agree with it wherever a section exists.
    #[test]
    fn sub_status_accessors_agree_with_the_section_table() {
        for &ss in SubStatus::ALL {
            if let Some(section) = ss.column_section() {
                assert_eq!(ss.header_label(), section.header_label(), "{ss:?}");
                assert_eq!(ss.column_priority(), section.column_priority(), "{ss:?}");
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod sectioned_columns_tests {
    use super::*;

    /// Exactly Running and Review have sections
    /// (`core.allium`: ColumnSectionLayout.sectioned_statuses). Derived from
    /// the section mapping rather than restated as its own predicate, so the
    /// claim cannot drift away from the thing that decides it.
    #[test]
    fn only_running_and_review_have_sections() {
        for &status in TaskStatus::ALL_INCLUDING_ARCHIVED {
            let has_sections = SubStatus::ALL
                .iter()
                .any(|ss| ss.is_valid_for(status) && ss.column_section().is_some());
            assert_eq!(
                has_sections,
                matches!(status, TaskStatus::Running | TaskStatus::Review),
                "{status:?}"
            );
        }
    }

    /// Sections are grouped by identity but *sorted* by number, so two
    /// sections reachable in the same column must not share a priority. If two
    /// did, the sort would interleave their cards and the grouping would emit
    /// the section's header more than once — each copy folding with only part
    /// of the section's cards.
    ///
    /// `Active` and `AwaitingReview` do share a slot, which is why this is
    /// scoped per column rather than globally: they belong to Running and
    /// Review respectively and never meet.
    #[test]
    fn no_two_sections_in_one_column_share_a_priority() {
        for &status in TaskStatus::ALL {
            let mut sections: Vec<ColumnSection> = SubStatus::ALL
                .iter()
                .filter(|ss| ss.is_valid_for(status))
                .filter_map(|ss| ss.column_section())
                .collect();
            // The derived sections are Review-only and reachable there
            // regardless of sub-status (see `ColumnSection::for_task`).
            if status == TaskStatus::Review {
                sections.extend([
                    ColumnSection::Parked,
                    ColumnSection::ChangesRequestedByMe,
                    ColumnSection::ApprovedByMe,
                ]);
            }
            for a in &sections {
                for b in &sections {
                    if a == b {
                        continue;
                    }
                    assert_ne!(
                        a.column_priority(),
                        b.column_priority(),
                        "{a:?} and {b:?} share a priority and both occur in {status:?}"
                    );
                }
            }
        }
    }

    /// The sectioned set and the flatten-exempt set name complementary pairs
    /// today but answer different questions — one is about epic grouping, the
    /// other about section grouping. Pinned apart so a change to one is not
    /// quietly assumed to change the other.
    #[test]
    fn the_sectioned_and_unflattened_sets_are_answered_separately() {
        for &status in TaskStatus::ALL {
            let has_sections = SubStatus::ALL
                .iter()
                .any(|ss| ss.is_valid_for(status) && ss.column_section().is_some());
            assert_ne!(
                has_sections,
                status.is_unflattened(),
                "{status:?} — if these ever coincide by design, say so here"
            );
        }
    }
}
