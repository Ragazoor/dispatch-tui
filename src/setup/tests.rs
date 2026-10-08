use super::*;
use crate::process::MockProcessRunner;
use serde_json::json;

// -- File I/O --

#[test]
fn read_json_file_missing_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nonexistent.json");
    let result = read_json_file(&path).unwrap();
    assert!(result.is_none());
}

#[test]
fn read_json_file_invalid_json_errors() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.json");
    fs::write(&path, "not json").unwrap();
    let result = read_json_file(&path);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Invalid JSON"),);
}

#[test]
fn write_and_read_json_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.json");
    let value = json!({"key": "value"});
    write_json_file(&path, &value).unwrap();
    let read_back = read_json_file(&path).unwrap().unwrap();
    assert_eq!(read_back, value);
}

// -- write_file_if_changed --

#[test]
fn write_file_if_changed_creates_new() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new.txt");
    let changed = write_file_if_changed(&path, "hello", false).unwrap();
    assert!(changed);
    assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
}

#[test]
fn write_file_if_changed_skips_identical() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("same.txt");
    fs::write(&path, "hello").unwrap();
    let changed = write_file_if_changed(&path, "hello", false).unwrap();
    assert!(!changed);
}

#[test]
fn write_file_if_changed_updates_stale() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stale.txt");
    fs::write(&path, "old").unwrap();
    let changed = write_file_if_changed(&path, "new", false).unwrap();
    assert!(changed);
    assert_eq!(fs::read_to_string(&path).unwrap(), "new");
}

#[test]
fn write_file_if_changed_creates_missing_parent_directories() {
    // Both callers need this: the plugin path has nested skill/hook
    // directories, and the statusline settings file can land in a
    // `~/.claude` that does not exist yet.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("deeper").join("new.txt");
    assert!(write_file_if_changed(&path, "hello", false).unwrap());
    assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
}

/// Exact bytes, not trim-normalized: `plugins::plugin_needs_update_in` asks
/// the same question with exact equality, and the two must not disagree.
#[test]
fn write_file_if_changed_rewrites_when_only_whitespace_differs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("padded.txt");
    fs::write(&path, "hello\n").unwrap();
    assert!(write_file_if_changed(&path, "hello", false).unwrap());
    assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
}

#[test]
fn write_file_if_changed_sets_executable_permission() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("script.sh");
    write_file_if_changed(&path, "#!/bin/bash", true).unwrap();
    let mode = fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o755, 0o755, "should have executable permissions");
}

// -- MCP setup application --

#[test]
fn apply_mcp_setup_writes_to_target_not_legacy() {
    // Guard against regression: dispatch must write to the user-global
    // file (~/.claude.json), not ~/.claude/.mcp.json which Claude Code
    // does not read.
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join(".claude.json");
    let legacy = dir.path().join(".claude").join(".mcp.json");

    let changed = apply_mcp_setup(&target, &legacy, 3142).unwrap();
    assert!(changed);
    assert!(target.exists(), "target ~/.claude.json must be created");
    assert!(!legacy.exists(), "legacy file must not be created");

    let written = read_json_file(&target).unwrap().unwrap();
    assert_eq!(
        written["mcpServers"]["dispatch"]["url"],
        "http://localhost:3142/mcp"
    );
    assert!(written["mcpServers"]["dispatch"]["headersHelper"].is_string());
}

#[test]
fn apply_mcp_setup_preserves_existing_target_fields() {
    // ~/.claude.json contains many fields (themes, tips, etc.). Setup
    // must merge into it, not overwrite it.
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join(".claude.json");
    let legacy = dir.path().join(".claude").join(".mcp.json");
    write_json_file(
        &target,
        &json!({
            "theme": "dark",
            "mcpServers": {
                "github": {"type": "http", "url": "http://localhost:9999/mcp"}
            }
        }),
    )
    .unwrap();

    apply_mcp_setup(&target, &legacy, 3142).unwrap();

    let written = read_json_file(&target).unwrap().unwrap();
    assert_eq!(written["theme"], "dark");
    assert!(written["mcpServers"]["github"].is_object());
    assert!(written["mcpServers"]["dispatch"].is_object());
}

