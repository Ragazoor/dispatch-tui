use super::*;

/// `workspaces.allium: LearningVisibility`: the real reducer and the twin stamp
/// the same author and workspace on a new learning, whatever the caller sent.
#[test]
fn memory_caller_places_a_learning_like_the_real_reducer() {
    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start("memory-caller-learning-placement");
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

        let epic = bindings::Epic {
            created_by: identity.clone(),
            ..blank_epic()
        };
        let epic_id = real.create_epic(epic.clone()).await.unwrap().0;
        wait_for!(rows_real.epic(EpicId(epic_id)).is_some());
        mem.align_next_epic_id(epic_id);
        assert_eq!(mem.create_epic(epic).await.unwrap().0, epic_id);

        let task = bindings::Task {
            created_by: identity.clone(),
            ..blank_task_in_epic(epic_id)
        };
        let task_id = real.create_task(task.clone()).await.unwrap();
        wait_for!(rows_real.task(task_id).is_some());
        mem.align_next_task_id(task_id.0);
        assert_eq!(mem.create_task(task).await.unwrap(), task_id);

        // Each row names a workspace and an author the store must overwrite.
        let sent =
            |scope: &str, scope_ref: Option<String>, source: Option<i64>| bindings::Learning {
                scope: scope.into(),
                scope_ref,
                source_task_id: source,
                workspace_id: 999,
                author: "someone-else".into(),
                ..blank_learning()
            };
        let rows = [
            sent("user", None, None),
            sent("epic", Some(epic_id.to_string()), None),
            sent("repo", Some("/repo".into()), Some(task_id.0)),
            sent("repo", Some("/repo".into()), None),
        ];
        for row in rows {
            let real_id = real.create_learning(row.clone()).await.unwrap();
            wait_for!(rows_real.learning(real_id).is_some());
            let mem_id = mem.create_learning(row).await.unwrap();
            assert_eq!(real_id, mem_id, "generated learning id");
            let (real_row, mem_row) = (
                rows_real.learning(real_id).map(|l| learning_shape(&l)),
                rows_mem.learning(mem_id).map(|l| learning_shape(&l)),
            );
            assert_eq!(real_row, mem_row, "learning {real_id:?}");
            let stamped = rows_real.learning(real_id).expect("the learning");
            assert_eq!(stamped.author, identity);
            assert_ne!(stamped.workspace_id, 999);
        }
    });
}
