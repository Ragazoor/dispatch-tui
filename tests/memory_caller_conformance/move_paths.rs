//! Every path that moves a task between epics, run against the same
//! assertions on the real module and on the twin (task #98750).
//!
//! Each reducer that moves a task re-implements "leaving an epic". A rule one
//! sibling had and another forgot came back twice (N1 10-28, R2 10-31). So the
//! rules live in ONE place, `assert_move_rules`, and every move path is a row
//! in `MOVE_PATHS`. A new move path must be added here:
//! `every_task_move_site_in_the_module_is_in_the_table` fails until it is.
//!
//! The rules each path must meet:
//! - the task's unseen mark ends
//!   (`UnseenMarkEndsWithTheTaskLeavingItsEpicOrFinishing`);
//! - the task lands in the same epic on both sides, with the same row shape;
//! - a move onto an epic the caller cannot see is refused the same way on both
//!   sides and leaves the mark alone (`set_task_epic`, `update_task`).

use super::*;

/// One way a task changes epic. `sites` names the module functions that clear
/// the task's unseen mark for this path; the guard test checks that every such
/// function in the module appears in some row.
#[derive(Clone, Copy, Debug)]
enum MovePath {
    SetTaskEpic,
    UpdateTaskRelink,
    PatchTaskRepoReroute,
    UpdateTaskRepoReroute,
    RegroupEpic,
    FlattenEpic,
}

const MOVE_PATHS: &[(MovePath, &[&str])] = &[
    (MovePath::SetTaskEpic, &["tasks_epics.rs::move_task_row"]),
    (
        MovePath::UpdateTaskRelink,
        &["tasks_epics.rs::move_task_row"],
    ),
    (
        MovePath::PatchTaskRepoReroute,
        &["grouping.rs::relink_task"],
    ),
    (
        MovePath::UpdateTaskRepoReroute,
        &["grouping.rs::relink_task"],
    ),
    (MovePath::RegroupEpic, &["grouping.rs::relink_task"]),
    (MovePath::FlattenEpic, &["grouping.rs::relink_task"]),
];

/// Module functions that end a task's unseen mark on a move, with the table
/// row (or other test) that covers each. The feed move goes through
/// `IngestStore::update_task`, generic over the store, so the twin runs the
/// module's own body; `feeds.rs` drives it.
const COVERED_ELSEWHERE: &[&str] = &[
    // The generic ingest body runs on both sides; `feeds.rs` drives it.
    "feed_ingest.rs::update_task",
    // The same generic ingest body: it hands the move to `update_task`.
    "feed_ingest.rs::route_and_move",
];

/// Both stores plus the helpers every move path needs.
struct Pair<'a> {
    real: &'a SdkReducerCaller,
    mem: &'a MemoryReducerCaller,
    rows_real: &'a Arc<SharedRows>,
    rows_mem: &'a Arc<SharedRows>,
    identity: String,
    /// A second user, a member of the same workspace: owner of the private
    /// epic the first user must not be able to move a task into.
    other_real: &'a SdkReducerCaller,
    other_mem: &'a MemoryReducerCaller,
    /// The highest id given to an epic the first caller cannot see.
    hidden_max: std::cell::Cell<i64>,
}

/// Run one call, written once with `$c` as the caller, on both sides, and
/// require both to apply.
macro_rules! both_apply {
    ($p:expr, |$c:ident| $call:expr) => {{
        {
            let $c: &dyn ReducerCaller = $p.real;
            $call.await.unwrap().applied().unwrap();
        }
        {
            let $c: &dyn ReducerCaller = $p.mem;
            $call.await.unwrap().applied().unwrap();
        }
    }};
}

