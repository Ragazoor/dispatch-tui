use super::*;

// ---------------------------------------------------------------------------
// Phase 6c: feed ingestion and the stragglers
// ---------------------------------------------------------------------------

/// The two-part contract of a re-poll: fields the feed owns move, fields the
/// user or the store own do not. A re-poll changes `title`; a user-moved
/// `status`/`sub_status` and a store-set `worktree` must both survive it.
#[test]
fn feed_upsert_updates_feed_fields_and_preserves_user_and_store_fields() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );

    let first = instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([feed_item_json("ext-1", "original title", "backlog")]).to_string(),
            "test-creator",
        ],
    );
    assert!(first.status.success(), "{}", describe(&first));

    // The user moves it to Running and the store gives it a worktree —
    // neither of which a feed item carries or should be able to touch.
    let running = instance.call(
        "patch_task",
        &["1", &patch_setting("status", "running").to_string()],
    );
    assert!(running.status.success(), "{}", describe(&running));
    let mut worktree_patch = empty_task_patch();
    worktree_patch["worktree"] = serde_json::json!({"some": "wt-1"});
    let worktreed = instance.call("patch_task", &["1", &worktree_patch.to_string()]);
    assert!(worktreed.status.success(), "{}", describe(&worktreed));

    // Re-poll with a changed title and an item status the SQLite version's
    // `ON CONFLICT` would also ignore.
    let second = instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([feed_item_json("ext-1", "updated title", "done")]).to_string(),
            "test-creator",
        ],
    );
    assert!(second.status.success(), "{}", describe(&second));

    assert_eq!(
        column(&instance, "SELECT title FROM tasks WHERE id = 1"),
        "updated title",
        "the feed-owned field must update"
    );
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "running",
        "a re-poll must not move a user-managed status"
    );
    assert_eq!(
        column(&instance, "SELECT worktree FROM tasks WHERE id = 1"),
        "wt-1",
        "a re-poll must not touch a store-owned field"
    );
    assert!(
        no_rows(&instance, "SELECT id FROM tasks WHERE completed_at != ''"),
        "a task that was never done must not gain a completion stamp"
    );
}

/// An existing non-null url always wins over whatever the item carries; url
/// and url_type move together, never one without the other.
#[test]
fn feed_upsert_keeps_an_existing_url_over_a_re_polled_one() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    let mut first_item = feed_item_json("ext-1", "t", "backlog");
    first_item["url"] = serde_json::json!("https://example.com/1");
    first_item["url_type"] = serde_json::json!("pr");
    instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([first_item]).to_string(),
            "test-creator",
        ],
    );

    let mut second_item = feed_item_json("ext-1", "t", "backlog");
    second_item["url"] = serde_json::json!("https://example.com/2");
    second_item["url_type"] = serde_json::json!("issue");
    let second = instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([second_item]).to_string(),
            "test-creator",
        ],
    );
    assert!(second.status.success(), "{}", describe(&second));

    assert_eq!(
        column(
            &instance,
            "SELECT url FROM tasks WHERE external_id = 'ext-1'"
        ),
        "https://example.com/1",
        "the FIRST url must win, not the re-polled one"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT url_type FROM tasks WHERE external_id = 'ext-1'"
        ),
        "pr"
    );
}

/// A task born already `done` is stamped complete; a re-poll never re-derives
/// that stamp, matching `write_task`'s own insert-only rule.
#[test]
fn feed_upsert_stamps_completion_only_on_a_genuine_insert() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    let made = instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([feed_item_json("ext-1", "t", "done")]).to_string(),
            "test-creator",
        ],
    );
    assert!(made.status.success(), "{}", describe(&made));
    assert_ne!(
        column(
            &instance,
            "SELECT completed_at FROM tasks WHERE external_id = 'ext-1'"
        ),
        "",
        "a task created directly into done must be stamped"
    );
}

