//! The one way a test spawns a plugin script (`plugin/hooks/*.sh` and friends).
//!
//! A dispatched agent's window exports `DISPATCH_*` variables (`DISPATCH_TASK_ID`
//! among them) that the scripts prefer over what a test sets up. Two harnesses
//! once spawned the same scripts with different environment rules, and `cargo
//! test` failed inside every agent window. Both now build their command here, so
//! a test only sees a variable it sets itself: the command starts from an empty
//! environment (`GIT_*` from a hook must not reach the scripts' `git` either).
//!
//! Included by path from `tests/hook_scripts.rs` and `src/setup/hooks/tests.rs`
//! (the second is a unit-test module, which cannot reach `tests/`).

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

/// The names among `vars` that are dispatch's own.
pub fn dispatch_env_names(vars: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    vars.into_iter()
        .filter(|name| name.to_string_lossy().starts_with("DISPATCH_"))
        .collect()
}

/// Remove every `DISPATCH_*` variable named in `vars` from `cmd`'s environment.
pub fn strip_dispatch_env(cmd: &mut Command, vars: impl IntoIterator<Item = OsString>) {
    for name in dispatch_env_names(vars) {
        cmd.env_remove(name);
    }
}

/// The parent variables a plugin script may see. Everything else is dropped:
/// `GIT_*` from a git hook would steer the scripts' own `git` calls at the
/// wrong repository, and `DISPATCH_*` / `CARGO_*` are the parent's, not the test's.
const ALLOWED_ENV: &[&str] = &["HOME", "LANG", "LC_ALL", "TMPDIR", "USER", "TERM"];

/// The `(name, value)` pairs of `vars` whose name is in [`ALLOWED_ENV`].
pub fn kept_env(vars: impl IntoIterator<Item = (OsString, OsString)>) -> Vec<(OsString, OsString)> {
    vars.into_iter()
        .filter(|(name, _)| ALLOWED_ENV.iter().any(|a| name == a))
        .collect()
}

/// `bash <script>` with `PATH` set to `path_env` and an otherwise empty
/// environment, save [`ALLOWED_ENV`]. The caller adds the working directory,
/// stdio and any variable the test means to set (set it after this call).
pub fn plugin_script_command(script: &Path, path_env: &str) -> Command {
    let mut cmd = Command::new("bash");
    cmd.arg(script).env_clear();
    cmd.envs(kept_env(std::env::vars_os()));
    cmd.env("PATH", path_env);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echoed(cmd: &mut Command) -> String {
        let out = cmd.output().unwrap();
        String::from_utf8(out.stdout).unwrap()
    }

    #[test]
    fn only_dispatch_names_are_picked() {
        let names = dispatch_env_names(
            ["DISPATCH_TASK_ID", "PATH", "DISPATCH_X", "MY_DISPATCH_Y"].map(OsString::from),
        );
        assert_eq!(
            names,
            vec![OsString::from("DISPATCH_TASK_ID"), "DISPATCH_X".into()]
        );
    }

    #[test]
    fn a_child_sees_no_dispatch_variable_the_parent_had() {
        // The parent's environment is simulated by naming the variables: the
        // child inherits them from `cmd.env`, as it would from the real parent.
        let mut cmd = Command::new("bash");
        cmd.args([
            "-c",
            "echo \"[${DISPATCH_TASK_ID-unset}][${DISPATCH_OTHER-unset}]\"",
        ])
        .env("DISPATCH_TASK_ID", "7")
        .env("DISPATCH_OTHER", "x");
        strip_dispatch_env(
            &mut cmd,
            ["DISPATCH_TASK_ID", "DISPATCH_OTHER"].map(OsString::from),
        );
        assert_eq!(echoed(&mut cmd).trim(), "[unset][unset]");
    }

    #[test]
    fn only_allow_listed_names_are_kept() {
        let kept = kept_env(
            [
                "HOME",
                "GIT_DIR",
                "GIT_INDEX_FILE",
                "CARGO_HOME",
                "DISPATCH_X",
                "LANG",
            ]
            .map(|n| (OsString::from(n), OsString::from("v"))),
        );
        let names: Vec<_> = kept.iter().map(|(n, _)| n.to_string_lossy()).collect();
        assert_eq!(names, vec!["HOME", "LANG"]);
    }

    #[test]
    fn a_script_sees_no_git_cargo_or_dispatch_variable_of_the_parent() {
        // cargo sets CARGO_* in every test process, so the parent really has
        // names that must not reach the child.
        let script =
            std::env::temp_dir().join(format!("script_command_env_{}.sh", std::process::id()));
        std::fs::write(&script, "env\n").unwrap();
        let out = echoed(&mut plugin_script_command(&script, "/usr/bin:/bin"));
        let _ = std::fs::remove_file(&script);
        let leaked: Vec<_> = out
            .lines()
            .filter(|l| {
                ["CARGO", "GIT_", "DISPATCH_", "RUST"]
                    .iter()
                    .any(|p| l.starts_with(p))
            })
            .collect();
        assert!(leaked.is_empty(), "leaked into the script: {leaked:?}");
    }

    #[test]
    fn the_plugin_command_keeps_path_and_runs_bash() {
        let cmd = plugin_script_command(Path::new("x.sh"), "/a:/b");
        assert_eq!(cmd.get_program(), "bash");
        let path = cmd
            .get_envs()
            .find(|(k, _)| *k == "PATH")
            .and_then(|(_, v)| v);
        assert_eq!(path, Some(std::ffi::OsStr::new("/a:/b")));
    }
}
