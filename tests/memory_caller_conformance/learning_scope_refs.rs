//! Task #99287: the learning `scope_ref` refusals, the embedding repair
//! write, the re-scope on a manual repo-group delete and the refusal order of a
//! package link, on the real module and on the twin.
//! (`learnings.allium: RecordLearningViaMcp`, `ReScopeLearningsOnRepoGroupDelete`.)

use super::move_paths::{refusal_of as refusal, Pair, World};
use super::*;

/// One learning, with the same id on both sides.
async fn learning_both(p: &Pair<'_>, row: bindings::Learning) -> LearningId {
    let id = p.real.create_learning(row.clone()).await.unwrap();
    p.wait(|r| r.learning(id).is_some()).await;
    p.mem.align_next_learning_id(id.0);
    assert_eq!(p.mem.create_learning(row).await.unwrap(), id);
    id
}

#[test]
fn learning_scope_refs_and_package_links_match_the_module() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = Instance::start("memory-caller-learning-scope-refs");
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async move {
        let world = World::start(instance).await;
        let p = world.pair();

        // -- a blank repo ref, and an epic ref that names no epic ---------------
        for (scope, scope_ref) in [
            ("repo", ""),
            ("repo", "  "),
            ("epic", "9999"),
            ("epic", "abc"),
        ] {
            let row = bindings::Learning {
                scope: scope.into(),
                scope_ref: Some(scope_ref.into()),
                ..blank_learning()
            };
            let real = p.real.create_learning(row.clone()).await;
            let mem = p.mem.create_learning(row).await;
            match (real, mem) {
                (Err(r), Err(m)) => assert_eq!(
                    r.to_string(),
                    m.to_string(),
                    "{scope} {scope_ref:?}: the two refusals differ"
                ),
                other => panic!("{scope} {scope_ref:?}: both must refuse, got {other:?}"),
            }
        }

        // -- the embedding repair write leaves updated_at -----------------------
        let user = learning_both(&p, blank_learning()).await;
        let stamp = p.rows_real.learning(user).unwrap().updated_at;
        assert_eq!(p.rows_mem.learning(user).unwrap().updated_at, stamp);
        let embed = || bindings::LearningPatch {
            status: None,
            summary: None,
            embedding: Some(Some(vec![1, 2, 3, 4])),
        };
        p.real
            .patch_learning(user, embed())
            .await
            .unwrap()
            .applied()
            .unwrap();
        p.wait(|r| r.learnings_missing_embedding().iter().all(|l| l.id != user))
            .await;
        p.mem
            .patch_learning(user, embed())
            .await
            .unwrap()
            .applied()
            .unwrap();
        assert_eq!(
            p.rows_real.learning(user).unwrap().updated_at,
            stamp,
            "module"
        );
        assert_eq!(p.rows_mem.learning(user).unwrap().updated_at, stamp, "twin");

        // -- a repo-group sub-epic deleted by hand moves its learnings up -------
        let root = p.epic(true).await;
        let in_root = bindings::Task {
            repo_path: "/x/alpha".into(),
            created_by: p.identity.clone(),
            ..blank_task_in_epic(root)
        };
        let task = p.real.create_task(in_root.clone()).await.unwrap();
        p.wait(|r| r.task(task).is_some()).await;
        // The create routes the task into a repo-group sub-epic it makes, so
        // the twin's next epic id is aligned to that one too.
        let sub = p.rows_real.task(task).unwrap().epic_id.unwrap();
        p.mem.align_next_task_id(task.0);
        p.mem.align_next_epic_id(sub.0);
        assert_eq!(p.mem.create_task(in_root).await.unwrap(), task);
        assert_eq!(p.rows_mem.task(task).unwrap().epic_id, Some(sub));
        let done = || bindings::TaskPatch {
            status: Some("done".into()),
            ..blank_task_patch()
        };
        p.real
            .patch_task(task, done())
            .await
            .unwrap()
            .applied()
            .unwrap();
        p.wait(|r| {
            r.task(task)
                .is_some_and(|t| t.status == dispatch_tui::models::TaskStatus::Done)
        })
        .await;
        p.mem
            .patch_task(task, done())
            .await
            .unwrap()
            .applied()
            .unwrap();
        let scoped = learning_both(
            &p,
            bindings::Learning {
                scope: "epic".into(),
                scope_ref: Some(sub.0.to_string()),
                ..blank_learning()
            },
        )
        .await;
        p.real.delete_epic(sub).await.unwrap().applied().unwrap();
        let root_ref = root.to_string();
        p.wait(|r| {
            r.learning(scoped)
                .is_some_and(|l| l.scope_ref.as_deref() == Some(root_ref.as_str()))
        })
        .await;
        p.mem.delete_epic(sub).await.unwrap().applied().unwrap();
        assert!(p.rows_mem.epic(sub).is_none());
        assert_eq!(
            p.rows_real.learning(scoped).map(|l| learning_shape(&l)),
            p.rows_mem.learning(scoped).map(|l| learning_shape(&l)),
            "the re-scoped learning"
        );

        // -- a link into a missing package names the package, whatever the item --
        let package_row = bindings::WorkPackage {
            id: 0,
            title: "Plan".into(),
            description: String::new(),
            sort_order: None,
            created_by: p.identity.clone(),
            created_at: "2026-01-01 00:00:00.000".into(),
            updated_at: "2026-01-01 00:00:00.000".into(),
            workspace_id: 0,
            visibility: "public".into(),
        };
        let real_package = p
            .real
            .create_work_package(package_row.clone())
            .await
            .unwrap();
        let mem_package = p.mem.create_work_package(package_row).await.unwrap();
        // Deleted again on both sides: the link below names a missing package.
        p.real
            .delete_work_package(real_package)
            .await
            .unwrap()
            .applied()
            .unwrap();
        p.mem
            .delete_work_package(mem_package)
            .await
            .unwrap()
            .applied()
            .unwrap();
        let missing_task = dispatch_tui::models::WorkPackageItem::Task(TaskId(987_654));
        let sides: [(
            &str,
            &dyn ReducerCaller,
            dispatch_tui::models::WorkPackageId,
        ); 2] = [
            ("module", p.other_real, real_package),
            ("twin", p.other_mem, mem_package),
        ];
        for (side, caller, package) in sides {
            let why = refusal(
                caller
                    .link_work_package_item(package, missing_task)
                    .await
                    .unwrap(),
            );
            assert!(
                why.ends_with(&format!("no work package {package}")),
                "{side}: {why}"
            );
        }

        // -- rating for a task that does not exist: "task N not found" ----------
        let callers: [(&str, &dyn ReducerCaller); 2] = [("module", p.real), ("twin", p.mem)];
        for (side, caller) in callers {
            let why = refusal(
                caller
                    .apply_learning_verdicts(TaskId(999_999), vec![(user, LearningVerdict::Helped)])
                    .await
                    .unwrap(),
            );
            assert!(why.ends_with("task 999999 not found"), "{side}: {why}");
        }
    });
}