/// A newly-inserted feed task stamps `created_by` from the caller — the
/// running host's own identity (`core.allium: Task.created_by`,
/// `feeds.allium: UpsertFeedTasks`) — and a later re-poll of the same item
/// leaves it untouched, matching `created_by`'s "stamped once" rule.
#[test]
fn feed_upsert_stamps_created_by_on_insert_only() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );

    let made = instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([feed_item_json("ext-1", "t", "backlog")]).to_string(),
            "host-a",
        ],
    );
    assert!(made.status.success(), "{}", describe(&made));
    assert_eq!(
        column(
            &instance,
            "SELECT created_by FROM tasks WHERE external_id = 'ext-1'"
        ),
        "host-a",
        "a genuine insert must stamp created_by from the caller"
    );

    let repolled = instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([feed_item_json("ext-1", "updated", "backlog")]).to_string(),
            "host-b",
        ],
    );
    assert!(repolled.status.success(), "{}", describe(&repolled));
    assert_eq!(
        column(
            &instance,
            "SELECT created_by FROM tasks WHERE external_id = 'ext-1'"
        ),
        "host-a",
        "a re-poll from a different host must not overwrite created_by"
    );
}

// ---------------------------------------------------------------------------
// Retired feed items — delete means "I am done with this item"
// ---------------------------------------------------------------------------
//
// `core.allium: RetiredFeedItem`, written by `tasks.allium: DeleteTask` and
// `epics.allium: DeleteEpic`, read by `feeds.allium:
// IngestSkipsRetiredFeedItems` and dropped by `DropClosedRetiredFeedItems`
// (inside `ingest_feed`).
// Every retirement is keyed on the FEED EPIC — the nearest epic in the chain
// carrying a `feed_command` — never on the sub-epic the task sat in.

/// An epic carrying a `feed_command`, which is what makes it the key a
/// retirement is written under (`core/Epic.nearest_feed_epic`).
fn feed_epic_json(id: i64, title: &str, parent: i64) -> serde_json::Value {
    let mut epic = epic_json(id, title, "backlog", parent);
    epic["feed_command"] = serde_json::json!("echo []");
    epic
}

/// A feed task: a task row carrying `external_id`, in `epic`.
fn feed_task_json(id: i64, external_id: &str, status: &str, epic: i64) -> serde_json::Value {
    let mut task = task_json(id, external_id, status, epic, "");
    task["external_id"] = serde_json::json!(external_id);
    task
}

/// A `retired_feed_items` row as `seed_retired_feed_items` takes it. Seeded
/// ids are kept far above the auto_inc counter's start, because seeding an
/// explicit id leaves that counter unburned (#755) and a later generated
/// insert would otherwise collide with it.
fn retired_json(id: i64, feed_epic_id: i64, external_id: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "feed_epic_id": feed_epic_id,
        "external_id": external_id,
        "retired_at": "2026-09-19 10:00:00",
    })
}

fn retired_count(instance: &Instance, where_clause: &str) -> String {
    column(
        instance,
        &format!("SELECT count(*) AS c FROM retired_feed_items WHERE {where_clause}"),
    )
}

/// `tasks.allium: DeleteTask`'s retirement clause: deleting a feed task
/// writes one record keyed on the NEAREST FEED EPIC in its chain — here the
/// grandparent, not the sub-epic the task sat in — in the same call that
/// removes the row.
#[test]
fn deleting_a_feed_task_retires_it_under_its_nearest_feed_epic() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let seeded = instance.call(
        "seed_epics",
        &[&serde_json::json!([
            feed_epic_json(1, "feed", 0),
            epic_json(2, "repo group", "backlog", 1),
        ])
        .to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([feed_task_json(1, "ext-1", "done", 2)]).to_string()],
    );

    let deleted = instance.call("delete_task", &["1"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));

    assert!(
        no_rows(&instance, "SELECT id FROM tasks WHERE id = 1"),
        "the task row must be gone"
    );
    assert_eq!(
        retired_count(&instance, "feed_epic_id = 1 AND external_id = 'ext-1'"),
        "1",
        "the retirement must be keyed on the feed epic, not the sub-epic"
    );
    assert!(
        no_rows(
            &instance,
            "SELECT id FROM retired_feed_items WHERE feed_epic_id = 2"
        ),
        "nothing may be keyed on the sub-epic the task sat in"
    );
    assert_ne!(
        column(
            &instance,
            "SELECT retired_at FROM retired_feed_items WHERE external_id = 'ext-1'"
        ),
        "",
        "a retirement is stamped with the store's clock"
    );
}

