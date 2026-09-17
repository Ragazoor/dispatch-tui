use super::*;

// ---------------------------------------------------------------------------
// The dispatch orchestration seam
// ---------------------------------------------------------------------------
//
// These are the invariants the three hand-written copies of this flow each
// asserted separately — `DispatchClaimExclusive` and the release-on-failure
// unwind in `docs/specs/dispatch.allium`. They are asserted once here, against
// the seam every entry point now goes through.
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod dispatch_seam {
    use super::*;
    use crate::dispatch::mock_sequence::{DispatchScript, Step};
    use crate::models::{DispatchMode, Task};
    use crate::process::MockProcessRunner;
    use crate::service::{DispatchClaim, DispatchOutcome, DispatchRequest};

    /// A `Backlog` task rooted at a fresh temp repo, plus a service wired to
    /// `runner`. The tempdir is returned so the caller keeps it alive.
    async fn fixture(
        db: &Arc<dyn db::TaskStore>,
        runner: Arc<dyn crate::process::ProcessRunner>,
    ) -> (TaskService, Task, tempfile::TempDir) {
        let bootstrap = task_svc(db);
        let id = bootstrap
            .create_task(make_task_params("/placeholder"))
            .await
            .unwrap();
        let mut task = bootstrap.get_task(id).await.unwrap();
        // Pre-creating `.worktrees/<id>-<slug>` is what puts provisioning on
        // the reuse branch `DispatchScript::dispatch` describes.
        let slug = format!("{}-{}", task.id.0, crate::models::slugify(&task.title));
        let (dir, repo_path, _) = crate::dispatch::tests::make_test_repo_with_worktree(&slug);
        bootstrap
            .update_task(UpdateTaskParams::for_task(id).repo_path(repo_path.clone()))
            .await
            .unwrap();
        task.repo_path = repo_path;
        (task_svc_with_runner(db, runner), task, dir)
    }

    fn request(task: Task, mode: DispatchMode, claim: DispatchClaim) -> DispatchRequest {
        DispatchRequest {
            task,
            mode,
            emb_svc: crate::service::embeddings::EmbeddingService::new_test(),
            epic_ctx: None,
            claim,
        }
    }

    /// The happy path: the seam claims, provisions, and records where the agent
    /// landed, so the caller never writes `worktree`/`tmux_window` itself.
    #[tokio::test]
    async fn dispatch_claims_provisions_and_records_the_agent_location() {
        let db = test_db().await;
        let runner = DispatchScript::dispatch().shared_runner();
        let (svc, task, _dir) = fixture(&db, runner.clone()).await;
        let id = task.id;

        let outcome = svc
            .dispatch(request(task, DispatchMode::Dispatch, DispatchClaim::Take))
            .await;

        let DispatchOutcome::Launched(result) = outcome else {
            panic!("expected Launched, got {outcome:?}");
        };
        let stored = svc.get_task(id).await.unwrap();
        assert_eq!(stored.status, TaskStatus::Running);
        assert_eq!(
            stored.worktree.as_deref(),
            Some(result.worktree_path.as_str())
        );
        assert_eq!(
            stored.tmux_window.as_ref().map(|w| w.as_str()),
            Some(result.tmux_window.as_str())
        );
        // Paired with `worktree` per core/Task's `HostTracksWorktree`
        // invariant (docs/specs/core.allium): this write records the
        // worktree, so it owes the host (`DispatchTask` in
        // docs/specs/dispatch.allium).
        let (local_host_id, _label) = db.ensure_host_identity().await.unwrap();
        assert_eq!(stored.host.as_deref(), Some(local_host_id.as_str()));
        // The `Dispatch` half of the mode routing. Every launcher now shares
        // one permission mode (`EveryTaskAgentLaunchesInAutoMode`), so the
        // prompt is what tells the two apart. Its `Research` twin is below.
        let prompt =
            std::fs::read_to_string(format!("{}/.claude-prompt", result.worktree_path)).unwrap();
        assert!(
            !prompt.contains(crate::dispatch::RESEARCH_AGENT_INTRO),
            "standard dispatch must not reach build_research_prompt: {prompt}"
        );
    }

    /// A dispatch that fails after winning the claim owes the release: the task
    /// must be dispatchable again, exactly as it was before the attempt.
    #[tokio::test]
    async fn dispatch_releases_the_claim_when_provisioning_fails() {
        let db = test_db().await;
        let runner = DispatchScript::dispatch()
            .fails_at(Step::NewWindow)
            .shared_runner();
        let (svc, task, _dir) = fixture(&db, runner).await;
        let id = task.id;

        let outcome = svc
            .dispatch(request(task, DispatchMode::Dispatch, DispatchClaim::Take))
            .await;

        assert!(
            matches!(outcome, DispatchOutcome::Failed(_)),
            "expected Failed, got {outcome:?}"
        );
        let stored = svc.get_task(id).await.unwrap();
        assert_eq!(stored.status, TaskStatus::Backlog);
        assert!(stored.worktree.is_none());
        assert!(stored.tmux_window.is_none());
    }

    /// `DispatchClaimExclusive`: a task that is no longer in backlog is reported
    /// as a lost claim and — the part that matters — nothing is provisioned for
    /// it, so the winner's worktree is never cut twice.
    #[tokio::test]
    async fn dispatch_reports_a_lost_claim_and_provisions_nothing() {
        let db = test_db().await;
        let runner = Arc::new(MockProcessRunner::new(vec![]));
        let (svc, task, _dir) = fixture(&db, runner.clone()).await;
        // Something else got there first.
        assert!(svc.claim_backlog_task(task.id).await.unwrap());

        let outcome = svc
            .dispatch(request(task, DispatchMode::Dispatch, DispatchClaim::Take))
            .await;

        assert!(
            matches!(outcome, DispatchOutcome::ClaimLost),
            "expected ClaimLost, got {outcome:?}"
        );
        assert!(
            runner.recorded_calls().is_empty(),
            "a lost claim must provision nothing: {:?}",
            runner.recorded_calls()
        );
    }

    /// `DispatchTask`'s `requires: task.is_locally_owned`
    /// (docs/specs/dispatch.allium): a task whose `host` names another
    /// machine must be refused at the claim, exactly like a claim someone
    /// else already won — re-dispatching it here would provision a fresh
    /// worktree over a directory that is not on this disk and silently
    /// transfer ownership of work that machine may still be running.
    #[tokio::test]
    async fn dispatch_refuses_a_foreign_owned_task_and_provisions_nothing() {
        let db = test_db().await;
        let runner = Arc::new(MockProcessRunner::new(vec![]));
        let (svc, task, _dir) = fixture(&db, runner.clone()).await;
        db.patch_task(
            task.id,
            &db::TaskPatch::new().host(Some("some-other-machine")),
        )
        .await
        .unwrap();

        let outcome = svc
            .dispatch(request(
                task.clone(),
                DispatchMode::Dispatch,
                DispatchClaim::Take,
            ))
            .await;

        assert!(
            matches!(outcome, DispatchOutcome::ClaimLost),
            "expected ClaimLost, got {outcome:?}"
        );
        assert!(
            runner.recorded_calls().is_empty(),
            "a foreign-owned task must provision nothing: {:?}",
            runner.recorded_calls()
        );
        let stored = svc.get_task(task.id).await.unwrap();
        assert_eq!(stored.status, TaskStatus::Backlog, "left exactly as it was");
        assert_eq!(stored.host.as_deref(), Some("some-other-machine"));
    }

    /// `DispatchTask`'s host resolution happens BEFORE provisioning, so an
    /// unreadable settings store aborts the dispatch instead of recording a
    /// worktree with `host` null — the row core/Task's `HostTracksWorktree`
    /// forbids. This is the arm that has no test before now: while the id was
    /// read at the write instead, the only available outcome was that forbidden
    /// row, logged and carried on from.
    ///
    /// The store is broken by renaming the column every settings read selects;
    /// dropping the table does not work, because opening the database recreates
    /// it. The claim is passed as already `Held`, because the claim SQL reads
    /// `host_id` from that same table (`LOCALLY_OWNED_PREDICATE`) and would
    /// otherwise abort one step earlier — upholding the invariant, but by a
    /// different arm than the one under test.
    #[tokio::test]
    async fn dispatch_aborts_when_the_host_identity_cannot_be_resolved() {
        let concrete = Arc::new(Database::open_in_memory().await.unwrap());
        let db: Arc<dyn db::TaskStore> = concrete.clone();
        let runner = Arc::new(MockProcessRunner::new(vec![]));
        let (svc, task, _dir) = fixture(&db, runner.clone()).await;
        let id = task.id;

        concrete
            .db_call(|conn| {
                conn.execute_batch("ALTER TABLE settings RENAME COLUMN value TO renamed_value")
                    .map_err(anyhow::Error::from)
            })
            .await
            .unwrap();

        let outcome = svc
            .dispatch(request(
                task.clone(),
                DispatchMode::Dispatch,
                DispatchClaim::Held,
            ))
            .await;

        assert!(
            matches!(outcome, DispatchOutcome::Failed(_)),
            "expected Failed, got {outcome:?}"
        );
        assert!(
            runner.recorded_calls().is_empty(),
            "an unresolvable host must provision nothing: {:?}",
            runner.recorded_calls()
        );
        let stored = svc.get_task(id).await.unwrap();
        assert!(
            stored.worktree.is_none() && stored.host.is_none(),
            "HostTracksWorktree: neither field may be written when the other cannot be"
        );
    }

    /// The same exclusion under real concurrency: two callers race the seam and
    /// exactly one launches.
    #[tokio::test]
    async fn two_concurrent_dispatches_launch_exactly_one_agent() {
        let db = test_db().await;
        let runner = DispatchScript::dispatch().shared_runner();
        let (svc, task, _dir) = fixture(&db, runner).await;
        let svc = Arc::new(svc);

        let (s1, s2) = (svc.clone(), svc.clone());
        let (t1, t2) = (task.clone(), task);
        let h1 = tokio::spawn(async move {
            s1.dispatch(request(t1, DispatchMode::Dispatch, DispatchClaim::Take))
                .await
        });
        let h2 = tokio::spawn(async move {
            s2.dispatch(request(t2, DispatchMode::Dispatch, DispatchClaim::Take))
                .await
        });
        let (a, b) = (h1.await.unwrap(), h2.await.unwrap());

        let outcomes = [&a, &b];
        assert_eq!(
            outcomes
                .iter()
                .filter(|o| matches!(o, DispatchOutcome::Launched(_)))
                .count(),
            1,
            "exactly one caller may provision: {a:?} / {b:?}"
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|o| matches!(o, DispatchOutcome::ClaimLost))
                .count(),
            1,
            "the other must see a lost claim: {a:?} / {b:?}"
        );
    }

    /// `DispatchClaim::Held` is for the epic chain, whose
    /// `claim_next_backlog_task` both selected and claimed the row: the seam
    /// must not try to claim it a second time (which would lose, since the task
    /// is already Running) and must dispatch it.
    #[tokio::test]
    async fn dispatch_with_a_held_claim_does_not_reclaim() {
        let db = test_db().await;
        let runner = DispatchScript::dispatch().shared_runner();
        let (svc, task, _dir) = fixture(&db, runner).await;
        assert!(svc.claim_backlog_task(task.id).await.unwrap());

        let outcome = svc
            .dispatch(request(task, DispatchMode::Dispatch, DispatchClaim::Held))
            .await;

        assert!(
            matches!(outcome, DispatchOutcome::Launched(_)),
            "expected Launched, got {outcome:?}"
        );
    }

    /// Mode routing, the match that used to be written out twice: `Research`
    /// launches the research agent. Every launcher now shares one permission
    /// mode (`EveryTaskAgentLaunchesInAutoMode`), so the prompt — not the argv
    /// — is what identifies which agent the mode reached.
    #[tokio::test]
    async fn research_mode_launches_the_research_agent() {
        let db = test_db().await;
        let runner = DispatchScript::dispatch().shared_runner();
        let (svc, task, _dir) = fixture(&db, runner.clone()).await;

        let outcome = svc
            .dispatch(request(task, DispatchMode::Research, DispatchClaim::Take))
            .await;

        let DispatchOutcome::Launched(result) = outcome else {
            panic!("expected Launched, got {outcome:?}");
        };
        let prompt =
            std::fs::read_to_string(format!("{}/.claude-prompt", result.worktree_path)).unwrap();
        assert!(
            prompt.contains(crate::dispatch::RESEARCH_AGENT_INTRO),
            "research mode must reach build_research_prompt: {prompt}"
        );
    }

    /// With `epic_ctx: None` the seam resolves the epic banner itself, through
    /// the service's own handle — the request carries no database to read from.
    ///
    /// What this asserts is the banner reaching the launched prompt via
    /// `TaskService::dispatch`; that a caller *cannot* aim the reads at some
    /// other database is enforced by `DispatchRequest` having no `db` field,
    /// not by anything observable here.
    #[tokio::test]
    async fn dispatch_reads_the_epic_banner_from_the_services_own_handle() {
        let db = test_db().await;
        let runner = DispatchScript::dispatch().shared_runner();
        let (svc, task, _dir) = fixture(&db, runner.clone()).await;
        let epic = make_epic(&epic_svc(&db), "Own-handle epic").await;
        svc.update_task(UpdateTaskParams::for_task(task.id).epic_id(epic.id))
            .await
            .unwrap();
        // Re-read so the row handed to the seam carries the epic link; with
        // `epic_ctx: None` the banner is the prologue's own read.
        let task = svc.get_task(task.id).await.unwrap();

        let outcome = svc
            .dispatch(request(task, DispatchMode::Dispatch, DispatchClaim::Take))
            .await;

        let DispatchOutcome::Launched(result) = outcome else {
            panic!("expected Launched, got {outcome:?}");
        };
        let prompt =
            std::fs::read_to_string(format!("{}/.claude-prompt", result.worktree_path)).unwrap();
        assert!(
            prompt.contains("Own-handle epic"),
            "prompt must carry the epic banner the prologue read for itself: {prompt}"
        );
    }
}