#[test]
fn apply_mcp_setup_migrates_legacy_dispatch_entry() {
    // Upgrade path: dispatch entry sits in the wrong legacy file.
    // After running setup it must be installed in the target and
    // removed from the legacy file.
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join(".claude.json");
    let legacy_dir = dir.path().join(".claude");
    fs::create_dir_all(&legacy_dir).unwrap();
    let legacy = legacy_dir.join(".mcp.json");
    write_json_file(
        &legacy,
        &json!({
            "mcpServers": {
                "dispatch": {"type": "http", "url": "http://localhost:3142/mcp"},
                "github": {"type": "http", "url": "http://localhost:9999/mcp"}
            }
        }),
    )
    .unwrap();

    let changed = apply_mcp_setup(&target, &legacy, 3142).unwrap();
    assert!(changed);

    // Target got the dispatch entry (with headersHelper).
    let written = read_json_file(&target).unwrap().unwrap();
    assert!(written["mcpServers"]["dispatch"]["headersHelper"].is_string());

    // Legacy lost the dispatch entry but kept the other server.
    let legacy_after = read_json_file(&legacy).unwrap().unwrap();
    assert!(legacy_after["mcpServers"].get("dispatch").is_none());
    assert!(legacy_after["mcpServers"]["github"].is_object());
}

#[test]
fn apply_mcp_setup_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join(".claude.json");
    let legacy = dir.path().join(".claude").join(".mcp.json");

    apply_mcp_setup(&target, &legacy, 3142).unwrap();
    let changed = apply_mcp_setup(&target, &legacy, 3142).unwrap();
    assert!(
        !changed,
        "second apply with no changes must report unchanged"
    );
}

#[test]
fn setup_does_not_write_settings_json() {
    // Regression guard: the setup flow must not create or modify settings.json.
    // That file is user-owned config; dispatch must not add permissions to it.
    let dir = tempfile::tempdir().unwrap();
    let claude_json = dir.path().join(".claude.json");
    let legacy = dir.path().join(".mcp.json");
    let settings = dir.path().join("settings.json");

    apply_mcp_setup(&claude_json, &legacy, 3142).unwrap();

    assert!(
        !settings.exists(),
        "setup must not create settings.json; permissions are user-managed"
    );
}

// -- display_for --

#[test]
fn display_for_shortens_home_prefixed_paths() {
    // Uses the real $HOME (present in every test env) without mutating it,
    // so it is safe under parallel execution.
    let home = std::env::var("HOME").unwrap();
    let path = std::path::Path::new(&home).join("some").join("file.json");
    assert_eq!(display_for(&path), "~/some/file.json");
}

#[test]
fn display_for_leaves_non_home_paths_untouched() {
    // A path guaranteed not to sit under $HOME must be shown verbatim.
    let path = std::path::Path::new("/definitely/not/home/x.json");
    assert_eq!(display_for(path), "/definitely/not/home/x.json");
}

// -- run_uninstall_in: removal decision matrix --

/// Build a fully-populated uninstall layout under a temp dir: a plugin
/// directory with a file, a `~/.claude.json` carrying the dispatch MCP
/// entry, an empty legacy file, a statusline settings file, and a
/// `db_path` that does not yet exist.
fn uninstall_layout(root: &Path) -> UninstallPaths {
    let plugin_path = root.join("plugins").join("local").join("dispatch");
    fs::create_dir_all(&plugin_path).unwrap();
    fs::write(plugin_path.join(".claude-plugin.json"), "{}").unwrap();

    let mcp_path = root.join(".claude.json");
    write_json_file(
        &mcp_path,
        &json!({
            "mcpServers": {
                "dispatch": {"type": "http", "url": "http://localhost:3142/mcp"},
                "github": {"type": "http", "url": "http://localhost:9999/mcp"}
            }
        }),
    )
    .unwrap();

    let statusline_path = root.join(".claude").join(statusline::SETTINGS_FILE_NAME);
    fs::create_dir_all(statusline_path.parent().unwrap()).unwrap();
    fs::write(&statusline_path, "{}").unwrap();

    UninstallPaths {
        mcp_path,
        legacy_mcp_path: root.join(".claude").join(".mcp.json"),
        plugin_path,
        db_path: root.join("dispatch").join("tasks.db"),
        statusline_path,
    }
}

