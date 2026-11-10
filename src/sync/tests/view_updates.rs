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

/// Y2: an agent update in `stop_pending_at`, a column the decoder drops, must
/// not be erased by the delete half of the pair.
#[test]
fn an_agent_update_in_an_ignored_column_keeps_the_new_row() {
    let agent = |stop_pending_at: &str| bindings::Agent {
        id: 4,
        task_id: 9,
        name: "a".into(),
        host: "host-a".into(),
        start_mode: "research".into(),
        state: "current".into(),
        started_at: "2026-10-16 10:00:00.000".into(),
        stop_pending_at: stop_pending_at.into(),
        ..dispatch_spacetime_module::blank_agent().into()
    };
    let (old, new) = (agent(""), agent("2026-11-03 10:00:00.000"));
    let rows = SharedRows::new();
    rows.upsert_agent(&old);
    rows.upsert_agent(&new);
    rows.retire_agent(&old);
    assert_eq!(rows.agents().len(), 1, "insert first");
    assert!(rows.current_agent(TaskId(9)).is_some(), "index survives");

    let rows = SharedRows::new();
    rows.upsert_agent(&old);
    rows.retire_agent(&old);
    rows.upsert_agent(&new);
    assert_eq!(rows.agents().len(), 1, "delete first");
}

/// Y2: a poll claim re-written in `claimed_at` keeps its row.
#[test]
fn a_poll_owner_update_in_an_ignored_column_keeps_the_new_row() {
    let claim = |claimed_at: &str| bindings::PollOwner {
        id: 3,
        scope: "epic".into(),
        scope_id: 5,
        host: "host-a".into(),
        claimed_at: claimed_at.into(),
    };
    let (old, new) = (claim("2026-11-03 09:00:00"), claim("2026-11-03 10:00:00"));
    let rows = SharedRows::new();
    rows.upsert_poll_owner(&old);
    rows.upsert_poll_owner(&new);
    rows.retire_poll_owner(&old);
    assert!(
        rows.poll_owner(crate::models::PollScopeId::Epic(EpicId(5)))
            .is_some(),
        "insert first"
    );
}

/// Y4: a second undecodable version must keep the mark, whichever half of the
/// update arrives first.
#[test]
fn an_undecodable_to_undecodable_task_update_keeps_the_mark() {
    let bad = |title: &str| {
        let mut t = task(title);
        t.epic_id = 3;
        t.status = "teleported".into();
        t
    };
    let (old, new) = (bad("v1"), bad("v2"));
    let rows = SharedRows::new();
    rows.upsert_task(&old);
    rows.upsert_task(&new);
    rows.retire_task(&old);
    assert_eq!(
        rows.undecodable_task_ids_for_epic(EpicId(3)),
        vec![TaskId(7)]
    );
}

/// Y4: an epic that stops decoding leaves the board instead of lingering in
/// its old form.
#[test]
fn an_epic_that_stops_decoding_is_removed() {
    let rows = SharedRows::new();
    rows.upsert_epic(&epic("fine"));
    let mut bad = epic("bad");
    bad.status = "teleported".into();
    rows.upsert_epic(&bad);
    assert!(rows.epic(EpicId(7)).is_none());
}

fn learning(summary: &str) -> bindings::Learning {
    bindings::Learning {
        id: 7,
        kind: "pitfall".into(),
        summary: summary.into(),
        detail: None,
        scope: "repo".into(),
        scope_ref: Some("/repo".into()),
        tags: "[]".into(),
        status: "approved".into(),
        source_task_id: None,
        upvote_count: 0,
        last_upvoted_at: None,
        created_at: "2026-09-18 10:00:00".into(),
        updated_at: "2026-09-18 10:00:00".into(),
        embedding: None,
        workspace_id: 0,
        author: String::new(),
    }
}

fn retrieval(source: &str) -> bindings::LearningRetrieval {
    bindings::LearningRetrieval {
        id: 7,
        task_id: 1,
        learning_id: 2,
        source: source.into(),
        retrieved_at: "2026-09-18 10:00:00".into(),
    }
}

fn package(created_at: &str) -> bindings::WorkPackage {
    bindings::WorkPackage {
        id: 7,
        title: "pkg".into(),
        created_at: created_at.into(),
        updated_at: "2026-09-18 10:00:00".into(),
        ..dispatch_spacetime_module::blank_work_package().into()
    }
}

