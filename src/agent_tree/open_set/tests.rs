use super::*;
// The one on-disk encoding of a linked worktree, shared with every other
// test of this placement rule — see its own doc comment.
use crate::worktree_admin::tests::make_linked_worktree;

// ---- the selected source (AgentTreeSourceIsOneSelection) ----------

#[test]
fn a_selected_commit_reads_back_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let (worktree, _) = make_linked_worktree(dir.path(), "42-fix");
    let id = "0123456789abcdef0123456789abcdef01234567";

    write_selected_source(&worktree, Some(id)).unwrap();

    assert_eq!(read_selected_source(&worktree).as_deref(), Some(id));
}

/// The id is handed to git as an argument, so a recorded value that is not
/// a hex object id (an option such as `--output=x`) is not trusted.
#[test]
fn a_selected_source_that_is_not_a_hex_id_reads_as_unstaged_work() {
    let dir = tempfile::tempdir().unwrap();
    let (worktree, _) = make_linked_worktree(dir.path(), "42-fix");

    write_selected_source(&worktree, Some("--output=/tmp/x")).unwrap();

    assert_eq!(read_selected_source(&worktree), None);
}

/// Selecting "unstaged work" again replaces the commit rather than
/// leaving it behind.
#[test]
fn selecting_unstaged_work_reads_back_as_no_commit() {
    let dir = tempfile::tempdir().unwrap();
    let (worktree, _) = make_linked_worktree(dir.path(), "42-fix");
    write_selected_source(&worktree, Some("abc")).unwrap();

    write_selected_source(&worktree, None).unwrap();

    assert_eq!(read_selected_source(&worktree), None);
}

/// Unstaged work is the default source: nothing recorded reads as it.
#[test]
fn an_unwritten_selection_reads_as_unstaged_work() {
    let dir = tempfile::tempdir().unwrap();
    let (worktree, _) = make_linked_worktree(dir.path(), "42-fix");
    assert_eq!(read_selected_source(&worktree), None);
}

/// The selection and the open set are two things the tree publishes;
/// writing one must not clobber the other.
#[test]
fn the_selection_and_the_open_set_are_independent() {
    let dir = tempfile::tempdir().unwrap();
    let (worktree, _) = make_linked_worktree(dir.path(), "42-fix");

    write_open_set(&worktree, &paths_of(&["a.rs"])).unwrap();
    write_selected_source(&worktree, Some("abc")).unwrap();
    assert_eq!(read_open_set(&worktree), paths_of(&["a.rs"]));

    write_open_set(&worktree, &paths_of(&["b.rs"])).unwrap();
    assert_eq!(read_selected_source(&worktree).as_deref(), Some("abc"));
}

/// OpenSetIsNotWorktreeState covers the selection too: nothing the pane
/// records lands inside the working tree, where git would report it.
#[test]
fn the_selection_is_never_written_inside_the_working_tree() {
    let dir = tempfile::tempdir().unwrap();
    let (worktree, _) = make_linked_worktree(dir.path(), "42-fix");

    write_selected_source(&worktree, Some("abc")).unwrap();

    let stray: Vec<_> = std::fs::read_dir(&worktree)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name != ".git")
        .collect();
    assert!(stray.is_empty(), "written inside the worktree: {stray:?}");
}

/// A pane outside a linked worktree has nowhere to record a selection,
/// and reading reports unstaged work rather than failing.
#[test]
fn a_path_that_is_not_a_linked_worktree_reads_unstaged_work() {
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("main-checkout");
    std::fs::create_dir_all(plain.join(".git")).unwrap();
    let plain = plain.to_string_lossy().into_owned();

    assert_eq!(read_selected_source(&plain), None);
    assert!(write_selected_source(&plain, Some("abc")).is_err());
}

fn paths_of(paths: &[&str]) -> Vec<PathBuf> {
    paths.iter().map(PathBuf::from).collect()
}

#[test]
fn the_open_set_goes_in_the_worktrees_git_admin_directory() {
    let dir = tempfile::tempdir().unwrap();
    let (worktree, admin) = make_linked_worktree(dir.path(), "42-fix");

    assert_eq!(
        open_set_path(&worktree),
        Some(admin.join("dispatch-agent-tree.json"))
    );
}

