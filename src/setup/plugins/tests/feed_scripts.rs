use super::*;

#[test]
fn shipped_fetch_dependabot_script_emits_dependabot_tag() {
    let body = shipped("fetch-dependabot.sh");
    assert!(
        body.contains("tag: \"dependabot\""),
        "fetch-dependabot.sh must emit tag \"dependabot\""
    );
    assert!(
        !body.contains("tag: \"pr-review\""),
        "fetch-dependabot.sh must no longer emit tag \"pr-review\""
    );
}

/// The script filters by bot author — that is what makes the runbook's
/// "do not re-check the PR author" instruction true — but it must not name
/// the bot itself. A bot login is deployment-specific, and a template
/// hardcoding one org's Renovate app silently drops that deployment's
/// Dependabot PRs. It reads bots.conf's BOT_AUTHORS, the same list
/// fetch-reviews.sh's bot-author pass reads.
#[test]
fn shipped_fetch_dependabot_script_filters_on_every_configured_bot_author() {
    let body = shipped("fetch-dependabot.sh");
    assert!(
        body.contains("BOT_AUTHORS"),
        "fetch-dependabot.sh must take its bot logins from bots.conf's BOT_AUTHORS"
    );
    assert!(
        body.contains("--author \"$author\""),
        "fetch-dependabot.sh must filter by the bot author under iteration, not a \
         hardcoded login"
    );
    assert!(
        !body.contains("--author app/"),
        "no bot login may be hardcoded into the gh invocation"
    );
    assert!(
        body.contains("app/kognic-renovate"),
        "an absent or empty BOT_AUTHORS must fall back to app/kognic-renovate, so an \
         existing copy keeps emitting what it emitted before"
    );
}

// -- Shipped feed scripts (docs/specs/feeds.allium:
//    InstallShippedFeedScripts / InstallShippedFeedConfigs /
//    ReportShippedFeedScripts) --

fn scripts_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("scripts")
}

/// Put `content` at `<data_dir>/scripts/<name>`, creating the directory.
/// Used to stage the "a file is already there" branches.
fn plant(data_dir: &Path, name: &str, content: &str) -> PathBuf {
    let dir = scripts_dir(data_dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

/// The provenance manifest as a name -> digest map. A missing, unreadable
/// or malformed manifest reads as empty — the same way the rule treats it.
fn manifest_map(data_dir: &Path) -> serde_json::Map<String, Value> {
    let path = scripts_dir(data_dir).join(SCRIPT_MANIFEST_NAME);
    match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str::<Value>(&raw)
            .ok()
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default(),
        Err(_) => serde_json::Map::new(),
    }
}

fn write_manifest(data_dir: &Path, entries: &[(&str, String)]) {
    let dir = scripts_dir(data_dir);
    std::fs::create_dir_all(&dir).unwrap();
    let map: serde_json::Map<String, Value> = entries
        .iter()
        .map(|(k, v)| ((*k).to_string(), Value::String(v.clone())))
        .collect();
    std::fs::write(
        dir.join(SCRIPT_MANIFEST_NAME),
        serde_json::to_string(&Value::Object(map)).unwrap(),
    )
    .unwrap();
}

fn report_for<'a>(reports: &'a [ShippedScriptReport], name: &str) -> &'a ShippedScriptReport {
    reports.iter().find(|r| r.name == name).unwrap_or_else(|| {
        panic!("every shipped script must yield exactly one report; {name} had none")
    })
}

/// The `--yes` shape: never interactive, and a confirmer that panics if
/// anything prompts it.
fn install_scripts_unattended(data_dir: &Path) -> Vec<ShippedScriptReport> {
    install_shipped_feed_scripts(data_dir, None).unwrap()
}

fn shipped(name: &str) -> &'static str {
    SHIPPED_SCRIPTS
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("{name} must be in the shipped table"))
        .content
}

// Branch 1: absent -> written, executable, digest recorded.

#[test]
fn install_shipped_feed_scripts_installs_every_shipped_script_executable() {
    let data_dir = tempfile::tempdir().unwrap();
    let reports = install_scripts_unattended(data_dir.path());

    assert_eq!(
        reports.len(),
        SHIPPED_SCRIPTS.len(),
        "the loop never exits early, so every shipped script yields exactly one report"
    );
    for ShippedFile { name, .. } in SHIPPED_SCRIPTS {
        let path = installed_script_path(data_dir.path(), name);
        assert!(
            path.exists(),
            "{name} must reach <data_dir>/scripts/ — a release-binary install has no \
             git checkout to copy it out of"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            shipped(name),
            "{name} must be installed with the shipped content verbatim"
        );
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o755,
            "{name} is executed by the feed runner, so it must be installed 0755"
        );
        assert_eq!(
            report_for(&reports, name).outcome,
            ShippedScriptOutcome::Installed,
            "nothing was at {name}'s path, so the outcome is `installed`"
        );
    }
}