fn feed_request_row() -> bindings::FeedRequest {
    bindings::FeedRequest {
        id: 7,
        kind: "refresh".into(),
        host: "host-a".into(),
        state: "requested".into(),
        requested_at: "2026-10-12 12:00:00.000".into(),
        ..dispatch_spacetime_module::blank_feed_request().into()
    }
}

fn link(kind: &str) -> bindings::WorkPackageLink {
    bindings::WorkPackageLink {
        id: "7:task:3".into(),
        work_package_id: 7,
        kind: kind.into(),
        item_id: 3,
        created_at: "2026-09-18 10:00:00".into(),
    }
}

/// M15: the learning, retrieval, package and link rows follow the task and
/// epic rule. A row that stops decoding leaves the board.
#[test]
fn a_learning_that_stops_decoding_is_removed() {
    let rows = SharedRows::new();
    rows.upsert_learning(&learning("fine"));
    let mut bad = learning("bad");
    bad.kind = "teleported".into();
    rows.upsert_learning(&bad);
    assert!(rows.learning(crate::models::LearningId(7)).is_none());
}

#[test]
fn a_learning_retrieval_that_stops_decoding_is_removed() {
    let rows = SharedRows::new();
    rows.upsert_learning_retrieval(&retrieval("query_learnings"));
    rows.upsert_learning_retrieval(&retrieval("teleported"));
    assert!(rows.retrievals_for_task(TaskId(1)).is_empty());
}

#[test]
fn a_work_package_that_stops_decoding_is_removed() {
    let rows = SharedRows::new();
    rows.upsert_work_package(&package("2026-09-18 10:00:00"));
    rows.upsert_work_package(&package("not a time"));
    assert!(rows.work_packages().is_empty());
}

#[test]
fn a_work_package_link_that_stops_decoding_is_removed() {
    let rows = SharedRows::new();
    rows.upsert_work_package_link(&link("task"));
    rows.upsert_work_package_link(&link("teleported"));
    assert!(rows.work_package_links().is_empty());
}

// ---------------------------------------------------------------------------
// G5: an update in a column the decoder drops, for every table behind a view
// ---------------------------------------------------------------------------
//
// `retire_*` must compare the RAW row that left, not its decoded form: two rows
// that differ only in a dropped column decode equal, so a decoded compare lets
// the delete half of the update pair erase the row the insert just stored
// (task 98793; Y2; and `work_package_links.created_at` before this table).
// One case per table; `every_view_backed_table_has_an_ignored_column_case`
// fails when `rows.rs` gains a `retire_*` that is in neither list below.

/// An update pair `old` -> `new` that differs only in a column the table's
/// decoder drops: the row must survive in either arrival order.
fn survives_in_either_order<R>(
    table: &str,
    old: R,
    new: R,
    upsert: impl Fn(&SharedRows, &R),
    retire: impl Fn(&SharedRows, &R),
    present: impl Fn(&SharedRows) -> bool,
) {
    let rows = SharedRows::new();
    upsert(&rows, &old);
    upsert(&rows, &new);
    retire(&rows, &old);
    assert!(present(&rows), "{table}: the row was erased, insert first");

    let rows = SharedRows::new();
    upsert(&rows, &old);
    retire(&rows, &old);
    upsert(&rows, &new);
    assert!(present(&rows), "{table}: the row was erased, delete first");
}

