//! `core.allium: ArchivedTask`, run as steps written once and applied to the
//! real module and to the twin (task #99227, K5).
//!
//! Every person-made delete path (`delete_task`, `batch_delete`, `delete_epic`)
//! archives what it deletes, and who reads a record is frozen at archive time:
//! the record of a task in a private epic is its holder's alone, though the
//! workspace's other member shares the workspace. Both stores must write the
//! same record and answer the same read and listing to each person.

use super::loss_of_sight::{make_epic, make_task};
use super::move_paths::World;
use super::*;

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

/// A record as both stores must agree on it: each side stamps its own clock and
/// draws its own agent name.
fn comparable(mut record: bindings::ArchivedTask) -> bindings::ArchivedTask {
    record.archived_at.clear();
    record.completed_at.clear();
    record.agent_name.clear();
    record
}

/// What one person reads of `ids` on one side: each record (or none), then the
/// listing's ids.
async fn read_as(
    caller: &dyn ReducerCaller,
    ids: &[TaskId],
) -> (Vec<Option<bindings::ArchivedTask>>, Vec<i64>) {
    let mut records = Vec::new();
    for id in ids {
        records.push(
            caller
                .read_archived_task(*id)
                .await
                .unwrap()
                .map(comparable),
        );
    }
    let mut listed: Vec<i64> = caller
        .list_archived_tasks(None, vec![])
        .await
        .unwrap()
        .iter()
        .map(|a| a.id)
        .collect();
    listed.sort_unstable();
    (records, listed)
}

#[test]
fn every_delete_path_archives_the_same_record_for_the_same_readers() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = Instance::start("memory-caller-archive-records");
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async move {
        let world = World::start(instance).await;
        let p = world.pair();

        // A private epic of the first user, in the workspace both users share,
        // and a public one.
        let private = make_epic(
            &p,
            bindings::Epic {
                visibility: "private".into(),
                ..blank_epic()
            },
        )
        .await;
        let public = make_epic(&p, blank_epic()).await;
        let by_delete_task = make_task(&p, private).await;
        let by_batch = make_task(&p, private).await;
        let by_epic_delete = make_task(&p, public).await;
        let epic_less = {
            let row = bindings::Task {
                created_by: p.identity.clone(),
                owner: p.identity.clone(),
                ..blank_task_in_epic(0)
            };
            let id = p.real.create_task(row.clone()).await.unwrap();
            p.wait(|r| r.task(id).is_some()).await;
            p.mem.align_next_task_id(id.0);
            assert_eq!(p.mem.create_task(row).await.unwrap(), id);
            id
        };

        // A delete needs a finished task.
        let done = bindings::TaskPatch {
            status: Some("done".into()),
            sub_status: Some("none".into()),
            ..blank_task_patch()
        };
        for id in [by_delete_task, by_batch, by_epic_delete, epic_less] {
            both_apply!(p, |c| c.patch_task(id, done.clone()));
        }

        both_apply!(p, |c| c.delete_task(by_delete_task));
        both_apply!(p, |c| c.batch_delete(vec![by_batch], vec![]));
        both_apply!(p, |c| c.delete_epic(EpicId(public)));
        both_apply!(p, |c| c.delete_task(epic_less));

        let ids = [by_delete_task, by_batch, by_epic_delete, epic_less];
        for (who, real, mem) in [
            (
                "the holder",
                p.real as &dyn ReducerCaller,
                p.mem as &dyn ReducerCaller,
            ),
            (
                "a member who is not the holder",
                p.other_real as &dyn ReducerCaller,
                p.other_mem as &dyn ReducerCaller,
            ),
        ] {
            let (real_records, real_listed) = read_as(real, &ids).await;
            let (mem_records, mem_listed) = read_as(mem, &ids).await;
            assert_eq!(real_records, mem_records, "{who}: the records differ");
            // Ids are aligned across the two sides, so the listings match too.
            assert_eq!(real_listed, mem_listed, "{who}: the listings differ");
            if who == "the holder" {
                assert!(real_records.iter().all(Option::is_some), "{who} reads all");
            } else {
                assert!(
                    real_records[0].is_none() && real_records[1].is_none(),
                    "{who} reads a private record"
                );
                assert!(real_records[2].is_some(), "{who} misses a public one");
                assert!(
                    real_records[3].is_none(),
                    "{who} reads another's epic-less record"
                );
            }
        }
    });
}
