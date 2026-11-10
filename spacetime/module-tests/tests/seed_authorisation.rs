//! Who may call the `seed_*` reducers (`spacetime-seed.allium: StorePublisher`).
//!
//! The restore path writes rows with the ids and owners they already have, so
//! it is the one door that skips every permission rule. Only the store's own
//! identity or its publisher (stamped at `init`) may use it; a board may not.

mod common;
use common::*;
use dispatch_spacetime_module::*;
use spacetimedb::Table;

/// A fresh store whose publisher is `publisher`: `init` runs as that client,
/// as it does when the CLI publishes a new database.
fn published_by(publisher: &str) -> Store {
    let store = Store::bare();
    store
        .call(publisher, |ctx| {
            init(ctx);
            Ok(())
        })
        .expect("init");
    store
}

macro_rules! every_seed_reducer {
    ($each:ident) => {
        $each!(seed_tasks);
        $each!(seed_epics);
        $each!(seed_todos);
        $each!(seed_task_watchers);
        $each!(seed_work_packages);
        $each!(seed_work_package_links);
        $each!(seed_agents);
        $each!(seed_handed_out_agent_names);
        $each!(seed_repo_paths);
        $each!(seed_repo_base_branches);
        $each!(seed_poll_owners);
        $each!(seed_task_shells);
        $each!(seed_task_subagents);
        $each!(seed_hosts);
        $each!(seed_subscriptions);
        $each!(seed_settings);
        $each!(seed_filter_presets);
        $each!(seed_learnings);
        $each!(seed_learning_verdicts);
        $each!(seed_learning_retrievals);
        $each!(seed_usage_events);
        $each!(seed_retired_feed_items);
        $each!(seed_user_profiles);
        $each!(seed_workspaces);
        $each!(seed_memberships);
    };
}

#[test]
fn a_board_identity_is_refused_by_every_seed_reducer() {
    let store = published_by("publisher");
    macro_rules! refused {
        ($reducer:ident) => {
            let err = store
                .call("board", |ctx| $reducer(ctx, vec![]))
                .expect_err(concat!(stringify!($reducer), " must refuse a board"));
            assert!(err.contains("restore"), "{}: {err}", stringify!($reducer));
        };
    }
    every_seed_reducer!(refused);
}

#[test]
fn the_publisher_may_call_every_seed_reducer() {
    let store = published_by("publisher");
    macro_rules! allowed {
        ($reducer:ident) => {
            store
                .call("publisher", |ctx| $reducer(ctx, vec![]))
                .expect(stringify!($reducer));
        };
    }
    every_seed_reducer!(allowed);
}

#[test]
fn the_store_itself_may_call_every_seed_reducer() {
    let store = published_by("publisher");
    macro_rules! allowed {
        ($reducer:ident) => {
            store
                .call_as_scheduler(|ctx| $reducer(ctx, vec![]))
                .expect(stringify!($reducer));
        };
    }
    every_seed_reducer!(allowed);
}

#[test]
fn a_store_with_no_recorded_publisher_refuses_boards() {
    // A store published before the owner was recorded: fail closed.
    let store = Store::bare();
    let err = store
        .call("board", |ctx| seed_user_profiles(ctx, vec![]))
        .expect_err("refused");
    assert!(err.contains("restore"), "{err}");
}

#[test]
fn a_board_cannot_make_itself_super_admin_through_seeding() {
    let store = published_by("publisher");
    let me = identity_of("board");
    let row = UserProfile {
        identity: me.clone(),
        display_name: String::new(),
        is_super_admin: true,
        created_at: String::new(),
    };
    assert!(store
        .call("board", |ctx| seed_user_profiles(ctx, vec![row]))
        .is_err());
    assert!(store.db().user_profiles().identity().find(&me).is_none());
}

#[test]
fn init_records_the_first_publisher_only() {
    let store = published_by("publisher");
    // A second init (it never runs twice in production) must not hand the
    // role to whoever calls it.
    let _ = store.call("board", |ctx| {
        init(ctx);
        Ok(())
    });
    assert!(store
        .call("board", |ctx| seed_settings(ctx, vec![]))
        .is_err());
}

// ---- V7: the module stamps who a write is for; it does not take the client's word.