/// Table name (the `retire_<name>` suffix) and its case.
const IGNORED_COLUMN_CASES: &[(&str, fn())] = &[
    ("task", || {
        let old = task("same");
        let mut new = task("same");
        new.pr_learnings_gate_shown_at = "2026-11-02 08:40:00".into();
        survives_in_either_order(
            "task",
            old,
            new,
            |r, x| r.upsert_task(x),
            |r, x| r.retire_task(x),
            |r| r.task(TaskId(7)).is_some(),
        );
    }),
    ("epic", || {
        let old = epic("same");
        let mut new = epic("same");
        new.created_by = "someone-else".into();
        survives_in_either_order(
            "epic",
            old,
            new,
            |r, x| r.upsert_epic(x),
            |r, x| r.retire_epic(x),
            |r| r.epic(EpicId(7)).is_some(),
        );
    }),
    ("agent", || {
        let agent = |stop_pending_at: &str| bindings::Agent {
            id: 4,
            task_id: 9,
            name: "a".into(),
            host: "host-a".into(),
            start_mode: "research".into(),
            state: "current".into(),
            started_at: "2026-10-16 10:00:00.000".into(),
            stop_pending_at: stop_pending_at.into(),
            ..dispatch_spacetime_module::blank_agent().into()
        };
        survives_in_either_order(
            "agent",
            agent(""),
            agent("2026-11-03 10:00:00.000"),
            |r, x| r.upsert_agent(x),
            |r, x| r.retire_agent(x),
            |r| r.agents().len() == 1,
        );
    }),
    ("poll_owner", || {
        let claim = |claimed_at: &str| bindings::PollOwner {
            id: 3,
            scope: "epic".into(),
            scope_id: 5,
            host: "host-a".into(),
            claimed_at: claimed_at.into(),
        };
        survives_in_either_order(
            "poll_owner",
            claim("2026-11-03 09:00:00"),
            claim("2026-11-03 10:00:00"),
            |r, x| r.upsert_poll_owner(x),
            |r, x| r.retire_poll_owner(x),
            |r| {
                r.poll_owner(crate::models::PollScopeId::Epic(EpicId(5)))
                    .is_some()
            },
        );
    }),
    ("learning", || {
        // `embedding` is not on the decoded `Learning`.
        let mut new = learning("same");
        new.embedding = Some(vec![1, 2, 3, 4]);
        survives_in_either_order(
            "learning",
            learning("same"),
            new,
            |r, x| r.upsert_learning(x),
            |r, x| r.retire_learning(x),
            |r| r.learning(crate::models::LearningId(7)).is_some(),
        );
    }),
    ("work_package_link", || {
        // `created_at` is not on the decoded link.
        let at = |created_at: &str| bindings::WorkPackageLink {
            created_at: created_at.into(),
            ..link("task")
        };
        survives_in_either_order(
            "work_package_link",
            at("2026-09-18 10:00:00"),
            at("2026-09-18 11:00:00"),
            |r, x| r.upsert_work_package_link(x),
            |r, x| r.retire_work_package_link(x),
            |r| r.work_package_links().len() == 1,
        );
    }),
    ("work_package", || {
        // Whole and fractional seconds parse to the same instant.
        let row = |updated_at: &str| bindings::WorkPackage {
            updated_at: updated_at.into(),
            ..package("2026-09-18 10:00:00")
        };
        survives_in_either_order(
            "work_package",
            row("2026-09-18 10:00:00"),
            row("2026-09-18 10:00:00.000"),
            |r, x| r.upsert_work_package(x),
            |r, x| r.retire_work_package(x),
            |r| r.work_packages().len() == 1,
        );
    }),
    ("learning_retrieval", || {
        // Whole and fractional seconds parse to the same instant.
        let row = |retrieved_at: &str| bindings::LearningRetrieval {
            retrieved_at: retrieved_at.into(),
            ..retrieval("query_learnings")
        };
        survives_in_either_order(
            "learning_retrieval",
            row("2026-09-18 10:00:00"),
            row("2026-09-18 10:00:00.000"),
            |r, x| r.upsert_learning_retrieval(x),
            |r, x| r.retire_learning_retrieval(x),
            |r| r.retrievals_for_task(TaskId(1)).len() == 1,
        );
    }),
    ("dispatch_request", || {
        // A pane id where a window name belongs decodes to None.
        let row = |tmux_window: &str| bindings::DispatchRequest {
            id: 7,
            task_id: 7,
            kind: "dispatch".into(),
            host: "host-a".into(),
            mode: "research".into(),
            origin: "epic_chain".into(),
            state: "requested".into(),
            requested_at: "2026-10-11 12:00:00.000".into(),
            deadline: "2026-10-11 12:01:00.000".into(),
            tmux_window: tmux_window.into(),
            ..dispatch_spacetime_module::blank_dispatch_request().into()
        };
        survives_in_either_order(
            "dispatch_request",
            row("%3"),
            row("%4"),
            |r, x| r.upsert_dispatch_request(x),
            |r, x| r.retire_dispatch_request(x),
            |r| r.dispatch_requests().len() == 1,
        );
    }),
    ("session_request", || {
        let row = |tmux_window: &str| bindings::SessionRequest {
            id: 7,
            task_id: 7,
            kind: "wrap_up".into(),
            action: "rebase".into(),
            host: "host-a".into(),
            state: "requested".into(),
            requested_at: "2026-10-12 12:00:00.000".into(),
            deadline: "2026-10-12 12:02:00.000".into(),
            tmux_window: tmux_window.into(),
            ..dispatch_spacetime_module::blank_session_request().into()
        };
        survives_in_either_order(
            "session_request",
            row("%3"),
            row("%4"),
            |r, x| r.upsert_session_request(x),
            |r, x| r.retire_session_request(x),
            |r| r.session_requests().len() == 1,
        );
    }),
    ("feed_request", || {
        let row = |tmux_window: &str| {
            let mut r = feed_request_row();
            r.tmux_window = tmux_window.into();
            r
        };
        survives_in_either_order(
            "feed_request",
            row("%3"),
            row("%4"),
            |r, x| r.upsert_feed_request(x),
            |r, x| r.retire_feed_request(x),
            |r| r.feed_requests().len() == 1,
        );
    }),
    ("unseen_item", || {
        let item = |creator: &str| bindings::UnseenItem {
            kind: "epic".into(),
            item_id: 7,
            workspace_id: 1,
            creator: creator.into(),
            created_at: "2026-09-18 10:00:00".into(),
        };
        survives_in_either_order(
            "unseen_item",
            item("old"),
            item("new"),
            |r, x| r.upsert_unseen_item(x),
            |r, x| r.retire_unseen_item(x),
            |r| r.unseen_items().len() == 1,
        );
    }),
    ("unseen_task", || {
        let row = |subscriber: &str| bindings::UnseenTask {
            id: "7:me".into(),
            task_id: 7,
            subscriber: subscriber.into(),
        };
        survives_in_either_order(
            "unseen_task",
            row("a"),
            row("b"),
            |r, x| r.upsert_unseen_task(x),
            |r, x| r.retire_unseen_task(x),
            |r| r.unseen_task_ids() == vec![TaskId(7)],
        );
    }),
    ("bookmark", || {
        let row = |subscriber: &str| bindings::Bookmark {
            id: "7:me".into(),
            task_id: 7,
            subscriber: subscriber.into(),
        };
        survives_in_either_order(
            "bookmark",
            row("a"),
            row("b"),
            |r, x| r.upsert_bookmark(x),
            |r, x| r.retire_bookmark(x),
            |r| r.bookmarked_task_ids() == vec![TaskId(7)],
        );
    }),
    ("subscription", || {
        let row = |epic_id: i64| bindings::Subscription {
            id: "me:3".into(),
            epic_id,
            subscriber: "me".into(),
        };
        survives_in_either_order(
            "subscription",
            row(3),
            row(4),
            |r, x| r.upsert_subscription(x),
            |r, x| r.retire_subscription(x),
            |r| r.subscribed_epics("me").len() == 1,
        );
    }),
];

