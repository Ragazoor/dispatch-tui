//! Test 1 of the phase plan, at its root: a row that arrives from the shared
//! store decodes to the SAME domain value SQLite reads.
//!
//! Every board view is a function of `Vec<Task>` and `Vec<Epic>`, so "renders
//! identically" is "decodes identically" plus the snapshot tests that already
//! pin the rendering. This file carries the first half, and carries it against
//! a real database rather than against hand-written fixtures: the row under
//! test is dumped out of SQLite by the production dump, sentinel-encoded by the
//! production sentinel table, and compared against what SQLite itself returned
//! for the same task.
//!
//! **The one piece of scaffolding is `as_task`/`as_epic`/`as_todo` below**,
//! which assemble a binding struct from a dumped row. In production the SDK
//! hands that struct over already assembled, so there is nothing to test there
//! — but a column added to the module and forgotten here fails the comparison
//! rather than passing quietly, because the field then holds a default that the
//! SQLite side does not.

use crate::db::{
    CreateTaskRequest, CreateTodoRow, Database, EpicCrud, EpicPatch, EpicRead, HostStore, TaskCrud,
    TaskPatch, TaskRead, TodoRead, TodoStore,
};
use crate::models::{TaskStatus, TaskTag, TaskUrl, UrlType, WrapUpMode};
use crate::spacetime::{bindings, dump_from_sqlite, Row, SharedTable, Snapshot};
use crate::sync::decode;

// ---------------------------------------------------------------------------
// Turning a dumped row into the struct the SDK would have handed us
// ---------------------------------------------------------------------------

/// A dumped row's column, with the store's sentinel applied where the snapshot
/// holds a null.
///
/// The sentinel table is the production one (`SharedTable::sentinel_for`), so a
/// column that gains or loses a sentinel changes this test's inputs the same
/// way it changes the real store's rows.
fn cell(table: SharedTable, row: &Row, column: &str) -> serde_json::Value {
    let Some(raw) = row.get(column) else {
        // A module-only column has no SQLite counterpart and so cannot appear
        // in a dump of one. `tasks.owner` is the case: it is filled by the
        // seed's backfill, not by any row SQLite ever held. Standing it at its
        // sentinel here is what that backfill has not yet done.
        assert!(
            table.module_only_columns().contains(&column),
            "the dump has no column {column:?} for {}",
            table.name()
        );
        return table
            .sentinel_for(column)
            .map(|s| s.as_json())
            .unwrap_or(serde_json::Value::Null);
    };
    match table.sentinel_for(column) {
        Some(sentinel) if raw.is_null() => sentinel.as_json(),
        _ => raw.clone(),
    }
}

