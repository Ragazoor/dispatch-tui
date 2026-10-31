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
async fn the_creator_the_poll_owners_owner_and_an_admin_may_change_it() {
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
    // The owner of the host that holds the claim, who is neither creator nor admin.
    let mut caller = scenario();
    caller.lock().poll_owners.insert(1, claim(1, 1, "h-carol"));
    let outcome = set_command(&mut caller, "carol").await;
    assert!(matches!(outcome, ReducerOutcome::Applied(_)), "{outcome:?}");
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
