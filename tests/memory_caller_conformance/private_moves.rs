use super::*;

/// `workspaces.allium: RepoGroupsStayPublic` and `epics.allium: FlattenEpic`
/// (private moves and repo groups, task #98744): the real reducers and the twin
/// refuse a private repo group and an origin change alike, and a flatten ends
/// the unseen mark of every task it relinks.
#[test]
fn memory_caller_keeps_repo_groups_public_and_clears_marks_on_relink() {
    let instance = Instance::start("memory-caller-private-moves");
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

        // -- a grouped root, a repo sub-epic, and a signalled task in it ------
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
        let sub = rows_real.task(task).unwrap().epic_id.unwrap().0;
        wait_for!(rows_real.epic(EpicId(sub)).is_some());
        same_epic(sub);

        // -- RepoGroupsStayPublic: no private repo group, no origin change ------
        let why = refused(
            real.set_epic_visibility(EpicId(sub), "private".into())
                .await
                .unwrap(),
            mem.set_epic_visibility(EpicId(sub), "private".into())
                .await
                .unwrap(),
        );
        assert!(why.contains("repo-group"), "{why}");
        let origin = bindings::EpicPatch {
            origin: Some(String::new()),
            ..blank_epic_patch()
        };
        refused(
            real.patch_epic(EpicId(sub), origin.clone()).await.unwrap(),
            mem.patch_epic(EpicId(sub), origin).await.unwrap(),
        );
        let hand_made = bindings::Epic {
            created_by: identity.clone(),
            ..blank_epic()
        };
        let plain = real.create_epic(hand_made.clone()).await.unwrap().0;
        wait_for!(rows_real.epic(EpicId(plain)).is_some());
        mem.align_next_epic_id(plain);
        assert_eq!(mem.create_epic(hand_made).await.unwrap().0, plain);
        let claim = bindings::EpicPatch {
            origin: Some("repo-group".into()),
            ..blank_epic_patch()
        };
        refused(
            real.patch_epic(EpicId(plain), claim.clone()).await.unwrap(),
            mem.patch_epic(EpicId(plain), claim).await.unwrap(),
        );
        same_epic(sub);
        same_epic(plain);

        // -- R2: a flatten relink ends the unseen mark --------------------------
        let signalled = bindings::Task {
            repo_path: "/x/alpha".into(),
            created_by: identity.clone(),
            ..blank_task_in_epic(sub)
        };
        let s = real.create_task_signalled(signalled.clone()).await.unwrap();
        wait_for!(rows_real.task(s).is_some() && !rows_real.unseen_task_ids().is_empty());
        mem.align_next_task_id(s.0);
        assert_eq!(mem.create_task_signalled(signalled).await.unwrap(), s);
        assert_eq!(rows_real.unseen_task_ids(), rows_mem.unseen_task_ids());
        real.flatten_epic(EpicId(root))
            .await
            .unwrap()
            .applied()
            .unwrap();
        mem.flatten_epic(EpicId(root))
            .await
            .unwrap()
            .applied()
            .unwrap();
        wait_for!(rows_real.unseen_task_ids().is_empty());
        assert!(rows_mem.unseen_task_ids().is_empty());
        assert_eq!(
            rows_real.task(s).map(|t| t.epic_id),
            rows_mem.task(s).map(|t| t.epic_id)
        );
    });
}
