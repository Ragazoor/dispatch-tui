use super::*;
use crate::models::test_tmux_window;

mod repo_path {
    use super::*;

    #[tokio::test]
    async fn exec_delete_repo_path_removes_and_refreshes() {
        let (rt, mut app) = test_runtime().await;
        rt.exec_save_repo_path(&mut app, "/repo1".into()).await;
        rt.exec_save_repo_path(&mut app, "/repo2".into()).await;
        assert_eq!(app.repo_paths().len(), 2);

        rt.exec_delete_repo_path(&mut app, "/repo1").await;
        assert_eq!(app.repo_paths().len(), 1);
        assert!(app.repo_paths().contains(&"/repo2".to_string()));
        assert!(app.error_popup().is_none());
    }
}

mod browser_and_tmux_window {
    use super::*;

    #[tokio::test]
    async fn exec_open_in_browser_calls_xdg_open() {
        let db = test_db().await;
        let (tx, _rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(vec![
            MockProcessRunner::ok(), // xdg-open
        ]));
        let rt = make_runtime(db, tx, mock.clone()).await;

        rt.exec_open_in_browser("https://github.com/org/repo/pull/1".into())
            .await
            .unwrap();
        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "xdg-open");
        assert!(calls[0]
            .1
            .contains(&"https://github.com/org/repo/pull/1".to_string()));
    }

    #[tokio::test]
    async fn exec_kill_tmux_window_calls_kill() {
        let db = test_db().await;
        let (tx, _rx) = mpsc::unbounded_channel();
        let mock = Arc::new(
            MockProcessRunner::new(vec![
                MockProcessRunner::ok(), // tmux kill-window
            ])
            .with_windows(&["task-1"]),
        );
        let rt = make_runtime(db, tx, mock.clone()).await;

        rt.exec_kill_tmux_window(test_tmux_window("task-1"))
            .await
            .unwrap();
        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "tmux");
        assert!(calls[0].1.contains(&"kill-window".to_string()));
        // Targeted by resolved pane ID, not by name — see `tmux::window_target`.
        assert!(calls[0].1.contains(&mock.pane_id_of("task-1")));
    }

    /// Clearing subagent entries for a task that no longer exists is moot, not
    /// a fault: the task was deleted while its hook was still in flight. Only
    /// the typed `ServiceError::NotFound` is demoted, so a real DB failure on
    /// the same path still warns.
    #[tokio::test]
    async fn exec_clear_subagents_is_silent_when_the_task_is_gone() {
        let log = crate::test_log::logged_during(|| async {
            let db = test_db().await;
            let (tx, _rx) = mpsc::unbounded_channel();
            let mock = Arc::new(MockProcessRunner::new(vec![]));
            let rt = make_runtime(db, tx, mock).await;

            // Never created, so the service reports NotFound.
            rt.exec_clear_subagents(crate::models::TaskId(4242), crate::models::DrainMode::Drain)
                .await;
        })
        .await;

        assert!(
            !log.contains("failed to clear subagent entries"),
            "a task that no longer exists must not warn, got log: {log}"
        );
    }

    #[tokio::test]
    async fn exec_kill_tmux_window_failure_is_best_effort() {
        let db = test_db().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(vec![MockProcessRunner::fail(
            "no such window",
        )]));
        let rt = make_runtime(db, tx, mock).await;

        rt.exec_kill_tmux_window(test_tmux_window("gone-window"))
            .await
            .unwrap();

        // Kill-window failure is best-effort — no error message sent
        assert!(rx.try_recv().is_err(), "Expected no message, but got one");
    }
}

mod load_init_helpers {
    use super::*;

    #[tokio::test]
    async fn load_notifications_pref_defaults_to_false_when_not_set() {
        let db = Database::open_in_memory().await.unwrap();
        let mut app = empty_app();
        load_notifications_pref(&db, &mut app).await;
        assert!(!app.notifications_enabled());
    }

    #[tokio::test]
    async fn load_notifications_pref_sets_true_when_enabled() {
        let db = Database::open_in_memory().await.unwrap();
        db.set_setting_bool("notifications_enabled", true)
            .await
            .unwrap();
        let mut app = empty_app();
        load_notifications_pref(&db, &mut app).await;
        assert!(app.notifications_enabled());
    }

    #[tokio::test]
    async fn load_repo_filter_loads_paths_and_mode() {
        let db = Database::open_in_memory().await.unwrap();
        db.set_setting_string(
            "repo_filter",
            &serde_json::to_string(&vec!["/repo/a".to_string(), "/repo/b".to_string()]).unwrap(),
        )
        .await
        .unwrap();
        db.set_setting_string("repo_filter_mode", RepoFilterMode::Exclude.as_str())
            .await
            .unwrap();
        let mut app = empty_app();

        load_repo_filter(&db, &mut app).await;

        assert_eq!(
            app.repo_filter(),
            &std::collections::HashSet::from(["/repo/a".to_string(), "/repo/b".to_string()])
        );
        assert_eq!(app.repo_filter_mode(), RepoFilterMode::Exclude);
    }

    #[tokio::test]
    async fn load_repo_filter_leaves_defaults_when_nothing_saved() {
        let db = Database::open_in_memory().await.unwrap();
        let mut app = empty_app();

        load_repo_filter(&db, &mut app).await;

        assert!(app.repo_filter().is_empty());
        assert_eq!(app.repo_filter_mode(), RepoFilterMode::Include);
    }

    #[tokio::test]
    async fn load_repo_filter_ignores_an_unparseable_saved_mode() {
        let db = Database::open_in_memory().await.unwrap();
        db.set_setting_string("repo_filter_mode", "bogus")
            .await
            .unwrap();
        let mut app = empty_app();

        load_repo_filter(&db, &mut app).await;

        assert_eq!(
            app.repo_filter_mode(),
            RepoFilterMode::Include,
            "an unparseable saved mode must leave the default in place"
        );
    }

    #[tokio::test]
    async fn apply_tmux_focus_warning_returns_none_when_enabled() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"on\n")]);
        let result = apply_tmux_focus_warning(&mock);
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn apply_tmux_focus_warning_returns_status_info_when_disabled() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"off\n")]);
        let result = apply_tmux_focus_warning(&mock);
        assert!(matches!(
            result,
            Some(Message::System(
                crate::tui::messages::SystemMessage::StatusInfo(_)
            ))
        ));
    }
}

/// The shared dispatch prologue. Four launch sites (dispatch_task and the epic chain
/// in src/mcp/handlers/tasks/dispatch.rs, exec_quick_dispatch and exec_dispatch_agent
/// in src/runtime/tasks.rs) run it; their own end-to-end tests cover the wiring,
/// these pin the prologue itself.
mod prepare_inputs {
    use super::*;

