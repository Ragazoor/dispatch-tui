//! Domain types into store rows — the mirror of `tests::decode`.
//!
//! What is worth testing here is not the field-for-field copy, which the
//! compiler already checks: it is the two places the two sides disagree about
//! how to say "nothing", and the one place a single field becomes two columns.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::db::{CreateTaskRequest, TaskPatch};
use crate::models::{SubStatus, TaskStatus, TaskTag, TaskUrl, TmuxWindow, UrlType};
use crate::sync::encode;

/// A real timestamp in the format both stores write. Not "now": a placeholder
/// that never reaches a parser in one test reaches one in the next, and the
/// failure reads as a decoder bug rather than as a bad fixture.
const NOW: &str = "2026-09-19 00:00:00.000";

fn a_request() -> CreateTaskRequest<'static> {
    CreateTaskRequest {
        title: "t",
        description: "d",
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
    }
}

/// A created row asks the store for its id. Sending one would be choosing an id
/// the store has not reserved, and the next generated id would collide with it.
#[test]
fn a_created_row_carries_no_id() {
    let row = encode::create_task_row(&a_request(), "me", NOW);
    assert_eq!(row.id, 0);
}

/// `core.allium: OwnerTracksUserBoardTask`, both arms. A task with no epic says
/// whose board it is on; a task with one must not, or the same card is on two
/// boards and neither reader looks wrong locally.
#[test]
fn the_owner_is_set_exactly_when_there_is_no_epic() {
    let on_my_board = encode::create_task_row(&a_request(), "me", NOW);
    assert_eq!(on_my_board.owner, "me");
    assert_eq!(on_my_board.epic_id, 0);

    let in_an_epic = encode::create_task_row(
        &CreateTaskRequest {
            epic_id: Some(crate::models::EpicId(7)),
            ..a_request()
        },
        "me",
        NOW,
    );
    assert_eq!(in_an_epic.owner, "");
    assert_eq!(in_an_epic.epic_id, 7);
}

/// An empty label list is `[]`, not `""`. The column holds JSON and the decoder
/// parses it, so the empty string is a parse failure rather than an empty list
/// — a task that would be dropped from the board instead of drawn with no
/// labels.
#[test]
fn a_task_with_no_labels_carries_an_empty_json_array() {
    let row = encode::create_task_row(&a_request(), "me", NOW);
    assert_eq!(row.labels, "[]");
    assert_eq!(
        crate::sync::decode::task(&row).unwrap().labels,
        Vec::<String>::new()
    );
}

/// The sub-status follows the status rather than being defaulted separately.
#[test]
fn the_sub_status_matches_the_status_it_was_created_in() {
    let row = encode::create_task_row(
        &CreateTaskRequest {
            status: TaskStatus::Running,
            ..a_request()
        },
        "me",
        NOW,
    );
    assert_eq!(
        row.sub_status,
        SubStatus::default_for(TaskStatus::Running).as_str()
    );
}

// -- The two absences -------------------------------------------------------

/// THE ONE THAT MATTERS. `None` and `Some(None)` both look like absence and
/// mean opposite things: leave the worktree alone, versus release it. Collapsing
/// them leaves a task the board thinks was released still holding a checkout.
#[test]
fn untouched_and_cleared_are_not_the_same_patch() {
    let untouched = encode::task_patch(&TaskPatch::new());
    assert_eq!(untouched.worktree, None);

    let cleared = encode::task_patch(&TaskPatch::new().worktree(None));
    assert_eq!(cleared.worktree, Some(String::new()));

    let set = encode::task_patch(&TaskPatch::new().worktree(Some("/wt")));
    assert_eq!(set.worktree, Some("/wt".to_string()));
}

/// The same three-way distinction on a field the module stores as an enum
/// string.
#[test]
fn a_cleared_tag_is_the_empty_string_not_an_untouched_field() {
    assert_eq!(encode::task_patch(&TaskPatch::new()).tag, None);
    assert_eq!(
        encode::task_patch(&TaskPatch::new().tag(None)).tag,
        Some(String::new())
    );
    assert_eq!(
        encode::task_patch(&TaskPatch::new().tag(Some(TaskTag::Bug))).tag,
        Some(TaskTag::Bug.as_str().to_string())
    );
}

/// `sort_order` stays doubly optional all the way through, and is the only
/// field that does. Zero is a real sort order — the top of a column — so a
/// sentinel would silently reorder cards.
#[test]
fn sort_order_keeps_both_levels_of_absence() {
    assert_eq!(encode::task_patch(&TaskPatch::new()).sort_order, None);
    assert_eq!(
        encode::task_patch(&TaskPatch::new().sort_order(None)).sort_order,
        Some(None)
    );
    assert_eq!(
        encode::task_patch(&TaskPatch::new().sort_order(Some(0))).sort_order,
        Some(Some(0))
    );
}

