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

        let pr = |n: u32| format!("https://github.com/org/repo-{n}/pull/{n}");
        let ingest = |items: Vec<bindings::FeedIngestItem>, additive: bool| {
            let real = &real;
            let mem = &mem;
            let identity = identity.clone();
            async move {
                let r = real
                    .ingest_feed(EpicId(root), items.clone(), additive, identity.clone())
                    .await
                    .unwrap();
                let m = mem
                    .ingest_feed(EpicId(root), items, additive, identity)
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

        // -- cycle 3: additive removes nothing ------------------------------------
        let (r, m) = ingest(vec![ingest_item("pr-5", &pr(5), &["team-request"])], true).await;
        r.applied().unwrap();
        m.applied().unwrap();
        let want = same_layout(3);
        wait_for!(feed_layout(&rows_real, EpicId(root)) == want);

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
            .ingest_feed(role, vec![], false, identity.clone())
            .await
            .unwrap();
        let on_role_mem = mem
            .ingest_feed(role, vec![], false, identity.clone())
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
            .ingest_feed(EpicId(9999), vec![], false, identity.clone())
            .await
            .unwrap();
        let missing_mem = mem
            .ingest_feed(EpicId(9999), vec![], false, identity.clone())
            .await
            .unwrap();
        assert!(matches!(missing, ReducerOutcome::Refused(_)), "{missing:?}");
        assert!(
            matches!(missing_mem, ReducerOutcome::Refused(_)),
            "{missing_mem:?}"
        );
    });
}