/// A manual task (no `external_id`) and a feed-shaped task under no feed epic
/// both retire nothing: there is no cycle to suppress.
#[test]
fn deleting_a_manual_task_or_one_under_no_feed_epic_retires_nothing() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([
            feed_epic_json(1, "feed", 0),
            epic_json(2, "plain", "backlog", 0),
        ])
        .to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            task_json(1, "manual", "done", 1, ""),
            feed_task_json(2, "ext-orphan", "done", 2),
        ])
        .to_string()],
    );

    for id in ["1", "2"] {
        let deleted = instance.call("delete_task", &[id]);
        assert!(deleted.status.success(), "{}", describe(&deleted));
    }

    assert!(
        no_rows(&instance, "SELECT id FROM tasks"),
        "both tasks must still be deleted"
    );
    assert!(
        no_rows(&instance, "SELECT id FROM retired_feed_items"),
        "neither a manual task nor one under no feed epic may be retired"
    );
}

/// `core/RetiredFeedItem: UniqueRetiredFeedItemPerFeed`. Deleting a second
/// task that carries an already-retired (feed epic, external_id) is a no-op;
/// the same id under a DIFFERENT feed epic is its own record. Both properties
/// go entirely through `delete_task` — there is no standalone
/// direct-retirement reducer, since every real retirement writes inline from
/// a delete (or the migration).
#[test]
fn retiring_the_same_feed_item_twice_writes_one_record() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([
            feed_epic_json(1, "feed", 0),
            epic_json(2, "sub-a", "backlog", 1),
            epic_json(3, "sub-b", "backlog", 1),
            feed_epic_json(4, "other feed", 0),
        ])
        .to_string()],
    );

    // Two tasks with the same id in two sub-epics of one feed: both deletes
    // resolve to the same (feed_epic, external_id) key and the second must
    // not write a second record.
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            feed_task_json(1, "ext-1", "done", 2),
            feed_task_json(2, "ext-1", "done", 3),
        ])
        .to_string()],
    );
    for id in ["1", "2"] {
        let deleted = instance.call("delete_task", &[id]);
        assert!(deleted.status.success(), "{}", describe(&deleted));
    }
    assert_eq!(
        retired_count(&instance, "feed_epic_id = 1 AND external_id = 'ext-1'"),
        "1",
        "a delete of an already-retired id must not write a second record"
    );

    // The same external_id, deleted under a DIFFERENT feed epic, is its own
    // record.
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([feed_task_json(3, "ext-1", "done", 4)]).to_string()],
    );
    let other = instance.call("delete_task", &["3"]);
    assert!(other.status.success(), "{}", describe(&other));
    assert_eq!(
        retired_count(&instance, "external_id = 'ext-1'"),
        "2",
        "the same id under a different feed epic is a separate record"
    );
}

/// `feeds.allium: IngestSkipsRetiredFeedItems`, via the reconciling reducer:
/// an item whose id is retired under the upsert epic's nearest feed epic, with
/// no surviving task, is never inserted — while a non-retired item in the same
/// batch is. The record is restored through `seed_retired_feed_items`, so this
/// also covers the snapshot seed path.
#[test]
fn feed_upsert_refuses_to_insert_a_retired_external_id() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([
            feed_epic_json(1, "feed", 0),
            epic_json(2, "repo group", "backlog", 1),
        ])
        .to_string()],
    );
    let seeded = instance.call(
        "seed_retired_feed_items",
        &[&serde_json::json!([retired_json(100, 1, "ext-retired")]).to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));

    let upserted = instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([
                feed_item_json("ext-retired", "back again", "backlog"),
                feed_item_json("ext-new-1", "fresh", "backlog"),
            ])
            .to_string(),
            "test-creator",
        ],
    );
    assert!(upserted.status.success(), "{}", describe(&upserted));
    assert!(
        no_rows(
            &instance,
            "SELECT id FROM tasks WHERE external_id = 'ext-retired'"
        ),
        "upsert_feed_tasks must not insert a retired id"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT count(*) AS c FROM tasks WHERE external_id = 'ext-new-1'"
        ),
        "1",
        "upsert_feed_tasks must still insert the batch's non-retired item"
    );
}

