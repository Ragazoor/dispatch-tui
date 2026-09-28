//! `MemoryReducerCaller` behaves like the real reducers, for the domains this
//! build covers.
//!
//! Spec: `docs/specs/spacetime-memory-store.allium`'s `ReducerConformance`
//! contract and `StandInFidelity` surface (`ConformanceIsCiGated`).
//!
//! Mirrors the shape `tests/spacetime_module.rs` and
//! `src/spacetime/tests/module_schema.rs` already use to diff the real module
//! against SQLite: stand up a throwaway SpacetimeDB instance, publish the real
//! module into it, then drive the SAME sequence of [`ReducerCaller`] calls —
//! literally the same trait, the same method, the same argument values —
//! against [`SdkReducerCaller`] (the real store) and against
//! [`MemoryReducerCaller`], and assert the two land on the same `SharedRows`
//! state. Sending one `bindings::Task`/`bindings::Epic` value to both callers
//! is what rules out the row itself drifting between the two arms; only the
//! two callers' OWN behaviour is under test.
//!
//! **What this does and does not prove.** `MemoryReducerCaller` calls the
//! module's own pure helpers directly (`derive_epic_status`,
//! `stamps_completion`, `apply_task_patch`, `apply_epic_patch`,
//! `validate_task_ownership`) — for those, "the same function" makes
//! agreement structural rather than something a test needs to establish. What
//! is NOT guaranteed by construction is everything this file's own
//! reimplementation invents by hand: id generation, row storage, delete
//! cascades, claim/release exclusivity (which side refuses, not its exact
//! refusal text — see `TaskShape`/`EpicShape` below for what row state IS
//! compared field-for-field), and upsert-by-key semantics for repo config and
//! subscriptions. This suite's scenario is chosen to exercise exactly that
//! surface.
//!
//! **Skipped when `spacetime` is not on `PATH`.** CI's Test job installs and
//! pins it, hard-failing the job rather than letting the install silently
//! fail — see `tests/spacetime_module.rs`'s own header for the full picture,
//! including why the Coverage job still takes this skip.
//!
//! **tasks_and_epics/repo_config/subscriptions only (task #4975).** Extend
//! this same file's scenario, rather than starting a new one, as each later
//! work package (#5002 settings, #5003 learnings, #5004 usage/agent_state)
//! lands — `spacetime-memory-store.allium`'s `ConformanceIsCiGated` guarantee.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dispatch_tui::models::{EpicId, TaskId, TaskStatus};
use dispatch_tui::service::{Clock, SystemClock};
use dispatch_tui::spacetime::bindings;
use dispatch_tui::sync::{
    MemoryReducerCaller, ReducerCaller, SdkReducerCaller, SettledIdentity, SharedRows,
    SpacetimeSdkConnector, StoreConnector, SubscriptionRequest,
};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const DATABASE_PREFIX: &str = "dispatch-memory-caller-conformance";

static NEXT_DATABASE_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn spacetime_available_or_skip() -> bool {
    let present = Command::new("spacetime")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !present {
        eprintln!("skipping: spacetime not available on PATH");
    }
    present
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    listener.local_addr().expect("read the bound port").port()
}

/// A trimmed copy of `tests/spacetime_module.rs`'s own `Instance` — each file
/// under `tests/` compiles as its own binary, so there is no shared module to
/// import this from. This keeps only what standing up and publishing into a
/// throwaway instance needs; every reducer call and row read in this file
/// goes through [`ReducerCaller`]/`SharedRows` instead of the CLI, so the
/// `call`/`sql` helpers that copy has do not need to be repeated here.
struct Instance {
    child: Child,
    port: u16,
    dir: tempfile::TempDir,
    database: String,
}

impl Instance {
    fn start() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let port = free_port();
        let child = Command::new("spacetime")
            .arg(format!(
                "--config-path={}",
                dir.path().join("cli.toml").display()
            ))
            .arg("start")
            .arg(format!("--listen-addr=127.0.0.1:{port}"))
            .arg(format!("--data-dir={}", dir.path().join("data").display()))
            .arg("--non-interactive")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn spacetime start");