#[test]
fn run_uninstall_in_removes_plugin_and_mcp_when_confirmed() {
    let dir = tempfile::tempdir().unwrap();
    let paths = uninstall_layout(dir.path());
    let confirmer = FakeConfirmer::new(vec![true], vec![]);

    run_uninstall_in(&paths, &confirmer, false, false).unwrap();

    assert!(!paths.plugin_path.exists(), "plugin dir must be removed");
    let mcp = read_json_file(&paths.mcp_path).unwrap().unwrap();
    assert!(
        mcp["mcpServers"].get("dispatch").is_none(),
        "dispatch MCP entry must be removed"
    );
    assert!(
        mcp["mcpServers"]["github"].is_object(),
        "unrelated MCP servers must be preserved"
    );
    assert!(
        !paths.statusline_path.exists(),
        "statusline settings file must be removed"
    );
    assert_eq!(confirmer.confirm_call_count(), 1, "one 'Continue?' prompt");
}

#[test]
fn run_uninstall_in_statusline_file_missing_is_a_noop() {
    let dir = tempfile::tempdir().unwrap();
    let paths = uninstall_layout(dir.path());
    fs::remove_file(&paths.statusline_path).unwrap();
    let confirmer = FakeConfirmer::new(vec![true], vec![]);

    // Must not error just because the statusline file was already absent.
    run_uninstall_in(&paths, &confirmer, false, false).unwrap();

    assert!(!paths.statusline_path.exists());
}

#[test]
fn run_uninstall_in_aborts_when_declined() {
    let dir = tempfile::tempdir().unwrap();
    let paths = uninstall_layout(dir.path());
    let confirmer = FakeConfirmer::new(vec![false], vec![]);

    run_uninstall_in(&paths, &confirmer, false, false).unwrap();

    assert!(
        paths.plugin_path.exists(),
        "declining must leave the plugin dir untouched"
    );
    let mcp = read_json_file(&paths.mcp_path).unwrap().unwrap();
    assert!(
        mcp["mcpServers"]["dispatch"].is_object(),
        "declining must leave the MCP entry untouched"
    );
}

#[test]
fn run_uninstall_in_yes_skips_continue_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let paths = uninstall_layout(dir.path());
    // never() panics if any prompt fires — asserts --yes suppresses "Continue?".
    let confirmer = FakeConfirmer::never();

    run_uninstall_in(&paths, &confirmer, true, false).unwrap();

    assert!(!paths.plugin_path.exists(), "plugin dir must be removed");
    assert_eq!(confirmer.confirm_call_count(), 0);
}

