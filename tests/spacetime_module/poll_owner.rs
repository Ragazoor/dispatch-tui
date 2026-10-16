use super::*;

// ---------------------------------------------------------------------------
// Phase 7: poll ownership (`core.allium: PollOwner`)
// ---------------------------------------------------------------------------

/// `claim_poll_owner` fills an ABSENT row and is a no-op on an EXISTING
/// one, whoever it names — the only way an existing claim moves is
/// `override_poll_owner`. `pr-workflow.allium: PollPrStatus`.
#[test]
fn claim_poll_owner_fills_an_absent_row_but_not_an_existing_one() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    register_own_hosts(&instance);
    running_task(&instance, 1);

    let first = instance.call("claim_poll_owner", &["task", "1", "host-a"]);
    assert!(first.status.success(), "{}", describe(&first));
    assert_eq!(
        column(
            &instance,
            "SELECT host FROM poll_owners WHERE scope = 'task' AND scope_id = 1"
        ),
        "host-a"
    );

    // A second claim by a different host must not steal it.
    let second = instance.call("claim_poll_owner", &["task", "1", "host-b"]);
    assert!(second.status.success(), "{}", describe(&second));
    assert_eq!(
        column(
            &instance,
            "SELECT host FROM poll_owners WHERE scope = 'task' AND scope_id = 1"
        ),
        "host-a",
        "claim must not steal an existing owner"
    );
    assert_eq!(
        column(&instance, "SELECT count(*) AS c FROM poll_owners"),
        "1"
    );
}

/// `override_poll_owner` reassigns an EXISTING claim unconditionally —
/// the only way ownership ever moves. `pr-workflow.allium:
/// OverridePrPollOwner`.
#[test]
fn override_poll_owner_reassigns_unconditionally() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    register_own_hosts(&instance);
    running_task(&instance, 1);

    let claimed = instance.call("claim_poll_owner", &["task", "1", "host-a"]);
    assert!(claimed.status.success(), "{}", describe(&claimed));

    let overridden = instance.call("override_poll_owner", &["task", "1", "host-b"]);
    assert!(overridden.status.success(), "{}", describe(&overridden));
    assert_eq!(
        column(
            &instance,
            "SELECT host FROM poll_owners WHERE scope = 'task' AND scope_id = 1"
        ),
        "host-b",
        "override must reassign even an existing owner"
    );
}

/// `override_poll_owner` on a scope with no existing claim behaves like
/// an ordinary claim — `feeds.allium: OverrideFeedOwner`'s "an override with
/// no existing owner behaves like a normal claim".
#[test]
fn override_poll_owner_with_no_existing_claim_behaves_like_a_claim() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    register_own_hosts(&instance);
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "Feed Epic", "backlog", 0)]).to_string()],
    );

    let overridden = instance.call("override_poll_owner", &["epic", "1", "host-a"]);
    assert!(overridden.status.success(), "{}", describe(&overridden));
    assert_eq!(
        column(
            &instance,
            "SELECT host FROM poll_owners WHERE scope = 'epic' AND scope_id = 1"
        ),
        "host-a"
    );

    // A later claim by another host must not steal it back.
    let claimed = instance.call("claim_poll_owner", &["epic", "1", "host-b"]);
    assert!(claimed.status.success(), "{}", describe(&claimed));
    assert_eq!(
        column(
            &instance,
            "SELECT host FROM poll_owners WHERE scope = 'epic' AND scope_id = 1"
        ),
        "host-a"
    );
}

