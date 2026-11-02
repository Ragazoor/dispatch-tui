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

#[test]
fn an_unseen_item_update_pair_keeps_the_new_row_in_either_order() {
    let item = |creator: &str| bindings::UnseenItem {
        kind: "epic".into(),
        item_id: 7,
        workspace_id: 1,
        creator: creator.into(),
        created_at: "2026-09-18 10:00:00".into(),
    };
    let rows = SharedRows::new();
    rows.upsert_unseen_item(&item("old"));
    rows.upsert_unseen_item(&item("new"));
    rows.retire_unseen_item(&item("old"));
    assert_eq!(rows.unseen_items()[0].creator, "new");

    let rows = SharedRows::new();
    rows.upsert_unseen_item(&item("old"));
    rows.retire_unseen_item(&item("old"));
    rows.upsert_unseen_item(&item("new"));
    assert_eq!(rows.unseen_items()[0].creator, "new");
}

/// The same three cases for every other view-served table: an update pair in
/// either order keeps the new row, and a row leaving the view is removed.
/// `row(tag)` builds a row whose one varying field is `tag`; `probe` reads
/// that field back, or `None` when the row is gone.
macro_rules! pins_view_updates {
    ($name:ident, $upsert:ident, $retire:ident, $row:expr, $probe:expr) => {
        #[test]
        fn $name() {
            let row = $row;
            let probe = $probe;

            let rows = SharedRows::new();
            rows.$upsert(&row("old"));
            rows.$upsert(&row("new"));
            rows.$retire(&row("old"));
            assert_eq!(probe(&rows).as_deref(), Some("new"), "insert first");

            let rows = SharedRows::new();
            rows.$upsert(&row("old"));
            rows.$retire(&row("old"));
            rows.$upsert(&row("new"));
            assert_eq!(probe(&rows).as_deref(), Some("new"), "delete first");

            let rows = SharedRows::new();
            rows.$upsert(&row("only"));
            rows.$retire(&row("only"));
            assert_eq!(probe(&rows), None, "leaving the view");
        }
    };
}

pins_view_updates!(
    a_workspace_update_pair_keeps_the_new_row,
    upsert_workspace,
    retire_workspace,
    |tag: &str| bindings::Workspace {
        id: 7,
        name: tag.into(),
        ..dispatch_spacetime_module::blank_workspace().into()
    },
    |rows: &SharedRows| rows.workspaces().first().map(|w| w.name.clone())
);

pins_view_updates!(
    a_membership_update_pair_keeps_the_new_row,
    upsert_membership,
    retire_membership,
    |tag: &str| bindings::Membership {
        id: 7,
        role: tag.into(),
        ..dispatch_spacetime_module::blank_membership().into()
    },
    |rows: &SharedRows| rows.memberships().first().map(|m| m.role.clone())
);

pins_view_updates!(
    a_setting_update_pair_keeps_the_new_row,
    upsert_setting,
    retire_setting,
    |tag: &str| bindings::Setting {
        id: "host-a/theme".into(),
        host: "host-a".into(),
        key: "theme".into(),
        value: tag.into(),
    },
    |rows: &SharedRows| rows.setting("theme")
);

pins_view_updates!(
    a_work_package_update_pair_keeps_the_new_row,
    upsert_work_package,
    retire_work_package,
    |tag: &str| bindings::WorkPackage {
        id: 7,
        title: tag.into(),
        created_at: "2026-09-18 10:00:00".into(),
        updated_at: "2026-09-18 10:00:00".into(),
        ..dispatch_spacetime_module::blank_work_package().into()
    },
    |rows: &SharedRows| rows.work_packages().first().map(|p| p.title.clone())
);