#[test]
fn install_shipped_feed_scripts_records_a_digest_for_each_installed_script() {
    let data_dir = tempfile::tempdir().unwrap();
    install_scripts_unattended(data_dir.path());

    let manifest = manifest_map(data_dir.path());
    for ShippedFile { name, .. } in SHIPPED_SCRIPTS {
        assert_eq!(
            manifest.get(name).and_then(|v| v.as_str()),
            Some(script_digest(shipped(name)).as_str()),
            "the manifest must record the digest of what dispatch wrote to {name}, \
             so the next release can update it silently instead of prompting"
        );
    }
}

#[test]
fn install_shipped_feed_scripts_writes_the_manifest_beside_the_scripts() {
    let data_dir = tempfile::tempdir().unwrap();
    install_scripts_unattended(data_dir.path());
    assert!(
        scripts_dir(data_dir.path())
            .join(SCRIPT_MANIFEST_NAME)
            .exists(),
        "the manifest lives alongside the scripts it describes, so a moved or copied \
         <data_dir> carries its provenance with it"
    );
}

// Branch 2: present and identical -> no rewrite, digest recorded anyway.

#[test]
fn install_shipped_feed_scripts_reports_in_sync_without_rewriting_an_identical_file() {
    let data_dir = tempfile::tempdir().unwrap();
    install_scripts_unattended(data_dir.path());
    let path = installed_script_path(data_dir.path(), "fetch-cve.sh");
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();

    let reports = install_scripts_unattended(data_dir.path());

    assert_eq!(
        report_for(&reports, "fetch-cve.sh").outcome,
        ShippedScriptOutcome::InSync,
        "a file that already matches the shipped content is `in_sync`"
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        before,
        "an in-sync script must not be rewritten — there is nothing to write"
    );
}

#[test]
fn install_shipped_feed_scripts_adopts_an_identical_pre_manifest_file() {
    // A deployment that predates the manifest: the right content is on
    // disk, but nothing records that dispatch put it there.
    let data_dir = tempfile::tempdir().unwrap();
    plant(
        data_dir.path(),
        "fetch-reviews.sh",
        shipped("fetch-reviews.sh"),
    );

    let reports = install_scripts_unattended(data_dir.path());

    assert_eq!(
        report_for(&reports, "fetch-reviews.sh").outcome,
        ShippedScriptOutcome::InSync,
        "the file demonstrably IS the shipped content, so no write is needed"
    );
    assert_eq!(
        manifest_map(data_dir.path())
            .get("fetch-reviews.sh")
            .and_then(|v| v.as_str()),
        Some(script_digest(shipped("fetch-reviews.sh")).as_str()),
        "claiming provenance over an identical file asserts nothing untrue, and is how \
         a pre-manifest deployment heals itself so the NEXT release updates silently"
    );
}

// Branch 3: differs, provenance proven -> silent overwrite, no backup.

#[test]
fn install_shipped_feed_scripts_silently_updates_a_script_dispatch_wrote() {
    let data_dir = tempfile::tempdir().unwrap();
    // Stand in for "a previous release shipped this content and nobody has
    // touched it since": on-disk digest == recorded digest, content differs
    // from what ships today.
    let previous = "#!/usr/bin/env bash\n# previous release\necho '[]'\n";
    let path = plant(data_dir.path(), "fetch-security.sh", previous);
    write_manifest(
        data_dir.path(),
        &[("fetch-security.sh", script_digest(previous))],
    );

    // interactive = true, yet the confirmer must never be reached.
    let confirmer = FakeConfirmer::never();
    let reports = install_shipped_feed_scripts(data_dir.path(), Some(&confirmer)).unwrap();

    assert_eq!(
        report_for(&reports, "fetch-security.sh").outcome,
        ShippedScriptOutcome::Updated,
        "dispatch wrote this file and a newer version now ships, so it is `updated`"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        shipped("fetch-security.sh"),
        "a proven-provenance script must be brought up to the shipped content"
    );
    assert_eq!(
        confirmer.confirm_call_count() + confirmer.dangerous_call_count(),
        0,
        "prompting for untouched scripts is what trains the user to hold down `y`; \
         the manifest exists so this case is silent"
    );
    assert!(
        !scripts_dir(data_dir.path())
            .join(format!("fetch-security.sh{SCRIPT_BACKUP_SUFFIX}"))
            .exists(),
        "the replaced content is byte-for-byte a previous release's, recoverable from \
         the release — backing it up would litter every upgrade with .bak files"
    );
    assert!(
        report_for(&reports, "fetch-security.sh")
            .backup_path
            .is_none(),
        "backup_path is set iff a .bak copy was taken"
    );
}

