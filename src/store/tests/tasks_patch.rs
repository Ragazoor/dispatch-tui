use super::*;
use crate::models::test_tmux_window;

// ---------------------------------------------------------------------------
// patch_struct! macro correctness — has_changes() and setter coverage
// ---------------------------------------------------------------------------

#[tokio::test]
async fn task_patch_default_has_no_changes() {
    assert!(!TaskPatch::default().has_changes());
}

#[tokio::test]
async fn task_patch_each_setter_marks_has_changes() {
    assert!(TaskPatch::new().status(TaskStatus::Running).has_changes());
    assert!(TaskPatch::new().plan_path(Some("p")).has_changes());
    assert!(TaskPatch::new().plan_path(None).has_changes());
    assert!(TaskPatch::new().title("t").has_changes());
    assert!(TaskPatch::new().description("d").has_changes());
    assert!(TaskPatch::new().repo_path("/r").has_changes());
    assert!(TaskPatch::new().worktree(Some("w")).has_changes());
    assert!(TaskPatch::new().worktree(None).has_changes());
    assert!(TaskPatch::new()
        .tmux_window(Some(&test_tmux_window("tw")))
        .has_changes());
    assert!(TaskPatch::new().tmux_window(None).has_changes());
    assert!(TaskPatch::new().sub_status(SubStatus::Active).has_changes());
    let url = crate::models::TaskUrl::new("u", crate::models::UrlType::Other);
    assert!(TaskPatch::new().url(Some(&url)).has_changes());
    assert!(TaskPatch::new().url(None).has_changes());
    assert!(TaskPatch::new().tag(Some(TaskTag::Bug)).has_changes());
    assert!(TaskPatch::new().tag(None).has_changes());
    assert!(TaskPatch::new().sort_order(Some(1)).has_changes());
    assert!(TaskPatch::new().sort_order(None).has_changes());
    assert!(TaskPatch::new().base_branch("main").has_changes());
    assert!(TaskPatch::new().external_id(Some("x")).has_changes());
    assert!(TaskPatch::new().external_id(None).has_changes());
    let labels: Vec<String> = vec!["x".into()];
    assert!(TaskPatch::new().labels(&labels).has_changes());
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

mod property_tests {
    use super::*;
    use proptest::prelude::*;

    /// A `'static` window for the `'static`-lifetime patch below — `TaskPatch`
    /// borrows its window, so a temporary would not outlive the returned patch.
    static PATCH_WINDOW: crate::models::TmuxWindow = crate::models::TmuxWindow::from_static("w");

    /// Build a `TaskPatch` with the subset of fields indicated by `bits`.
    /// Each bit (0-12) maps to one field in `has_changes()` order.
    fn taskpatch_from_bits(bits: u16) -> TaskPatch<'static> {
        let mut p = TaskPatch::new();
        if bits & (1 << 0) != 0 {
            p = p.status(crate::models::TaskStatus::Backlog);
        }
        if bits & (1 << 1) != 0 {
            p = p.plan_path(Some("plan.md"));
        }
        if bits & (1 << 2) != 0 {
            p = p.title("t");
        }
        if bits & (1 << 3) != 0 {
            p = p.description("d");
        }
        if bits & (1 << 4) != 0 {
            p = p.repo_path("/repo");
        }
        if bits & (1 << 5) != 0 {
            p = p.worktree(Some(".wt"));
        }
        if bits & (1 << 6) != 0 {
            p = p.tmux_window(Some(&PATCH_WINDOW));
        }
        if bits & (1 << 7) != 0 {
            p = p.sub_status(crate::models::SubStatus::Active);
        }
        if bits & (1 << 8) != 0 {
            static URL: std::sync::LazyLock<crate::models::TaskUrl> =
                std::sync::LazyLock::new(|| {
                    crate::models::TaskUrl::new(
                        "https://github.com/pr/1",
                        crate::models::UrlType::Pr,
                    )
                });
            p = p.url(Some(&URL));
        }
        if bits & (1 << 9) != 0 {
            p = p.tag(Some(crate::models::TaskTag::Bug));
        }
        if bits & (1 << 10) != 0 {
            p = p.sort_order(Some(1));
        }
        if bits & (1 << 11) != 0 {
            p = p.base_branch("main");
        }
        if bits & (1 << 12) != 0 {
            p = p.external_id(Some("ext-1"));
        }
        p
    }

    /// Build an `EpicPatch` with the subset of fields indicated by `bits`.
    /// Each bit (0-8) maps to one field in `has_changes()` order.
    fn epicpatch_from_bits(bits: u16) -> EpicPatch<'static> {
        let mut p = EpicPatch::new();
        if bits & (1 << 0) != 0 {
            p = p.title("epic title");
        }
        if bits & (1 << 1) != 0 {
            p = p.description("desc");
        }
        if bits & (1 << 2) != 0 {
            p = p.status(crate::models::TaskStatus::Running);
        }
        if bits & (1 << 3) != 0 {
            p = p.plan_path(Some("plan.md"));
        }
        if bits & (1 << 4) != 0 {
            p = p.sort_order(Some(1));
        }
        if bits & (1 << 5) != 0 {
            p = p.auto_dispatch(true);
        }
        if bits & (1 << 6) != 0 {
            p = p.feed_command(Some("cmd"));
        }
        if bits & (1 << 7) != 0 {
            p = p.feed_interval_secs(Some(60));
        }
        p
    }

    proptest! {
        #[test]
        fn taskpatch_has_changes_iff_any_field_set(bits in 0u16..8192) {
            let patch = taskpatch_from_bits(bits);
            prop_assert_eq!(patch.has_changes(), bits != 0);
        }

        #[test]
        fn epicpatch_has_changes_iff_any_field_set(bits in 0u16..256) {
            let patch = epicpatch_from_bits(bits);
            prop_assert_eq!(patch.has_changes(), bits != 0);
        }

        /// Applying a `TaskPatch` to a baseline task and re-reading should yield:
        /// - `Some(_)` patch fields → applied to the row
        /// - `None` patch fields   → preserved from baseline
        ///
        /// For nullable fields, `Some(Some(v))` writes `v` and `Some(None)` writes NULL.
        ///
        /// `status` and `sort_order` are exercised in dedicated property tests below
        /// because they have additional invariants (sub_status coupling, signed integer).
        #[test]
        fn taskpatch_roundtrip(
            title       in proptest::option::of("[a-zA-Z0-9 ]{1,32}"),
            description in proptest::option::of("[a-zA-Z0-9 ]{0,32}"),
            repo_path   in proptest::option::of("/[a-z]{1,16}"),
            base_branch in proptest::option::of("[a-z]{1,16}"),
            plan_path   in proptest::option::of(proptest::option::of("[a-z]{1,16}\\.md")),
            worktree    in proptest::option::of(proptest::option::of("/[a-z]{1,16}")),
            tmux_window in proptest::option::of(proptest::option::of("[a-z]{1,16}")),
            url         in proptest::option::of(proptest::option::of("https://x/[0-9]{1,4}")),
            external_id in proptest::option::of(proptest::option::of("[a-z]{1,16}")),
        ) {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async {
                let db = in_memory_db().await;
                let id = db
                    .create_task(CreateTaskRequest {
                        description: "baseline desc",
                        ..CreateTaskRequest::fixture("Baseline", "/baseline")
                    })
                    .await
                    .unwrap();
                let baseline = db.get_task(id).await.unwrap();

                // Generated as strings, then lifted to the typed window field:
                // `[a-z]{1,16}` is always a valid window name (non-empty, not a
                // pane id), so no generated value is dropped by the lift.
                let tmux_window: Option<Option<crate::models::TmuxWindow>> = tmux_window
                    .map(|inner| inner.map(|s| test_tmux_window(&s)));

                let mut p = TaskPatch::new();
                if let Some(t)  = title.as_deref()       { p = p.title(t); }
                if let Some(d)  = description.as_deref() { p = p.description(d); }
                if let Some(r)  = repo_path.as_deref()   { p = p.repo_path(r); }
                if let Some(bb) = base_branch.as_deref() { p = p.base_branch(bb); }
                if let Some(ref pp) = plan_path   { p = p.plan_path(pp.as_deref()); }
                if let Some(ref w)  = worktree    { p = p.worktree(w.as_deref()); }
                if let Some(ref tw) = tmux_window { p = p.tmux_window(tw.as_ref()); }
                // Map the generated string into a typed url (inferred type).
                let url_typed: Option<Option<crate::models::TaskUrl>> = url.as_ref().map(|inner| {
                    inner
                        .as_ref()
                        .map(|s| crate::models::TaskUrl::new(s.clone(), crate::models::UrlType::infer(s)))
                });
                if let Some(ref u)  = url_typed   { p = p.url(u.as_ref()); }
                if let Some(ref e)  = external_id { p = p.external_id(e.as_deref()); }

                db.patch_task(id, &p).await.unwrap();
                let after = db.get_task(id).await.unwrap();

                prop_assert_eq!(&after.title,       &title.unwrap_or(baseline.title));
                prop_assert_eq!(&after.description, &description.unwrap_or(baseline.description));
                prop_assert_eq!(&after.repo_path,   &repo_path.unwrap_or(baseline.repo_path));
                prop_assert_eq!(&after.base_branch, &base_branch.unwrap_or(baseline.base_branch));
                prop_assert_eq!(&after.plan_path,   &plan_path.unwrap_or(baseline.plan_path));
                prop_assert_eq!(&after.worktree,    &worktree.unwrap_or(baseline.worktree));
                prop_assert_eq!(&after.tmux_window, &tmux_window.unwrap_or(baseline.tmux_window));
                prop_assert_eq!(&after.url,         &url_typed.unwrap_or(baseline.url));
                prop_assert_eq!(&after.external_id, &external_id.unwrap_or(baseline.external_id));
                prop_assert_eq!(after.status,     baseline.status);
                prop_assert_eq!(after.sub_status, baseline.sub_status);
                Ok::<(), proptest::test_runner::TestCaseError>(())
            })?;
        }

        /// `sort_order` is `nullable i64` — round-trip both Some(v) and None separately.
        #[test]
        fn taskpatch_roundtrip_sort_order(
            sort_order in proptest::option::of(proptest::option::of(any::<i64>())),
        ) {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async {
                let db = in_memory_db().await;
                let id = db
                    .create_task(CreateTaskRequest {
                        description: "d",
                        sort_order: Some(42),
                        ..CreateTaskRequest::fixture("T", "/r")
                    })
                    .await
                    .unwrap();
                let baseline = db.get_task(id).await.unwrap();
                let mut p = TaskPatch::new();
                if let Some(so) = sort_order { p = p.sort_order(so); }
                db.patch_task(id, &p).await.unwrap();
                let after = db.get_task(id).await.unwrap();
                prop_assert_eq!(after.sort_order, sort_order.unwrap_or(baseline.sort_order));
                Ok::<(), proptest::test_runner::TestCaseError>(())
            })?;
        }

        /// Applying an `EpicPatch` to a baseline epic and re-reading should yield
        /// the same Some(_) ↔ field, None ↔ baseline contract as `TaskPatch`.
        #[test]
        fn epicpatch_roundtrip(
            title       in proptest::option::of("[a-zA-Z0-9 ]{1,32}"),
            description in proptest::option::of("[a-zA-Z0-9 ]{0,32}"),
            plan_path   in proptest::option::of(proptest::option::of("[a-z]{1,16}\\.md")),
            sort_order  in proptest::option::of(proptest::option::of(any::<i64>())),
            auto_dispatch in proptest::option::of(any::<bool>()),
            feed_command  in proptest::option::of(proptest::option::of("[a-z]{1,16}")),
            feed_interval in proptest::option::of(proptest::option::of(1i64..86_400)),
        ) {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async {
                let db = in_memory_db().await;
                let epic = db
                    .create_epic("Baseline epic", "baseline", None).await
                    .unwrap();
                let baseline = db.get_epic(epic.id).await.unwrap();

                let mut p = EpicPatch::new();
                if let Some(t)  = title.as_deref()       { p = p.title(t); }
                if let Some(d)  = description.as_deref() { p = p.description(d); }
                if let Some(ref pp) = plan_path { p = p.plan_path(pp.as_deref()); }
                if let Some(so) = sort_order    { p = p.sort_order(so); }
                if let Some(ad) = auto_dispatch { p = p.auto_dispatch(ad); }
                if let Some(ref fc) = feed_command  { p = p.feed_command(fc.as_deref()); }
                if let Some(fi) = feed_interval     { p = p.feed_interval_secs(fi); }

                db.patch_epic(epic.id, &p).await.unwrap();
                let after = db.get_epic(epic.id).await.unwrap();

                prop_assert_eq!(&after.title,         &title.unwrap_or(baseline.title));
                prop_assert_eq!(&after.description,   &description.unwrap_or(baseline.description));
                prop_assert_eq!(&after.plan_path,     &plan_path.unwrap_or(baseline.plan_path));
                prop_assert_eq!(after.sort_order,     sort_order.unwrap_or(baseline.sort_order));
                prop_assert_eq!(after.auto_dispatch,  auto_dispatch.unwrap_or(baseline.auto_dispatch));
                prop_assert_eq!(&after.feed_command,  &feed_command.unwrap_or(baseline.feed_command));
                prop_assert_eq!(after.feed_interval_secs, feed_interval.unwrap_or(baseline.feed_interval_secs));
                Ok::<(), proptest::test_runner::TestCaseError>(())
            })?;
        }
    }
}