/// Tables whose decoder reads every column, or that have no decoder: there is
/// no dropped column to vary. `pins_view_updates!` above covers their update
/// pair. If a decoder starts dropping a column, move the table into
/// `IGNORED_COLUMN_CASES`.
const DECODES_EVERY_COLUMN: &[(&str, &str)] = &[
    ("workspace", "stored raw: no decoder"),
    ("membership", "stored raw: no decoder"),
    ("learning_verdict", "stored raw: no decoder"),
    ("setting", "stored raw: no decoder"),
];

#[test]
fn an_update_in_a_dropped_column_keeps_the_row_in_every_view_backed_table() {
    let mut failed = Vec::new();
    for (table, case) in IGNORED_COLUMN_CASES {
        if std::panic::catch_unwind(case).is_err() {
            failed.push(*table);
        }
    }
    assert!(
        failed.is_empty(),
        "an update in a dropped column erased the row for: {failed:?}"
    );
}

/// Every table behind a view has a `retire_*` in `rows.rs`. One that is in
/// neither list has no dropped-column test: add a case.
#[test]
fn every_view_backed_table_has_an_ignored_column_case() {
    let src = include_str!("../rows.rs");
    let mut tables: Vec<&str> = src
        .match_indices("pub fn retire_")
        .map(|(at, _)| {
            let rest = &src[at + "pub fn retire_".len()..];
            &rest[..rest.find('(').unwrap_or(rest.len())]
        })
        .collect();
    tables.sort_unstable();
    tables.dedup();
    assert!(
        tables.len() >= 15,
        "the scan found too few tables: {tables:?}"
    );
    for table in tables {
        let covered = IGNORED_COLUMN_CASES.iter().any(|(t, _)| *t == table)
            || DECODES_EVERY_COLUMN.iter().any(|(t, _)| *t == table);
        assert!(
            covered,
            "rows.rs has retire_{table} but view_updates.rs has no dropped-column case for it"
        );
    }
}
