//! `learnings.allium: RateLearningViaMcp` and `workspaces.allium:
//! LearningVisibility` (task #99118): rating a learning the caller cannot see
//! answers "not found", on the real module and on the twin, whether or not the
//! learning was ever retrieved for the task.

use super::move_paths::{Pair, World};
use super::*;

/// A public epic in the default workspace, with the same id on both sides.
async fn make_epic(p: &Pair<'_>) -> i64 {
    let epic = bindings::Epic {
        created_by: p.identity.clone(),
        workspace_id: DEFAULT_WORKSPACE,
        ..blank_epic()
    };
    let id = p.real.create_epic(epic.clone()).await.unwrap().0;
    p.wait(|r| r.epic(EpicId(id)).is_some()).await;
    p.mem.align_next_epic_id(id);
    assert_eq!(p.mem.create_epic(epic).await.unwrap().0, id);
    id
}

fn refusal(outcome: ReducerOutcome) -> String {
    match outcome {
        ReducerOutcome::Refused(why) => why,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn rating_an_unseen_learning_answers_not_found_on_both_stores() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = Instance::start("memory-caller-rating-visibility");
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async move {
        let world = World::start(instance).await;
        let p = world.pair();

        // A task in a public epic: both users see it, whatever happens to the
        // epic the learning below hangs on.
        let open_epic = make_epic(&p).await;
        let task_row = bindings::Task {
            created_by: p.identity.clone(),
            ..blank_task_in_epic(open_epic)
        };
        let task = p.real.create_task(task_row.clone()).await.unwrap();
        p.wait(|r| r.task(task).is_some()).await;
        p.mem.align_next_task_id(task.0);
        assert_eq!(p.mem.create_task(task_row).await.unwrap(), task);

        // A learning private to the first user: unseen, never retrieved.
        let private = bindings::Learning {
            scope: "user".into(),
            ..blank_learning()
        };
        let private_id = p.real.create_learning(private.clone()).await.unwrap();
        assert_eq!(p.mem.create_learning(private).await.unwrap(), private_id);

        // A learning on the first user's epic: seen, retrieved and rated, then
        // unseen once the epic goes private.
        let epic_id = make_epic(&p).await;
        let shared = bindings::Learning {
            scope: "epic".into(),
            scope_ref: Some(epic_id.to_string()),
            ..blank_learning()
        };
        let shared_id = p.real.create_learning(shared.clone()).await.unwrap();
        assert_eq!(p.mem.create_learning(shared).await.unwrap(), shared_id);

        let callers: [(&str, &dyn ReducerCaller); 2] =
            [("module", p.other_real), ("twin", p.other_mem)];
        for (side, caller) in callers {
            let why = refusal(
                caller
                    .apply_learning_verdicts(task, vec![(private_id, LearningVerdict::Helped)])
                    .await
                    .unwrap(),
            );
            assert!(
                why.contains(&format!("learning {} not found", private_id.0)),
                "{side}: {why}"
            );
            caller
                .record_learning_retrieval(task, shared_id, RetrievalSource::PromptInjection)
                .await
                .unwrap()
                .applied()
                .unwrap();
            caller
                .apply_learning_verdicts(task, vec![(shared_id, LearningVerdict::Helped)])
                .await
                .unwrap()
                .applied()
                .unwrap();
        }

        for caller in [p.real as &dyn ReducerCaller, p.mem] {
            caller
                .set_epic_visibility(EpicId(epic_id), "private".into())
                .await
                .unwrap()
                .applied()
                .unwrap();
        }
        let callers: [(&str, &dyn ReducerCaller); 2] =
            [("module", p.other_real), ("twin", p.other_mem)];
        for (side, caller) in callers {
            let why = refusal(
                caller
                    .apply_learning_verdicts(task, vec![(shared_id, LearningVerdict::Wrong)])
                    .await
                    .unwrap(),
            );
            assert!(
                why.contains(&format!("learning {} not found", shared_id.0)),
                "{side}: {why}"
            );
        }
    });
}
