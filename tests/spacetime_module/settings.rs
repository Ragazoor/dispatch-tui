use super::*;

// ---------------------------------------------------------------------------
// Phase 9: settings (`docs/specs/settings.allium`)
// ---------------------------------------------------------------------------

/// `save_setting` upserts by `(host, key)`, the same shape `register_host`
/// upserts by id.
#[test]
fn save_setting_upserts_by_host_and_key() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    register_own_hosts(&instance);

    let first = instance.call("save_setting", &["host-a", "theme", "dark"]);
    assert!(first.status.success(), "{}", describe(&first));
    let second = instance.call("save_setting", &["host-a", "theme", "light"]);
    assert!(second.status.success(), "{}", describe(&second));

    assert_eq!(column(&instance, "SELECT count(*) AS c FROM settings"), "1");
    assert_eq!(
        column(
            &instance,
            "SELECT value FROM settings WHERE id = 'host-a/theme'"
        ),
        "light"
    );
}

/// **Test 1 of task #4913, exercised against the real store**: a setting
/// written by one host does not affect, and is not returned as, another
/// host's row.
#[test]
fn a_setting_written_by_one_host_is_a_separate_row_from_anothers() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    register_own_hosts(&instance);

    instance.call("save_setting", &["host-a", "theme", "dark"]);
    instance.call("save_setting", &["host-b", "theme", "light"]);

    assert_eq!(column(&instance, "SELECT count(*) AS c FROM settings"), "2");
    assert_eq!(
        column(
            &instance,
            "SELECT value FROM settings WHERE id = 'host-a/theme'"
        ),
        "dark",
        "host-a's own write must be unaffected by host-b's"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT value FROM settings WHERE id = 'host-b/theme'"
        ),
        "light"
    );
}

#[test]
fn clear_setting_removes_the_row() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    register_own_hosts(&instance);
    instance.call("save_setting", &["host-a", "theme", "dark"]);

    let cleared = instance.call("clear_setting", &["host-a", "theme"]);
    assert!(cleared.status.success(), "{}", describe(&cleared));

    assert!(no_rows(
        &instance,
        "SELECT id FROM settings WHERE id = 'host-a/theme'"
    ));
}

/// Clearing a key that was never set is a no-op, not a refusal
/// (`settings.allium: ClearSetting`).
#[test]
fn clearing_an_unset_setting_is_a_no_op() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    register_own_hosts(&instance);

    let cleared = instance.call("clear_setting", &["host-a", "never-set"]);
    assert!(cleared.status.success(), "{}", describe(&cleared));
}
