//! Feed ingestion, retired feed items and the delete guards, run against the
//! in-memory store. Twins of `tests/spacetime_module/feeds.rs`.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

const CREATOR: &str = "test-creator";

fn feed_item(external_id: &str, title: &str, status: &str) -> FeedTaskUpsertItem {
    FeedTaskUpsertItem {
        external_id: external_id.into(),
        title: title.into(),
        description: String::new(),
        repo_path: "/repo".into(),
        status: status.into(),
        sub_status: "none".into(),
        base_branch: "main".into(),
        tag: "chore".into(),
        labels: "[]".into(),
        sort_order: None,
        url: String::new(),
        url_type: String::new(),
        wrap_up_mode: String::new(),
    }
}

fn ingest(store: &Store, epic: i64, items: Vec<FeedTaskUpsertItem>, created_by: &str) {
    let wrapped = items
        .into_iter()
        .map(|item| FeedIngestItem {
            item,
            signals: vec![],
        })
        .collect();
    store
        .call("alice", |ctx| {
            ingest_feed(ctx, epic, wrapped, false, created_by.into())
        })
        .expect("ingest_feed");
}

/// A top-level (or nested) epic that carries a `feed_command`, which is what
/// makes it the key a retirement is written under.
fn feed_epic(store: &Store, title: &str, parent: i64) -> i64 {
    plain_epic_with(store, title, parent, "echo []")
}

fn plain_epic(store: &Store, title: &str, parent: i64) -> i64 {
    plain_epic_with(store, title, parent, "")
}

fn plain_epic_with(store: &Store, title: &str, parent: i64, feed_command: &str) -> i64 {
    let row = Epic {
        title: title.into(),
        parent_epic_id: parent,
        feed_command: feed_command.into(),
        workspace_id: TEST_WORKSPACE,
        ..blank_epic()
    };
    store
        .call("alice", |ctx| create_epic(ctx, row))
        .expect("create_epic");
    store
        .db()
        .epics()
        .iter()
        .map(|e| e.id)
        .max()
        .expect("an epic")
}

/// A feed task: a task row carrying `external_id`, in `epic`.
fn feed_task(store: &Store, external_id: &str, status: &str, epic: i64) -> i64 {
    let row = Task {
        external_id: external_id.into(),
        ..epic_task(external_id, epic, status)
    };
    store.create("alice", row)
}

fn retire_seed(store: &Store, rows: &[(i64, i64, &str)]) {
    let rows: Vec<RetiredFeedItem> = rows
        .iter()
        .map(|&(id, feed_epic_id, external_id)| RetiredFeedItem {
            id,
            feed_epic_id,
            external_id: external_id.into(),
            retired_at: "2026-09-19 10:00:00".into(),
        })
        .collect();
    store
        .call("alice", |ctx| seed_retired_feed_items(ctx, rows))
        .expect("seed_retired_feed_items");
}

fn retired_count(store: &Store, pred: impl Fn(&RetiredFeedItem) -> bool) -> usize {
    store
        .db()
        .retired_feed_items()
        .iter()
        .filter(|r| pred(r))
        .count()
}

fn by_external_id(store: &Store, external_id: &str) -> Vec<Task> {
    store
        .db()
        .tasks()
        .iter()
        .filter(|t| t.external_id == external_id)
        .collect()
}

fn task_exists(store: &Store, id: i64) -> bool {
    store.db().tasks().id().find(id).is_some()
}

/// The two-part contract of a re-poll: fields the feed owns move, fields the
/// user or the store own do not.
#[test]
fn feed_upsert_updates_feed_fields_and_preserves_user_and_store_fields() {
    let store = Store::new();
    let epic = plain_epic(&store, "E", 0);
    ingest(
        &store,
        epic,
        vec![feed_item("ext-1", "original title", "backlog")],
        CREATOR,
    );
    let id = by_external_id(&store, "ext-1")[0].id;

    // The user moves it to Running and the store gives it a worktree.
    store
        .call("alice", |ctx| {
            patch_task(
                ctx,
                id,
                TaskPatch {
                    status: Some("running".into()),
                    worktree: Some("wt-1".into()),
                    ..TaskPatch::default()
                },
            )
        })
        .expect("patch_task");

    ingest(
        &store,
        epic,
        vec![feed_item("ext-1", "updated title", "backlog")],
        CREATOR,
    );

    let task = store.task(id);
    assert_eq!(
        task.title, "updated title",
        "the feed-owned field must update"
    );
    assert_eq!(
        task.status, "running",
        "a re-poll must not move a user-managed status"
    );
    assert_eq!(
        task.worktree, "wt-1",
        "a re-poll must not touch a store-owned field"
    );
    assert_eq!(
        task.completed_at, "",
        "a never-done task gains no completion stamp"
    );
}

