use super::*;

/// `store-split.allium: IdentitiesAreTheCallers` (task #98755): the real
/// reducers and the twin stamp the caller as owner of a task that is created
/// with no epic or detached from its epic, whatever owner the argument names,
/// and `unsubscribe_from_epic` refuses another person's subscriber. A new
/// learning's author is the caller too, whatever the row names.
///
/// The board never reads `owner` back, but the per-caller views deliver a user
/// board task only to its owner. So a task the real store delivers back was
/// stamped with the caller, and the twin must hold the same task.
#[test]
fn memory_caller_stamps_the_caller_and_guards_unsubscribe() {
    let instance = Instance::start("memory-caller-identity-stamping");
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
            .with_identity_rules();
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                "host-a",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));
        let real = SdkReducerCaller::new(connector, Arc::new(SettledIdentity::default()));
        register_profiles(&real, &mem, &accepted.identity).await;
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

        // -- an epic-less create lands on the caller's board -----------------------
        // (A forged owner cannot be read back by id: the SDK caller matches on
        // it, as an honest board sends its own. The module test forges it.)
        let loose = bindings::Task {
            owner: identity.clone(),
            created_by: identity.clone(),
            ..blank_task_in_epic(0)
        };
        let t = real.create_task(loose.clone()).await.unwrap();
        // Delivered only if `owner` is the caller (the `own_tasks` view).
        wait_for!(rows_real.task(t).is_some());
        mem.align_next_task_id(t.0);
        assert_eq!(mem.create_task(loose).await.unwrap(), t);
        assert!(
            rows_mem.task(t).is_some(),
            "twin must hold the stamped task"
        );

        // -- detach: the creator may, and a patch that names an owner is refused -------------
        let epic = bindings::Epic {
            created_by: identity.clone(),
            ..blank_epic()
        };
        let e = real.create_epic(epic.clone()).await.unwrap().0;
        wait_for!(rows_real.epic(EpicId(e)).is_some());
        mem.align_next_epic_id(e);
        assert_eq!(mem.create_epic(epic).await.unwrap().0, e);
        let inside = bindings::Task {
            created_by: identity.clone(),
            ..blank_task_in_epic(e)
        };
        let d = real.create_task(inside.clone()).await.unwrap();
        wait_for!(rows_real.task(d).is_some());
        mem.align_next_task_id(d.0);
        assert_eq!(mem.create_task(inside).await.unwrap(), d);
        // Anyone may detach a task onto their own board (R7).
        let applied = |out: ReducerOutcome| matches!(out, ReducerOutcome::Applied(_));
        assert!(applied(
            real.set_task_epic(d, None, "someone-else".into())
                .await
                .unwrap()
        ));
        assert!(applied(
            mem.set_task_epic(d, None, "someone-else".into())
                .await
                .unwrap()
        ));
        wait_for!(rows_real.task(d).is_some_and(|t| t.epic_id.is_none()));
        assert_eq!(
            rows_real.task(d).map(|t| t.epic_id),
            rows_mem.task(d).map(|t| t.epic_id)
        );

        // The store owns `owner`: a patch that names one is refused.
        let name_owner = bindings::TaskPatch {
            owner: Some("someone-else".into()),
            ..blank_task_patch()
        };
        let why = refused(
            real.patch_task(d, name_owner.clone()).await.unwrap(),
            mem.patch_task(d, name_owner).await.unwrap(),
        );
        assert!(why.contains("owner"), "{why}");

        // -- unsubscribe: another person's subscriber is refused ----------------
        let why = refused(
            real.unsubscribe_from_epic("someone-else".into(), EpicId(e))
                .await
                .unwrap(),
            mem.unsubscribe_from_epic("someone-else".into(), EpicId(e))
                .await
                .unwrap(),
        );
        assert!(why.contains("caller") || why.contains("identity"), "{why}");

        // -- unsubscribe: an epic that does not exist reads as missing ----------
        let why = refused(
            real.unsubscribe_from_epic(identity.clone(), EpicId(999_999))
                .await
                .unwrap(),
            mem.unsubscribe_from_epic(identity.clone(), EpicId(999_999))
                .await
                .unwrap(),
        );
        assert!(why.contains("no epic"), "{why}");

        // -- subscribe: another person's subscriber is refused ------------------
        let why = refused(
            real.subscribe_to_epic("someone-else".into(), EpicId(e))
                .await
                .unwrap(),
            mem.subscribe_to_epic("someone-else".into(), EpicId(e))
                .await
                .unwrap(),
        );
        assert!(why.contains("caller") || why.contains("identity"), "{why}");

        // -- subscribe: an epic that does not exist reads as missing ------------
        let why = refused(
            real.subscribe_to_epic(identity.clone(), EpicId(999_999))
                .await
                .unwrap(),
            mem.subscribe_to_epic(identity.clone(), EpicId(999_999))
                .await
                .unwrap(),
        );
        assert!(why.contains("no epic"), "{why}");

        // -- a learning's author is the caller, whatever the row names --------
        // (`store-split.allium: IdentitiesAreTheCallers`.) One row per scope,
        // each naming another author and a dead workspace; both stores keep
        // the same row, stamped with the caller.
        let sent =
            |scope: &str, scope_ref: Option<String>, source: Option<i64>| bindings::Learning {
                scope: scope.into(),
                scope_ref,
                source_task_id: source,
                workspace_id: 999,
                author: "someone-else".into(),
                ..blank_learning()
            };
        for row in [
            sent("user", None, None),
            sent("epic", Some(e.to_string()), None),
            sent("repo", Some("/repo".into()), Some(d.0)),
            sent("repo", Some("/repo".into()), None),
        ] {
            let real_id = real.create_learning(row.clone()).await.unwrap();
            wait_for!(rows_real.learning(real_id).is_some());
            mem.align_next_learning_id(real_id.0);
            let mem_id = mem.create_learning(row).await.unwrap();
            assert_eq!(
                rows_real.learning(real_id).map(|l| learning_shape(&l)),
                rows_mem.learning(mem_id).map(|l| learning_shape(&l)),
                "learning {real_id:?}"
            );
            for (side, rows, id) in [("real", &rows_real, real_id), ("twin", &rows_mem, mem_id)] {
                let stamped = rows.learning(id).expect("the learning");
                assert_eq!(stamped.author, identity, "{side}: {real_id:?}");
            }
        }
    });
}