#[test]
fn install_shipped_feed_scripts_re_records_the_digest_after_a_silent_update() {
    let data_dir = tempfile::tempdir().unwrap();
    let previous = "#!/usr/bin/env bash\n# previous release\necho '[]'\n";
    plant(data_dir.path(), "fetch-security.sh", previous);
    write_manifest(
        data_dir.path(),
        &[("fetch-security.sh", script_digest(previous))],
    );

    install_shipped_feed_scripts(data_dir.path(), Some(&FakeConfirmer::never())).unwrap();

    assert_eq!(
        manifest_map(data_dir.path())
            .get("fetch-security.sh")
            .and_then(|v| v.as_str()),
        Some(script_digest(shipped("fetch-security.sh")).as_str()),
        "a stale recorded digest would send the NEXT release to the prompt branch for a \
         file dispatch itself wrote"
    );
}

// Branch 4: differs, provenance unknown.

#[test]
fn install_shipped_feed_scripts_backs_up_before_an_approved_overwrite() {
    let data_dir = tempfile::tempdir().unwrap();
    let edit = "#!/usr/bin/env bash\n# my local debugging edit\necho '[]'\n";
    let path = plant(data_dir.path(), "fetch-log-warnings.sh", edit);

    // No confirm() answers queued: reaching the default-YES prompt panics.
    let confirmer = FakeConfirmer::new(vec![], vec![true]);
    let reports = install_shipped_feed_scripts(data_dir.path(), Some(&confirmer)).unwrap();

    let report = report_for(&reports, "fetch-log-warnings.sh");
    let backup =
        scripts_dir(data_dir.path()).join(format!("fetch-log-warnings.sh{SCRIPT_BACKUP_SUFFIX}"));
    assert_eq!(
        std::fs::read_to_string(&backup).unwrap(),
        edit,
        "the user's content must be copied to <name>.bak BEFORE the write, so a crash \
         mid-write cannot leave them with neither copy \
         (feeds.allium: EveryDestructiveWriteIsPrecededByABackup)"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        shipped("fetch-log-warnings.sh"),
        "an explicit yes authorises the overwrite"
    );
    assert_eq!(report.outcome, ShippedScriptOutcome::Updated);
    assert_eq!(
        report.backup_path.as_deref(),
        Some(backup.as_path()),
        "the report must name the backup so the user can restore it"
    );
    assert_eq!(
        confirmer.dangerous_call_count(),
        1,
        "the prompt goes through confirm_dangerous, which defaults to NO — the \
         destructive answer is never the one you get by pressing enter"
    );
    assert_eq!(
        confirmer.confirm_call_count(),
        0,
        "confirm() defaults to YES and must never be used for this prompt"
    );
}

#[test]
fn install_shipped_feed_scripts_replaces_an_existing_backup() {
    let data_dir = tempfile::tempdir().unwrap();
    let edit = "#!/usr/bin/env bash\n# second edit\necho '[]'\n";
    plant(data_dir.path(), "fetch-cve.sh", edit);
    let backup = plant(
        data_dir.path(),
        &format!("fetch-cve.sh{SCRIPT_BACKUP_SUFFIX}"),
        "#!/usr/bin/env bash\n# first edit, already discarded once\n",
    );

    install_shipped_feed_scripts(
        data_dir.path(),
        Some(&FakeConfirmer::new(vec![], vec![true])),
    )
    .unwrap();

    assert_eq!(
        std::fs::read_to_string(&backup).unwrap(),
        edit,
        "the backup is a safety net for the overwrite just approved, not an archive: \
         there is only ever one .bak per script and it REPLACES the old one"
    );
}

