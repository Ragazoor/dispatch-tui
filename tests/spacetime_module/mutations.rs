use super::*;

// ---------------------------------------------------------------------------
// Phase 6 — the mutations, and the two things only a real store can show
// ---------------------------------------------------------------------------
//
// Everything below writes through a REDUCER and reads back over SQL. The point
// is not that the reducers compile — `check-spacetime-module.sh` covers that —
// it is the two properties that need more than one writer to exist at all:
// an epic status derived once from every host's children, and two hosts
// changing one task without undoing each other.

/// THE FLAPPING TEST. An epic whose children live on two machines has ONE
/// status, and it is the one every child agrees on.
///
/// The set-up is the exact disagreement the 2026-09-13 design named: host-a
/// sees only a done child and would write `done`; host-b sees only a running
/// one and would write `backlog`. Neither is wrong about what it can see. The
/// store sees both, and the answer below is the only one that is right about
/// the epic.
#[test]
fn an_epics_status_is_derived_from_every_hosts_children() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();

    let seeded = instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "Shared", "backlog", 0)]).to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));
    let seeded = instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            task_json(1, "finished on host-a", "done", 1, "host-a"),
            task_json(2, "still going on host-b", "running", 1, "host-b"),
        ])
        .to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));

    let recalculated = instance.call("recalculate_epic_status", &["1"]);
    assert!(recalculated.status.success(), "{}", describe(&recalculated));
    assert_eq!(
        column(&instance, "SELECT status FROM epics WHERE id = 1"),
        "backlog",
        "one child is still running on another host, so the epic is not done"
    );

    // The other host finishes. Now — and only now — the epic is done, and the
    // transition is driven by the child's own patch rather than by anybody
    // remembering to recalculate.
    let patched = instance.call(
        "patch_task",
        &["2", &patch_setting("status", "done").to_string()],
    );
    assert!(patched.status.success(), "{}", describe(&patched));
    assert_eq!(
        column(&instance, "SELECT status FROM epics WHERE id = 1"),
        "done"
    );
}

/// The completion stamp is the store's clock, and it lands on the transition
/// into done rather than on every write afterwards.
#[test]
fn finishing_an_epic_stamps_a_completion_from_the_stores_clock() {
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
        &[&serde_json::json!([task_json(1, "t", "running", 1, "host-a")]).to_string()],
    );

    let patched = instance.call(
        "patch_task",
        &["1", &patch_setting("status", "done").to_string()],
    );
    assert!(patched.status.success(), "{}", describe(&patched));

    let stamped = column(&instance, "SELECT completed_at FROM epics WHERE id = 1");
    assert_ne!(stamped, "", "finishing an epic must stamp a completion");
    // The format both stores write: `YYYY-MM-DD HH:MM:SS.mmm`.
    assert_eq!(stamped.len(), 23, "unexpected timestamp shape: {stamped}");

    // Reopening does not unmake a completion. `completed_at` records the last
    // one, and the regression writes only the status.
    let reopened = instance.call(
        "patch_task",
        &["1", &patch_setting("status", "running").to_string()],
    );
    assert!(reopened.status.success(), "{}", describe(&reopened));
    assert_eq!(
        column(&instance, "SELECT status FROM epics WHERE id = 1"),
        "backlog"
    );
    assert_eq!(
        column(&instance, "SELECT completed_at FROM epics WHERE id = 1"),
        stamped,
        "a regression must not clear the last completion"
    );
}

/// TWO HOSTS WRITING ONE TASK CONVERGE. Each names only the field it changed,
/// so the later write does not undo the earlier one.
///
/// This is the whole reason the patch reducers take one `Option` per field
/// rather than a row. With a full-row write each host would send a complete
/// task built from what it last saw, and the second would silently revert the
/// first's change to a field it never touched.
#[test]
fn two_hosts_patching_different_fields_of_one_task_converge() {
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
        &[&serde_json::json!([task_json(1, "original", "backlog", 1, "")]).to_string()],
    );

    // Host A renames it.
    let a = instance.call(
        "patch_task",
        &[
            "1",
            &patch_setting("title", "renamed by host-a").to_string(),
        ],
    );
    assert!(a.status.success(), "{}", describe(&a));

    // Host B, which still believes the title is "original", moves it.
    let b = instance.call(
        "patch_task",
        &["1", &patch_setting("worktree", "/wt/host-b").to_string()],
    );
    assert!(b.status.success(), "{}", describe(&b));

    assert_eq!(
        column(&instance, "SELECT title FROM tasks WHERE id = 1"),
        "renamed by host-a",
        "host B's write must not revert a field it never named"
    );
    assert_eq!(
        column(&instance, "SELECT worktree FROM tasks WHERE id = 1"),
        "/wt/host-b"
    );
}

