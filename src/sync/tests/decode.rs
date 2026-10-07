//! The decoder's refusals and soft-fails: a row that arrives from the shared
//! store either decodes to the domain value it stands for or is dropped under
//! the decode-failure policy (`docs/conventions.md`, "Soft-fail decoding").
//!
//! Every board view is a function of `Vec<Task>` and `Vec<Epic>`, so these pin
//! the edges of that function. The happy path is exercised by every test that
//! writes through a memory-attached handle, which decodes through the same
//! code.

use crate::spacetime::bindings;
use crate::sync::decode;

// ---------------------------------------------------------------------------
// The comparison
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// The sentinels, stated directly
// ---------------------------------------------------------------------------

/// `sort_order` is the deliberate exception: its zero is the top of a column,
/// so it stayed optional in the module and must not be sentinel-decoded.
#[test]
fn sort_order_zero_is_a_position_not_an_absence() {
    let mut row = blank_task();
    row.sort_order = Some(0);

    assert_eq!(decode::task(&row).unwrap().sort_order, Some(0));

    row.sort_order = None;
    assert_eq!(decode::task(&row).unwrap().sort_order, None);
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

/// A status this binary does not know fails the row.
///
/// The caller drops it, so a task written by a newer binary is MISSING from the
/// board rather than sitting on it under a plausible wrong status. The same
/// bargain `drop_undecodable` makes for every bulk read.
#[test]
fn an_unknown_enum_fails_the_row() {
    for (label, mutate) in [
        (
            "status",
            (|r: &mut bindings::Task| r.status = "teleported".into()) as fn(&mut bindings::Task),
        ),
        ("sub_status", |r| r.sub_status = "teleported".into()),
        ("tag", |r| r.tag = "teleported".into()),
        ("wrap_up_mode", |r| r.wrap_up_mode = "teleported".into()),
        ("url_type", |r| {
            r.url = "https://example.test/x".into();
            r.url_type = "teleported".into();
        }),
    ] {
        let mut row = blank_task();
        mutate(&mut row);
        let error = decode::task(&row).unwrap_err().to_string();
        assert!(
            error.contains("teleported"),
            "{label}: the refusal must name the value it could not read, got {error:?}"
        );
    }
}

/// An operator decision (task #16386): a row with a REMOVED status is dropped,
/// not mapped to done and not kept. It is skipped quietly, so a store holding
/// thousands of them does not fill the log, and it does not count as a decode
/// failure. `sync.allium`: `RowsWithARemovedStatusAreDropped`.
#[test]
fn a_row_with_a_removed_status_is_dropped_quietly() {
    let shared = crate::sync::SharedRows::new();

    let mut task = blank_task();
    task.status = "archived".into();
    shared.upsert_task(&task);
    assert!(shared.task(crate::models::TaskId(task.id)).is_none());

    let mut epic = blank_epic();
    epic.status = "archived".into();
    shared.upsert_epic(&epic);
    assert!(shared.epic(crate::models::EpicId(epic.id)).is_none());
}

/// A removed status is not a decode failure: it must not move the counter that
/// says a board is quietly losing real rows.
#[test]
fn a_removed_status_does_not_count_as_a_decode_failure() {
    let shared = crate::sync::SharedRows::new();
    let before = crate::store::decode_fallback_count();
    let mut task = blank_task();
    task.status = "archived".into();
    for _ in 0..50 {
        shared.upsert_task(&task);
    }
    // Other tests may bump the counter concurrently, but none of them can
    // account for fifty; one per archived row would.
    assert!(crate::store::decode_fallback_count() - before < 50);
}

/// The rule is one predicate, shared with the import: anything not on the
/// removed list is still refused with a warning.
#[test]
fn only_listed_statuses_count_as_removed() {
    assert!(crate::models::is_removed_status("archived"));
    for live in ["backlog", "running", "review", "done", "teleported", ""] {
        assert!(!crate::models::is_removed_status(live), "{live:?}");
    }
    let shared = crate::sync::SharedRows::new();
    let mut task = blank_task();
    task.status = "teleported".into();
    shared.upsert_task(&task);
    assert!(shared.task(crate::models::TaskId(task.id)).is_none());
}

/// A url without its type, or the reverse, is a row the application cannot
/// produce. Coercing it to `None` would hide a corrupt row behind a card that
/// looks merely un-linked.
#[test]
fn a_half_written_url_fails_the_row() {
    let mut row = blank_task();
    row.url = "https://example.test/pull/1".into();
    assert!(decode::task(&row).is_err());

    let mut row = blank_task();
    row.url_type = "pr".into();
    assert!(decode::task(&row).is_err());
}

/// A malformed window name is dropped rather than refused: the card is worth
/// drawing without it, and refusing would take the task off the board over a
/// cosmetic field.
#[test]
fn a_malformed_tmux_window_is_dropped_not_refused() {
    // A pane id, which `TmuxWindow` refuses: it bypasses name resolution in
    // tmux, so it is not a window name.
    let mut row = blank_task();
    row.tmux_window = "%3".into();

    let decoded = decode::task(&row).expect("the row must still decode");
    assert_eq!(decoded.tmux_window, None);
    assert_eq!(decoded.title, "Blank");
}

/// An unknown feed role or origin defaults rather than refusing — the epic and
/// every task under it would otherwise vanish from the board.
#[test]
fn an_unknown_feed_role_defaults_rather_than_failing() {
    let mut row = blank_epic();
    row.feed_role = "teleported".into();
    row.origin = "teleported".into();

    let decoded = decode::epic(&row).expect("the epic must still decode");
    assert_eq!(decoded.feed_role, crate::models::FeedRole::None);
    assert_eq!(decoded.origin, crate::models::EpicOrigin::Manual);
}

/// An unparseable required timestamp fails the row. There is no sensible
/// default for "when was this created", and a card sorted by a made-up date is
/// a card in the wrong place.
#[test]
fn an_unparseable_timestamp_fails_the_row() {
    let mut row = blank_task();
    row.created_at = "yesterday".into();

    assert!(decode::task(&row)
        .unwrap_err()
        .to_string()
        .contains("created_at"));
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A minimal valid row: every sentinel at its absent value.
fn blank_task() -> bindings::Task {
    bindings::Task {
        id: 1,
        title: "Blank".into(),
        repo_path: "/repo".into(),
        created_at: "2026-09-18 10:00:00".into(),
        updated_at: "2026-09-18 10:00:00".into(),
        labels: String::new(),
        owner: String::new(),
        ..dispatch_spacetime_module::blank_task().into()
    }
}

fn blank_epic() -> bindings::Epic {
    bindings::Epic {
        id: 1,
        title: "Blank".into(),
        created_at: "2026-09-18 10:00:00".into(),
        updated_at: "2026-09-18 10:00:00".into(),
        ..dispatch_spacetime_module::blank_epic().into()
    }
}

/// `DeleteEpicRefused` (epics.allium): a dropped task is remembered by id and
/// epic, so the delete pre-check can still count it in the subtree. A later
/// good copy or a removal forgets it.
#[test]
fn an_undecodable_task_is_remembered_against_its_epic_until_replaced_or_removed() {
    use crate::models::{EpicId, TaskId};
    let shared = crate::sync::SharedRows::new();
    let mut task = blank_task();
    task.epic_id = 7;
    task.status = "teleported".into();
    shared.upsert_task(&task);
    assert_eq!(
        shared.undecodable_task_ids_for_epic(EpicId(7)),
        vec![TaskId(task.id)]
    );
    assert!(shared.undecodable_task_ids_for_epic(EpicId(8)).is_empty());

    task.status = "done".into();
    shared.upsert_task(&task);
    assert!(shared.undecodable_task_ids_for_epic(EpicId(7)).is_empty());

    task.status = "teleported".into();
    shared.upsert_task(&task);
    shared.remove_task(TaskId(task.id));
    assert!(shared.undecodable_task_ids_for_epic(EpicId(7)).is_empty());
}