#[test]
fn install_shipped_feed_scripts_keeps_a_file_the_user_declines_to_overwrite() {
    let data_dir = tempfile::tempdir().unwrap();
    let edit = "#!/usr/bin/env bash\n# mine\necho '[]'\n";
    let path = plant(data_dir.path(), "fetch-reviews.sh", edit);

    let confirmer = FakeConfirmer::new(vec![], vec![false]);
    let reports = install_shipped_feed_scripts(data_dir.path(), Some(&confirmer)).unwrap();

    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        edit,
        "a declined overwrite must leave the file exactly as the user had it"
    );
    assert_eq!(
        report_for(&reports, "fetch-reviews.sh").outcome,
        ShippedScriptOutcome::Kept,
        "`kept` is the one outcome that leaves the deployment running something other \
         than what dispatch ships"
    );
    assert!(
        !manifest_map(data_dir.path()).contains_key("fetch-reviews.sh"),
        "recording a digest for a file dispatch did not write would license a SILENT \
         overwrite next run — exactly the consent the manifest exists to track"
    );
    assert!(
        !scripts_dir(data_dir.path())
            .join(format!("fetch-reviews.sh{SCRIPT_BACKUP_SUFFIX}"))
            .exists(),
        "nothing was destroyed, so nothing needed backing up"
    );
}

/// feeds.allium: UnknownProvenanceIsNeverSilentlyOverwritten.
#[test]
fn install_shipped_feed_scripts_never_overwrites_unknown_provenance_unattended() {
    let data_dir = tempfile::tempdir().unwrap();
    let edit = "#!/usr/bin/env bash\n# edited on the box\necho '[]'\n";
    let path = plant(data_dir.path(), "fetch-dependabot.sh", edit);

    let confirmer = FakeConfirmer::never();
    let reports = install_shipped_feed_scripts(data_dir.path(), None).unwrap();

    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        edit,
        "`--yes` means \"do not stop to ask about the safe things\", not \"destroy a file \
         of unknown provenance while nobody is watching\""
    );
    assert_eq!(
        report_for(&reports, "fetch-dependabot.sh").outcome,
        ShippedScriptOutcome::Kept,
        "a non-interactive run keeps an unknown-provenance script"
    );
    assert_eq!(
        confirmer.confirm_call_count() + confirmer.dangerous_call_count(),
        0,
        "a script-invoked setup in CI has nobody to ask, so it must not ask"
    );
    assert!(
        !manifest_map(data_dir.path()).contains_key("fetch-dependabot.sh"),
        "a kept script's provenance is still unknown"
    );
}

#[test]
fn install_shipped_feed_scripts_treats_a_malformed_manifest_as_empty() {
    let data_dir = tempfile::tempdir().unwrap();
    let edit = "#!/usr/bin/env bash\n# mine\necho '[]'\n";
    let path = plant(data_dir.path(), "fetch-cve.sh", edit);
    let dir = scripts_dir(data_dir.path());
    std::fs::write(dir.join(SCRIPT_MANIFEST_NAME), "{ this is not json").unwrap();

    let reports = install_scripts_unattended(data_dir.path());

    assert_eq!(
        report_for(&reports, "fetch-cve.sh").outcome,
        ShippedScriptOutcome::Kept,
        "losing the manifest costs prompts, not correctness: every script falls to the \
         conservative unknown-provenance branch"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        edit,
        "a truncated JSON file dispatch wrote itself must not cost the user their edit"
    );
}

#[test]
fn install_shipped_feed_scripts_survives_an_unreadable_manifest_directory_entry() {
    let data_dir = tempfile::tempdir().unwrap();
    let dir = scripts_dir(data_dir.path());
    std::fs::create_dir_all(dir.join(SCRIPT_MANIFEST_NAME)).unwrap();

    let reports = install_scripts_unattended(data_dir.path());

    assert_eq!(
        reports.len(),
        SHIPPED_SCRIPTS.len(),
        "an unreadable manifest is treated as empty and NEVER fails setup — a setup that \
         refused to run because of a cache would trade the whole install on it"
    );
}

// Per-script failure.