pins_view_updates!(
    a_dispatch_request_update_pair_keeps_the_new_row,
    upsert_dispatch_request,
    retire_dispatch_request,
    |tag: &str| bindings::DispatchRequest {
        id: 7,
        task_id: 7,
        kind: "dispatch".into(),
        host: "host-a".into(),
        mode: "research".into(),
        origin: "epic_chain".into(),
        state: "requested".into(),
        requested_at: "2026-10-11 12:00:00.000".into(),
        deadline: "2026-10-11 12:01:00.000".into(),
        requested_by: tag.into(),
        ..dispatch_spacetime_module::blank_dispatch_request().into()
    },
    |rows: &SharedRows| rows
        .dispatch_requests()
        .first()
        .map(|r| r.requested_by.clone())
);

pins_view_updates!(
    a_session_request_update_pair_keeps_the_new_row,
    upsert_session_request,
    retire_session_request,
    |tag: &str| bindings::SessionRequest {
        id: 7,
        task_id: 7,
        kind: "wrap_up".into(),
        action: "rebase".into(),
        host: "host-a".into(),
        requested_by: tag.into(),
        state: "requested".into(),
        requested_at: "2026-10-12 12:00:00.000".into(),
        deadline: "2026-10-12 12:02:00.000".into(),
        ..dispatch_spacetime_module::blank_session_request().into()
    },
    |rows: &SharedRows| rows
        .session_requests()
        .first()
        .map(|r| r.requested_by.clone())
);

pins_view_updates!(
    an_agent_update_pair_keeps_the_new_row,
    upsert_agent,
    retire_agent,
    |tag: &str| bindings::Agent {
        id: 4,
        task_id: 9,
        name: tag.into(),
        host: "host-a".into(),
        start_mode: "research".into(),
        state: "ended".into(),
        started_at: "2026-10-16 10:00:00.000".into(),
        ended_at: "2026-10-16 11:00:00.000".into(),
        end_reason: "worktree_gone".into(),
        ..dispatch_spacetime_module::blank_agent().into()
    },
    |rows: &SharedRows| rows.agents().first().map(|a| a.name.clone())
);

/// A row that never decoded leaves the view too: the id must stop counting as
/// an undecodable task of its epic.
#[test]
fn an_undecodable_task_leaving_the_view_is_forgotten() {
    let rows = SharedRows::new();
    let mut bad = task("bad");
    bad.epic_id = 3;
    bad.status = "teleported".into();
    rows.upsert_task(&bad);
    assert_eq!(
        rows.undecodable_task_ids_for_epic(EpicId(3)),
        vec![TaskId(7)]
    );
    rows.retire_task(&bad);
    assert!(rows.undecodable_task_ids_for_epic(EpicId(3)).is_empty());
}

/// Task 98793: the PR-learnings hook sets `pr_learnings_gate_shown_at`, a
/// column the decoder ignores. Old and new then decode equal, so the delete
/// half of the update pair erased the row until something else touched it.
#[test]
fn a_task_update_in_an_ignored_column_keeps_the_new_row_insert_first() {
    let rows = SharedRows::new();
    let old = task("same");
    let mut new = task("same");
    new.pr_learnings_gate_shown_at = "2026-11-02 08:40:00".into();
    rows.upsert_task(&old);
    rows.upsert_task(&new);
    rows.retire_task(&old);
    assert!(rows.task(TaskId(7)).is_some());
}

#[test]
fn a_task_update_in_an_ignored_column_keeps_the_new_row_delete_first() {
    let rows = SharedRows::new();
    let old = task("same");
    let mut new = task("same");
    new.pr_learnings_gate_shown_at = "2026-11-02 08:40:00".into();
    rows.upsert_task(&old);
    rows.retire_task(&old);
    rows.upsert_task(&new);
    assert!(rows.task(TaskId(7)).is_some());
}

#[test]
fn an_epic_update_in_an_ignored_column_keeps_the_new_row() {
    let rows = SharedRows::new();
    let old = epic("same");
    let mut new = epic("same");
    // `created_by` is a column the epic decoder ignores.
    new.created_by = "someone-else".into();
    rows.upsert_epic(&old);
    rows.upsert_epic(&new);
    rows.retire_epic(&old);
    assert!(rows.epic(EpicId(7)).is_some());
}