    #[tokio::test]
    async fn prepare_inputs_reads_epic_context_and_injections() {
        use crate::models::{LearningKind, LearningScope, RetrievalSource};
        use crate::service::embeddings::{serialize_embedding, EmbeddingService};
        use crate::store::CreateLearningRow;

        let (rt, _app) = test_runtime().await;
        let db = rt.db_write().clone();
        let epic = db.create_epic("Chained Epic", "desc", None).await.unwrap();
        let task_id = db
            .create_task(CreateTaskRequest {
                title: "title",
                description: "desc",
                repo_path: "/repo/a",
                plan: None,
                status: models::TaskStatus::Backlog,
                base_branch: "main",
                epic_id: Some(epic.id),
                sort_order: None,
                tag: None,
                wrap_up_mode: None,
                auto_run_plan: false,
                phoenix: false,
            })
            .await
            .unwrap();
        let task = db.get_task(task_id).await.unwrap().unwrap();
        let learning_id = db
            .create_learning(CreateLearningRow {
                kind: LearningKind::Convention,
                summary: "Use Arc for shared state.",
                detail: None,
                scope: LearningScope::Repo,
                scope_ref: Some("/repo/a"),
                tags: &[],
                source_task_id: None,
                embedding: Some(&serialize_embedding(&[0.1f32; 384])),
            })
            .await
            .unwrap();

        let inputs =
            crate::dispatch::prepare_inputs(&*db, &task, &EmbeddingService::new_test()).await;

        let epic_ctx = inputs.epic_ctx.expect("epic context read from the DB");
        assert_eq!(epic_ctx.epic_id, epic.id);
        assert_eq!(epic_ctx.epic_title, "Chained Epic");
        assert_eq!(
            inputs.injected.iter().map(|l| l.id).collect::<Vec<_>>(),
            vec![learning_id]
        );

        // The prologue's side effect: each injection is recorded as a retrieval.
        let rows = db.list_retrievals_for_task(task.id).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(matches!(rows[0].source, RetrievalSource::PromptInjection));
    }

    #[tokio::test]
    async fn prepare_inputs_with_epic_ctx_uses_the_supplied_context() {
        use crate::service::embeddings::EmbeddingService;

        let (rt, _app) = test_runtime().await;
        let db = rt.db_write().clone();
        // Deliberately epic-less: a from_db read would yield None, so seeing the
        // supplied context proves it was not re-read.
        let task = create_task_returning(
            &*db,
            "title",
            "desc",
            "/repo/a",
            None,
            models::TaskStatus::Backlog,
        )
        .await
        .unwrap();
        let supplied = crate::dispatch::EpicContext {
            epic_id: models::EpicId(7),
            epic_title: "Already in hand".to_string(),
            under_cve_feed: false,
        };

        let inputs = crate::dispatch::prepare_inputs_with_epic_ctx(
            &*db,
            &task,
            &EmbeddingService::new_test(),
            Some(supplied),
        )
        .await;

        let epic_ctx = inputs.epic_ctx.expect("the supplied context is returned");
        assert_eq!(epic_ctx.epic_id, models::EpicId(7));
        assert_eq!(epic_ctx.epic_title, "Already in hand");
        assert!(inputs.injected.is_empty());
    }
}

mod backfill_embeddings {
    use super::*;

    #[tokio::test]
    async fn backfill_fills_missing_embeddings() {
        use crate::models::{LearningKind, LearningScope};
        use crate::service::embeddings::EmbeddingService;
        use crate::store::{CreateLearningRow, LearningStore};

        let db = Arc::new(Database::open_in_memory().await.unwrap());

        // Insert two learnings without embeddings.
        let id1 = db
            .create_learning(CreateLearningRow {
                kind: LearningKind::Convention,
                summary: "always use snake_case",
                detail: None,
                scope: LearningScope::User,
                scope_ref: None,
                tags: &[],
                source_task_id: None,
                embedding: None,
            })
            .await
            .unwrap();
        let id2 = db
            .create_learning(CreateLearningRow {
                kind: LearningKind::Pitfall,
                summary: "avoid unwrap in production",
                detail: Some("use ? or expect with a message"),
                scope: LearningScope::User,
                scope_ref: None,
                tags: &["rust".to_string()],
                source_task_id: None,
                embedding: None,
            })
            .await
            .unwrap();

        // Confirm both are missing embeddings before backfill.
        let missing_before = db.list_learnings_missing_embedding().await.unwrap();
        assert_eq!(
            missing_before.len(),
            2,
            "expected 2 learnings missing embeddings"
        );

        // Run the backfill using the test stub service.
        let emb_svc = EmbeddingService::new_noop();
        let db_for_backfill: Arc<dyn crate::store::LearningStore + Send + Sync> = db.clone();
        super::backfill_embeddings(db_for_backfill, emb_svc)
            .await
            .unwrap();

        // After backfill, no learnings should be missing embeddings.
        let missing_after = db.list_learnings_missing_embedding().await.unwrap();
        assert!(
            missing_after.is_empty(),
            "expected 0 learnings missing embeddings after backfill, got {}",
            missing_after.len()
        );

        // Both learnings should now have non-empty embeddings stored.
        let l1 = db.get_learning(id1).await.unwrap().unwrap();
        let l2 = db.get_learning(id2).await.unwrap().unwrap();
        // Verify via list_all_approved_non_task_learnings which returns embeddings
        let all = db.list_all_approved_non_task_learnings().await.unwrap();
        let emb1 = all.iter().find(|(l, _)| l.id == l1.id).map(|(_, e)| e);
        let emb2 = all.iter().find(|(l, _)| l.id == l2.id).map(|(_, e)| e);
        assert!(
            emb1.is_some_and(|e| !e.is_empty()),
            "learning 1 should have embedding"
        );
        assert!(
            emb2.is_some_and(|e| !e.is_empty()),
            "learning 2 should have embedding"
        );
    }

    #[tokio::test]
    async fn backfill_is_noop_when_no_missing_embeddings() {
        use crate::models::{LearningKind, LearningScope};
        use crate::service::embeddings::{serialize_embedding, EmbeddingService};
        use crate::store::{CreateLearningRow, LearningStore};

        let db = Arc::new(Database::open_in_memory().await.unwrap());

        // Insert a learning that already has an embedding.
        let sentinel = serialize_embedding(&vec![0.1f32; 384]);
        db.create_learning(CreateLearningRow {
            kind: LearningKind::Convention,
            summary: "already embedded",
            detail: None,
            scope: LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: Some(&sentinel),
        })
        .await
        .unwrap();

        let missing_before = db.list_learnings_missing_embedding().await.unwrap();
        assert!(
            missing_before.is_empty(),
            "precondition: no missing embeddings"
        );

        // Backfill should succeed without doing any work.
        let emb_svc = EmbeddingService::new_noop();
        let db_for_backfill: Arc<dyn crate::store::LearningStore + Send + Sync> = db.clone();
        super::backfill_embeddings(db_for_backfill, emb_svc)
            .await
            .unwrap();

        let missing_after = db.list_learnings_missing_embedding().await.unwrap();
        assert!(
            missing_after.is_empty(),
            "still no missing embeddings after no-op backfill"
        );
    }
}

/// Local-first repo sync (docs/specs/repo-sync.allium)
mod repo_sync {
    use super::*;