fn s(table: SharedTable, row: &Row, column: &str) -> String {
    cell(table, row, column)
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn i(table: SharedTable, row: &Row, column: &str) -> i64 {
    cell(table, row, column).as_i64().unwrap_or_default()
}

fn opt_i(table: SharedTable, row: &Row, column: &str) -> Option<i64> {
    cell(table, row, column).as_i64()
}

fn b(table: SharedTable, row: &Row, column: &str) -> bool {
    let value = cell(table, row, column);
    value
        .as_bool()
        .unwrap_or_else(|| value.as_i64().is_some_and(|n| n != 0))
}

pub(super) fn as_task(row: &Row) -> bindings::Task {
    const T: SharedTable = SharedTable::Tasks;
    bindings::Task {
        id: i(T, row, "id"),
        title: s(T, row, "title"),
        description: s(T, row, "description"),
        repo_path: s(T, row, "repo_path"),
        status: s(T, row, "status"),
        worktree: s(T, row, "worktree"),
        tmux_window: s(T, row, "tmux_window"),
        plan_path: s(T, row, "plan_path"),
        epic_id: i(T, row, "epic_id"),
        sub_status: s(T, row, "sub_status"),
        tag: s(T, row, "tag"),
        sort_order: opt_i(T, row, "sort_order"),
        created_at: s(T, row, "created_at"),
        updated_at: s(T, row, "updated_at"),
        base_branch: s(T, row, "base_branch"),
        external_id: s(T, row, "external_id"),
        labels: s(T, row, "labels"),
        last_pre_tool_use_at: s(T, row, "last_pre_tool_use_at"),
        last_notification_at: s(T, row, "last_notification_at"),
        wrap_up_mode: s(T, row, "wrap_up_mode"),
        url: s(T, row, "url"),
        url_type: s(T, row, "url_type"),
        pr_learnings_gate_shown_at: s(T, row, "pr_learnings_gate_shown_at"),
        auto_run_plan: b(T, row, "auto_run_plan"),
        live_subagents: i(T, row, "live_subagents"),
        stop_pending: b(T, row, "stop_pending"),
        stop_pending_at: s(T, row, "stop_pending_at"),
        live_shells: i(T, row, "live_shells"),
        oldest_live_shell_started_at: s(T, row, "oldest_live_shell_started_at"),
        last_peer_message_sent_at: s(T, row, "last_peer_message_sent_at"),
        last_peer_message_received_at: s(T, row, "last_peer_message_received_at"),
        phoenix: b(T, row, "phoenix"),
        host: s(T, row, "host"),
        owner: s(T, row, "owner"),
        completed_at: s(T, row, "completed_at"),
    }
}

pub(super) fn as_epic(row: &Row) -> bindings::Epic {
    const T: SharedTable = SharedTable::Epics;
    bindings::Epic {
        id: i(T, row, "id"),
        title: s(T, row, "title"),
        description: s(T, row, "description"),
        status: s(T, row, "status"),
        plan_path: s(T, row, "plan_path"),
        sort_order: opt_i(T, row, "sort_order"),
        created_at: s(T, row, "created_at"),
        updated_at: s(T, row, "updated_at"),
        auto_dispatch: b(T, row, "auto_dispatch"),
        parent_epic_id: i(T, row, "parent_epic_id"),
        feed_command: s(T, row, "feed_command"),
        feed_interval_secs: i(T, row, "feed_interval_secs"),
        group_by_repo: b(T, row, "group_by_repo"),
        feed_role: s(T, row, "feed_role"),
        origin: s(T, row, "origin"),
        feed_append_only: b(T, row, "feed_append_only"),
        completed_at: s(T, row, "completed_at"),
    }
}

pub(super) fn as_todo(row: &Row) -> bindings::Todo {
    const T: SharedTable = SharedTable::Todos;
    bindings::Todo {
        id: i(T, row, "id"),
        title: s(T, row, "title"),
        done: b(T, row, "done"),
        sort_order: i(T, row, "sort_order"),
        created_at: s(T, row, "created_at"),
        task_id: i(T, row, "task_id"),
        epic_id: i(T, row, "epic_id"),
        parent_id: i(T, row, "parent_id"),
        owner: s(T, row, "owner"),
    }
}

pub(super) fn rows(snapshot: &Snapshot, table: SharedTable) -> Vec<Row> {
    snapshot
        .extract(table)
        .unwrap_or_else(|| panic!("the dump has no {} extract", table.name()))
        .rows
        .clone()
}

// ---------------------------------------------------------------------------
// The comparison
// ---------------------------------------------------------------------------

/// A board with one task of each interesting shape: every optional column set
/// on one of them and left unset on another, so both arms of every sentinel are
/// exercised in one pass.
pub(super) async fn populated_board() -> Database {
    let db = Database::open_in_memory().await.unwrap();
    db.adopt_user_identity("c200e1f4bcae4a1b9f0e7d2a3c5b8e60")
        .await
        .unwrap();

    let epic = db
        .create_epic("An epic", "with a body", None)
        .await
        .unwrap();
    db.patch_epic(
        epic.id,
        &EpicPatch::new()
            .plan_path(Some("/plans/epic.md"))
            .auto_dispatch(true)
            .feed_command(Some("echo hi"))
            .feed_interval_secs(Some(300)),
    )
    .await
    .unwrap();
    // A second epic left as bare as `create_epic` leaves it, so every one of
    // the columns filled above is also seen empty.
    db.create_epic("A bare epic", "", None).await.unwrap();

    let bare = db
        .create_task(CreateTaskRequest {
            title: "Bare",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();
    assert!(db.task_exists(bare).await.unwrap());

    let full = db
        .create_task(CreateTaskRequest {
            title: "Full",
            description: "a description\nwith a newline",
            repo_path: "/repo",
            plan: Some("/plans/full.md"),
            status: TaskStatus::Running,
            base_branch: "develop",
            epic_id: Some(epic.id),
            sort_order: Some(42),
            tag: Some(TaskTag::Feature),
            wrap_up_mode: Some(WrapUpMode::Rebase),
            auto_run_plan: true,
            phoenix: true,
        })
        .await
        .unwrap();
    db.patch_task(
        full,
        &TaskPatch::new()
            .worktree(Some("/repo/.worktrees/full"))
            .host(Some("machine-a"))
            .external_id(Some("ext-1"))
            .labels(&["one".to_string(), "two".to_string()])
            .url(Some(&TaskUrl::new(
                "https://example.test/pull/1".to_string(),
                UrlType::Pr,
            ))),
    )
    .await
    .unwrap();

    db.insert_todo(CreateTodoRow {
        title: "Linked and owned",
        task_id: Some(full.0),
        epic_id: None,
        owner: Some("c200e1f4bcae4a1b9f0e7d2a3c5b8e60"),
    })
    .await
    .unwrap();
    db.insert_todo(CreateTodoRow {
        title: "Bare",
        task_id: None,
        epic_id: None,
        owner: None,
    })
    .await
    .unwrap();

    db
}

/// **The claim of this whole phase, in one assertion.**
///
/// Every task on a populated board, taken the long way round — SQLite, dump,
/// sentinels, the shared store's row shape, and back — is the task SQLite read
/// directly. Field for field, including the ones nothing on screen renders.
#[tokio::test]
async fn every_task_decodes_to_exactly_what_sqlite_read() {
    let db = populated_board().await;
    let snapshot = dump_from_sqlite(&db).await.unwrap();

    let from_sqlite = db.list_all().await.unwrap();
    assert!(
        from_sqlite.len() >= 2,
        "the fixture must exercise both arms"
    );

    for row in rows(&snapshot, SharedTable::Tasks) {
        let decoded = decode::task(&as_task(&row)).unwrap();
        let expected = from_sqlite
            .iter()
            .find(|t| t.id == decoded.id)
            .unwrap_or_else(|| panic!("no SQLite task {:?}", decoded.id));
        assert_eq!(&decoded, expected);
    }
}

#[tokio::test]
async fn every_epic_decodes_to_exactly_what_sqlite_read() {
    let db = populated_board().await;
    let snapshot = dump_from_sqlite(&db).await.unwrap();

    let from_sqlite = db.list_epics().await.unwrap();
    assert!(from_sqlite.len() >= 2);

    for row in rows(&snapshot, SharedTable::Epics) {
        let decoded = decode::epic(&as_epic(&row)).unwrap();
        let expected = from_sqlite
            .iter()
            .find(|e| e.id == decoded.id)
            .unwrap_or_else(|| panic!("no SQLite epic {:?}", decoded.id));
        assert_eq!(&decoded, expected);
    }
}

#[tokio::test]
async fn every_todo_decodes_to_exactly_what_sqlite_read() {
    let db = populated_board().await;
    let snapshot = dump_from_sqlite(&db).await.unwrap();

    let from_sqlite = db.list_todos().await.unwrap();
    assert_eq!(from_sqlite.len(), 2);

    for row in rows(&snapshot, SharedTable::Todos) {
        let decoded = decode::todo(&as_todo(&row)).unwrap();
        let expected = from_sqlite
            .iter()
            .find(|t| t.id == decoded.id)
            .unwrap_or_else(|| panic!("no SQLite todo {:?}", decoded.id));
        assert_eq!(&decoded, expected);
    }
}

// ---------------------------------------------------------------------------
// The sentinels, stated directly
// ---------------------------------------------------------------------------

/// A blank string is absence, not a blank value.
///
/// Asserted on its own because the round trip above can only show that the two
/// decoders AGREE. If both read `""` as `Some("")` the comparison still passes
/// and every card grows an empty worktree.
#[tokio::test]
async fn an_empty_string_column_is_absence() {
    let db = populated_board().await;
    let snapshot = dump_from_sqlite(&db).await.unwrap();
    let bare = rows(&snapshot, SharedTable::Tasks)
        .into_iter()
        .map(|row| as_task(&row))
        .find(|row| row.title == "Bare")
        .unwrap();

    assert_eq!(bare.worktree, "", "the fixture must carry the sentinel");
    let decoded = decode::task(&bare).unwrap();

    assert_eq!(decoded.worktree, None);
    assert_eq!(decoded.host, None);
    assert_eq!(decoded.plan_path, None);
    assert_eq!(decoded.external_id, None);
    assert_eq!(decoded.tag, None);
    assert_eq!(decoded.url, None);
    assert_eq!(decoded.tmux_window, None);
    assert_eq!(decoded.last_notification_at, None);
}

/// Zero is absence for a foreign key, because SpacetimeDB's `auto_inc` starts
/// at 1 and zero is the value that asks for a fresh id.
#[tokio::test]
async fn a_zero_foreign_key_is_absence() {
    let db = populated_board().await;
    let snapshot = dump_from_sqlite(&db).await.unwrap();
    let bare = rows(&snapshot, SharedTable::Tasks)
        .into_iter()
        .map(|row| as_task(&row))
        .find(|row| row.title == "Bare")
        .unwrap();

    assert_eq!(bare.epic_id, 0);
    assert_eq!(decode::task(&bare).unwrap().epic_id, None);
}

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
/// bargain `collect_decodable` already makes for SQLite's bulk reads.
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
        description: String::new(),
        repo_path: "/repo".into(),
        status: "backlog".into(),
        worktree: String::new(),
        tmux_window: String::new(),
        plan_path: String::new(),
        epic_id: 0,
        sub_status: "none".into(),
        tag: String::new(),
        sort_order: None,
        created_at: "2026-09-18 10:00:00".into(),
        updated_at: "2026-09-18 10:00:00".into(),
        base_branch: "main".into(),
        external_id: String::new(),
        labels: String::new(),
        last_pre_tool_use_at: String::new(),
        last_notification_at: String::new(),
        wrap_up_mode: String::new(),
        url: String::new(),
        url_type: String::new(),
        pr_learnings_gate_shown_at: String::new(),
        auto_run_plan: false,
        live_subagents: 0,
        stop_pending: false,
        stop_pending_at: String::new(),
        live_shells: 0,
        oldest_live_shell_started_at: String::new(),
        last_peer_message_sent_at: String::new(),
        last_peer_message_received_at: String::new(),
        phoenix: false,
        host: String::new(),
        owner: String::new(),
        completed_at: String::new(),
    }
}

fn blank_epic() -> bindings::Epic {
    bindings::Epic {
        id: 1,
        title: "Blank".into(),
        description: String::new(),
        status: "backlog".into(),
        plan_path: String::new(),
        sort_order: None,
        created_at: "2026-09-18 10:00:00".into(),
        updated_at: "2026-09-18 10:00:00".into(),
        auto_dispatch: false,
        parent_epic_id: 0,
        feed_command: String::new(),
        feed_interval_secs: 0,
        group_by_repo: false,
        feed_role: "none".into(),
        origin: "manual".into(),
        feed_append_only: false,
        completed_at: String::new(),
    }
}

pub(super) fn as_repo_path(row: &Row) -> bindings::RepoPath {
    const T: SharedTable = SharedTable::RepoPaths;
    bindings::RepoPath {
        id: i(T, row, "id"),
        path: s(T, row, "path"),
        last_used: s(T, row, "last_used"),
        verify_command: s(T, row, "verify_command"),
    }
}

pub(super) fn as_repo_base_branch(row: &Row) -> bindings::RepoBaseBranch {
    const T: SharedTable = SharedTable::RepoBaseBranches;
    bindings::RepoBaseBranch {
        id: i(T, row, "id"),
        repo_path: s(T, row, "repo_path"),
        branch: s(T, row, "branch"),
        last_used: s(T, row, "last_used"),
    }
}

pub(super) fn as_host(row: &Row) -> bindings::Host {
    const T: SharedTable = SharedTable::Hosts;
    bindings::Host {
        id: s(T, row, "id"),
        label: s(T, row, "label"),
        owner: s(T, row, "owner"),
    }
}
