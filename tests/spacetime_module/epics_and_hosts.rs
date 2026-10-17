use super::*;

/// One call updates every task in the batch, and leaves a missing id as a
/// silent no-op rather than failing the whole call.
#[test]
fn batch_patch_sub_status_applies_the_whole_batch_in_one_call() {
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
            task_json(1, "a", "running", 1, ""),
            task_json(2, "b", "running", 1, ""),
        ])
        .to_string()],
    );

    let applied = instance.call(
        "batch_patch_sub_status",
        &[&serde_json::json!([
            {"task_id": 1, "sub_status": "active"},
            {"task_id": 2, "sub_status": "stale"},
            {"task_id": 999, "sub_status": "active"},
        ])
        .to_string()],
    );
    assert!(
        applied.status.success(),
        "a missing id must not fail the batch: {}",
        describe(&applied)
    );

    assert_eq!(
        column(&instance, "SELECT sub_status FROM tasks WHERE id = 1"),
        "active"
    );
    assert_eq!(
        column(&instance, "SELECT sub_status FROM tasks WHERE id = 2"),
        "stale"
    );
}

/// The successor is created and the predecessor's `phoenix` flag clears in
/// one call — `TheFlagIsTheReceipt`.
#[test]
fn respawn_phoenix_successor_creates_and_clears_the_flag_atomically() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    // Created, not seeded — see the identical note in
    // create_repo_group_sub_epic_is_idempotent: this test goes
    // on to auto-insert the successor on this same `tasks` table.
    let mut predecessor = task_json(0, "recurring", "done", 0, "");
    predecessor["owner"] = serde_json::json!("user-a");
    predecessor["phoenix"] = serde_json::json!(true);
    instance.call("create_task", &[&predecessor.to_string()]);

    let mut successor = task_json(0, "recurring", "backlog", 0, "");
    successor["owner"] = serde_json::json!("user-a");
    successor["phoenix"] = serde_json::json!(true);
    successor["created_at"] = serde_json::json!("2026-09-21 10:00:00");
    successor["updated_at"] = serde_json::json!("2026-09-21 10:00:00");
    let made = instance.call("respawn_phoenix_successor", &["1", &successor.to_string()]);
    assert!(made.status.success(), "{}", describe(&made));

    assert_eq!(
        column(&instance, "SELECT phoenix FROM tasks WHERE id = 1"),
        "false",
        "the predecessor's flag must clear"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT count(*) AS c FROM tasks WHERE title = 'recurring' AND phoenix = true"
        ),
        "1",
        "exactly one new successor must carry the flag onward"
    );

    let missing = instance.call(
        "respawn_phoenix_successor",
        &["999", &successor.to_string()],
    );
    assert!(
        !missing.status.success(),
        "a missing predecessor must refuse rather than orphan a successor"
    );
}

/// Plain upsert by id: a second call with a changed label overwrites rather
/// than duplicating.
#[test]
fn register_host_upserts_by_id() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let me = instance.identity();

    let first = instance.call("register_host", &["host-1", "first-label", &me]);
    assert!(first.status.success(), "{}", describe(&first));
    let second = instance.call("register_host", &["host-1", "second-label", &me]);
    assert!(second.status.success(), "{}", describe(&second));

    assert_eq!(column(&instance, "SELECT count(*) AS c FROM hosts"), "1");
    assert_eq!(
        column(&instance, "SELECT label FROM hosts WHERE id = 'host-1'"),
        "second-label"
    );
}