/// Task-scope and epic-scope claims are independent rows, even when they
/// happen to share the same numeric id — the `scope` column, not the id
/// alone, is what `core.allium: UniquePollOwnerPerScope` keys on.
#[test]
fn task_and_epic_scope_claims_do_not_collide_on_the_same_id() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    register_own_hosts(&instance);
    running_task(&instance, 1);
    instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "Feed Epic", "backlog", 0)]).to_string()],
    );

    let task_claim = instance.call("claim_poll_owner", &["task", "1", "host-a"]);
    assert!(task_claim.status.success(), "{}", describe(&task_claim));
    let epic_claim = instance.call("claim_poll_owner", &["epic", "1", "host-b"]);
    assert!(epic_claim.status.success(), "{}", describe(&epic_claim));

    assert_eq!(
        column(
            &instance,
            "SELECT host FROM poll_owners WHERE scope = 'task' AND scope_id = 1"
        ),
        "host-a"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT host FROM poll_owners WHERE scope = 'epic' AND scope_id = 1"
        ),
        "host-b"
    );
    assert_eq!(
        column(&instance, "SELECT count(*) AS c FROM poll_owners"),
        "2"
    );
}

/// An invalid scope string is rejected rather than silently accepted — the
/// collapse from four scope-specific reducers to one scope-parameterised pair
/// traded compile-time scope safety (four distinct function names) for a
/// runtime check (`require_poll_scope`), so that check needs its own coverage.
#[test]
fn claim_poll_owner_rejects_an_unrecognised_scope() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    register_own_hosts(&instance);

    let result = instance.call("claim_poll_owner", &["bogus", "1", "host-a"]);
    assert!(
        !result.status.success(),
        "an unrecognised scope must be rejected, got {}",
        describe(&result)
    );
    assert_eq!(
        column(&instance, "SELECT count(*) AS c FROM poll_owners"),
        "0"
    );
}

/// D11 follow-up (`store-split.allium`: ClaimPollOwner, OverridePollOwner,
/// SaveSettingForHost, ClearSettingForHost): a client may name a host in these
/// reducers only when it owns it. A teammate's host, an ownerless one and an
/// unregistered one are all refused, and nothing changes.
#[test]
fn host_scoped_writes_refuse_a_caller_that_does_not_own_the_host() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    register_own_hosts(&instance);
    register_teammates_host(&instance, "their-host");
    running_task(&instance, 1);
    instance.call("claim_poll_owner", &["task", "1", "host-a"]);
    instance.call("save_setting", &["host-a", "theme", "dark"]);

    // The teammate acts for my host-a.
    let teammate = |reducer: &str, args: &[&str]| instance.call_as("teammate", reducer, args);
    for out in [
        teammate("override_poll_owner", &["task", "1", "host-a"]),
        teammate("claim_poll_owner", &["epic", "1", "host-a"]),
        teammate("save_setting", &["host-a", "theme", "light"]),
        teammate("clear_setting", &["host-a", "theme"]),
    ] {
        assert!(!out.status.success(), "{}", describe(&out));
    }
    // I act for the teammate's host, an unregistered one.
    for out in [
        instance.call("override_poll_owner", &["task", "1", "their-host"]),
        instance.call("claim_poll_owner", &["epic", "1", "their-host"]),
        instance.call("save_setting", &["their-host", "theme", "x"]),
        instance.call("clear_setting", &["their-host", "theme"]),
        instance.call("claim_poll_owner", &["epic", "1", "nowhere"]),
        instance.call("save_setting", &["nowhere", "theme", "x"]),
        instance.call("clear_setting", &["nowhere", "theme"]),
    ] {
        assert!(!out.status.success(), "{}", describe(&out));
    }

    assert_eq!(
        column(
            &instance,
            "SELECT host FROM poll_owners WHERE scope = 'task' AND scope_id = 1"
        ),
        "host-a"
    );
    assert_eq!(
        column(&instance, "SELECT count(*) AS c FROM poll_owners"),
        "1"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT value FROM settings WHERE id = 'host-a/theme'"
        ),
        "dark"
    );
    assert_eq!(column(&instance, "SELECT count(*) AS c FROM settings"), "1");
}

/// The backlog claim reducers are gone, not checked: a client cannot claim a
/// task for a host any more (`store-split.allium`, D11 follow-up).
#[test]
fn the_backlog_claim_reducers_are_gone() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    for out in [
        instance.call("claim_backlog_task", &["1", "host-a"]),
        instance.call("release_backlog_claim", &["1"]),
    ] {
        assert!(!out.status.success(), "{}", describe(&out));
    }
}