/// feeds.allium: EveryDestructiveWriteIsPrecededByABackup — the fallback on
/// "cannot back up" is to not destroy, never to destroy without a copy.
#[test]
fn install_shipped_feed_scripts_abandons_a_script_whose_backup_fails() {
    let data_dir = tempfile::tempdir().unwrap();
    let edit = "#!/usr/bin/env bash\n# mine\necho '[]'\n";
    let path = plant(data_dir.path(), "fetch-cve.sh", edit);
    // A directory sits where the .bak must go, so the copy cannot succeed.
    let backup = scripts_dir(data_dir.path()).join(format!("fetch-cve.sh{SCRIPT_BACKUP_SUFFIX}"));
    std::fs::create_dir_all(backup.join("occupied")).unwrap();

    let reports = install_shipped_feed_scripts(
        data_dir.path(),
        Some(&FakeConfirmer::new(vec![], vec![true])),
    )
    .unwrap();

    let report = report_for(&reports, "fetch-cve.sh");
    assert_eq!(
        report.outcome,
        ShippedScriptOutcome::Failed,
        "a backup failure abandons the script before the write"
    );
    assert!(
        report.error.as_deref().is_some_and(|e| !e.is_empty()),
        "a failure with no error text is a line the user cannot act on"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        edit,
        "no write may be attempted once the backup failed — there is no reachable state \
         where the destructive write happened and the backup did not"
    );
    assert!(
        !manifest_map(data_dir.path()).contains_key("fetch-cve.sh"),
        "feeds.allium: FailedScriptsClaimNoProvenance — recording a digest dispatch did \
         not successfully write would convert a failure into future consent"
    );
    for name in SHIPPED_SCRIPTS
        .iter()
        .map(|f| f.name)
        .filter(|n| *n != "fetch-cve.sh")
    {
        assert_eq!(
            report_for(&reports, name).outcome,
            ShippedScriptOutcome::Installed,
            "failures are per-script and independent: four scripts landing fine and one \
             failing is four scripts installed"
        );
    }
}

#[test]
fn install_shipped_feed_scripts_reports_write_failures_without_failing_the_run() {
    let data_dir = tempfile::tempdir().unwrap();
    let dir = scripts_dir(data_dir.path());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();

    let result = install_shipped_feed_scripts(data_dir.path(), None);

    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    let reports = result.expect(
        "one unwritable file must not block the MCP config and the plugin install, which \
         had nothing to do with the failure",
    );
    for ShippedFile { name, .. } in SHIPPED_SCRIPTS {
        let report = report_for(&reports, name);
        assert_eq!(
            report.outcome,
            ShippedScriptOutcome::Failed,
            "{name} could not be written, so it is `failed`"
        );
        assert!(
            report.error.is_some(),
            "error is non-null exactly when the outcome is failed"
        );
    }
    assert!(
        manifest_map(data_dir.path()).is_empty(),
        "feeds.allium: FailedScriptsClaimNoProvenance"
    );
}

/// feeds.allium: FailedScriptsClaimNoProvenance. `write_shipped_script`
/// writes the content and then chmods, so a chmod that raises leaves the
/// right bytes on disk with the wrong mode. Branch 2 matches on content
/// alone, so without this repair the next run adopts the file as `in_sync`,
/// records a digest for it and reports "already up to date" — while the
/// feed_command fails on every cycle because the script is not executable.
#[test]
fn install_shipped_feed_scripts_repairs_a_non_executable_identical_script() {
    let data_dir = tempfile::tempdir().unwrap();
    // `plant` writes 0644 — the state a half-failed install leaves behind.
    let path = plant(data_dir.path(), "fetch-cve.sh", shipped("fetch-cve.sh"));

    let reports = install_scripts_unattended(data_dir.path());

    assert_eq!(
        report_for(&reports, "fetch-cve.sh").outcome,
        ShippedScriptOutcome::InSync,
        "the content is already right, so there is nothing to write"
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o755,
        "an installed script the feed runner cannot execute is not `in_sync` with what \
         dispatch ships, however right its bytes are"
    );
}

/// feeds.allium: ShippedScriptReport — backup_path is set iff a .bak was
/// taken. A backup that succeeds and a write that then fails leaves a real
/// .bak on disk, on the one path where the user most needs to restore it.
#[test]
fn install_shipped_feed_scripts_names_the_backup_even_when_the_write_fails() {
    let data_dir = tempfile::tempdir().unwrap();
    let edit = "#!/usr/bin/env bash\n# mine\necho '[]'\n";
    let path = plant(data_dir.path(), "fetch-reviews.sh", edit);
    // Readable (so the copy succeeds) but not writable (so the write does
    // not). The directory stays writable, so the .bak lands.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();

    let reports = install_shipped_feed_scripts(
        data_dir.path(),
        Some(&FakeConfirmer::new(vec![], vec![true])),
    )
    .unwrap();

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let report = report_for(&reports, "fetch-reviews.sh");
    let backup =
        scripts_dir(data_dir.path()).join(format!("fetch-reviews.sh{SCRIPT_BACKUP_SUFFIX}"));
    assert_eq!(
        report.outcome,
        ShippedScriptOutcome::Failed,
        "the write raised, so the script is `failed`"
    );
    assert_eq!(
        std::fs::read_to_string(&backup).unwrap(),
        edit,
        "the backup was taken before the write, and it survives the failure"
    );
    assert_eq!(
        report.backup_path.as_deref(),
        Some(backup.as_path()),
        "a .bak nothing names is a file the user cannot find when they need it most"
    );
    assert!(
        script_report_line(report).is_some_and(|l| l.contains(&backup.display().to_string())),
        "the failed line must name the backup, got {:?}",
        script_report_line(report)
    );
}

