//! Does the task's repository keep Allium specs?
//!
//! One directory walk under `docs/`, run once per dispatch, that decides which design step
//! the prompt names — see `DesignStepMatchesTheReposSpecs` in
//! `docs/specs/dispatch-prompt.allium`. A repo that keeps specs gets the spec-first
//! sequence (`allium:elicit` → `allium:tend` → `allium:propagate` → implement →
//! `allium:weed`); a repo that keeps none gets `superpowers:brainstorming`
//! instead, and no `allium_instruction` in its trailing block.

use std::path::Path;

use crate::models::expand_tilde;

/// Relative path, from the repo root, under which Allium specs are searched
/// for, at any depth. A repo may keep them in `docs/specs/` or somewhere like
/// `docs/project/specs/`.
const SPEC_DIR: &str = "docs";

/// File extension that marks an Allium spec.
const SPEC_EXT: &str = "allium";

/// True when `repo_path` holds at least one `*.allium` file anywhere under `docs/`.
///
/// Read against the parent repo rather than the freshly provisioned worktree:
/// the two are checkouts of the same repository, and the parent path is the one
/// the task actually carries.
///
/// Every failure to *see* such a file answers `false` — a missing `docs`
/// directory, an empty one, one holding no `.allium` file, and one that cannot
/// be read all take the same branch. There is deliberately no error arm, so the
/// prompt an agent receives never depends on which way a read failed.
pub(super) fn repo_has_allium_specs(repo_path: &str) -> bool {
    // An empty path would join to the bare relative `docs/specs`, answering for
    // dispatch's own working directory rather than the task's repo. Dispatch
    // refuses to launch a task with no repo_path at all, so `false` here is the
    // answer for a path that cannot be checked, not a reachable prompt shape.
    if repo_path.is_empty() {
        return false;
    }
    let dir = Path::new(&expand_tilde(repo_path)).join(SPEC_DIR);
    dir_holds_spec(&dir)
}

/// Walks `dir` recursively. Unreadable directories count as empty. Symlinked
/// directories are not followed, so a link cycle cannot loop the walk.
fn dir_holds_spec(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        let path = e.path();
        let is_real_dir = e.file_type().is_ok_and(|t| t.is_dir());
        if is_real_dir {
            return dir_holds_spec(&path);
        }
        // `is_file` follows symlinks, so a linked-in spec still counts, while a
        // directory that merely happens to be named `*.allium` does not.
        path.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case(SPEC_EXT))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// Creates `<root>/docs/specs` and returns it.
    fn spec_dir(root: &Path) -> std::path::PathBuf {
        let dir = root.join("docs/specs");
        fs::create_dir_all(&dir).expect("create docs/specs");
        dir
    }

    /// The subject under test, taking the path the way a `Task` carries it.
    fn has_specs(repo: &Path) -> bool {
        repo_has_allium_specs(repo.to_str().expect("utf8 path"))
    }

    #[test]
    fn true_when_docs_specs_holds_an_allium_file() {
        let repo = tempdir().expect("tempdir");
        let dir = spec_dir(repo.path());
        fs::write(dir.join("dispatch.allium"), "-- allium: 3\n").expect("write spec");
        assert!(has_specs(repo.path()));
    }

    #[test]
    fn false_when_docs_specs_is_missing() {
        let repo = tempdir().expect("tempdir");
        assert!(!has_specs(repo.path()));
    }

    #[test]
    fn false_when_docs_specs_is_empty() {
        let repo = tempdir().expect("tempdir");
        spec_dir(repo.path());
        assert!(!has_specs(repo.path()));
    }

    #[test]
    fn false_when_docs_specs_holds_no_allium_file() {
        let repo = tempdir().expect("tempdir");
        let dir = spec_dir(repo.path());
        fs::write(dir.join("README.md"), "not a spec").expect("write file");
        fs::write(dir.join("notes.txt"), "not a spec either").expect("write file");
        assert!(!has_specs(repo.path()));
    }

    /// A directory merely *named* `something.allium` is not a spec file, so it
    /// does not flip the answer.
    #[test]
    fn false_when_the_only_allium_entry_is_a_directory() {
        let repo = tempdir().expect("tempdir");
        let dir = spec_dir(repo.path());
        fs::create_dir(dir.join("named.allium")).expect("create allium-named dir");
        assert!(!has_specs(repo.path()));
    }

    /// A repo may keep its specs anywhere under `docs/`, such as
    /// `docs/project/specs/` in allium-tools.
    #[test]
    fn true_when_a_spec_sits_in_a_nested_docs_directory() {
        let repo = tempdir().expect("tempdir");
        let dir = repo.path().join("docs/project/specs");
        fs::create_dir_all(&dir).expect("create nested dir");
        fs::write(dir.join("x.allium"), "-- allium: 3\n").expect("write spec");
        assert!(has_specs(repo.path()));
    }

    #[test]
    fn false_when_an_allium_file_sits_outside_docs() {
        let repo = tempdir().expect("tempdir");
        fs::create_dir_all(repo.path().join("src")).expect("create src");
        fs::write(repo.path().join("src/x.allium"), "-- allium: 3\n").expect("write file");
        fs::write(repo.path().join("top.allium"), "-- allium: 3\n").expect("write file");
        assert!(!has_specs(repo.path()));
    }

    #[test]
    fn false_when_the_repo_path_does_not_exist() {
        let repo = tempdir().expect("tempdir");
        let missing = repo.path().join("no-such-repo");
        assert!(!has_specs(&missing));
    }

    #[test]
    fn false_when_the_repo_path_is_a_file_not_a_directory() {
        let repo = tempdir().expect("tempdir");
        let file = repo.path().join("a-file");
        fs::write(&file, "not a repo").expect("write file");
        assert!(!has_specs(&file));
    }

    #[test]
    fn false_when_the_repo_path_is_empty() {
        assert!(!repo_has_allium_specs(""));
    }

    /// This repository keeps its own specs in `docs/specs`, so a dispatch
    /// against it must take the spec-first branch. Guards the constant pair
    /// against a rename that would silently downgrade every dispatch here.
    #[test]
    fn true_for_this_repository() {
        assert!(repo_has_allium_specs(env!("CARGO_MANIFEST_DIR")));
    }
}
