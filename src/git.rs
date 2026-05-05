//! Small git plumbing helpers shared across the crate.

use crate::process::ProcessRunner;

/// Detect the default branch for a repo by inspecting `origin/HEAD`.
///
/// Falls back to `"main"` when the remote ref is missing or the command
/// fails (no remote, fresh clone without `git remote set-head`, etc.).
pub fn detect_default_branch(repo_path: &str, runner: &dyn ProcessRunner) -> String {
    if let Ok(output) = runner.run(
        "git",
        &["-C", repo_path, "symbolic-ref", "refs/remotes/origin/HEAD"],
    ) {
        if output.status.success() {
            let refname = String::from_utf8_lossy(&output.stdout).trim().to_string();
            // e.g. "refs/remotes/origin/master" → "master"
            if let Some(branch) = refname.rsplit('/').next() {
                if !branch.is_empty() {
                    return branch.to_string();
                }
            }
        }
    }
    "main".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::MockProcessRunner;

    #[test]
    fn detect_default_branch_returns_remote_head_when_set() {
        let runner = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
            b"refs/remotes/origin/master\n",
        )]);
        assert_eq!(detect_default_branch("/repo", &runner), "master");
    }

    #[test]
    fn detect_default_branch_falls_back_when_origin_head_missing() {
        let runner = MockProcessRunner::new(vec![MockProcessRunner::fail(
            "fatal: ref refs/remotes/origin/HEAD is not a symbolic ref",
        )]);
        assert_eq!(detect_default_branch("/repo", &runner), "main");
    }

    #[test]
    fn detect_default_branch_falls_back_when_runner_errors() {
        let runner = MockProcessRunner::new(vec![Err(anyhow::anyhow!("git not on PATH"))]);
        assert_eq!(detect_default_branch("/repo", &runner), "main");
    }

    #[test]
    fn detect_default_branch_invokes_correct_git_command() {
        let runner = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
            b"refs/remotes/origin/main\n",
        )]);
        let _ = detect_default_branch("/some/repo", &runner);
        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "git");
        assert_eq!(
            calls[0].1,
            vec![
                "-C".to_string(),
                "/some/repo".to_string(),
                "symbolic-ref".to_string(),
                "refs/remotes/origin/HEAD".to_string(),
            ]
        );
    }
}
