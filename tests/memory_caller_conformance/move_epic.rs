use super::*;

/// `workspaces.allium: MoveEpicToWorkspace`: the real reducer and the twin move
/// a root epic and its tree the same way, and refuse the same cases.
#[test]
fn memory_caller_moves_an_epic_like_the_real_reducer() {
    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start("memory-caller-move-epic");
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

        for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
            caller
                .create_workspace("Other".into())
                .await
                .unwrap()
                .applied()
                .unwrap();
        }
        wait_for!(rows_real.workspaces().iter().any(|w| w.name == "Other"));
        let other = rows_real
            .workspaces()
            .iter()
            .find(|w| w.name == "Other")
            .map(|w| w.id)
            .expect("the new workspace");
        assert!(rows_mem.workspaces().iter().any(|w| w.id == other));

        let root = bindings::Epic {
            workspace_id: DEFAULT_WORKSPACE,
            created_by: identity.clone(),
            ..blank_epic()
        };
        let root_id = real.create_epic(root.clone()).await.unwrap().0;
        wait_for!(rows_real.epic(EpicId(root_id)).is_some());
        mem.align_next_epic_id(root_id);
        assert_eq!(mem.create_epic(root).await.unwrap().0, root_id);
        let sub = bindings::Epic {
            parent_epic_id: root_id,
            created_by: identity.clone(),
            ..blank_epic()
        };
        let sub_id = real.create_epic(sub.clone()).await.unwrap().0;
        wait_for!(rows_real.epic(EpicId(sub_id)).is_some());
        mem.align_next_epic_id(sub_id);
        assert_eq!(mem.create_epic(sub).await.unwrap().0, sub_id);

        // Refusals first: a sub-epic, the workspace it is already in, and a
        // workspace that does not exist. Both stores must say no.
        for (epic, to) in [
            (sub_id, other),
            (root_id, DEFAULT_WORKSPACE),
            (root_id, 999),
        ] {
            let real_out = real.move_epic_to_workspace(EpicId(epic), to).await.unwrap();
            let mem_out = mem.move_epic_to_workspace(EpicId(epic), to).await.unwrap();
            assert!(
                matches!(real_out, ReducerOutcome::Refused(_)),
                "real store: move {epic} to {to}"
            );
            assert!(
                matches!(mem_out, ReducerOutcome::Refused(_)),
                "twin: move {epic} to {to}"
            );
        }

        real.move_epic_to_workspace(EpicId(root_id), other)
            .await
            .unwrap()
            .applied()
            .unwrap();
        mem.move_epic_to_workspace(EpicId(root_id), other)
            .await
            .unwrap()
            .applied()
            .unwrap();
        wait_for!(rows_real
            .epic(EpicId(sub_id))
            .is_some_and(|e| e.workspace_id == other));
        for id in [root_id, sub_id] {
            let workspace_of = |rows: &SharedRows| rows.epic(EpicId(id)).map(|e| e.workspace_id);
            assert_eq!(workspace_of(&rows_real), Some(other), "real epic {id}");
            assert_eq!(workspace_of(&rows_mem), Some(other), "twin epic {id}");
        }
    });
}
