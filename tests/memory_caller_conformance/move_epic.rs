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

        // A poll claim held for a person who is not in the destination is
        // released by the move (task #98746, V6), on both sides.
        for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
            caller
                .register_host("host-a".into(), String::new(), identity.clone())
                .await
                .unwrap()
                .applied()
                .unwrap();
            caller
                .claim_poll_owner(PollScopeId::Epic(EpicId(root_id)), "host-a".into())
                .await
                .unwrap()
                .applied()
                .unwrap();
        }
        wait_for!(rows_real
            .poll_owner(PollScopeId::Epic(EpicId(root_id)))
            .is_some());
        assert!(rows_mem
            .poll_owner(PollScopeId::Epic(EpicId(root_id)))
            .is_some());

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
        // A super-admin is a member of no workspace by being one, so the move
        // released the claim (the holder is not in `Other`) and took the tree
        // off this board: the views serve epics only in a workspace the caller
        // belongs to. Joining `Other` now brings the tree back, and with it any
        // claim the move had kept, in the same transaction.
        for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
            caller
                .add_member(other, identity.clone(), "member".into())
                .await
                .unwrap()
                .applied()
                .unwrap();
        }
        wait_for!(rows_real
            .epic(EpicId(sub_id))
            .is_some_and(|e| e.workspace_id == other));
        for id in [root_id, sub_id] {
            let workspace_of = |rows: &SharedRows| rows.epic(EpicId(id)).map(|e| e.workspace_id);
            assert_eq!(workspace_of(&rows_real), Some(other), "real epic {id}");
            assert_eq!(workspace_of(&rows_mem), Some(other), "twin epic {id}");
        }
        wait_for!(rows_real
            .poll_owner(PollScopeId::Epic(EpicId(root_id)))
            .is_none());
        assert!(
            rows_mem
                .poll_owner(PollScopeId::Epic(EpicId(root_id)))
                .is_none(),
            "twin released the claim"
        );

        // `update_epic` is all-or-nothing, and each part is judged against the
        // epic as the parts before it left it (task #98852, M6).
        let mut make_epic = |parent_epic_id: i64| {
            let epic = bindings::Epic {
                workspace_id: DEFAULT_WORKSPACE,
                parent_epic_id,
                created_by: identity.clone(),
                ..blank_epic()
            };
            async move { epic }
        };
        let mut ids = Vec::new();
        for parent in [0, 0] {
            let epic = make_epic(parent).await;
            let id = real.create_epic(epic.clone()).await.unwrap().0;
            wait_for!(rows_real.epic(EpicId(id)).is_some());
            mem.align_next_epic_id(id);
            assert_eq!(mem.create_epic(epic).await.unwrap().0, id);
            ids.push(id);
        }
        let (loose, parent) = (ids[0], ids[1]);
        let child = make_epic(parent).await;
        let child_id = real.create_epic(child.clone()).await.unwrap().0;
        wait_for!(rows_real.epic(EpicId(child_id)).is_some());
        mem.align_next_epic_id(child_id);
        assert_eq!(mem.create_epic(child).await.unwrap().0, child_id);

        // A root epic given a parent AND a workspace: the move refuses the
        // sub-epic, so the parent must not be written either.
        let adopt = || bindings::EpicPatch {
            parent_epic_id: Some(parent),
            ..blank_epic_patch()
        };
        let real_out = real
            .update_epic(EpicId(loose), adopt(), Some(other), None)
            .await
            .unwrap();
        let mem_out = mem
            .update_epic(EpicId(loose), adopt(), Some(other), None)
            .await
            .unwrap();
        assert!(
            matches!(real_out, ReducerOutcome::Refused(_)),
            "{real_out:?}"
        );
        assert!(matches!(mem_out, ReducerOutcome::Refused(_)), "{mem_out:?}");
        assert_eq!(rows_real.epic(EpicId(loose)).unwrap().parent_epic_id, None);
        assert_eq!(rows_mem.epic(EpicId(loose)).unwrap().parent_epic_id, None);

        // A sub-epic made a root AND moved: the patch runs first, so the move
        // sees a root and both succeed.
        let detach = || bindings::EpicPatch {
            parent_epic_id: Some(0),
            ..blank_epic_patch()
        };
        real.update_epic(EpicId(child_id), detach(), Some(other), None)
            .await
            .unwrap()
            .applied()
            .unwrap();
        mem.update_epic(EpicId(child_id), detach(), Some(other), None)
            .await
            .unwrap()
            .applied()
            .unwrap();
        wait_for!(rows_real
            .epic(EpicId(child_id))
            .is_some_and(|e| e.workspace_id == other));
        for rows in [&rows_real, &rows_mem] {
            let e = rows.epic(EpicId(child_id)).unwrap();
            assert_eq!((e.parent_epic_id, e.workspace_id), (None, other));
        }

        // A task never leaves its workspace for another workspace's epic
        // (`TaskMovesStayInTheWorkspace`).
        let stay = bindings::Task {
            created_by: identity.clone(),
            ..blank_task_in_epic(loose)
        };
        let task = real.create_task(stay.clone()).await.unwrap();
        wait_for!(rows_real.task(task).is_some());
        mem.align_next_task_id(task.0);
        assert_eq!(mem.create_task(stay).await.unwrap(), task);
        let real_out = real
            .set_task_epic(task, Some(EpicId(child_id)), String::new())
            .await
            .unwrap();
        let mem_out = mem
            .set_task_epic(task, Some(EpicId(child_id)), String::new())
            .await
            .unwrap();
        for out in [real_out, mem_out] {
            match out {
                ReducerOutcome::Refused(why) => {
                    assert!(why.contains("stays in its workspace"), "{why}")
                }
                other => panic!("the move must be refused, got {other:?}"),
            }
        }
        for rows in [&rows_real, &rows_mem] {
            assert_eq!(rows.task(task).unwrap().epic_id, Some(EpicId(loose)));
        }
    });
}
