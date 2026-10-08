//! Config artefact drift detection and the startup config update.

use super::*;

/// Apply the dispatch MCP entry to `target` (Claude Code's user-global config,
/// `~/.claude.json`) and remove any stale entry from `legacy` (the old wrong
/// path, `~/.claude/.mcp.json`, that earlier dispatch versions wrote to and
/// that Claude Code never read).
///
/// Returns `true` if either file changed.
///
/// Asks nothing. Consent for the whole update was obtained once, before this
/// was reached — see `startup::resolve_startup_config_in` and `startup.allium`'s
/// `ConfigurationIsNeverWrittenWithoutConsent`.
pub(super) fn apply_mcp_setup(target: &Path, legacy: &Path, port: u16) -> Result<bool> {
    let mut changed = false;

    let existing = read_json_file(target)?;
    let merged = merge_mcp_config(existing, port);
    if merged.changed {
        let display = display_for(target);
        write_json_file(target, &merged.value)?;
        println!("MCP config: added dispatch to {display} (port {port})");
        changed = true;
    }

    match remove_mcp_config(legacy) {
        Ok(true) => {
            println!(
                "MCP config: removed stale dispatch entry from {} (Claude Code did not read this file)",
                legacy.display()
            );
            changed = true;
        }
        Ok(false) => {}
        Err(e) => eprintln!("Warning: failed to clean up legacy MCP config: {e}"),
    }

    Ok(changed)
}

/// Best-effort tilde-shortened display for paths under `$HOME`.
pub(super) fn display_for(path: &Path) -> String {
    if let Ok(home) = std::env::var("HOME") {
        let home_path = std::path::Path::new(&home);
        if let Ok(stripped) = path.strip_prefix(home_path) {
            return format!("~/{}", stripped.display());
        }
    }
    path.display().to_string()
}

// ---------------------------------------------------------------------------
// Configuration locations
// ---------------------------------------------------------------------------

/// Filesystem locations the setup flow writes to. Grouped so tests can point
/// the whole flow at temp directories instead of the real `$HOME`.
pub struct SetupPaths {
    pub claude_dir: PathBuf,
    pub mcp_path: PathBuf,
    pub legacy_mcp_path: PathBuf,
    pub tmux_conf_path: PathBuf,
    pub statusline_path: PathBuf,
    /// Where the statusLine decorator is told to publish the budget snapshot.
    /// Fixed per machine and independent of `--db` — see
    /// `docs/specs/observability.allium`:
    /// `SnapshotLocationIsFixedNotDerivedFromTheOpenDatabase`.
    pub budget_snapshot_path: PathBuf,
}

impl SetupPaths {
    /// The set, composed from a configuration directory and trust store the
    /// caller already resolved.
    ///
    /// There is deliberately no `resolve()` beside this: every caller is handed
    /// its locations, so no lookup is left inside setup that could reach the
    /// operator's real configuration on a run that is not their session. See
    /// `SettingsLocationIsAnExplicitStartupInput`.
    ///
    /// `runtime::StartupPaths::setup_paths` is the one caller: startup is
    /// handed the operator's locations and hands them onward, so the
    /// configuration check performs no `$HOME` lookup of its own. The two
    /// remaining values are fixed per machine rather than per configuration
    /// directory, so they are resolved here — see
    /// `SnapshotLocationIsFixedNotDerivedFromTheOpenDatabase`. The MCP entry's
    /// helper command used to be a third: it is now composed at compile time
    /// (`config::CALLER_HEADERS_COMMAND`), so there is nothing to resolve and
    /// nothing to hand onward — see `startup.allium`'s
    /// `TheHelperIsTheBareCommandName`.
    pub fn under(claude_dir: &Path, mcp_path: &Path) -> Result<Self> {
        Ok(Self {
            legacy_mcp_path: claude_dir.join(".mcp.json"),
            statusline_path: statusline::settings_path(claude_dir),
            claude_dir: claude_dir.to_path_buf(),
            mcp_path: mcp_path.to_path_buf(),
            tmux_conf_path: tmux::tmux_conf_path()?,
            budget_snapshot_path: crate::budget_snapshot_path(),
        })
    }
}

