//! A row behind a view has no primary key in the generated bindings, so an
//! update arrives as INSERT(new) then DELETE(old) with the same id and no
//! `on_update` (`docs/specs/sync.allium`, `AViewUpdateNeverErasesTheRow`;
//! `docs/plans/2026-10-28-per-caller-views-spike.md`, answer 3).
//!
//! The SDK's callback order for the pair is not something to depend on, so
//! both orders are pinned.

use crate::models::{EpicId, TaskId};
use crate::spacetime::bindings;
use crate::sync::rows::SharedRows;

fn task(title: &str) -> bindings::Task {
    bindings::Task {
        id: 7,
        title: title.into(),
        repo_path: "/repo".into(),
        created_at: "2026-09-18 10:00:00".into(),
        updated_at: "2026-09-18 10:00:00".into(),
        labels: String::new(),
        owner: String::new(),
        ..dispatch_spacetime_module::blank_task().into()
    }
}

fn epic(title: &str) -> bindings::Epic {
    bindings::Epic {
        id: 7,
        title: title.into(),
        created_at: "2026-09-18 10:00:00".into(),
        updated_at: "2026-09-18 10:00:00".into(),
        ..dispatch_spacetime_module::blank_epic().into()
    }
}

#[test]
fn a_task_update_pair_keeps_the_new_row_insert_first() {
    let rows = SharedRows::new();
    rows.upsert_task(&task("old"));
    rows.upsert_task(&task("new"));
    rows.retire_task(&task("old"));
    assert_eq!(rows.task(TaskId(7)).unwrap().title, "new");
}

#[test]
fn a_task_update_pair_keeps_the_new_row_delete_first() {
    let rows = SharedRows::new();
    rows.upsert_task(&task("old"));
    rows.retire_task(&task("old"));
    rows.upsert_task(&task("new"));
    assert_eq!(rows.task(TaskId(7)).unwrap().title, "new");
}

#[test]
fn a_task_leaving_the_view_is_removed() {
    let rows = SharedRows::new();
    rows.upsert_task(&task("only"));
    rows.retire_task(&task("only"));
    assert!(rows.task(TaskId(7)).is_none());
}

#[test]
fn an_epic_update_pair_keeps_the_new_row_in_either_order() {
    let rows = SharedRows::new();
    rows.upsert_epic(&epic("old"));
    rows.upsert_epic(&epic("new"));
    rows.retire_epic(&epic("old"));
    assert_eq!(rows.epic(EpicId(7)).unwrap().title, "new");

    let rows = SharedRows::new();
    rows.upsert_epic(&epic("old"));
    rows.retire_epic(&epic("old"));
    rows.upsert_epic(&epic("new"));
    assert_eq!(rows.epic(EpicId(7)).unwrap().title, "new");
}

#[test]
fn an_epic_leaving_the_view_is_removed() {
    let rows = SharedRows::new();
    rows.upsert_epic(&epic("only"));
    rows.retire_epic(&epic("only"));
    assert!(rows.epic(EpicId(7)).is_none());
}