/// An existing non-null url always wins over whatever the item carries; url
/// and url_type move together, never one without the other.
#[test]
fn feed_upsert_keeps_an_existing_url_over_a_re_polled_one() {
    let store = Store::new();
    let epic = plain_epic(&store, "E", 0);
    let mut first = feed_item("ext-1", "t", "backlog");
    first.url = "https://example.com/1".into();
    first.url_type = "pr".into();
    ingest(&store, epic, vec![first], CREATOR);

    let mut second = feed_item("ext-1", "t", "backlog");
    second.url = "https://example.com/2".into();
    second.url_type = "issue".into();
    ingest(&store, epic, vec![second], CREATOR);

    let task = &by_external_id(&store, "ext-1")[0];
    assert_eq!(task.url, "https://example.com/1", "the FIRST url must win");
    assert_eq!(task.url_type, "pr");
}

/// A task born already `done` is stamped complete; a re-poll never re-derives
/// that stamp, matching `write_task`'s own insert-only rule.
#[test]
fn feed_upsert_stamps_completion_only_on_a_genuine_insert() {
    let store = Store::new();
    let epic = plain_epic(&store, "E", 0);
    ingest(&store, epic, vec![feed_item("ext-1", "t", "done")], CREATOR);

    assert_ne!(
        by_external_id(&store, "ext-1")[0].completed_at,
        "",
        "a task created directly into done must be stamped"
    );
}

/// A newly-inserted feed task stamps `created_by` from the caller, and a later
/// re-poll of the same item leaves it untouched (`core.allium: Task.created_by`,
/// `feeds.allium: UpsertFeedTasks`).
#[test]
fn feed_upsert_stamps_created_by_on_insert_only() {
    let store = Store::new();
    let epic = plain_epic(&store, "E", 0);

    ingest(
        &store,
        epic,
        vec![feed_item("ext-1", "t", "backlog")],
        "host-a",
    );
    assert_eq!(
        by_external_id(&store, "ext-1")[0].created_by,
        "host-a",
        "a genuine insert must stamp created_by from the caller"
    );

    ingest(
        &store,
        epic,
        vec![feed_item("ext-1", "updated", "backlog")],
        "host-b",
    );
    assert_eq!(
        by_external_id(&store, "ext-1")[0].created_by,
        "host-a",
        "a re-poll from a different host must not overwrite created_by"
    );
}

// Retired feed items — delete means "I am done with this item"
//
// `core.allium: RetiredFeedItem`, written by `tasks.allium: DeleteTask` and
// `epics.allium: DeleteEpic`, read by `feeds.allium:
// IngestSkipsRetiredFeedItems`. Every retirement is keyed on the FEED EPIC —
// the nearest epic in the chain carrying a `feed_command` — never on the
// sub-epic the task sat in.

/// `tasks.allium: DeleteTask`'s retirement clause: deleting a feed task writes
/// one record keyed on the NEAREST FEED EPIC in its chain, in the same call
/// that removes the row.
#[test]
fn deleting_a_feed_task_retires_it_under_its_nearest_feed_epic() {
    let store = Store::new();
    let feed = feed_epic(&store, "feed", 0);
    let group = plain_epic(&store, "repo group", feed);
    let id = feed_task(&store, "ext-1", "done", group);

    store
        .call("alice", |ctx| delete_task(ctx, id))
        .expect("delete_task");

    assert!(!task_exists(&store, id), "the task row must be gone");
    assert_eq!(
        retired_count(&store, |r| r.feed_epic_id == feed
            && r.external_id == "ext-1"),
        1,
        "the retirement must be keyed on the feed epic, not the sub-epic"
    );
    assert_eq!(
        retired_count(&store, |r| r.feed_epic_id == group),
        0,
        "nothing may be keyed on the sub-epic the task sat in"
    );
    let stamped = store
        .db()
        .retired_feed_items()
        .iter()
        .next()
        .expect("a record")
        .retired_at;
    assert_ne!(
        stamped, "",
        "a retirement is stamped with the store's clock"
    );
}