#[test]
fn run_uninstall_in_yes_still_prompts_before_forgetting_the_identity() {
    // Regression guard: --yes suppresses "Continue?" but must NOT
    // auto-confirm the irreversible forgetting of this machine's identity.
    let dir = tempfile::tempdir().unwrap();
    let paths = uninstall_layout(dir.path());
    let data_dir = paths.db_path.parent().unwrap();
    fs::create_dir_all(data_dir).unwrap();
    fs::write(data_dir.join("host.json"), br#"{"host_id":"h"}"#).unwrap();

    // No confirm answers queued (would panic if consulted); dangerous -> no.
    let confirmer = FakeConfirmer::new(vec![], vec![false]);
    run_uninstall_in(&paths, &confirmer, true, true).unwrap();

    assert_eq!(confirmer.confirm_call_count(), 0, "--yes skips 'Continue?'");
    assert_eq!(
        confirmer.dangerous_call_count(),
        1,
        "--yes must still prompt before forgetting the identity"
    );
    assert!(
        data_dir.join("host.json").exists(),
        "host file kept because dangerous declined"
    );
}

#[test]
fn run_uninstall_in_noop_when_nothing_present() {
    let dir = tempfile::tempdir().unwrap();
    // Bare paths: nothing exists on disk.
    let paths = UninstallPaths {
        mcp_path: dir.path().join(".claude.json"),
        legacy_mcp_path: dir.path().join(".mcp.json"),
        plugin_path: dir.path().join("plugin"),
        db_path: dir.path().join("dispatch").join("tasks.db"),
        statusline_path: dir.path().join(statusline::SETTINGS_FILE_NAME),
    };
    let confirmer = FakeConfirmer::new(vec![true], vec![]);

    // Must not error even though there is nothing to remove.
    run_uninstall_in(&paths, &confirmer, false, false).unwrap();
}

// -- run_setup_in: setup decision flow --

/// Build empty setup paths under a temp root plus a fresh in-memory db and
/// a temp data dir. Returns `(paths, data_dir)` — `data_dir` must be kept
/// alive for its temp path to remain valid.
fn setup_layout(root: &Path) -> SetupPaths {
    let claude_dir = root.join(".claude");
    SetupPaths {
        claude_dir: claude_dir.clone(),
        mcp_path: root.join(".claude.json"),
        legacy_mcp_path: claude_dir.join(".mcp.json"),
        tmux_conf_path: root.join(".tmux.conf"),
        statusline_path: statusline::settings_path(&claude_dir),
        budget_snapshot_path: root.join("data").join("rate-limits.json"),
    }
}

#[test]
fn apply_config_update_writes_everything() {
    let root = tempfile::tempdir().unwrap();
    let paths = setup_layout(root.path());

    // focus-events currently OFF, then set-option succeeds.
    let runner = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"off\n"),
        MockProcessRunner::ok(),
    ]);
    inspect_and_apply(&ctx(&paths, 3142, &runner, root.path()));

    // MCP config written to the target with the dispatch entry.
    let mcp = read_json_file(&paths.mcp_path).unwrap().unwrap();
    assert_eq!(
        mcp["mcpServers"]["dispatch"]["url"], "http://localhost:3142/mcp",
        "dispatch MCP entry must be written"
    );
    // Plugin installed under the injected claude dir.
    let plugin_base = plugins::plugin_dir_under(&paths.claude_dir);
    assert!(
        plugin_base.join(".claude-plugin/plugin.json").exists(),
        "plugin must be installed under the injected claude dir"
    );
    // tmux.conf gained the focus-events line.
    let conf = fs::read_to_string(&paths.tmux_conf_path).unwrap();
    assert!(conf.contains("focus-events on"));

    // Status line: dispatch-owned settings file written, statusLine.command
    // matches the snapshot path derived from data_dir, and the invariant
    // that we never touch settings.json holds through the real code path
    // (not just via apply_mcp_setup, which the unit tests exercise).
    assert!(
        paths.statusline_path.exists(),
        "statusline settings file must be written"
    );
    let statusline_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&paths.statusline_path).unwrap()).unwrap();
    assert_eq!(statusline_json["statusLine"]["type"], "command");
    let expected_command = format!(
        "dispatch statusline --snapshot '{}'",
        paths.budget_snapshot_path.display()
    );
    assert_eq!(
        statusline_json["statusLine"]["command"], expected_command,
        "command must point at the machine-wide snapshot location, which is \
         independent of the database's own directory"
    );
    // `SnapshotLocationIsFixedNotDerivedFromTheOpenDatabase` used to need an
    // assertion here that the open database's directory never reached the
    // settings file. It is now enforced by the signature instead: the apply
    // path takes no database and no data directory, so there is nothing for
    // the snapshot location to be derived from but `SetupPaths`.
    assert!(
        !paths.claude_dir.join("settings.json").exists(),
        "the apply path must never create settings.json"
    );
}

#[test]
fn apply_config_update_chains_to_existing_status_line_without_touching_settings_json() {
    let root = tempfile::tempdir().unwrap();
    let paths = setup_layout(root.path());

    fs::create_dir_all(&paths.claude_dir).unwrap();
    let settings_path = paths.claude_dir.join("settings.json");
    let settings_before =
        r#"{"statusLine":{"type":"command","command":"my-prev-line"}}"#.to_string();
    fs::write(&settings_path, &settings_before).unwrap();

    let runner = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"off\n"),
        MockProcessRunner::ok(),
    ]);
    inspect_and_apply(&ctx(&paths, 3142, &runner, root.path()));

    let statusline_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&paths.statusline_path).unwrap()).unwrap();
    let command = statusline_json["statusLine"]["command"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        command.contains("--chain 'my-prev-line'"),
        "must chain to the discovered statusLine command, got: {command}"
    );

    let settings_after = fs::read_to_string(&settings_path).unwrap();
    assert_eq!(
        settings_before, settings_after,
        "settings.json must be byte-identical after the apply — we only read it"
    );
}

#[test]
fn apply_config_update_writes_tmux_conf_when_focus_events_already_enabled() {
    let root = tempfile::tempdir().unwrap();
    let paths = setup_layout(root.path());

    // The running server already has focus-events on, but ~/.tmux.conf does
    // not carry the line — so the artefact is still stale, because the
    // option would not survive the next tmux server restart.
    let runner = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"on\n"),
        MockProcessRunner::ok(),
    ]);
    inspect_and_apply(&ctx(&paths, 3142, &runner, root.path()));

    let conf = fs::read_to_string(&paths.tmux_conf_path).unwrap();
    assert!(
        conf.contains("focus-events on"),
        "an enabled server with no conf line must still persist to .tmux.conf"
    );
}