// -- Config files (InstallShippedFeedConfigs) --

#[test]
fn install_shipped_feed_configs_creates_every_config_non_executable() {
    let data_dir = tempfile::tempdir().unwrap();
    install_shipped_feed_configs(data_dir.path()).unwrap();

    for ShippedFile { name, .. } in SHIPPED_SCRIPT_CONFIGS {
        let path = installed_script_path(data_dir.path(), name);
        assert!(
            path.exists(),
            "{name} must be created — installing a reader without its conf seeds an inert \
             script with no file to configure it from"
        );
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o111,
            0,
            "{name} is sourced by the scripts, never run, so it must not be executable"
        );
    }
}

/// feeds.allium: ConfigFilesAreNeverOverwritten. There is no flag, no
/// prompt and no manifest state that unlocks it.
#[test]
fn install_shipped_feed_configs_never_overwrites_an_existing_config() {
    let data_dir = tempfile::tempdir().unwrap();
    for ShippedFile { name, .. } in SHIPPED_SCRIPT_CONFIGS {
        plant(data_dir.path(), name, &format!("# mine: {name}\n"));
    }

    install_shipped_feed_configs(data_dir.path()).unwrap();
    install_shipped_feed_configs(data_dir.path()).unwrap();

    for ShippedFile { name, .. } in SHIPPED_SCRIPT_CONFIGS {
        assert_eq!(
            std::fs::read_to_string(installed_script_path(data_dir.path(), name)).unwrap(),
            format!("# mine: {name}\n"),
            "setup never writes over an existing config path — not with --yes, not with a \
             prompt, not ever"
        );
    }
}

#[test]
fn install_shipped_feed_configs_records_no_provenance() {
    let data_dir = tempfile::tempdir().unwrap();
    install_shipped_feed_configs(data_dir.path()).unwrap();

    let manifest = manifest_map(data_dir.path());
    for ShippedFile { name, .. } in SHIPPED_SCRIPT_CONFIGS {
        assert!(
            !manifest.contains_key(name),
            "{name} is never updated, so its provenance is never consulted — an entry \
             would be a fact nothing reads"
        );
    }
}

#[test]
fn install_shipped_feed_configs_leaves_a_user_bots_conf_alone() {
    let data_dir = tempfile::tempdir().unwrap();
    install_shipped_feed_configs(data_dir.path()).unwrap();
    let bots_conf = installed_script_path(data_dir.path(), "bots.conf");
    assert!(
        std::fs::read_to_string(&bots_conf)
            .unwrap()
            .contains("BOT_AUTHORS"),
        "the installed bots.conf must declare BOT_AUTHORS"
    );
    std::fs::write(&bots_conf, "BOT_AUTHORS=(\"app/mine\")\n").unwrap();
    install_shipped_feed_configs(data_dir.path()).unwrap();
    assert_eq!(
        std::fs::read_to_string(&bots_conf).unwrap(),
        "BOT_AUTHORS=(\"app/mine\")\n",
        "install must not overwrite user edits to bots.conf"
    );
}

#[test]
fn install_shipped_feed_configs_leaves_a_user_repos_conf_alone() {
    let data_dir = tempfile::tempdir().unwrap();
    install_shipped_feed_configs(data_dir.path()).unwrap();
    let repos_conf = installed_script_path(data_dir.path(), "repos.conf");
    std::fs::write(&repos_conf, "REPOS=(\"myorg/custom\")\n").unwrap();
    install_shipped_feed_configs(data_dir.path()).unwrap();
    assert_eq!(
        std::fs::read_to_string(&repos_conf).unwrap(),
        "REPOS=(\"myorg/custom\")\n",
        "install must not overwrite user edits to repos.conf"
    );
}

// -- Config defaults --

#[test]
fn shipped_script_set_is_the_whole_shipped_set_not_a_curated_subset() {
    assert_eq!(
        SHIPPED_SCRIPTS.iter().map(|f| f.name).collect::<Vec<_>>(),
        vec![
            "fetch-dependabot.sh",
            "fetch-reviews.sh",
            "fetch-cve.sh",
            "fetch-security.sh",
            "fetch-log-warnings.sh",
        ],
        "before this rule only fetch-dependabot.sh was installed, so the other four \
         reached a deployment only via a git checkout the user may not have"
    );
    assert_eq!(
        SHIPPED_SCRIPT_CONFIGS
            .iter()
            .map(|f| f.name)
            .collect::<Vec<_>>(),
        vec!["repos.conf", "bots.conf", "org.conf", "log-warnings.conf"],
        "org.conf and log-warnings.conf are the confs of the newly shipped readers"
    );
    assert_eq!(SCRIPT_MANIFEST_NAME, ".install-manifest.json");
    assert_eq!(SCRIPT_BACKUP_SUFFIX, ".bak");
}

