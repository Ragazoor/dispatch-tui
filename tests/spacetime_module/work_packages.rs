use super::*;

// -- Work packages (`docs/specs/work-packages.allium`) -----------------------

fn work_package_json(title: &str) -> serde_json::Value {
    serde_json::json!({
        "id": 0,
        "title": title,
        "description": "",
        "created_by": "user-a",
        "created_at": "2026-10-09 10:00:00",
        "updated_at": "2026-10-09 10:00:00",
    })
}

/// `work-packages.allium: CreateWorkPackage` — a blank title is refused and
/// nothing is created; a padded one is trimmed.
#[test]
fn creating_a_work_package_trims_its_title_and_refuses_a_blank_one() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let blank = instance.call(
        "create_work_package",
        &[&work_package_json("   ").to_string()],
    );
    assert!(!blank.status.success(), "{}", describe(&blank));
    assert!(no_rows(&instance, "SELECT id FROM work_packages"));

    let made = instance.call(
        "create_work_package",
        &[&work_package_json("  Window 1  ").to_string()],
    );
    assert!(made.status.success(), "{}", describe(&made));
    assert_eq!(
        column(&instance, "SELECT title FROM work_packages WHERE id = 1"),
        "Window 1"
    );
}

/// `work-packages.allium: UpdateWorkPackage` — a missing package is refused,
/// a blank title is refused, a description change leaves the title alone.
#[test]
fn patching_a_work_package_refuses_a_missing_package_and_a_blank_title() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "create_work_package",
        &[&work_package_json("Window 1").to_string()],
    );
    let missing = instance.call(
        "patch_work_package",
        &[
            "99",
            &serde_json::json!({"title": {"some": "x"}, "description": {"none": []}}).to_string(),
        ],
    );
    assert!(!missing.status.success(), "{}", describe(&missing));
    let blank = instance.call(
        "patch_work_package",
        &[
            "1",
            &serde_json::json!({"title": {"some": " "}, "description": {"none": []}}).to_string(),
        ],
    );
    assert!(!blank.status.success(), "{}", describe(&blank));
    let described = instance.call(
        "patch_work_package",
        &[
            "1",
            &serde_json::json!({"title": {"none": []}, "description": {"some": "the plan"}})
                .to_string(),
        ],
    );
    assert!(described.status.success(), "{}", describe(&described));
    assert_eq!(
        column(&instance, "SELECT title FROM work_packages WHERE id = 1"),
        "Window 1"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT description FROM work_packages WHERE id = 1"
        ),
        "the plan"
    );
}

/// `work-packages.allium: LinkItem` / `UnlinkItem` — both idempotent; a link
/// to an unknown item, an unknown package or an unknown kind is refused.
#[test]
fn linking_is_idempotent_and_refuses_unknown_targets() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "create_work_package",
        &[&work_package_json("Window 1").to_string()],
    );
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([task_json(1, "t", "backlog", 1, "")]).to_string()],
    );

    for _ in 0..2 {
        let linked = instance.call("link_work_package_item", &["1", "task", "1"]);
        assert!(linked.status.success(), "{}", describe(&linked));
    }
    let linked = instance.call("link_work_package_item", &["1", "epic", "1"]);
    assert!(linked.status.success(), "{}", describe(&linked));
    assert_eq!(
        column(&instance, "SELECT count(*) AS c FROM work_package_links"),
        "2",
        "a repeated link must not duplicate"
    );

    for args in [
        ["1", "task", "99"],
        ["1", "epic", "99"],
        ["99", "task", "1"],
        ["1", "board", "1"],
    ] {
        let refused = instance.call("link_work_package_item", &args);
        assert!(
            !refused.status.success(),
            "{args:?}: {}",
            describe(&refused)
        );
    }

    for _ in 0..2 {
        let unlinked = instance.call("unlink_work_package_item", &["1", "task", "1"]);
        assert!(unlinked.status.success(), "{}", describe(&unlinked));
    }
    assert_eq!(
        column(&instance, "SELECT kind FROM work_package_links"),
        "epic"
    );
}

/// `work-packages.allium: DeleteWorkPackage` — the package and its links go;
/// the linked items stay.
#[test]
fn deleting_a_work_package_removes_its_links_and_keeps_the_items() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "create_work_package",
        &[&work_package_json("Window 1").to_string()],
    );
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([task_json(1, "t", "backlog", 1, "")]).to_string()],
    );
    let linked = instance.call("link_work_package_item", &["1", "task", "1"]);
    assert!(linked.status.success(), "{}", describe(&linked));

    let deleted = instance.call("delete_work_package", &["1"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));
    assert!(no_rows(&instance, "SELECT id FROM work_packages"));
    assert!(no_rows(&instance, "SELECT id FROM work_package_links"));
    assert_eq!(
        column(&instance, "SELECT title FROM tasks WHERE id = 1"),
        "t"
    );
}

/// `work-packages.allium: DeletingAnItemRemovesItsLinks` — deleting a task,
/// or an epic with its subtree, removes every link naming a deleted row, and
/// only those: a task and an epic sharing an id are different items.
#[test]
fn deleting_linked_items_removes_their_links_and_only_theirs() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "create_work_package",
        &[&work_package_json("Window 1").to_string()],
    );
    instance.call(
        "seed_epics",
        &[&serde_json::json!([
            epic_json(1, "E", "done", 0),
            epic_json(2, "F", "backlog", 0)
        ])
        .to_string()],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([
            task_json(1, "in E", "done", 1, ""),
            task_json(2, "in F", "done", 2, ""),
            task_json(3, "kept", "backlog", 2, ""),
        ])
        .to_string()],
    );
    for (kind, id) in [
        ("epic", "1"),
        ("task", "1"),
        ("task", "2"),
        ("task", "3"),
        ("epic", "2"),
    ] {
        let linked = instance.call("link_work_package_item", &["1", kind, id]);
        assert!(linked.status.success(), "{}", describe(&linked));
    }

    let deleted = instance.call("delete_task", &["2"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));
    let deleted = instance.call("delete_epic", &["1"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));

    let kept = instance.sql("SELECT id FROM work_package_links");
    assert!(kept.contains("1/task/3"), "{kept}");
    assert!(kept.contains("1/epic/2"), "{kept}");
    assert_eq!(
        column(&instance, "SELECT count(*) AS c FROM work_package_links"),
        "2",
        "{kept}"
    );
}

/// The batch path owes the same cascade (`tasks.allium: BatchDelete`).
#[test]
fn batch_delete_removes_the_links_of_everything_it_deletes() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "create_work_package",
        &[&work_package_json("Window 1").to_string()],
    );
    instance.call(
        "seed_epics",
        &[
            &serde_json::json!([epic_json(1, "E", "done", 0), epic_json(2, "F", "done", 0)])
                .to_string(),
        ],
    );
    instance.call(
        "seed_tasks",
        &[&serde_json::json!([task_json(2, "in F", "done", 2, "")]).to_string()],
    );
    for (kind, id) in [("epic", "1"), ("task", "2")] {
        let linked = instance.call("link_work_package_item", &["1", kind, id]);
        assert!(linked.status.success(), "{}", describe(&linked));
    }

    let deleted = instance.call("batch_delete", &["[2]", "[1]"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));
    assert!(no_rows(&instance, "SELECT id FROM work_package_links"));
}