        let database = format!(
            "{DATABASE_PREFIX}-{}",
            NEXT_DATABASE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let mut instance = Instance {
            child,
            port,
            dir,
            database,
        };
        instance.await_ready();
        assert!(
            instance
                .child
                .try_wait()
                .expect("poll the instance process")
                .is_none(),
            "this test's spacetime instance exited during startup — most likely \
             another test won the race for port {}",
            instance.port
        );
        instance
    }

    fn await_ready(&self) {
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        loop {
            if self.answers_http() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the test instance never served HTTP on port {}",
                self.port
            );
            // allow-test-sleep: deadline-bounded poll, not a fixed wait.
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn answers_http(&self) -> bool {
        use std::io::{Read, Write};
        let Ok(mut stream) = std::net::TcpStream::connect(("127.0.0.1", self.port)) else {
            return false;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let request = format!(
            "GET /v1/identity HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
            self.port
        );
        if stream.write_all(request.as_bytes()).is_err() {
            return false;
        }
        let mut answer = Vec::new();
        matches!(stream.read_to_end(&mut answer), Ok(n) if n > 0) && answer.starts_with(b"HTTP/")
    }

    fn database(&self) -> &str {
        &self.database
    }

    fn host(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn config_arg(&self) -> String {
        format!(
            "--config-path={}",
            self.dir.path().join("cli.toml").display()
        )
    }

    fn publish(&self, module_path: &Path) -> std::process::Output {
        run(&[
            &self.config_arg(),
            "publish",
            "-p",
            &module_path.display().to_string(),
            "-s",
            &self.host(),
            "-y",
            "--delete-data=never",
            self.database(),
        ])
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new("spacetime")
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("running `spacetime {}`: {e}", args.join(" ")))
}

fn describe(out: &std::process::Output) -> String {
    format!(
        "status {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn module_path() -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "spacetime", "module"]
        .iter()
        .collect()
}

// ---------------------------------------------------------------------------
// Row fixtures — one `bindings::Task`/`bindings::Epic`/`bindings::TaskPatch`
// value per step, sent to BOTH callers, so the row itself cannot drift
// between the two arms. Mirrors the module's own private `blank_task`/
// `blank_epic` defaults (spacetime/module/src/lib.rs).
// ---------------------------------------------------------------------------

fn blank_epic() -> bindings::Epic {
    bindings::Epic {
        id: 0,
        title: "conformance epic".into(),
        description: String::new(),
        status: "backlog".into(),
        plan_path: String::new(),
        sort_order: None,
        created_at: "2026-01-01 00:00:00.000".into(),
        updated_at: "2026-01-01 00:00:00.000".into(),
        auto_dispatch: false,
        parent_epic_id: 0,
        feed_command: String::new(),
        feed_interval_secs: 0,
        group_by_repo: false,
        feed_role: "none".into(),
        origin: "manual".into(),
        feed_append_only: false,
        completed_at: String::new(),
        created_by: String::new(),
    }
}

fn blank_task_in_epic(epic_id: i64) -> bindings::Task {
    bindings::Task {
        id: 0,
        title: "conformance task".into(),
        description: String::new(),
        repo_path: "/repo".into(),
        status: "backlog".into(),
        worktree: String::new(),
        tmux_window: String::new(),
        plan_path: String::new(),
        epic_id,
        sub_status: "none".into(),
        tag: String::new(),
        sort_order: None,
        created_at: "2026-01-01 00:00:00.000".into(),
        updated_at: "2026-01-01 00:00:00.000".into(),
        base_branch: "main".into(),
        external_id: String::new(),
        labels: "[]".into(),
        last_pre_tool_use_at: String::new(),
        last_notification_at: String::new(),
        wrap_up_mode: String::new(),
        url: String::new(),
        url_type: String::new(),
        pr_learnings_gate_shown_at: String::new(),
        auto_run_plan: false,
        live_subagents: 0,
        stop_pending: false,
        stop_pending_at: String::new(),
        live_shells: 0,
        oldest_live_shell_started_at: String::new(),
        last_peer_message_sent_at: String::new(),
        last_peer_message_received_at: String::new(),
        phoenix: false,
        host: String::new(),
        owner: String::new(),
        completed_at: String::new(),
        created_by: String::new(),
    }
}

fn blank_task_patch() -> bindings::TaskPatch {
    bindings::TaskPatch {
        title: None,
        description: None,
        repo_path: None,
        status: None,
        worktree: None,
        tmux_window: None,
        plan_path: None,
        epic_id: None,
        sub_status: None,
        tag: None,
        sort_order: None,
        base_branch: None,
        external_id: None,
        labels: None,
        last_pre_tool_use_at: None,
        last_notification_at: None,
        wrap_up_mode: None,
        url: None,
        url_type: None,
        pr_learnings_gate_shown_at: None,
        auto_run_plan: None,
        live_subagents: None,
        stop_pending: None,
        stop_pending_at: None,
        last_peer_message_sent_at: None,
        last_peer_message_received_at: None,
        phoenix: None,
        host: None,
        owner: None,
        completed_at: None,
    }
}

fn blank_epic_patch() -> bindings::EpicPatch {
    bindings::EpicPatch {
        title: None,
        description: None,
        status: None,
        plan_path: None,
        sort_order: None,
        auto_dispatch: None,
        parent_epic_id: None,
        feed_command: None,
        feed_interval_secs: None,
        group_by_repo: None,
        feed_role: None,
        origin: None,
        feed_append_only: None,
        completed_at: None,
    }
}

/// A projection of `crate::models::Task` that drops exactly the two fields a
/// real store's own clock and this test's `SystemClock` will never agree on
/// bit-for-bit — `updated_at` (stamped by every write) and
/// `last_pre_tool_use_at` (stamped by `claim_backlog_task`, which this
/// scenario exercises) — each reduced to presence like `completed_at`
/// already was. Every other field of `crate::models::Task` is compared
/// exactly, per `ReducerConformance.SameEndState`'s "same resulting row
/// state": including the ones nothing in this scenario writes on either
/// side (`tmux_window`, `url`, `wrap_up_mode`, `last_notification_at`,
/// `last_peer_message_sent_at`/`last_peer_message_received_at`) costs
/// nothing — both sides hold whatever `blank_task_in_epic` gave them — and
/// would still catch either side spuriously stamping one. (`live_shells`,
/// `oldest_live_shell_started_at` and `stop_pending_at` aren't part of
/// `crate::models::Task` at all — they're bindings/DB-only fields the board
/// read model doesn't surface — so there is nothing here to compare them
/// against.)
#[derive(Debug, PartialEq)]
struct TaskShape {
    title: String,
    description: String,
    repo_path: String,
    status: TaskStatus,
    sub_status: dispatch_tui::models::SubStatus,
    epic_id: Option<i64>,
    host: Option<String>,
    worktree: Option<String>,
    tmux_window: Option<dispatch_tui::models::TmuxWindow>,
    plan_path: Option<String>,
    url: Option<dispatch_tui::models::TaskUrl>,
    tag: Option<dispatch_tui::models::TaskTag>,
    sort_order: Option<i64>,
    base_branch: String,
    external_id: Option<String>,
    labels: Vec<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    last_notification_at: Option<chrono::DateTime<chrono::Utc>>,
    last_peer_message_sent_at: Option<chrono::DateTime<chrono::Utc>>,
    last_peer_message_received_at: Option<chrono::DateTime<chrono::Utc>>,
    wrap_up_mode: Option<dispatch_tui::models::WrapUpMode>,
    auto_run_plan: bool,
    phoenix: bool,
    live_subagents: i64,
    stop_pending: bool,
    has_completed_at: bool,
    has_last_pre_tool_use_at: bool,
}

fn task_shape(t: &dispatch_tui::models::Task) -> TaskShape {
    TaskShape {
        title: t.title.clone(),
        description: t.description.clone(),
        repo_path: t.repo_path.clone(),
        status: t.status,
        sub_status: t.sub_status,
        epic_id: t.epic_id.map(|e| e.0),
        host: t.host.clone(),
        worktree: t.worktree.clone(),
        tmux_window: t.tmux_window.clone(),
        plan_path: t.plan_path.clone(),
        url: t.url.clone(),
        tag: t.tag,
        sort_order: t.sort_order,
        base_branch: t.base_branch.clone(),
        external_id: t.external_id.clone(),
        labels: t.labels.clone(),
        created_at: t.created_at,
        last_notification_at: t.last_notification_at,
        last_peer_message_sent_at: t.last_peer_message_sent_at,
        last_peer_message_received_at: t.last_peer_message_received_at,
        wrap_up_mode: t.wrap_up_mode,
        auto_run_plan: t.auto_run_plan,
        phoenix: t.phoenix,
        live_subagents: t.live_subagents,
        stop_pending: t.stop_pending,
        has_completed_at: t.completed_at.is_some(),
        has_last_pre_tool_use_at: t.last_pre_tool_use_at.is_some(),
    }
}

/// The `Epic` twin of `TaskShape`: drops `updated_at` (clock-stamped by every
/// write) and reduces `completed_at` to presence, on the same reasoning.
/// Every other field this scenario's fixtures populate is compared exactly.
#[derive(Debug, PartialEq)]
struct EpicShape {
    title: String,
    description: String,
    status: TaskStatus,
    plan_path: Option<String>,
    sort_order: Option<i64>,
    parent_epic_id: Option<i64>,
    auto_dispatch: bool,
    feed_command: Option<String>,
    feed_interval_secs: Option<i64>,
    group_by_repo: bool,
    feed_append_only: bool,
    feed_role: dispatch_tui::models::FeedRole,
    origin: dispatch_tui::models::EpicOrigin,
    created_at: chrono::DateTime<chrono::Utc>,
    has_completed_at: bool,
}

fn epic_shape(e: &dispatch_tui::models::Epic) -> EpicShape {
    EpicShape {
        title: e.title.clone(),
        description: e.description.clone(),
        status: e.status,
        plan_path: e.plan_path.clone(),
        sort_order: e.sort_order,
        parent_epic_id: e.parent_epic_id.map(|p| p.0),
        auto_dispatch: e.auto_dispatch,
        feed_command: e.feed_command.clone(),
        feed_interval_secs: e.feed_interval_secs,
        group_by_repo: e.group_by_repo,
        feed_append_only: e.feed_append_only,
        feed_role: e.feed_role,
        origin: e.origin,
        created_at: e.created_at,
        has_completed_at: e.completed_at.is_some(),
    }
}

/// The orchestration-fidelity scenario: create two epics and a task inside
/// the first, claim and release the task (including the worktree-attached
/// refusal), patch the epic, move the task to the second epic and back
/// (`set_task_epic`, exercising the epic-chain recalculation on both the
/// source and destination), round-trip repo config, round-trip a
/// subscription, then delete the task and both epics — asserting after every
/// step that the real store and `MemoryReducerCaller` agree.
#[test]
fn memory_caller_matches_the_real_reducers() {
    if !spacetime_available_or_skip() {
        return;
    }

    let instance = Instance::start();
    let published = instance.publish(&module_path());
    assert!(published.status.success(), "{}", describe(&published));

    let rows_real = Arc::new(SharedRows::new());
    let rows_mem = Arc::new(SharedRows::new());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let mem = MemoryReducerCaller::new(rows_mem.clone(), clock);

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
        // `owner_board` must be non-empty hex (it becomes a SQL literal
        // matched against `owner`/`created_by`); this scenario never relies on
        // those two subscriptions, so any valid-looking hex string does.
        connector
            .subscribe(&SubscriptionRequest::new(
                accepted.identity.clone(),
                vec![1, 2],
                "host-a",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));
        let real = SdkReducerCaller::new(connector, Arc::new(SettledIdentity::default()));

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

        let compare_task = |id: i64| {
            assert_eq!(
                rows_real.task(TaskId(id)).map(|t| task_shape(&t)),
                rows_mem.task(TaskId(id)).map(|t| task_shape(&t)),
                "task {id}"
            );
        };
        let compare_epic = |id: i64| {
            assert_eq!(
                rows_real.epic(EpicId(id)).map(|e| epic_shape(&e)),
                rows_mem.epic(EpicId(id)).map(|e| epic_shape(&e)),
                "epic {id}"
            );
        };

        // -- create_epic (two: the task's home, and a destination to move it to) --
        let epic_id_real = real.create_epic(blank_epic()).await.unwrap();
        wait_for!(rows_real.epic(EpicId(epic_id_real)).is_some());
        let epic_id_mem = mem.create_epic(blank_epic()).await.unwrap();
        assert_eq!(epic_id_real, epic_id_mem, "generated epic id");
        compare_epic(epic_id_real);

        let epic2_id_real = real.create_epic(blank_epic()).await.unwrap();
        wait_for!(rows_real.epic(EpicId(epic2_id_real)).is_some());
        let epic2_id_mem = mem.create_epic(blank_epic()).await.unwrap();
        assert_eq!(epic2_id_real, epic2_id_mem, "generated second epic id");
        compare_epic(epic2_id_real);

        // -- create_task (in the epic, backlog) ----------------------------------
        let task_id_real = real
            .create_task(blank_task_in_epic(epic_id_real))
            .await
            .unwrap();
        wait_for!(rows_real.task(task_id_real).is_some());
        let task_id_mem = mem
            .create_task(blank_task_in_epic(epic_id_mem))
            .await
            .unwrap();
        assert_eq!(task_id_real, task_id_mem, "generated task id");
        compare_task(task_id_real.0);
        compare_epic(epic_id_real);

        // -- claim_backlog_task ---------------------------------------------------
        let real_claim = real
            .claim_backlog_task(task_id_real, "host-a".into())
            .await
            .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.status == TaskStatus::Running));
        let mem_claim = mem
            .claim_backlog_task(task_id_mem, "host-a".into())
            .await
            .unwrap();
        assert_eq!(real_claim.won(), mem_claim.won(), "claim outcome");
        compare_task(task_id_real.0);

        // -- release_backlog_claim refuses while a worktree is attached ------------
        let attach_worktree = || bindings::TaskPatch {
            worktree: Some("/tmp/conformance".into()),
            ..blank_task_patch()
        };
        real.patch_task(task_id_real, attach_worktree())
            .await
            .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.worktree.as_deref() == Some("/tmp/conformance")));
        mem.patch_task(task_id_mem, attach_worktree())
            .await
            .unwrap();
        compare_task(task_id_real.0);