#[test]
fn apply_config_update_is_idempotent_on_second_run() {
    let root = tempfile::tempdir().unwrap();
    let paths = setup_layout(root.path());

    let runner1 = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"off\n"),
        MockProcessRunner::ok(),
    ]);
    inspect_and_apply(&ctx(&paths, 3142, &runner1, root.path()));

    // Second run: MCP already configured, plugin up to date, focus-events
    // on and persisted. Nothing is stale, so the runner is asked only the
    // one question the inspect needs — a queue of one is the assertion that
    // no writer ran.
    let runner2 = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"on\n")]);
    let failed = inspect_and_apply(&ctx(&paths, 3142, &runner2, root.path()));

    assert!(
        failed.is_empty(),
        "nothing was stale, so nothing can fail: {failed:?}"
    );
}

/// docs/specs/observability.allium: StatusLineDecorator,
/// `SpawnSitesAndStartupNameTheSameConfigurationDirectory`. The writing
/// side's half of the layout, pinned to written-out expectations.
///
/// The spawn constant's half is pinned the same way, against its own
/// hand-written literal, by `spawn_constant_has_exactly_one_space_between_flags`
/// in `src/dispatch/tests.rs`. Both halves expand one shared definition, so
/// a rename cannot move only one of them — but it must fail *visibly on
/// both sides*, which is what these written-out expectations are for.
/// Deriving them from the shared tokens instead would make them agree with
/// the code no matter what it said.
///
/// The `$HOME` assertion is the other half of the agreement, and the one
/// thing here that is not compile-time: the spawn constant names the
/// directory as `~/…`, which the launching shell resolves under the home
/// directory. It cannot see a lookup that gained a *second* input — that
/// would match wherever the new input is unset, CI included, and part only
/// for the operators who set it.
#[test]
fn the_configuration_layout_is_what_the_spawn_constant_names() {
    let home = std::env::var("HOME").expect("$HOME must be set");
    assert_eq!(
        claude_dir().expect("$HOME must be set"),
        std::path::Path::new(&home).join(".claude"),
        "the production lookup must be the home directory plus the name \
         the spawn constant expands, and nothing else"
    );

    let claude_dir = std::path::Path::new("/h/.claude");
    assert_eq!(
        statusline::settings_path(claude_dir),
        std::path::Path::new("/h/.claude/dispatch-statusline.json")
    );
    assert_eq!(
        plugins::plugin_dir_under(claude_dir),
        std::path::Path::new("/h/.claude/plugins/local/dispatch")
    );
}

/// docs/specs/observability.allium: StatusLineDecorator,
/// `SpawnSitesAndStartupNameTheSameConfigurationDirectory`. The plugin
/// directory's writer-side link.
///
/// The startup configuration check is what installs the plugin, and it is
/// *handed* its configuration directory rather than resolving one (see
/// `SettingsLocationIsAnExplicitStartupInput`). So this pins what
/// [`SetupPaths::under`] composes from the directory it is given: the two
/// dispatch-owned locations inside it must be the shared layout, not a
/// second spelling of it.
///
/// Every other test of this flow injects temp directories, so without this
/// the composition itself is exercised by nothing.
#[test]
fn setup_paths_compose_the_shared_configuration_layout() {
    let claude_dir = std::path::Path::new("/h/.claude");
    let paths = SetupPaths::under(claude_dir, std::path::Path::new("/h/.claude.json"))
        .expect("composing a layout must not fail");

    assert_eq!(
        paths.claude_dir, claude_dir,
        "the configuration directory must be the one handed in, untouched"
    );
    assert_eq!(
        paths.statusline_path,
        statusline::settings_path(claude_dir),
        "the settings file the apply writes and the one the spawn constant \
         names must be the same file"
    );
    assert_eq!(
        paths.legacy_mcp_path,
        claude_dir.join(".mcp.json"),
        "the legacy file cleaned up must sit inside the same directory"
    );
}

