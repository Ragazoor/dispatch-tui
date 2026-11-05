//! `workspaces.allium: ClaimsFollowSight`, run as steps written once and applied
//! to the real module and to the twin (task #98913, G5).
//!
//! Every path that takes an epic out of someone's sight must release the poll
//! claims that person's hosts hold on the epic and its tasks, and only those.
//! The marks (unseen, bookmark) go with them (task #98992, B1); the second
//! user's own rows are not visible from here, so the module's and the twin's
//! tests assert the marks and this test pins the claims and the path list.
//! `spacetime/module-tests/tests/feed_authority.rs` and
//! `src/sync/memory_caller/tests/feed_authority.rs` each run the same five paths
//! on their own side and seed a legacy claim straight into a table; this test is
//! the one that proves the two sides agree on the same reducer calls.
//!
//! The managed-root private flip (`managed_root_made_private` in those two
//! tests) is not here: it needs the `feed_role` column written outside any
//! reducer, which a [`ReducerCaller`] cannot do, so the two separate tests keep
//! it. Likewise a claim by a host whose owner did not set the feed command is
//! refused by `claim_poll_owner`, so the second user claims only a task here.

use super::move_paths::{Pair, World};
use super::*;

/// Where the third party's claim must be gone, and the first user's two must
/// stay, after every path. One name per `carol_loses_sight_by` arm.
const PATHS: [&str; 5] = [
    "set_epic_visibility",
    "detach_a_task",
    "reparent_under_a_private_epic",
    "move_epic_to_workspace",
    // Last: it takes the second user out of the default workspace for good.
    "remove_member",
];

/// Run one call, written once with `$c` as the caller, on both sides.
macro_rules! both_apply {
    ($p:expr, |$c:ident| $call:expr) => {{
        {
            let $c: &dyn ReducerCaller = $p.real;
            $call.await.unwrap().applied().unwrap();
        }
        {
            let $c: &dyn ReducerCaller = $p.mem;
            $call.await.unwrap().applied().unwrap();
        }
    }};
}

/// An epic in the default workspace, created by the first user with the same id
/// on both sides.
async fn make_epic(p: &Pair<'_>, row: bindings::Epic) -> i64 {
    let row = bindings::Epic {
        created_by: p.identity.clone(),
        workspace_id: DEFAULT_WORKSPACE,
        ..row
    };
    let id = p.real.create_epic(row.clone()).await.unwrap().0;
    p.wait(|r| r.epic(EpicId(id)).is_some()).await;
    p.mem.align_next_epic_id(id);
    assert_eq!(p.mem.create_epic(row).await.unwrap().0, id);
    id
}

async fn make_task(p: &Pair<'_>, epic: i64) -> TaskId {
    let row = bindings::Task {
        created_by: p.identity.clone(),
        ..blank_task_in_epic(epic)
    };
    let id = p.real.create_task(row.clone()).await.unwrap();
    p.wait(|r| r.task(id).is_some()).await;
    p.mem.align_next_task_id(id.0);
    assert_eq!(p.mem.create_task(row).await.unwrap(), id);
    id
}

/// The host that holds `target` on one side, if any.
fn holder(rows: &SharedRows, target: PollScopeId) -> Option<String> {
    rows.poll_owner(target).map(|p| p.host)
}

async fn carol_loses_sight_by(path: &str, p: &Pair<'_>, epic: i64, elsewhere: i64, theirs: TaskId) {
    match path {
        "detach_a_task" => {
            // The first user made the task, so they may detach it onto their
            // own board; the second user cannot see a task there.
            both_apply!(p, |c| c.set_task_epic(theirs, None, String::new()));
        }
        "set_epic_visibility" => {
            both_apply!(p, |c| c.set_epic_visibility(EpicId(epic), "private".into()));
        }
        "reparent_under_a_private_epic" => {
            let private = make_epic(
                p,
                bindings::Epic {
                    visibility: "private".into(),
                    ..blank_epic()
                },
            )
            .await;
            let patch = bindings::EpicPatch {
                parent_epic_id: Some(private),
                ..blank_epic_patch()
            };
            both_apply!(p, |c| c.patch_epic(EpicId(epic), patch.clone()));
        }
        "move_epic_to_workspace" => {
            both_apply!(p, |c| c.move_epic_to_workspace(EpicId(epic), elsewhere));
        }
        "remove_member" => {
            for (caller, rows) in [
                (p.real as &dyn ReducerCaller, p.rows_real),
                (p.mem as &dyn ReducerCaller, p.rows_mem),
            ] {
                let membership = rows
                    .memberships()
                    .iter()
                    .find(|m| m.workspace_id == DEFAULT_WORKSPACE && m.user == p.other_identity)
                    .expect("the second user is a member")
                    .id;
                caller
                    .remove_member(membership)
                    .await
                    .unwrap()
                    .applied()
                    .unwrap();
            }
        }
        other => panic!("unknown loss-of-sight path {other}"),
    }
}

