use super::*;

#[test]
fn quotes_plain_path() {
    assert_eq!(shell_quote("/home/a/b.json"), "'/home/a/b.json'");
}

#[test]
fn quotes_path_with_spaces() {
    assert_eq!(shell_quote("/home/my dir/b.json"), "'/home/my dir/b.json'");
}

#[test]
fn escapes_embedded_single_quote() {
    // A path containing a single quote must not terminate the quoting.
    assert_eq!(shell_quote("/home/o'brien/b"), r#"'/home/o'\''brien/b'"#);
}

/// The writer goes through the shared invocation rather than spelling the
/// program name again, so the statusLine artefact and the MCP entry's
/// helper cannot drift apart.
///
/// docs/specs/startup.allium: `TheHelperIsTheBareCommandName`.
#[test]
fn builds_a_command_naming_the_shared_invocation() {
    let cmd = build_command(Path::new("/d/rate-limits.json"), None);

    assert!(
        cmd.starts_with(&format!("{STATUSLINE_INVOCATION} ")),
        "the statusline command must invoke the shared program name, got {cmd}"
    );
}

#[test]
fn builds_command_with_chain() {
    let cmd = build_command(Path::new("/d/rate-limits.json"), Some("claude-statusline"));
    assert_eq!(
        cmd,
        "dispatch statusline --snapshot '/d/rate-limits.json' --chain 'claude-statusline'"
    );
}

#[test]
fn builds_command_without_chain() {
    let cmd = build_command(Path::new("/d/rate-limits.json"), None);
    assert_eq!(cmd, "dispatch statusline --snapshot '/d/rate-limits.json'");
}

#[test]
fn discovers_existing_status_line_command() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("settings.json"),
        r#"{"statusLine":{"type":"command","command":"claude-statusline"}}"#,
    )
    .unwrap();
    assert_eq!(
        discover_chain(tmp.path()).as_deref(),
        Some("claude-statusline")
    );
}

#[test]
fn discovers_none_when_no_settings_file() {
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(discover_chain(tmp.path()), None);
}

#[test]
fn discovers_none_when_no_status_line_key() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("settings.json"), r#"{"permissions":{}}"#).unwrap();
    assert_eq!(discover_chain(tmp.path()), None);
}

#[test]
fn discovers_none_when_settings_malformed() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("settings.json"), "{ not json").unwrap();
    assert_eq!(discover_chain(tmp.path()), None);
}

#[test]
fn recursion_guard_refuses_to_chain_to_itself() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("settings.json"),
        r#"{"statusLine":{"type":"command","command":"dispatch statusline --snapshot /d/x.json"}}"#,
    )
    .unwrap();
    assert_eq!(
        discover_chain(tmp.path()),
        None,
        "must not chain to a dispatch statusline invocation"
    );
}

/// Writes settings to a fresh tempdir and parses them back, returning
/// whether the write reported a change alongside the parsed value.
fn write_and_parse(chain: Option<&str>) -> (bool, serde_json::Value) {
    write_and_parse_on_port(chain, crate::DEFAULT_PORT)
}

fn write_and_parse_on_port(chain: Option<&str>, port: u16) -> (bool, serde_json::Value) {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("dispatch-statusline.json");
    let changed = write_settings_file(&path, Path::new("/d/rl.json"), chain, port).unwrap();
    let v = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    (changed, v)
}

/// Every dispatch-spawned session is told which board it belongs to, so
/// the Claude Code hooks running inside it can reach one that is not on
/// the default port. Without this a board started with `--port` is up and
/// serving while every one of its agents' hooks fails — see
/// `HookDelivery` in `docs/specs/agent-health.allium`.
#[test]
fn tells_the_session_which_board_it_belongs_to() {
    let (_, v) = write_and_parse_on_port(None, 8899);
    assert_eq!(
        v["env"]["DISPATCH_PORT"], "8899",
        "the session must carry its board's port, not the default"
    );
}

/// The port is part of what the file says, so a board on a different port
/// finds the file out of date rather than inheriting the last board's.
/// `OneDefinitionOfOutOfDate` again: the drift check reads the same bytes
/// the writer would produce.
#[test]
fn a_different_board_port_is_drift() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("dispatch-statusline.json");
    let snapshot = Path::new("/d/rl.json");
    write_settings_file(&path, snapshot, None, 3142).unwrap();
    assert!(settings_up_to_date(&path, snapshot, None, 3142));
    assert!(
        !settings_up_to_date(&path, snapshot, None, 8899),
        "a board on another port must see the file as out of date"
    );
}

#[test]
fn writes_valid_settings_json() {
    let (changed, v) = write_and_parse(Some("cs"));
    assert!(changed);
    assert_eq!(v["statusLine"]["type"], "command");
    assert_eq!(
        v["statusLine"]["command"],
        "dispatch statusline --snapshot '/d/rl.json' --chain 'cs'"
    );
}

#[test]
fn writes_sandbox_disabled_with_no_other_keys() {
    let (_, v) = write_and_parse(None);
    assert_eq!(
        v["sandbox"],
        json!({"enabled": false}),
        "the sandbox is fully disabled for dispatch-spawned sessions — \
         see SandboxDisabledForDockerAndUnixSockets in dispatch.allium; \
         no other sandbox sub-key should be written since they're inert \
         once disabled"
    );
}

#[test]
fn write_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("dispatch-statusline.json");
    assert!(write_settings_file(
        &path,
        Path::new("/d/rl.json"),
        Some("cs"),
        crate::DEFAULT_PORT
    )
    .unwrap());
    assert!(
        !write_settings_file(
            &path,
            Path::new("/d/rl.json"),
            Some("cs"),
            crate::DEFAULT_PORT
        )
        .unwrap(),
        "second identical write must report no change"
    );
}

/// The one thing this file owns about writing: that it goes through the shared
/// helper at all rather than a bare `fs::write`. The write-if-changed contract
/// itself — exact-bytes comparison included — is asserted where it lives, in
/// `src/setup/mod.rs`.
#[test]
fn write_creates_missing_parent_directories() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("claude").join("dispatch-statusline.json");
    assert!(
        write_settings_file(&path, Path::new("/d/rl.json"), None, crate::DEFAULT_PORT).unwrap()
    );
    assert!(path.exists());
}

#[test]
fn write_reports_change_when_chain_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("dispatch-statusline.json");
    write_settings_file(
        &path,
        Path::new("/d/rl.json"),
        Some("old"),
        crate::DEFAULT_PORT,
    )
    .unwrap();
    assert!(write_settings_file(
        &path,
        Path::new("/d/rl.json"),
        Some("new"),
        crate::DEFAULT_PORT
    )
    .unwrap());
}