/// docs/specs/observability.allium: StatusLineDecorator,
/// `AnUnavailableHomeDirectoryIsAFailureNotAPath`. The failure half.
///
/// Both spellings of "no home directory" a shell has must land in the same
/// place. Neither may compose into a location: `$HOME` absent would give a
/// path at the filesystem root, and `$HOME=` a path relative to whatever
/// directory the process is running in — each perfectly well-formed, and
/// each silently substituted for the operator's own.
#[test]
fn an_unavailable_home_directory_is_a_failure_not_a_path() {
    for absent in [None, Some("")] {
        let err = home_dir_from_value(absent)
            .expect_err("an unavailable home directory must not resolve to a path");
        assert!(
            format!("{err:#}").contains("$HOME"),
            "the failure must name the home directory, since nothing \
             downstream can: {err:#}"
        );
    }
}

/// docs/specs/observability.allium: StatusLineDecorator,
/// `AnUnavailableHomeDirectoryIsAFailureNotAPath`. The layout half.
///
/// The home directory is a parameter here rather than the process's own.
/// Comparing two locations both derived from the real `$HOME` cancels it
/// out of each side and asserts nothing about where either one sits.
#[test]
fn the_trust_store_sits_beside_the_configuration_directory() {
    let home = std::path::Path::new("/h");

    assert_eq!(
        user_global_config_path_in(home),
        std::path::Path::new("/h/.claude.json")
    );
    assert_eq!(
        user_global_config_path_in(home).parent(),
        claude_dir_in(home).parent(),
        "the trust store sits beside the configuration directory, not \
         inside it — which is why its name is not one of the \
         `claude_paths` tokens"
    );
}

// -- Configuration drift: inspect (startup.allium's ConfigDriftEngine) --

/// Build a [`ConfigContext`] over injected paths and a mock tmux server.
fn ctx<'a>(
    paths: &'a SetupPaths,
    port: u16,
    runner: &'a dyn crate::process::ProcessRunner,
    data_dir: &'a Path,
) -> ConfigContext<'a> {
    ConfigContext {
        paths,
        port,
        runner,
        data_dir,
        // Nobody to ask. The feed-script tests that need a prompt build
        // their context directly.
        confirmer: None,
    }
}

/// Inspect then apply — the pair as `resolve_startup_config_in` drives it.
/// Returns the artefacts that could not be written.
fn inspect_and_apply(ctx: &ConfigContext<'_>) -> Vec<ConfigArtefact> {
    let drift = inspect_config_drift_in(ctx);
    apply_config_update_in(&drift, ctx)
}

/// A tmux runner that reports focus-events already on, so drift tests that
/// are not about tmux do not have to queue process results.
fn focus_events_on() -> MockProcessRunner {
    MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"on\n")])
}

/// `setup_layout` with every artefact already current, so a test can make
/// exactly one of them stale and assert only that one is reported.
fn make_current(paths: &SetupPaths, data_dir: &Path, port: u16) {
    let runner = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"off\n"),
        MockProcessRunner::ok(),
    ]);
    let failed = inspect_and_apply(&ctx(paths, port, &runner, data_dir));
    assert!(failed.is_empty(), "fixture setup must not fail: {failed:?}");
}

#[test]
fn inspect_reports_every_artefact_on_a_cold_machine() {
    let root = tempfile::tempdir().unwrap();
    let paths = setup_layout(root.path());

    let drift = inspect_config_drift_in(&ctx(&paths, 3142, &focus_events_on(), root.path()));

    assert!(
        !drift.is_clean(),
        "nothing is configured yet, so nothing can be clean"
    );
    for artefact in [
        ConfigArtefact::McpServerEntry,
        ConfigArtefact::Plugin,
        ConfigArtefact::StatusLine,
    ] {
        assert!(
            drift.items.contains(&artefact),
            "{artefact:?} must be reported stale on a cold machine: {drift:?}"
        );
    }
}

#[test]
fn inspect_reports_nothing_once_everything_is_current() {
    let root = tempfile::tempdir().unwrap();
    let paths = setup_layout(root.path());
    make_current(&paths, root.path(), 3142);

    let drift = inspect_config_drift_in(&ctx(&paths, 3142, &focus_events_on(), root.path()));

    assert!(
        drift.is_clean(),
        "OneDefinitionOfOutOfDate: apply just wrote everything, so nothing is stale: {drift:?}"
    );
}

