use super::*;
use crate::models::{EpicBuilder, SubStatus, Task, TaskBuilder, TaskId, TaskStatus};
use chrono::Utc;

fn make_epic(id: i64, status: TaskStatus, plan_path: Option<&str>, parent: Option<i64>) -> Epic {
    EpicBuilder::new(id)
        .status(status)
        .plan(plan_path)
        .parent(parent)
        .build()
}

fn make_task(id: i64, status: TaskStatus, sub_status: SubStatus, epic: Option<i64>) -> Task {
    TaskBuilder::new(id)
        .status(status)
        .sub_status(sub_status)
        .epic(epic)
        .build()
}

#[test]
fn feed_role_roundtrips_kebab_case() {
    assert_eq!(
        serde_json::to_string(&FeedRole::MyReviews).unwrap(),
        "\"my-reviews\""
    );
    assert_eq!(
        serde_json::from_str::<FeedRole>("\"reviews-parent\"").unwrap(),
        FeedRole::ReviewsParent
    );
    assert_eq!(FeedRole::default(), FeedRole::None);
}

#[test]
fn epic_substatus_label_all_variants() {
    assert_eq!(EpicSubstatus::Unplanned.label(), "unplanned");
    assert_eq!(EpicSubstatus::Planned.label(), "planned");
    assert_eq!(EpicSubstatus::Active.label(), "active");
    assert_eq!(EpicSubstatus::Blocked(1).label(), "1 blocked");
    assert_eq!(EpicSubstatus::Blocked(5).label(), "5 blocked");
    assert_eq!(EpicSubstatus::InReview.label(), "in review");
    assert_eq!(EpicSubstatus::Done.label(), "done");
}

#[test]
fn epic_substatus_header_label_active_states() {
    assert_eq!(EpicSubstatus::Blocked(2).header_label(), "needs input");
    assert_eq!(EpicSubstatus::Active.header_label(), "active");
    assert_eq!(EpicSubstatus::InReview.header_label(), "awaiting review");
}

#[test]
fn epic_substatus_header_label_terminal_states_are_empty() {
    assert_eq!(EpicSubstatus::Unplanned.header_label(), "");
    assert_eq!(EpicSubstatus::Planned.header_label(), "");
    assert_eq!(EpicSubstatus::Done.header_label(), "");
}

#[test]
fn epic_substatus_blocked_counts_conflict_and_crashed() {
    for sub in [SubStatus::Conflict, SubStatus::Crashed] {
        let epic = make_epic(1, TaskStatus::Running, None, None);
        let t = make_task(1, TaskStatus::Running, sub, None);
        assert_eq!(
            epic_substatus(&epic, &[&t]),
            EpicSubstatus::Blocked(1),
            "{sub:?}"
        );
    }
}

#[test]
fn descendant_epic_ids_includes_root_itself() {
    let epics = vec![make_epic(1, TaskStatus::Backlog, None, None)];
    let ids = descendant_epic_ids(EpicId(1), &epics);
    assert!(ids.contains(&EpicId(1)));
    assert_eq!(ids.len(), 1);
}

#[test]
fn descendant_epic_ids_includes_direct_children() {
    let epics = vec![
        make_epic(1, TaskStatus::Backlog, None, None),
        make_epic(2, TaskStatus::Backlog, None, Some(1)),
        make_epic(3, TaskStatus::Backlog, None, Some(1)),
    ];
    let ids = descendant_epic_ids(EpicId(1), &epics);
    assert!(ids.contains(&EpicId(1)));
    assert!(ids.contains(&EpicId(2)));
    assert!(ids.contains(&EpicId(3)));
    assert_eq!(ids.len(), 3);
}

