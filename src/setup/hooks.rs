//! Tests for the embedded hook scripts.
//!
//! Hook installation itself is part of `install_plugin` (see [`super::plugins`]) — the
//! hook bytes live in the plugin's `hooks/` directory and are embedded via
//! `PLUGIN_DIR`. This module owns the suite that asserts hook script behaviour
//! and the `hooks.json` metadata so the hook contract is in one obvious place.

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::super::plugins::PLUGIN_DIR;
    use serde_json::Value;

    fn hook_script() -> &'static str {
        PLUGIN_DIR
            .get_file("hooks/scripts/task-status-hook")
            .expect("task-status-hook must be embedded")
            .contents_utf8()
            .expect("task-status-hook must be UTF-8")
    }

    fn usage_hook_script() -> &'static str {
        PLUGIN_DIR
            .get_file("hooks/scripts/task-usage-hook")
            .expect("task-usage-hook must be embedded")
            .contents_utf8()
            .expect("task-usage-hook must be UTF-8")
    }

    #[test]
    fn hook_script_is_valid_bash() {
        assert!(hook_script().starts_with("#!/usr/bin/env bash"));
    }

    #[test]
    fn usage_hook_script_is_valid_bash() {
        assert!(usage_hook_script().starts_with("#!/usr/bin/env bash"));
    }

    #[test]
    fn hook_script_handles_all_events() {
        let s = hook_script();
        // PreToolUse and PostToolUse share a case arm (PreToolUse|PostToolUse)
        assert!(s.contains("PreToolUse"), "hook must handle PreToolUse");
        assert!(s.contains("PostToolUse"), "hook must handle PostToolUse");
        assert!(s.contains("Stop)"), "hook must handle Stop");
        assert!(s.contains("Notification)"), "hook must handle Notification");
        assert!(
            s.contains("UserPromptSubmit)"),
            "hook must handle UserPromptSubmit"
        );
    }

    #[test]
    fn hook_script_skips_dispatch_mcp_in_pretooluse() {
        // The PreToolUse handler must read tool_name from the JSON input
        // and skip dispatch MCP tool calls to avoid clobbering review status
        // during wrap-up (get_task and wrap_up would otherwise set running).
        let s = hook_script();
        assert!(
            s.contains("tool_name"),
            "hook must extract tool_name from PreToolUse input"
        );
        assert!(
            s.contains("mcp__dispatch__"),
            "hook must skip mcp__dispatch__ tools in PreToolUse"
        );
    }

    #[test]
    fn hook_script_uses_dispatch_hook_subcommand() {
        let s = hook_script();
        assert!(
            s.contains("dispatch hook"),
            "must use new `dispatch hook` subcommand"
        );
        assert!(s.contains("pre_tool_use"));
        assert!(s.contains("notification"));
        assert!(s.contains("stop"));
        assert!(s.contains("user_prompt_submit"));
        assert!(
            !s.contains("--sub-status"),
            "old --sub-status flag must not appear"
        );
        assert!(
            !s.contains("--needs-input"),
            "deprecated --needs-input flag must not appear"
        );
    }

    #[test]
    fn hook_script_extracts_task_id_from_git_branch() {
        // Agents commonly cd into subdirectories of the worktree. The hook
        // must still resolve the task ID — `task-usage-hook` already does
        // this via `git branch --show-current`; `task-status-hook` must
        // do the same so PreToolUse/Stop/Notification keep firing when
        // the agent's cwd is below the worktree root.
        let s = hook_script();
        assert!(
            s.contains("git") && s.contains("branch"),
            "task-status-hook must derive the task ID from the git branch \
             so it works from subdirectories of the worktree"
        );
    }

    #[cfg(unix)]
    #[test]
    fn hook_resolves_task_id_from_worktree_subdirectory() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        use std::process::{Command, Stdio};

        let tmp = tempfile::tempdir().expect("tempdir");
        // Create a git worktree-shaped checkout: branch starts with the task ID.
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        run(&["git", "init", "-q", "-b", "567-foo"], &repo);
        run(&["git", "config", "user.email", "t@e.st"], &repo);
        run(&["git", "config", "user.name", "T"], &repo);
        std::fs::write(repo.join("README"), "x").unwrap();
        run(&["git", "add", "."], &repo);
        run(&["git", "commit", "-q", "-m", "init"], &repo);
        let sub = repo.join("sub").join("deep");
        std::fs::create_dir_all(&sub).unwrap();

        // Drop the embedded script to a real file so bash can execute it.
        let script_path = tmp.path().join("task-status-hook");
        std::fs::write(&script_path, hook_script()).unwrap();
        let mut perm = std::fs::metadata(&script_path).unwrap().permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&script_path, perm).unwrap();

        // Shim `dispatch` on PATH so we can observe the call without invoking
        // the real binary or touching the live database.
        let bin = tmp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let observed = tmp.path().join("dispatch.log");
        let shim = format!(
            "#!/usr/bin/env bash\necho \"$@\" >> {}\n",
            observed.display()
        );
        let dispatch_shim = bin.join("dispatch");
        std::fs::write(&dispatch_shim, shim).unwrap();
        let mut p = std::fs::metadata(&dispatch_shim).unwrap().permissions();
        p.set_mode(0o755);
        std::fs::set_permissions(&dispatch_shim, p).unwrap();

        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let payload = format!(
            r#"{{"cwd":"{}","hook_event_name":"PreToolUse","tool_name":"Read"}}"#,
            sub.display()
        );
        let mut child = Command::new("bash")
            .arg(&script_path)
            .env("PATH", &path)
            .current_dir(&sub)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn hook");
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(payload.as_bytes())
            .unwrap();
        let status = child.wait().expect("wait");
        assert!(status.success(), "hook script exited non-zero");

        let log = std::fs::read_to_string(&observed).unwrap_or_default();
        assert!(
            log.contains("hook 567 pre_tool_use"),
            "expected `dispatch hook 567 pre_tool_use` to be invoked from a \
             subdirectory of the worktree; got: {log:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn hook_dispatches_user_prompt_submit_event() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        use std::process::{Command, Stdio};

        let tmp = tempfile::tempdir().expect("tempdir");
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        run(&["git", "init", "-q", "-b", "789-bar"], &repo);
        run(&["git", "config", "user.email", "t@e.st"], &repo);
        run(&["git", "config", "user.name", "T"], &repo);
        std::fs::write(repo.join("README"), "x").unwrap();
        run(&["git", "add", "."], &repo);
        run(&["git", "commit", "-q", "-m", "init"], &repo);

        let script_path = tmp.path().join("task-status-hook");
        std::fs::write(&script_path, hook_script()).unwrap();
        let mut perm = std::fs::metadata(&script_path).unwrap().permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&script_path, perm).unwrap();

        let bin = tmp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let observed = tmp.path().join("dispatch.log");
        let shim = format!(
            "#!/usr/bin/env bash\necho \"$@\" >> {}\n",
            observed.display()
        );
        let dispatch_shim = bin.join("dispatch");
        std::fs::write(&dispatch_shim, shim).unwrap();
        let mut p = std::fs::metadata(&dispatch_shim).unwrap().permissions();
        p.set_mode(0o755);
        std::fs::set_permissions(&dispatch_shim, p).unwrap();

        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let payload = format!(
            r#"{{"cwd":"{}","hook_event_name":"UserPromptSubmit","prompt":"hi"}}"#,
            repo.display()
        );
        let mut child = Command::new("bash")
            .arg(&script_path)
            .env("PATH", &path)
            .current_dir(&repo)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn hook");
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(payload.as_bytes())
            .unwrap();
        let status = child.wait().expect("wait");
        assert!(status.success(), "hook script exited non-zero");

        let log = std::fs::read_to_string(&observed).unwrap_or_default();
        assert!(
            log.contains("hook 789 user_prompt_submit"),
            "expected `dispatch hook 789 user_prompt_submit` to be invoked; got: {log:?}"
        );
    }

    #[cfg(unix)]
    fn run(args: &[&str], cwd: &std::path::Path) {
        let status = std::process::Command::new(args[0])
            .args(&args[1..])
            .current_dir(cwd)
            .status()
            .expect("spawn");
        assert!(status.success(), "command failed: {args:?}");
    }

    fn pr_learnings_hook_script() -> &'static str {
        PLUGIN_DIR
            .get_file("hooks/scripts/pr-learnings-hook")
            .expect("pr-learnings-hook must be embedded")
            .contents_utf8()
            .expect("pr-learnings-hook must be UTF-8")
    }

    fn hooks_json_value() -> Value {
        let content = PLUGIN_DIR
            .get_file("hooks/hooks.json")
            .expect("hooks.json must be embedded")
            .contents_utf8()
            .expect("hooks.json must be UTF-8");
        serde_json::from_str(content).expect("hooks.json is invalid JSON")
    }

    fn hook_commands_for_event<'a>(value: &'a Value, event: &str) -> Vec<&'a str> {
        value["hooks"][event][0]["hooks"]
            .as_array()
            .expect("hooks array")
            .iter()
            .filter_map(|h| h["command"].as_str())
            .collect()
    }

    #[test]
    fn pr_learnings_hook_is_valid_bash() {
        assert!(pr_learnings_hook_script().starts_with("#!/usr/bin/env bash"));
    }

    #[test]
    fn pr_learnings_hook_matches_gh_pr_create_and_calls_gate() {
        let s = pr_learnings_hook_script();
        assert!(
            s.contains("gh pr create"),
            "must match gh pr create commands"
        );
        assert!(s.contains("pr-gate"), "must call `dispatch pr-gate`");
        assert!(
            s.contains("tool_input") || s.contains(".command"),
            "must read the Bash command from the hook JSON"
        );
    }

    #[test]
    fn hooks_json_registers_pr_learnings_hook() {
        let value = hooks_json_value();
        let commands = hook_commands_for_event(&value, "PreToolUse");
        assert!(
            commands.iter().any(|c| c.contains("task-status-hook")),
            "existing task-status-hook must remain registered"
        );
        assert!(
            commands.iter().any(|c| c.contains("pr-learnings-hook")),
            "pr-learnings-hook must be registered under PreToolUse"
        );
    }

    #[cfg(unix)]
    #[test]
    fn pr_learnings_hook_invokes_gate_only_for_gh_pr_create() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        use std::process::{Command, Stdio};

        let tmp = tempfile::tempdir().expect("tempdir");
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        run(&["git", "init", "-q", "-b", "321-pr"], &repo);
        run(&["git", "config", "user.email", "t@e.st"], &repo);
        run(&["git", "config", "user.name", "T"], &repo);
        std::fs::write(repo.join("README"), "x").unwrap();
        run(&["git", "add", "."], &repo);
        run(&["git", "commit", "-q", "-m", "init"], &repo);

        // Drop the embedded script to a real executable file.
        let script_path = tmp.path().join("pr-learnings-hook");
        std::fs::write(&script_path, pr_learnings_hook_script()).unwrap();
        let mut perm = std::fs::metadata(&script_path).unwrap().permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&script_path, perm).unwrap();

        // Shim `dispatch` on PATH (exit 0 so the script doesn't abort on block).
        let bin = tmp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let observed = tmp.path().join("dispatch.log");
        let shim = format!(
            "#!/usr/bin/env bash\necho \"$@\" >> {}\n",
            observed.display()
        );
        let dispatch_shim = bin.join("dispatch");
        std::fs::write(&dispatch_shim, shim).unwrap();
        let mut p = std::fs::metadata(&dispatch_shim).unwrap().permissions();
        p.set_mode(0o755);
        std::fs::set_permissions(&dispatch_shim, p).unwrap();
        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );

        let invoke = |payload: &str| {
            let mut child = Command::new("bash")
                .arg(&script_path)
                .env("PATH", &path)
                .current_dir(&repo)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn hook");
            child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(payload.as_bytes())
                .unwrap();
            let _ = child.wait().expect("wait");
        };

        // Matching command -> gate invoked.
        invoke(&format!(
            r#"{{"cwd":"{}","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{{"command":"gh pr create --draft"}}}}"#,
            repo.display()
        ));
        // Non-matching command -> gate NOT invoked.
        invoke(&format!(
            r#"{{"cwd":"{}","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{{"command":"gh pr view"}}}}"#,
            repo.display()
        ));

        let log = std::fs::read_to_string(&observed).unwrap_or_default();
        assert!(
            log.contains("pr-gate 321"),
            "expected `dispatch pr-gate 321` for gh pr create; got: {log:?}"
        );
        assert_eq!(
            log.matches("pr-gate").count(),
            1,
            "gate must fire only for gh pr create, not gh pr view; got: {log:?}"
        );
    }

    #[test]
    fn hooks_json_is_valid() {
        let value = hooks_json_value();
        assert!(
            value["hooks"].is_object(),
            "missing top-level hooks wrapper"
        );
        assert!(
            value["hooks"]["PreToolUse"].is_array(),
            "missing PreToolUse"
        );
        assert!(
            value["hooks"]["PostToolUse"].is_array(),
            "missing PostToolUse"
        );
        assert!(value["hooks"]["Stop"].is_array(), "missing Stop");
        assert!(
            value["hooks"]["Notification"].is_array(),
            "missing Notification"
        );
        assert!(
            value["hooks"]["UserPromptSubmit"].is_array(),
            "missing UserPromptSubmit"
        );
    }

    #[test]
    fn hooks_json_registers_user_prompt_submit_hook() {
        let value = hooks_json_value();
        let commands = hook_commands_for_event(&value, "UserPromptSubmit");
        assert!(
            commands.iter().any(|c| c.contains("task-status-hook")),
            "task-status-hook must be registered under UserPromptSubmit"
        );
    }

    #[test]
    fn hooks_json_registers_post_tool_use_hook() {
        // PostToolUse must register task-status-hook so activity timestamps
        // are refreshed after every tool call — catching activity between
        // chained sub-agent invocations that would otherwise expire the
        // 10-minute active threshold.
        let value = hooks_json_value();
        let commands = hook_commands_for_event(&value, "PostToolUse");
        assert!(
            commands.iter().any(|c| c.contains("task-status-hook")),
            "task-status-hook must be registered under PostToolUse"
        );
    }
}