#[test]
fn inspect_writes_nothing_at_all() {
    let root = tempfile::tempdir().unwrap();
    let paths = setup_layout(root.path());

    let _ = inspect_config_drift_in(&ctx(&paths, 3142, &focus_events_on(), root.path()));

    // InspectWritesNothing: not one file, not one directory.
    assert!(
        !paths.mcp_path.exists(),
        "inspect must not create the MCP config"
    );
    assert!(
        !paths.claude_dir.exists(),
        "inspect must not create the configuration directory"
    );
    assert!(
        !paths.statusline_path.exists(),
        "inspect must not create the statusline settings file"
    );
    assert!(
        !paths.tmux_conf_path.exists(),
        "inspect must not create ~/.tmux.conf"
    );
}

// -- Shipped feed scripts as a config artefact
//    (docs/specs/feeds.allium: InstallShippedFeedScripts /
//    InstallShippedFeedConfigs) --

#[test]
fn inspect_reports_feed_scripts_stale_on_a_cold_machine() {
    let root = tempfile::tempdir().unwrap();
    let paths = setup_layout(root.path());

    let drift = inspect_config_drift_in(&ctx(&paths, 3142, &focus_events_on(), root.path()));

    assert!(
        drift.items.contains(&ConfigArtefact::FeedScripts),
        "a repo fix to a feed script never reaches a deployment unless the drift check \
         reports the scripts: {drift:?}"
    );
}

/// `startup.allium`: InspectWritesNothing. The inspect runs on every launch,
/// including one the operator then declines.
#[test]
fn inspecting_feed_scripts_writes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let paths = setup_layout(root.path());

    inspect_config_drift_in(&ctx(&paths, 3142, &focus_events_on(), root.path()));

    assert!(
        !root.path().join("scripts").exists(),
        "an operator who declines the prompt must end the launch with their filesystem \
         byte-identical to how it started — not even an empty scripts directory"
    );
}

#[test]
fn applying_feed_scripts_installs_every_script_and_config() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let paths = setup_layout(root.path());
    let runner = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"off\n"),
        MockProcessRunner::ok(),
    ]);

    let failed = inspect_and_apply(&ctx(&paths, 3142, &runner, root.path()));

    assert!(!failed.contains(&ConfigArtefact::FeedScripts), "{failed:?}");
    for plugins::ShippedFile { name, .. } in plugins::SHIPPED_SCRIPTS {
        let path = plugins::installed_script_path(root.path(), name);
        assert!(path.exists(), "{name} must reach <data_dir>/scripts/");
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "{name} must be executable");
    }
    for plugins::ShippedFile { name, .. } in plugins::SHIPPED_SCRIPT_CONFIGS {
        assert!(
            plugins::installed_script_path(root.path(), name).exists(),
            "{name} must be created alongside the script that sources it"
        );
    }
    assert!(
        inspect_config_drift_in(&ctx(&paths, 3142, &focus_events_on(), root.path()))
            .items
            .iter()
            .all(|a| *a != ConfigArtefact::FeedScripts),
        "a second inspect must find the scripts current — otherwise every launch reports \
         drift it just fixed"
    );
}

/// feeds.allium: UnknownProvenanceIsNeverSilentlyOverwritten. A `None`
/// confirmer is the non-interactive launch: nobody can answer, so the
/// destructive branch must not be reachable at all.
#[test]
fn applying_feed_scripts_keeps_an_edited_script_when_nobody_can_be_asked() {
    let root = tempfile::tempdir().unwrap();
    let paths = setup_layout(root.path());
    let scripts = root.path().join("scripts");
    fs::create_dir_all(&scripts).unwrap();
    let edited = scripts.join("fetch-reviews.sh");
    let mine = "#!/usr/bin/env bash\n# my local edit\necho '[]'\n";
    fs::write(&edited, mine).unwrap();

    let runner = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"off\n"),
        MockProcessRunner::ok(),
    ]);
    inspect_and_apply(&ctx(&paths, 3142, &runner, root.path()));

    assert_eq!(
        fs::read_to_string(&edited).unwrap(),
        mine,
        "a scripted launch has nobody to ask, so it must be incapable of eating an edit"
    );
    assert!(
        !scripts.join("fetch-reviews.sh.bak").exists(),
        "nothing was overwritten, so nothing was backed up"
    );
}

