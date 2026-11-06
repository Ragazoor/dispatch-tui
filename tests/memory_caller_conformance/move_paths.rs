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
pub(super) struct Pair<'a> {
    pub(super) real: &'a SdkReducerCaller,
    pub(super) mem: &'a MemoryReducerCaller,
    pub(super) rows_real: &'a Arc<SharedRows>,
    pub(super) rows_mem: &'a Arc<SharedRows>,
    pub(super) identity: String,
    /// A second user, a member of the same workspace: owner of the private
    /// epic the first user must not be able to move a task into.
    pub(super) other_real: &'a SdkReducerCaller,
    pub(super) other_mem: &'a MemoryReducerCaller,
    pub(super) other_identity: String,
    /// The highest id given to an epic the first caller cannot see.
    hidden_max: std::cell::Cell<i64>,
}

/// A published module, two connected users (the first a super-admin, the
/// second a member of the default workspace) and the twin of each, owned so a
/// test can borrow a [`Pair`] from it.
pub(super) struct World {
    _instance: Instance,
    real: SdkReducerCaller,
    mem: MemoryReducerCaller,
    rows_real: Arc<SharedRows>,
    rows_mem: Arc<SharedRows>,
    identity: String,
    other_real: SdkReducerCaller,
    other_mem: MemoryReducerCaller,
    other_identity: String,
}

impl World {
    /// Publish the module into `instance` and connect both users on both
    /// stores.
    pub(super) async fn start(instance: Instance) -> World {
        let rows_real = Arc::new(SharedRows::new());
        let rows_mem = Arc::new(SharedRows::new());
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
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
        World {
            _instance: instance,
            real,
            mem,
            rows_real,
            rows_mem,
            identity: accepted.identity,
            other_real,
            other_mem,
            other_identity: accepted2.identity,
        }
    }