#[test]
fn every_loss_of_sight_path_releases_the_claims_it_strands_on_both_stores() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = Instance::start("memory-caller-loss-of-sight");
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async move {
        let world = World::start(instance).await;
        let p = world.pair();

        // Each person polls from a host of their own.
        for (host, owner, real, mem) in [
            ("h-first", &p.identity, p.real, p.mem),
            ("h-second", &p.other_identity, p.other_real, p.other_mem),
        ] {
            for caller in [real as &dyn ReducerCaller, mem as &dyn ReducerCaller] {
                caller
                    .register_host(host.into(), String::new(), owner.clone())
                    .await
                    .unwrap()
                    .applied()
                    .unwrap();
            }
        }
        // A workspace the second user is not in, for the move.
        both_apply!(p, |c| c.create_workspace("Elsewhere".into()));
        p.wait(|r| r.workspaces().iter().any(|w| w.name == "Elsewhere"))
            .await;
        let elsewhere = |rows: &SharedRows| {
            rows.workspaces()
                .iter()
                .find(|w| w.name == "Elsewhere")
                .expect("the workspace exists")
                .id
        };
        assert_eq!(elsewhere(p.rows_real), elsewhere(p.rows_mem));
        let elsewhere = elsewhere(p.rows_real);
        // The move judges by membership: the first user belongs there too, so
        // their claims must survive it.
        let identity = p.identity.clone();
        both_apply!(p, |c| c.add_member(
            elsewhere,
            identity.clone(),
            "member".into()
        ));

        for path in PATHS {
            // The first user's feed epic, with a task each person polls. A claim
            // on the epic needs the setter's host, so the second user claims
            // the task only.
            let epic = make_epic(
                &p,
                bindings::Epic {
                    feed_command: "gh-feed".into(),
                    ..blank_epic()
                },
            )
            .await;
            let (mine, theirs) = (make_task(&p, epic).await, make_task(&p, epic).await);
            let (epic_scope, mine_scope, theirs_scope) = (
                PollScopeId::Epic(EpicId(epic)),
                PollScopeId::Task(mine),
                PollScopeId::Task(theirs),
            );
            both_apply!(p, |c| c.claim_poll_owner(epic_scope, "h-first".into()));
            both_apply!(p, |c| c.claim_poll_owner(mine_scope, "h-first".into()));
            for caller in [p.other_real as &dyn ReducerCaller, p.other_mem] {
                caller
                    .claim_poll_owner(theirs_scope, "h-second".into())
                    .await
                    .unwrap()
                    .applied()
                    .unwrap();
            }
            p.wait(|r| holder(r, epic_scope).is_some() && holder(r, mine_scope).is_some())
                .await;

            carol_loses_sight_by(path, &p, epic, elsewhere, theirs).await;

            // The real store's rows arrive by subscription; the claim on the
            // second user's task is the one that must go.
            p.wait(|r| holder(r, theirs_scope).is_none()).await;
            for (side, rows) in [("module", p.rows_real), ("twin", p.rows_mem)] {
                assert_eq!(
                    holder(rows, theirs_scope),
                    None,
                    "{path}: the {side} kept the claim of a host whose owner lost sight"
                );
                assert_eq!(
                    holder(rows, epic_scope).as_deref(),
                    Some("h-first"),
                    "{path}: the {side} released the epic claim of a host that still sees it"
                );
                assert_eq!(
                    holder(rows, mine_scope).as_deref(),
                    Some("h-first"),
                    "{path}: the {side} released the task claim of a host that still sees it"
                );
            }
        }
    });
}

/// The path list names every arm of `carol_loses_sight_by`, and the module's
/// own test covers the same set plus the managed-root flip: a path added to one
/// and not the other fails here rather than silently going unrun on one side.
#[test]
fn the_conformance_paths_are_the_modules_paths_minus_the_managed_root_flip() {
    let module = include_str!("../../spacetime/module-tests/tests/feed_authority.rs");
    let listed = module
        .split("const LOSS_OF_SIGHT_PATHS")
        .nth(1)
        .and_then(|rest| rest.split("];").next())
        .expect("the module test lists its paths");
    let mut module_paths: Vec<&str> = listed
        .split('"')
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, name)| name)
        .collect();
    module_paths.retain(|name| *name != "managed_root_made_private");
    module_paths.sort_unstable();
    let mut here = PATHS.to_vec();
    here.sort_unstable();
    assert_eq!(here, module_paths);
}