/// Suppression covers insertion only. A task still on the board with a
/// retired id is matched and refreshed like any other feed task, and the
/// record itself is untouched by the upsert.
#[test]
fn feed_upsert_refreshes_a_surviving_task_whose_id_is_retired() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([feed_epic_json(1, "feed", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([feed_task_json(1, "ext-1", "done", 1)]).to_string()],
    );
    let seeded = instance.call(
        "seed_retired_feed_items",
        &[&serde_json::json!([retired_json(100, 1, "ext-1")]).to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));

    let upserted = instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([feed_item_json("ext-1", "refreshed title", "backlog")]).to_string(),
            "test-creator",
        ],
    );
    assert!(upserted.status.success(), "{}", describe(&upserted));

    assert_eq!(
        column(&instance, "SELECT title FROM tasks WHERE id = 1"),
        "refreshed title",
        "a surviving task with a retired id must still be refreshed"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT count(*) AS c FROM tasks WHERE external_id = 'ext-1'"
        ),
        "1",
        "the refresh must not duplicate the row"
    );
    assert_eq!(
        retired_count(&instance, "feed_epic_id = 1 AND external_id = 'ext-1'"),
        "1",
        "an upsert must not touch the retirement record"
    );
}

/// `epics.allium: DeleteEpic`'s retirement clause, feed epic surviving: every
/// feed task anywhere in the deleted subtree — nested sub-epics included — is
/// retired under the surviving feed epic above it; a manual task is not.
#[test]
fn deleting_a_sub_epic_retires_its_whole_subtrees_feed_tasks() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([
            feed_epic_json(1, "feed", 0),
            epic_json(2, "sub", "backlog", 1),
            epic_json(3, "sub-sub", "backlog", 2),
        ])
        .to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            feed_task_json(1, "ext-a", "done", 2),
            feed_task_json(2, "ext-b", "done", 3),
            task_json(3, "manual", "done", 2, ""),
        ])
        .to_string()],
    );

    let deleted = instance.call("delete_epic", &["2"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));

    assert!(
        no_rows(&instance, "SELECT id FROM tasks"),
        "the whole subtree's tasks must be deleted"
    );
    assert!(
        !no_rows(&instance, "SELECT id FROM epics WHERE id = 1"),
        "the feed epic above the deleted sub-epic must survive"
    );
    assert_eq!(
        retired_count(&instance, "feed_epic_id = 1 AND external_id = 'ext-a'"),
        "1",
        "a feed task directly in the deleted epic must be retired"
    );
    assert_eq!(
        retired_count(&instance, "feed_epic_id = 1 AND external_id = 'ext-b'"),
        "1",
        "a feed task in a nested sub-epic must be retired too"
    );
    assert_eq!(
        retired_count(&instance, "feed_epic_id > 0"),
        "2",
        "a manual task must not be retired, and nothing keyed elsewhere"
    );

    // And the retirement bites: the feed's next cycle does not re-insert.
    instance.call(
        "upsert_feed_tasks",
        &[
            "1",
            &serde_json::json!([feed_item_json("ext-a", "a", "backlog")]).to_string(),
            "test-creator",
        ],
    );
    assert!(
        no_rows(
            &instance,
            "SELECT id FROM tasks WHERE external_id = 'ext-a'"
        ),
        "a cycle after the delete must not put the item straight back"
    );
}