    pub(super) fn pair(&self) -> Pair<'_> {
        Pair {
            other_real: &self.other_real,
            other_mem: &self.other_mem,
            other_identity: self.other_identity.clone(),
            hidden_max: std::cell::Cell::new(0),
            real: &self.real,
            mem: &self.mem,
            rows_real: &self.rows_real,
            rows_mem: &self.rows_mem,
            identity: self.identity.clone(),
        }
    }
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
    pub(super) async fn wait(&self, cond: impl Fn(&SharedRows) -> bool) {
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
    pub(super) async fn private_epic_of_another(&self) -> i64 {
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

    pub(super) async fn epic(&self, grouped: bool) -> i64 {
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
                            .update_task(
                                task,
                                Some(dest_epic),
                                String::new(),
                                String::new(),
                                blank_task_patch(),
                                String::new(),
                                None,
                            )
                            .await,
                        p.mem
                            .update_task(
                                task,
                                Some(dest_epic),
                                String::new(),
                                String::new(),
                                blank_task_patch(),
                                String::new(),
                                None,
                            )
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
                    String::new(),
                    blank_task_patch(),
                    String::new(),
                    None,
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
                    String::new(),
                    patch.clone(),
                    String::new(),
                    None,
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

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async move {
        let world = World::start(instance).await;
        let pair = world.pair();
        for (path, _) in MOVE_PATHS {
            run_path(&pair, *path).await;
        }
    });
}

/// The name a line declares, if it is a `fn` header in any form: `fn`,
/// `pub fn`, `pub(crate) fn`, `pub(super) fn`, `pub(in path) fn`, with
/// `const`/`async`/`unsafe` in front (G3: a `pub(super) fn` or `async fn` used
/// to be missed, so its writes were charged to the previous function), and
/// with attributes in front on the same line (G8: `#[reducer] pub fn mv`).
fn fn_name(line: &str) -> Option<String> {
    let mut rest = line.trim_start();
    while rest.starts_with("#[") {
        rest = rest[rest.find(']')? + 1..].trim_start();
    }
    if let Some(after) = rest.strip_prefix("pub") {
        rest = after;
        if rest.starts_with('(') {
            rest = rest[rest.find(')')? + 1..].trim_start();
        } else if rest.starts_with(char::is_whitespace) {
            rest = rest.trim_start();
        } else {
            return None;
        }
    }
    while let Some(after) = ["const ", "async ", "unsafe "]
        .iter()
        .find_map(|q| rest.strip_prefix(q))
    {
        rest = after.trim_start();
    }
    let name: String = rest
        .strip_prefix("fn ")?
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `src` with the inside of every comment, string and char literal replaced by
/// spaces. Byte offsets and newlines are kept, so an offset found in the result
/// is the same offset in `src`. Brace counting and the scans below run on this
/// (G8: a `"}"` in a string ended a span).
fn blank_literals(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let blank = |out: &mut String, c: char| {
        if c == '\n' {
            out.push('\n');
        } else {
            out.extend(std::iter::repeat_n(' ', c.len_utf8()));
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                blank(&mut out, chars[i]);
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            let mut depth = 0;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    blank(&mut out, chars[i]);
                    blank(&mut out, chars[i + 1]);
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    blank(&mut out, chars[i]);
                    blank(&mut out, chars[i + 1]);
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    blank(&mut out, chars[i]);
                    i += 1;
                }
            }
        } else if c == '"' || (c == 'r' && raw_string_start(&chars, i).is_some()) {
            // A normal string, or a raw string `r#"..."#`.
            let (hashes, body_at) = if c == '"' {
                (None, i + 1)
            } else {
                let (h, at) = raw_string_start(&chars, i).unwrap_or((0, i + 1));
                (Some(h), at)
            };
            for ch in &chars[i..body_at] {
                out.push(*ch);
            }
            i = body_at;
            while i < chars.len() {
                let ch = chars[i];
                let closes = match hashes {
                    None => ch == '"',
                    Some(h) => {
                        ch == '"'
                            && chars[i + 1..].iter().take(h).filter(|c| **c == '#').count() == h
                    }
                };
                if closes {
                    out.push('"');
                    i += 1;
                    for _ in 0..hashes.unwrap_or(0) {
                        out.push('#');
                        i += 1;
                    }
                    break;
                }
                if hashes.is_none() && ch == '\\' {
                    blank(&mut out, ch);
                    i += 1;
                    if let Some(esc) = chars.get(i) {
                        blank(&mut out, *esc);
                        i += 1;
                    }
                    continue;
                }
                blank(&mut out, ch);
                i += 1;
            }
        } else if c == '\'' {
            // A char literal (`'x'`, `'\n'`) is blanked; a lifetime is not.
            let close = if next == Some('\\') {
                chars[i + 2..]
                    .iter()
                    .position(|c| *c == '\'')
                    .map(|p| i + 2 + p)
            } else if chars.get(i + 2) == Some(&'\'') {
                Some(i + 2)
            } else {
                None
            };
            match close {
                Some(end) => {
                    out.push('\'');
                    for ch in &chars[i + 1..end] {
                        blank(&mut out, *ch);
                    }
                    out.push('\'');
                    i = end + 1;
                }
                None => {
                    out.push(c);
                    i += 1;
                }
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// `(number of #, index just after the opening quote)` when `chars[at]` starts a
/// raw string (`r"`, `r#"`), and is not the tail of an identifier.
fn raw_string_start(chars: &[char], at: usize) -> Option<(usize, usize)> {
    if at > 0 && is_ident_char(chars[at - 1]) {
        return None;
    }
    let mut j = at + 1;
    let mut hashes = 0;
    while chars.get(j) == Some(&'#') {
        hashes += 1;
        j += 1;
    }
    (chars.get(j) == Some(&'"')).then_some((hashes, j + 1))
}

/// Offset of the `}` that closes the `{` just before `open`, or the end.
fn matching_close(body: &str, open: usize) -> usize {
    let mut depth = 1;
    for (i, c) in body[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return open + i;
                }
            }
            _ => {}
        }
    }
    body.len()
}

/// Byte offsets (into `span`) of an `epic_id` field in a `Task { .. }` literal
/// span: `epic_id,` / `epic_id }` shorthand or `epic_id: <value>`, never a
/// read (`t.epic_id`). A literal holds no declarations (a `struct Task {` is
/// skipped by the caller), so any `epic_id:` value is a write, however it
/// starts (G8: `Option::from(e)` and `public_epic` were filtered as types).
fn epic_id_fields(span: &str) -> Vec<usize> {
    span.match_indices("epic_id")
        .filter(|(at, _)| {
            let before = span[..*at].chars().next_back();
            if before.is_some_and(|c| is_ident_char(c) || c == '.') {
                return false;
            }
            let rest = span[at + "epic_id".len()..].trim_start();
            rest.starts_with(',')
                || rest.starts_with('}')
                || rest
                    .strip_prefix(':')
                    .is_some_and(|v| !v.trim_start().starts_with(':'))
        })
        .map(|(at, _)| at)
        .collect()
}

/// Byte ranges of the bodies of `impl .. Task` blocks (`impl Task`,
/// `impl Trait for Task`), where `Self { .. }` builds a `Task`.
fn impl_task_spans(body: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    for (at, _) in body.match_indices("impl") {
        let word_before = body[..at].chars().next_back().is_some_and(is_ident_char);
        let after = body[at + 4..].chars().next();
        if word_before || !after.is_some_and(|c| c.is_whitespace() || c == '<') {
            continue;
        }
        let Some(brace) = body[at..].find('{') else {
            continue;
        };
        let mut header = body[at..at + brace].trim_end();
        if let Some(h) = header.strip_suffix('>') {
            header = h.rsplit_once('<').map_or(h, |(l, _)| l).trim_end();
        }
        if header.rsplit(|c: char| !is_ident_char(c)).next() == Some("Task") {
            let open = at + brace + 1;
            spans.push((open, matching_close(body, open)));
        }
    }
    spans
}

/// Where the first `epic_id` write of every write in `body` sits, as
/// `(offset, function)` pairs, one per write: a field in a `Task { .. }` (or,
/// inside `impl Task`, a `Self { .. }`) literal, an `.epic_id =` assignment, an
/// `.epic_id.replace(` style call, or a `&mut ..epic_id` borrow.
fn epic_id_write_sites(body: &str) -> Vec<(usize, String)> {
    let body = &blank_literals(body);
    let mut headers: Vec<(usize, String)> = Vec::new();
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        if let Some(name) = fn_name(line) {
            headers.push((offset, name));
        }
        offset += line.len();
    }
    let owner = |at: usize| {
        headers
            .iter()
            .rev()
            .find(|(start, _)| *start <= at)
            .map(|(_, name)| name.clone())
            .unwrap_or_default()
    };
    let impl_task = impl_task_spans(body);
    let mut sites = Vec::new();
    for pattern in ["Task {", "Self {"] {
        for (at, _) in body.match_indices(pattern) {
            if pattern == "Self {" && !impl_task.iter().any(|(s, e)| *s <= at && at < *e) {
                continue;
            }
            let head = body[..at].trim_end();
            // `-> Task {` opens a function body, not a literal.
            if head.ends_with("->") {
                continue;
            }
            if body[..at].chars().next_back().is_some_and(is_ident_char)
                || matches!(
                    head.rsplit(|c: char| !is_ident_char(c)).next(),
                    Some("impl" | "struct" | "for" | "enum" | "type")
                )
            {
                continue;
            }
            let open = at + pattern.len();
            let close = matching_close(body, open);
            // The closing brace is part of the span: `Task { epic_id }`.
            let span = &body[open..(close + 1).min(body.len())];
            if let Some(first) = epic_id_fields(span).first() {
                sites.push((open + first, owner(open + first)));
            }
        }
    }
    for (at, _) in body.match_indices(".epic_id") {
        let rest = body[at + ".epic_id".len()..].trim_start();
        if body[at + ".epic_id".len()..]
            .chars()
            .next()
            .is_some_and(is_ident_char)
        {
            continue;
        }
        let assigns = rest.starts_with('=') && !rest.starts_with("==") && !rest.starts_with("=>");
        let mutates = [".replace(", ".take(", ".insert(", ".get_or_insert", ".set("]
            .iter()
            .any(|m| rest.starts_with(m));
        if assigns || mutates {
            sites.push((at, owner(at)));
        }
    }
    for (at, _) in body.match_indices("&mut") {
        let rest = body[at + 4..].trim_start();
        let path: String = rest
            .chars()
            .take_while(|c| is_ident_char(*c) || *c == '.')
            .collect();
        if path.ends_with(".epic_id") || path == "epic_id" {
            sites.push((at, owner(at)));
        }
    }
    sites.sort();
    sites
}

