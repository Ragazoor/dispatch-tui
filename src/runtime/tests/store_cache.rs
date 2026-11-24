//! The saved-store cache the upgrade smoke test starts from — `startup.allium`:
//! `ThePreviousStoreIsTheModuleTheBranchLeftMainAt`.
//!
//! One entry per module hash, in `.test-stores/` beside the checkout's shared
//! git directory, so every worktree of a checkout reads and fills the same
//! cache. `.gitignore` keeps it out of git.

use std::path::{Path, PathBuf};

use crate::spacetime::managed_store::module_hash;

/// How many entries the cache keeps, most recently used first.
pub(super) const KEEP: usize = 5;

/// What a test touches in an entry each time it uses it; pruning orders by it.
const USED_MARKER: &str = "last-used";

/// Where an entry is put together before it is moved into place.
const BUILDING_PREFIX: &str = ".building-";

/// A build left this long was abandoned by a run that died mid-build.
const ABANDONED_BUILD: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// Git's stdout for `args` in `repo`, or `None` when it fails.
fn git_stdout(repo: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let out = crate::test_git::try_git(repo, args);
    out.status.success().then_some(out.stdout)
}

/// `.test-stores/` beside the shared git directory of the checkout `repo`
/// is in. `None` outside a git checkout.
pub(super) fn cache_root(repo: &Path) -> Option<PathBuf> {
    let common = git_stdout(
        repo,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let common = PathBuf::from(String::from_utf8_lossy(&common).trim());
    Some(common.parent()?.join(".test-stores"))
}

/// The hash of the module this branch upgrades from: the one at the commit it
/// shares with local `main` (`git merge-base main HEAD`). `None` without a
/// local `main` or a shared commit.
pub(super) fn base_module_hash(repo: &Path) -> Option<String> {
    let base = git_stdout(repo, &["merge-base", "main", "HEAD"])?;
    let base = String::from_utf8_lossy(&base).trim().to_string();
    let module = git_stdout(
        repo,
        &["show", &format!("{base}:src/spacetime/module.wasm")],
    )?;
    Some(module_hash(&module))
}

fn entry_dir(root: &Path, hash: &str) -> PathBuf {
    root.join(hash.trim_start_matches("sha256:"))
}

fn mark_used(entry: &Path) {
    let _ = std::fs::write(entry.join(USED_MARKER), b"");
}

/// The saved entry for `hash`, marked as used now, if there is one.
pub(super) fn take(root: &Path, hash: &str) -> Option<PathBuf> {
    let entry = entry_dir(root, hash);
    if !entry.join(USED_MARKER).is_file() {
        return None;
    }
    mark_used(&entry);
    Some(entry)
}

/// The entry for `hash`, built by `build` into the empty directory it is
/// given when the cache has none. Two processes may build at once; the first
/// to move its build into place wins and the other's is dropped. Prunes to
/// [`KEEP`].
pub(super) async fn take_or_build<F>(
    root: &Path,
    hash: &str,
    build: impl FnOnce(PathBuf) -> F,
) -> std::io::Result<PathBuf>
where
    F: std::future::Future<Output = ()>,
{
    if let Some(entry) = take(root, hash) {
        return Ok(entry);
    }
    std::fs::create_dir_all(root)?;
    let staged = tempfile::Builder::new()
        .prefix(BUILDING_PREFIX)
        .tempdir_in(root)?
        .keep();
    build(staged.clone()).await;
    let entry = entry_dir(root, hash);
    if std::fs::rename(&staged, &entry).is_err() {
        std::fs::remove_dir_all(&staged)?;
    }
    mark_used(&entry);
    prune(root, KEEP, std::time::SystemTime::now());
    Ok(entry)
}

/// Drop every entry but the `keep` most recently used, and every build
/// abandoned for longer than [`ABANDONED_BUILD`]. Builds in progress are not
/// entries.
pub(super) fn prune(root: &Path, keep: usize, now: std::time::SystemTime) {
    let Ok(dir) = std::fs::read_dir(root) else {
        return;
    };
    let (builds, entries): (Vec<_>, Vec<_>) = dir
        .flatten()
        .partition(|e| e.file_name().to_string_lossy().starts_with(BUILDING_PREFIX));
    for build in builds {
        let age = build
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok());
        if age.is_some_and(|age| age > ABANDONED_BUILD) {
            let _ = std::fs::remove_dir_all(build.path());
        }
    }
    let mut entries: Vec<(std::time::SystemTime, PathBuf)> = entries
        .into_iter()
        .filter_map(|e| {
            let used = e
                .path()
                .join(USED_MARKER)
                .metadata()
                .ok()?
                .modified()
                .ok()?;
            Some((used, e.path()))
        })
        .collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.0));
    for (_, stale) in entries.into_iter().skip(keep) {
        let _ = std::fs::remove_dir_all(stale);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_git::git;
    use std::time::{Duration, SystemTime};

    fn used_at(root: &Path, hash: &str, secs: u64) {
        std::fs::File::options()
            .write(true)
            .open(entry_dir(root, hash).join(USED_MARKER))
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
            .unwrap();
    }

    async fn build_marker(dir: PathBuf) {
        std::fs::write(dir.join("store"), b"built").unwrap();
    }

    #[test]
    fn a_worktree_shares_the_main_checkouts_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "x"]);
        git(&repo, &["worktree", "add", "-q", ".worktrees/w", "-b", "w"]);
        let expected = repo.canonicalize().unwrap().join(".test-stores");
        assert_eq!(cache_root(&repo), Some(expected.clone()));
        assert_eq!(cache_root(&repo.join(".worktrees/w")), Some(expected));
    }

    #[test]
    fn outside_a_checkout_there_is_no_cache() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(cache_root(tmp.path()), None);
    }

    fn commit_module(repo: &Path, contents: &[u8]) {
        std::fs::create_dir_all(repo.join("src/spacetime")).unwrap();
        std::fs::write(repo.join("src/spacetime/module.wasm"), contents).unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-q", "-m", "x"]);
    }

    #[test]
    fn a_branch_upgrades_from_the_module_where_it_left_main() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        git(repo, &["init", "-q", "-b", "main"]);
        commit_module(repo, b"old main");
        git(repo, &["checkout", "-q", "-b", "branch"]);
        commit_module(repo, b"branch");
        git(repo, &["checkout", "-q", "main"]);
        commit_module(repo, b"new main");
        git(repo, &["checkout", "-q", "branch"]);
        assert_eq!(base_module_hash(repo), Some(module_hash(b"old main")));
    }

    #[test]
    fn a_branch_that_merged_main_upgrades_from_mains_module() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        git(repo, &["init", "-q", "-b", "main"]);
        commit_module(repo, b"old main");
        git(repo, &["checkout", "-q", "-b", "branch"]);
        git(repo, &["checkout", "-q", "main"]);
        commit_module(repo, b"new main");
        git(repo, &["checkout", "-q", "branch"]);
        git(repo, &["merge", "-q", "--ff-only", "main"]);
        commit_module(repo, b"branch");
        assert_eq!(base_module_hash(repo), Some(module_hash(b"new main")));
    }

    #[test]
    fn without_a_local_main_there_is_no_base_module() {
        let tmp = tempfile::tempdir().unwrap();
        git(tmp.path(), &["init", "-q", "-b", "trunk"]);
        commit_module(tmp.path(), b"trunk");
        assert_eq!(base_module_hash(tmp.path()), None);
    }

    #[test]
    fn an_abandoned_build_is_pruned() {
        let tmp = tempfile::tempdir().unwrap();
        let building = tmp.path().join(format!("{BUILDING_PREFIX}old"));
        std::fs::create_dir_all(&building).unwrap();
        let modified = std::fs::metadata(&building).unwrap().modified().unwrap();
        prune(
            tmp.path(),
            KEEP,
            modified + ABANDONED_BUILD + Duration::from_secs(1),
        );
        assert!(!building.exists());
    }

    #[test]
    fn a_missing_entry_is_not_taken() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(take(tmp.path(), "sha256:aa"), None);
    }

    #[tokio::test]
    async fn an_entry_is_built_once_and_then_taken_from_the_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".test-stores");
        let mut builds = 0;
        let first = take_or_build(&root, "sha256:aa", |d| {
            builds += 1;
            build_marker(d)
        })
        .await
        .unwrap();
        let second = take_or_build(&root, "sha256:aa", |d| {
            builds += 1;
            build_marker(d)
        })
        .await
        .unwrap();
        assert_eq!(builds, 1);
        assert_eq!(first, second);
        assert_eq!(std::fs::read(first.join("store")).unwrap(), b"built");
        assert_eq!(take(&root, "sha256:aa"), Some(first));
    }

    #[tokio::test]
    async fn a_build_that_loses_the_race_leaves_the_winners_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let entry = take_or_build(root, "sha256:aa", |d| {
            // Another process finishes first, while this one is still building.
            let winner = entry_dir(root, "sha256:aa");
            std::fs::create_dir_all(&winner).unwrap();
            std::fs::write(winner.join("store"), b"winner").unwrap();
            std::fs::write(winner.join(USED_MARKER), b"").unwrap();
            build_marker(d)
        })
        .await
        .unwrap();
        assert_eq!(std::fs::read(entry.join("store")).unwrap(), b"winner");
        let left: Vec<_> = std::fs::read_dir(root).unwrap().flatten().collect();
        assert_eq!(left.len(), 1, "the losing build is removed: {left:?}");
    }

    #[tokio::test]
    async fn pruning_keeps_the_most_recently_used_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for (i, hash) in ["a", "b", "c", "d"].iter().enumerate() {
            take_or_build(root, hash, build_marker).await.unwrap();
            used_at(root, hash, 100 + i as u64);
        }
        used_at(root, "a", 200);
        prune(root, 2, SystemTime::now());
        let mut left: Vec<String> = std::fs::read_dir(root)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["a", "d"]);
    }

    #[tokio::test]
    async fn pruning_leaves_builds_in_progress() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let building = root.join(format!("{BUILDING_PREFIX}x"));
        std::fs::create_dir_all(&building).unwrap();
        take_or_build(root, "a", build_marker).await.unwrap();
        prune(root, 0, SystemTime::now());
        assert!(building.is_dir());
        assert!(!entry_dir(root, "a").exists());
    }

    #[tokio::test]
    async fn building_prunes_to_the_cache_size() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for i in 0..KEEP + 2 {
            let hash = format!("h{i}");
            take_or_build(root, &hash, build_marker).await.unwrap();
            used_at(root, &hash, 100 + i as u64);
        }
        assert_eq!(std::fs::read_dir(root).unwrap().count(), KEEP);
        assert!(take(root, &format!("h{}", KEEP + 1)).is_some());
    }
}
