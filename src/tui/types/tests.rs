use super::*;
use crate::models::{EpicBuilder, TaskId};
use std::collections::HashSet;

fn make_test_epic(id: i64, parent: Option<i64>) -> Epic {
    EpicBuilder::new(id).parent(parent).build()
}

fn make_test_task(id: i64, status: TaskStatus, epic: Option<i64>) -> Task {
    Task {
        id: TaskId(id),
        title: format!("Task {id}"),
        status,
        epic_id: epic.map(EpicId),
        ..Default::default()
    }
}

// -- Message / Command size --

/// The bus enums are moved by value on every keystroke, every async result
/// and every loop iteration (`LoopEvent::Message`), so an entity stored
/// inline in one variant is paid for by every other variant. The rule is
/// that no domain entity is ever inline: `Task` and `Epic` payloads are
/// boxed wherever the bus carries them.
///
/// Asserted as a relation rather than a byte count on purpose.
/// `clippy::large_enum_variant` compares the largest variant to the
/// *second* largest, so it is structurally blind to the case these enums
/// are most exposed to — the editor, epic and task domains all growing
/// together, which keeps the spread small while the total doubles. A fixed
/// ceiling would catch that, but its first failure invites a bump; the
/// relation states the actual invariant and never needs maintenance.
fn assert_no_entity_inline<T>(name: &str) {
    let size = std::mem::size_of::<T>();
    let task = std::mem::size_of::<Task>();
    assert!(
        size < task,
        "{name} is {size} bytes against a {task}-byte `Task` — an entity has \
             been inlined into the bus. Box the payload rather than relaxing this."
    );
}

#[test]
fn message_enum_carries_no_inline_entity() {
    assert_no_entity_inline::<Message>("Message");
}

#[test]
fn command_enum_carries_no_inline_entity() {
    assert_no_entity_inline::<Command>("Command");
}

// -- SubtaskStats --

#[test]
fn subtask_stats_counts_direct_tasks_only_without_nested_epics() {
    let epics = vec![make_test_epic(1, None)];
    let tasks = vec![
        make_test_task(1, TaskStatus::Running, Some(1)),
        make_test_task(2, TaskStatus::Done, Some(1)),
    ];
    let cm = crate::models::build_children_map(&epics);
    let stats = SubtaskStats::for_epic(&epics[0], &tasks, &cm);
    assert_eq!(stats.running, 1);
    assert_eq!(stats.done, 1);
    assert_eq!(stats.total, 2);
}

#[test]
fn subtask_stats_includes_tasks_from_nested_sub_epics() {
    let epics = vec![make_test_epic(1, None), make_test_epic(2, Some(1))];
    let tasks = vec![
        make_test_task(1, TaskStatus::Backlog, Some(1)),
        make_test_task(2, TaskStatus::Running, Some(2)),
        make_test_task(3, TaskStatus::Done, Some(2)),
    ];
    let cm = crate::models::build_children_map(&epics);
    let stats = SubtaskStats::for_epic(&epics[0], &tasks, &cm);
    assert_eq!(stats.backlog, 1);
    assert_eq!(stats.running, 1);
    assert_eq!(stats.done, 1);
    assert_eq!(stats.total, 3);
}

#[test]
fn subtask_stats_includes_tasks_from_deeply_nested_epics() {
    let epics = vec![
        make_test_epic(1, None),
        make_test_epic(2, Some(1)),
        make_test_epic(3, Some(2)),
    ];
    let tasks = vec![make_test_task(1, TaskStatus::Running, Some(3))];
    let cm = crate::models::build_children_map(&epics);
    let stats = SubtaskStats::for_epic(&epics[0], &tasks, &cm);
    assert_eq!(stats.running, 1);
    assert_eq!(stats.total, 1);
}

#[test]
fn subtask_stats_ignores_tasks_with_no_epic_id() {
    let epics = vec![make_test_epic(1, None)];
    let tasks = vec![
        make_test_task(1, TaskStatus::Running, Some(1)),
        make_test_task(2, TaskStatus::Running, None), // unowned — must not count
    ];
    let cm = crate::models::build_children_map(&epics);
    let stats = SubtaskStats::for_epic(&epics[0], &tasks, &cm);
    assert_eq!(stats.running, 1);
    assert_eq!(stats.total, 1);
}

#[test]
fn subtask_stats_blocked_substatus_includes_nested_blocked_tasks() {
    use crate::models::{EpicSubstatus, SubStatus};

    let mut parent = make_test_epic(1, None);
    parent.status = TaskStatus::Running;
    let child_epic = make_test_epic(2, Some(1));
    let epics = vec![parent.clone(), child_epic];

    // A blocked task lives on the child epic, not directly on parent.
    let mut blocked_task = make_test_task(1, TaskStatus::Running, Some(2));
    blocked_task.sub_status = SubStatus::Crashed;
    let tasks = vec![blocked_task];

    let cm = crate::models::build_children_map(&epics);
    let stats = SubtaskStats::for_epic(&parent, &tasks, &cm);
    assert_eq!(stats.substatus, EpicSubstatus::Blocked(1));
}