#[test]
fn descendant_epic_ids_is_recursive() {
    // root(1) → child(2) → grandchild(3)
    let epics = vec![
        make_epic(1, TaskStatus::Backlog, None, None),
        make_epic(2, TaskStatus::Backlog, None, Some(1)),
        make_epic(3, TaskStatus::Backlog, None, Some(2)),
    ];
    let ids = descendant_epic_ids(EpicId(1), &epics);
    assert!(ids.contains(&EpicId(1)));
    assert!(ids.contains(&EpicId(2)));
    assert!(ids.contains(&EpicId(3)));
    assert_eq!(ids.len(), 3);
}

#[test]
fn descendant_epic_ids_excludes_unrelated_subtree() {
    let epics = vec![
        make_epic(1, TaskStatus::Backlog, None, None),
        make_epic(2, TaskStatus::Backlog, None, None),
        make_epic(3, TaskStatus::Backlog, None, Some(2)),
    ];
    let ids = descendant_epic_ids(EpicId(1), &epics);
    assert!(ids.contains(&EpicId(1)));
    assert!(!ids.contains(&EpicId(2)));
    assert!(!ids.contains(&EpicId(3)));
}

#[test]
fn descendant_epic_ids_is_cycle_safe() {
    // Malformed: epic 1 → parent 2, epic 2 → parent 1
    let epics = vec![
        make_epic(1, TaskStatus::Backlog, None, Some(2)),
        make_epic(2, TaskStatus::Backlog, None, Some(1)),
    ];
    let ids = descendant_epic_ids(EpicId(1), &epics);
    assert!(ids.contains(&EpicId(1)));
    assert!(ids.contains(&EpicId(2)));
}

// ---------------------------------------------------------------------------
// build_children_map / descendant_epic_ids_with_map
// ---------------------------------------------------------------------------

#[test]
fn descendant_epic_ids_with_map_matches_original_for_deep_tree() {
    // root(1) → child(2) → grandchild(3), unrelated(4) → child(5)
    let epics = vec![
        make_epic(1, TaskStatus::Backlog, None, None),
        make_epic(2, TaskStatus::Backlog, None, Some(1)),
        make_epic(3, TaskStatus::Backlog, None, Some(2)),
        make_epic(4, TaskStatus::Backlog, None, None),
        make_epic(5, TaskStatus::Backlog, None, Some(4)),
    ];
    let children = build_children_map(&epics);

    for root in [EpicId(1), EpicId(2), EpicId(3), EpicId(4), EpicId(5)] {
        let original = descendant_epic_ids(root, &epics);
        let with_map = descendant_epic_ids_with_map(root, &children);
        assert_eq!(
            original, with_map,
            "descendant_epic_ids_with_map must match original for root {root:?}"
        );
    }
}

#[test]
fn build_children_map_returns_empty_for_flat_epics() {
    let epics = vec![
        make_epic(1, TaskStatus::Backlog, None, None),
        make_epic(2, TaskStatus::Backlog, None, None),
    ];
    let children = build_children_map(&epics);
    assert!(
        children.is_empty(),
        "no parent-child relationships → empty map"
    );
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
    let task = Task {
        id: TaskId(1),
        title: "Test".to_string(),
        description: "Desc".to_string(),
        repo_path: "/repo".to_string(),
        ..Default::default()
    };
    assert!(task.epic_id.is_none());
}

#[test]
fn task_with_epic_id() {
    let task = Task {
        id: TaskId(1),
        title: "Test".to_string(),
        description: "Desc".to_string(),
        repo_path: "/repo".to_string(),
        epic_id: Some(EpicId(5)),
        ..Default::default()
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
        status: TaskStatus::Backlog,
        plan_path: None,
        sort_order: None,
        completed_at: None,
        auto_dispatch: true,
        parent_epic_id: None,
        feed_command: None,
        feed_interval_secs: None,
        group_by_repo: false,
        feed_append_only: false,
        feed_role: FeedRole::None,
        origin: EpicOrigin::Manual,
        created_at: now,
        updated_at: now,
    };
    assert_eq!(epic.id, EpicId(1));
    assert_eq!(epic.status, TaskStatus::Backlog);
}

