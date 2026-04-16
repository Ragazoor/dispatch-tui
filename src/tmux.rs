use anyhow::{bail, Context, Result};

use crate::process::ProcessRunner;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Create a new tmux window with the given name, starting in `working_dir`.
pub fn new_window(name: &str, working_dir: &str, runner: &dyn ProcessRunner) -> Result<()> {
    let output = runner.run("tmux", &["new-window", "-d", "-n", name, "-c", working_dir])?;
    if !output.status.success() {
        bail!("tmux new-window failed with status {}", output.status);
    }
    Ok(())
}

/// Send literal text to a tmux window, then press Enter.
///
/// Uses `-l` to prevent tmux from interpreting escape sequences in the text.
/// Enter is sent as a separate `send-keys` call without `-l`.
pub fn send_keys(window: &str, keys: &str, runner: &dyn ProcessRunner) -> Result<()> {
    let output = runner.run("tmux", &["send-keys", "-t", window, "-l", keys])?;
    if !output.status.success() {
        bail!("tmux send-keys -l failed with status {}", output.status);
    }
    let output = runner.run("tmux", &["send-keys", "-t", window, "Enter"])?;
    if !output.status.success() {
        bail!("tmux send-keys Enter failed with status {}", output.status);
    }
    Ok(())
}

/// Capture the last `lines` lines of output from a tmux pane, returned trimmed.
pub fn capture_pane(window: &str, lines: usize, runner: &dyn ProcessRunner) -> Result<String> {
    let lines_arg = format!("-{lines}");
    let output = runner.run(
        "tmux",
        &["capture-pane", "-t", window, "-p", "-S", &lines_arg],
    )?;
    if !output.status.success() {
        bail!("tmux capture-pane failed with status {}", output.status);
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(text)
}

/// Return true if a tmux window with the given name currently exists.
pub fn has_window(window: &str, runner: &dyn ProcessRunner) -> Result<bool> {
    let output = runner
        .run("tmux", &["list-windows", "-F", "#{window_name}"])
        .context("failed to run tmux list-windows")?;
    // list-windows exits non-zero when there are no windows / no session;
    // treat that as "window not found" rather than a hard error.
    if !output.status.success() {
        return Ok(false);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text.lines().any(|line| line.trim() == window))
}

/// Return the Unix timestamp of the last activity in a tmux window.
///
/// Uses `tmux display-message` with the `#{window_activity}` format variable,
/// which reports a per-second resolution timestamp updated on any pane I/O.
pub fn window_activity(window: &str, runner: &dyn ProcessRunner) -> Result<u64> {
    let output = runner.run(
        "tmux",
        &["display-message", "-p", "-t", window, "#{window_activity}"],
    )?;
    if !output.status.success() {
        bail!("tmux display-message failed with status {}", output.status);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.trim()
        .parse::<u64>()
        .with_context(|| format!("failed to parse window_activity timestamp: {text:?}"))
}

/// Kill the tmux window with the given name.
pub fn kill_window(window: &str, runner: &dyn ProcessRunner) -> Result<()> {
    let output = runner.run("tmux", &["kill-window", "-t", window])?;
    if !output.status.success() {
        bail!("tmux kill-window failed with status {}", output.status);
    }
    Ok(())
}

/// Switch the active tmux window to the one with the given name.
pub fn select_window(window: &str, runner: &dyn ProcessRunner) -> Result<()> {
    let output = runner.run("tmux", &["select-window", "-t", window])?;
    if !output.status.success() {
        bail!("tmux select-window failed with status {}", output.status);
    }
    Ok(())
}

/// Store the worktree path as a per-window user option so the session-level
/// `after-split-window` hook (installed by [`ensure_split_hook`]) can look it
/// up when a split happens in this window.
pub fn set_window_dispatch_dir(
    window: &str,
    working_dir: &str,
    runner: &dyn ProcessRunner,
) -> Result<()> {
    let output = runner.run(
        "tmux",
        &[
            "set-option",
            "-w",
            "-t",
            window,
            "@dispatch_dir",
            working_dir,
        ],
    )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("ambiguous") {
            bail!(
                "multiple tmux windows named '{}' exist — close the duplicate windows before dispatching",
                window
            );
        }
        bail!(
            "tmux set-option failed with status {}: {}",
            output.status,
            stderr.trim()
        );
    }
    Ok(())
}

/// Install a single session-level `after-split-window` hook that reads the
/// `@dispatch_dir` window option.  If the option is set on the window being
/// split, the new pane `cd`s into that directory; otherwise nothing happens.
///
/// This is idempotent — calling it multiple times replaces the same hook.
pub fn ensure_split_hook(runner: &dyn ProcessRunner) -> Result<()> {
    // if-shell -F only format-expands its test argument, NOT the branch
    // command.  send-keys doesn't expand formats either, so we wrap it in
    // run-shell -C which does expand #{…} before executing the tmux command.
    let hook_cmd = "if-shell -F '#{@dispatch_dir}' 'run-shell -bC \"send-keys \\\"cd #{@dispatch_dir}\\\" Enter\"'";
    let output = runner.run("tmux", &["set-hook", "after-split-window", hook_cmd])?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "tmux set-hook failed with status {}: {}",
            output.status,
            stderr.trim()
        );
    }
    Ok(())
}