/// An untouched patch touches nothing at all. Asserted over the fields most
/// likely to gain an accidental default, because a patch that quietly set one
/// would clobber a colleague's concurrent edit to it.
#[test]
fn an_empty_patch_names_no_field() {
    let empty = encode::task_patch(&TaskPatch::new());
    assert_eq!(empty.title, None);
    assert_eq!(empty.status, None);
    assert_eq!(empty.sub_status, None);
    assert_eq!(empty.host, None);
    assert_eq!(empty.completed_at, None);
    assert_eq!(empty.auto_run_plan, None);
    assert_eq!(empty.phoenix, None);
    assert_eq!(empty.labels, None);
}

// -- One field, two columns -------------------------------------------------

/// The url and its type move together or not at all. A patch that set one and
/// left the other produces the `inconsistent url=.. url_type=..` row the
/// decoder refuses — a task that vanishes from the board with no other symptom.
#[test]
fn a_url_patch_moves_both_columns() {
    let set = encode::task_patch(
        &TaskPatch::new().url(Some(&TaskUrl::new("https://example/pr/1", UrlType::Pr))),
    );
    assert_eq!(set.url, Some("https://example/pr/1".to_string()));
    assert_eq!(set.url_type, Some(UrlType::Pr.as_str().to_string()));

    let cleared = encode::task_patch(&TaskPatch::new().url(None));
    assert_eq!(cleared.url, Some(String::new()));
    assert_eq!(cleared.url_type, Some(String::new()));

    let untouched = encode::task_patch(&TaskPatch::new());
    assert_eq!(untouched.url, None);
    assert_eq!(untouched.url_type, None);
}

/// The counters have dedicated writers so no handler can desync them, and the
/// patch route deliberately does not reach them. A patch that could would undo
/// that guarantee at the seam.
#[test]
fn the_denormalised_counters_are_not_patchable() {
    let everything = TaskPatch::new()
        .title("t")
        .status(TaskStatus::Running)
        .worktree(Some("/wt"));
    let patch = encode::task_patch(&everything);
    assert_eq!(patch.live_subagents, None);
    assert_eq!(patch.live_shells, None);
    assert_eq!(patch.stop_pending_at, None);
    assert_eq!(patch.oldest_live_shell_started_at, None);
    // `epic_id` and `owner` are the other two, held out because moving a task
    // between epics has to recalculate both and because the owner is tied to
    // the epic being absent.
    assert_eq!(patch.epic_id, None);
    assert_eq!(patch.owner, None);
}

/// A tmux window round-trips through the same string form the decoder parses.
#[test]
fn a_tmux_window_encodes_the_way_the_decoder_reads_it() {
    let window = TmuxWindow::for_task(crate::models::TaskId(1));
    let patch = encode::task_patch(&TaskPatch::new().tmux_window(Some(&window)));
    let encoded = patch.tmux_window.clone().unwrap();

    let row = crate::spacetime::bindings::Task {
        tmux_window: encoded,
        ..encode::create_task_row(&a_request(), "me", NOW)
    };
    assert_eq!(
        crate::sync::decode::task(&row).unwrap().tmux_window,
        Some(window)
    );
}