// --- Epic.status direct access ---
// epic_status() was a wrapper that once derived status from subtasks.
// It was deleted; callers should access epic.status directly.

#[test]
fn epic_has_status_field_directly_accessible() {
    // Regression guard: epic.status is public and accessible directly.
    // Previously callers used epic_status(&epic) — that wrapper no longer exists.
    for status in [
        TaskStatus::Done,
        TaskStatus::Backlog,
        TaskStatus::Running,
        TaskStatus::Review,
    ] {
        let epic = make_epic(1, status, None, None);
        assert_eq!(epic.status, status);
    }
}

// --- EpicSubstatus / epic_substatus ---

fn test_epic() -> Epic {
    EpicBuilder::new(1)
        .title("Test")
        .auto_dispatch(true)
        .build()
}

fn test_task() -> Task {
    TaskBuilder::new(1).title("T").build()
}

/// The store decides repo grouping now (store-split M2); this is the
/// board's own read of the same predicate, and the two must not drift.
#[test]
fn can_auto_group_agrees_with_the_modules_rule() {
    for grouped in [true, false] {
        for command in [None, Some("gh pr list".to_string())] {
            let epic = Epic {
                group_by_repo: grouped,
                feed_command: command.clone(),
                ..test_epic()
            };
            assert_eq!(
                epic.can_auto_group(),
                dispatch_spacetime_module::can_auto_group(
                    grouped,
                    command.as_deref().unwrap_or("")
                ),
                "grouped={grouped} command={command:?}"
            );
        }
    }
}

#[test]
fn epic_substatus_unplanned() {
    let epic = Epic {
        plan_path: None,
        status: TaskStatus::Backlog,
        ..test_epic()
    };
    assert_eq!(epic_substatus(&epic, &[]), EpicSubstatus::Unplanned);
}

#[test]
fn epic_substatus_planned() {
    let epic = Epic {
        plan_path: Some("plan.md".into()),
        status: TaskStatus::Backlog,
        ..test_epic()
    };
    assert_eq!(epic_substatus(&epic, &[]), EpicSubstatus::Planned);
}

#[test]
fn epic_substatus_active_with_backlog() {
    let epic = Epic {
        status: TaskStatus::Running,
        ..test_epic()
    };
    let t1 = Task {
        status: TaskStatus::Running,
        sub_status: SubStatus::Active,
        ..test_task()
    };
    let t2 = Task {
        status: TaskStatus::Backlog,
        ..test_task()
    };
    assert_eq!(epic_substatus(&epic, &[&t1, &t2]), EpicSubstatus::Active);
}

#[test]
fn epic_substatus_active_all_running() {
    let epic = Epic {
        status: TaskStatus::Running,
        ..test_epic()
    };
    let t1 = Task {
        status: TaskStatus::Running,
        sub_status: SubStatus::Active,
        ..test_task()
    };
    let t2 = Task {
        status: TaskStatus::Done,
        sub_status: SubStatus::None,
        ..test_task()
    };
    assert_eq!(epic_substatus(&epic, &[&t1, &t2]), EpicSubstatus::Active);
}

#[test]
fn epic_substatus_blocked_stale() {
    let epic = Epic {
        status: TaskStatus::Running,
        ..test_epic()
    };
    let t1 = Task {
        status: TaskStatus::Running,
        sub_status: SubStatus::Stale,
        ..test_task()
    };
    let t2 = Task {
        status: TaskStatus::Backlog,
        ..test_task()
    };
    assert_eq!(
        epic_substatus(&epic, &[&t1, &t2]),
        EpicSubstatus::Blocked(1)
    );
}

