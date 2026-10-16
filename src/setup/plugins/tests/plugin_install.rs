use super::*;

#[test]
fn plugin_hook_scripts_are_executable() {
    let hooks_scripts = PLUGIN_DIR
        .get_dir("hooks/scripts")
        .expect("hooks/scripts dir must exist");
    for file in hooks_scripts.files() {
        assert!(
            is_executable(file.path()),
            "{} should be marked executable",
            file.path().display()
        );
    }
}

/// Task #81924: the board no longer runs the verify command
/// (`AgentsRunTheVerifyCommand`), so the skill sends the agent to run it
/// before it commits and calls `wrap_up`.
#[test]
fn wrap_up_skill_has_the_agent_run_the_verify_command() {
    let content = skill_body("wrap-up");
    assert!(
        content.contains("## Step 6: Verify, then commit") && content.contains("`Verify command:`"),
        "the skill must send the agent to run the get_task verify command before the commit"
    );
    let closing = wrap_up_closing_sequence();
    assert!(
        !closing.contains("failing verify command"),
        "wrap_up no longer refuses a failing verify command, got: {closing}"
    );
    assert!(
        closing.contains("re-run the verify command"),
        "the closing sequence must name the re-run after a rebase, got: {closing}"
    );
}

// -- Plugin removal --

#[test]
fn remove_plugin_deletes_directory() {
    let dir = tempfile::tempdir().unwrap();
    let plugin = dir.path().join("dispatch");
    fs::create_dir_all(plugin.join("hooks/scripts")).unwrap();
    fs::write(plugin.join("hooks/hooks.json"), "{}").unwrap();

    let removed = remove_plugin(&plugin).unwrap();
    assert!(removed);
    assert!(!plugin.exists());
}

#[test]
fn remove_plugin_noop_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    let plugin = dir.path().join("dispatch");

    let removed = remove_plugin(&plugin).unwrap();
    assert!(!removed);
}

// -- plugin_needs_update --

#[test]
fn plugin_needs_update_true_when_empty_dir() {
    let dir = tempfile::tempdir().unwrap();
    assert!(plugin_needs_update_in(dir.path()).unwrap());
}

fn write_all_plugin_files(base: &std::path::Path) {
    fn write_dir(dir: &Dir, base: &std::path::Path) {
        for file in dir.files() {
            let path = base.join(file.path());
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, file.contents_utf8().unwrap_or("")).unwrap();
        }
        for subdir in dir.dirs() {
            if is_dev_only(subdir.path()) {
                continue;
            }
            write_dir(subdir, base);
        }
    }
    write_dir(&PLUGIN_DIR, base);
}

#[test]
fn plugin_needs_update_false_when_all_match() {
    let dir = tempfile::tempdir().unwrap();
    write_all_plugin_files(dir.path());
    assert!(!plugin_needs_update_in(dir.path()).unwrap());
}

#[test]
fn plugin_needs_update_true_when_one_file_differs() {
    let dir = tempfile::tempdir().unwrap();
    write_all_plugin_files(dir.path());
    // Corrupt one file
    fs::write(dir.path().join(".claude-plugin/plugin.json"), "corrupted").unwrap();
    assert!(plugin_needs_update_in(dir.path()).unwrap());
}

#[test]
fn plugin_needs_update_true_when_stale_file_present() {
    let dir = tempfile::tempdir().unwrap();
    write_all_plugin_files(dir.path());
    // Add a file that is no longer in the embedded plugin
    let stale_dir = dir.path().join("skills").join("old-removed-skill");
    fs::create_dir_all(&stale_dir).unwrap();
    fs::write(stale_dir.join("SKILL.md"), "# Old skill").unwrap();
    assert!(
        plugin_needs_update_in(dir.path()).unwrap(),
        "stale on-disk file should trigger update"
    );
}

#[test]
fn install_removes_stale_files() {
    let dir = tempfile::tempdir().unwrap();
    write_all_plugin_files(dir.path());
    // Plant a stale skill that is no longer embedded
    let stale_dir = dir.path().join("skills").join("old-removed-skill");
    fs::create_dir_all(&stale_dir).unwrap();
    let stale_file = stale_dir.join("SKILL.md");
    fs::write(&stale_file, "# Old skill").unwrap();

    let changed = install_plugin_in(dir.path()).unwrap();

    assert!(changed, "removing a stale file must count as a change");
    assert!(
        !stale_file.exists(),
        "stale file must be removed after install"
    );
}

#[test]
fn install_removes_empty_dirs_after_stale_file_pruned() {
    let dir = tempfile::tempdir().unwrap();
    write_all_plugin_files(dir.path());
    let stale_dir = dir.path().join("skills").join("old-removed-skill");
    fs::create_dir_all(&stale_dir).unwrap();
    fs::write(stale_dir.join("SKILL.md"), "# Old skill").unwrap();

    install_plugin_in(dir.path()).unwrap();

    assert!(
        !stale_dir.exists(),
        "empty stale directory must be removed after pruning its files"
    );
}

#[test]
fn install_is_idempotent_after_pruning() {
    let dir = tempfile::tempdir().unwrap();
    write_all_plugin_files(dir.path());
    let stale_dir = dir.path().join("skills").join("old-removed-skill");
    fs::create_dir_all(&stale_dir).unwrap();
    fs::write(stale_dir.join("SKILL.md"), "# Old skill").unwrap();

    install_plugin_in(dir.path()).unwrap();
    let changed = install_plugin_in(dir.path()).unwrap();
    assert!(
        !changed,
        "second install with nothing to change must be idempotent"
    );
}

#[test]
fn install_does_not_ship_the_evals_directory() {
    let dir = tempfile::tempdir().unwrap();
    install_plugin_in(dir.path()).unwrap();
    assert!(
        !dir.path().join("evals").exists(),
        "plugin/evals/ is a local test fixture and must not be installed"
    );
    assert!(dir.path().join("skills").is_dir());
}

#[test]
fn install_removes_evals_left_by_an_older_install() {
    let dir = tempfile::tempdir().unwrap();
    write_all_plugin_files(dir.path());
    let old = dir.path().join("evals/wrap-up-done/prompt.md");
    fs::create_dir_all(old.parent().unwrap()).unwrap();
    fs::write(&old, "old").unwrap();

    assert!(plugin_needs_update_in(dir.path()).unwrap());
    install_plugin_in(dir.path()).unwrap();

    assert!(!dir.path().join("evals").exists());
}

#[test]
fn plugin_with_evals_embedded_needs_no_update_once_installed() {
    let dir = tempfile::tempdir().unwrap();
    install_plugin_in(dir.path()).unwrap();
    assert!(!plugin_needs_update_in(dir.path()).unwrap());
}