#[tokio::test]
async fn create_task_wrap_up_mode_defaults_to_none() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest::fixture("T", "/repo"))
        .await
        .unwrap();
    let task = db.get_task(id).await.unwrap();
    assert_eq!(task.wrap_up_mode, None);
}

#[tokio::test]
async fn create_task_with_wrap_up_mode_rebase() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            wrap_up_mode: Some(WrapUpMode::Rebase),
            ..CreateTaskRequest::fixture("T", "/repo")
        })
        .await
        .unwrap();
    let task = db.get_task(id).await.unwrap();
    assert_eq!(task.wrap_up_mode, Some(WrapUpMode::Rebase));
}

#[tokio::test]
async fn patch_task_wrap_up_mode() {
    let db = in_memory_db().await;
    let task = create_task_returning(&db, "T", "", "/repo", None, TaskStatus::Backlog)
        .await
        .unwrap();
    assert_eq!(task.wrap_up_mode, None);

    // Set to Pr
    db.patch_task(
        task.id,
        &TaskPatch::new().wrap_up_mode(Some(WrapUpMode::Pr)),
    )
    .await
    .unwrap();
    let task = db.get_task(task.id).await.unwrap();
    assert_eq!(task.wrap_up_mode, Some(WrapUpMode::Pr));

    // Clear it
    db.patch_task(task.id, &TaskPatch::new().wrap_up_mode(None))
        .await
        .unwrap();
    let task = db.get_task(task.id).await.unwrap();
    assert_eq!(task.wrap_up_mode, None);
}