/// Check whether tmux has `focus-events` enabled globally.
///
/// Returns `false` if the option is off or if the query fails (e.g. not
/// running inside tmux).
pub fn focus_events_enabled(runner: &dyn ProcessRunner) -> bool {
    let Ok(output) = runner.run("tmux", &["show-options", "-gv", "focus-events"]) else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.trim() == "on"
}

/// Enable tmux `focus-events` globally.
///
/// This is idempotent — calling it when already enabled is a no-op.
pub fn set_focus_events(runner: &dyn ProcessRunner) -> Result<()> {
    let output = runner.run("tmux", &["set-option", "-g", "focus-events", "on"])?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "tmux set-option focus-events failed with status {}: {}",
            output.status,
            stderr.trim()
        );
    }
    Ok(())
}

/// Write `set -g focus-events on` to `~/.tmux.conf` so the setting persists
/// across tmux server restarts.
pub fn write_focus_events_to_tmux_conf() -> Result<()> {
    let home = std::env::var("HOME").context("HOME not set")?;
    let conf = std::path::PathBuf::from(home).join(".tmux.conf");
    write_focus_events_to_tmux_conf_at(&conf)
}

fn write_focus_events_to_tmux_conf_at(path: &std::path::Path) -> Result<()> {
    let existing = if path.exists() {
        std::fs::read_to_string(path).context("failed to read .tmux.conf")?
    } else {
        String::new()
    };
    if existing.contains("focus-events on") {
        return Ok(());
    }
    let addition = if existing.ends_with('\n') || existing.is_empty() {
        "set -g focus-events on\n".to_string()
    } else {
        "\nset -g focus-events on\n".to_string()
    };
    std::fs::write(path, existing + &addition).context("failed to write .tmux.conf")?;
    Ok(())
}