/// The validator runs at the store, and its refusal changes nothing.
///
/// `core.allium: OwnerTracksUserBoardTask`, enforced by the module's
/// `write_task`. Asserted here rather than only as a unit test because what
/// matters is that a REDUCER refuses — a validator that lived on the client
/// could be skipped by any other client.
#[test]
fn the_store_refuses_a_user_board_task_with_no_owner() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();

    let refused = instance.call(
        "create_task",
        &[&task_json(0, "nobody's task", "backlog", 0, "").to_string()],
    );
    assert!(
        !refused.status.success(),
        "a task with no epic and no owner must be refused, got {}",
        describe(&refused)
    );

    assert!(
        no_rows(&instance, "SELECT id FROM tasks"),
        "a refused create must leave no row behind"
    );

    // The same row WITH an owner is accepted, so the refusal is the rule rather
    // than the reducer being broken.
    let accepted = instance.call(
        "create_task",
        &[&{
            let mut row = task_json(0, "my task", "backlog", 0, "");
            row["owner"] = serde_json::json!("user-me");
            row
        }
        .to_string()],
    );
    assert!(accepted.status.success(), "{}", describe(&accepted));
}

/// **Test 3 of Phase 10 (task #4914).** A task's delete detaches its
/// learnings (`source_task_id` set null, the learning survives as orphaned
/// provenance) and cascades their retrievals — reproduced as explicit
/// reducer logic rather than a SQLite `ON DELETE` clause
/// (`docs/specs/learnings.allium`'s Storage Backend section).
#[test]
fn deleting_a_task_detaches_its_learnings_and_cascades_their_retrievals() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    // `done`, not `backlog`: this test is about learning detachment, not the
    // delete guard, and task #4971's reducer guard refuses to delete an
    // undone task.
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

    // SQL's `--format json` renders a SATS sum type positionally rather than
    // as the named `{"some": ..}`/`{"none": []}` shape reducer ARGUMENTS use
    // (see `seeded_learning_json` above) — captured here, before the delete,
    // so the "detached" assertion below compares against a shape this same
    // run observed rather than one hard-coded from a guess.
    let some_1 = column(
        &instance,
        "SELECT source_task_id FROM learnings WHERE id = 1",
    );

    let deleted = instance.call("delete_task", &["1"]);
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
        "a retrieval naming the deleted task must go with it"
    );
}

/// `tasks.allium: DeleteTask`'s `requires: task.status = done`, enforced HERE,
/// at the reducer, not only by the TUI input layer's subscription-view check.
/// Calling the reducer directly, as if a stale view (a task reopened on one
/// board still reading as done on another) had let the caller through, must
/// still refuse and delete nothing: permanent delete makes the gap a
/// client-only guard leaves irrecoverable (task #4971). The single-row twin of
/// `deleting_an_epic_with_an_undone_subtree_task_is_refused_at_the_reducer`.
#[test]
fn deleting_an_undone_task_is_refused_at_the_reducer() {
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
        &[&serde_json::json!([task_json(1, "still running", "running", 1, "")]).to_string()],
    );

    let refused = instance.call("delete_task", &["1"]);
    assert!(
        !refused.status.success(),
        "a task that is not done must be refused: {}",
        describe(&refused)
    );
    assert!(
        describe(&refused).contains("not done"),
        "the refusal must say why: {}",
        describe(&refused)
    );
    assert!(
        !no_rows(&instance, "SELECT id FROM tasks WHERE id = 1"),
        "a refused delete must leave the task row in place"
    );

    // The same task once finished is accepted, so the refusal is the rule
    // rather than the reducer being broken.
    let finished = instance.call(
        "patch_task",
        &["1", &patch_setting("status", "done").to_string()],
    );
    assert!(finished.status.success(), "{}", describe(&finished));
    let accepted = instance.call("delete_task", &["1"]);
    assert!(accepted.status.success(), "{}", describe(&accepted));
    assert!(
        no_rows(&instance, "SELECT id FROM tasks WHERE id = 1"),
        "the accepted delete must remove the row"
    );
}

/// A learning's delete takes its own retrievals with it — the other half of
/// the cascade `deleting_a_task_detaches_its_learnings_and_cascades_their_retrievals`
/// covers. Also confirms `DeleteLearningViaMcp`'s refusal for a missing id,
/// unlike `delete_task`'s silent no-op.
#[test]
fn deleting_a_learning_cascades_its_retrievals_and_refuses_when_missing() {
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
        &[&serde_json::json!([task_json(1, "t", "backlog", 1, "")]).to_string()],
    );
    let seeded = instance.call(
        "seed_learnings",
        &[&serde_json::json!([seeded_learning_json(1, "A learning", None)]).to_string()],
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

    let deleted = instance.call("delete_learning", &["1"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));

    assert!(
        no_rows(&instance, "SELECT id FROM learnings"),
        "the learning itself must be gone"
    );
    assert!(
        no_rows(&instance, "SELECT id FROM learning_retrievals"),
        "its retrievals must go with it"
    );

    let refused = instance.call("delete_learning", &["1"]);
    assert!(
        !refused.status.success(),
        "deleting an id that no longer exists must be refused, not a silent no-op"
    );
}