// ---------------------------------------------------------------------------
// Configuration drift — see docs/specs/startup.allium
// ---------------------------------------------------------------------------

/// One configuration artefact dispatch manages on the operator's behalf.
/// `startup.allium`'s `ConfigArtefact`.
///
/// Named individually because the drift report is what the operator reads
/// immediately before being asked for permission, and "something is out of
/// date" is not enough to consent to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigArtefact {
    /// The dispatch MCP server in Claude Code's user-global config.
    McpServerEntry,
    /// The embedded skills, commands and hooks.
    Plugin,
    /// The dispatch-owned statusLine settings file.
    StatusLine,
    /// The tmux focus-events option and its persisted form in `~/.tmux.conf`.
    TmuxFocusEvents,
    /// The shipped feed scripts and their config files under
    /// `<data_dir>/scripts/`. See `InstallShippedFeedScripts` and
    /// `InstallShippedFeedConfigs` in docs/specs/feeds.allium.
    FeedScripts,
}

/// What the startup check found out of date. `startup.allium`'s `ConfigDrift`.
///
/// An empty report is the normal steady state and the only case that produces
/// no output at all.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigDrift {
    pub items: Vec<ConfigArtefact>,
}

impl ConfigDrift {
    pub fn is_clean(&self) -> bool {
        self.items.is_empty()
    }
}

/// Everything an artefact's two halves need to reach the filesystem and the
/// running tmux server. One struct rather than a parameter list per artefact,
/// so a new artefact needing a new input adds a field here instead of changing
/// four signatures.
pub(crate) struct ConfigContext<'a> {
    pub paths: &'a SetupPaths,
    pub port: u16,
    pub runner: &'a dyn crate::process::ProcessRunner,
    /// Where `<data_dir>/scripts/` lives. Not derivable from `paths`, which
    /// describes the Claude Code configuration directory; the feed scripts sit
    /// beside the dispatch database instead.
    pub data_dir: &'a Path,
    /// `None` when nobody can answer — a scripted launch, a CI job, stdin
    /// redirected from nowhere. The absence IS the input, so no path can read a
    /// queued "yes" out of silence. Only `ConfigArtefact::FeedScripts` reads it,
    /// and only on its one destructive branch — see its `apply` arm.
    pub confirmer: Option<&'a dyn Confirmer>,
}

impl ConfigArtefact {
    /// The four artefacts, in the order they are checked and applied.
    ///
    /// The single enumeration. `inspect_config_drift_in` filters it and
    /// `apply_config_update_in` walks it; neither carries a list of its own, so
    /// a fifth artefact is one variant and two match arms, not four coordinated
    /// edits across the module.
    pub(crate) const ALL: [Self; 5] = [
        Self::McpServerEntry,
        Self::Plugin,
        Self::StatusLine,
        Self::TmuxFocusEvents,
        Self::FeedScripts,
    ];