#[tokio::test]
async fn patch_auto_run_plan_true() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            description: "d",
            ..CreateTaskRequest::fixture("T", "/r")
        })
        .await
        .unwrap();
    db.patch_task(id, &TaskPatch::new().auto_run_plan(true))
        .await
        .unwrap();
    let task = db.get_task(id).await.expect("task should exist");
    assert!(task.auto_run_plan);
}

#[tokio::test]
async fn row_to_task_sub_status_none_string_maps_to_none_variant() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            description: "d",
            ..CreateTaskRequest::fixture("t", "/repo")
        })
        .await
        .unwrap();
    let task = db.get_task(id).await.unwrap();
    assert_eq!(task.sub_status, SubStatus::None);
}

#[tokio::test]
async fn row_to_task_base_branch_defaults_to_main() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            description: "d",
            ..CreateTaskRequest::fixture("t", "/repo")
        })
        .await
        .unwrap();
    let task = db.get_task(id).await.unwrap();
    assert_eq!(task.base_branch, "main");
}

// ---------------------------------------------------------------------------
// OwnedTaskPatch / OwnedCreateTaskRequest mirror parity
// ---------------------------------------------------------------------------

/// Every field in TaskPatch must survive the round-trip through OwnedTaskPatch
/// into the database.  This test catches any field that the From impl silently
/// drops from the DB write.
#[tokio::test]
async fn patch_task_all_fields_round_trip() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            description: "orig desc",
            status: TaskStatus::Running,
            ..CreateTaskRequest::fixture("original", "/orig")
        })
        .await
        .unwrap();

    let labels = vec!["lbl-a".to_string(), "lbl-b".to_string()];
    let ts_pre = chrono::Utc::now() - chrono::Duration::seconds(120);
    let ts_notif = chrono::Utc::now() - chrono::Duration::seconds(60);
    let patch_url = crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/99",
        crate::models::UrlType::Pr,
    );

    db.patch_task(
        id,
        &TaskPatch::new()
            .status(TaskStatus::Running)
            .sub_status(SubStatus::Active)
            .plan_path(Some("docs/my-plan.md"))
            .title("patched title")
            .description("patched desc")
            .repo_path("/patched/repo")
            .worktree(Some(".worktrees/1394"))
            .tmux_window(Some(&test_tmux_window("session:1394")))
            .url(Some(&patch_url))
            .tag(Some(TaskTag::Feature))
            .sort_order(Some(42))
            .base_branch("feature-branch")
            .external_id(Some("ext-xyz"))
            .labels(&labels)
            .last_pre_tool_use_at(Some(ts_pre))
            .last_notification_at(Some(ts_notif))
            .wrap_up_mode(Some(WrapUpMode::Pr)),
    )
    .await
    .unwrap();

    let task = db.get_task(id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Running, "status");
    assert_eq!(task.sub_status, SubStatus::Active, "sub_status");
    assert_eq!(
        task.plan_path.as_deref(),
        Some("docs/my-plan.md"),
        "plan_path"
    );
    assert_eq!(task.title, "patched title", "title");
    assert_eq!(task.description, "patched desc", "description");
    assert_eq!(task.repo_path, "/patched/repo", "repo_path");
    assert_eq!(
        task.worktree.as_deref(),
        Some(".worktrees/1394"),
        "worktree"
    );
    assert_eq!(
        task.tmux_window.as_ref().map(|w| w.as_str()),
        Some("session:1394"),
        "tmux_window"
    );
    assert_eq!(task.url, Some(patch_url), "url");
    assert_eq!(task.tag, Some(TaskTag::Feature), "tag");
    assert_eq!(task.sort_order, Some(42), "sort_order");
    assert_eq!(task.base_branch, "feature-branch", "base_branch");
    assert_eq!(task.external_id.as_deref(), Some("ext-xyz"), "external_id");
    assert_eq!(task.labels, labels, "labels");
    let stored_pre = task
        .last_pre_tool_use_at
        .expect("last_pre_tool_use_at written");
    assert!(
        (stored_pre - ts_pre).num_seconds().abs() <= 1,
        "last_pre_tool_use_at"
    );
    let stored_notif = task
        .last_notification_at
        .expect("last_notification_at written");
    assert!(
        (stored_notif - ts_notif).num_seconds().abs() <= 1,
        "last_notification_at"
    );
    assert_eq!(task.wrap_up_mode, Some(WrapUpMode::Pr), "wrap_up_mode");
}

