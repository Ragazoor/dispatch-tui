//! Identities are the caller's, never a client's claim (task #98755, V4/V5;
//! `workspaces.allium`: "Identities are the caller's").
//!
//! A task that lands on a user board takes the CALLER as owner, and a created
//! task is stamped `created_by = caller`; `unsubscribe_from_epic` refuses a
//! subscriber that is not the caller and hides epics the caller cannot see;
//! `claim_store_owner` records a publisher only while none is recorded.

mod common;
use common::*;
use dispatch_spacetime_module::*;
use spacetimedb::Table;

fn subscribe(store: &Store, who: &str, epic: i64) {
    store
        .call(who, |ctx| subscribe_to_epic(ctx, identity_of(who), epic))
        .expect("subscribe");
}

fn follows(store: &Store, who: &str, epic: i64) -> bool {
    let id = subscription_id(&identity_of(who), epic);
    store.db().subscriptions().id().find(&id).is_some()
}

#[test]
fn an_epic_less_create_takes_the_caller_as_owner() {
    let store = Store::new();
    // Bob names Alice as owner of a task he creates: it lands on Bob's board.
    let id = store.create("bob", user_task("T", "alice"));
    assert_eq!(store.task(id).owner, identity_of("bob"));
}

#[test]
fn a_signalled_epic_less_create_takes_the_caller_as_owner() {
    let store = Store::new();
    store
        .call("bob", |ctx| {
            create_task_signalled(ctx, user_task("T", "alice"))
        })
        .expect("create");
    let task = store
        .db()
        .tasks()
        .iter()
        .next()
        .expect("the task was created");
    assert_eq!(task.owner, identity_of("bob"));
    assert_eq!(task.created_by, identity_of("bob"));
}

#[test]
fn a_created_task_is_stamped_with_its_creator() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    let forged = Task {
        created_by: identity_of("carol"),
        ..epic_task("T", epic, "backlog")
    };
    let id = store.create("bob", forged);
    assert_eq!(store.task(id).created_by, identity_of("bob"));
}

#[test]
fn a_member_cannot_detach_a_task_from_its_workspace_whatever_owner_is_named() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    let id = store.create("alice", epic_task("T", epic, "backlog"));

    let why = store
        .call("alice", |ctx| set_task_epic(ctx, id, 0, "nobody".into()))
        .unwrap_err();
    assert!(why.contains("stays in its workspace"), "{why}");
    assert_eq!(store.task(id).epic_id, epic);
}

#[test]
fn update_task_cannot_detach_a_task_from_its_workspace() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    let id = store.create("alice", epic_task("T", epic, "backlog"));

    let why = store
        .call("alice", |ctx| {
            update_task(
                ctx,
                id,
                Some(0),
                identity_of("bob"),
                String::new(),
                TaskPatch::default(),
            )
        })
        .unwrap_err();
    assert!(why.contains("stays in its workspace"), "{why}");
    assert_eq!(store.task(id).epic_id, epic);
}

#[test]
fn the_store_itself_may_name_the_owner_of_a_detached_task() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    let id = store.create("alice", epic_task("T", epic, "backlog"));

    store
        .call_as_scheduler(|ctx| set_task_epic(ctx, id, 0, identity_of("bob")))
        .expect("detach");
    assert_eq!(store.task(id).owner, identity_of("bob"));
}

#[test]
fn unsubscribe_refuses_a_subscriber_that_is_not_the_caller() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    subscribe(&store, "bob", epic);

    let err = store
        .call("alice", |ctx| {
            unsubscribe_from_epic(ctx, identity_of("bob"), epic)
        })
        .expect_err("Alice may not unfollow for Bob");
    assert!(err.contains("caller"), "{err}");
    assert!(follows(&store, "bob", epic));
}

#[test]
fn unsubscribe_removes_the_callers_own_follow() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    subscribe(&store, "bob", epic);
    store
        .call("bob", |ctx| {
            unsubscribe_from_epic(ctx, identity_of("bob"), epic)
        })
        .expect("unsubscribe");
    assert!(!follows(&store, "bob", epic));
}

#[test]
fn unsubscribe_answers_an_epic_the_caller_cannot_see_as_missing() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    // Carol is in no workspace that holds the epic: bare store, no sight.
    let err = store
        .call("carol", |ctx| {
            unsubscribe_from_epic(ctx, identity_of("carol"), epic + 100)
        })
        .expect_err("no such epic");
    assert!(err.contains("no epic"), "{err}");
}

#[test]
fn claim_store_owner_records_the_caller_once() {
    let store = Store::bare();
    store
        .call("publisher", |ctx| claim_store_owner(ctx))
        .expect("the first claim");
    let err = store
        .call("board", |ctx| claim_store_owner(ctx))
        .expect_err("a recorded publisher cannot be replaced");
    assert!(err.contains("already"), "{err}");
    // The publisher now passes the seed gate; the board does not.
    assert!(store
        .call("publisher", |ctx| seed_subscriptions(ctx, vec![]))
        .is_ok());
    assert!(store
        .call("board", |ctx| seed_subscriptions(ctx, vec![]))
        .is_err());
}
