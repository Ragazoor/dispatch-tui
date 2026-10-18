//! Poll ownership (`core.allium: PollOwner`), run against the in-memory
//! store. Twins of `tests/spacetime_module/poll_owner.rs`.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

fn two_hosts() -> Store {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    store.register_host_for(HOST_B, "alice");
    store
}

fn claim(store: &Store, who: &str, scope: &str, id: i64, host: &str) -> Result<(), String> {
    store.call(who, |ctx| {
        claim_poll_owner(ctx, scope.into(), id, host.into())
    })
}

fn override_owner(
    store: &Store,
    who: &str,
    scope: &str,
    id: i64,
    host: &str,
) -> Result<(), String> {
    store.call(who, |ctx| {
        override_poll_owner(ctx, scope.into(), id, host.into())
    })
}

fn owner_of(store: &Store, scope: &str, id: i64) -> Option<String> {
    store
        .db()
        .poll_owners()
        .iter()
        .find(|p| p.scope == scope && p.scope_id == id)
        .map(|p| p.host)
}

fn owner_rows(store: &Store) -> usize {
    store.db().poll_owners().iter().count()
}

/// `claim_poll_owner` fills an ABSENT row and is a no-op on an EXISTING
/// one, whoever it names — the only way an existing claim moves is
/// `override_poll_owner`. `pr-workflow.allium: PollPrStatus`.
#[test]
fn claim_poll_owner_fills_an_absent_row_but_not_an_existing_one() {
    let store = two_hosts();

    claim(&store, "alice", "task", 1, HOST_A).expect("first claim");
    assert_eq!(owner_of(&store, "task", 1).as_deref(), Some(HOST_A));

    claim(&store, "alice", "task", 1, HOST_B).expect("second claim");
    assert_eq!(
        owner_of(&store, "task", 1).as_deref(),
        Some(HOST_A),
        "claim must not steal an existing owner"
    );
    assert_eq!(owner_rows(&store), 1);
}

/// `override_poll_owner` reassigns an EXISTING claim unconditionally — the
/// only way ownership ever moves. `pr-workflow.allium: OverridePrPollOwner`.
#[test]
fn override_poll_owner_reassigns_unconditionally() {
    let store = two_hosts();
    claim(&store, "alice", "task", 1, HOST_A).expect("claim");

    override_owner(&store, "alice", "task", 1, HOST_B).expect("override");
    assert_eq!(
        owner_of(&store, "task", 1).as_deref(),
        Some(HOST_B),
        "override must reassign even an existing owner"
    );
}

/// `override_poll_owner` on a scope with no existing claim behaves like an
/// ordinary claim (`feeds.allium: OverrideFeedOwner`).
#[test]
fn override_poll_owner_with_no_existing_claim_behaves_like_a_claim() {
    let store = two_hosts();

    override_owner(&store, "alice", "epic", 1, HOST_A).expect("override");
    assert_eq!(owner_of(&store, "epic", 1).as_deref(), Some(HOST_A));

    // A later claim by another host must not steal it back.
    claim(&store, "alice", "epic", 1, HOST_B).expect("claim");
    assert_eq!(owner_of(&store, "epic", 1).as_deref(), Some(HOST_A));
}

/// Task-scope and epic-scope claims are independent rows, even when they
/// share a numeric id: `core.allium: UniquePollOwnerPerScope` keys on the
/// `scope` column as well as the id.
#[test]
fn task_and_epic_scope_claims_do_not_collide_on_the_same_id() {
    let store = two_hosts();

    claim(&store, "alice", "task", 1, HOST_A).expect("task claim");
    claim(&store, "alice", "epic", 1, HOST_B).expect("epic claim");

    assert_eq!(owner_of(&store, "task", 1).as_deref(), Some(HOST_A));
    assert_eq!(owner_of(&store, "epic", 1).as_deref(), Some(HOST_B));
    assert_eq!(owner_rows(&store), 2);
}

/// An invalid scope string is rejected rather than silently accepted: the
/// scope-parameterised pair trades compile-time safety for the runtime check
/// `require_poll_scope`, which needs its own coverage.
#[test]
fn claim_poll_owner_rejects_an_unrecognised_scope() {
    let store = two_hosts();

    let refused = claim(&store, "alice", "bogus", 1, HOST_A);
    assert!(refused.is_err(), "an unrecognised scope must be rejected");
    assert_eq!(owner_rows(&store), 0);
}

/// D11 follow-up (`store-split.allium`: ClaimPollOwner, OverridePollOwner,
/// SaveSettingForHost, ClearSettingForHost): a client may name a host in
/// these reducers only when it owns it. A teammate's host, and an
/// unregistered one, are refused and nothing changes.
#[test]
fn host_scoped_writes_refuse_a_caller_that_does_not_own_the_host() {
    let store = two_hosts();
    store.register_host_for("their-host", "teammate");
    claim(&store, "alice", "task", 1, HOST_A).expect("claim");
    store
        .call("alice", |ctx| {
            save_setting(ctx, HOST_A.into(), "theme".into(), "dark".into())
        })
        .expect("save_setting");

    let save_as = |who: &str, host: &str, value: &str| {
        store.call(who, |ctx| {
            save_setting(ctx, host.into(), "theme".into(), value.into())
        })
    };
    let clear_as = |who: &str, host: &str| {
        store.call(who, |ctx| clear_setting(ctx, host.into(), "theme".into()))
    };

    // The teammate acts for alice's host-a.
    assert!(override_owner(&store, "teammate", "task", 1, HOST_A).is_err());
    assert!(claim(&store, "teammate", "epic", 1, HOST_A).is_err());
    assert!(save_as("teammate", HOST_A, "light").is_err());
    assert!(clear_as("teammate", HOST_A).is_err());

    // Alice acts for the teammate's host, and for an unregistered one.
    assert!(override_owner(&store, "alice", "task", 1, "their-host").is_err());
    assert!(claim(&store, "alice", "epic", 1, "their-host").is_err());
    assert!(save_as("alice", "their-host", "x").is_err());
    assert!(clear_as("alice", "their-host").is_err());
    assert!(claim(&store, "alice", "epic", 1, "nowhere").is_err());
    assert!(save_as("alice", "nowhere", "x").is_err());
    assert!(clear_as("alice", "nowhere").is_err());

    assert_eq!(owner_of(&store, "task", 1).as_deref(), Some(HOST_A));
    assert_eq!(owner_rows(&store), 1);
    let settings: Vec<_> = store.db().settings().iter().collect();
    assert_eq!(settings.len(), 1);
    assert_eq!(settings[0].value, "dark");
}