#[tokio::test]
async fn create_task_persists_wrap_up_mode() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            description: "d",
            wrap_up_mode: Some(WrapUpMode::Rebase),
            ..CreateTaskRequest::fixture("T", "/r")
        })
        .await
        .unwrap();
    let task = db.get_task(id).await.unwrap();
    assert_eq!(task.wrap_up_mode, Some(WrapUpMode::Rebase));
}

#[tokio::test]
async fn mark_pr_learnings_gate_shown_sets_once() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            status: TaskStatus::Running,
            ..CreateTaskRequest::fixture("t", "/tmp/r")
        })
        .await
        .unwrap();

    // First call sets the flag -> true (block).
    assert!(db.mark_pr_learnings_gate_shown(id).await.unwrap());
    // Second call: already set -> false (allow).
    assert!(!db.mark_pr_learnings_gate_shown(id).await.unwrap());
}

#[tokio::test]
async fn mark_pr_learnings_gate_shown_missing_task_is_false() {
    let db = in_memory_db().await;
    assert!(!db
        .mark_pr_learnings_gate_shown(TaskId(999_999))
        .await
        .unwrap());
}

// -- The dispatch claim, on request rows (store-split M4) ---------------------
//
// Since M4 the claim IS the dispatch request (`store-split.allium:
// RequestDispatch`): one module transaction that claims the task and writes
// the row. These pin the claim's guarantees at the store, through
// `request_dispatch`, and the epic chain's ordering through the service that
// now picks its candidates (`TaskService::request_next_backlog_subtask`).

