use std::collections::HashMap;

use crate::git::detect_default_branch;
use crate::process::ProcessRunner;

/// Resolve a base branch for each `repo_paths[i]`, caching by unique path so
/// `git symbolic-ref` is invoked at most once per distinct repo. Empty paths
/// (unresolved repos) get `"main"` without shelling out.
pub(crate) fn resolve_base_branches(
    repo_paths: &[String],
    runner: &dyn ProcessRunner,
) -> Vec<String> {
    let mut cache: HashMap<&str, String> = HashMap::new();
    repo_paths
        .iter()
        .map(|path| {
            cache
                .entry(path.as_str())
                .or_insert_with(|| {
                    if path.is_empty() {
                        "main".to_string()
                    } else {
                        detect_default_branch(path, runner)
                    }
                })
                .clone()
        })
        .collect()
}

/// A `ProcessRunner` that always fails — used in tests that only need the
/// `"main"` fallback and don't want to set up git subprocess stubs.
#[cfg(test)]
pub(super) struct AlwaysFailRunner;

#[cfg(test)]
impl ProcessRunner for AlwaysFailRunner {
    fn run(&self, _: &str, _: &[&str]) -> anyhow::Result<std::process::Output> {
        crate::process::MockProcessRunner::fail("not a git repo")
    }
}

/// Execute the feed shell command and return raw stdout bytes on success.
///
/// Logs a warning and returns `None` on spawn failure or non-zero exit.
pub(super) async fn exec_feed_command(
    cmd: &str,
    epic_id: i64,
    epic_title: &str,
) -> Option<Vec<u8>> {
    let output = match tokio::process::Command::new("sh")
        .args(["-c", cmd])
        .output()
        .await
    {
        Ok(o) => o,
        Err(err) => {
            tracing::warn!(
                epic_id,
                epic_title,
                "FeedRunner: failed to spawn command: {err:#}"
            );
            return None;
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(
            epic_id,
            epic_title,
            "FeedRunner: command exited non-zero: {stderr}"
        );
        return None;
    }

    Some(output.stdout)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use super::*;
    use crate::process::{MockProcessRunner, ProcessRunner};

    struct FixedBranchRunner(std::collections::HashMap<String, String>);

    impl FixedBranchRunner {
        fn new(pairs: &[(&str, &str)]) -> Self {
            Self(
                pairs
                    .iter()
                    .map(|(p, b)| (p.to_string(), b.to_string()))
                    .collect(),
            )
        }
    }

    impl ProcessRunner for FixedBranchRunner {
        fn run(&self, _program: &str, args: &[&str]) -> anyhow::Result<std::process::Output> {
            let path = args.get(1).copied().unwrap_or("");
            match self.0.get(path) {
                Some(branch) => MockProcessRunner::ok_with_stdout(
                    format!("refs/remotes/origin/{branch}\n").as_bytes(),
                ),
                None => MockProcessRunner::fail("unknown repo"),
            }
        }
    }

    struct CountingRunner(Arc<AtomicUsize>);

    impl ProcessRunner for CountingRunner {
        fn run(&self, _: &str, _: &[&str]) -> anyhow::Result<std::process::Output> {
            self.0.fetch_add(1, Ordering::SeqCst);
            MockProcessRunner::ok_with_stdout(b"refs/remotes/origin/main\n")
        }
    }

    #[test]
    fn empty_path_resolves_to_main_without_calling_runner() {
        let paths = vec!["".to_string(), "".to_string()];
        let branches = resolve_base_branches(&paths, &AlwaysFailRunner);
        assert_eq!(branches, vec!["main", "main"]);
    }

    #[test]
    fn known_path_resolves_to_configured_branch() {
        let runner = FixedBranchRunner::new(&[("/repo/a", "develop")]);
        let paths = vec!["/repo/a".to_string()];
        let branches = resolve_base_branches(&paths, &runner);
        assert_eq!(branches, vec!["develop"]);
    }

    #[test]
    fn same_path_queried_only_once() {
        let counter = Arc::new(AtomicUsize::new(0));
        let runner = CountingRunner(counter.clone());
        let paths = vec!["/repo".to_string(), "/repo".to_string(), "/repo".to_string()];
        let _ = resolve_base_branches(&paths, &runner);
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "runner called more than once for the same path"
        );
    }

    #[test]
    fn unknown_path_falls_back_to_main() {
        let paths = vec!["/unknown/repo".to_string()];
        let branches = resolve_base_branches(&paths, &AlwaysFailRunner);
        assert_eq!(branches, vec!["main"]);
    }
}
