use super::*;

/// One feed item as the board posts it (`encode::feed_ingest_item`'s shape),
/// sent to BOTH callers.
fn ingest_item(external_id: &str, url: &str, signals: &[&str]) -> bindings::FeedIngestItem {
    bindings::FeedIngestItem {
        item: bindings::FeedTaskUpsertItem {
            external_id: external_id.into(),
            title: format!("title {external_id}"),
            description: String::new(),
            repo_path: String::new(),
            status: "backlog".into(),
            sub_status: "none".into(),
            base_branch: "main".into(),
            tag: "pr-review".into(),
            labels: "[]".into(),
            sort_order: None,
            url: url.into(),
            url_type: if url.is_empty() { "" } else { "pr" }.into(),
            wrap_up_mode: String::new(),
        },
        signals: signals.iter().map(|s| s.to_string()).collect(),
    }
}

/// Where every feed task sits: `(external_id, parent title, own title)`,
/// sorted, read from the board's rows. Ids are left out on purpose: the two
/// sides must agree about the layout, not about which integer each got.
fn feed_layout(rows: &SharedRows, root: EpicId) -> Vec<(String, String, String)> {
    let epics = rows.epics();
    let title_of = |id: EpicId| {
        epics
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.title.clone())
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    for epic in &epics {
        for task in rows.tasks_for_epic(epic.id) {
            if let Some(ext) = task.external_id {
                let parent = epic.parent_epic_id.map(title_of).unwrap_or_default();
                out.push((ext, parent, epic.title.clone()));
            }
        }
    }
    let _ = root;
    out.sort();
    out
}