    /// The three responses one fetching refresh consumes: symbolic-ref, fetch,
    /// rev-list.
    fn refresh_responses_fetching(counts: &[u8]) -> Vec<anyhow::Result<std::process::Output>> {
        vec![
            MockProcessRunner::ok_with_stdout(b"refs/remotes/origin/main\n"),
            MockProcessRunner::ok(),
            MockProcessRunner::ok_with_stdout(counts),
        ]
    }

    async fn expect_measurement(
        rx: &mut mpsc::UnboundedReceiver<Message>,
    ) -> crate::repo_sync::RepoSyncMeasurement {
        match recv_msg(rx).await {
            Message::RepoSync(crate::tui::messages::RepoSyncMessage::Measured(m)) => m,
            other => panic!("expected a repo-sync measurement, got {other:?}"),
        }
    }

    // rule-success.RefreshRepoSyncState: the refresh runs off the event loop and
    // reports its measurement back as a message.
    #[tokio::test]
    async fn exec_refresh_repo_sync_reports_the_measurement() {
        let db = test_db().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(refresh_responses_fetching(
            b"3\t1\n",
        )));
        let rt = make_runtime(db, tx, mock.clone()).await;

        rt.exec_refresh_repo_sync("/repo".to_string(), true)
            .await
            .unwrap();

        let m = expect_measurement(&mut rx).await;
        assert_eq!(m.repo_path, "/repo");
        assert_eq!(m.base_branch, "main");
        assert_eq!(
            m.counts,
            Some(crate::repo_sync::AheadBehind {
                ahead: 3,
                behind: 1
            })
        );
        assert!(mock
            .recorded_calls()
            .iter()
            .any(|(_, a)| a.contains(&"fetch".to_string())));
    }

    // Only the fetching refresh points perform a fetch; every other caller rides
    // refs some other operation already refreshed.
    #[tokio::test]
    async fn exec_refresh_repo_sync_without_fetch_touches_no_network() {
        let db = test_db().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"refs/remotes/origin/main\n"),
            MockProcessRunner::ok_with_stdout(b"0\t2\n"),
        ]));
        let rt = make_runtime(db, tx, mock.clone()).await;

        rt.exec_refresh_repo_sync("/repo".to_string(), false)
            .await
            .unwrap();

        let m = expect_measurement(&mut rx).await;
        assert_eq!(
            m.counts,
            Some(crate::repo_sync::AheadBehind {
                ahead: 0,
                behind: 2
            })
        );
        assert!(
            !mock
                .recorded_calls()
                .iter()
                .any(|(_, a)| a.contains(&"fetch".to_string())),
            "a non-fetching refresh must be a pure local ref read"
        );
    }

    // rule-success.RefreshRepoSyncStateOnStartup + OneRepoSetForDriftMeasurement:
    // one fetching refresh per saved repo path, and no other repository.
    #[tokio::test]
    async fn exec_refresh_all_repo_sync_fetches_once_per_saved_repo_path() {
        let db = test_db().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut responses = refresh_responses_fetching(b"1\t0\n");
        responses.extend(refresh_responses_fetching(b"0\t1\n"));
        let mock = Arc::new(MockProcessRunner::new(responses));
        let rt = make_runtime(db, tx, mock.clone()).await;

        let paths = vec!["/repo-a".to_string(), "/repo-b".to_string()];
        for handle in rt.exec_refresh_all_repo_sync(&paths) {
            handle.await.unwrap();
        }

        let mut seen = vec![
            expect_measurement(&mut rx).await.repo_path,
            expect_measurement(&mut rx).await.repo_path,
        ];
        seen.sort();
        assert_eq!(seen, paths);
        assert_eq!(
            mock.recorded_calls()
                .iter()
                .filter(|(_, a)| a.contains(&"fetch".to_string()))
                .count(),
            2,
            "exactly one fetch per saved repo path"
        );
    }

    #[tokio::test]
    async fn exec_refresh_all_repo_sync_does_nothing_without_saved_paths() {
        let db = test_db().await;
        let (tx, _rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(vec![]));
        let rt = make_runtime(db, tx, mock.clone()).await;

        assert!(rt.exec_refresh_all_repo_sync(&[]).is_empty());
        assert!(mock.recorded_calls().is_empty());
    }

    // rule-success.SyncRepo, reported back through the success channel.
    #[tokio::test]
    async fn exec_sync_repo_reports_the_counts_it_moved() {
        let db = test_db().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"), // remote
            MockProcessRunner::ok_with_stdout(b"main\n"),                        // branch
            MockProcessRunner::ok_with_stdout(b""),                              // clean
            MockProcessRunner::ok(),                                             // fetch
            MockProcessRunner::ok_with_stdout(b"3\t1\n"),                        // rev-list
            MockProcessRunner::ok(),                                             // merge
            MockProcessRunner::ok_with_stdout(b"4\t0\n"),                        // recount
            MockProcessRunner::ok(),                                             // push
        ]));
        let rt = make_runtime(db, tx, mock).await;

        rt.exec_sync_repo("/repo".to_string(), "main".to_string())
            .await
            .unwrap();

        match recv_msg(&mut rx).await {
            Message::RepoSync(crate::tui::messages::RepoSyncMessage::Succeeded {
                repo_path,
                outcome,
            }) => {
                assert_eq!(repo_path, "/repo");
                assert_eq!(
                    outcome,
                    crate::repo_sync::SyncOutcome::Synced {
                        pulled: 1,
                        pushed: 4
                    }
                );
            }
            other => panic!("expected a sync success, got {other:?}"),
        }
    }

    // rule-success.ReportRepoSyncFailure: the failure channel carries the detail
    // that makes the cause actionable, plus whether retrying is the fix.
    #[tokio::test]
    async fn exec_sync_repo_reports_a_failure_with_its_detail() {
        let db = test_db().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"),
            MockProcessRunner::ok_with_stdout(b"feature\n"), // not on base branch
        ]));
        let rt = make_runtime(db, tx, mock).await;

        rt.exec_sync_repo("/repo".to_string(), "main".to_string())
            .await
            .unwrap();

        match recv_msg(&mut rx).await {
            Message::RepoSync(crate::tui::messages::RepoSyncMessage::Failed {
                repo_path,
                detail,
                retryable,
            }) => {
                assert_eq!(repo_path, "/repo");
                assert!(
                    detail.contains("feature") && detail.contains("main"),
                    "the branch found and the one expected: {detail}"
                );
                assert!(!retryable, "the operator must checkout main first");
            }
            other => panic!("expected a sync failure, got {other:?}"),
        }
    }

    // rule-success.RefreshRepoSyncStateAfterRebase: a rebase that moved the repo's
    // base branch triggers a non-fetching refresh.
    #[tokio::test]
    async fn apply_loop_event_branch_rebased_refreshes_the_repo() {
        let db = test_db().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"refs/remotes/origin/main\n"),
            MockProcessRunner::ok_with_stdout(b"2\t0\n"),
        ]));
        let rt = make_runtime(db, tx, mock.clone()).await;
        let mut app = App::new(vec![]);

        let cmds = apply_loop_event(
            &mut app,
            LoopEvent::Mcp(mcp::McpEvent::BranchRebased {
                repo_path: "/repo".to_string(),
            }),
            &rt,
        );

        assert!(cmds.is_empty(), "the refresh is spawned, not queued");
        let m = expect_measurement(&mut rx).await;
        assert_eq!(m.repo_path, "/repo");
        assert!(
            !mock
                .recorded_calls()
                .iter()
                .any(|(_, a)| a.contains(&"fetch".to_string())),
            "the rebase already refreshed the refs"
        );
    }

    // rule-success.RefreshRepoSyncStateAfterDispatch: an agent launched off-board
    // (the dispatch_task tool, or epic auto-dispatch chaining) refreshes the
    // repository's drift too, without a fetch — provisioning already fetched
    // origin/<base>.
    #[tokio::test]
    async fn apply_loop_event_agent_launched_refreshes_the_repo() {
        let db = test_db().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"refs/remotes/origin/main\n"),
            MockProcessRunner::ok_with_stdout(b"1\t0\n"),
        ]));
        let rt = make_runtime(db, tx, mock.clone()).await;
        let mut app = App::new(vec![]);

        let cmds = apply_loop_event(
            &mut app,
            LoopEvent::Mcp(mcp::McpEvent::AgentLaunched {
                repo_path: "/repo".to_string(),
            }),
            &rt,
        );

        assert!(cmds.is_empty(), "the refresh is spawned, not queued");
        let m = expect_measurement(&mut rx).await;
        assert_eq!(m.repo_path, "/repo");
        assert!(
            !mock
                .recorded_calls()
                .iter()
                .any(|(_, a)| a.contains(&"fetch".to_string())),
            "provisioning already fetched origin/<base>"
        );
    }

    // rule-failure.RefreshRepoSyncStateAfterRebase.1: no repository could be
    // resolved from the rebased branch, so nothing is refreshed.
    #[tokio::test]
    async fn apply_loop_event_branch_rebased_without_a_repo_refreshes_nothing() {
        let db = test_db().await;
        let (tx, _rx) = mpsc::unbounded_channel();
        let mock = Arc::new(MockProcessRunner::new(vec![]));
        let rt = make_runtime(db, tx, mock.clone()).await;
        let mut app = App::new(vec![]);

        apply_loop_event(
            &mut app,
            LoopEvent::Mcp(mcp::McpEvent::BranchRebased {
                repo_path: String::new(),
            }),
            &rt,
        );

        assert!(
            mock.recorded_calls().is_empty(),
            "an unresolvable repository must not be measured"
        );
    }

    /// `SurfaceAutoDispatchFailure` (docs/specs/epics.allium): the chain's failure
    /// event reaches the board as a message, so the marker, the status line and the
    /// notification are all decided by the app rather than by the loop.
    #[tokio::test]
    async fn apply_loop_event_auto_dispatch_failed_marks_the_subtask() {
        let (rt, mut app) = test_runtime().await;

        let cmds = apply_loop_event(
            &mut app,
            LoopEvent::Mcp(mcp::McpEvent::AutoDispatchFailed {
                task_id: TaskId(1),
                epic_id: crate::models::EpicId(9),
                reason: "no such repo".to_string(),
            }),
            &rt,
        );

        assert!(
            app.auto_dispatch_failed(TaskId(1)),
            "the failure must reach the board's marker, got commands: {cmds:?}"
        );
        let status = app.status_message().unwrap_or_default();
        assert!(
            status.contains("no such repo"),
            "the reason must reach the status line, got: {status}"
        );
    }
}

