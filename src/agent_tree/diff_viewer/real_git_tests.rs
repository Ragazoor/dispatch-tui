use super::*;
use crate::agent_tree::test_repo::TestRepo;
use crate::process::RealProcessRunner;

fn diff_of(
    repo: &TestRepo,
    commit: Option<&str>,
    path: &str,
    untracked: &[&str],
) -> Option<FileDiff> {
    file_diff(
        repo.root(),
        commit,
        Path::new(path),
        &untracked_set(untracked),
        &RealProcessRunner::default(),
    )
    .expect("file_diff")
}

fn body(diff: Option<FileDiff>) -> String {
    match diff.map(|d| d.content) {
        Some(FileDiffContent::Shown(body)) => body,
        other => panic!("expected a shown diff, got {other:?}"),
    }
}

/// RefreshAgentTreeDiff's "Untracked files": an open untracked path is a
/// diff against nothing — every line an addition. Not a refusal, and no
/// advice to stage it: in this view staging would remove it.
#[test]
fn an_untracked_file_is_shown_whole_as_additions() {
    let repo = TestRepo::new();
    repo.write("new.rs", "one\ntwo\n");

    let body = body(diff_of(&repo, None, "new.rs", &["new.rs"]));

    let added: Vec<&str> = body
        .lines()
        .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
        .collect();
    assert_eq!(added, vec!["+one", "+two"], "{body}");
    assert!(!body
        .lines()
        .any(|l| l.starts_with('-') && !l.starts_with("---")));
}

/// Both refusals apply to an untracked file's contents as to any diff.
#[test]
fn an_untracked_binary_file_is_refused_as_binary() {
    let repo = TestRepo::new();
    repo.write_bytes("logo.png", b"\x89PNG\0\x01\x02\x03\0binary");

    let diff = diff_of(&repo, None, "logo.png", &["logo.png"]).expect("a refusal");

    assert_eq!(diff.content, FileDiffContent::Refused(DiffRefusal::Binary));
}

#[test]
fn an_untracked_file_too_large_to_show_is_refused() {
    let repo = TestRepo::new();
    repo.write("huge.txt", &"x\n".repeat(DIFF_MAX_BYTES));

    let diff = diff_of(&repo, None, "huge.txt", &["huge.txt"]).expect("a refusal");

    assert_eq!(
        diff.content,
        FileDiffContent::Refused(DiffRefusal::TooLarge)
    );
}

/// Unstaged work shows the unstaged hunk only — never staged lines under
/// a row that exists because of an unstaged one.
#[test]
fn only_the_unstaged_hunk_of_a_file_is_shown() {
    let repo = TestRepo::new();
    repo.write("seed.txt", "seed\nstaged line\n");
    repo.git(&["add", "seed.txt"]);
    repo.append("seed.txt", "unstaged line\n");

    let body = body(diff_of(&repo, None, "seed.txt", &[]));

    assert!(body.contains("+unstaged line"), "{body}");
    assert!(!body.contains("+staged line"), "{body}");
}

/// A file whose whole change is staged has nothing to show for unstaged
/// work — the same answer as a reverted one.
#[test]
fn a_fully_staged_file_has_nothing_to_show() {
    let repo = TestRepo::new();
    repo.write("seed.txt", "seed\nstaged line\n");
    repo.git(&["add", "seed.txt"]);

    assert_eq!(diff_of(&repo, None, "seed.txt", &[]), None);
}

/// A selected commit's diff is that commit against its parent, whatever
/// the working tree holds meanwhile.
#[test]
fn a_selected_commits_diff_is_that_commit_against_its_parent() {
    let repo = TestRepo::new();
    repo.write("a.rs", "a1\n");
    repo.commit_all("first");
    repo.write("a.rs", "a1\na2\n");
    let second = repo.commit_all("second");
    repo.write("a.rs", "work in progress\n");

    let body = body(diff_of(&repo, Some(&second), "a.rs", &[]));

    assert!(body.contains("+a2"), "{body}");
    assert!(!body.contains("work in progress"), "{body}");
}

/// A root commit is diffed against the empty tree.
#[test]
fn a_root_commits_file_is_diffed_against_nothing() {
    let repo = TestRepo::new();
    let root_commit = repo.git(&["rev-list", "--max-parents=0", "HEAD"]);

    let body = body(diff_of(&repo, Some(root_commit.trim()), "seed.txt", &[]));

    assert!(body.contains("+seed"), "{body}");
}

/// A path the selected commit did not touch contributes nothing
/// (OpenDiffPathsMaySurviveTheirFiles: "the user selected a source that
/// does not touch it").
#[test]
fn a_path_the_selected_commit_did_not_touch_shows_nothing() {
    let repo = TestRepo::new();
    repo.write("a.rs", "a\n");
    let commit = repo.commit_all("add a");

    assert_eq!(diff_of(&repo, Some(&commit), "seed.txt", &[]), None);
}