/// Return the name of the currently active tmux window.
pub fn current_window_name(runner: &dyn ProcessRunner) -> Result<String> {
    let output = runner.run("tmux", &["display-message", "-p", "#W"])?;
    if !output.status.success() {
        bail!("tmux display-message failed with status {}", output.status);
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(text)
}

/// Rename a tmux window. Pass `""` as `target` to rename the current window.
pub fn rename_window(target: &str, new_name: &str, runner: &dyn ProcessRunner) -> Result<()> {
    let output = runner.run("tmux", &["rename-window", "-t", target, new_name])?;
    if !output.status.success() {
        bail!("tmux rename-window failed with status {}", output.status);
    }
    Ok(())
}

/// Bind a tmux key (with the default prefix) to a command string.
pub fn bind_key(key: &str, command: &str, runner: &dyn ProcessRunner) -> Result<()> {
    let output = runner.run("tmux", &["bind-key", key, command])?;
    if !output.status.success() {
        bail!("tmux bind-key failed with status {}", output.status);
    }
    Ok(())
}

/// Remove a tmux key binding (with the default prefix).
pub fn unbind_key(key: &str, runner: &dyn ProcessRunner) -> Result<()> {
    let output = runner.run("tmux", &["unbind-key", key])?;
    if !output.status.success() {
        bail!("tmux unbind-key failed with status {}", output.status);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Split mode operations
// ---------------------------------------------------------------------------

/// Return the tmux pane ID of the current pane (e.g. "%42").
pub fn current_pane_id(runner: &dyn ProcessRunner) -> Result<String> {
    let output = runner.run("tmux", &["display-message", "-p", "#{pane_id}"])?;
    if !output.status.success() {
        bail!("tmux display-message failed with status {}", output.status);
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(text)
}

/// Create a horizontal split (right pane) at 40% width, keeping focus on the
/// left pane. Returns the new pane's ID.
pub fn split_window_horizontal(target_pane: &str, runner: &dyn ProcessRunner) -> Result<String> {
    let output = runner.run(
        "tmux",
        &[
            "split-window",
            "-h",
            "-d",
            "-l",
            "40%",
            "-t",
            target_pane,
            "-P",
            "-F",
            "#{pane_id}",
        ],
    )?;
    if !output.status.success() {
        bail!("tmux split-window failed with status {}", output.status);
    }
    let pane_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(pane_id)
}

/// Move a tmux window into the current window as a right pane (40% width).
/// Returns the new pane's ID.
pub fn join_pane(
    source_window: &str,
    target_pane: &str,
    runner: &dyn ProcessRunner,
) -> Result<String> {
    // Get the source pane ID first — pane IDs are preserved across moves,
    // and join-pane does not support -P/-F for printing the result.
    let id_output = runner.run(
        "tmux",
        &["display-message", "-p", "-t", source_window, "#{pane_id}"],
    )?;
    if !id_output.status.success() {
        bail!(
            "tmux display-message failed for source window '{}'",
            source_window
        );
    }
    let pane_id = String::from_utf8_lossy(&id_output.stdout)
        .trim()
        .to_string();

    let output = runner.run(
        "tmux",
        &[
            "join-pane",
            "-h",
            "-d",
            "-s",
            source_window,
            "-t",
            target_pane,
            "-l",
            "40%",
        ],
    )?;
    if !output.status.success() {
        bail!("tmux join-pane failed with status {}", output.status);
    }
    Ok(pane_id)
}

/// Break a pane out into its own tmux window with the given name.
pub fn break_pane_to_window(
    pane_id: &str,
    window_name: &str,
    runner: &dyn ProcessRunner,
) -> Result<()> {
    let output = runner.run(
        "tmux",
        &["break-pane", "-d", "-s", pane_id, "-n", window_name],
    )?;
    if !output.status.success() {
        bail!("tmux break-pane failed with status {}", output.status);
    }
    Ok(())
}

/// Kill a specific tmux pane by ID.
pub fn kill_pane(pane_id: &str, runner: &dyn ProcessRunner) -> Result<()> {
    let output = runner.run("tmux", &["kill-pane", "-t", pane_id])?;
    if !output.status.success() {
        bail!("tmux kill-pane failed with status {}", output.status);
    }
    Ok(())
}

/// Replace the content of a pane with a fresh shell, preserving the pane itself.
pub fn respawn_pane(pane_id: &str, runner: &dyn ProcessRunner) -> Result<()> {
    let output = runner.run("tmux", &["respawn-pane", "-k", "-t", pane_id])?;
    if !output.status.success() {
        bail!(
            "tmux respawn-pane failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Get the pane ID for a window's first pane.
pub fn pane_id_for_window(window: &str, runner: &dyn ProcessRunner) -> Result<String> {
    let output = runner.run(
        "tmux",
        &["display-message", "-p", "-t", window, "#{pane_id}"],
    )?;
    if !output.status.success() {
        bail!("tmux display-message failed for window '{}'", window);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Atomically swap the contents of two panes without changing the layout.
/// `source` can be a pane ID or `<window>.0` to reference a window's first pane.
/// `-d` keeps focus on the current pane.
pub fn swap_pane(source: &str, target: &str, runner: &dyn ProcessRunner) -> Result<()> {
    let output = runner.run("tmux", &["swap-pane", "-d", "-s", source, "-t", target])?;
    if !output.status.success() {
        bail!("tmux swap-pane failed with status {}", output.status);
    }
    Ok(())
}

/// Move tmux focus to the specified pane.
pub fn select_pane(pane_id: &str, runner: &dyn ProcessRunner) -> Result<()> {
    let output = runner.run("tmux", &["select-pane", "-t", pane_id])?;
    if !output.status.success() {
        bail!("tmux select-pane failed with status {}", output.status);
    }
    Ok(())
}

/// Check whether a tmux pane with the given ID still exists.
pub fn pane_exists(pane_id: &str, runner: &dyn ProcessRunner) -> bool {
    runner
        .run("tmux", &["display-message", "-t", pane_id, "-p", ""])
        .map(|output| output.status.success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Internal helpers (kept for arg-shape unit tests)
// ---------------------------------------------------------------------------

#[cfg(test)]
fn select_pane_args(pane_id: &str) -> Vec<String> {
    vec![
        "select-pane".to_string(),
        "-t".to_string(),
        pane_id.to_string(),
    ]
}

#[cfg(test)]
fn select_window_args(window: &str) -> Vec<String> {
    vec![
        "select-window".to_string(),
        "-t".to_string(),
        window.to_string(),
    ]
}

#[cfg(test)]
fn new_window_args(name: &str, working_dir: &str) -> Vec<String> {
    vec![
        "new-window".to_string(),
        "-d".to_string(),
        "-n".to_string(),
        name.to_string(),
        "-c".to_string(),
        working_dir.to_string(),
    ]
}

#[cfg(test)]
fn capture_pane_args(window: &str, lines: usize) -> Vec<String> {
    vec![
        "capture-pane".to_string(),
        "-t".to_string(),
        window.to_string(),
        "-p".to_string(),
        "-S".to_string(),
        format!("-{lines}"),
    ]
}

#[cfg(test)]
fn window_activity_args(window: &str) -> Vec<String> {
    vec![
        "display-message".to_string(),
        "-p".to_string(),
        "-t".to_string(),
        window.to_string(),
        "#{window_activity}".to_string(),
    ]
}

#[cfg(test)]
fn set_window_dispatch_dir_args(window: &str, working_dir: &str) -> Vec<String> {
    vec![
        "set-option".to_string(),
        "-w".to_string(),
        "-t".to_string(),
        window.to_string(),
        "@dispatch_dir".to_string(),
        working_dir.to_string(),
    ]
}

#[cfg(test)]
fn ensure_split_hook_args() -> Vec<String> {
    vec![
        "set-hook".to_string(),
        "after-split-window".to_string(),
        "if-shell -F '#{@dispatch_dir}' 'run-shell -bC \"send-keys \\\"cd #{@dispatch_dir}\\\" Enter\"'"
            .to_string(),
    ]
}

#[cfg(test)]
fn current_window_name_args() -> Vec<String> {
    vec![
        "display-message".to_string(),
        "-p".to_string(),
        "#W".to_string(),
    ]
}

#[cfg(test)]
fn rename_window_args(target: &str, new_name: &str) -> Vec<String> {
    vec![
        "rename-window".to_string(),
        "-t".to_string(),
        target.to_string(),
        new_name.to_string(),
    ]
}

#[cfg(test)]
fn bind_key_args(key: &str, command: &str) -> Vec<String> {
    vec!["bind-key".to_string(), key.to_string(), command.to_string()]
}

#[cfg(test)]
fn unbind_key_args(key: &str) -> Vec<String> {
    vec!["unbind-key".to_string(), key.to_string()]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_window_args_correct() {
        let args = new_window_args("task-42", "/some/path");
        assert_eq!(
            args,
            vec!["new-window", "-d", "-n", "task-42", "-c", "/some/path"]
        );
    }

    #[test]
    fn capture_pane_args_correct() {
        let args = capture_pane_args("task-42", 5);
        assert_eq!(
            args,
            vec!["capture-pane", "-t", "task-42", "-p", "-S", "-5"]
        );
    }

    #[test]
    fn capture_pane_args_different_line_count() {
        let args = capture_pane_args("my-window", 100);
        assert_eq!(args[5], "-100");
    }

    #[test]
    fn has_window_finds_match_in_output() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
            b"main\ntask-42\nother-window\n",
        )]);
        let result = has_window("task-42", &mock).unwrap();
        assert!(result);
    }

    #[test]
    fn has_window_no_match() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
            b"main\nother-window\n",
        )]);
        let result = has_window("task-42", &mock).unwrap();
        assert!(!result);
    }

    #[test]
    fn has_window_exact_match_not_prefix() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"task-42\n")]);
        let result = has_window("task-4", &mock).unwrap();
        assert!(!result);
    }

    #[test]
    fn select_window_args_correct() {
        let args = select_window_args("task-42");
        assert_eq!(args, vec!["select-window", "-t", "task-42"]);
    }

    // --- ProcessRunner-based tests ---

    use crate::process::MockProcessRunner;

    #[test]
    fn new_window_issues_correct_tmux_args() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
        new_window("task-42", "/some/path", &mock).unwrap();
        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "tmux");
        assert_eq!(
            calls[0].1,
            vec!["new-window", "-d", "-n", "task-42", "-c", "/some/path"]
        );
    }

    #[test]
    fn capture_pane_returns_trimmed_stdout() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
            b"  hello from tmux  \n",
        )]);
        let result = capture_pane("task-42", 5, &mock).unwrap();
        assert_eq!(result, "hello from tmux");
    }

    #[test]
    fn has_window_returns_false_on_nonzero_exit() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no sessions")]);
        let result = has_window("task-42", &mock).unwrap();
        assert!(!result);
    }

    #[test]
    fn window_activity_args_correct() {
        let args = window_activity_args("task-42");
        assert_eq!(
            args,
            vec![
                "display-message",
                "-p",
                "-t",
                "task-42",
                "#{window_activity}"
            ]
        );
    }

    #[test]
    fn window_activity_parses_timestamp() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"1711700000\n")]);
        let result = window_activity("task-42", &mock).unwrap();
        assert_eq!(result, 1711700000);
    }

    #[test]
    fn window_activity_fails_on_nonzero_exit() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no window")]);
        assert!(window_activity("task-42", &mock).is_err());
    }

    #[test]
    fn set_window_dispatch_dir_args_correct() {
        let args = set_window_dispatch_dir_args("task-42", "/some/path");
        assert_eq!(
            args,
            vec![
                "set-option",
                "-w",
                "-t",
                "task-42",
                "@dispatch_dir",
                "/some/path",
            ]
        );
    }

    #[test]
    fn set_window_dispatch_dir_issues_correct_tmux_args() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
        set_window_dispatch_dir("task-42", "/some/path", &mock).unwrap();
        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "tmux");
        assert_eq!(
            calls[0].1,
            vec![
                "set-option",
                "-w",
                "-t",
                "task-42",
                "@dispatch_dir",
                "/some/path",
            ]
        );
    }

    #[test]
    fn set_window_dispatch_dir_detects_ambiguous_windows() {
        let mock =
            MockProcessRunner::new(vec![MockProcessRunner::fail("ambiguous window: task-42")]);
        let err = set_window_dispatch_dir("task-42", "/some/path", &mock).unwrap_err();
        assert!(err.to_string().contains("multiple tmux windows"));
    }

    #[test]
    fn ensure_split_hook_args_correct() {
        let args = ensure_split_hook_args();
        assert_eq!(
            args,
            vec![
                "set-hook",
                "after-split-window",
                "if-shell -F '#{@dispatch_dir}' 'run-shell -bC \"send-keys \\\"cd #{@dispatch_dir}\\\" Enter\"'",
            ]
        );
    }

    #[test]
    fn ensure_split_hook_issues_correct_tmux_args() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
        ensure_split_hook(&mock).unwrap();
        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "tmux");
        assert_eq!(
            calls[0].1,
            vec![
                "set-hook",
                "after-split-window",
                "if-shell -F '#{@dispatch_dir}' 'run-shell -bC \"send-keys \\\"cd #{@dispatch_dir}\\\" Enter\"'",
            ]
        );
    }

    #[test]
    fn current_window_name_args_correct() {
        let args = current_window_name_args();
        assert_eq!(args, vec!["display-message", "-p", "#W"]);
    }

    #[test]
    fn current_window_name_returns_trimmed_stdout() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"dispatch\n")]);
        let result = current_window_name(&mock).unwrap();
        assert_eq!(result, "dispatch");
    }

    #[test]
    fn current_window_name_issues_correct_tmux_args() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"dispatch\n")]);
        current_window_name(&mock).unwrap();
        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "tmux");
        assert_eq!(calls[0].1, vec!["display-message", "-p", "#W"]);
    }

    #[test]
    fn current_window_name_fails_on_nonzero_exit() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no session")]);
        assert!(current_window_name(&mock).is_err());
    }

    #[test]
    fn rename_window_args_correct() {
        let args = rename_window_args("dispatch", "my-old-name");
        assert_eq!(args, vec!["rename-window", "-t", "dispatch", "my-old-name"]);
    }

    #[test]
    fn rename_window_issues_correct_tmux_args() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
        rename_window("dispatch", "my-old-name", &mock).unwrap();
        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "tmux");
        assert_eq!(
            calls[0].1,
            vec!["rename-window", "-t", "dispatch", "my-old-name"]
        );
    }

    #[test]
    fn rename_window_fails_on_nonzero_exit() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("no window")]);
        assert!(rename_window("dispatch", "other", &mock).is_err());
    }

    #[test]
    fn bind_key_args_correct() {
        let args = bind_key_args("g", "select-window -t dispatch");
        assert_eq!(args, vec!["bind-key", "g", "select-window -t dispatch"]);
    }

    #[test]
    fn bind_key_issues_correct_tmux_args() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
        bind_key("g", "select-window -t dispatch", &mock).unwrap();
        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "tmux");
        assert_eq!(
            calls[0].1,
            vec!["bind-key", "g", "select-window -t dispatch"]
        );
    }

    #[test]
    fn unbind_key_args_correct() {
        let args = unbind_key_args("g");
        assert_eq!(args, vec!["unbind-key", "g"]);
    }

    #[test]
    fn unbind_key_issues_correct_tmux_args() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
        unbind_key("g", &mock).unwrap();
        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "tmux");
        assert_eq!(calls[0].1, vec!["unbind-key", "g"]);
    }

    #[test]
    fn join_pane_issues_correct_tmux_args() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"%5\n"), // display-message to get source pane ID
            MockProcessRunner::ok(),                    // join-pane (no -P/-F)
        ]);
        let pane_id = join_pane("task-42", "%1", &mock).unwrap();
        assert_eq!(pane_id, "%5");
        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 2);
        // First call: get the source pane ID
        assert_eq!(
            calls[0].1,
            vec!["display-message", "-p", "-t", "task-42", "#{pane_id}"]
        );
        // Second call: join-pane without -P or -F
        assert_eq!(
            calls[1].1,
            vec![
                "join-pane",
                "-h",
                "-d",
                "-s",
                "task-42",
                "-t",
                "%1",
                "-l",
                "40%"
            ]
        );
    }

    #[test]
    fn join_pane_returns_source_pane_id() {
        let mock = MockProcessRunner::new(vec![
            MockProcessRunner::ok_with_stdout(b"%99\n"),
            MockProcessRunner::ok(),
        ]);
        let result = join_pane("my-window", "%0", &mock).unwrap();
        assert_eq!(result, "%99");
    }

    #[test]
    fn select_pane_args_correct() {
        let args = select_pane_args("%42");
        assert_eq!(args, vec!["select-pane", "-t", "%42"]);
    }

    #[test]
    fn select_pane_issues_correct_tmux_args() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
        select_pane("%42", &mock).unwrap();
        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "tmux");
        assert_eq!(calls[0].1, vec!["select-pane", "-t", "%42"]);
    }

    #[test]
    fn focus_events_enabled_returns_true_when_on() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"on\n")]);
        assert!(focus_events_enabled(&mock));
    }

    #[test]
    fn focus_events_enabled_returns_false_when_off() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"off\n")]);
        assert!(!focus_events_enabled(&mock));
    }

    #[test]
    fn set_focus_events_issues_correct_tmux_args() {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::ok()]);
        set_focus_events(&mock).unwrap();
        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "tmux");
        assert_eq!(calls[0].1, vec!["set-option", "-g", "focus-events", "on"]);
    }

    #[test]
    fn write_focus_events_creates_file_if_absent() {
        let dir = tempfile::tempdir().unwrap();
        let conf = dir.path().join(".tmux.conf");
        write_focus_events_to_tmux_conf_at(&conf).unwrap();
        let content = std::fs::read_to_string(&conf).unwrap();
        assert!(content.contains("set -g focus-events on"));
    }

    #[test]
    fn write_focus_events_appends_to_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let conf = dir.path().join(".tmux.conf");
        std::fs::write(&conf, "set -g mouse on\n").unwrap();
        write_focus_events_to_tmux_conf_at(&conf).unwrap();
        let content = std::fs::read_to_string(&conf).unwrap();
        assert!(content.contains("set -g mouse on"));
        assert!(content.contains("set -g focus-events on"));
    }

    #[test]
    fn write_focus_events_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let conf = dir.path().join(".tmux.conf");
        std::fs::write(&conf, "set -g focus-events on\n").unwrap();
        write_focus_events_to_tmux_conf_at(&conf).unwrap();
        let content = std::fs::read_to_string(&conf).unwrap();
        assert_eq!(
            content.matches("focus-events on").count(),
            1,
            "should not duplicate the line"
        );
    }
}
