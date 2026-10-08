use super::*;
use crate::agent_tree::open_set::{read_open_set, write_open_set, write_selected_source};
use crate::agent_tree::test_repo::TestRepo;
use crate::process::{MockProcessRunner, RealProcessRunner};
use crate::worktree_admin::tests::make_linked_worktree;

/// A diff pane over a real worktree, refreshed with real git.
struct Pane {
    repo: TestRepo,
    last: LastSeen,
    lines: Vec<DiffLine>,
    state: DiffState,
}

impl Pane {
    fn new() -> Self {
        Self {
            repo: TestRepo::new(),
            last: LastSeen::default(),
            lines: Vec::new(),
            state: DiffState::new(),
        }
    }

    /// Publish the open set, as the tree does.
    fn open(&self, open: &[&str]) {
        let paths: Vec<PathBuf> = open.iter().map(PathBuf::from).collect();
        write_open_set(&self.repo.root_str(), &paths).unwrap();
    }

    /// Publish the selected source, as the tree does.
    fn select(&self, commit: Option<&str>) {
        write_selected_source(&self.repo.root_str(), commit).unwrap();
    }

    fn refresh_with(&mut self, runner: &dyn ProcessRunner) {
        refresh(
            self.repo.root(),
            runner,
            &mut self.last,
            &mut self.lines,
            &mut self.state,
        );
    }

    fn refresh(&mut self) {
        self.refresh_with(&RealProcessRunner::default());
    }

    fn text(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Real git, with every argv recorded — for the steady-state cost.
#[derive(Default)]
struct Recording {
    inner: RealProcessRunner,
    calls: std::sync::Mutex<Vec<String>>,
}

impl Recording {
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.calls.lock().unwrap())
    }
}

impl ProcessRunner for Recording {
    fn run(&self, program: &str, args: &[&str]) -> Result<std::process::Output> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{program} {}", args.join(" ")));
        self.inner.run(program, args)
    }

    fn run_with_timeout(
        &self,
        program: &str,
        args: &[&str],
        timeout: std::time::Duration,
    ) -> Result<std::process::Output> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{program} {}", args.join(" ")));
        self.inner.run_with_timeout(program, args, timeout)
    }
}

#[test]
fn an_empty_open_set_blanks_the_pane_without_asking_git() {
    let dir = tempfile::tempdir().unwrap();
    let (root, _) = make_linked_worktree(dir.path(), "task");
    let mut last = LastSeen::default();
    let mut lines = vec![DiffLine {
        kind: DiffLineKind::Context,
        text: "stale".into(),
    }];
    let mut state = DiffState::new();
    state.notice = Some("old".into());
    let runner = MockProcessRunner::new(vec![]);

    refresh(Path::new(&root), &runner, &mut last, &mut lines, &mut state);

    assert!(lines.is_empty());
    assert_eq!(state.notice, None);
    assert!(runner.recorded_calls().is_empty());
}

/// By default the pane shows unstaged work.
#[test]
fn a_first_refresh_builds_the_document_of_unstaged_work() {
    let mut pane = Pane::new();
    pane.repo.append("seed.txt", "more\n");
    pane.open(&["seed.txt"]);

    pane.refresh();

    assert!(pane
        .lines
        .iter()
        .any(|l| l.kind == DiffLineKind::Heading && l.text == "seed.txt"));
    assert!(pane
        .lines
        .iter()
        .any(|l| l.kind == DiffLineKind::Added && l.text == "+more"));
    assert_eq!(pane.state.notice, None);
}

/// An open untracked file shows its whole contents as additions.
#[test]
fn an_open_untracked_file_shows_its_whole_contents() {
    let mut pane = Pane::new();
    pane.repo.write("new.rs", "one\n");
    pane.open(&["new.rs"]);

    pane.refresh();

    assert!(
        pane.lines.iter().any(|l| l.text == "+one"),
        "{}",
        pane.text()
    );
}

/// The counts cannot see an untracked file change, so its contents are
/// part of the fingerprint: a growing new file must not freeze at its
/// first read.
#[test]
fn a_growing_untracked_file_is_re_read() {
    let mut pane = Pane::new();
    pane.repo.write("new.rs", "one\n");
    pane.open(&["new.rs"]);
    pane.refresh();

    pane.repo.append("new.rs", "two\n");
    pane.refresh();

    assert!(
        pane.lines.iter().any(|l| l.text == "+two"),
        "{}",
        pane.text()
    );
}