/// The whole point of the placement: nothing under the admin directory is
/// inside the working tree, so git cannot report it and the agent cannot
/// commit it. See OpenSetIsNotWorktreeState in the spec.
#[test]
fn the_open_set_is_never_written_inside_the_working_tree() {
    let dir = tempfile::tempdir().unwrap();
    let (worktree, _) = make_linked_worktree(dir.path(), "42-fix");

    write_open_set(&worktree, &paths_of(&["a.rs"])).unwrap();

    let path = open_set_path(&worktree).unwrap();
    assert!(
        !path.starts_with(&worktree),
        "{} must not be inside {worktree}",
        path.display()
    );
}

#[test]
fn a_written_set_reads_back_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let (worktree, _) = make_linked_worktree(dir.path(), "42-fix");
    // Deliberately not sorted: the round trip has to preserve the tree's
    // row order, which is what the diff pane renders in.
    let paths = paths_of(&["z.rs", "src/lib.rs", "docs/my notes.md"]);

    write_open_set(&worktree, &paths).unwrap();

    assert_eq!(read_open_set(&worktree), paths);
}

#[test]
fn an_unwritten_set_reads_as_empty() {
    let dir = tempfile::tempdir().unwrap();
    let (worktree, _) = make_linked_worktree(dir.path(), "42-fix");
    assert!(read_open_set(&worktree).is_empty());
}

/// The diff pane reads on its own timer while the tree writes on keypresses.
/// A half-written or hand-mangled file must cost one blank pane, not a
/// crash — soft-fail decoding, as everywhere else in this subsystem.
#[test]
fn an_unreadable_set_reads_as_empty() {
    let dir = tempfile::tempdir().unwrap();
    let (worktree, _) = make_linked_worktree(dir.path(), "42-fix");
    let path = open_set_path(&worktree).unwrap();
    std::fs::write(&path, b"{ not json at all").unwrap();

    assert!(read_open_set(&worktree).is_empty());
}

/// A path that escapes the worktree names nothing the tree ever rendered,
/// so it is dropped rather than handed to a git command.
#[test]
fn a_path_escaping_the_worktree_is_dropped_on_read() {
    let dir = tempfile::tempdir().unwrap();
    let (worktree, _) = make_linked_worktree(dir.path(), "42-fix");
    let path = open_set_path(&worktree).unwrap();
    std::fs::write(&path, br#"["../outside.rs","/etc/passwd","ok.rs"]"#).unwrap();

    assert_eq!(read_open_set(&worktree), paths_of(&["ok.rs"]));
}

#[test]
fn writing_replaces_rather_than_merges() {
    let dir = tempfile::tempdir().unwrap();
    let (worktree, _) = make_linked_worktree(dir.path(), "42-fix");

    write_open_set(&worktree, &paths_of(&["a.rs", "b.rs"])).unwrap();
    write_open_set(&worktree, &paths_of(&["c.rs"])).unwrap();

    assert_eq!(read_open_set(&worktree), paths_of(&["c.rs"]));
}

#[test]
fn clearing_leaves_an_empty_set_behind() {
    let dir = tempfile::tempdir().unwrap();
    let (worktree, _) = make_linked_worktree(dir.path(), "42-fix");
    write_open_set(&worktree, &paths_of(&["a.rs"])).unwrap();

    clear_open_set(&worktree).unwrap();

    assert!(read_open_set(&worktree).is_empty());
}

/// A main checkout's `.git` is a directory, not a pointer file, so there is
/// nowhere to put the set — and reading reports "nothing open" rather than
/// failing, which is what keeps `dispatch agent-tree` usable outside a
/// dispatched worktree.
#[test]
fn a_path_that_is_not_a_linked_worktree_has_nowhere_to_record() {
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("main-checkout");
    std::fs::create_dir_all(plain.join(".git")).unwrap();
    let plain = plain.to_string_lossy().into_owned();

    assert_eq!(open_set_path(&plain), None);
    assert!(read_open_set(&plain).is_empty());
    assert!(write_open_set(&plain, &paths_of(&["a.rs"])).is_err());
    assert!(clear_open_set(&plain).is_ok());
}