    /// How the artefact is named to the operator, in the prompt and the
    /// non-interactive report alike.
    pub fn label(self) -> &'static str {
        match self {
            Self::McpServerEntry => "MCP server entry",
            Self::Plugin => "plugin (skills, commands, hooks)",
            Self::StatusLine => "status line settings",
            Self::TmuxFocusEvents => "tmux focus-events",
            Self::FeedScripts => "feed scripts",
        }
    }

    /// Whether this artefact's on-disk state already matches what this build
    /// would write. **Reads only** — `InspectWritesNothing`: no file and no
    /// directory is created here, not even an empty one.
    ///
    /// Paired with [`ConfigArtefact::apply`] on the same variant, which is what
    /// makes `OneDefinitionOfOutOfDate` structural rather than a promise each
    /// author has to remember: the two halves of an artefact are two arms of
    /// one match, so a reader sees them together and a new artefact cannot
    /// acquire one without the other.
    ///
    /// An artefact that cannot be read is not current. The writer's own error
    /// is a better report than a read error here, and reporting drift costs the
    /// operator a prompt rather than a silently skipped update.
    fn is_current(self, ctx: &ConfigContext<'_>) -> bool {
        let paths = ctx.paths;
        match self {
            // Current only if the merge would change nothing *and* the legacy
            // file Claude Code never read carries no entry to clean up.
            Self::McpServerEntry => {
                let merge_is_noop = read_json_file(&paths.mcp_path)
                    .is_ok_and(|existing| !merge_mcp_config(existing, ctx.port).changed);
                merge_is_noop && !config::has_dispatch_entry(&paths.legacy_mcp_path)
            }
            Self::Plugin => {
                !plugins::plugin_needs_update_in(&plugins::plugin_dir_under(&paths.claude_dir))
                    .unwrap_or(true)
            }
            // `discover_chain` treats a missing directory as "nothing to chain
            // to", so this is safe on a cold machine.
            Self::StatusLine => statusline::settings_up_to_date(
                &paths.statusline_path,
                &paths.budget_snapshot_path,
                statusline::discover_chain(&paths.claude_dir).as_deref(),
                ctx.port,
            ),
            // Both halves: the running server's option, and the line that
            // survives the next server restart.
            Self::TmuxFocusEvents => {
                tmux::focus_events_enabled(ctx.runner)
                    && tmux::tmux_conf_has_focus_events(&paths.tmux_conf_path)
            }
            Self::FeedScripts => plugins::shipped_scripts_are_current(ctx.data_dir),
        }
    }

    /// Bring this artefact up to date. Writes; asks nothing — consent for the
    /// whole report was obtained once before this was reached.
    fn apply(self, ctx: &ConfigContext<'_>) -> Result<()> {
        let paths = ctx.paths;
        match self {
            Self::McpServerEntry => {
                apply_mcp_setup(&paths.mcp_path, &paths.legacy_mcp_path, ctx.port).map(|_| ())
            }
            Self::Plugin => {
                let plugin_base = plugins::plugin_dir_under(&paths.claude_dir);
                plugins::install_plugin_in(&plugin_base)?;
                report_plugin_install(&plugin_base);
                Ok(())
            }
            Self::StatusLine => {
                let chain = statusline::discover_chain(&paths.claude_dir);
                let wrote = statusline::write_settings_file(
                    &paths.statusline_path,
                    &paths.budget_snapshot_path,
                    chain.as_deref(),
                    ctx.port,
                )?;
                if wrote {
                    println!(
                        "Status line: wrote {} (budget indicator){}",
                        display_for(&paths.statusline_path),
                        match &chain {
                            Some(c) => format!(", chaining to `{c}`"),
                            None => String::new(),
                        }
                    );
                }
                Ok(())
            }
            Self::TmuxFocusEvents => apply_focus_events(paths, ctx.runner),
            // The one arm that may ask a second question. Consent for the
            // report covers writes that destroy nothing; overwriting a script
            // dispatch cannot prove it wrote is the only branch in the system
            // that can destroy the operator's own work, so it asks for itself,
            // defaulting to No, and takes a .bak first. A `None` confirmer —
            // nobody to ask — keeps such a script untouched. See
            // `InstallShippedFeedScripts` in docs/specs/feeds.allium.
            Self::FeedScripts => apply_feed_scripts(ctx),
        }
    }
}

/// Install and update the shipped feed scripts, then their config files, and
/// print what happened.
///
/// A per-script failure is reported rather than raised: the other scripts still
/// landed, and refusing the whole update over one unwritable file would leave
/// the operator with neither the update nor a way to make progress
/// (`PartialFailureIsStillProgress`). A config failure is whole-artefact and is
/// raised, so the engine reports and retries it like any other artefact.
pub(super) fn apply_feed_scripts(ctx: &ConfigContext<'_>) -> Result<()> {
    let reports = plugins::install_shipped_feed_scripts(ctx.data_dir, ctx.confirmer)?;
    let config_error = plugins::install_shipped_feed_configs(ctx.data_dir).err();

    println!(
        "Feed scripts: {}",
        display_for(&ctx.data_dir.join("scripts"))
    );
    for line in plugins::feed_scripts_section_lines(&reports) {
        println!("{line}");
    }

    // A per-script failure is already a line in the report above, and the other
    // scripts still landed — the artefact as a whole is not failed. A config
    // failure is whole-artefact (it is the shared directory, or nothing), so it
    // is raised: the engine then names `feed scripts` among what it could not
    // update and retries it next launch, like any other artefact.
    match config_error {
        Some(e) => Err(e.context("Failed to install the feed script config files")),
        None => Ok(()),
    }
}

