use super::*;

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
            .with_sender(accepted.identity.clone());
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                vec![],
                "host-a",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));
        let real = SdkReducerCaller::new(connector, Arc::new(SettledIdentity::default()));
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
        assert_eq!(mem.create_epic(grouped).await.unwrap().0, root);

        let in_root = |repo: &str| bindings::Task {
            repo_path: repo.into(),
            created_by: identity.clone(),
            ..blank_task_in_epic(root)
        };
        let task = real.create_task(in_root("/x/alpha")).await.unwrap();
        wait_for!(rows_real.task(task).is_some());
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
    });
}