/// Helper: a subtask of `epic_id` in `status`, with an explicit `sort_order`.
async fn subtask(
    db: &Store,
    epic_id: EpicId,
    title: &str,
    status: TaskStatus,
    sort_order: Option<i64>,
) -> TaskId {
    db.create_task(CreateTaskRequest {
        status,
        epic_id: Some(epic_id),
        sort_order,
        ..CreateTaskRequest::fixture(title, "/tmp/r")
    })
    .await
    .unwrap()
}

/// Helper: a backlog subtask of `epic_id` carrying the phoenix flag.
async fn phoenix_subtask(
    db: &Store,
    epic_id: EpicId,
    title: &str,
    sort_order: Option<i64>,
) -> TaskId {
    db.create_task(CreateTaskRequest {
        epic_id: Some(epic_id),
        sort_order,
        phoenix: true,
        ..CreateTaskRequest::fixture(title, "/tmp/r")
    })
    .await
    .unwrap()
}

/// A request from the TUI key, with its mode named.
async fn request(db: &Store, id: TaskId) -> RequestDispatchAnswer {
    db.request_dispatch(
        id,
        crate::models::DispatchOrigin::User,
        Some(crate::models::DispatchMode::Dispatch),
    )
    .await
    .unwrap()
}

fn applied(answer: &RequestDispatchAnswer) -> bool {
    matches!(answer, RequestDispatchAnswer::Requested(_))
}

/// The service the epic chain claims through, over `db`.
fn chain(db: &std::sync::Arc<Store>) -> crate::service::TaskService {
    crate::service::TaskService::new(db.clone(), crate::process::MockProcessRunner::unused())
}

async fn next_in_chain(db: &std::sync::Arc<Store>, epic: EpicId) -> Option<TaskId> {
    chain(db)
        .request_next_backlog_subtask(epic)
        .await
        .unwrap()
        .map(|(task, _)| task.id)
}