/// Every managed artefact whose on-disk state differs from what this build
/// would write. `ConfigDriftEngine.inspect_config_drift`.
///
/// **Reads only** — `InspectWritesNothing`. An operator who declines the prompt
/// ends the run with their configuration byte-identical to how it started.
pub(crate) fn inspect_config_drift_in(ctx: &ConfigContext<'_>) -> ConfigDrift {
    ConfigDrift {
        items: ConfigArtefact::ALL
            .into_iter()
            .filter(|artefact| !artefact.is_current(ctx))
            .collect(),
    }
}

/// Bring the artefacts named in `drift` up to date.
/// `ConfigDriftEngine.apply_config_update`.
///
/// Only the artefacts the report names, because the writers were already
/// idempotent and re-deriving every predicate the inspect just computed bought
/// nothing — a second plugin tree walk, a second tmux subprocess, a second read
/// of every file. `OneDefinitionOfOutOfDate` still holds: `is_current` remains
/// the only definition, now evaluated once.
///
/// Returns the artefacts it could not write — one failure does not abandon the
/// rest (`PartialFailureIsStillProgress`), because refusing the whole update
/// over a single unwritable file leaves the operator with neither the update
/// nor a way to make progress.
pub(crate) fn apply_config_update_in(
    drift: &ConfigDrift,
    ctx: &ConfigContext<'_>,
) -> Vec<ConfigArtefact> {
    if drift.is_clean() {
        return Vec::new();
    }

    if let Err(e) = fs::create_dir_all(&ctx.paths.claude_dir) {
        eprintln!(
            "Warning: failed to create {}: {e}",
            ctx.paths.claude_dir.display()
        );
        return drift.items.clone();
    }

    drift
        .items
        .iter()
        .filter(|artefact| match artefact.apply(ctx) {
            Ok(()) => false,
            Err(e) => {
                eprintln!("Warning: failed to update {}: {e:#}", artefact.label());
                true
            }
        })
        .copied()
        .collect()
}

/// Enable focus-events for the running tmux server and persist the option.
/// Both halves, because the drift check reports on both.
pub(super) fn apply_focus_events(
    paths: &SetupPaths,
    runner: &dyn crate::process::ProcessRunner,
) -> Result<()> {
    // No re-check of whether the option is already set: reaching here means the
    // drift report named this artefact, and `set-option` is idempotent. Asking
    // again would spawn a second tmux subprocess to re-derive what
    // `ConfigArtefact::is_current` just decided.
    tmux::set_focus_events(runner)?;
    println!("Tmux: enabled focus-events for the running server");
    tmux::write_focus_events_to_tmux_conf_at(&paths.tmux_conf_path)
}

/// Report what the plugin install put on disk, naming `plugin_base` itself
/// rather than a hand-written copy of the layout that a rename would leave
/// stale.
pub(super) fn report_plugin_install(plugin_base: &Path) {
    println!(
        "Plugin: installed dispatch plugin to {}/",
        plugin_base.display()
    );
    let skills: Vec<String> = plugins::built_in_skills_dir()
        .map(|d| {
            let mut names: Vec<String> = d
                .dirs()
                .filter_map(|sd| sd.path().file_name()?.to_str().map(|n| format!("/{n}")))
                .collect();
            names.sort();
            names
        })
        .unwrap_or_default();
    println!("  → Skills: {}", skills.join(", "));
    let commands: Vec<String> = plugins::PLUGIN_DIR
        .get_dir("commands")
        .map(|d| {
            let mut names: Vec<String> = d
                .files()
                .filter_map(|f| f.path().file_stem()?.to_str().map(|n| format!("/{n}")))
                .collect();
            names.sort();
            names
        })
        .unwrap_or_default();
    println!("  → Commands: {}", commands.join(", "));
    println!("  → Hooks: task-status, task-usage");
}

// ---------------------------------------------------------------------------
// run_uninstall — reverse of the configuration apply above
// ---------------------------------------------------------------------------