/// `feeds.allium: IngestFeedEmission`. The in-memory caller ingests an
/// emission the way the real module does: roles, repo groups, moves, the
/// stale delete, additive mode and the refusals.
#[test]
fn memory_caller_ingests_a_feed_like_the_real_module() {
    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start("memory-caller-ingest");
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
        for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
            caller
                .register_host("host-a".into(), String::new(), identity.clone())
                .await
                .unwrap()
                .applied()
                .unwrap();
        }

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

        let parent = bindings::Epic {
            title: "Reviews".into(),
            feed_role: "reviews-parent".into(),
            feed_command: "gh-reviews".into(),
            feed_interval_secs: 60,
            created_by: identity.clone(),
            ..blank_epic()
        };
        let root = real.create_epic(parent.clone()).await.unwrap().0;
        wait_for!(rows_real.epic(EpicId(root)).is_some());
        assert_eq!(mem.create_epic(parent).await.unwrap().0, root);

        // Nobody follows the reviews parent: a new review request is marked
        // for everyone who sees it (`feeds.allium: NewReviewRequestIsSignalled`).
        // Task ids are not compared across the two stores: each numbers the
        // role groups' inserts in its own order. The PR names a task in both.
        let unseen_prs = |rows: &SharedRows| {
            let unseen = rows.unseen_task_ids();
            let mut prs: Vec<String> = rows
                .tasks()
                .into_iter()
                .filter(|t| unseen.contains(&t.id))
                .filter_map(|t| t.external_id)
                .collect();
            prs.sort();
            prs
        };

        let pr = |n: u32| format!("https://github.com/org/repo-{n}/pull/{n}");
        let ingest = |items: Vec<bindings::FeedIngestItem>, additive: bool| {
            let real = &real;
            let mem = &mem;
            let identity = identity.clone();
            async move {
                let r = real
                    .ingest_feed(
                        EpicId(root),
                        items.clone(),
                        additive,
                        identity.clone(),
                        "host-a".into(),
                    )
                    .await
                    .unwrap();
                let m = mem
                    .ingest_feed(EpicId(root), items, additive, identity, "host-a".into())
                    .await
                    .unwrap();
                (r, m)
            }
        };
        let same_layout = |expected: usize| {
            let want = feed_layout(&rows_mem, EpicId(root));
            assert_eq!(want.len(), expected, "{want:?}");
            want
        };

        // -- cycle 1: routing, role sub-epics, an excluded PR ---------------------
        let (r, m) = ingest(
            vec![
                ingest_item("pr-1", &pr(1), &["direct-request"]),
                ingest_item("pr-2", &pr(2), &["team-request"]),
                ingest_item("pr-3", &pr(3), &["author-bot"]),
                ingest_item("pr-4", &pr(4), &["author-me"]),
            ],
            false,
        )
        .await;
        r.applied().unwrap();
        m.applied().unwrap();
        let want = same_layout(3);
        wait_for!(feed_layout(&rows_real, EpicId(root)) == want);
        assert_eq!(
            want,
            vec![
                ("pr-1".into(), "Reviews".into(), "My Reviews".into()),
                ("pr-2".into(), "Reviews".into(), "Team Reviews".into()),
                ("pr-3".into(), "Reviews".into(), "Bots".into()),
            ]
        );
        // The new My Reviews and Team Reviews PRs are signalled; Bots is not.
        assert_eq!(
            unseen_prs(&rows_mem),
            vec!["pr-1".to_string(), "pr-2".to_string()]
        );
        wait_for!(unseen_prs(&rows_real) == unseen_prs(&rows_mem));

        // -- cycle 2: pr-2 is reviewed (a move), pr-3 is gone (a delete) ----------
        let (r, m) = ingest(
            vec![
                ingest_item("pr-1", &pr(1), &["direct-request"]),
                ingest_item("pr-2", &pr(2), &["team-request", "reviewed"]),
            ],
            false,
        )
        .await;
        r.applied().unwrap();
        m.applied().unwrap();
        let want = same_layout(2);
        wait_for!(feed_layout(&rows_real, EpicId(root)) == want);
        // A move into My Reviews is not a new task: pr-2's Team Reviews mark
        // ends with the move, and the move writes none.
        assert_eq!(unseen_prs(&rows_mem), vec!["pr-1".to_string()]);
        assert_eq!(unseen_prs(&rows_real), unseen_prs(&rows_mem));

        // -- cycle 2b: pr-1 leaves My Reviews, so its mark ends -------------------
        // (`tasks.allium: UnseenMarkEndsWithTheTaskLeavingItsEpic`).
        let (r, m) = ingest(
            vec![
                ingest_item("pr-1", &pr(1), &["team-request"]),
                ingest_item("pr-2", &pr(2), &["team-request", "reviewed"]),
            ],
            false,
        )
        .await;
        r.applied().unwrap();
        m.applied().unwrap();
        let want = same_layout(2);
        wait_for!(feed_layout(&rows_real, EpicId(root)) == want);
        assert_eq!(unseen_prs(&rows_mem), Vec::<String>::new());
        wait_for!(unseen_prs(&rows_real).is_empty());

        // -- cycle 3: additive removes nothing ------------------------------------
        let (r, m) = ingest(vec![ingest_item("pr-5", &pr(5), &["team-request"])], true).await;
        r.applied().unwrap();
        m.applied().unwrap();
        let want = same_layout(3);
        wait_for!(feed_layout(&rows_real, EpicId(root)) == want);
        // The additive path signals its inserts the same way.
        assert_eq!(unseen_prs(&rows_mem), vec!["pr-5".to_string()]);
        wait_for!(unseen_prs(&rows_real) == unseen_prs(&rows_mem));

        // -- refusals --------------------------------------------------------------
        let (r, m) = ingest(vec![], false).await;
        r.applied().unwrap();
        m.applied().unwrap();
        let role = rows_real
            .epics()
            .into_iter()
            .find(|e| e.feed_role == dispatch_tui::models::FeedRole::Bots)
            .expect("the Bots sub-epic exists")
            .id;
        let on_role = real
            .ingest_feed(role, vec![], false, identity.clone(), "host-a".into())
            .await
            .unwrap();
        let on_role_mem = mem
            .ingest_feed(role, vec![], false, identity.clone(), "host-a".into())
            .await
            .unwrap();
        match (on_role, on_role_mem) {
            (ReducerOutcome::Refused(r), ReducerOutcome::Refused(m)) => {
                assert!(
                    r.contains(&m) || m.contains(&r),
                    "real {r:?} vs memory {m:?}"
                );
                assert!(m.contains("reconciled only via its reviews parent"), "{m}");
            }
            other => panic!("both sides must refuse a role sub-epic, got {other:?}"),
        }
        let missing = real
            .ingest_feed(
                EpicId(9999),
                vec![],
                false,
                identity.clone(),
                "host-a".into(),
            )
            .await
            .unwrap();
        let missing_mem = mem
            .ingest_feed(
                EpicId(9999),
                vec![],
                false,
                identity.clone(),
                "host-a".into(),
            )
            .await
            .unwrap();
        assert!(matches!(missing, ReducerOutcome::Refused(_)), "{missing:?}");
        assert!(
            matches!(missing_mem, ReducerOutcome::Refused(_)),
            "{missing_mem:?}"
        );

        // -- feed requests (`store-split.allium: FeedRequest`) -----------------------
        // Both sides must answer every step alike. host-a is this board's host
        // (its rows arrive on the subscription); host-b is a second host of the
        // same person that asks host-a, the poll owner, to refresh.
        for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
            caller
                .register_host("host-b".into(), String::new(), identity.clone())
                .await
                .unwrap()
                .applied()
                .unwrap();
        }
        let root_id = EpicId(root);
        macro_rules! both {
            ($call:ident($($arg:expr),*)) => {{
                let r = real.$call($($arg.clone()),*).await.unwrap();
                let m = mem.$call($($arg.clone()),*).await.unwrap();
                assert_eq!(r.won(), m.won(), "{}: real {r:?} vs memory {m:?}", stringify!($call));
                r.won()
            }};
        }
        let host_a = "host-a".to_string();
        let host_b = "host-b".to_string();
        // No poll owner yet: refused on both.
        assert!(!both!(request_feed_refresh(root_id, host_b)));
        let a = PollScopeId::Epic(root_id);
        assert!(both!(claim_poll_owner(a, host_a)));
        // The owner may not ask itself; the other host may.
        assert!(!both!(request_feed_refresh(root_id, host_a)));
        assert!(both!(request_feed_refresh(root_id, host_b)));
        // Coalesced into the open request.
        assert!(both!(request_feed_refresh(root_id, host_b)));
        wait_for!(rows_real.feed_requests().len() == 1);
        assert_eq!(rows_mem.feed_requests().len(), 1);
        let real_id = rows_real.feed_requests()[0].id;
        let mem_id = rows_mem.feed_requests()[0].id;
        assert_eq!(real_id, mem_id, "feed request id");
        // Only the row's own host claims it; a report needs a running row.
        assert!(!both!(claim_feed_request(real_id, host_b)));
        assert!(!both!(report_feed_request_done(real_id, host_a)));
        assert!(both!(claim_feed_request(real_id, host_a)));
        assert!(both!(report_feed_request_done(real_id, host_a)));
        assert!(!both!(report_feed_request_done(real_id, host_a)));
        wait_for!(
            rows_real.feed_requests()[0].state == dispatch_tui::models::FeedRequestState::Done
        );
        assert_eq!(
            rows_mem.feed_requests()[0].state,
            dispatch_tui::models::FeedRequestState::Done
        );

        // -- review Q8: an ask after the first finished is a new row -------------------
        assert!(both!(request_feed_refresh(root_id, host_b)));
        wait_for!(rows_real.feed_requests().len() == 2);
        assert_eq!(rows_mem.feed_requests().len(), 2);
        let second = rows_real
            .feed_requests()
            .into_iter()
            .map(|r| r.id)
            .max()
            .unwrap();
        // -- review Q4: a running row is not claimed again inside the grace ------------
        assert!(both!(claim_feed_request(second, host_a)));
        assert!(!both!(claim_feed_request(second, host_a)));
        // -- review Q8: an ask while the owner runs the second is a third row ----------
        assert!(both!(request_feed_refresh(root_id, host_b)));
        wait_for!(rows_real.feed_requests().len() == 3);
        assert_eq!(rows_mem.feed_requests().len(), 3);

        // -- review Q9: an ingest from a host that does not hold the claim -------------
        let refused_real = real
            .ingest_feed(root_id, vec![], false, identity.clone(), host_b.clone())
            .await
            .unwrap();
        let refused_mem = mem
            .ingest_feed(root_id, vec![], false, identity.clone(), host_b.clone())
            .await
            .unwrap();
        match (refused_real, refused_mem) {
            (ReducerOutcome::Refused(r), ReducerOutcome::Refused(m)) => {
                assert_eq!(r, m);
                assert!(m.contains("claimed by host-a"), "{m}");
            }
            other => panic!("both sides must refuse a host without the claim, got {other:?}"),
        }

        // -- review Q1: a teardown row names the agent's window --------------------
        let (r, m) = ingest(vec![ingest_item("pr-9", &pr(9), &["team-request"])], false).await;
        r.applied().unwrap();
        m.applied().unwrap();
        let task_of = |rows: &SharedRows| {
            rows.tasks()
                .into_iter()
                .find(|t| t.external_id.as_deref() == Some("pr-9"))
                .map(|t| t.id)
        };
        wait_for!(task_of(&rows_real).is_some());
        let id9 = task_of(&rows_mem).unwrap();
        assert_eq!(task_of(&rows_real), Some(id9), "task id");
        // Hand the task to host-b (the caller's own), running; the window is the
        // current agent's, so back-fill the agent, then give it the window.
        for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
            caller
                .patch_task(
                    id9,
                    bindings::TaskPatch {
                        status: Some("running".into()),
                        sub_status: Some("active".into()),
                        worktree: Some("/repo/.worktrees/9-pr".into()),
                        host: Some("host-b".into()),
                        ..blank_task_patch()
                    },
                )
                .await
                .unwrap()
                .applied()
                .unwrap();
        }
        let backfilled = instance.call("backfill_agents", &[]);
        assert!(backfilled.status.success(), "{}", describe(&backfilled));
        mem.backfill_agents().unwrap().applied().unwrap();
        for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
            caller
                .patch_task(
                    id9,
                    bindings::TaskPatch {
                        tmux_window: Some("task-9".into()),
                        ..blank_task_patch()
                    },
                )
                .await
                .unwrap()
                .applied()
                .unwrap();
        }
        // The item leaves the feed on host-a's ingest: host-b owes a teardown.
        let (r, m) = ingest(vec![], false).await;
        r.applied().unwrap();
        m.applied().unwrap();
        // The row is for host-b, so host-a's subscription does not deliver it:
        // the real side is read with SQL, the twin from its rows.
        let t_mem = rows_mem
            .feed_requests()
            .into_iter()
            .find(|r| r.kind == dispatch_tui::models::FeedRequestKind::Teardown)
            .expect("the twin writes a teardown row");
        assert_eq!(t_mem.host, "host-b");
        assert_eq!(t_mem.worktree.as_deref(), Some("/repo/.worktrees/9-pr"));
        assert_eq!(
            t_mem.tmux_window.as_ref().map(|w| w.as_str()),
            Some("task-9")
        );
        let t_real = instance
            .sql("SELECT host, worktree, tmux_window FROM feed_requests WHERE kind = 'teardown'");
        for expected in ["host-b", "/repo/.worktrees/9-pr", "task-9"] {
            assert!(
                t_real.contains(expected),
                "the real module's teardown row must name {expected}: {t_real}"
            );
        }
    });
}