// -- RepoFilterMode --

#[test]
fn repo_filter_mode_as_str() {
    assert_eq!(RepoFilterMode::Include.as_str(), "include");
    assert_eq!(RepoFilterMode::Exclude.as_str(), "exclude");
}

#[test]
fn repo_filter_mode_from_str_roundtrip() {
    for mode in [RepoFilterMode::Include, RepoFilterMode::Exclude] {
        let s = mode.as_str();
        let parsed: RepoFilterMode = s.parse().unwrap();
        assert_eq!(parsed, mode);
    }
}

#[test]
fn repo_filter_mode_from_str_invalid() {
    assert!("bogus".parse::<RepoFilterMode>().is_err());
    assert!("".parse::<RepoFilterMode>().is_err());
    assert!("Include".parse::<RepoFilterMode>().is_err());
}

#[test]
fn repo_filter_mode_default_is_include() {
    assert_eq!(RepoFilterMode::default(), RepoFilterMode::Include);
}

// -- repo_filter_matches --

/// Test-only mirror of the repo-filter predicate. Lives here (not in the
/// production region) because only these tests exercise it.
fn repo_filter_matches(filter: &HashSet<String>, mode: RepoFilterMode, repo: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    match mode {
        RepoFilterMode::Include => filter.contains(repo),
        RepoFilterMode::Exclude => !filter.contains(repo),
    }
}

#[test]
fn repo_filter_matches_empty_filter_matches_any_repo() {
    let filter = HashSet::new();
    assert!(repo_filter_matches(
        &filter,
        RepoFilterMode::Include,
        "org/any"
    ));
    assert!(repo_filter_matches(
        &filter,
        RepoFilterMode::Exclude,
        "org/any"
    ));
}

#[test]
fn repo_filter_matches_include_mode() {
    let filter: HashSet<String> = ["org/a".to_string()].into();
    assert!(repo_filter_matches(
        &filter,
        RepoFilterMode::Include,
        "org/a"
    ));
    assert!(!repo_filter_matches(
        &filter,
        RepoFilterMode::Include,
        "org/b"
    ));
}

#[test]
fn repo_filter_matches_exclude_mode() {
    let filter: HashSet<String> = ["org/a".to_string()].into();
    assert!(!repo_filter_matches(
        &filter,
        RepoFilterMode::Exclude,
        "org/a"
    ));
    assert!(repo_filter_matches(
        &filter,
        RepoFilterMode::Exclude,
        "org/b"
    ));
}

// -- StatusState --

#[test]
fn status_set_shows_a_transient_message() {
    let mut status = StatusState::default();
    status.set("saved".to_string());
    assert_eq!(status.message.as_deref(), Some("saved"));
    assert!(status.message_set_at.is_some());
    assert!(!status.message_sticky);
}

#[test]
fn status_set_sticky_survives_the_ttl() {
    let mut status = StatusState::default();
    status.set_sticky("dispatching".to_string());
    assert_eq!(status.message.as_deref(), Some("dispatching"));
    assert!(status.message_sticky);
}

#[test]
fn status_clear_drops_message_timestamp_and_stickiness() {
    let mut status = StatusState::default();
    status.set_sticky("dispatching".to_string());
    status.clear();
    assert!(status.message.is_none());
    assert!(status.message_set_at.is_none());
    assert!(!status.message_sticky);
}

// -- InputState text editing --

fn input_with(buffer: &str, caret: usize) -> InputState {
    let mut input = InputState::default();
    input.set_buffer(buffer.to_string());
    input.caret = caret;
    input
}

#[test]
fn input_insert_char_puts_it_at_the_caret() {
    let mut input = input_with("ac", 1);
    input.insert_char('b');
    assert_eq!((input.buffer.as_str(), input.caret), ("abc", 2));
}

#[test]
fn input_backspace_and_delete_forward_edit_around_the_caret() {
    let mut input = input_with("abc", 2);
    input.backspace();
    assert_eq!((input.buffer.as_str(), input.caret), ("ac", 1));
    input.delete_forward();
    assert_eq!((input.buffer.as_str(), input.caret), ("a", 1));
}

#[test]
fn input_edits_in_a_repo_picker_reset_the_list_cursor() {
    for edit in [
        (|i: &mut InputState| i.insert_char('x')) as fn(&mut InputState),
        InputState::backspace,
        InputState::delete_forward,
    ] {
        let mut input = input_with("abc", 1);
        input.mode = InputMode::InputRepoPath;
        input.repo_cursor = 3;
        edit(&mut input);
        assert_eq!(input.repo_cursor, 0);
    }
}

#[test]
fn input_caret_moves_stay_within_the_buffer() {
    let mut input = input_with("ab cd", 3);
    input.cursor_left();
    assert_eq!(input.caret, 2);
    input.cursor_right();
    assert_eq!(input.caret, 3);
    input.cursor_home();
    assert_eq!(input.caret, 0);
    input.cursor_end();
    assert_eq!(input.caret, 5);
    input.cursor_word_left();
    assert_eq!(input.caret, 3);
    input.cursor_word_right();
    assert_eq!(input.caret, 5);
}