fn all_in_sync() -> Vec<ShippedScriptReport> {
    SHIPPED_SCRIPTS
        .iter()
        .map(|f| report(f.name, ShippedScriptOutcome::InSync))
        .collect()
}

#[test]
fn feed_scripts_section_says_already_up_to_date_when_nothing_is_reportable() {
    let lines = feed_scripts_section_lines(&all_in_sync());
    assert_eq!(
        lines.len(),
        1,
        "a missing section is ambiguous between \"nothing to do\" and \"this version of \
         dispatch does not do that yet\"; a present one-liner is not"
    );
    assert!(
        lines[0].contains("already up to date"),
        "expected the up-to-date one-liner, got {:?}",
        lines[0]
    );
}

#[test]
fn feed_scripts_section_omits_in_sync_scripts() {
    let mut reports = all_in_sync();
    reports[1] = report("fetch-reviews.sh", ShippedScriptOutcome::Installed);
    let lines = feed_scripts_section_lines(&reports);

    assert_eq!(
        lines.len(),
        1,
        "a section that lists all five every run is noise the user learns to skip — \
         which costs them the one line that is not noise"
    );
    assert!(
        lines[0].contains("fetch-reviews.sh"),
        "the only changed script must be the only line, got {:?}",
        lines[0]
    );
}

#[test]
fn feed_scripts_section_puts_failures_before_changes() {
    let mut reports = all_in_sync();
    reports[0] = report("fetch-dependabot.sh", ShippedScriptOutcome::Installed);
    let mut failure = report("fetch-log-warnings.sh", ShippedScriptOutcome::Failed);
    failure.error = Some("Permission denied (os error 13)".to_string());
    reports[4] = failure;

    let lines = feed_scripts_section_lines(&reports);

    assert_eq!(lines.len(), 2, "two reportable scripts, two lines");
    assert!(
        lines[0].contains("fetch-log-warnings.sh"),
        "failures come first even though the failing script is last in shipped order: \
         putting it last risks it scrolling off behind four successes, got {lines:?}"
    );
    assert!(
        lines[0].contains("Permission denied"),
        "a failed line must carry the error text, got {:?}",
        lines[0]
    );
    assert!(
        lines[0].contains("/data/scripts/fetch-log-warnings.sh"),
        "a failed line names the full path, got {:?}",
        lines[0]
    );
    assert!(
        lines[1].contains("fetch-dependabot.sh"),
        "changes follow the failures, got {lines:?}"
    );
}

#[test]
fn feed_scripts_section_orders_each_group_by_shipped_order() {
    let reports = vec![
        report("fetch-dependabot.sh", ShippedScriptOutcome::Installed),
        report("fetch-reviews.sh", ShippedScriptOutcome::Updated),
        report("fetch-cve.sh", ShippedScriptOutcome::Kept),
        report("fetch-security.sh", ShippedScriptOutcome::Installed),
        report("fetch-log-warnings.sh", ShippedScriptOutcome::Installed),
    ];
    let lines = feed_scripts_section_lines(&reports);
    let names: Vec<&str> = SHIPPED_SCRIPTS.iter().map(|f| f.name).collect();
    for (line, name) in lines.iter().zip(names) {
        assert!(
            line.contains(name),
            "within a group the order is config.shipped_scripts order, so repeated runs \
             print stably; expected {name} in {line:?}"
        );
    }
}

#[test]
fn feed_scripts_section_names_the_full_path_and_reason_for_a_kept_script() {
    let reports = vec![report("fetch-cve.sh", ShippedScriptOutcome::Kept)];
    let lines = feed_scripts_section_lines(&reports);
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].contains("/data/scripts/fetch-cve.sh"),
        "`kept` is the only outcome the user may need to act on, and they cannot act on \
         a bare file name, got {:?}",
        lines[0]
    );
    assert!(
        lines[0].contains("differs"),
        "the line must say WHY it was kept — \"kept\" alone reads like reassurance \
         rather than the open question it is, got {:?}",
        lines[0]
    );
}