/// `feeds.allium: GroupedFeedUpsert`, migration by move. A task whose item
/// now names another repo moves between two PRESENT sub-epics, and the sub-epic
/// it leaves is upserted first. Every move runs before any group's stale
/// delete, so the task survives the move on the real module and the twin alike.
#[test]
fn grouped_ingest_moves_a_task_between_present_sub_epics() {
    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start("memory-caller-grouped-move");
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
        for caller in [&real as &dyn ReducerCaller, &mem as &dyn ReducerCaller] {
            caller
                .register_host("host-a".into(), String::new(), identity.clone())
                .await
                .unwrap()
                .applied()
                .unwrap();
        }

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

        let grouped = bindings::Epic {
            title: "Feed".into(),
            feed_command: "gh-feed".into(),
            feed_interval_secs: 60,
            group_by_repo: true,
            created_by: identity.clone(),
            ..blank_epic()
        };
        let root = real.create_epic(grouped.clone()).await.unwrap().0;
        wait_for!(rows_real.epic(EpicId(root)).is_some());
        assert_eq!(mem.create_epic(grouped).await.unwrap().0, root);

        let ingest = |items: Vec<bindings::FeedIngestItem>| {
            let real = &real;
            let mem = &mem;
            let identity = identity.clone();
            async move {
                let r = real
                    .ingest_feed(
                        EpicId(root),
                        items.clone(),
                        false,
                        identity.clone(),
                        "host-a".into(),
                    )
                    .await
                    .unwrap();
                let m = mem
                    .ingest_feed(EpicId(root), items, false, identity, "host-a".into())
                    .await
                    .unwrap();
                (r, m)
            }
        };

        // Two items with no URL: both land in "other".
        let (r, m) = ingest(vec![
            ingest_item("x-1", "", &[]),
            ingest_item("x-2", "", &[]),
        ])
        .await;
        r.applied().unwrap();
        m.applied().unwrap();
        let want = feed_layout(&rows_mem, EpicId(root));
        wait_for!(feed_layout(&rows_real, EpicId(root)) == want);
        let moving = rows_mem
            .tasks()
            .into_iter()
            .find(|t| t.external_id.as_deref() == Some("x-1"))
            .unwrap()
            .id;

        // x-1 now names repo-1; x-2 keeps "other" present, and first.
        let (r, m) = ingest(vec![
            ingest_item("x-2", "", &[]),
            ingest_item("x-1", "https://github.com/org/repo-1/pull/1", &[]),
        ])
        .await;
        r.applied().unwrap();
        m.applied().unwrap();
        let want = feed_layout(&rows_mem, EpicId(root));
        assert_eq!(
            want,
            vec![
                ("x-1".into(), "Feed".into(), "repo-1".into()),
                ("x-2".into(), "Feed".into(), "other".into()),
            ]
        );
        wait_for!(feed_layout(&rows_real, EpicId(root)) == want);
        assert!(
            rows_mem.tasks().iter().any(|t| t.id == moving),
            "a move keeps the row"
        );
    });
}
