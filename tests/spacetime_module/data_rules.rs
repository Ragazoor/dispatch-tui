use super::*;

// -- store-split M2 (task #53255): data rules in the reducers and the gate ------

/// `mcp-task-tools.allium: UpdateTaskViaMcp`, enforced where the rows are.
#[test]
fn an_illegal_sub_status_is_refused_at_the_reducer_with_the_old_wording() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "create_epic",
        &[&epic_json(0, "epic", "backlog", 0).to_string()],
    );
    instance.call(
        "create_task",
        &[&task_json(0, "task", "backlog", 1, "").to_string()],
    );

    let refused = instance.call(
        "patch_task",
        &["1", &patch_setting("sub_status", "active").to_string()],
    );
    assert!(!refused.status.success(), "{}", describe(&refused));
    assert!(
        describe(&refused).contains("sub_status 'active' is not valid for status 'backlog'"),
        "{}",
        describe(&refused)
    );

    // Judged against the status the patch moves to, not the one the row has.
    let mut both = patch_setting("sub_status", "active");
    both["status"] = serde_json::json!({"some": "running"});
    let accepted = instance.call("patch_task", &["1", &both.to_string()]);
    assert!(accepted.status.success(), "{}", describe(&accepted));
}

/// `epics.allium: UpdateEpicViaMcp` / `ReparentEpicFromTui`.
#[test]
fn a_parent_cycle_is_refused_at_the_reducer() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    instance.call(
        "create_epic",
        &[&epic_json(0, "a", "backlog", 0).to_string()],
    );
    instance.call(
        "create_epic",
        &[&epic_json(0, "b", "backlog", 1).to_string()],
    );

    let mut patch = blank_epic_patch_json();
    patch["parent_epic_id"] = serde_json::json!({"some": 2});
    let refused = instance.call("patch_epic", &["1", &patch.to_string()]);
    assert!(!refused.status.success(), "{}", describe(&refused));
    assert!(
        describe(&refused).contains("would create a cycle in the epic hierarchy"),
        "{}",
        describe(&refused)
    );
}

/// `epics.allium`'s routing note: the store places a task of a grouped epic.
#[test]
fn a_task_created_in_a_grouped_epic_lands_in_its_repo_sub_epic() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let mut root = epic_json(0, "root", "backlog", 0);
    root["group_by_repo"] = serde_json::json!(true);
    instance.call("create_epic", &[&root.to_string()]);
    let mut task = task_json(0, "task", "backlog", 1, "");
    task["repo_path"] = serde_json::json!("/x/alpha");
    let created = instance.call("create_task", &[&task.to_string()]);
    assert!(created.status.success(), "{}", describe(&created));

    let epics = instance
        .sql("SELECT id, title, parent_epic_id, origin FROM epics WHERE origin = 'repo-group'");
    assert!(
        epics.contains("alpha") && epics.contains("repo-group"),
        "the sub-epic for the task's repo: {epics}"
    );
    let landed = instance.sql("SELECT epic_id FROM tasks");
    assert!(
        !landed.contains("[1]") && landed.contains("[2]"),
        "the task is in the sub-epic (epic 2), not the root: {landed}"
    );
}

/// A connection stamps the rules version, so a store upgraded by a plain
/// publish passes the gate: nothing in production calls `set_schema_version`.
#[test]
fn a_board_connecting_to_an_upgraded_store_passes_the_version_gate() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = Instance::start("module-test");
    let scratch = tempfile::tempdir().expect("temp dir");
    let previous = committed_module(scratch.path());
    let first = instance.publish(&previous, Some(&scratch_target_dir()));
    assert!(first.status.success(), "{}", describe(&first));
    let migrated = instance.publish(&module_path(), None);
    assert!(migrated.status.success(), "{}", describe(&migrated));

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async {
        let connector =
            SpacetimeSdkConnector::new(instance.database(), Arc::new(SharedRows::new()));
        connector
            .connect(&instance.host(), None)
            .await
            .unwrap_or_else(|e| panic!("the gate must accept the module it was built with: {e}"));
    });
    let json = instance.sql("SELECT version FROM rules_version");
    assert!(
        json.contains(&format!(
            "[{}]",
            dispatch_tui::sync::rules_gate::BOARD_RULES_VERSION
        )),
        "the connection stamped the module's rules version: {json}"
    );
}