        let real_release = real.release_backlog_claim(task_id_real).await.unwrap();
        let mem_release = mem.release_backlog_claim(task_id_mem).await.unwrap();
        assert!(
            !real_release.won(),
            "real must refuse with a worktree attached"
        );
        assert!(
            !mem_release.won(),
            "mem must refuse with a worktree attached"
        );
        compare_task(task_id_real.0);

        // Clear the worktree so release can actually apply, on both sides.
        let clear_worktree = || bindings::TaskPatch {
            worktree: Some(String::new()),
            ..blank_task_patch()
        };
        real.patch_task(task_id_real, clear_worktree())
            .await
            .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.worktree.is_none()));
        mem.patch_task(task_id_mem, clear_worktree()).await.unwrap();
        compare_task(task_id_real.0);

        let real_release = real.release_backlog_claim(task_id_real).await.unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.status == TaskStatus::Backlog));
        let mem_release = mem.release_backlog_claim(task_id_mem).await.unwrap();
        assert!(
            real_release.won() && mem_release.won(),
            "release must apply now"
        );
        compare_task(task_id_real.0);

        // -- patch_epic -------------------------------------------------------------
        real.patch_epic(
            epic_id_real,
            bindings::EpicPatch {
                title: Some("renamed conformance epic".into()),
                ..blank_epic_patch()
            },
        )
        .await
        .unwrap();
        wait_for!(rows_real
            .epic(EpicId(epic_id_real))
            .is_some_and(|e| e.title == "renamed conformance epic"));
        mem.patch_epic(
            epic_id_mem,
            bindings::EpicPatch {
                title: Some("renamed conformance epic".into()),
                ..blank_epic_patch()
            },
        )
        .await
        .unwrap();
        compare_epic(epic_id_real);

        // -- set_task_epic: move the (backlog) task to the second epic --------------
        real.set_task_epic(task_id_real, epic2_id_real, String::new())
            .await
            .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.epic_id == Some(EpicId(epic2_id_real))));
        mem.set_task_epic(task_id_mem, epic2_id_mem, String::new())
            .await
            .unwrap();
        compare_task(task_id_real.0);
        compare_epic(epic_id_real);
        compare_epic(epic2_id_real);

        // -- recalculate_epic_status is idempotent when nothing changed -------------
        real.recalculate_epic_status(epic2_id_real).await.unwrap();
        mem.recalculate_epic_status(epic2_id_mem).await.unwrap();
        compare_epic(epic2_id_real);

        // Move it back, so the delete section below only has one epic's worth
        // of tasks to clean up.
        real.set_task_epic(task_id_real, epic_id_real, String::new())
            .await
            .unwrap();
        wait_for!(rows_real
            .task(task_id_real)
            .is_some_and(|t| t.epic_id == Some(EpicId(epic_id_real))));
        mem.set_task_epic(task_id_mem, epic_id_mem, String::new())
            .await
            .unwrap();
        compare_task(task_id_real.0);
        compare_epic(epic_id_real);
        compare_epic(epic2_id_real);

        // -- repo configuration ---------------------------------------------------
        real.save_repo_path("/repo".into(), "2026-01-01 00:00:00.000".into())
            .await
            .unwrap();
        wait_for!(!rows_real.repo_paths().is_empty());
        mem.save_repo_path("/repo".into(), "2026-01-01 00:00:00.000".into())
            .await
            .unwrap();
        assert_eq!(rows_real.repo_paths(), rows_mem.repo_paths(), "repo_paths");

        real.set_verify_command("/repo".into(), "cargo test".into())
            .await
            .unwrap();
        wait_for!(rows_real.verify_command("/repo").as_deref() == Some("cargo test"));
        mem.set_verify_command("/repo".into(), "cargo test".into())
            .await
            .unwrap();
        assert_eq!(
            rows_real.verify_command("/repo"),
            rows_mem.verify_command("/repo"),
            "verify_command"
        );

        real.record_base_branch(
            "/repo".into(),
            "main".into(),
            "2026-01-01 00:00:00.000".into(),
        )
        .await
        .unwrap();
        wait_for!(!rows_real.base_branches().is_empty());
        mem.record_base_branch(
            "/repo".into(),
            "main".into(),
            "2026-01-01 00:00:00.000".into(),
        )
        .await
        .unwrap();
        assert_eq!(
            rows_real.base_branches(),
            rows_mem.base_branches(),
            "base_branches"
        );

        real.delete_repo_path("/repo".into()).await.unwrap();
        wait_for!(rows_real.repo_paths().is_empty());
        mem.delete_repo_path("/repo".into()).await.unwrap();
        assert!(rows_mem.repo_paths().is_empty());

        // -- subscriptions ----------------------------------------------------------
        // The connection's OWN identity, not an arbitrary hex string: this
        // connection's subscription asks for `subscriptions WHERE subscriber =
        // '{accepted.identity}'` (`subscription_queries`), so a row filed
        // under any other subscriber would never arrive here to compare.
        let subscriber = accepted.identity.as_str();
        real.subscribe_to_epic(subscriber.into(), epic_id_real)
            .await
            .unwrap();
        wait_for!(rows_real.subscribed_epics(subscriber) == vec![epic_id_real]);
        mem.subscribe_to_epic(subscriber.into(), epic_id_mem)
            .await
            .unwrap();
        assert_eq!(
            rows_real.subscribed_epics(subscriber),
            rows_mem.subscribed_epics(subscriber),
            "subscribed_epics after subscribe"
        );

        real.unsubscribe_from_epic(subscriber.into(), epic_id_real)
            .await
            .unwrap();
        wait_for!(rows_real.subscribed_epics(subscriber).is_empty());
        mem.unsubscribe_from_epic(subscriber.into(), epic_id_mem)
            .await
            .unwrap();
        assert_eq!(
            rows_real.subscribed_epics(subscriber),
            rows_mem.subscribed_epics(subscriber),
            "subscribed_epics after unsubscribe"
        );

        // -- delete_task, then delete_epic ---------------------------------------
        real.delete_task(task_id_real).await.unwrap();
        wait_for!(rows_real.task(task_id_real).is_none());
        mem.delete_task(task_id_mem).await.unwrap();
        assert!(rows_mem.task(task_id_mem).is_none());
        compare_epic(epic_id_real);

        real.delete_epic(epic_id_real).await.unwrap();
        wait_for!(rows_real.epic(EpicId(epic_id_real)).is_none());
        mem.delete_epic(epic_id_mem).await.unwrap();
        assert!(rows_mem.epic(EpicId(epic_id_mem)).is_none());

        real.delete_epic(epic2_id_real).await.unwrap();
        wait_for!(rows_real.epic(EpicId(epic2_id_real)).is_none());
        mem.delete_epic(epic2_id_mem).await.unwrap();
        assert!(rows_mem.epic(EpicId(epic2_id_mem)).is_none());
    });
}
