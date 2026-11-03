use super::*;

/// Publishing the same module over itself is accepted and changes nothing.
#[test]
fn publishing_the_module_twice_automigrates() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = Instance::start("module-test");
    let module = module_path();

    let first = instance.publish(&module, None);
    assert!(
        first.status.success(),
        "first publish: {}",
        describe(&first)
    );

    let second = instance.publish(&module, None);
    assert!(
        second.status.success(),
        "republishing an unchanged module must automigrate: {}",
        describe(&second)
    );
}

/// The shape in the last commit migrates into the shape in the working tree,
/// without destroying data.
///
/// This is the one that catches a column added anywhere but the end. It is also
/// what caught `#[default(..)]` being required on an appended column, which
/// nothing in the source hints at.
#[test]
fn the_committed_module_automigrates_into_the_working_tree_module() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = Instance::start("module-test");
    let scratch = tempfile::tempdir().expect("temp dir");
    let previous = committed_module(scratch.path());

    let first = instance.publish(&previous, Some(&scratch_target_dir()));
    assert!(
        first.status.success(),
        "publishing the committed module: {}",
        describe(&first)
    );
    let before = schema_version_row(&instance);
    assert_eq!(before[0], 1, "init stamps exactly one schema_version row");

    let migrated = instance.publish(&module_path(), None);
    assert!(
        migrated.status.success(),
        "the working tree's module must automigrate over the committed one, \
         without --delete-data. A column added anywhere but the end fails here: {}",
        describe(&migrated)
    );

    // The row survived rather than being recreated. A publish does not re-run
    // `init`, so the SQLite version coming back unchanged is what says the
    // database was migrated rather than dropped and rebuilt.
    let after = schema_version_row(&instance);
    assert_eq!(
        after[1], before[1],
        "the SQLite schema version must survive the migration untouched"
    );

    // Re-stamping is what moves `module_version` off its migration default, and
    // it is a separate step from publishing on purpose: a publish cannot run
    // `init`, so nothing else would ever update it.
    let restamp = instance.call("set_schema_version", &["97"]);
    assert!(restamp.status.success(), "{}", describe(&restamp));
    let stamped = schema_version_row(&instance);
    assert_eq!(
        stamped[2], 1,
        "re-stamping must record the running module's own version"
    );
}

/// A board's login cannot take the publisher role, and the publisher's own
/// claim on a store that already recorded it is refused harmlessly, which the
/// bring-up treats as success. Also: the restore-only reducers refuse a board.
#[test]
fn only_the_publisher_may_claim_or_use_the_restore_only_reducers() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = Instance::start("module-test");
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    let again = instance.call("claim_store_owner", &[]);
    assert!(!again.status.success(), "{}", describe(&again));
    assert!(
        describe(&again).contains("already has a recorded publisher"),
        "{}",
        describe(&again)
    );

    let board = instance.call_as("board", "claim_store_owner", &[]);
    assert!(!board.status.success(), "{}", describe(&board));
    for (reducer, args) in [
        ("set_schema_version", vec!["97"]),
        ("burn_id_sequence", vec!["\"tasks\"", "3"]),
        ("probe_generated_task_id", vec![]),
    ] {
        let out = instance.call_as("board", reducer, &args);
        assert!(!out.status.success(), "{reducer}: {}", describe(&out));
        assert!(
            describe(&out).contains("restore"),
            "{reducer}: {}",
            describe(&out)
        );
    }
    let ok = instance.call("set_schema_version", &["97"]);
    assert!(ok.status.success(), "{}", describe(&ok));
}