#[tokio::test]
async fn the_chain_requests_the_lowest_sort_order_subtask() {
    let db = std::sync::Arc::new(in_memory_db().await);
    let epic = db.create_epic("E", "", None).await.unwrap();
    let third = subtask(&db, epic.id, "c", TaskStatus::Backlog, Some(30)).await;
    let first = subtask(&db, epic.id, "a", TaskStatus::Backlog, Some(10)).await;
    let second = subtask(&db, epic.id, "b", TaskStatus::Backlog, Some(20)).await;

    assert_eq!(next_in_chain(&db, epic.id).await, Some(first));
    for untouched in [second, third] {
        assert_eq!(
            db.get_task(untouched).await.unwrap().status,
            TaskStatus::Backlog,
            "only the selected row may be claimed"
        );
    }
}

/// The ordering key is `COALESCE(sort_order, id)` then `id`: a
/// null-sort_order subtask sorts by its own id, so it loses to an explicitly
/// lower sort_order and wins against a higher one, regardless of insertion
/// order.
#[tokio::test]
async fn the_chain_falls_back_to_id_when_sort_order_is_null() {
    let db = std::sync::Arc::new(in_memory_db().await);
    let epic = db.create_epic("E", "", None).await.unwrap();
    let unordered = subtask(&db, epic.id, "no sort_order", TaskStatus::Backlog, None).await;
    let above = subtask(&db, epic.id, "sorts after", TaskStatus::Backlog, Some(500)).await;
    let below = subtask(&db, epic.id, "sorts before", TaskStatus::Backlog, Some(0)).await;

    assert_eq!(
        next_in_chain(&db, epic.id).await,
        Some(below),
        "sort_order 0 must beat a null whose fallback key is its own id"
    );
    assert_eq!(
        next_in_chain(&db, epic.id).await,
        Some(unordered),
        "the null-sort_order subtask beats sort_order 500 via its id fallback"
    );
    assert_eq!(next_in_chain(&db, epic.id).await, Some(above));
    assert_eq!(next_in_chain(&db, epic.id).await, None);
}

#[tokio::test]
async fn the_chain_skips_non_backlog_subtasks() {
    let db = std::sync::Arc::new(in_memory_db().await);
    let epic = db.create_epic("E", "", None).await.unwrap();
    subtask(&db, epic.id, "running", TaskStatus::Running, Some(1)).await;
    subtask(&db, epic.id, "review", TaskStatus::Review, Some(2)).await;
    subtask(&db, epic.id, "done", TaskStatus::Done, Some(3)).await;
    let backlog = subtask(&db, epic.id, "backlog", TaskStatus::Backlog, Some(4)).await;

    assert_eq!(next_in_chain(&db, epic.id).await, Some(backlog));
}

/// `PhoenixIsNeverChained` (docs/specs/epics.allium): the chain passes OVER a
/// phoenix subtask and takes the next ordinary one behind it — and stops when
/// only phoenix ones remain, rather than looping.
#[tokio::test]
async fn the_chain_skips_phoenix_subtasks() {
    let db = std::sync::Arc::new(in_memory_db().await);
    let epic = db.create_epic("E", "", None).await.unwrap();
    let recurring = phoenix_subtask(&db, epic.id, "recurring", Some(1)).await;
    let ordinary = subtask(&db, epic.id, "ordinary", TaskStatus::Backlog, Some(2)).await;

    assert_eq!(
        next_in_chain(&db, epic.id).await,
        Some(ordinary),
        "the phoenix subtask sorts first but is not a candidate"
    );
    assert_eq!(
        db.get_task(recurring).await.unwrap().status,
        TaskStatus::Backlog,
        "and it is left in backlog, unclaimed"
    );
    assert_eq!(next_in_chain(&db, epic.id).await, None);
}

#[tokio::test]
async fn the_chain_ignores_other_epics_subtasks() {
    let db = std::sync::Arc::new(in_memory_db().await);
    let mine = db.create_epic("mine", "", None).await.unwrap();
    let other = db.create_epic("other", "", None).await.unwrap();
    let theirs = subtask(&db, other.id, "theirs", TaskStatus::Backlog, Some(1)).await;

    assert_eq!(next_in_chain(&db, mine.id).await, None);
    assert_eq!(
        db.get_task(theirs).await.unwrap().status,
        TaskStatus::Backlog
    );
}

/// The module refuses a foreign-owned subtask, and the chain moves on to the
/// next ordinary one behind it, leaving the foreign one untouched.
#[tokio::test]
async fn the_chain_moves_past_a_foreign_owned_subtask() {
    let db = std::sync::Arc::new(in_memory_db().await);
    let epic = db.create_epic("E", "", None).await.unwrap();
    let foreign = subtask(&db, epic.id, "foreign", TaskStatus::Backlog, Some(10)).await;
    db.patch_task(
        foreign,
        &TaskPatch::new()
            .worktree(Some("/elsewhere/wt"))
            .host(Some("some-other-machine")),
    )
    .await
    .unwrap();
    let next = subtask(&db, epic.id, "next", TaskStatus::Backlog, Some(20)).await;

    assert_eq!(next_in_chain(&db, epic.id).await, Some(next));
    assert_eq!(
        db.get_task(foreign).await.unwrap().status,
        TaskStatus::Backlog,
        "the foreign-owned subtask is left untouched, not claimed"
    );
}

