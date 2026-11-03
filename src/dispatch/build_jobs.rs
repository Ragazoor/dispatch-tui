//! The `CARGO_BUILD_JOBS` an agent launch sets, or the decision to set none —
//! see `AgentBuildsRunFewerParallelJobs` in docs/specs/dispatch.allium.
//!
//! Cargo ranks the environment above `.cargo/config.toml`, so a launch that
//! always set the variable would raise a repo's own `build.jobs = 2` to the
//! default. Hence the launch leaves the variable out when a cargo config in
//! reach of the worktree sets `build.jobs`.

use std::path::Path;

/// `dispatch.allium: config.agent_cargo_build_jobs`.
const AGENT_CARGO_BUILD_JOBS: usize = 8;

/// The default job count on a machine with `cores` cores: the configured cap,
/// but never more jobs than cores.
pub(super) fn default_build_jobs(cores: usize) -> usize {
    AGENT_CARGO_BUILD_JOBS.min(cores.max(1))
}

/// Whether cargo config `text` sets `build.jobs`, as `[build]` with `jobs`, or
/// as the dotted `build.jobs` key. A line scan, not a TOML parse: the only
/// question is whether the key is there, and a miss costs one extra variable.
pub(super) fn config_sets_build_jobs(text: &str) -> bool {
    let mut in_build = false;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if let Some(header) = line.strip_prefix('[') {
            in_build = header.trim_end_matches(']').trim() == "build";
            continue;
        }
        let Some((key, _)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if (in_build && key == "jobs") || key.replace(' ', "") == "build.jobs" {
            return true;
        }
    }
    false
}

/// Whether a cargo config file under `dir/.cargo/`, or in any parent of `dir`,
/// or under `cargo_home`, sets `build.jobs`.
pub(crate) fn cargo_config_sets_jobs(dir: &Path, cargo_home: Option<&Path>) -> bool {
    let mut roots: Vec<std::path::PathBuf> = dir.ancestors().map(|d| d.join(".cargo")).collect();
    roots.extend(cargo_home.map(Path::to_path_buf));
    roots.iter().any(|root| {
        ["config.toml", "config"].iter().any(|name| {
            std::fs::read_to_string(root.join(name)).is_ok_and(|t| config_sets_build_jobs(&t))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_eight_jobs_on_a_big_machine() {
        assert_eq!(default_build_jobs(22), 8);
    }

    #[test]
    fn the_default_never_exceeds_the_core_count() {
        assert_eq!(default_build_jobs(4), 4);
        assert_eq!(default_build_jobs(0), 1);
    }

    #[test]
    fn a_build_table_with_jobs_counts() {
        assert!(config_sets_build_jobs("[build]\njobs = 2\n"));
        assert!(config_sets_build_jobs(
            "[net]\nx = 1\n\n[build]\nrustflags = []\njobs=2 # lean\n"
        ));
    }

    #[test]
    fn a_dotted_key_counts() {
        assert!(config_sets_build_jobs("build.jobs = 2\n"));
    }

    #[test]
    fn jobs_outside_the_build_table_does_not_count() {
        assert!(!config_sets_build_jobs(
            "[build]\nrustflags = []\n[test]\njobs = 2\n"
        ));
        assert!(!config_sets_build_jobs("# jobs = 2\n[build]\n# jobs = 2\n"));
    }

    #[test]
    fn a_repo_config_in_a_parent_directory_is_found() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".cargo")).unwrap();
        std::fs::write(dir.path().join(".cargo/config.toml"), "[build]\njobs = 2\n").unwrap();
        let worktree = dir.path().join("sub/worktree");
        std::fs::create_dir_all(&worktree).unwrap();

        assert!(cargo_config_sets_jobs(&worktree, None));
    }

    #[test]
    fn the_users_cargo_home_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("config.toml"), "[build]\njobs = 2\n").unwrap();

        assert!(cargo_config_sets_jobs(dir.path(), Some(home.path())));
        assert!(!cargo_config_sets_jobs(dir.path(), None));
    }
}
