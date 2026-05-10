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
        assert!(s.contains("PreToolUse)"));
        assert!(s.contains("Stop)"));
        assert!(s.contains("Notification)"));
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
    fn hooks_json_is_valid() {
        let content = PLUGIN_DIR
            .get_file("hooks/hooks.json")
            .expect("hooks.json must be embedded")
            .contents_utf8()
            .expect("hooks.json must be UTF-8");
        let value: Value = serde_json::from_str(content).expect("hooks.json is invalid JSON");
        assert!(
            value["hooks"].is_object(),
            "missing top-level hooks wrapper"
        );
        assert!(
            value["hooks"]["PreToolUse"].is_array(),
            "missing PreToolUse"
        );
        assert!(value["hooks"]["Stop"].is_array(), "missing Stop");
        assert!(
            value["hooks"]["Notification"].is_array(),
            "missing Notification"
        );
    }
}
