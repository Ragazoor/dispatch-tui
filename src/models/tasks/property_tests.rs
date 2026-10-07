use super::model_tests::make_task_with;
use super::*;
use proptest::prelude::*;

const TASK_STATUSES: &[TaskStatus] = &[
    TaskStatus::Backlog,
    TaskStatus::Running,
    TaskStatus::Review,
    TaskStatus::Done,
];

const TASK_TAGS: &[TaskTag] = &[
    TaskTag::Bug,
    TaskTag::Feature,
    TaskTag::Chore,
    TaskTag::PrReview,
    TaskTag::Research,
    TaskTag::Fix,
    TaskTag::Dependabot,
];

/// A tag option spanning every `TaskTag` variant plus the untagged case —
/// the full input domain for `DispatchMode::for_task` routing.
fn tag_option_strategy() -> impl Strategy<Value = Option<TaskTag>> {
    prop_oneof![
        Just(None),
        (0..TASK_TAGS.len()).prop_map(|i| Some(TASK_TAGS[i])),
    ]
}

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
    fn slugify_never_panics(input in "\\PC{0,2000}") {
        // slugify should never panic on arbitrary input
        let _ = slugify(&input);
    }

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
        // For Backlog and Done only SubStatus::None is valid. Running and
        // Review require a specific active sub-status.
        for &terminal in &[TaskStatus::Backlog, TaskStatus::Done] {
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

    /// `DispatchMode::for_task` over the full `tag × plan-presence` domain:
    /// a plan always forces `Dispatch`; without a plan only the `research`
    /// tag routes to its dedicated `Research` agent, everything else
    /// (including untagged) falls through to `Dispatch`.
    #[test]
    fn dispatch_mode_routing(
        tag in tag_option_strategy(),
        has_plan in any::<bool>(),
    ) {
        let plan = if has_plan { Some("plan.md") } else { None };
        let mode = DispatchMode::for_task(&make_task_with(plan, tag));

        // Only an unplanned research task routes to the dedicated agent;
        // everything else (plan present, or any other tag) → Dispatch.
        let expected = if !has_plan && tag == Some(TaskTag::Research) {
            DispatchMode::Research
        } else {
            DispatchMode::Dispatch
        };

        prop_assert_eq!(mode, expected, "tag={:?} has_plan={}", tag, has_plan);
    }
}