/// Staging a new file and editing nothing further moves it from untracked
/// to absent, with an empty count answer both before and after. It must
/// leave the pane rather than keep its whole-file diff on screen.
#[test]
fn staging_an_open_untracked_file_takes_it_out_of_the_pane() {
    let mut pane = Pane::new();
    pane.repo.write("new.rs", "one\n");
    pane.open(&["new.rs"]);
    pane.refresh();
    assert!(!pane.lines.is_empty());

    pane.repo.git(&["add", "new.rs"]);
    pane.refresh();

    assert!(pane.lines.is_empty(), "{}", pane.text());
}

/// AgentTreeSourceIsOneSelection: the pane reads the selection from where
/// it reads the open set and follows it — to a commit and back again —
/// even though nothing in the worktree moved in between.
#[test]
fn the_pane_follows_the_selected_source() {
    let mut pane = Pane::new();
    pane.repo.write("a.rs", "a1\n");
    pane.repo.commit_all("first");
    pane.repo.write("a.rs", "a1\na2\n");
    let second = pane.repo.commit_all("second");
    pane.repo.write("a.rs", "a1\na2\nwork in progress\n");
    pane.open(&["a.rs"]);

    pane.refresh();
    assert!(pane.text().contains("+work in progress"), "{}", pane.text());
    assert!(!pane.text().contains("+a2"), "{}", pane.text());

    pane.select(Some(&second));
    pane.refresh();
    assert!(pane.text().contains("+a2"), "{}", pane.text());
    assert!(!pane.text().contains("work in progress"), "{}", pane.text());

    pane.select(None);
    pane.refresh();
    assert!(pane.text().contains("+work in progress"), "{}", pane.text());
}

/// A path the agent committed since it was opened shows nothing — and
/// stays open (OpenDiffPathsMaySurviveTheirFiles).
#[test]
fn a_path_committed_since_it_was_opened_shows_nothing_and_stays_open() {
    let mut pane = Pane::new();
    pane.repo.append("seed.txt", "more\n");
    pane.open(&["seed.txt"]);
    pane.refresh();
    assert!(!pane.lines.is_empty());

    pane.repo.commit_all("commit it");
    pane.refresh();

    assert!(pane.lines.is_empty(), "{}", pane.text());
    assert_eq!(
        read_open_set(&pane.repo.root_str()),
        vec![PathBuf::from("seed.txt")]
    );
}

/// The steady-state cost: a pass where nothing moved re-reads no contents,
/// so it runs fewer git commands than the pass that built the document,
/// and leaves the document as it was.
#[test]
fn a_refresh_where_nothing_moved_skips_the_diff() {
    let mut pane = Pane::new();
    pane.repo.append("seed.txt", "more\n");
    pane.open(&["seed.txt"]);
    let runner = Recording::default();

    pane.refresh_with(&runner);
    let building = runner.take();
    let built = pane.lines.clone();
    pane.refresh_with(&runner);
    let steady = runner.take();

    assert_eq!(pane.lines, built);
    assert!(
        steady.len() < building.len(),
        "steady {steady:?} vs building {building:?}"
    );
}

/// A failed git query leaves the document untouched and says so, the same
/// way the tree keeps its last good tree.
#[test]
fn a_failed_git_query_keeps_the_last_document_and_sets_a_notice() {
    let dir = tempfile::tempdir().unwrap();
    let (root, _) = make_linked_worktree(dir.path(), "task");
    write_open_set(&root, &[PathBuf::from("a.rs")]).unwrap();
    let mut last = LastSeen::default();
    let kept = vec![DiffLine {
        kind: DiffLineKind::Context,
        text: "the last good document".into(),
    }];
    let mut lines = kept.clone();
    let mut state = DiffState::new();
    let runner = MockProcessRunner::new(vec![MockProcessRunner::fail(
        "fatal: unable to read index.lock",
    )]);

    refresh(Path::new(&root), &runner, &mut last, &mut lines, &mut state);

    assert_eq!(lines, kept);
    let notice = state.notice.expect("a notice");
    assert!(notice.contains("index.lock"), "{notice}");
}

#[test]
fn the_notice_clears_on_the_next_good_pass() {
    let mut pane = Pane::new();
    pane.repo.append("seed.txt", "more\n");
    pane.open(&["seed.txt"]);
    pane.state.notice = Some("earlier failure".into());

    pane.refresh();

    assert_eq!(pane.state.notice, None);
}