/// The one place a second question is asked after the report's single
/// consent — and it defaults to No.
#[test]
fn applying_feed_scripts_asks_dangerously_before_overwriting_an_edited_script() {
    let root = tempfile::tempdir().unwrap();
    let paths = setup_layout(root.path());
    let scripts = root.path().join("scripts");
    fs::create_dir_all(&scripts).unwrap();
    let edited = scripts.join("fetch-cve.sh");
    let mine = "#!/usr/bin/env bash\n# my local edit\necho '[]'\n";
    fs::write(&edited, mine).unwrap();

    let runner = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"off\n"),
        MockProcessRunner::ok(),
    ]);
    // No confirm() answers queued: reaching the default-YES prompt panics.
    let confirmer = FakeConfirmer::new(vec![], vec![true]);
    let context = ConfigContext {
        paths: &paths,
        port: 3142,
        runner: &runner,
        data_dir: root.path(),
        confirmer: Some(&confirmer),
    };

    inspect_and_apply(&context);

    assert_eq!(
        confirmer.dangerous_call_count(),
        1,
        "exactly one script had unknown provenance, and its prompt must go through \
         confirm_dangerous (default No), not confirm (default Yes)"
    );
    assert_eq!(
        fs::read_to_string(scripts.join("fetch-cve.sh.bak")).unwrap(),
        mine,
        "the approved overwrite must be preceded by a .bak copy of the user's file"
    );
    assert_ne!(
        fs::read_to_string(&edited).unwrap(),
        mine,
        "an explicit yes authorises the overwrite"
    );
}

/// `PartialFailureIsStillProgress`: one unwritable directory must not cost
/// the operator the MCP entry and the plugin install, which had nothing to
/// do with it.
#[test]
fn an_unwritable_scripts_directory_does_not_abandon_the_other_artefacts() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let paths = setup_layout(root.path());
    let scripts = root.path().join("scripts");
    fs::create_dir_all(&scripts).unwrap();
    fs::set_permissions(&scripts, fs::Permissions::from_mode(0o555)).unwrap();

    let runner = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"off\n"),
        MockProcessRunner::ok(),
    ]);
    let failed = inspect_and_apply(&ctx(&paths, 3142, &runner, root.path()));

    fs::set_permissions(&scripts, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        paths.mcp_path.exists(),
        "the MCP entry had nothing to do with the scripts directory and must still be \
         written: {failed:?}"
    );
    assert!(
        plugins::plugin_dir_under(&paths.claude_dir).exists(),
        "nor did the plugin install: {failed:?}"
    );
}

#[test]
fn inspect_reports_a_changed_port_as_mcp_drift() {
    let root = tempfile::tempdir().unwrap();
    let paths = setup_layout(root.path());
    make_current(&paths, root.path(), 3142);

    let drift = inspect_config_drift_in(&ctx(&paths, 4242, &focus_events_on(), root.path()));

    assert!(
        drift.items.contains(&ConfigArtefact::McpServerEntry),
        "a different port means the recorded MCP entry is stale: {drift:?}"
    );
}

#[test]
fn inspect_reports_a_hand_edited_statusline_file_as_drift() {
    let root = tempfile::tempdir().unwrap();
    let paths = setup_layout(root.path());
    make_current(&paths, root.path(), 3142);
    fs::write(&paths.statusline_path, "{}\n").unwrap();

    let drift = inspect_config_drift_in(&ctx(&paths, 3142, &focus_events_on(), root.path()));

    assert!(
        drift.items.contains(&ConfigArtefact::StatusLine),
        "an out-of-band edit is drift: {drift:?}"
    );
}

#[test]
fn inspect_reports_focus_events_off_as_tmux_drift() {
    let root = tempfile::tempdir().unwrap();
    let paths = setup_layout(root.path());
    make_current(&paths, root.path(), 3142);

    let off = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"off\n")]);
    let drift = inspect_config_drift_in(&ctx(&paths, 3142, &off, root.path()));

    assert!(
        drift.items.contains(&ConfigArtefact::TmuxFocusEvents),
        "focus-events off is drift even when ~/.tmux.conf carries the line: {drift:?}"
    );
}

// -- Configuration drift: apply (ApplyIsIdempotent) --

#[test]
fn a_second_apply_reports_no_further_drift() {
    let root = tempfile::tempdir().unwrap();
    let paths = setup_layout(root.path());
    make_current(&paths, root.path(), 3142);

    let runner = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"on\n")]);
    inspect_and_apply(&ctx(&paths, 3142, &runner, root.path()));

    let drift = inspect_config_drift_in(&ctx(&paths, 3142, &focus_events_on(), root.path()));
    assert!(
        drift.is_clean(),
        "ApplyIsIdempotent: the pair converges rather than flip-flopping: {drift:?}"
    );
}