/// Each function in `file` that writes a task's `epic_id`, with how many
/// writes it makes. A move path that forgot the unseen mark would not call
/// `clear_unseen_for_task` at all, so the scan keys on the write, not on the
/// clear (G1; N1 and R2 were exactly that). The count is kept (G8): a second
/// write in a function already covered by a row is a new path to review.
fn epic_id_write_count(file: &str, body: &str) -> Vec<(String, usize)> {
    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
    for (_, name) in epic_id_write_sites(body) {
        *counts.entry(format!("{file}::{name}")).or_default() += 1;
    }
    counts.into_iter().collect()
}

/// The `module/src` functions that write a task's `epic_id`, once each.
fn epic_id_writers(file: &str, body: &str) -> Vec<String> {
    epic_id_write_count(file, body)
        .into_iter()
        .map(|(site, _)| site)
        .collect()
}

/// Every `.rs` file under `dir`, recursively (G3: `read_dir` alone missed a
/// module split into a subdirectory), as `(path relative to dir, source)`.
fn module_sources(dir: &std::path::Path) -> Vec<(String, String)> {
    fn walk(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.push((rel, std::fs::read_to_string(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out
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

/// G3 blind spot 1: a one-line literal.
#[test]
fn the_epic_id_scan_finds_a_one_line_task_literal() {
    for body in [
        "fn mv(t: Task, e: i64) -> Task {\n    Task { epic_id: Some(e), ..t }\n}",
        "fn mv(t: Task, epic_id: Option<i64>) -> Task {\n    Task { epic_id, ..t }\n}",
        "fn mv(t: Task, e: i64) -> Task {\n    write(Task { title: x, epic_id: e, ..t });\n}",
        "fn mv(t: Task, epic_id: i64) -> Task {\n    Task { ..t, epic_id }\n}",
    ] {
        assert_eq!(
            epic_id_writers("g.rs", body),
            vec!["g.rs::mv".to_string()],
            "{body}"
        );
    }
}

/// G3 blind spot 2: every header form owns the writes under it.
#[test]
fn the_epic_id_scan_attributes_a_write_to_any_fn_header_form() {
    for header in [
        "pub(super) fn mv",
        "pub(crate) fn mv",
        "pub(in crate::a) fn mv",
        "async fn mv",
        "pub async fn mv",
        "pub(super) async fn mv",
        "pub const fn mv",
        "unsafe fn mv",
    ] {
        let body = format!(
            "fn first() {{}}\n{header}(t: Task) {{\n    Task {{\n        epic_id: None,\n        ..t\n    }};\n}}"
        );
        assert_eq!(
            epic_id_writers("g.rs", &body),
            vec!["g.rs::mv".to_string()],
            "{header}"
        );
    }
}

/// What a Task-shaped line is not: a type, an impl, a read, a parameter.
#[test]
fn the_epic_id_scan_ignores_declarations_and_reads() {
    let body = "pub struct Task {\n    pub epic_id: Option<i64>,\n}\nimpl Task {\n    fn epic(&self) -> Option<i64> { self.epic_id }\n}\nfn read(t: Task) -> bool {\n    t.epic_id == Some(1)\n}\nfn other(t: Task) -> Task {\n    Task { title: t.epic_id.map(|e| e.to_string()), ..t }\n}";
    assert_eq!(epic_id_writers("g.rs", body), Vec::<String>::new());
}

/// G8: values that merely start like a type name are still writes.
#[test]
fn the_epic_id_scan_finds_a_value_that_starts_like_a_type() {
    for value in [
        "Option::from(e)",
        "public_epic",
        "i64::from(e)",
        "Patch::set(e)",
    ] {
        let body =
            format!("fn mv(t: Task, e: i64) -> Task {{\n    Task {{ epic_id: {value}, ..t }}\n}}");
        assert_eq!(
            epic_id_writers("g.rs", &body),
            vec!["g.rs::mv".to_string()],
            "{value}"
        );
    }
}

/// G8: `Self { epic_id, ..self }` inside `impl Task`, and only there.
#[test]
fn the_epic_id_scan_finds_a_self_literal_in_impl_task() {
    let body = "impl Task {\n    fn moved(self, epic_id: i64) -> Self {\n        Self { epic_id: Some(epic_id), ..self }\n    }\n}";
    assert_eq!(
        epic_id_writers("g.rs", body),
        vec!["g.rs::moved".to_string()]
    );
    let other = "impl Other {\n    fn moved(self, epic_id: i64) -> Self {\n        Self { epic_id, ..self }\n    }\n}";
    assert_eq!(epic_id_writers("g.rs", other), Vec::<String>::new());
}

/// G8: a write through a method or a mutable borrow.
#[test]
fn the_epic_id_scan_finds_replace_and_mut_borrow_writes() {
    for write in [
        "t.epic_id.replace(Some(e));",
        "let old = std::mem::replace(&mut t.epic_id, Some(e));",
        "let slot = &mut t.epic_id;",
        "let slot = &mut   t.epic_id;",
        "t.epic_id=Some(e);",
    ] {
        let body = format!("fn mv(t: &mut Task, e: i64) {{\n    {write}\n}}");
        assert_eq!(
            epic_id_writers("g.rs", &body),
            vec!["g.rs::mv".to_string()],
            "{write}"
        );
    }
    let reads = "fn rd(t: &Task) -> bool {\n    t.epic_id == Some(1) && t.epic_id.is_some() && f(&t.epic_id)\n}";
    assert_eq!(epic_id_writers("g.rs", reads), Vec::<String>::new());
}

/// G8: a brace inside a string or a comment does not end a span.
#[test]
fn the_epic_id_scan_ignores_braces_in_strings_and_comments() {
    let body = "fn mv(t: Task, e: i64) -> Task {\n    Task {\n        title: \"}\".into(),\n        // } stray\n        epic_id: Some(e),\n        ..t\n    }\n}";
    assert_eq!(epic_id_writers("g.rs", body), vec!["g.rs::mv".to_string()]);
    let quoted = "fn msg() -> &'static str {\n    \"Task { epic_id: 1 }\"\n}";
    assert_eq!(epic_id_writers("g.rs", quoted), Vec::<String>::new());
}

/// G8: an attribute on the `fn` line still names the function.
#[test]
fn the_epic_id_scan_attributes_a_write_under_an_attributed_fn_line() {
    let body =
        "fn first() {}\n#[reducer] pub fn mv(t: Task) {\n    Task { epic_id: None, ..t };\n}";
    assert_eq!(epic_id_writers("g.rs", body), vec!["g.rs::mv".to_string()]);
}

/// A return type is not a literal: the function body is not a `Task { .. }`.
#[test]
fn the_epic_id_scan_does_not_read_a_return_type_as_a_literal() {
    let body = "fn make(epic_id: i64) -> Task {\n    Task { epic_id, ..blank() }\n}";
    assert_eq!(
        epic_id_write_count("g.rs", body),
        vec![("g.rs::make".to_string(), 1)]
    );
}

/// G8: a second write in one function is counted, not folded into the first.
#[test]
fn the_epic_id_scan_counts_each_write_in_a_function() {
    let body = "fn mv(t: &mut Task, e: i64) {\n    t.epic_id = Some(e);\n    t.epic_id = None;\n}";
    assert_eq!(
        epic_id_write_count("g.rs", body),
        vec![("g.rs::mv".to_string(), 2)]
    );
}

/// G3 blind spot 3: module files in subdirectories.
#[test]
fn module_sources_recurses_into_subdirectories() {
    let dir = std::env::temp_dir().join(format!("dispatch-move-scan-{}", std::process::id()));
    let nested = dir.join("a").join("b");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(dir.join("top.rs"), "fn x() {}").unwrap();
    std::fs::write(nested.join("deep.rs"), "fn y() {}").unwrap();
    std::fs::write(nested.join("notes.txt"), "no").unwrap();
    let mut names: Vec<String> = module_sources(&dir).into_iter().map(|(n, _)| n).collect();
    names.sort();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(
        names,
        vec![
            std::path::Path::new("a")
                .join("b")
                .join("deep.rs")
                .to_string_lossy()
                .into_owned(),
            "top.rs".to_string()
        ]
    );
}

/// Every writer under `dir` (recursively), minus fixtures and tests (they
/// build tasks, they do not move them), with its write count.
fn sites_under(dir: &std::path::Path) -> Vec<(String, usize)> {
    let mut sites = Vec::new();
    for (rel, body) in module_sources(dir) {
        let name = std::path::Path::new(&rel)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let in_tests_dir = std::path::Path::new(&rel)
            .components()
            .any(|c| c.as_os_str() == "tests");
        if in_tests_dir || matches!(name.as_str(), "blanks.rs" | "tests.rs") {
            continue;
        }
        // Subdirectory files keep their path so two `mod.rs` do not collide.
        sites.extend(epic_id_write_count(&rel, &body));
    }
    sites.sort();
    sites
}

fn module_write_counts() -> Vec<(String, usize)> {
    sites_under(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("spacetime/module/src"))
}

fn module_sites() -> Vec<String> {
    module_write_counts().into_iter().map(|(s, _)| s).collect()
}

/// The twin's move functions: `src/sync/memory_caller/` (G8: it was not
/// scanned, so a move the twin made and the module did not went unseen).
fn twin_write_counts() -> Vec<(String, usize)> {
    sites_under(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/sync/memory_caller"))
}

/// Writers the module makes more than once, with the count. A function that
/// gains a write fails `every_module_site_makes_the_writes_the_table_expects`:
/// the new write is a path the row does not exercise.
const MORE_THAN_ONE_WRITE: &[(&str, usize)] = &[];

/// Every twin function that writes a task's `epic_id`, with the module site it
/// mirrors (or why none). A twin write absent from this list is a move only
/// the twin makes, so no module path pins it.
const TWIN_SITES: &[(&str, &str)] = &[
    (
        "agent_state.rs::apply_respawn_phoenix_successor",
        "mirrors tasks_epics.rs::respawn_phoenix_successor: inserts the successor, no prior mark",
    ),
    (
        "convert.rs::from",
        "a row type conversion between the module and binding Task, not a move",
    ),
    (
        "feed.rs::upsert_feed_item",
        "mirrors feed.rs::upsert_feed_item: stays in its epic",
    ),
    (
        "grouping.rs::relink_task",
        "mirrors grouping.rs::relink_task (MOVE_PATHS)",
    ),
    (
        "tasks_epics.rs::apply_create_task",
        "mirrors tasks_epics.rs::create_task: an insert, no prior mark",
    ),
    (
        "tasks_epics.rs::apply_set_task_epic",
        "mirrors tasks_epics.rs::move_task_row (MOVE_PATHS)",
    ),
];

#[test]
fn every_module_site_makes_the_writes_the_table_expects() {
    for (site, n) in module_write_counts() {
        let want = MORE_THAN_ONE_WRITE
            .iter()
            .find(|(s, _)| *s == site)
            .map_or(1, |(_, n)| *n);
        assert_eq!(
            n, want,
            "{site} writes a task's epic_id {n} time(s), expected {want}: a new write \
             is a new move path; add it to the table or update MORE_THAN_ONE_WRITE"
        );
    }
}

#[test]
fn every_twin_move_function_is_listed() {
    let twin = twin_write_counts();
    assert!(
        !twin.is_empty(),
        "the twin scan found no site: update this test"
    );
    for (site, _) in &twin {
        assert!(
            TWIN_SITES.iter().any(|(s, _)| s == site),
            "{site} writes a task's epic_id on the twin but is not in TWIN_SITES: a move \
             only the twin makes is untested; list it with the module site it mirrors"
        );
    }
    for (site, _) in TWIN_SITES {
        assert!(
            twin.iter().any(|(s, _)| s == site),
            "TWIN_SITES lists {site}, which no longer writes a task's epic_id"
        );
    }
}

/// A move path nobody added to the table. Needs no store: it reads the module
/// source for every function that writes a task's `epic_id` and requires each
/// to be named by a table row, by `COVERED_ELSEWHERE` or by `NOT_A_MOVE`.
#[test]
fn every_task_move_site_in_the_module_is_in_the_table() {
    let sites = module_sites();
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

/// G3 blind spot 4: a site added to an existing row is not exercised by a new
/// path. Each row names exactly one site, so a second writer needs its own
/// `MovePath` (and its own `run_path` arm), not a ride on a sibling's row.
#[test]
fn every_move_path_row_names_exactly_one_site() {
    for (path, sites) in MOVE_PATHS {
        assert_eq!(
            sites.len(),
            1,
            "{path:?} names {} sites: give each site its own MovePath so \
             run_path exercises it",
            sites.len()
        );
    }
}

/// A row that names a site the scan no longer finds is a stale claim: the
/// path would pass while covering nothing.
#[test]
fn every_site_a_row_names_still_writes_epic_id() {
    let found = module_sites();
    for (path, sites) in MOVE_PATHS {
        for site in *sites {
            assert!(
                found.iter().any(|f| f == site),
                "{path:?} names {site}, which no longer writes a task's epic_id"
            );
        }
    }
}
