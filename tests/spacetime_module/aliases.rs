use super::*;

// -- Aliases (`core.allium: TaskAliasIsASecondName`) --------------------------

/// A task written without an alias is given one, and no two tasks share it.
#[test]
fn every_seeded_task_gets_its_own_alias() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", ""), (2, "running", "")]);
    let a = task_column(&instance, "alias", 1);
    let b = task_column(&instance, "alias", 2);
    assert!(!a.is_empty() && !b.is_empty(), "{a:?} {b:?}");
    assert_ne!(a, b);
}

/// A rename to an alias another task holds, or to digits, is refused and the
/// row keeps its alias; a free one is accepted.
#[test]
fn an_alias_rename_is_refused_when_taken_or_digit_only() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", ""), (2, "running", "")]);
    let taken = task_column(&instance, "alias", 1);
    let before = task_column(&instance, "alias", 2);

    for bad in [taken.as_str(), "12345", "Not Valid"] {
        let mut patch = empty_task_patch();
        patch["alias"] = serde_json::json!({"some": bad});
        let out = instance.call("patch_task", &["2", &patch.to_string()]);
        assert!(!out.status.success(), "{bad:?} must be refused");
        assert_eq!(task_column(&instance, "alias", 2), before);
    }

    let mut patch = empty_task_patch();
    patch["alias"] = serde_json::json!({"some": "moss"});
    let out = instance.call("patch_task", &["2", &patch.to_string()]);
    assert!(out.status.success(), "{}", describe(&out));
    assert_eq!(task_column(&instance, "alias", 2), "moss");
}

/// A task keeps its alias across an unrelated patch.
#[test]
fn an_alias_survives_an_unrelated_patch() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    let before = task_column(&instance, "alias", 1);
    set_task_field(&instance, 1, "title", serde_json::json!("renamed"));
    assert_eq!(task_column(&instance, "alias", 1), before);
}

/// The backfill reducer is a no-op once every task has an alias.
#[test]
fn backfilling_aliases_changes_nothing_when_all_are_set() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", ""), (2, "running", "")]);
    let before = (
        task_column(&instance, "alias", 1),
        task_column(&instance, "alias", 2),
    );
    let out = instance.call("backfill_task_aliases", &[]);
    assert!(out.status.success(), "{}", describe(&out));
    assert_eq!(
        before,
        (
            task_column(&instance, "alias", 1),
            task_column(&instance, "alias", 2)
        )
    );
}