mod invalidate_feed_cache {
    use super::*;

    /// A live receiver observes the invalidate signal.
    #[tokio::test]
    async fn notifies_the_feed_runner_of_a_change() {
        let (rt, _app) = test_runtime().await;
        let mut watch_rx = rt
            .feed_invalidate_tx
            .as_ref()
            .expect("make_runtime always wires up a live feed runner")
            .subscribe();

        rt.invalidate_feed_cache();

        tokio::time::timeout(TEST_TIMEOUT, watch_rx.changed())
            .await
            .expect("invalidate_feed_cache must notify the feed runner within the timeout")
            .expect("the sender must still be alive");
    }

    /// Best-effort: no live receiver (e.g. the feed runner was never
    /// started) must not panic.
    #[tokio::test]
    async fn is_a_noop_without_a_receiver() {
        let (mut rt, _app) = test_runtime().await;
        rt.feed_invalidate_tx = None;

        rt.invalidate_feed_cache();
    }
}

mod bootstrap {
    use super::*;
    use crate::sync::tests::{accepted, refused, ScriptedConnector};
    use crate::sync::StoreConnector;

    const TEST_STORE: &str = "http://store.test";

    /// The stand-in store: the in-memory reducers over fresh rows, behind a
    /// connector that answers as scripted.
    fn test_store_with(
        database: crate::store::Database,
        connector: Arc<dyn StoreConnector>,
    ) -> StoreParts {
        use crate::sync as sy;
        let rows = Arc::new(sy::SharedRows::new());
        let clock: Arc<dyn crate::service::Clock> = Arc::new(crate::service::SystemClock);
        let reducer_caller: Arc<dyn sy::ReducerCaller> = Arc::new(
            sy::memory_caller::MemoryReducerCaller::new(rows.clone(), clock.clone()),
        );
        let settled_identity = Arc::new(sy::SettledIdentity::default());
        settled_identity.settle("c0ffee");
        let board_reads = Arc::new(sy::SubscriptionBoardReads::new(rows.clone()));
        let database = database.with_shared_store(crate::store::SharedStorePorts {
            writer: Arc::new(sy::ReducerWriter::new(
                reducer_caller.clone(),
                settled_identity.clone(),
                clock,
                "bootstrap-test-host".to_string(),
                board_reads.clone(),
            )),
            reader: board_reads.clone(),
            learning_reader: Arc::new(sy::SubscriptionLearningReads::new(rows.clone())),
            usage_reader: Arc::new(sy::SubscriptionUsageReads::new(rows.clone())),
            retired_feed_item_reader: Arc::new(sy::SubscriptionRetiredFeedItemReads::new(
                rows.clone(),
            )),
        });
        StoreParts {
            database: Arc::new(database),
            rows,
            board_reads,
            connector,
            settled_identity,
            reducer_caller,
            store_identity: |_| None,
        }
    }

    /// One scripted answer: a connect beyond the one startup makes panics,
    /// which is the point — startup makes exactly one attempt.
    fn test_store(database: crate::store::Database, _host: &str) -> StoreParts {
        test_store_with(
            database,
            ScriptedConnector::new(vec![accepted("c0ffee", "token")]),
        )
    }

    fn unreachable_store(database: crate::store::Database, _host: &str) -> StoreParts {
        test_store_with(
            database,
            ScriptedConnector::new(vec![refused("connection refused")]),
        )
    }