/// A manual task (no `external_id`) and a feed-shaped task under no feed epic
/// both retire nothing: there is no cycle to suppress.
#[test]
fn deleting_a_manual_task_or_one_under_no_feed_epic_retires_nothing() {
    let store = Store::new();
    let feed = feed_epic(&store, "feed", 0);
    let plain = plain_epic(&store, "plain", 0);
    let manual = store.create("alice", epic_task("manual", feed, "done"));
    let orphan = feed_task(&store, "ext-orphan", "done", plain);

    for id in [manual, orphan] {
        store
            .call("alice", |ctx| delete_task(ctx, id))
            .expect("delete_task");
    }

    assert_eq!(
        store.db().tasks().iter().count(),
        0,
        "both tasks must be deleted"
    );
    assert_eq!(
        store.db().retired_feed_items().iter().count(),
        0,
        "neither a manual task nor one under no feed epic may be retired"
    );
}

/// `core/RetiredFeedItem: UniqueRetiredFeedItemPerFeed`. Deleting a second task
/// that carries an already-retired (feed epic, external_id) is a no-op; the
/// same id under a DIFFERENT feed epic is its own record.
#[test]
fn retiring_the_same_feed_item_twice_writes_one_record() {
    let store = Store::new();
    let feed = feed_epic(&store, "feed", 0);
    let sub_a = plain_epic(&store, "sub-a", feed);
    let sub_b = plain_epic(&store, "sub-b", feed);
    let other_feed = feed_epic(&store, "other feed", 0);

    // Two tasks with one external id in two sub-epics of one feed: both
    // deletes resolve to the same key and the second must not add a record.
    let first = feed_task(&store, "ext-1", "done", sub_a);
    let second = feed_task(&store, "ext-1", "done", sub_b);
    for id in [first, second] {
        store
            .call("alice", |ctx| delete_task(ctx, id))
            .expect("delete_task");
    }
    assert_eq!(
        retired_count(&store, |r| r.feed_epic_id == feed
            && r.external_id == "ext-1"),
        1,
        "a delete of an already-retired id must not write a second record"
    );

    let third = feed_task(&store, "ext-1", "done", other_feed);
    store
        .call("alice", |ctx| delete_task(ctx, third))
        .expect("delete_task");
    assert_eq!(
        retired_count(&store, |r| r.external_id == "ext-1"),
        2,
        "the same id under a different feed epic is a separate record"
    );
}

/// `feeds.allium: IngestSkipsRetiredFeedItems`: an item whose id is retired
/// under the upsert epic's nearest feed epic, with no surviving task, is never
/// inserted — while a non-retired item in the same batch is.
#[test]
fn feed_upsert_refuses_to_insert_a_retired_external_id() {
    let store = Store::new();
    let feed = feed_epic(&store, "feed", 0);
    let group = plain_epic(&store, "repo group", feed);
    retire_seed(&store, &[(100, feed, "ext-retired")]);

    ingest(
        &store,
        feed,
        vec![
            feed_item("ext-retired", "back again", "backlog"),
            feed_item("ext-new-1", "fresh", "backlog"),
        ],
        CREATOR,
    );
    let _ = group;

    assert!(
        by_external_id(&store, "ext-retired").is_empty(),
        "ingest_feed must not insert a retired id"
    );
    assert_eq!(
        by_external_id(&store, "ext-new-1").len(),
        1,
        "ingest_feed must still insert the batch's non-retired item"
    );
}

/// Suppression covers insertion only. A task still on the board with a retired
/// id is matched and refreshed like any other feed task, and the record itself
/// is untouched by the upsert.
#[test]
fn feed_upsert_refreshes_a_surviving_task_whose_id_is_retired() {
    let store = Store::new();
    let feed = feed_epic(&store, "feed", 0);
    let id = feed_task(&store, "ext-1", "done", feed);
    retire_seed(&store, &[(100, feed, "ext-1")]);

    ingest(
        &store,
        feed,
        vec![feed_item("ext-1", "refreshed title", "backlog")],
        CREATOR,
    );

    assert_eq!(
        store.task(id).title,
        "refreshed title",
        "a surviving task with a retired id must still be refreshed"
    );
    assert_eq!(by_external_id(&store, "ext-1").len(), 1, "no duplicate row");
    assert_eq!(
        retired_count(&store, |r| r.feed_epic_id == feed
            && r.external_id == "ext-1"),
        1,
        "an upsert must not touch the retirement record"
    );
}

