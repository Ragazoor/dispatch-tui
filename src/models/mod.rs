// ---------------------------------------------------------------------------
// Path utilities
// ---------------------------------------------------------------------------

/// Expand a leading `~` or `~/` to the user's home directory.
/// Returns the path unchanged if it doesn't start with `~` or `$HOME` is unset.
pub fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return format!("{}/{rest}", home.to_string_lossy());
        }
    } else if path == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return home.to_string_lossy().into_owned();
        }
    }
    path.to_string()
}

// ---------------------------------------------------------------------------
// ID newtypes
// ---------------------------------------------------------------------------

/// Generate a zero-cost i64 newtype with Display, From/Into<i64>, FromStr,
/// Serialize/Deserialize, and basic unit tests.
macro_rules! define_id_newtype {
    ($(#[$attr:meta])* $name:ident, $test_mod:ident) => {
        $(#[$attr])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(pub i64);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<i64> for $name {
            fn from(v: i64) -> Self {
                $name(v)
            }
        }

        impl From<$name> for i64 {
            fn from(id: $name) -> Self {
                id.0
            }
        }

        impl std::str::FromStr for $name {
            type Err = std::num::ParseIntError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                s.parse::<i64>().map($name)
            }
        }

        #[cfg(test)]
        mod $test_mod {
            use super::$name;

            #[test]
            fn display() {
                assert_eq!($name(42).to_string(), "42");
            }

            #[test]
            fn copy_eq_hash() {
                let a = $name(1);
                let b = a;
                assert_eq!(a, b);
                let mut set = std::collections::HashSet::new();
                set.insert(a);
                assert!(set.contains(&b));
            }

            #[test]
            fn debug_contains_value() {
                assert!(format!("{:?}", $name(7)).contains("7"));
            }

            #[test]
            fn from_into_i64() {
                let id = $name::from(5i64);
                let raw: i64 = id.into();
                assert_eq!(raw, 5);
            }
        }
    };
}

pub mod projects;
pub use projects::*;

pub mod learnings;
pub use learnings::*;

pub mod tasks;
pub use tasks::*;

pub mod epics;
pub use epics::*;

pub mod review;
pub use review::*;

// ---------------------------------------------------------------------------
// VisualColumn — the 8 visual columns for the kanban board
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct VisualColumn {
    pub label: &'static str,
    pub parent_status: TaskStatus,
    pub sub_statuses: &'static [SubStatus],
}