    /// A named store that cannot be reached aborts before drawing, with the
    /// attempt's reason. startup.allium: AbortWhenTheStoreCannotBeReached.
    #[tokio::test]
    async fn bootstrap_aborts_when_the_store_cannot_be_reached() {
        let (_dir, db_path, paths) = fixture().await;

        match TuiRuntime::bootstrap_with(
            &db_path,
            0,
            &paths,
            TEST_STORE.into(),
            unreachable_store,
            false,
        )
        .await
        {
            Ok(_) => panic!("a board whose store is unreachable must not start"),
            Err(err) => assert_eq!(
                err.to_string(),
                crate::startup::StartupAbort::StoreUnavailable {
                    reason: "connection refused".to_string()
                }
                .message()
            ),
        }
    }

    // -- StoreIdentityPin (task #28710) ------------------------------------

    const PINNED_DB: &str = "c200298fac876590c951a7e10328c408fb4664de2bd945a55a0eff22ff8e1f77";
    const OTHER_DB: &str = "c200a1ada23f68e494f28f6a791a2afe105f5a0056e01e6762e3d3618a893ccb";

    /// A reachable store whose database is PINNED_DB.
    fn store_holding_pinned_db(database: crate::store::Database, host: &str) -> StoreParts {
        StoreParts {
            store_identity: |_| Some(PINNED_DB.to_string()),
            ..test_store(database, host)
        }
    }

    /// A store whose database is OTHER_DB, behind a connector with no scripted
    /// answers: any connection attempt panics, so a test that passes proves
    /// the launch never connected.
    fn store_holding_other_db_never_connect(
        database: crate::store::Database,
        _host: &str,
    ) -> StoreParts {
        StoreParts {
            store_identity: |_| Some(OTHER_DB.to_string()),
            ..test_store_with(database, ScriptedConnector::new(vec![]))
        }
    }

    /// A reachable store whose database is OTHER_DB.
    fn store_holding_other_db(database: crate::store::Database, host: &str) -> StoreParts {
        StoreParts {
            store_identity: |_| Some(OTHER_DB.to_string()),
            ..test_store(database, host)
        }
    }

    /// startup.allium: PinTheStoreOnceItAnswers. A first launch pins the
    /// database it reached.
    #[tokio::test]
    async fn bootstrap_pins_the_store_database_once_it_answers() {
        let (_dir, db_path, paths) = fixture().await;

        let _bootstrap = TuiRuntime::bootstrap_with(
            &db_path,
            0,
            &paths,
            TEST_STORE.into(),
            store_holding_pinned_db,
            false,
        )
        .await
        .expect("a first launch has nothing pinned and must start");

        assert_eq!(
            crate::startup::pinned_store_identity(&db_path).as_deref(),
            Some(PINNED_DB)
        );
    }

    /// startup.allium: AbortWhenTheStoreIsNotTheOneThisInstallUses. A store
    /// holding a different database aborts the launch BEFORE the first
    /// connection, which would already write (the connector here panics on any
    /// attempt), and the pin is left as it was.
    #[tokio::test]
    async fn bootstrap_refuses_a_store_holding_a_different_database() {
        let (_dir, db_path, paths) = fixture().await;
        assert!(crate::startup::pin_store_identity(&db_path, PINNED_DB));

        let result = TuiRuntime::bootstrap_with(
            &db_path,
            0,
            &paths,
            TEST_STORE.into(),
            store_holding_other_db_never_connect,
            false,
        )
        .await;

        match result {
            Ok(_) => panic!("a different database must not be connected to"),
            Err(err) => assert_eq!(
                err.downcast::<crate::startup::StartupAbort>().ok(),
                Some(crate::startup::StartupAbort::StoreSwitched {
                    address: TEST_STORE.to_string(),
                    pinned: PINNED_DB.to_string(),
                    found: OTHER_DB.to_string(),
                })
            ),
        }
        assert_eq!(
            crate::startup::pinned_store_identity(&db_path).as_deref(),
            Some(PINNED_DB)
        );
    }

    /// startup.allium: ConnectWhenTheStoreIsTheOneThisInstallUses (an
    /// accepted switch) and PinTheStoreOnceItAnswers: --accept-store-switch
    /// lets one launch through, and the new database is pinned so the next
    /// launch needs no flag.
    #[tokio::test]
    async fn bootstrap_accepts_a_switch_when_told_to_and_repins() {
        let (_dir, db_path, paths) = fixture().await;
        assert!(crate::startup::pin_store_identity(&db_path, PINNED_DB));

        let result = TuiRuntime::bootstrap_with(
            &db_path,
            0,
            &paths,
            TEST_STORE.into(),
            store_holding_other_db,
            true,
        )
        .await;

        assert!(
            result.is_ok(),
            "an accepted switch must start: {:?}",
            result.err()
        );
        assert_eq!(
            crate::startup::pinned_store_identity(&db_path).as_deref(),
            Some(OTHER_DB)
        );
    }

    /// The board draws its store's address (sync.allium: ConnectionIndicator,
    /// TheStoreIsAlwaysNamed), so bootstrap hands it to the App.
    #[tokio::test]
    async fn bootstrap_gives_the_app_its_store_address() {
        let (_dir, db_path, paths) = fixture().await;

        let bootstrap =
            TuiRuntime::bootstrap_with(&db_path, 0, &paths, TEST_STORE.into(), test_store, false)
                .await
                .expect("bootstrap must succeed");

        assert_eq!(bootstrap.app.store_server(), Some(TEST_STORE));
    }

    /// The record a board leaves beside its database (task #4982,
    /// startup.allium: StoreAddressRecord).
    fn store_record(db_path: &std::path::Path) -> std::path::PathBuf {
        db_path.parent().unwrap().join("store-server")
    }

    /// startup.allium: RecordTheNamedStoreOnceItAnswers. A board on a named
    /// store leaves the address beside its database once the first connection
    /// has succeeded, so a command in a plain terminal reaches the same store.
    #[tokio::test]
    async fn bootstrap_records_a_named_store_beside_the_database_once_it_answers() {
        let (_dir, db_path, paths) = fixture().await;

        let _bootstrap =
            TuiRuntime::bootstrap_with(&db_path, 0, &paths, TEST_STORE.into(), test_store, false)
                .await
                .expect("bootstrap must succeed against a reachable named store");

        let recorded = std::fs::read_to_string(store_record(&db_path))
            .expect("a board on a named store must record its address beside the database");
        assert_eq!(recorded.trim(), TEST_STORE);
    }

    /// startup.allium: RecordTheNamedStoreOnceItAnswers, "ONLY ONCE THE FIRST
    /// CONNECTION HAS SUCCEEDED": a typo on the command line must not become
    /// what every later command reaches for.
    #[tokio::test]
    async fn bootstrap_records_nothing_when_the_named_store_cannot_be_reached() {
        let (_dir, db_path, paths) = fixture().await;

        let result = TuiRuntime::bootstrap_with(
            &db_path,
            0,
            &paths,
            TEST_STORE.into(),
            unreachable_store,
            false,
        )
        .await;

        assert!(result.is_err(), "an unreachable store must abort startup");
        assert!(
            !store_record(&db_path).exists(),
            "an address that never answered must not be recorded"
        );
    }