/// `epics.allium: DeleteEpic`'s retirement clause, feed epic surviving: every
/// feed task anywhere in the deleted subtree — nested sub-epics included — is
/// retired under the surviving feed epic above it; a manual task is not.
#[test]
fn deleting_a_sub_epic_retires_its_whole_subtrees_feed_tasks() {
    let store = Store::new();
    let feed = feed_epic(&store, "feed", 0);
    let sub = plain_epic(&store, "sub", feed);
    let sub_sub = plain_epic(&store, "sub-sub", sub);
    feed_task(&store, "ext-a", "done", sub);
    feed_task(&store, "ext-b", "done", sub_sub);
    store.create("alice", epic_task("manual", sub, "done"));

    store
        .call("alice", |ctx| delete_epic(ctx, sub))
        .expect("delete_epic");

    assert_eq!(
        store.db().tasks().iter().count(),
        0,
        "the whole subtree's tasks go"
    );
    assert!(
        store.db().epics().id().find(feed).is_some(),
        "the feed epic above the deleted sub-epic must survive"
    );
    assert_eq!(
        retired_count(&store, |r| r.feed_epic_id == feed
            && r.external_id == "ext-a"),
        1,
        "a feed task directly in the deleted epic must be retired"
    );
    assert_eq!(
        retired_count(&store, |r| r.feed_epic_id == feed
            && r.external_id == "ext-b"),
        1,
        "a feed task in a nested sub-epic must be retired too"
    );
    assert_eq!(
        store.db().retired_feed_items().iter().count(),
        2,
        "a manual task must not be retired"
    );

    // And the retirement bites: the feed's next cycle does not re-insert.
    ingest(
        &store,
        feed,
        vec![feed_item("ext-a", "a", "backlog")],
        CREATOR,
    );
    assert!(
        by_external_id(&store, "ext-a").is_empty(),
        "a cycle after the delete must not put the item straight back"
    );
}

/// `epics.allium: DeleteEpic`'s guard — every task anywhere in the subtree is
/// `done` — enforced at the reducer, not only in the client's view check.
#[test]
fn deleting_an_epic_with_an_undone_subtree_task_is_refused_at_the_reducer() {
    let store = Store::new();
    let root = plain_epic(&store, "root", 0);
    let sub = plain_epic(&store, "sub", root);
    store.create("alice", epic_task("done one", root, "done"));
    let running = store.create("alice", epic_task("still running", sub, "running"));

    let why = store
        .call("alice", |ctx| delete_epic(ctx, root))
        .expect_err("a subtree holding an undone task must be refused");
    assert!(why.contains("not done"), "the refusal must say why: {why}");
    assert_eq!(store.db().epics().iter().count(), 2, "the subtree stays");
    assert_eq!(store.db().tasks().iter().count(), 2, "every task stays");

    // The same subtree with its task finished is accepted, so the refusal is
    // the rule rather than the reducer being broken.
    store
        .call("alice", |ctx| {
            patch_task(ctx, running, status_patch("done"))
        })
        .expect("patch_task");
    store
        .call("alice", |ctx| delete_epic(ctx, root))
        .expect("delete_epic");
}

/// `tasks.allium: BatchDelete`'s atomic guarantee: one done task and one undone
/// task in a batch refuse the WHOLE call and leave every row untouched.
#[test]
fn batch_delete_refuses_the_whole_batch_when_one_task_is_not_done() {
    let store = Store::new();
    let epic = plain_epic(&store, "E", 0);
    let finished = store.create("alice", epic_task("finished", epic, "done"));
    let running = store.create("alice", epic_task("still running", epic, "running"));

    let why = store
        .call("alice", |ctx| {
            batch_delete(ctx, vec![finished, running], vec![])
        })
        .expect_err("one non-done task must refuse the whole batch");
    assert!(why.contains("not done"), "the refusal must say why: {why}");
    assert!(
        task_exists(&store, finished),
        "a refused batch deletes nothing at all, the done task included"
    );
    assert!(task_exists(&store, running));

    // Once both are done the batch is accepted and removes everything.
    store
        .call("alice", |ctx| {
            patch_task(ctx, running, status_patch("done"))
        })
        .expect("patch_task");
    store
        .call("alice", |ctx| {
            batch_delete(ctx, vec![finished, running], vec![])
        })
        .expect("batch_delete");
    assert_eq!(store.db().tasks().iter().count(), 0);
}

/// The success path over BOTH domains at once: a plain task and a fully-done
/// epic's subtree, deleted together in one `batch_delete` call.
#[test]
fn batch_delete_deletes_a_task_and_an_epic_subtree_together() {
    let store = Store::new();
    let epic = plain_epic(&store, "E", 0);
    let in_epic = store.create("alice", epic_task("in the epic", epic, "done"));
    let plain = store.create(
        "alice",
        Task {
            status: "done".into(),
            ..user_task("plain", "alice")
        },
    );
    let _ = in_epic;

    store
        .call("alice", |ctx| batch_delete(ctx, vec![plain], vec![epic]))
        .expect("batch_delete");

    assert_eq!(
        store.db().tasks().iter().count(),
        0,
        "both the plain task and the epic's subtask must be gone"
    );
    assert_eq!(
        store.db().epics().iter().count(),
        0,
        "the epic must be gone too"
    );
}