/// `epics.allium: DeleteEpic`'s guard — every task anywhere in the subtree is
/// `done` — enforced HERE, at the reducer, not only in
/// `EpicService::delete_epic`'s subscription-view check. Calling the reducer
/// directly, as if that client-side guard had been bypassed or fooled by an
/// incomplete view, must still refuse: permanent delete makes the gap a
/// client-only guard leaves irrecoverable (task #4971).
#[test]
fn deleting_an_epic_with_an_undone_subtree_task_is_refused_at_the_reducer() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([
            epic_json(1, "root", "backlog", 0),
            epic_json(2, "sub", "backlog", 1),
        ])
        .to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            task_json(1, "done one", "done", 1, ""),
            task_json(2, "still running", "running", 2, ""),
        ])
        .to_string()],
    );

    let refused = instance.call("delete_epic", &["1"]);
    assert!(
        !refused.status.success(),
        "a subtree holding an undone task anywhere in it (even in a nested \
         sub-epic) must be refused: {}",
        describe(&refused)
    );
    assert!(
        describe(&refused).contains("not done"),
        "the refusal must say why: {}",
        describe(&refused)
    );

    assert!(
        !no_rows(&instance, "SELECT id FROM epics"),
        "a refused delete must leave the whole subtree in place"
    );
    assert!(
        !no_rows(&instance, "SELECT id FROM tasks"),
        "a refused delete must leave every task in place too"
    );

    // The same subtree with its task finished is accepted, so the refusal is
    // the rule rather than the reducer being broken.
    let finished = instance.call(
        "patch_task",
        &["2", &patch_setting("status", "done").to_string()],
    );
    assert!(finished.status.success(), "{}", describe(&finished));
    let accepted = instance.call("delete_epic", &["1"]);
    assert!(accepted.status.success(), "{}", describe(&accepted));
}

/// `tasks.allium: BatchDelete`'s atomic guarantee, over the reducer directly:
/// one task in the batch is done (would pass on its own, exactly like
/// `deleting_an_undone_task_is_refused_at_the_reducer`'s accepted case) and
/// one is not. Before `batch_delete` existed, the caller looped independent
/// `delete_task`/`delete_epic` calls, so the done one would have been deleted
/// while the other alone was refused — a partial batch. `batch_delete` must
/// instead refuse the WHOLE call and leave every row, including the done one,
/// untouched.
#[test]
fn batch_delete_refuses_the_whole_batch_when_one_task_is_not_done() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            task_json(1, "finished", "done", 1, ""),
            task_json(2, "still running", "running", 1, ""),
        ])
        .to_string()],
    );

    let refused = instance.call("batch_delete", &["[1, 2]", "[]"]);
    assert!(
        !refused.status.success(),
        "one non-done task must refuse the whole batch: {}",
        describe(&refused)
    );
    assert!(
        describe(&refused).contains("not done"),
        "the refusal must say why: {}",
        describe(&refused)
    );
    assert!(
        !no_rows(&instance, "SELECT id FROM tasks WHERE id = 1"),
        "the done task must survive too — a refused batch deletes nothing at all"
    );
    assert!(
        !no_rows(&instance, "SELECT id FROM tasks WHERE id = 2"),
        "the not-done task must survive"
    );

    // The same batch once both are done is accepted and deletes everything
    // together, so the refusal above is the atomicity rule rather than the
    // reducer being broken.
    let finished = instance.call(
        "patch_task",
        &["2", &patch_setting("status", "done").to_string()],
    );
    assert!(finished.status.success(), "{}", describe(&finished));
    let accepted = instance.call("batch_delete", &["[1, 2]", "[]"]);
    assert!(accepted.status.success(), "{}", describe(&accepted));
    assert!(
        no_rows(&instance, "SELECT id FROM tasks"),
        "the accepted batch must remove every row"
    );
}

/// The success path over BOTH domains at once: a plain task and a
/// fully-done epic's subtree, deleted together in one `batch_delete` call —
/// the shape `handle_batch_delete` (src/tui/update/selection.rs) sends when
/// the selection mixes tasks and an epic.
#[test]
fn batch_delete_deletes_a_task_and_an_epic_subtree_together() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "done", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            task_json(1, "plain", "done", 0, "user-1"),
            task_json(2, "in the epic", "done", 1, ""),
        ])
        .to_string()],
    );

    let accepted = instance.call("batch_delete", &["[1]", "[1]"]);
    assert!(accepted.status.success(), "{}", describe(&accepted));
    assert!(
        no_rows(&instance, "SELECT id FROM tasks"),
        "both the plain task and the epic's subtask must be gone"
    );
    assert!(
        no_rows(&instance, "SELECT id FROM epics"),
        "the epic must be gone too"
    );
}