impl Pair<'_> {
    async fn wait(&self, cond: impl Fn(&SharedRows) -> bool) {
        let mut woken = self.rows_real.changed();
        tokio::time::timeout(Duration::from_secs(10), async {
            while !cond(self.rows_real) {
                woken.changed().await.expect("subscription must deliver");
            }
        })
        .await
        .unwrap_or_else(|_| panic!("real store never reached the expected state"));
    }

    /// An epic another user holds privately: unseen to this caller.
    async fn private_epic_of_another(&self) -> i64 {
        let row = bindings::Epic {
            visibility: "private".into(),
            ..blank_epic()
        };
        // The creator's own subscription may not carry it back either, so
        // the id is the next one the store hands out.
        let id = self
            .rows_real
            .epics()
            .iter()
            .map(|e| e.id.0)
            .max()
            .unwrap_or(0)
            .max(self.hidden_max.get())
            + 1;
        self.hidden_max.set(id);
        let made = self.other_real.create_epic(row.clone()).await;
        assert!(
            made.is_ok() || format!("{made:?}").contains("outside this board"),
            "{made:?}"
        );
        self.other_mem.align_next_epic_id(id);
        assert_eq!(self.other_mem.create_epic(row).await.unwrap().0, id);
        id
    }

    async fn epic(&self, grouped: bool) -> i64 {
        let row = bindings::Epic {
            group_by_repo: grouped,
            created_by: self.identity.clone(),
            ..blank_epic()
        };
        let id = self.real.create_epic(row.clone()).await.unwrap().0;
        self.wait(|r| r.epic(EpicId(id)).is_some()).await;
        self.mem.align_next_epic_id(id);
        assert_eq!(self.mem.create_epic(row).await.unwrap().0, id);
        id
    }

    /// A signalled task, so it carries an unseen mark on both sides.
    async fn signalled_task(&self, epic: i64, repo: &str) -> TaskId {
        let row = bindings::Task {
            repo_path: repo.into(),
            created_by: self.identity.clone(),
            ..blank_task_in_epic(epic)
        };
        let id = self.real.create_task_signalled(row.clone()).await.unwrap();
        self.wait(|r| r.task(id).is_some() && r.unseen_task_ids().contains(&id))
            .await;
        self.mem.align_next_task_id(id.0);
        assert_eq!(self.mem.create_task_signalled(row).await.unwrap(), id);
        assert!(self.rows_mem.unseen_task_ids().contains(&id));
        id
    }

    /// The one set of rules every move path meets.
    async fn assert_move_rules(&self, label: &str, task: TaskId, expect_in: i64) {
        self.wait(|r| !r.unseen_task_ids().contains(&task)).await;
        assert!(
            !self.rows_mem.unseen_task_ids().contains(&task),
            "{label}: the twin kept the unseen mark of a task that left its epic"
        );
        let shape = |rows: &SharedRows| rows.task(task).map(|t| task_shape(&t));
        let (real, mem) = (shape(self.rows_real), shape(self.rows_mem));
        assert_eq!(real, mem, "{label}: task shape differs");
        assert_eq!(
            real.and_then(|t| t.epic_id),
            Some(expect_in),
            "{label}: the task is in the wrong epic"
        );
    }
}