/// Deleting an epic detaches its subtasks' learnings and cascades their
/// retrievals, as `delete_task` does: each path that removes task rows must
/// reproduce it.
#[test]
fn deleting_an_epic_detaches_its_subtasks_learnings_and_cascades_their_retrievals() {
    let store = Store::new();
    let epic = plain_epic(&store, "E", 0);
    // `done`: this test is about learning detachment, not the delete guard.
    let task = store.create("alice", epic_task("source", epic, "done"));
    store
        .call("alice", |ctx| {
            seed_learnings(
                ctx,
                vec![Learning {
                    id: 1,
                    kind: "pitfall".into(),
                    summary: "A learning".into(),
                    detail: None,
                    scope: "repo".into(),
                    scope_ref: None,
                    tags: "[]".into(),
                    status: "approved".into(),
                    source_task_id: Some(task),
                    upvote_count: 0,
                    last_upvoted_at: None,
                    created_at: "2026-09-19 10:00:00".into(),
                    updated_at: "2026-09-19 10:00:00".into(),
                    embedding: None,
                    workspace_id: 0,
                }],
            )
        })
        .expect("seed_learnings");
    store
        .call("alice", |ctx| {
            seed_learning_retrievals(
                ctx,
                vec![LearningRetrieval {
                    id: 1,
                    task_id: task,
                    learning_id: 1,
                    source: "prompt_injection".into(),
                    retrieved_at: "2026-09-19 10:00:00".into(),
                }],
            )
        })
        .expect("seed_learning_retrievals");

    store
        .call("alice", |ctx| delete_epic(ctx, epic))
        .expect("delete_epic");

    let learning = store
        .db()
        .learnings()
        .id()
        .find(1)
        .expect("the learning survives");
    assert_eq!(
        learning.source_task_id, None,
        "the learning is detached from its deleted source task"
    );
    assert_eq!(
        store.db().learning_retrievals().iter().count(),
        0,
        "a retrieval naming a task inside the deleted epic must go with it"
    );
}

/// Deleting a feed epic is a reset: its feed tasks are retired under nothing
/// (the feed epic is itself doomed), its existing records are dropped with it,
/// and records keyed on a feed epic outside the doomed subtree survive. A feed
/// epic nested under a deleted plain parent is doomed the same way.
#[test]
fn deleting_a_feed_epic_retires_nothing_and_drops_its_records() {
    let store = Store::new();
    let feed = feed_epic(&store, "feed", 0);
    let group = plain_epic(&store, "repo group", feed);
    let other_feed = feed_epic(&store, "other feed", 0);
    let parent = plain_epic(&store, "plain parent", 0);
    let nested = feed_epic(&store, "nested feed", parent);
    feed_task(&store, "ext-a", "done", group);
    feed_task(&store, "ext-b", "done", feed);
    feed_task(&store, "ext-c", "done", nested);
    retire_seed(
        &store,
        &[
            (100, feed, "ext-old"),
            (101, other_feed, "ext-kept"),
            (102, nested, "ext-nested-old"),
        ],
    );

    store
        .call("alice", |ctx| delete_epic(ctx, feed))
        .expect("delete_epic");
    assert_eq!(
        retired_count(&store, |r| r.feed_epic_id == feed),
        0,
        "a deleted feed epic's records go with it, and none are written for it"
    );
    assert_eq!(
        retired_count(&store, |r| r.external_id == "ext-a"
            || r.external_id == "ext-b"),
        0,
        "a feed task under a doomed feed epic has nothing to be retired under"
    );
    assert_eq!(
        retired_count(&store, |r| r.feed_epic_id == other_feed
            && r.external_id == "ext-kept"),
        1,
        "another feed's record must survive"
    );

    store
        .call("alice", |ctx| delete_epic(ctx, parent))
        .expect("delete_epic");
    assert_eq!(
        retired_count(&store, |r| r.feed_epic_id == nested),
        0,
        "a feed epic nested in a deleted subtree is reset too"
    );
    assert_eq!(
        store.db().retired_feed_items().iter().count(),
        1,
        "only the surviving feed's record may remain"
    );
}