/// THE RACE. Every candidate is refused — here, every one's worktree is on
/// another machine — so the loop offers each once, runs out, and reports an
/// empty backlog rather than claiming something it was refused.
#[tokio::test]
async fn a_chain_refused_on_every_candidate_requests_nothing() {
    let db = std::sync::Arc::new(in_memory_db().await);
    let epic = db.create_epic("E", "", None).await.unwrap();
    for (title, order) in [("a", 1), ("b", 2)] {
        let id = subtask(&db, epic.id, title, TaskStatus::Backlog, Some(order)).await;
        db.patch_task(
            id,
            &TaskPatch::new()
                .worktree(Some("/elsewhere/wt"))
                .host(Some("some-other-machine")),
        )
        .await
        .unwrap();
    }

    assert_eq!(next_in_chain(&db, epic.id).await, None);
    assert!(db.dispatch_requests().await.is_empty());
}

/// The phoenix skip belongs to the CHAIN, not to dispatch. `PhoenixIsNeverChained`
/// stops an epic launching agents at a recurring task on its own; it does not
/// stop a human doing it, which is the entire point of the flag. The request
/// reducer accepts a phoenix task.
#[tokio::test]
async fn a_phoenix_task_the_chain_would_skip_can_be_requested_by_hand() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let id = phoenix_subtask(&db, epic.id, "recurring", Some(1)).await;

    assert!(applied(&request(&db, id).await));
    assert_eq!(db.get_task(id).await.unwrap().status, TaskStatus::Running);
    assert!(
        db.get_task(id).await.unwrap().phoenix,
        "dispatching does not consume the flag; only entering Done does"
    );
}

#[tokio::test]
async fn a_request_applies_the_full_claim_and_names_this_host() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let id = subtask(&db, epic.id, "target", TaskStatus::Backlog, Some(1)).await;

    let RequestDispatchAnswer::Requested(request_id) = request(&db, id).await else {
        panic!("the request must apply");
    };

    let claimed = db.get_task(id).await.unwrap();
    assert_eq!(claimed.status, TaskStatus::Running);
    assert_eq!(
        claimed.sub_status,
        SubStatus::default_for(TaskStatus::Running)
    );
    assert!(claimed.last_pre_tool_use_at.is_some());
    assert!(
        claimed.worktree.is_none(),
        "the claim runs ahead of provisioning"
    );
    let row = db.dispatch_request(request_id).await.unwrap();
    assert_eq!(
        row.host, MEMORY_HOST_ID,
        "the requester is this handle's host"
    );
}

#[tokio::test]
async fn a_request_for_a_task_out_of_backlog_writes_nothing() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let id = subtask(&db, epic.id, "running", TaskStatus::Running, Some(1)).await;

    assert!(!applied(&request(&db, id).await));
    assert!(
        db.get_task(id)
            .await
            .unwrap()
            .last_pre_tool_use_at
            .is_none(),
        "a refused request writes nothing at all — one transaction, so it cannot half-apply"
    );
    assert!(db.latest_dispatch_request_for_task(id).await.is_none());
}

#[tokio::test]
async fn a_request_for_a_missing_task_is_refused() {
    let db = in_memory_db().await;
    assert!(!applied(&request(&db, TaskId(999_999)).await));
}

#[tokio::test]
async fn a_task_is_requested_at_most_once() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let id = subtask(&db, epic.id, "target", TaskStatus::Backlog, Some(1)).await;

    assert!(applied(&request(&db, id).await));
    assert!(
        !applied(&request(&db, id).await),
        "the row has left Backlog, so a second request for it must be refused"
    );
}

// -- host gating on the request (task #4812 distributed-dispatch foundations) --
//
// `DispatchTask`'s `requires: task.is_locally_owned` (docs/specs/dispatch.allium)
// is the module's `RequestDispatch` check `task.host = null or task.host =
// requester`.

#[tokio::test]
async fn a_task_with_no_host_can_be_requested() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let id = subtask(&db, epic.id, "target", TaskStatus::Backlog, Some(1)).await;

    // A never-dispatched task has `host = NULL`, so the gate is a no-op —
    // `is_locally_owned`'s first arm (core/Task in docs/specs/core.allium).
    assert!(applied(&request(&db, id).await));
}