fn refusal_of(out: ReducerOutcome) -> String {
    match out {
        ReducerOutcome::Refused(why) => why,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

async fn run_path(p: &Pair<'_>, path: MovePath) {
    let label = format!("{path:?}");
    match path {
        MovePath::SetTaskEpic | MovePath::UpdateTaskRelink => {
            let (from, to) = (p.epic(false).await, p.epic(false).await);
            let task = p.signalled_task(from, "/x/alpha").await;
            // Sight is checked on the destination: a missing or unseen epic is refused
            // identically and the mark stays.
            let hidden = p.private_epic_of_another().await;
            for dest in [999_999, hidden] {
                let dest_epic = Some(EpicId(dest));
                let (r, m) = match path {
                    MovePath::SetTaskEpic => (
                        p.real.set_task_epic(task, dest_epic, String::new()).await,
                        p.mem.set_task_epic(task, dest_epic, String::new()).await,
                    ),
                    _ => (
                        p.real
                            .update_task(task, Some(dest_epic), String::new(), blank_task_patch())
                            .await,
                        p.mem
                            .update_task(task, Some(dest_epic), String::new(), blank_task_patch())
                            .await,
                    ),
                };
                let (r, m) = (refusal_of(r.unwrap()), refusal_of(m.unwrap()));
                assert!(r.contains(&m) || m.contains(&r), "{label}: {r:?} vs {m:?}");
                assert!(p.rows_real.unseen_task_ids().contains(&task), "{label}");
                assert!(p.rows_mem.unseen_task_ids().contains(&task), "{label}");
            }

            let to_id = Some(EpicId(to));
            if matches!(path, MovePath::SetTaskEpic) {
                both_apply!(p, |c| c.set_task_epic(task, to_id, String::new()));
            } else {
                both_apply!(p, |c| c.update_task(
                    task,
                    Some(to_id),
                    String::new(),
                    blank_task_patch()
                ));
            }
            p.assert_move_rules(&label, task, to).await;
        }
        MovePath::PatchTaskRepoReroute | MovePath::UpdateTaskRepoReroute => {
            let root = p.epic(true).await;
            let task = p.signalled_task(root, "/x/alpha").await;
            let sub = p.rows_real.task(task).unwrap().epic_id.unwrap().0;
            let patch = bindings::TaskPatch {
                repo_path: Some("/x/beta".into()),
                ..blank_task_patch()
            };
            if matches!(path, MovePath::PatchTaskRepoReroute) {
                both_apply!(p, |c| c.patch_task(task, patch.clone()));
            } else {
                both_apply!(p, |c| c.update_task(
                    task,
                    None,
                    String::new(),
                    patch.clone()
                ));
            }
            p.wait(|r| r.task(task).and_then(|t| t.epic_id) != Some(EpicId(sub)))
                .await;
            let now_in = p.rows_real.task(task).unwrap().epic_id.unwrap().0;
            assert_ne!(now_in, root, "{label}: it belongs in a repo sub-epic");
            p.assert_move_rules(&label, task, now_in).await;
        }
        MovePath::RegroupEpic => {
            // A task sits directly on a root that is not yet grouped; grouping
            // it and regrouping moves the task into its repo sub-epic.
            let root = p.epic(false).await;
            let task = p.signalled_task(root, "/x/alpha").await;
            let group = bindings::EpicPatch {
                group_by_repo: Some(true),
                ..blank_epic_patch()
            };
            both_apply!(p, |c| c.patch_epic(EpicId(root), group.clone()));
            both_apply!(p, |c| c.regroup_epic(EpicId(root)));
            p.wait(|r| r.task(task).and_then(|t| t.epic_id) != Some(EpicId(root)))
                .await;
            let now_in = p.rows_real.task(task).unwrap().epic_id.unwrap().0;
            p.assert_move_rules(&label, task, now_in).await;
        }
        MovePath::FlattenEpic => {
            let root = p.epic(true).await;
            let task = p.signalled_task(root, "/x/alpha").await;
            both_apply!(p, |c| c.flatten_epic(EpicId(root)));
            p.assert_move_rules(&label, task, root).await;
        }
    }
}

/// `UnseenMarkEndsWithTheTaskLeavingItsEpicOrFinishing`, for every move path.
#[test]
fn every_task_move_path_meets_the_same_rules_on_both_stores() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = Instance::start("memory-caller-move-paths");
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
        // The second user.
        let rows_other = Arc::new(SharedRows::new());
        let connector2 = Arc::new(SpacetimeSdkConnector::new(
            instance.database(),
            rows_other.clone(),
        ));
        let accepted2 = connector2
            .connect(&instance.host(), None)
            .await
            .unwrap_or_else(|e| panic!("connect: {e}"));
        connector2
            .subscribe(&SubscriptionRequest::new(
                accepted2.identity.clone(),
                "host-b",
            ))
            .await
            .unwrap_or_else(|e| panic!("subscribe: {e}"));
        let other_real = SdkReducerCaller::new(connector2, Arc::new(SettledIdentity::default()));
        let other_mem = mem.as_user(accepted2.identity.clone());
        // The second user's profile arrives with their subscription; retry
        // until the store knows the person.
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let out = real
                    .add_member(
                        DEFAULT_WORKSPACE,
                        accepted2.identity.clone(),
                        "member".to_string(),
                    )
                    .await;
                match out {
                    Ok(ReducerOutcome::Applied(_)) => break,
                    _ => tokio::task::yield_now().await,
                }
            }
        })
        .await
        .expect("the store knows the second user");
        other_mem.register_user_profile();
        mem.add_member(
            DEFAULT_WORKSPACE,
            accepted2.identity.clone(),
            "member".to_string(),
        )
        .await
        .unwrap()
        .applied()
        .unwrap();
        let pair = Pair {
            other_real: &other_real,
            other_mem: &other_mem,
            hidden_max: std::cell::Cell::new(0),
            real: &real,
            mem: &mem,
            rows_real: &rows_real,
            rows_mem: &rows_mem,
            identity: accepted.identity.clone(),
        };
        for (path, _) in MOVE_PATHS {
            run_path(&pair, *path).await;
        }
    });
}

/// Whether the struct literal that contains the next line is a `Task`: walk
/// back over closed literals to the line that opened this one.
fn opens_task_literal(before: &[&str]) -> bool {
    let mut depth = 0;
    for line in before.iter().rev() {
        let t = line.trim_end();
        if t.ends_with('}') || t.ends_with("},") {
            depth += 1;
        } else if t.ends_with('{') {
            if depth == 0 {
                let head = t.trim_end_matches('{').trim_end();
                return head.ends_with("Task");
            }
            depth -= 1;
        }
    }
    false
}