impl VisualColumn {
    pub const COUNT: usize = 8;
    pub const ALL: &'static [VisualColumn] = &[
        VisualColumn {
            label: "Backlog",
            parent_status: TaskStatus::Backlog,
            sub_statuses: &[SubStatus::None],
        },
        VisualColumn {
            label: "Active",
            parent_status: TaskStatus::Running,
            sub_statuses: &[SubStatus::Active],
        },
        VisualColumn {
            label: "Blocked",
            parent_status: TaskStatus::Running,
            sub_statuses: &[SubStatus::NeedsInput],
        },
        VisualColumn {
            label: "Stale",
            parent_status: TaskStatus::Running,
            sub_statuses: &[SubStatus::Stale, SubStatus::Crashed, SubStatus::Conflict],
        },
        VisualColumn {
            label: "PR Created",
            parent_status: TaskStatus::Review,
            sub_statuses: &[SubStatus::AwaitingReview, SubStatus::Conflict],
        },
        VisualColumn {
            label: "Revise",
            parent_status: TaskStatus::Review,
            sub_statuses: &[SubStatus::ChangesRequested],
        },
        VisualColumn {
            label: "Approved",
            parent_status: TaskStatus::Review,
            sub_statuses: &[SubStatus::Approved],
        },
        VisualColumn {
            label: "Done",
            parent_status: TaskStatus::Done,
            sub_statuses: &[SubStatus::None],
        },
    ];

    pub fn contains(&self, sub_status: SubStatus) -> bool {
        self.sub_statuses.contains(&sub_status)
    }

    pub fn parent_group_start(status: TaskStatus) -> usize {
        Self::ALL
            .iter()
            .position(|vc| vc.parent_status == status)
            .unwrap_or(0)
    }

    pub fn parent_group_span(status: TaskStatus) -> usize {
        Self::ALL
            .iter()
            .filter(|vc| vc.parent_status == status)
            .count()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use chrono::Utc;
    use proptest::prelude::*;

    // --- expand_tilde ---

    #[test]
    fn expand_tilde_with_path() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            expand_tilde("~/projects/foo"),
            format!("{home}/projects/foo")
        );
    }

    #[test]
    fn expand_tilde_bare() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(expand_tilde("~"), home);
    }

    #[test]
    fn expand_tilde_absolute_unchanged() {
        assert_eq!(expand_tilde("/home/user/foo"), "/home/user/foo");
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
    fn archived_column_index_is_column_count() {
        assert_eq!(
            TaskStatus::Archived.column_index(),
            TaskStatus::COLUMN_COUNT
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

        // Review: AwaitingReview, ChangesRequested, Approved
        assert!(!SubStatus::None.is_valid_for(TaskStatus::Review));
        assert!(!SubStatus::Active.is_valid_for(TaskStatus::Review));
        assert!(SubStatus::AwaitingReview.is_valid_for(TaskStatus::Review));
        assert!(SubStatus::ChangesRequested.is_valid_for(TaskStatus::Review));
        assert!(SubStatus::Approved.is_valid_for(TaskStatus::Review));

        // Done: only None
        assert!(SubStatus::None.is_valid_for(TaskStatus::Done));
        assert!(!SubStatus::Active.is_valid_for(TaskStatus::Done));

        // Archived: only None
        assert!(SubStatus::None.is_valid_for(TaskStatus::Archived));
        assert!(!SubStatus::Active.is_valid_for(TaskStatus::Archived));
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
        assert_eq!(
            SubStatus::default_for(TaskStatus::Archived),
            SubStatus::None
        );
    }

    #[test]
    fn substatus_rules_consistent_with_visual_columns() {
        // Every SubStatus in a VisualColumn must be valid for that column's parent_status
        for vc in VisualColumn::ALL {
            for &sub in vc.sub_statuses {
                assert!(
                    sub.is_valid_for(vc.parent_status),
                    "{sub:?} in column {:?} but not valid for {:?}",
                    vc.label,
                    vc.parent_status
                );
            }
        }
        // Every valid (status, substatus) pair must appear in exactly one VisualColumn
        for &status in TaskStatus::ALL {
            for &sub in SubStatus::ALL {
                if sub.is_valid_for(status) {
                    let count = VisualColumn::ALL
                        .iter()
                        .filter(|vc| vc.parent_status == status && vc.contains(sub))
                        .count();
                    assert_eq!(
                        count, 1,
                        "{sub:?}/{status:?} is valid but appears in {count} VisualColumns"
                    );
                }
            }
        }
        // default_for() must be valid
        for &status in TaskStatus::ALL {
            let default = SubStatus::default_for(status);
            assert!(
                default.is_valid_for(status),
                "default_for({status:?}) = {default:?} is not valid"
            );
        }
    }

    // --- VisualColumn ---

    #[test]
    fn visual_columns_count_is_8() {
        assert_eq!(VisualColumn::ALL.len(), 8);
        assert_eq!(VisualColumn::COUNT, 8);
        assert_eq!(VisualColumn::ALL.len(), VisualColumn::COUNT);
    }

    #[test]
    fn visual_column_parent_status_mapping() {
        assert_eq!(VisualColumn::ALL[0].parent_status, TaskStatus::Backlog);
        assert_eq!(VisualColumn::ALL[1].parent_status, TaskStatus::Running);
        assert_eq!(VisualColumn::ALL[2].parent_status, TaskStatus::Running);
        assert_eq!(VisualColumn::ALL[3].parent_status, TaskStatus::Running);
        assert_eq!(VisualColumn::ALL[4].parent_status, TaskStatus::Review);
        assert_eq!(VisualColumn::ALL[5].parent_status, TaskStatus::Review);
        assert_eq!(VisualColumn::ALL[6].parent_status, TaskStatus::Review);
        assert_eq!(VisualColumn::ALL[7].parent_status, TaskStatus::Done);
    }

    #[test]
    fn visual_column_contains_substatus() {
        // Column 3 ("Stale") contains Stale and Crashed, but not Active
        let stale_col = &VisualColumn::ALL[3];
        assert!(stale_col.contains(SubStatus::Stale));
        assert!(stale_col.contains(SubStatus::Crashed));
        assert!(!stale_col.contains(SubStatus::Active));
    }

    #[test]
    fn visual_column_parent_group_start() {
        assert_eq!(VisualColumn::parent_group_start(TaskStatus::Backlog), 0);
        assert_eq!(VisualColumn::parent_group_start(TaskStatus::Running), 1);
        assert_eq!(VisualColumn::parent_group_start(TaskStatus::Review), 4);
        assert_eq!(VisualColumn::parent_group_start(TaskStatus::Done), 7);
    }

    #[test]
    fn visual_column_parent_group_span() {
        assert_eq!(VisualColumn::parent_group_span(TaskStatus::Backlog), 1);
        assert_eq!(VisualColumn::parent_group_span(TaskStatus::Running), 3);
        assert_eq!(VisualColumn::parent_group_span(TaskStatus::Review), 3);
        assert_eq!(VisualColumn::parent_group_span(TaskStatus::Done), 1);
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

    proptest! {
        #[test]
        fn slugify_never_panics(input in "\\PC{0,2000}") {
            // slugify should never panic on arbitrary input
            let _ = slugify(&input);
        }
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

    #[test]
    fn status_archived_roundtrip() {
        let s = TaskStatus::Archived.as_str();
        assert_eq!(s, "archived");
        let parsed = TaskStatus::parse(s).expect("roundtrip failed");
        assert_eq!(parsed, TaskStatus::Archived);
    }

    #[test]
    fn status_archived_is_terminal() {
        assert_eq!(TaskStatus::Archived.next(), TaskStatus::Archived);
        assert_eq!(TaskStatus::Archived.prev(), TaskStatus::Archived);
    }

    #[test]
    fn status_archived_has_no_column() {
        // Archived is not a kanban column — COLUMN_COUNT stays 4
        assert_eq!(TaskStatus::COLUMN_COUNT, 4);
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

    // --- EpicId ---

    #[test]
    fn epic_id_display() {
        let id = EpicId(42);
        assert_eq!(format!("{id}"), "42");
    }

    #[test]
    fn epic_id_equality() {
        assert_eq!(EpicId(1), EpicId(1));
        assert_ne!(EpicId(1), EpicId(2));
    }

    #[test]
    fn task_epic_id_defaults_to_none() {
        let now = Utc::now();
        let task = Task {
            id: TaskId(1),
            title: "Test".to_string(),
            description: "Desc".to_string(),
            repo_path: "/repo".to_string(),
            status: TaskStatus::Backlog,
            worktree: None,
            tmux_window: None,
            plan_path: None,
            epic_id: None,
            sub_status: SubStatus::None,
            pr_url: None,
            tag: None,
            sort_order: None,
            base_branch: "main".to_string(),
            external_id: None,
            labels: Vec::new(),
            created_at: now,
            updated_at: now,
            project_id: ProjectId(1),
        };
        assert!(task.epic_id.is_none());
    }

    #[test]
    fn task_with_epic_id() {
        let now = Utc::now();
        let task = Task {
            id: TaskId(1),
            title: "Test".to_string(),
            description: "Desc".to_string(),
            repo_path: "/repo".to_string(),
            status: TaskStatus::Backlog,
            worktree: None,
            tmux_window: None,
            plan_path: None,
            epic_id: Some(EpicId(5)),
            sub_status: SubStatus::None,
            pr_url: None,
            tag: None,
            sort_order: None,
            base_branch: "main".to_string(),
            external_id: None,
            labels: Vec::new(),
            created_at: now,
            updated_at: now,
            project_id: ProjectId(1),
        };
        assert_eq!(task.epic_id, Some(EpicId(5)));
    }

    #[test]
    fn epic_struct_fields() {
        let now = Utc::now();
        let epic = Epic {
            id: EpicId(1),
            title: "Auth Rewrite".to_string(),
            description: "Rewrite auth system".to_string(),
            repo_path: "/repo".to_string(),
            status: TaskStatus::Backlog,
            plan_path: None,
            sort_order: None,
            auto_dispatch: true,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            created_at: now,
            updated_at: now,
            project_id: ProjectId(1),
        };
        assert_eq!(epic.id, EpicId(1));
        assert_eq!(epic.status, TaskStatus::Backlog);
    }

    // --- Epic.status direct access ---
    // epic_status() was a wrapper that once derived status from subtasks.
    // It was deleted; callers should access epic.status directly.

    fn make_epic_for_status(status: TaskStatus) -> Epic {
        Epic {
            id: EpicId(1),
            title: String::new(),
            description: String::new(),
            repo_path: String::new(),
            status,
            plan_path: None,
            sort_order: None,
            auto_dispatch: true,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            project_id: ProjectId(1),
        }
    }

    #[test]
    fn epic_has_status_field_directly_accessible() {
        // Regression guard: epic.status is public and accessible directly.
        // Previously callers used epic_status(&epic) — that wrapper no longer exists.
        let epic = make_epic_for_status(TaskStatus::Done);
        assert_eq!(epic.status, TaskStatus::Done);

        let epic = make_epic_for_status(TaskStatus::Backlog);
        assert_eq!(epic.status, TaskStatus::Backlog);

        let epic = make_epic_for_status(TaskStatus::Running);
        assert_eq!(epic.status, TaskStatus::Running);

        let epic = make_epic_for_status(TaskStatus::Review);
        assert_eq!(epic.status, TaskStatus::Review);
    }

    // --- ReviewDecision ---

    #[test]
    fn review_decision_column_count() {
        assert_eq!(ReviewDecision::COLUMN_COUNT, 4);
        assert_eq!(ReviewDecision::ALL.len(), 4);
    }

    #[test]
    fn review_decision_column_index_roundtrip() {
        for (i, decision) in ReviewDecision::ALL.iter().enumerate() {
            assert_eq!(decision.column_index(), i);
        }
    }

    #[test]
    fn review_decision_from_column_index() {
        assert_eq!(
            ReviewDecision::from_column_index(0),
            Some(ReviewDecision::ReviewRequired)
        );
        assert_eq!(
            ReviewDecision::from_column_index(1),
            Some(ReviewDecision::WaitingForResponse)
        );
        assert_eq!(
            ReviewDecision::from_column_index(2),
            Some(ReviewDecision::ChangesRequested)
        );
        assert_eq!(
            ReviewDecision::from_column_index(3),
            Some(ReviewDecision::Approved)
        );
        assert_eq!(ReviewDecision::from_column_index(4), None);
    }

    #[test]
    fn review_decision_as_str() {
        assert_eq!(ReviewDecision::ReviewRequired.as_str(), "Needs Review");
        assert_eq!(
            ReviewDecision::ChangesRequested.as_str(),
            "Changes Requested"
        );
        assert_eq!(ReviewDecision::Approved.as_str(), "Approved");
    }

    #[test]
    fn review_decision_parse() {
        assert_eq!(
            ReviewDecision::parse("REVIEW_REQUIRED"),
            Some(ReviewDecision::ReviewRequired)
        );
        assert_eq!(
            ReviewDecision::parse("CHANGES_REQUESTED"),
            Some(ReviewDecision::ChangesRequested)
        );
        assert_eq!(
            ReviewDecision::parse("APPROVED"),
            Some(ReviewDecision::Approved)
        );
        assert_eq!(ReviewDecision::parse("bogus"), None);
        assert_eq!(ReviewDecision::parse(""), None);
    }

    // --- pr_number_from_url ---

    #[test]
    fn pr_number_from_standard_url() {
        assert_eq!(
            pr_number_from_url("https://github.com/org/repo/pull/42"),
            Some(42)
        );
    }

    #[test]
    fn pr_number_from_url_with_trailing_slash() {
        assert_eq!(
            pr_number_from_url("https://github.com/org/repo/pull/42/"),
            Some(42)
        );
    }

    #[test]
    fn pr_number_from_url_with_query_params() {
        assert_eq!(
            pr_number_from_url("https://github.com/org/repo/pull/42?diff=split"),
            Some(42)
        );
    }

    #[test]
    fn pr_number_from_url_no_number() {
        assert_eq!(pr_number_from_url("https://github.com/org/repo"), None);
    }

    #[test]
    fn pr_number_from_empty_url() {
        assert_eq!(pr_number_from_url(""), None);
    }

    #[test]
    fn pr_number_from_url_with_fragment() {
        assert_eq!(
            pr_number_from_url("https://github.com/org/repo/pull/42#issuecomment-123"),
            Some(42)
        );
    }

    // --- url_label ---

    #[test]
    fn url_label_github_pr() {
        assert_eq!(url_label("https://github.com/org/repo/pull/42"), "PR #42");
    }

    #[test]
    fn url_label_github_pr_no_number() {
        assert_eq!(url_label("https://github.com/org/repo/pull/"), "PR");
    }

    #[test]
    fn url_label_github_pr_with_query() {
        assert_eq!(
            url_label("https://github.com/org/repo/pull/42?diff=split"),
            "PR #42"
        );
    }

    #[test]
    fn url_label_github_pr_with_fragment() {
        assert_eq!(
            url_label("https://github.com/org/repo/pull/42#issuecomment-123"),
            "PR #42"
        );
    }

    #[test]
    fn url_label_github_issue() {
        assert_eq!(
            url_label("https://github.com/org/repo/issues/7"),
            "Issue #7"
        );
    }

    #[test]
    fn url_label_github_issue_no_number() {
        assert_eq!(url_label("https://github.com/org/repo/issues/"), "Issue");
    }

    #[test]
    fn url_label_github_issue_with_fragment() {
        assert_eq!(
            url_label("https://github.com/org/repo/issues/7#issuecomment-999"),
            "Issue #7"
        );
    }

    #[test]
    fn url_label_arbitrary_url() {
        assert_eq!(
            url_label("https://jira.example.com/browse/PROJ-123"),
            "Link"
        );
    }

    #[test]
    fn url_label_empty_string() {
        assert_eq!(url_label(""), "Link");
    }

    // --- github_repo_from_pr_url ---

    #[test]
    fn github_repo_from_standard_url() {
        assert_eq!(
            github_repo_from_pr_url("https://github.com/org/repo/pull/42"),
            Some("org/repo".to_string())
        );
    }

    #[test]
    fn github_repo_from_url_with_query_params() {
        assert_eq!(
            github_repo_from_pr_url("https://github.com/org/repo/pull/42?diff=split"),
            Some("org/repo".to_string())
        );
    }

    #[test]
    fn github_repo_from_url_with_fragment() {
        assert_eq!(
            github_repo_from_pr_url("https://github.com/org/repo/pull/42#issuecomment-123"),
            Some("org/repo".to_string())
        );
    }

    #[test]
    fn github_repo_from_non_github_url() {
        assert_eq!(
            github_repo_from_pr_url("https://gitlab.com/org/repo/pull/42"),
            None
        );
    }

    #[test]
    fn github_repo_from_empty_url() {
        assert_eq!(github_repo_from_pr_url(""), None);
    }

    #[test]
    fn github_repo_from_url_with_trailing_slash() {
        assert_eq!(
            github_repo_from_pr_url("https://github.com/org/repo/pull/42/"),
            Some("org/repo".to_string())
        );
    }

    #[test]
    fn review_decision_db_roundtrip() {
        for decision in ReviewDecision::ALL {
            let s = decision.as_db_str();
            let parsed = ReviewDecision::from_db_str(s)
                .unwrap_or_else(|| panic!("failed to parse db str: {s}"));
            assert_eq!(parsed, decision);
        }
    }

    #[test]
    fn ci_status_db_roundtrip() {
        for status in [
            CiStatus::Pending,
            CiStatus::Success,
            CiStatus::Failure,
            CiStatus::None,
        ] {
            let s = status.as_db_str();
            assert_eq!(CiStatus::from_db_str(s), status, "roundtrip failed for {s}");
        }
    }

    // --- EpicSubstatus ---

    fn test_epic() -> Epic {
        Epic {
            id: EpicId(1),
            title: "Test".to_string(),
            description: "".to_string(),
            repo_path: "/repo".to_string(),
            status: TaskStatus::Backlog,
            plan_path: None,
            sort_order: None,
            auto_dispatch: true,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            project_id: ProjectId(1),
        }
    }

    fn test_task() -> Task {
        Task {
            id: TaskId(1),
            title: "T".to_string(),
            description: "".to_string(),
            repo_path: "/repo".to_string(),
            status: TaskStatus::Backlog,
            sub_status: SubStatus::None,
            worktree: None,
            tmux_window: None,
            plan_path: None,
            epic_id: None,
            pr_url: None,
            tag: None,
            sort_order: None,
            base_branch: "main".to_string(),
            external_id: None,
            labels: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            project_id: ProjectId(1),
        }
    }

    #[test]
    fn epic_substatus_unplanned() {
        let epic = Epic {
            plan_path: None,
            status: TaskStatus::Backlog,
            ..test_epic()
        };
        assert_eq!(epic_substatus(&epic, &[], None), EpicSubstatus::Unplanned);
    }

    #[test]
    fn epic_substatus_planned() {
        let epic = Epic {
            plan_path: Some("plan.md".into()),
            status: TaskStatus::Backlog,
            ..test_epic()
        };
        assert_eq!(epic_substatus(&epic, &[], None), EpicSubstatus::Planned);
    }

    #[test]
    fn epic_substatus_active_with_backlog() {
        let epic = Epic {
            status: TaskStatus::Running,
            ..test_epic()
        };
        let subtasks = vec![
            Task {
                status: TaskStatus::Running,
                sub_status: SubStatus::Active,
                ..test_task()
            },
            Task {
                status: TaskStatus::Backlog,
                ..test_task()
            },
        ];
        assert_eq!(
            epic_substatus(&epic, &subtasks, None),
            EpicSubstatus::Active
        );
    }

    #[test]
    fn epic_substatus_active_all_running() {
        let epic = Epic {
            status: TaskStatus::Running,
            ..test_epic()
        };
        let subtasks = vec![
            Task {
                status: TaskStatus::Running,
                sub_status: SubStatus::Active,
                ..test_task()
            },
            Task {
                status: TaskStatus::Done,
                sub_status: SubStatus::None,
                ..test_task()
            },
        ];
        assert_eq!(
            epic_substatus(&epic, &subtasks, None),
            EpicSubstatus::Active
        );
    }

    #[test]
    fn epic_substatus_blocked_stale() {
        let epic = Epic {
            status: TaskStatus::Running,
            ..test_epic()
        };
        let subtasks = vec![
            Task {
                status: TaskStatus::Running,
                sub_status: SubStatus::Stale,
                ..test_task()
            },
            Task {
                status: TaskStatus::Backlog,
                ..test_task()
            },
        ];
        assert_eq!(
            epic_substatus(&epic, &subtasks, None),
            EpicSubstatus::Blocked(1)
        );
    }

    #[test]
    fn epic_substatus_blocked_needs_input() {
        let epic = Epic {
            status: TaskStatus::Running,
            ..test_epic()
        };
        let subtasks = vec![
            Task {
                status: TaskStatus::Running,
                sub_status: SubStatus::NeedsInput,
                ..test_task()
            },
            Task {
                status: TaskStatus::Running,
                sub_status: SubStatus::Active,
                ..test_task()
            },
        ];
        assert_eq!(
            epic_substatus(&epic, &subtasks, None),
            EpicSubstatus::Blocked(1)
        );
    }

    #[test]
    fn epic_substatus_blocked_count() {
        let epic = Epic {
            status: TaskStatus::Running,
            ..test_epic()
        };
        let subtasks = vec![
            Task {
                status: TaskStatus::Running,
                sub_status: SubStatus::NeedsInput,
                ..test_task()
            },
            Task {
                status: TaskStatus::Running,
                sub_status: SubStatus::Stale,
                ..test_task()
            },
            Task {
                status: TaskStatus::Running,
                sub_status: SubStatus::Active,
                ..test_task()
            },
        ];
        assert_eq!(
            epic_substatus(&epic, &subtasks, None),
            EpicSubstatus::Blocked(2)
        );
    }

    #[test]
    fn epic_substatus_in_review() {
        let epic = Epic {
            status: TaskStatus::Review,
            ..test_epic()
        };
        assert_eq!(epic_substatus(&epic, &[], None), EpicSubstatus::InReview);
    }

    #[test]
    fn epic_substatus_wrapping_up() {
        let epic = Epic {
            status: TaskStatus::Review,
            ..test_epic()
        };
        assert_eq!(
            epic_substatus(&epic, &[], Some(EpicId(1))),
            EpicSubstatus::WrappingUp
        );
    }

    #[test]
    fn epic_substatus_done() {
        let epic = Epic {
            status: TaskStatus::Done,
            ..test_epic()
        };
        assert_eq!(epic_substatus(&epic, &[], None), EpicSubstatus::Done);
    }

    // --- DispatchMode ---

    fn make_task_with(plan: Option<&str>, tag: Option<TaskTag>) -> Task {
        let now = chrono::Utc::now();
        Task {
            id: TaskId(1),
            title: String::new(),
            description: String::new(),
            repo_path: String::new(),
            status: TaskStatus::Backlog,
            worktree: None,
            tmux_window: None,
            plan_path: plan.map(String::from),
            epic_id: None,
            sub_status: SubStatus::None,
            pr_url: None,
            tag,
            sort_order: None,
            base_branch: "main".to_string(),
            external_id: None,
            labels: Vec::new(),
            created_at: now,
            updated_at: now,
            project_id: ProjectId(1),
        }
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

    // --- descendant_task_ids / descendant_epic_ids ---

    fn epic_with(id: i64, parent: Option<i64>) -> Epic {
        let now = Utc::now();
        Epic {
            id: EpicId(id),
            title: format!("Epic {id}"),
            description: String::new(),
            repo_path: "/repo".to_string(),
            status: TaskStatus::Backlog,
            plan_path: None,
            sort_order: None,
            auto_dispatch: false,
            parent_epic_id: parent.map(EpicId),
            feed_command: None,
            feed_interval_secs: None,
            created_at: now,
            updated_at: now,
            project_id: ProjectId(1),
        }
    }

    fn task_under(id: i64, epic: Option<i64>) -> Task {
        let now = Utc::now();
        Task {
            id: TaskId(id),
            title: format!("Task {id}"),
            description: String::new(),
            repo_path: "/repo".to_string(),
            status: TaskStatus::Backlog,
            worktree: None,
            tmux_window: None,
            plan_path: None,
            epic_id: epic.map(EpicId),
            sub_status: SubStatus::None,
            pr_url: None,
            tag: None,
            sort_order: None,
            base_branch: "main".to_string(),
            external_id: None,
            labels: Vec::new(),
            created_at: now,
            updated_at: now,
            project_id: ProjectId(1),
        }
    }

    #[test]
    fn descendant_task_ids_includes_direct_children() {
        let epics = vec![epic_with(1, None)];
        let tasks = vec![task_under(10, Some(1)), task_under(11, None)];
        let ids = descendant_task_ids(EpicId(1), &epics, &tasks);
        assert!(ids.contains(&TaskId(10)));
        assert!(!ids.contains(&TaskId(11)));
    }

    #[test]
    fn descendant_task_ids_is_recursive() {
        // root(1) -> mid(2) -> leaf(3)
        let epics = vec![
            epic_with(1, None),
            epic_with(2, Some(1)),
            epic_with(3, Some(2)),
        ];
        let tasks = vec![
            task_under(10, Some(1)),
            task_under(20, Some(2)),
            task_under(30, Some(3)),
        ];
        let ids = descendant_task_ids(EpicId(1), &epics, &tasks);
        assert!(ids.contains(&TaskId(10)));
        assert!(ids.contains(&TaskId(20)));
        assert!(ids.contains(&TaskId(30)));
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn descendant_task_ids_excludes_sibling_subtree() {
        // root_a(1) with child 10; root_b(2) with child 20
        let epics = vec![epic_with(1, None), epic_with(2, None)];
        let tasks = vec![task_under(10, Some(1)), task_under(20, Some(2))];
        let ids = descendant_task_ids(EpicId(1), &epics, &tasks);
        assert!(ids.contains(&TaskId(10)));
        assert!(!ids.contains(&TaskId(20)));
    }

    #[test]
    fn descendant_task_ids_is_cycle_safe() {
        // Malformed: epic 1 points to epic 2, epic 2 points back to epic 1.
        let epics = vec![epic_with(1, Some(2)), epic_with(2, Some(1))];
        let tasks = vec![task_under(10, Some(1)), task_under(20, Some(2))];
        // Must terminate. From root=1, descendants include {1, 2}, so both tasks.
        let ids = descendant_task_ids(EpicId(1), &epics, &tasks);
        assert!(ids.contains(&TaskId(10)));
        assert!(ids.contains(&TaskId(20)));
    }

    #[test]
    fn dispatch_mode_without_plan_routes_only_dedicated_tags() {
        for tag in [
            None,
            Some(TaskTag::Feature),
            Some(TaskTag::Bug),
            Some(TaskTag::Chore),
        ] {
            assert_eq!(
                DispatchMode::for_task(&make_task_with(None, tag)),
                DispatchMode::Dispatch,
                "tag {tag:?} should fall through to Dispatch"
            );
        }
        assert_eq!(
            DispatchMode::for_task(&make_task_with(None, Some(TaskTag::PrReview))),
            DispatchMode::PrReview
        );
        assert_eq!(
            DispatchMode::for_task(&make_task_with(None, Some(TaskTag::Research))),
            DispatchMode::Research
        );
        assert_eq!(
            DispatchMode::for_task(&make_task_with(None, Some(TaskTag::Fix))),
            DispatchMode::Fix
        );
    }

    mod property_tests {
        use super::*;

        const TASK_STATUSES: &[TaskStatus] = &[
            TaskStatus::Backlog,
            TaskStatus::Running,
            TaskStatus::Review,
            TaskStatus::Done,
            TaskStatus::Archived,
        ];

        const TASK_TAGS: &[TaskTag] = &[
            TaskTag::Bug,
            TaskTag::Feature,
            TaskTag::Chore,
            TaskTag::PrReview,
            TaskTag::Research,
            TaskTag::Fix,
        ];

        fn task_status_strategy() -> impl Strategy<Value = TaskStatus> {
            (0..TASK_STATUSES.len()).prop_map(|i| TASK_STATUSES[i])
        }

        fn task_tag_strategy() -> impl Strategy<Value = TaskTag> {
            (0..TASK_TAGS.len()).prop_map(|i| TASK_TAGS[i])
        }

        fn sub_status_strategy() -> impl Strategy<Value = SubStatus> {
            (0..SubStatus::ALL.len()).prop_map(|i| SubStatus::ALL[i])
        }

        proptest! {
            #[test]
            fn taskstatus_parse_roundtrip(idx in 0..TaskStatus::ALL.len()) {
                let status = TaskStatus::ALL[idx];
                let parsed = TaskStatus::parse(status.as_str());
                prop_assert_eq!(parsed, Some(status));
            }

            #[test]
            fn tasktag_parse_roundtrip(tag in task_tag_strategy()) {
                let parsed = TaskTag::parse(tag.as_str());
                prop_assert_eq!(parsed, Some(tag));
            }

            #[test]
            fn substatus_default_is_valid_for_status(status in task_status_strategy()) {
                let default_ss = SubStatus::default_for(status);
                prop_assert!(
                    default_ss.is_valid_for(status),
                    "default_for({:?}) = {:?} is not valid for that status",
                    status,
                    default_ss
                );
            }

            #[test]
            fn substatus_none_is_only_valid_for_terminal_statuses(ss in sub_status_strategy()) {
                // For Backlog, Done, and Archived only SubStatus::None is valid.
                // Running and Review require a specific active sub-status.
                for &terminal in &[TaskStatus::Backlog, TaskStatus::Done, TaskStatus::Archived] {
                    let valid = ss.is_valid_for(terminal);
                    let expected = matches!(ss, SubStatus::None);
                    prop_assert_eq!(valid, expected);
                }
            }

            #[test]
            fn substatus_column_priority_never_panics(ss in sub_status_strategy()) {
                // column_priority() is a pure exhaustive match — just confirm it always
                // returns a value for every variant.
                let _ = ss.column_priority();
            }
        }
    }

    #[test]
    fn default_base_branch_is_main() {
        assert_eq!(DEFAULT_BASE_BRANCH, "main");
    }
}