/// **Test 1 of Phase 11 (task #4915), against a real server.** A usage event
/// recorded on one host's board is visible from another's.
///
/// Host A is the `record_usage_event` reducer call — a different process,
/// the same convention `a_row_written_elsewhere_arrives_through_the_subscription`
/// uses. Host B is this test's own `SharedRows`, subscribed with nothing
/// followed: `usage_events` needs no `epic_id` to see a row, because it is
/// unconditionally subscribed, the same as `learnings`.
#[test]
fn a_usage_event_recorded_elsewhere_is_visible_from_here() {
    if !spacetime_available_or_skip() {
        return;
    }
    use dispatch_tui::store::UsageQuery;

    let instance = Instance::start("module-test");
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    let rows = Arc::new(SharedRows::new());
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");

    runtime.block_on(async {
        let connector = SpacetimeSdkConnector::new(instance.database(), rows.clone());
        let accepted = connector
            .connect(&instance.host(), None)
            .await
            .unwrap_or_else(|e| panic!("connect: {e}"));
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                vec![],
                "host-b",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));

        assert!(
            rows.usage_summary(&UsageQuery::default()).is_empty(),
            "nothing has been written yet, so nothing may have arrived"
        );

        let mut woken = rows.changed();
        woken.mark_unchanged();

        // Host A records the event directly through the reducer.
        let recorded = instance.call(
            "record_usage_event",
            &[
                &usage_event_json("keybinding", "dispatch_task", Some("d"), "human").to_string(),
                "100",
            ],
        );
        assert!(recorded.status.success(), "{}", describe(&recorded));

        woken
            .changed()
            .await
            .expect("the subscription must deliver the usage event");

        let summary = rows.usage_summary(&UsageQuery::default());
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0].action, "dispatch_task");
        assert_eq!(summary[0].count, 1);
    });
}

/// **Test 2 of Phase 11 (task #4915).** The row-count cap is enforced in the
/// reducer itself, not by local SQL — mirroring the original SQLite path's
/// `DELETE ... WHERE id <= MAX(id) - cap` in the same transaction as the
/// insert, but now as reducer logic every host's write goes through, the same
/// shape `deleting_a_task_detaches_its_learnings_and_cascades_their_retrievals`
/// proves for the learnings cascade above.
#[test]
fn recording_usage_events_prunes_beyond_the_cap() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();

    for i in 1..=5 {
        let recorded = instance.call(
            "record_usage_event",
            &[
                &usage_event_json("keybinding", &format!("action_{i}"), None, "human").to_string(),
                "3",
            ],
        );
        assert!(recorded.status.success(), "{}", describe(&recorded));
    }

    let count = column(&instance, "SELECT COUNT(*) AS c FROM usage_events");
    assert_eq!(count, "3", "the table must never exceed the cap");

    assert!(
        no_rows(
            &instance,
            "SELECT id FROM usage_events WHERE action = 'action_1'"
        ),
        "the oldest rows must be the ones pruned"
    );
    assert!(
        !no_rows(
            &instance,
            "SELECT id FROM usage_events WHERE action = 'action_5'"
        ),
        "the newest row must survive the prune"
    );
}

/// A `usage_events` row, JSON-encoded for `record_usage_event`'s CLI call.
fn usage_event_json(
    category: &str,
    action: &str,
    detail: Option<&str>,
    actor: &str,
) -> serde_json::Value {
    serde_json::json!({
        "id": 0,
        "recorded_at": "2026-09-26 10:00:00",
        "category": category,
        "action": action,
        "detail": match detail {
            Some(d) => serde_json::json!({"some": d}),
            None => serde_json::json!({"none": []}),
        },
        "actor": actor,
    })
}

/// A task created straight into Done carries a completion stamp.
///
/// `stamps_completion` covers every TRANSITION into done, and a create is not a
/// transition — so without this a Done card would sort to the bottom of the
/// column forever. Nothing creates a Done task today; the SQLite side stamps it
/// anyway, for the same reason.
#[test]
fn a_task_created_in_done_is_stamped_as_completed() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();

    let made = instance.call(
        "create_task",
        &[&{
            let mut row = task_json(0, "born finished", "done", 0, "");
            row["owner"] = serde_json::json!("user-me");
            row
        }
        .to_string()],
    );
    assert!(made.status.success(), "{}", describe(&made));

    let stamped = column(
        &instance,
        "SELECT completed_at FROM tasks WHERE title = 'born finished'",
    );
    assert_ne!(stamped, "", "a task born in done must carry a completion");
    assert_eq!(stamped.len(), 23, "unexpected timestamp shape: {stamped}");
}