/// The `module/src` functions that write a task's `epic_id`: a `Task { epic_id,
/// .. }` literal or an `.epic_id =` assignment. A move path that forgot the
/// unseen mark would not call `clear_unseen_for_task` at all, so the scan keys
/// on the write, not on the clear (G1; N1 and R2 were exactly that).
fn epic_id_writers(file: &str, body: &str) -> Vec<String> {
    let mut sites = Vec::new();
    let mut current = String::new();
    let lines: Vec<&str> = body.lines().collect();
    for (at, line) in lines.iter().enumerate() {
        let t = line.trim_start();
        if let Some(rest) = t
            .strip_prefix("pub(crate) fn ")
            .or_else(|| t.strip_prefix("pub fn "))
            .or_else(|| t.strip_prefix("fn "))
        {
            current = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
        }
        let field = t.strip_prefix("epic_id").is_some_and(|rest| {
            let rest = rest.trim_start();
            // `epic_id,` (shorthand) or `epic_id: <value>`, never a parameter
            // or struct field declaration (`epic_id: i64,`, `Patch<i64>`).
            rest.starts_with(',')
                || rest.strip_prefix(':').is_some_and(|v| {
                    let v = v.trim_start();
                    !(v.starts_with("i64")
                        || v.starts_with("Option")
                        || v.starts_with("Patch")
                        || v.starts_with("pub"))
                })
        });
        let assigned = t.contains(".epic_id =") && !t.contains(".epic_id ==");
        if (field && opens_task_literal(&lines[..at])) || assigned {
            sites.push(format!("{file}::{current}"));
        }
    }
    sites.sort();
    sites.dedup();
    sites
}

/// Writers that are not moves of an existing task, each with why.
const NOT_A_MOVE: &[(&str, &str)] = &[
    (
        "session_request.rs::phoenix_successor",
        "builds a new task row for an insert: no prior mark",
    ),
    (
        "tasks_epics.rs::respawn_phoenix_successor",
        "inserts the successor row: no prior mark",
    ),
    (
        "feed.rs::upsert_feed_item",
        "looked up by (epic_id, external_id): it stays in its epic",
    ),
    ("tasks_epics.rs::create_task", "an insert: no prior mark"),
    (
        "tasks_epics.rs::create_task_signalled",
        "an insert: the mark is created for the viewers afterwards",
    ),
];

#[test]
fn the_epic_id_scan_finds_a_writer_that_never_clears_the_mark() {
    let body = "fn relink_task(ctx: &C, task: Task, target: i64) {\n    write_task(ctx, Task {\n        epic_id: target,\n        ..task\n    });\n}\nfn sig(epic_id: i64) {}\nfn sub() {\n    s(Subscription {\n        epic_id,\n    });\n}";
    assert_eq!(
        epic_id_writers("g.rs", body),
        vec!["g.rs::relink_task".to_string()]
    );
    let assign = "fn mv(t: &mut Task) {\n    t.epic_id = 3;\n    if t.epic_id == 3 {}\n}";
    assert_eq!(
        epic_id_writers("g.rs", assign),
        vec!["g.rs::mv".to_string()]
    );
}

/// A move path nobody added to the table. Needs no store: it reads the module
/// source for every function that writes a task's `epic_id` and requires each
/// to be named by a table row, by `COVERED_ELSEWHERE` or by `NOT_A_MOVE`.
#[test]
fn every_task_move_site_in_the_module_is_in_the_table() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("spacetime/module/src");
    let mut sites = Vec::new();
    for entry in std::fs::read_dir(&src).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        // Fixtures and tests build tasks, they do not move them.
        if matches!(name.as_str(), "blanks.rs" | "tests.rs") {
            continue;
        }
        let body = std::fs::read_to_string(&path).unwrap();
        sites.extend(epic_id_writers(&name, &body));
    }
    sites.sort();
    sites.dedup();
    assert!(
        !sites.is_empty(),
        "the scan found no site: update this test"
    );
    for site in sites {
        let in_table = MOVE_PATHS
            .iter()
            .any(|(_, covers)| covers.contains(&site.as_str()));
        let elsewhere = COVERED_ELSEWHERE.contains(&site.as_str());
        let not_a_move = NOT_A_MOVE.iter().any(|(s, _)| *s == site);
        assert!(
            in_table || elsewhere || not_a_move,
            "{site} writes a task's epic_id but no MOVE_PATHS row covers it: \
             add the path to tests/memory_caller_conformance/move_paths.rs"
        );
    }
}
