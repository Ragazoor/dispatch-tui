use super::*;

// ---------------------------------------------------------------------------
// The dispatch seam, on request rows
// ---------------------------------------------------------------------------
//
// `DispatchClaimExclusive` (`docs/specs/dispatch.allium`) and the host stamp,
// asserted against the seam every entry point goes through since store-split
// M4: `request_dispatch` (the claim, in the module) then
// `serve_dispatch_request` (this host's provisioning). The request-row
// mechanics themselves — deadlines, refused reports, teardown — are pinned in
// `dispatch_requests.rs` beside this file.
mod dispatch_seam {
    use super::*;
    use crate::dispatch::mock_sequence::DispatchScript;
    use crate::models::{DispatchMode, DispatchOrigin, DispatchRequestId, Task};
    use crate::process::MockProcessRunner;
    use crate::service::ServedDispatch;
    use crate::store::RequestDispatchAnswer;

    /// A `Backlog` task rooted at a fresh temp repo, plus a service wired to
    /// `runner`. The tempdir is returned so the caller keeps it alive.
    async fn fixture(
        db: &Arc<dyn store::TaskStore>,
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

    fn requested(answer: RequestDispatchAnswer) -> DispatchRequestId {
        match answer {
            RequestDispatchAnswer::Requested(id) => id,
            RequestDispatchAnswer::Refused(why) => panic!("expected the request to apply: {why}"),
        }
    }

    /// Ask, then serve the row as this host's reactor would.
    async fn request_and_serve(svc: &TaskService, task: TaskId) -> ServedDispatch {
        let id = requested(
            svc.request_dispatch(task, DispatchOrigin::User, Some(DispatchMode::Dispatch))
                .await
                .unwrap(),
        );
        let row = svc.db.dispatch_request(id).await.unwrap();
        svc.serve_dispatch_request(row, crate::embeddings::EmbeddingService::new_test())
            .await
    }

    /// The happy path end to end: the module, not the caller, records where
    /// the agent landed — and the standard mode does not reach the research
    /// prompt (its research twin is `serve_launches_the_agent_the_row_names`).
    #[tokio::test]
    async fn a_served_request_records_the_agent_location() {
        let db = test_db().await;
        let runner = DispatchScript::dispatch().shared_runner();
        let (svc, task, _dir) = fixture(&db, runner.clone()).await;
        let id = task.id;

        let outcome = request_and_serve(&svc, id).await;

        let ServedDispatch::Dispatched(result) = outcome else {
            panic!("expected Dispatched, got {outcome:?}");
        };
        let stored = svc.get_task(id).await.unwrap();
        assert_eq!(stored.status, TaskStatus::Running);
        assert_eq!(
            stored.worktree.as_deref(),
            Some(result.worktree_path.as_str())
        );
        // Paired with `worktree` per core/Task's `HostTracksWorktree`
        // invariant (docs/specs/core.allium).
        let (local_host_id, _label) = db.ensure_host_identity().await.unwrap();
        assert_eq!(stored.host.as_deref(), Some(local_host_id.as_str()));
        let prompt =
            std::fs::read_to_string(format!("{}/.claude-prompt", result.worktree_path)).unwrap();
        assert!(
            !prompt.contains(crate::dispatch::RESEARCH_AGENT_INTRO),
            "standard dispatch must not reach build_research_prompt: {prompt}"
        );
    }

    /// The stamp is the host the store handle was built with — the request
    /// row's host, written when the request was made — not a second read of
    /// the host file. With no host file at all there is nothing to re-read,
    /// and the dispatch still records this machine (`dispatch.allium:
    /// ProvisionDispatch`, "THE HOST IS FIXED BEFORE ANYTHING IS PROVISIONED").
    #[tokio::test]
    async fn the_stamp_is_the_host_the_store_was_built_with() {
        let empty = tempfile::tempdir().unwrap();
        let db: Arc<dyn store::TaskStore> = Arc::new(Store::in_memory_with_host_file(empty.path()));
        let runner = DispatchScript::dispatch().shared_runner();
        let (svc, task, _dir) = fixture(&db, runner).await;
        let id = task.id;

        let outcome = request_and_serve(&svc, id).await;

        assert!(
            matches!(outcome, ServedDispatch::Dispatched(_)),
            "expected Dispatched, got {outcome:?}"
        );
        let stored = svc.get_task(id).await.unwrap();
        assert_eq!(stored.host.as_deref(), Some(crate::store::MEMORY_HOST_ID));
    }

    /// `DispatchTask`'s `requires: task.is_locally_owned`, held by the module
    /// (`store-split.allium: RequestDispatch`): a task whose `host` names
    /// another machine is refused, nothing is written, and its owner's
    /// worktree record is left exactly as it was.
    #[tokio::test]
    async fn a_foreign_owned_task_is_refused_and_left_alone() {
        let db = test_db().await;
        let runner = Arc::new(MockProcessRunner::new(vec![]));
        let (svc, task, _dir) = fixture(&db, runner.clone()).await;
        db.patch_task(
            task.id,
            &store::TaskPatch::new()
                .worktree(Some("/elsewhere/.worktrees/1-x"))
                .host(Some("some-other-machine")),
        )
        .await
        .unwrap();

        let answer = svc
            .request_dispatch(task.id, DispatchOrigin::User, Some(DispatchMode::Dispatch))
            .await
            .unwrap();

        assert!(
            matches!(answer, RequestDispatchAnswer::Refused(_)),
            "expected a refusal, got {answer:?}"
        );
        assert!(db.latest_dispatch_request_for_task(task.id).await.is_none());
        assert!(runner.recorded_calls().is_empty());
        let stored = svc.get_task(task.id).await.unwrap();
        assert_eq!(stored.status, TaskStatus::Backlog, "left exactly as it was");
        assert_eq!(stored.host.as_deref(), Some("some-other-machine"));
    }

    /// `DispatchClaimExclusive` under real concurrency: two callers ask for the
    /// same task at once, exactly one request is written, and serving it
    /// launches exactly one agent.
    #[tokio::test]
    async fn two_concurrent_requests_write_exactly_one_row() {
        let db = test_db().await;
        let runner = DispatchScript::dispatch().shared_runner();
        let (svc, task, _dir) = fixture(&db, runner).await;
        let svc = Arc::new(svc);

        let (s1, s2) = (svc.clone(), svc.clone());
        let id = task.id;
        let h1 = tokio::spawn(async move {
            s1.request_dispatch(id, DispatchOrigin::User, Some(DispatchMode::Dispatch))
                .await
        });
        let h2 =
            tokio::spawn(async move { s2.request_dispatch(id, DispatchOrigin::Mcp, None).await });
        let (a, b) = (h1.await.unwrap().unwrap(), h2.await.unwrap().unwrap());

        let answers = [&a, &b];
        let won: Vec<DispatchRequestId> = answers
            .iter()
            .filter_map(|a| match a {
                RequestDispatchAnswer::Requested(id) => Some(*id),
                RequestDispatchAnswer::Refused(_) => None,
            })
            .collect();
        assert_eq!(
            won.len(),
            1,
            "exactly one request may be written: {a:?} / {b:?}"
        );
        assert_eq!(db.dispatch_requests().await.len(), 1);

        let row = db.dispatch_request(won[0]).await.unwrap();
        let outcome = svc
            .serve_dispatch_request(row, crate::embeddings::EmbeddingService::new_test())
            .await;
        assert!(
            matches!(outcome, ServedDispatch::Dispatched(_)),
            "the one request is provisioned once: {outcome:?}"
        );
    }

    /// The prologue resolves the epic banner itself, through the service's own
    /// handle — the row carries only the task id, so there is no other
    /// database a caller could aim the reads at.
    #[tokio::test]
    async fn serving_reads_the_epic_banner_from_the_services_own_handle() {
        let db = test_db().await;
        let runner = DispatchScript::dispatch().shared_runner();
        let (svc, task, _dir) = fixture(&db, runner.clone()).await;
        let epic = make_epic(&epic_svc(&db), "Own-handle epic").await;
        svc.update_task(UpdateTaskParams::for_task(task.id).epic_id(epic.id))
            .await
            .unwrap();

        let outcome = request_and_serve(&svc, task.id).await;

        let ServedDispatch::Dispatched(result) = outcome else {
            panic!("expected Dispatched, got {outcome:?}");
        };
        let prompt =
            std::fs::read_to_string(format!("{}/.claude-prompt", result.worktree_path)).unwrap();
        assert!(
            prompt.contains("Own-handle epic"),
            "prompt must carry the epic banner the prologue read for itself: {prompt}"
        );
    }
}
