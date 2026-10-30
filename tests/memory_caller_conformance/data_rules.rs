use super::*;

/// The module's `MAX_EPIC_DEPTH`, which a delete refuses to walk past.
const MODULE_MAX_EPIC_DEPTH: usize = 64;

/// Store-split M2 (task #53255): the data rules that moved into the reducers
/// are enforced identically by `MemoryReducerCaller`.
///
/// The twin calls the module's own pure rule functions, so the verdicts agree
/// by construction; what this checks is the part written by hand on each side —
/// routing a task into its repo sub-epic, rerouting on a repo change, cleaning
/// up the emptied sub-epic, and `regroup_epic`/`flatten_epic` — and that a
/// refusal leaves the same state behind.
#[test]
fn memory_caller_enforces_the_same_data_rules_as_the_real_reducers() {
    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start("memory-caller-rules");
    let published = instance.publish(&module_path(), None);
    assert!(published.status.success(), "{}", describe(&published));

    let rows_real = Arc::new(SharedRows::new());
    let rows_mem = Arc::new(SharedRows::new());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async move {
        let connector = Arc::new(SpacetimeSdkConnector::new(
            instance.database(),
            rows_real.clone(),
        ));
        let accepted = connector
            .connect(&instance.host(), None)
            .await
            .unwrap_or_else(|e| panic!("connect: {e}"));
        // Epics the scenario creates are seen through `own_creations`
        // (`created_by = identity`), so the twin writes as the same identity.
        let mem = MemoryReducerCaller::new(rows_mem.clone(), clock)
            .with_sender(accepted.identity.clone())
            .with_workspace_rules();
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                vec![],
                "host-a",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));
        let real = SdkReducerCaller::new(connector, Arc::new(SettledIdentity::default()));
        join_default_workspace(&real, &mem, &accepted.identity).await;
        let identity = accepted.identity.clone();

        let mut woken = rows_real.changed();
        macro_rules! wait_for {
            ($cond:expr) => {
                tokio::time::timeout(Duration::from_secs(10), async {
                    while !$cond {
                        woken.changed().await.expect("subscription must deliver");
                    }
                })
                .await
                .unwrap_or_else(|_| panic!("real store never reached the expected state"))
            };
        }
        let same_epic = |id: i64| {
            let shape = |rows: &SharedRows| {
                rows.epic(EpicId(id)).map(|e| EpicShape {
                    created_at: chrono::DateTime::UNIX_EPOCH,
                    ..epic_shape(&e)
                })
            };
            assert_eq!(shape(&rows_real), shape(&rows_mem), "epic {id}");
        };
        let same_task = |id: i64| {
            assert_eq!(
                rows_real.task(TaskId(id)).map(|t| task_shape(&t)),
                rows_mem.task(TaskId(id)).map(|t| task_shape(&t)),
                "task {id}"
            );
        };
        // Both refused, with the same reason, and nothing changed.
        let refused = |real: ReducerOutcome, mem: ReducerOutcome| -> String {
            match (real, mem) {
                (ReducerOutcome::Refused(r), ReducerOutcome::Refused(m)) => {
                    assert!(
                        r.contains(&m) || m.contains(&r),
                        "real {r:?} vs memory {m:?}"
                    );
                    m
                }
                other => panic!("both sides must refuse, got {other:?}"),
            }
        };

        // -- a grouped root, and a task created in it ---------------------------
        let grouped = bindings::Epic {
            group_by_repo: true,
            created_by: identity.clone(),
            ..blank_epic()
        };
        let root = real.create_epic(grouped.clone()).await.unwrap().0;
        wait_for!(rows_real.epic(EpicId(root)).is_some());
        mem.align_next_epic_id(root);
        assert_eq!(mem.create_epic(grouped).await.unwrap().0, root);

        let in_root = |repo: &str| bindings::Task {
            repo_path: repo.into(),
            created_by: identity.clone(),
            ..blank_task_in_epic(root)
        };
        let task = real.create_task(in_root("/x/alpha")).await.unwrap();
        wait_for!(rows_real.task(task).is_some());
        mem.align_next_task_id(task.0);
        assert_eq!(mem.create_task(in_root("/x/alpha")).await.unwrap(), task);
        same_task(task.0);
        let sub = rows_real.task(task).unwrap().epic_id.unwrap().0;
        assert_ne!(sub, root, "the task was routed into a repo sub-epic");
        wait_for!(rows_real.epic(EpicId(sub)).is_some());
        same_epic(sub);
        same_epic(root);

        // -- sub-status legality -------------------------------------------------
        let illegal = bindings::TaskPatch {
            sub_status: Some("active".into()),
            ..blank_task_patch()
        };
        let why = refused(
            real.patch_task(task, illegal.clone()).await.unwrap(),
            mem.patch_task(task, illegal).await.unwrap(),
        );
        assert!(why.contains("sub_status 'active' is not valid for status 'backlog'"));
        same_task(task.0);

        // -- unknown status (spacetime-seed.allium: StoreRefusesUnknownStatus) --
        let unknown_task_status = bindings::TaskPatch {
            status: Some("archived".into()),
            ..blank_task_patch()
        };
        let why = refused(
            real.patch_task(task, unknown_task_status.clone())
                .await
                .unwrap(),
            mem.patch_task(task, unknown_task_status).await.unwrap(),
        );
        assert!(why.contains("unknown status \"archived\""), "{why}");
        same_task(task.0);
        let unknown_epic_status = bindings::EpicPatch {
            status: Some("archived".into()),
            ..blank_epic_patch()
        };
        let why = refused(
            real.patch_epic(EpicId(root), unknown_epic_status.clone())
                .await
                .unwrap(),
            mem.patch_epic(EpicId(root), unknown_epic_status)
                .await
                .unwrap(),
        );
        assert!(why.contains("unknown status \"archived\""), "{why}");
        same_epic(root);

        // -- reparent checks -----------------------------------------------------
        let into_own_sub_epic = bindings::EpicPatch {
            parent_epic_id: Some(sub),
            ..blank_epic_patch()
        };
        let why = refused(
            real.patch_epic(EpicId(root), into_own_sub_epic.clone())
                .await
                .unwrap(),
            mem.patch_epic(EpicId(root), into_own_sub_epic)
                .await
                .unwrap(),
        );
        assert!(why.contains("cycle"), "{why}");
        let detach = bindings::EpicPatch {
            parent_epic_id: Some(0),
            ..blank_epic_patch()
        };
        let why = refused(
            real.patch_epic(EpicId(sub), detach.clone()).await.unwrap(),
            mem.patch_epic(EpicId(sub), detach).await.unwrap(),
        );
        assert!(why.contains("Cannot reparent an auto-created repo-group sub-epic"));
        same_epic(root);
        same_epic(sub);

        // -- a repo change re-routes the task and removes the emptied sub-epic ---
        let to_beta = bindings::TaskPatch {
            repo_path: Some("/x/beta".into()),
            ..blank_task_patch()
        };
        real.patch_task(task, to_beta.clone()).await.unwrap();
        wait_for!(rows_real.epic(EpicId(sub)).is_none());
        mem.patch_task(task, to_beta).await.unwrap();
        assert!(rows_mem.epic(EpicId(sub)).is_none());
        same_task(task.0);
        let beta = rows_real.task(task).unwrap().epic_id.unwrap().0;
        wait_for!(rows_real.epic(EpicId(beta)).is_some());
        same_epic(beta);
        same_epic(root);

        // -- flatten, then regroup -----------------------------------------------
        let ungroup = bindings::EpicPatch {
            group_by_repo: Some(false),
            ..blank_epic_patch()
        };
        real.patch_epic(EpicId(root), ungroup.clone())
            .await
            .unwrap();
        mem.patch_epic(EpicId(root), ungroup).await.unwrap();
        real.flatten_epic(EpicId(root))
            .await
            .unwrap()
            .applied()
            .unwrap();
        wait_for!(rows_real.epic(EpicId(beta)).is_none());
        mem.flatten_epic(EpicId(root))
            .await
            .unwrap()
            .applied()
            .unwrap();
        assert!(rows_mem.epic(EpicId(beta)).is_none());
        same_task(task.0);
        same_epic(root);

        let regroup = bindings::EpicPatch {
            group_by_repo: Some(true),
            ..blank_epic_patch()
        };
        real.patch_epic(EpicId(root), regroup.clone())
            .await
            .unwrap();
        mem.patch_epic(EpicId(root), regroup).await.unwrap();
        real.regroup_epic(EpicId(root))
            .await
            .unwrap()
            .applied()
            .unwrap();
        wait_for!(rows_real
            .task(task)
            .is_some_and(|t| t.epic_id != Some(EpicId(root))));
        mem.regroup_epic(EpicId(root))
            .await
            .unwrap()
            .applied()
            .unwrap();
        same_task(task.0);
        let regrouped = rows_real.task(task).unwrap().epic_id.unwrap().0;
        wait_for!(rows_real.epic(EpicId(regrouped)).is_some());
        same_epic(regrouped);
        same_epic(root);

        // -- epic_id moves through set_task_epic, never patch_task --------------
        let by_patch = bindings::TaskPatch {
            epic_id: Some(root),
            ..blank_task_patch()
        };
        let why = refused(
            real.patch_task(task, by_patch.clone()).await.unwrap(),
            mem.patch_task(task, by_patch).await.unwrap(),
        );
        assert!(why.contains("set_task_epic"), "{why}");
        same_task(task.0);

        // -- the batch skips an entry the status does not allow ------------------
        let batch = vec![(task, dispatch_tui::models::SubStatus::Approved)];
        real.batch_patch_sub_status(batch.clone())
            .await
            .unwrap()
            .applied()
            .unwrap();
        mem.batch_patch_sub_status(batch)
            .await
            .unwrap()
            .applied()
            .unwrap();
        same_task(task.0);
        assert_eq!(
            rows_real.task(task).unwrap().sub_status,
            SubStatus::None,
            "backlog never holds approved"
        );

        // -- set_task_epic names an epic that does not exist ---------------------
        let missing = EpicId(9_999_999);
        let why = refused(
            real.set_task_epic(task, Some(missing), String::new())
                .await
                .unwrap(),
            mem.set_task_epic(task, Some(missing), String::new())
                .await
                .unwrap(),
        );
        assert!(why.contains("does not exist"), "{why}");
        same_task(task.0);

        // -- update_task: the relink and the patch are one transaction -----------
        let plain = bindings::Epic {
            created_by: identity.clone(),
            ..blank_epic()
        };
        let other = real.create_epic(plain.clone()).await.unwrap().0;
        wait_for!(rows_real.epic(EpicId(other)).is_some());
        mem.align_next_epic_id(other);
        assert_eq!(mem.create_epic(plain).await.unwrap().0, other);
        let before = rows_real.task(task).unwrap();
        let illegal_with_repo = bindings::TaskPatch {
            repo_path: Some("/x/elsewhere".into()),
            sub_status: Some("active".into()),
            ..blank_task_patch()
        };
        let why = refused(
            real.update_task(
                task,
                Some(Some(EpicId(other))),
                String::new(),
                illegal_with_repo.clone(),
            )
            .await
            .unwrap(),
            mem.update_task(
                task,
                Some(Some(EpicId(other))),
                String::new(),
                illegal_with_repo,
            )
            .await
            .unwrap(),
        );
        assert!(why.contains("is not valid for status"), "{why}");
        same_task(task.0);
        let after = rows_real.task(task).unwrap();
        assert_eq!(after.epic_id, before.epic_id, "the task did not move");
        assert_eq!(after.repo_path, before.repo_path, "the repo did not change");
        same_epic(other);

        let retitle = bindings::TaskPatch {
            title: Some("never written".into()),
            ..blank_task_patch()
        };
        let why = refused(
            real.update_task(task, Some(Some(missing)), String::new(), retitle.clone())
                .await
                .unwrap(),
            mem.update_task(task, Some(Some(missing)), String::new(), retitle)
                .await
                .unwrap(),
        );
        assert!(why.contains("does not exist"), "{why}");
        same_task(task.0);

        let gone = TaskId(9_999_999);
        let why = refused(
            real.update_task(gone, None, String::new(), blank_task_patch())
                .await
                .unwrap(),
            mem.update_task(gone, None, String::new(), blank_task_patch())
                .await
                .unwrap(),
        );
        assert!(why.contains("not found"), "{why}");

        // -- Done is stamped by the store, whatever the patch names ---------------
        let finishing = |title: &str| bindings::Task {
            title: title.into(),
            created_by: identity.clone(),
            ..blank_task_in_epic(other)
        };
        let done_real = real.create_task(finishing("finishing")).await.unwrap();
        wait_for!(rows_real.task(done_real).is_some());
        mem.align_next_task_id(done_real.0);
        let done_mem = mem.create_task(finishing("finishing")).await.unwrap();
        let stale = "2001-01-01 00:00:00.000";
        let to_done = bindings::TaskPatch {
            status: Some("done".into()),
            completed_at: Some(stale.into()),
            ..blank_task_patch()
        };
        real.update_task(done_real, None, String::new(), to_done.clone())
            .await
            .unwrap()
            .applied()
            .unwrap();
        mem.update_task(done_mem, None, String::new(), to_done)
            .await
            .unwrap()
            .applied()
            .unwrap();
        wait_for!(rows_real
            .task(done_real)
            .is_some_and(|t| t.status == TaskStatus::Done));
        for (side, rows, id) in [
            ("real", &rows_real, done_real),
            ("memory", &rows_mem, done_mem),
        ] {
            let stamped = rows.task(id).unwrap().completed_at;
            assert!(
                stamped.is_some_and(|at| at.timestamp() > 1_577_836_800),
                "{side} store kept the stale completion: {stamped:?}"
            );
        }

        // -- respawn_phoenix_successor on a task that is not a phoenix -----------
        let successor = bindings::Task {
            title: "never respawned".into(),
            ..blank_task_in_epic(root)
        };
        let real_why = real
            .respawn_phoenix_successor(task, successor.clone())
            .await
            .unwrap_err()
            .to_string();
        let why = mem
            .respawn_phoenix_successor(task, successor)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            real_why.contains(&why) || why.contains(&real_why),
            "real {real_why:?} vs memory {why:?}"
        );
        assert!(why.contains("is not a phoenix task"), "{why}");
        same_task(task.0);

        // -- a subtree nested past the depth cap is refused, not half-deleted ----
        let owned_epic = |parent_epic_id: i64| bindings::Epic {
            parent_epic_id,
            created_by: identity.clone(),
            ..blank_epic()
        };
        let deep_root = real.create_epic(owned_epic(0)).await.unwrap().0;
        mem.align_next_epic_id(deep_root);
        assert_eq!(mem.create_epic(owned_epic(0)).await.unwrap().0, deep_root);
        let mut parent = deep_root;
        for _ in 0..=MODULE_MAX_EPIC_DEPTH {
            let child = owned_epic(parent);
            wait_for!(rows_real.epic(EpicId(parent)).is_some());
            let id = real.create_epic(child.clone()).await.unwrap().0;
            mem.align_next_epic_id(id);
            assert_eq!(mem.create_epic(child).await.unwrap().0, id);
            parent = id;
        }
        wait_for!(rows_real.epic(EpicId(parent)).is_some());
        let why = refused(
            real.delete_epic(EpicId(deep_root)).await.unwrap(),
            mem.delete_epic(EpicId(deep_root)).await.unwrap(),
        );
        assert!(why.contains("levels deep"), "{why}");
        let why = refused(
            real.batch_delete(vec![], vec![EpicId(deep_root)])
                .await
                .unwrap(),
            mem.batch_delete(vec![], vec![EpicId(deep_root)])
                .await
                .unwrap(),
        );
        assert!(why.contains("levels deep"), "{why}");
        same_epic(deep_root);
        same_epic(parent);
    });
}