    /// startup.allium: RecordTheNamedStoreOnceItAnswers -- a record that
    /// cannot be written does not stop the launch. The record path is made a
    /// directory, so no write can land there. Paired with the success test
    /// above, which proves bootstrap attempts the write at all.
    #[tokio::test]
    async fn bootstrap_starts_even_when_the_store_record_cannot_be_written() {
        let (_dir, db_path, paths) = fixture().await;
        std::fs::create_dir(store_record(&db_path)).unwrap();

        let result =
            TuiRuntime::bootstrap_with(&db_path, 0, &paths, TEST_STORE.into(), test_store, false)
                .await;

        assert!(
            result.is_ok(),
            "an unwritable store record must not abort the launch: {:?}",
            result.err()
        );
        assert!(
            store_record(&db_path).is_dir(),
            "the unwritable path is left as it was"
        );
    }

    /// Temp-backed `StartupPaths` plus a database path, the fixture every
    /// test here needs. A bootstrap test must never be handed the operator's
    /// real locations — see docs/specs/observability.allium: StatusLineDecorator,
    /// `SettingsLocationIsAnExplicitStartupInput` — so this is the only shape
    /// a new one should use.
    ///
    /// Pre-names the host (in `host.json`, beside the path) before handing it back. These tests exercise
    /// bootstrap's other wiring (feed seeding, budget snapshot path, the trust
    /// store), not `startup.allium`'s host-label gate, and `cargo test`'s
    /// stdin is never a terminal — an unnamed host here would hit
    /// `AbortWhenTheHostIsUnnamedAndNoOneCanAnswer` for a reason unrelated to
    /// what each test actually checks.
    async fn fixture() -> (tempfile::TempDir, std::path::PathBuf, StartupPaths) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("bootstrap.db");
        let paths = StartupPaths {
            claude_dir: dir.path().join("claude"),
            claude_json_path: dir.path().join(".claude.json"),
        };
        // The host file `--db`'s directory holds, already named.
        crate::host_file::resolve_for_launch(dir.path()).unwrap();
        crate::host_file::rename_host(dir.path(), "bootstrap-test-host").unwrap();
        (dir, db_path, paths)
    }

    /// The happy path: reads the host file, spawns the
    /// MCP server and feed runner in the background, and hydrates the
    /// returned `App`/`TuiRuntime` from persisted settings. Binds port 0 so
    /// the OS picks a free ephemeral port — this pins the startup wiring,
    /// not the MCP server's own behaviour.
    #[tokio::test]
    async fn wires_up_a_working_app_and_runtime() {
        let (_dir, db_path, paths) = fixture().await;

        let bootstrap =
            TuiRuntime::bootstrap_with(&db_path, 0, &paths, TEST_STORE.into(), test_store, false)
                .await
                .expect("bootstrap must succeed against a fresh, writable db path");

        assert!(
            bootstrap.app.tasks().is_empty(),
            "a fresh database has no tasks to hydrate"
        );
        assert!(
            bootstrap
                .runtime
                .database
                .list_all()
                .await
                .unwrap()
                .is_empty(),
            "the returned runtime must be backed by the same freshly-opened database"
        );
        assert!(
            bootstrap.runtime.feed_runner.is_some(),
            "bootstrap must wire up a feed runner for the runtime to own"
        );
    }

    /// The budget snapshot location is account-global and fixed per machine, so
    /// booting against a throwaway database must not move it into that
    /// database's directory (docs/specs/observability.allium:
    /// SnapshotLocationIsFixedNotDerivedFromTheOpenDatabase). Deriving it from
    /// the open database is what let a single `cargo test` run silently
    /// repoint every later Claude session at a temp directory.
    #[tokio::test]
    async fn budget_snapshot_path_ignores_the_open_database() {
        let (dir, db_path, paths) = fixture().await;

        let bootstrap =
            TuiRuntime::bootstrap_with(&db_path, 0, &paths, TEST_STORE.into(), test_store, false)
                .await
                .expect("bootstrap must succeed against a fresh, writable db path");

        assert!(
            !bootstrap
                .runtime
                .budget_snapshot_path
                .starts_with(dir.path()),
            "the snapshot location must not follow the throwaway database"
        );
    }

    /// docs/specs/startup.allium: `ConfigurationIsNeverWrittenWithoutConsent`.
    ///
    /// `bootstrap` used to rewrite the dispatch-owned statusLine settings file
    /// on every TUI start, as a safety net for a user who had not re-run setup.
    /// The startup configuration check now covers exactly that case, *and asks
    /// first* — so a net that still wrote unconditionally would hand a
    /// declining operator the write they refused, one layer further down where
    /// the prompt cannot see it.
    ///
    /// `bootstrap` must therefore put nothing at all into the configuration
    /// directory it is handed. The check's own scoping (the settings file lands
    /// in the supplied directory, and the chain is discovered from that same
    /// directory's `settings.json`) is covered by
    /// `apply_config_update_chains_to_existing_status_line_without_touching_settings_json`
    /// in `src/setup/mod.rs`.
    #[tokio::test]
    async fn bootstrap_writes_nothing_into_the_supplied_claude_dir() {
        let (_dir, db_path, paths) = fixture().await;

        TuiRuntime::bootstrap_with(&db_path, 0, &paths, TEST_STORE.into(), test_store, false)
            .await
            .expect("bootstrap must succeed against a fresh, writable db path");

        assert!(
            !crate::setup::statusline::settings_path(&paths.claude_dir).exists(),
            "bootstrap must not write configuration the operator was never asked about"
        );
        assert!(
            !paths.claude_dir.exists(),
            "bootstrap must not even create the configuration directory"
        );
    }

    /// docs/specs/startup.allium, scope note: the example feed epic is a row
    /// in dispatch's own store, not operator configuration, so it is not gated
    /// on the startup consent prompt.
    ///
    /// It lives here rather than beside that prompt because it needs the first
    /// store connection, which that prompt runs before. (The stand-in store
    /// here is an in-memory SQLite handle, unrouted, so the row is read back
    /// from the runtime's own handle.)
    #[tokio::test]
    async fn bootstrap_seeds_the_example_feed_epic_without_asking() {
        let (_dir, db_path, paths) = fixture().await;

        let bootstrap =
            TuiRuntime::bootstrap_with(&db_path, 0, &paths, TEST_STORE.into(), test_store, false)
                .await
                .expect("bootstrap must succeed against a fresh, writable db path");

        let epics = crate::store::EpicRead::list_epics(&*bootstrap.runtime.database)
            .await
            .unwrap();
        assert!(
            epics.iter().any(|e| e.feed_command.is_some()),
            "a fresh database must get its example feed epic, with no prompt: {epics:?}"
        );
    }

    /// docs/specs/startup.allium: `NameHostFromStartupPrompt`'s failure
    /// branch — the second face of the broken-settings-store condition
    /// `AbortWhenTheHostIdentityStoreIsUnusable` names for its own (read/mint)
    /// face. Here the identity read fine, the machine is unnamed, and the
    /// label `host.allium: RenameHost` was just handed could not be
    /// persisted. Before this change `bootstrap` let that write's error
    /// propagate raw (`database.rename_host(&new_label).await?`), with no
    /// named remedy — this is the gap `startup.allium`'s open question asked
    /// about, and `NameHostFromStartupPrompt` now aborts on it directly with
    /// the same `host_identity_unavailable` reason the read/mint face uses,
    /// on the recorded reasoning that both failures share one remedy (repair
    /// the settings store) and so do not earn a second `StartupAbortReason` —
    /// nor a widened guard on the other rule: see both rules' guidance for
    /// why this is two rules sharing one reason rather than one rule with a
    /// guard spanning both.
    ///
    /// Exercised against `persist_host_label` directly rather than through
    /// the full interactive prompt: `resolve_host_label_interactively`
    /// decides whether to prompt from
    /// `std::io::IsTerminal::is_terminal(&stdin())`, which `cargo test`'s
    /// stdin never reports true for, so there is no way to drive `bootstrap`
    /// into "the operator answered" through its public entry point in a
    /// test. `persist_host_label` is the unit `bootstrap` calls once an
    /// answer is in hand, factored out for exactly this reason.
    ///
    /// The label write is isolated from the identity read by making
    /// `host.json` unusable to write after a mint: here it is replaced by a
    /// directory, which no whole-file replacement can rename over. (The first
    /// face — an identity that cannot be read or minted — is
    /// `host_file_bootstrap`'s unparseable-host-file test.)
    #[tokio::test]
    async fn persist_host_label_maps_a_failed_persist_to_host_identity_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        crate::host_file::resolve_for_launch(dir.path()).unwrap();
        let db = crate::store::Database::unattached().with_host_file(dir.path());
        let host_file = crate::host_file::host_file_path(dir.path());
        std::fs::remove_file(&host_file).unwrap();
        std::fs::create_dir(&host_file).unwrap();

        match persist_host_label(&db, "my-new-name").await {
            Ok(()) => {
                panic!("a label write that fails at the store must not be reported as persisted")
            }
            Err(abort) => assert_eq!(
                abort,
                crate::startup::StartupAbort::HostIdentityUnavailable,
                "a failed label persist must abort through the same reason a failed \
                 mint does, not propagate as an unclassified error"
            ),
        }
    }

    /// The trust store is the other operator-owned file bootstrap wires up.
    /// It comes from the same `StartupPaths`, so a run that is not the
    /// operator's session cannot reach `$HOME/.claude.json` either.
    #[tokio::test]
    async fn trust_store_path_comes_from_the_supplied_paths() {
        let (_dir, db_path, paths) = fixture().await;

        let bootstrap =
            TuiRuntime::bootstrap_with(&db_path, 0, &paths, TEST_STORE.into(), test_store, false)
                .await
                .expect("bootstrap must succeed against a fresh, writable db path");

        assert_eq!(
            bootstrap.runtime.claude_json_path, paths.claude_json_path,
            "the trust store path must be the one startup was handed"
        );
    }
}