/// The same learning-detachment
/// `deleting_a_task_detaches_its_learnings_and_cascades_their_retrievals`
/// covers for `delete_task`, missing for the same reason `delete_task_side_effects`
/// exists: each path that removes task rows must reproduce it.
#[test]
fn deleting_an_epic_detaches_its_subtasks_learnings_and_cascades_their_retrievals() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    // `done`, not `backlog`: this test is about learning detachment, not the
    // delete guard, and task #4971's server-side guard now refuses to delete
    // a subtree holding an undone task.
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([task_json(1, "source", "done", 1, "")]).to_string()],
    );
    let seeded = instance.call(
        "seed_learnings",
        &[&serde_json::json!([seeded_learning_json(1, "A learning", Some(1))]).to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));
    let seeded = instance.call(
        "seed_learning_retrievals",
        &[&serde_json::json!([{
            "id": 1,
            "task_id": 1,
            "learning_id": 1,
            "source": "prompt_injection",
            "retrieved_at": "2026-09-19 10:00:00",
        }])
        .to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));

    let some_1 = column(
        &instance,
        "SELECT source_task_id FROM learnings WHERE id = 1",
    );

    let deleted = instance.call("delete_epic", &["1"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));

    let after_delete = column(
        &instance,
        "SELECT source_task_id FROM learnings WHERE id = 1",
    );
    assert_ne!(
        after_delete, some_1,
        "source_task_id must change — the delete is a no-op if this still reads Some(1)"
    );
    assert_eq!(
        after_delete, "[1,[]]",
        "the learning survives, detached from its deleted source task (none's tag, empty payload)"
    );
    assert!(
        no_rows(&instance, "SELECT id FROM learning_retrievals"),
        "a retrieval naming a task inside the deleted epic must go with it"
    );
}

/// Deleting a feed epic is a reset: its feed tasks are retired under nothing
/// (the feed epic is itself doomed), its existing records are dropped with it,
/// and records keyed on a feed epic outside the doomed subtree survive. A feed
/// epic nested under a deleted plain parent is doomed the same way.
#[test]
fn deleting_a_feed_epic_retires_nothing_and_drops_its_records() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([
            feed_epic_json(1, "feed", 0),
            epic_json(2, "repo group", "backlog", 1),
            feed_epic_json(3, "other feed", 0),
            epic_json(4, "plain parent", "backlog", 0),
            feed_epic_json(5, "nested feed", 4),
        ])
        .to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            feed_task_json(1, "ext-a", "done", 2),
            feed_task_json(2, "ext-b", "done", 1),
            feed_task_json(3, "ext-c", "done", 5),
        ])
        .to_string()],
    );
    instance.call(
        "seed_retired_feed_items",
        &[&serde_json::json!([
            retired_json(100, 1, "ext-old"),
            retired_json(101, 3, "ext-kept"),
            retired_json(102, 5, "ext-nested-old"),
        ])
        .to_string()],
    );

    let deleted = instance.call("delete_epic", &["1"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));
    assert!(
        no_rows(
            &instance,
            "SELECT id FROM retired_feed_items WHERE feed_epic_id = 1"
        ),
        "a deleted feed epic's records go with it, and none are written for it"
    );
    assert!(
        no_rows(
            &instance,
            "SELECT id FROM retired_feed_items WHERE external_id = 'ext-a' OR external_id = 'ext-b'"
        ),
        "a feed task under a doomed feed epic has nothing to be retired under"
    );
    assert_eq!(
        retired_count(&instance, "feed_epic_id = 3 AND external_id = 'ext-kept'"),
        "1",
        "another feed's record must survive"
    );

    let deleted_parent = instance.call("delete_epic", &["4"]);
    assert!(
        deleted_parent.status.success(),
        "{}",
        describe(&deleted_parent)
    );
    assert!(
        no_rows(
            &instance,
            "SELECT id FROM retired_feed_items WHERE feed_epic_id = 5"
        ),
        "a feed epic nested in a deleted subtree is reset too"
    );
    assert_eq!(
        retired_count(&instance, "feed_epic_id > 0"),
        "1",
        "only the surviving feed's record may remain"
    );
}