#[test]
fn feed_scripts_section_names_the_backup_for_a_backed_up_script() {
    let mut r = report("fetch-reviews.sh", ShippedScriptOutcome::Updated);
    r.backup_path = Some(PathBuf::from("/data/scripts/fetch-reviews.sh.bak"));
    let lines = feed_scripts_section_lines(&[r]);
    assert!(
        lines[0].contains("/data/scripts/fetch-reviews.sh.bak"),
        "the backup path is named in the report so the user can restore, got {:?}",
        lines[0]
    );
}

#[test]
fn feed_scripts_section_never_folds_a_failure_into_already_up_to_date() {
    let mut reports = all_in_sync();
    let mut failure = report("fetch-security.sh", ShippedScriptOutcome::Failed);
    failure.error = Some("No space left on device".to_string());
    reports[3] = failure;

    let lines = feed_scripts_section_lines(&reports);

    assert_eq!(lines.len(), 1);
    assert!(
        !lines[0].contains("already up to date"),
        "a run containing a failure can never take the nothing-happened branch, even if \
         every other script was in sync, got {:?}",
        lines[0]
    );
    assert!(lines[0].contains("No space left on device"));
}

#[test]
fn fetch_dependabot_uses_repos_conf_when_present() {
    // Write a repos.conf with a fake repo; the script should attempt to probe
    // it and fail — but the failure message confirms repos.conf was sourced.
    let data_dir = tempfile::tempdir().unwrap();
    install_scripts_unattended(data_dir.path());
    install_shipped_feed_configs(data_dir.path()).unwrap();
    let script_path = installed_script_path(data_dir.path(), "fetch-dependabot.sh");
    let repos_conf = installed_script_path(data_dir.path(), "repos.conf");
    std::fs::write(&repos_conf, "REPOS=(\"fake-owner/fake-repo-xyz\")\n").unwrap();

    let output = std::process::Command::new("bash")
        .arg(&script_path)
        .output()
        .expect("script must be runnable");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("fake-owner/fake-repo-xyz"),
        "script must attempt to probe repos from repos.conf; stderr={stderr}"
    );
}

#[test]
fn installed_example_script_emits_empty_feed_item_array() {
    // The shipped example must be inert (REPOS empty) so a fresh install
    // does not flood the kanban board with someone else's repos.
    let data_dir = tempfile::tempdir().unwrap();
    install_scripts_unattended(data_dir.path());
    install_shipped_feed_configs(data_dir.path()).unwrap();
    let path = installed_script_path(data_dir.path(), "fetch-dependabot.sh");

    let output = std::process::Command::new("bash")
        .arg(&path)
        .output()
        .expect("running the installed example script must not fail");
    assert!(
        output.status.success(),
        "example script exited non-zero: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: Vec<crate::models::FeedItem> = serde_json::from_slice(&output.stdout)
        .expect("example script must emit a JSON array of FeedItem");
    assert!(parsed.is_empty(), "example script must emit [] by default");
}

/// A confirmer whose every prompt fails, like a closed stdin.
struct FailingConfirmer;

impl crate::setup::Confirmer for FailingConfirmer {
    fn confirm(&self, _prompt: &str) -> Result<bool> {
        anyhow::bail!("input closed")
    }
    fn confirm_dangerous(&self, _prompt: &str) -> Result<bool> {
        anyhow::bail!("input closed")
    }
    fn prompt_text(&self, _prompt: &str, _default: &str) -> Result<String> {
        anyhow::bail!("input closed")
    }
}

#[test]
fn install_shipped_feed_scripts_survives_a_failing_prompt_and_still_writes_the_manifest() {
    let data_dir = tempfile::tempdir().unwrap();
    let edit = "#!/usr/bin/env bash\n# mine\necho '[]'\n";
    let path = plant(data_dir.path(), "fetch-reviews.sh", edit);

    let reports = install_shipped_feed_scripts(data_dir.path(), Some(&FailingConfirmer))
        .expect("a prompt error must not abort the run");

    assert_eq!(std::fs::read_to_string(&path).unwrap(), edit);
    assert_eq!(
        report_for(&reports, "fetch-reviews.sh").outcome,
        ShippedScriptOutcome::Failed
    );
    assert!(
        scripts_dir(data_dir.path())
            .join(SCRIPT_MANIFEST_NAME)
            .exists(),
        "the manifest must be written even when a prompt fails"
    );
    assert!(
        manifest_map(data_dir.path()).len() >= SHIPPED_SCRIPTS.len() - 1,
        "the other scripts' digests must be recorded"
    );
}