#[test]
fn epic_substatus_blocked_needs_input() {
    let epic = Epic {
        status: TaskStatus::Running,
        ..test_epic()
    };
    let t1 = Task {
        status: TaskStatus::Running,
        sub_status: SubStatus::NeedsInput,
        ..test_task()
    };
    let t2 = Task {
        status: TaskStatus::Running,
        sub_status: SubStatus::Active,
        ..test_task()
    };
    assert_eq!(
        epic_substatus(&epic, &[&t1, &t2]),
        EpicSubstatus::Blocked(1)
    );
}

#[test]
fn epic_substatus_blocked_count() {
    let epic = Epic {
        status: TaskStatus::Running,
        ..test_epic()
    };
    let t1 = Task {
        status: TaskStatus::Running,
        sub_status: SubStatus::NeedsInput,
        ..test_task()
    };
    let t2 = Task {
        status: TaskStatus::Running,
        sub_status: SubStatus::Stale,
        ..test_task()
    };
    let t3 = Task {
        status: TaskStatus::Running,
        sub_status: SubStatus::Active,
        ..test_task()
    };
    assert_eq!(
        epic_substatus(&epic, &[&t1, &t2, &t3]),
        EpicSubstatus::Blocked(2)
    );
}

#[test]
fn epic_substatus_in_review() {
    let epic = Epic {
        status: TaskStatus::Review,
        ..test_epic()
    };
    assert_eq!(epic_substatus(&epic, &[]), EpicSubstatus::InReview);
}

#[test]
fn epic_substatus_done() {
    let epic = Epic {
        status: TaskStatus::Done,
        ..test_epic()
    };
    assert_eq!(epic_substatus(&epic, &[]), EpicSubstatus::Done);
}

// --- descendant_task_ids ---
// These reuse the module's `make_epic`/`make_task` builders: a backlog epic
// with the given parent, and a backlog task under the given epic.

fn epic_with(id: i64, parent: Option<i64>) -> Epic {
    make_epic(id, TaskStatus::Backlog, None, parent)
}

fn task_under(id: i64, epic: Option<i64>) -> Task {
    make_task(id, TaskStatus::Backlog, SubStatus::None, epic)
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
fn ancestor_titles_root_returns_self_only() {
    let mut root = epic_with(1, None);
    root.title = "Root".into();
    let epics = vec![root.clone()];
    assert_eq!(ancestor_titles(&root, &epics), vec!["Root"]);
}

#[test]
fn ancestor_titles_orders_root_to_self() {
    // root(1) -> mid(2) -> leaf(3)
    let mut root = epic_with(1, None);
    root.title = "Root".into();
    let mut mid = epic_with(2, Some(1));
    mid.title = "Mid".into();
    let mut leaf = epic_with(3, Some(2));
    leaf.title = "Leaf".into();
    let epics = vec![root, mid, leaf.clone()];
    assert_eq!(ancestor_titles(&leaf, &epics), vec!["Root", "Mid", "Leaf"]);
}

#[test]
fn ancestor_titles_is_cycle_safe() {
    // 1 -> 2 -> 1 (malformed cycle); must terminate.
    let mut a = epic_with(1, Some(2));
    a.title = "A".into();
    let mut b = epic_with(2, Some(1));
    b.title = "B".into();
    let epics = vec![a.clone(), b];
    let out = ancestor_titles(&a, &epics);
    // Terminates and includes A's own title as the last segment.
    assert_eq!(out.last(), Some(&"A"));
    assert!(out.len() <= 2);
}

#[test]
fn epic_origin_roundtrips_kebab_case() {
    assert_eq!(EpicOrigin::Manual.as_str(), "manual");
    assert_eq!(EpicOrigin::RepoGroup.as_str(), "repo-group");
    assert_eq!(EpicOrigin::parse("manual"), Some(EpicOrigin::Manual));
    assert_eq!(EpicOrigin::parse("repo-group"), Some(EpicOrigin::RepoGroup));
    assert_eq!(EpicOrigin::parse("bogus"), None);
    assert_eq!(EpicOrigin::default(), EpicOrigin::Manual);
}
