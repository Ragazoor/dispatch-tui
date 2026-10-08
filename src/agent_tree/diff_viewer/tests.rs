use super::*;
use crate::agent_tree::changes::GIT_TIMEOUT;
use crate::process::MockProcessRunner;

/// A commit the user has selected in the tree's commits section.
const COMMIT: &str = "3333333333333333333333333333333333333333";

fn no_untracked() -> BTreeSet<PathBuf> {
    BTreeSet::new()
}

fn diff_rig(stdout: &str) -> MockProcessRunner {
    MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(stdout.as_bytes())])
}

const A_PATCH: &str = "diff --git a/a.rs b/a.rs\n@@ -1 +1 @@\n-old\n+new\n";

#[test]
fn a_tracked_files_diff_comes_back_as_its_body() {
    let runner = diff_rig(A_PATCH);

    let diff = file_diff(
        Path::new("/wt"),
        None,
        Path::new("a.rs"),
        &no_untracked(),
        &runner,
    )
    .unwrap()
    .unwrap();

    assert_eq!(diff.content, FileDiffContent::Shown(A_PATCH.to_owned()));
}

/// Unstaged work is the working tree against the INDEX, so the diff names
/// no revision — the same comparison the tree's badge answers
/// (RefreshAgentTreeDiff's "The same baseline, asked the same way"). `--`
/// still separates the pathspec, so a path that looks like a ref is read
/// as a path.
#[test]
fn unstaged_work_is_diffed_against_the_index_naming_no_revision() {
    let runner = diff_rig(A_PATCH);

    file_diff(
        Path::new("/wt"),
        None,
        Path::new("main"),
        &no_untracked(),
        &runner,
    )
    .unwrap();

    assert_eq!(
        runner.flattened_calls(),
        vec!["git -C /wt diff --no-renames -- main".to_string()]
    );
}

/// With a commit selected, the one diff asked for names that commit,
/// keeps rename detection off, and still bounds the path with `--`.
#[test]
fn a_selected_commits_diff_names_the_commit_and_the_path() {
    let runner = diff_rig(A_PATCH);

    let diff = file_diff(
        Path::new("/wt"),
        Some(COMMIT),
        Path::new("main"),
        &no_untracked(),
        &runner,
    )
    .unwrap()
    .unwrap();

    assert_eq!(diff.content, FileDiffContent::Shown(A_PATCH.to_owned()));
    let calls = runner.flattened_calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0].contains(COMMIT), "{calls:?}");
    assert!(calls[0].contains("--no-renames"), "{calls:?}");
    assert!(calls[0].ends_with("-- main"), "{calls:?}");
}

#[test]
fn a_binary_file_is_refused_with_its_own_reason() {
    let runner = diff_rig(
        "diff --git a/logo.png b/logo.png\nBinary files a/logo.png and b/logo.png differ\n",
    );

    let diff = file_diff(
        Path::new("/wt"),
        None,
        Path::new("logo.png"),
        &no_untracked(),
        &runner,
    )
    .unwrap()
    .unwrap();

    assert_eq!(diff.content, FileDiffContent::Refused(DiffRefusal::Binary));
}

/// The notice is matched at the START of a line, so a patch that merely
/// contains the sentence — a diff of this very file, say — is still shown
/// as a patch.
#[test]
fn a_patch_mentioning_the_binary_notice_is_not_mistaken_for_one() {
    let body = "diff --git a/x.rs b/x.rs\n@@ -1 +1 @@\n+// Binary files a and b differ\n";
    let runner = diff_rig(body);

    let diff = file_diff(
        Path::new("/wt"),
        None,
        Path::new("x.rs"),
        &no_untracked(),
        &runner,
    )
    .unwrap()
    .unwrap();

    assert_eq!(diff.content, FileDiffContent::Shown(body.to_owned()));
}

#[test]
fn a_diff_over_the_cap_is_refused_rather_than_rendered() {
    let huge = format!(
        "diff --git a/big.rs b/big.rs\n{}",
        "+x\n".repeat(DIFF_MAX_BYTES)
    );
    let runner = diff_rig(&huge);

    let diff = file_diff(
        Path::new("/wt"),
        None,
        Path::new("big.rs"),
        &no_untracked(),
        &runner,
    )
    .unwrap()
    .unwrap();

    assert_eq!(
        diff.content,
        FileDiffContent::Refused(DiffRefusal::TooLarge)
    );
}

/// A path the user opened and the agent then reverted, staged or
/// committed. Not an error, not a placeholder, and NOT a reason to close
/// it — see OpenDiffPathsMaySurviveTheirFiles in docs/specs/agent-tree.allium.
#[test]
fn a_path_git_no_longer_reports_shows_nothing_at_all() {
    let runner = diff_rig("");

    let diff = file_diff(
        Path::new("/wt"),
        None,
        Path::new("reverted.rs"),
        &no_untracked(),
        &runner,
    )
    .unwrap();

    assert_eq!(diff, None);
}

#[test]
fn a_git_failure_is_an_error_not_a_silently_empty_diff() {
    let runner = MockProcessRunner::new(vec![MockProcessRunner::fail("fatal: bad object")]);

    let err = file_diff(
        Path::new("/wt"),
        None,
        Path::new("a.rs"),
        &no_untracked(),
        &runner,
    )
    .expect_err("a failing git must not read as an empty diff");

    assert!(format!("{err:#}").contains("bad object"), "got {err:#}");
}

#[test]
fn every_diff_query_is_bounded_by_the_shared_timeout() {
    let runner = diff_rig(A_PATCH);
    file_diff(
        Path::new("/wt"),
        None,
        Path::new("a.rs"),
        &no_untracked(),
        &runner,
    )
    .unwrap();
    assert_eq!(runner.recorded_timeouts(), vec![Some(GIT_TIMEOUT)]);
}

// -- untracked_paths ---------------------------------------------------

/// One listing for the whole rebuild, and BOUNDED by the open paths: only
/// membership for those is ever asked, and an unbounded listing walks every
/// untracked file in the worktree to answer it.
#[test]
fn the_untracked_listing_is_taken_once_and_bounded_by_the_open_paths() {
    let runner = diff_rig("new.rs\0docs/my notes.md\0");
    let open = open_set(&["new.rs", "docs/my notes.md", "tracked.rs"]);

    let paths = untracked_paths(Path::new("/wt"), &open, &runner).unwrap();

    assert_eq!(paths, untracked_set(&["new.rs", "docs/my notes.md"]));
    assert_eq!(
        runner.flattened_calls(),
        vec![concat!(
            "git -C /wt ls-files --others --exclude-standard -z -- ",
            // In the order published, not sorted — the listing is bounded
            // by the open paths and does not care which order they come in.
            "new.rs docs/my notes.md tracked.rs"
        )
        .to_string()]
    );
}

/// The pathspec is the open set, so a path git does not report back is
/// tracked.
#[test]
fn a_tracked_open_path_is_absent_from_the_untracked_answer() {
    let runner = diff_rig("");
    let paths = untracked_paths(Path::new("/wt"), &open_set(&["tracked.rs"]), &runner).unwrap();
    assert!(paths.is_empty());
}