#[test]
fn a_board_cannot_subscribe_someone_else_to_an_epic() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    store.enrol("bob");
    let err = store
        .call("alice", |ctx| {
            subscribe_to_epic(ctx, identity_of("bob"), epic)
        })
        .expect_err("alice may not name bob");
    assert!(err.contains("own identity"), "{err}");
    assert!(store.db().subscriptions().iter().next().is_none());
    store
        .call("alice", |ctx| {
            subscribe_to_epic(ctx, identity_of("alice"), epic)
        })
        .expect("her own subscription");
}

#[test]
fn the_store_itself_may_subscribe_anyone() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    store
        .call_as_scheduler(|ctx| subscribe_to_epic(ctx, identity_of("bob"), epic))
        .expect("operator");
}

#[test]
fn a_board_cannot_clear_someone_elses_unseen_marks() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    store.enrol("bob");
    let task = store.create_signalled("alice", epic_task("t", epic, "backlog"));
    let err = store
        .call("alice", |ctx| mark_task_seen(ctx, identity_of("bob"), task))
        .expect_err("alice may not name bob");
    assert!(err.contains("own identity"), "{err}");
}

#[test]
fn a_created_epic_is_attributed_to_its_caller_whatever_the_row_says() {
    let store = Store::new();
    store.enrol("alice");
    let row = Epic {
        title: "E".into(),
        workspace_id: TEST_WORKSPACE,
        created_by: identity_of("bob"),
        ..blank_epic()
    };
    store
        .call("alice", |ctx| create_epic(ctx, row))
        .expect("create_epic");
    let epic = store.db().epics().iter().next().expect("epic");
    assert_eq!(epic.created_by, identity_of("alice"));
}

#[test]
fn a_created_work_package_is_attributed_to_its_caller_whatever_the_row_says() {
    let store = Store::new();
    store.enrol("alice");
    let row = WorkPackage {
        title: "P".into(),
        workspace_id: TEST_WORKSPACE,
        created_by: identity_of("bob"),
        ..blank_work_package()
    };
    store
        .call("alice", |ctx| create_work_package(ctx, row))
        .expect("create_work_package");
    let package = store.db().work_packages().iter().next().expect("package");
    assert_eq!(package.created_by, identity_of("alice"));
}

#[test]
fn the_store_itself_may_name_the_creator_of_an_epic() {
    let store = Store::new();
    let row = Epic {
        title: "E".into(),
        workspace_id: TEST_WORKSPACE,
        created_by: identity_of("bob"),
        ..blank_epic()
    };
    store
        .call_as_scheduler(|ctx| create_epic(ctx, row))
        .expect("operator");
    let epic = store.db().epics().iter().next().expect("epic");
    assert_eq!(epic.created_by, identity_of("bob"));
}

#[test]
fn a_board_cannot_burn_an_id_sequence() {
    let store = published_by("publisher");
    let err = store
        .call("board", |ctx| burn_id_sequence(ctx, "tasks".into(), 3))
        .expect_err("refused");
    assert!(err.contains("restore"), "{err}");
}

#[test]
fn a_board_cannot_probe_the_task_id_sequence() {
    let store = published_by("publisher");
    let err = store
        .call("board", probe_generated_task_id)
        .expect_err("refused");
    assert!(err.contains("restore"), "{err}");
}

#[test]
fn a_board_cannot_rewrite_the_schema_version() {
    let store = published_by("publisher");
    let err = store
        .call("board", |ctx| set_schema_version(ctx, 97))
        .expect_err("refused");
    assert!(err.contains("restore"), "{err}");
}

#[test]
fn the_publisher_may_use_the_restore_only_sequence_and_version_reducers() {
    let store = published_by("publisher");
    store
        .call("publisher", |ctx| burn_id_sequence(ctx, "tasks".into(), 3))
        .expect("burn");
    store
        .call("publisher", probe_generated_task_id)
        .expect("probe");
    store
        .call("publisher", |ctx| set_schema_version(ctx, 97))
        .expect("version");
}

#[test]
fn a_second_claimant_is_refused_and_the_first_stays_publisher() {
    // A store published before `store_owner` existed: no row until claimed.
    let store = Store::bare();
    store
        .call("publisher", |ctx| claim_store_owner(ctx))
        .expect("first claim");
    let err = store
        .call("board", |ctx| claim_store_owner(ctx))
        .expect_err("second claim refused");
    assert!(err.contains("already"), "{err}");
    assert!(store
        .call("board", |ctx| seed_user_profiles(ctx, vec![]))
        .is_err());
    store
        .call("publisher", |ctx| seed_user_profiles(ctx, vec![]))
        .expect("the first claimant restores");
}