#[tokio::test]
async fn a_foreign_owned_task_is_refused() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let id = subtask(&db, epic.id, "target", TaskStatus::Backlog, Some(1)).await;
    db.patch_task(
        id,
        &TaskPatch::new()
            .worktree(Some("/elsewhere/wt"))
            .host(Some("some-other-machine")),
    )
    .await
    .unwrap();

    let answer = request(&db, id).await;
    assert!(
        matches!(&answer, RequestDispatchAnswer::Refused(why) if why.contains("some-other-machine")),
        "another machine holds this task's worktree; requesting it here would \
         re-dispatch onto a directory that is not on this disk: {answer:?}"
    );
    let untouched = db.get_task(id).await.unwrap();
    assert_eq!(
        untouched.status,
        TaskStatus::Backlog,
        "a refused request writes nothing — one transaction, so it cannot half-apply"
    );
    assert_eq!(untouched.host.as_deref(), Some("some-other-machine"));
}

// -- ReleaseClaimOfFailedRequest -------------------------------------------------
//
// No client releases a claim any more; a FAILED request does, in the module,
// conditionally. Pinned here through the host's failure report.

/// Helper: a backlog task, requested and claimed by this host, ready to fail.
async fn provisioning_task(db: &Store) -> (TaskId, crate::models::DispatchRequestId) {
    let epic = db.create_epic("E", "", None).await.unwrap();
    let id = subtask(db, epic.id, "t", TaskStatus::Backlog, None).await;
    let RequestDispatchAnswer::Requested(request_id) = request(db, id).await else {
        panic!("the request must apply");
    };
    assert!(db.claim_dispatch_request(request_id).await.unwrap());
    (id, request_id)
}

#[tokio::test]
async fn a_failed_request_releases_an_unprovisioned_claim() {
    let db = in_memory_db().await;
    let (id, request_id) = provisioning_task(&db).await;

    assert!(db.report_dispatch_failed(request_id, "boom").await.unwrap());
    let released = db.get_task(id).await.unwrap();
    assert_eq!(released.status, TaskStatus::Backlog);
    assert_eq!(
        released.sub_status,
        SubStatus::default_for(TaskStatus::Backlog)
    );
    // The stamp the claim seeded is cleared, or the task is not quite "as it
    // was before the dispatch fired".
    assert!(released.last_pre_tool_use_at.is_none());
}

#[tokio::test]
async fn a_failed_request_spares_a_provisioned_task() {
    let db = in_memory_db().await;
    let (id, request_id) = provisioning_task(&db).await;
    // A worktree landed on the task meanwhile.
    db.patch_task(
        id,
        &TaskPatch::new()
            .worktree(Some("/tmp/wt"))
            .host(Some(MEMORY_HOST_ID)),
    )
    .await
    .unwrap();

    db.report_dispatch_failed(request_id, "late").await.unwrap();

    let kept = db.get_task(id).await.unwrap();
    assert_eq!(kept.status, TaskStatus::Running);
    assert_eq!(kept.worktree.as_deref(), Some("/tmp/wt"));
}

#[tokio::test]
async fn a_failed_request_spares_a_task_moved_out_of_running() {
    let db = in_memory_db().await;
    let (id, request_id) = provisioning_task(&db).await;
    // A human moved it on while provisioning was still in flight.
    db.patch_task(id, &TaskPatch::new().status(TaskStatus::Review))
        .await
        .unwrap();

    db.report_dispatch_failed(request_id, "late").await.unwrap();
    assert_eq!(db.get_task(id).await.unwrap().status, TaskStatus::Review);
}

#[tokio::test]
async fn batch_patch_sub_status_updates_all_tasks() {
    let db = in_memory_db().await;
    let t1 = db
        .create_task(CreateTaskRequest {
            status: TaskStatus::Running,
            ..CreateTaskRequest::fixture("A", "/r")
        })
        .await
        .unwrap();
    let t2 = db
        .create_task(CreateTaskRequest {
            status: TaskStatus::Running,
            ..CreateTaskRequest::fixture("B", "/r")
        })
        .await
        .unwrap();

    db.batch_patch_sub_status(&[(t1, SubStatus::Stale), (t2, SubStatus::NeedsInput)])
        .await
        .unwrap();

    assert_eq!(db.get_task(t1).await.unwrap().sub_status, SubStatus::Stale);
    assert_eq!(
        db.get_task(t2).await.unwrap().sub_status,
        SubStatus::NeedsInput
    );
}

#[tokio::test]
async fn batch_patch_sub_status_empty_is_no_op() {
    let db = in_memory_db().await;
    // Should not error on empty input.
    db.batch_patch_sub_status(&[]).await.unwrap();
}