/// `StartupPaths::resolve` itself — deliberately outside `mod bootstrap`, whose
/// fixture rule is that a bootstrap test is never handed the operator's real
/// locations. These resolve and compare only: nothing boots, and nothing is
/// written anywhere.
mod startup_paths {
    use super::*;

    /// docs/specs/observability.allium: StatusLineDecorator,
    /// `SpawnSitesAndStartupNameTheSameConfigurationDirectory`. The writer-side
    /// link for the settings file: `src/dispatch/tests.rs` binds the spawn
    /// constant's literal to the configuration-directory layout, and this binds
    /// startup to the same lookup. Neither alone rules out a `resolve` that
    /// quietly hands startup somewhere else.
    ///
    /// This mirrors `resolve`'s composition on purpose. It cannot fail for a
    /// change in what those lookups *return* — that is the dispatch-side test's
    /// job — only if `resolve` stops delegating to them.
    ///
    /// `src/main.rs::cmd_tui` needs no test of its own: `StartupPaths`' fields
    /// are private to this module, so `resolve` is the only way a caller
    /// outside it can obtain one. That link is compiler-enforced, not merely
    /// asserted.
    #[test]
    fn resolve_delegates_to_the_shared_configuration_directory_lookup() {
        let paths = StartupPaths::resolve().expect("$HOME must be set");

        assert_eq!(
            paths.claude_dir,
            crate::setup::claude_dir().expect("$HOME must be set"),
            "startup must be handed the same configuration directory setup \
             resolves — that is the directory the spawn constant's literals \
             are checked against"
        );
    }

    /// docs/specs/observability.allium: StatusLineDecorator,
    /// `AnUnavailableHomeDirectoryIsAFailureNotAPath`. The trust store's
    /// writer-side link.
    ///
    /// `resolve` returning a `Result` is what carries the absent-home failure
    /// out to `src/main.rs::cmd_tui`, and this pins that the trust store is
    /// one of the locations that failure covers — that `resolve` gets it from
    /// the same lookup as the configuration directory rather than deriving it
    /// a second way, which is how the two came to disagree about an absent
    /// `$HOME` in the first place.
    #[test]
    fn resolve_delegates_to_the_shared_trust_store_lookup() {
        let paths = StartupPaths::resolve().expect("$HOME must be set");

        assert_eq!(
            paths.claude_json_path,
            crate::setup::user_global_config_path().expect("$HOME must be set"),
            "startup must be handed the trust store the one $HOME reading \
             resolves, not a path derived independently of it"
        );
    }
}

mod store_address_publication {
    use super::*;

    /// startup.allium: PublishTheBoardsStoreOnItsSession. The address goes
    /// under the board's own variable, never the operator's
    /// DISPATCH_SPACETIME_SERVER, which `dispatch tui` reads (task #28729).
    #[test]
    fn a_board_publishes_its_store_under_the_board_variable() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);

        publish_store_server("dispatch", "http://store.example:3000", &mock);

        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].1,
            vec![
                "set-environment",
                "-t",
                "=dispatch",
                "DISPATCH_BOARD_STORE",
                "http://store.example:3000"
            ]
        );
    }

    /// A managed board publishes the managed address too, overwriting what an
    /// earlier named board left, and never unsets anything.
    #[test]
    fn a_managed_board_overwrites_rather_than_clears() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);

        publish_store_server("dispatch", "http://127.0.0.1:3000", &mock);

        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert!(!calls[0].1.contains(&"-u".to_string()));
        assert_eq!(calls[0].1[3], "DISPATCH_BOARD_STORE");
    }

    #[test]
    fn a_board_with_no_session_touches_nothing() {
        let mock = MockProcessRunner::new(vec![]);

        publish_store_server("", "http://127.0.0.1:3000", &mock);

        assert!(mock.recorded_calls().is_empty());
    }
}

/// The small startup helpers `runtime/mod.rs` owns, which need no store.
mod startup_helpers {
    use super::*;