/// A timestamp encodes to the string the decoder parses back to the same
/// instant. The two stores' rows have to compare equal, and a format that
/// drifted by one character would pass every test that reads it back through
/// the same parser.
#[test]
fn a_timestamp_round_trips_through_the_decoder() {
    let at = chrono::DateTime::parse_from_rfc3339("2026-09-19T12:34:56.789Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let patch = encode::task_patch(&TaskPatch::new().completed_at(Some(at)));

    let row = crate::spacetime::bindings::Task {
        completed_at: patch.completed_at.clone().unwrap(),
        ..encode::create_task_row(&a_request(), "me", NOW)
    };
    assert_eq!(
        crate::sync::decode::task(&row).unwrap().completed_at,
        Some(at)
    );
}

/// A todo's position is the STORE's to choose, so the client sends none.
///
/// It cannot choose one: the bottom of a checklist is one past the highest
/// order on it, and this board sees only what it subscribes to. Sending a zero
/// — which this did once — puts every new todo at the TOP of any list that has
/// ever been reordered by hand, because reads order by `sort_order` ascending.
#[test]
fn a_created_todo_leaves_its_position_to_the_store() {
    let row = encode::create_todo_row(
        &crate::db::CreateTodoRow {
            title: "t",
            task_id: None,
            epic_id: None,
            owner: Some("user-me"),
        },
        NOW,
    );
    assert_eq!(
        row.sort_order, 0,
        "the placeholder the reducer overwrites; see create_todo in the module"
    );
    assert_eq!(row.owner, "user-me");
}

// -- The sentinels agree with the snapshot's declared ones -------------------

/// A CLEARED FIELD MUST USE THE SENTINEL THE SNAPSHOT DECLARES FOR IT.
///
/// The encoder names each sentinel at its own call site — `String::new()` or
/// `0` — on the grounds that only the compiler can tell which type a column
/// wants. That is right, and it leaves nothing checking the two agree.
/// `SharedTable::sentinel_columns` is the authority; this pins the encoder to
/// it for every column a patch can clear.
///
/// The `Zero` columns are where a mix-up would land, and where it would be
/// worst: `parent_epic_id: Some(0)` reads as "epic 0" rather than as "no
/// parent", which is exactly the confusion `snapshot.rs` warns about.
#[test]
fn a_cleared_field_uses_the_sentinel_the_snapshot_declares() {
    use crate::spacetime::{Sentinel, SharedTable};

    let zero = serde_json::Value::from(0);
    let empty = serde_json::Value::String(String::new());

    // Tasks: every clearable column the encoder writes, with what it writes.
    let cleared = encode::task_patch(
        &TaskPatch::new()
            .worktree(None)
            .tmux_window(None)
            .plan_path(None)
            .tag(None)
            .external_id(None)
            .wrap_up_mode(None)
            .url(None)
            .host(None)
            .completed_at(None)
            .last_pre_tool_use_at(None)
            .last_notification_at(None)
            .last_peer_message_sent_at(None)
            .last_peer_message_received_at(None),
    );
    for (column, written) in [
        ("worktree", cleared.worktree.clone()),
        ("tmux_window", cleared.tmux_window.clone()),
        ("plan_path", cleared.plan_path.clone()),
        ("tag", cleared.tag.clone()),
        ("external_id", cleared.external_id.clone()),
        ("wrap_up_mode", cleared.wrap_up_mode.clone()),
        ("url", cleared.url.clone()),
        ("url_type", cleared.url_type.clone()),
        ("host", cleared.host.clone()),
        ("completed_at", cleared.completed_at.clone()),
        ("last_pre_tool_use_at", cleared.last_pre_tool_use_at.clone()),
        ("last_notification_at", cleared.last_notification_at.clone()),
        (
            "last_peer_message_sent_at",
            cleared.last_peer_message_sent_at.clone(),
        ),
        (
            "last_peer_message_received_at",
            cleared.last_peer_message_received_at.clone(),
        ),
    ] {
        let declared = SharedTable::Tasks
            .sentinel_for(column)
            .unwrap_or_else(|| panic!("tasks.{column} is not a declared sentinel column"));
        assert_eq!(
            declared,
            Sentinel::EmptyString,
            "tasks.{column} changed sentinel type; the encoder writes a string"
        );
        assert_eq!(
            written.as_deref(),
            Some(""),
            "clearing tasks.{column} must write the empty-string sentinel"
        );
        assert_eq!(declared.as_json(), empty);
    }

    // Epics: the one `Zero` column a patch can clear, and its string siblings.
    let cleared = encode::epic_patch(
        &crate::db::EpicPatch::new()
            .parent_epic_id(None)
            .feed_interval_secs(None)
            .feed_command(None)
            .plan_path(None)
            .completed_at(None),
    );
    assert_eq!(
        SharedTable::Epics.sentinel_for("parent_epic_id"),
        Some(Sentinel::Zero)
    );
    assert_eq!(cleared.parent_epic_id, Some(0));
    assert_eq!(
        SharedTable::Epics.sentinel_for("feed_interval_secs"),
        Some(Sentinel::Zero)
    );
    assert_eq!(cleared.feed_interval_secs, Some(0));
    assert_eq!(Sentinel::Zero.as_json(), zero);
    for (column, written) in [
        ("feed_command", cleared.feed_command.clone()),
        ("plan_path", cleared.plan_path.clone()),
        ("completed_at", cleared.completed_at.clone()),
    ] {
        assert_eq!(
            SharedTable::Epics.sentinel_for(column),
            Some(Sentinel::EmptyString),
            "epics.{column}"
        );
        assert_eq!(written.as_deref(), Some(""), "epics.{column}");
    }

    // Todos: all three clearable columns are `Zero`, which is the set most
    // likely to be given an empty string by a copy-paste.
    let cleared = encode::todo_patch(
        &crate::db::TodoPatch::new()
            .task_id(None)
            .epic_id(None)
            .parent_id(None),
    );
    for (column, written) in [
        ("task_id", cleared.task_id),
        ("epic_id", cleared.epic_id),
        ("parent_id", cleared.parent_id),
    ] {
        assert_eq!(
            SharedTable::Todos.sentinel_for(column),
            Some(Sentinel::Zero),
            "todos.{column}"
        );
        assert_eq!(written, Some(0), "clearing todos.{column} must write 0");
    }
}
