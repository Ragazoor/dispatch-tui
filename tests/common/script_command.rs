//! The one way a test spawns a plugin script (`plugin/hooks/*.sh` and friends).
//!
//! A dispatched agent's window exports `DISPATCH_*` variables (`DISPATCH_TASK_ID`
//! among them) that the scripts prefer over what a test sets up. Two harnesses
//! once spawned the same scripts with different environment rules, and `cargo
//! test` failed inside every agent window. Both now build their command here, so
//! a test only sees a `DISPATCH_*` variable it sets itself.
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

/// `bash <script>` with `PATH` set to `path_env` and none of the parent's
/// `DISPATCH_*` variables. The caller adds the working directory, stdio and any
/// `DISPATCH_*` variable the test means to set (set it after this call).
pub fn plugin_script_command(script: &Path, path_env: &str) -> Command {
    let mut cmd = Command::new("bash");
    cmd.arg(script).env("PATH", path_env);
    strip_dispatch_env(&mut cmd, std::env::vars_os().map(|(name, _)| name));
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