    #[test]
    fn the_board_port_is_published_on_its_session() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
        publish_board_port("dispatch", 8899, &mock);
        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].1,
            vec![
                "set-environment",
                "-t",
                "=dispatch",
                "DISPATCH_PORT",
                "8899"
            ]
        );
    }

    #[test]
    fn a_board_with_no_session_publishes_no_port() {
        let mock = MockProcessRunner::new(vec![]);
        publish_board_port("", 8899, &mock);
        assert!(mock.recorded_calls().is_empty());
    }

    /// Publishing is best-effort: a tmux that refuses is logged, not raised.
    #[test]
    fn a_refused_publication_does_not_panic() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::fail("no such session"),
            MockProcessRunner::fail("no such session"),
        ]);
        publish_store_server("dispatch", "http://127.0.0.1:3000", &mock);
        publish_board_port("dispatch", 8899, &mock);
        assert_eq!(mock.recorded_calls().len(), 2);
    }

    #[test]
    fn an_absent_value_clears_the_field() {
        assert!(matches!(option_to_field_update(None), FieldUpdate::Clear));
        assert!(matches!(
            option_to_field_update(Some("x".into())),
            FieldUpdate::Set(v) if v == "x"
        ));
    }

    #[test]
    fn an_absent_window_clears_the_window() {
        assert!(matches!(
            option_to_tmux_window_update(None),
            crate::service::TmuxWindowUpdate::Clear
        ));
        assert!(matches!(
            option_to_tmux_window_update(Some(test_tmux_window("w"))),
            crate::service::TmuxWindowUpdate::Set(_)
        ));
    }

    #[test]
    fn base_branches_group_by_repo_keeping_recency_order() {
        let grouped = group_base_branches_by_repo(vec![
            ("/a".into(), "main".into()),
            ("/b".into(), "dev".into()),
            ("/a".into(), "old".into()),
        ]);
        assert_eq!(grouped["/a"], vec!["main", "old"]);
        assert_eq!(grouped["/b"], vec!["dev"]);
    }

    #[test]
    fn a_failure_with_no_managed_store_is_returned_unchanged() {
        let error = abort_managed_startup(None, anyhow::anyhow!("boom"));
        assert_eq!(error.to_string(), "boom");
    }
}

/// Task #4982, startup.allium: StoreAddressRecord and the three rules that
/// write and remove it. The bootstrap half (recorded once the first
/// connection succeeds, never for an unreachable store, a write failure is not
/// fatal) is in `mod bootstrap`; this is the target-dependent half.
mod store_address_record {
    use super::*;

    fn managed_target(dir: &std::path::Path) -> StoreTarget {
        StoreTarget::Managed(Arc::new(
            crate::spacetime::managed_store::ManagedStore::for_launch(dir.to_path_buf(), dir),
        ))
    }

    fn named_target() -> StoreTarget {
        StoreTarget::Named("http://store.example:3000".into())
    }

    /// A database path in a fresh temp directory, with a record already
    /// beside it -- the stale one a board that could not run its exit leaves.
    fn db_with_record(dir: &tempfile::TempDir, address: &str) -> std::path::PathBuf {
        let db_path = dir.path().join("dispatch.db");
        std::fs::write(record_path(&db_path), format!("{address}\n")).unwrap();
        db_path
    }

    fn record_path(db_path: &std::path::Path) -> std::path::PathBuf {
        db_path.parent().unwrap().join("store-server")
    }

    #[test]
    fn a_named_store_is_recorded_beside_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("dispatch.db");

        let kept = record_store_server_for(&named_target(), &db_path, "http://store.example:3000");

        assert!(kept);
        assert_eq!(
            crate::startup::recorded_store_server(&db_path),
            Some("http://store.example:3000".to_string())
        );
    }

    /// RecordTheNamedStoreOnceItAnswers requires a named store: a managed
    /// board's store is already where a command with nothing named looks.
    #[test]
    fn a_managed_board_records_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("dispatch.db");

        let kept = record_store_server_for(
            &managed_target(dir.path()),
            &db_path,
            "http://127.0.0.1:3000",
        );

        assert!(!kept);
        assert!(!record_path(&db_path).exists());
    }

    /// ForgetAStaleStoreRecordOnAManagedLaunch: from then on a command with
    /// nothing named reaches the managed address, where this board's store is.
    #[test]
    fn a_managed_launch_forgets_a_stale_record() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = db_with_record(&dir, "http://gone:3000");

        forget_stale_store_record_for(&managed_target(dir.path()), &db_path);

        assert!(
            !record_path(&db_path).exists(),
            "a managed launch must clear a record a named board left behind"
        );
    }

    /// The forget rule requires `explicit_store_server() = null`. A named
    /// launch replaces the record once its store answers instead; clearing it
    /// first would open a window where commands reach the managed address.
    #[test]
    fn a_named_launch_leaves_the_record_for_its_own_to_replace() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = db_with_record(&dir, "http://earlier:3000");

        forget_stale_store_record_for(&named_target(), &db_path);

        assert!(record_path(&db_path).exists());
    }

    /// ForgetTheStoreRecordWhenTheBoardExits: the record says "a board on this
    /// database is using this store", so it goes when the board does. The
    /// guard's drop is every exit at once -- a quit, a startup abort, a panic.
    #[test]
    fn a_named_boards_exit_forgets_its_record() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = db_with_record(&dir, "http://store.example:3000");

        drop(StoreRecordGuard::for_target(&named_target(), &db_path));

        assert!(
            !record_path(&db_path).exists(),
            "a named board's exit must remove the record it left"
        );
    }

    /// The exit rule requires a named store: a managed board recorded nothing,
    /// so there is nothing of its own to remove.
    #[test]
    fn a_managed_boards_exit_leaves_any_record_alone() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = db_with_record(&dir, "http://other-board:3000");

        drop(StoreRecordGuard::for_target(
            &managed_target(dir.path()),
            &db_path,
        ));

        assert!(record_path(&db_path).exists());
    }

    /// A named store is used as given, and the CLI is not asked for.
    #[test]
    fn a_named_store_is_selected_without_consulting_the_cli() {
        let target = select_store_target(
            Path::new("/tmp/scratch/dispatch.db"),
            Some("http://127.0.0.1:3099".to_string()),
            || panic!("a named store must not need the CLI"),
        )
        .unwrap();

        assert!(matches!(target, StoreTarget::Named(s) if s == "http://127.0.0.1:3099"));
    }

    /// Without a named store and without the CLI the launch aborts rather than
    /// starting nothing.
    #[test]
    fn no_named_store_and_no_cli_is_a_startup_abort() {
        let result = select_store_target(Path::new("/tmp/scratch/dispatch.db"), None, || false);

        assert!(result.is_err());
    }

    /// A managed launch keeps its store data in the fixed default location and
    /// logs beside the given database, never deriving the store from `--db`.
    #[test]
    fn a_managed_store_is_selected_when_none_is_named_and_the_cli_exists() {
        let target =
            select_store_target(Path::new("/tmp/scratch/dispatch.db"), None, || true).unwrap();

        assert!(target.managed().is_some());
    }
}
