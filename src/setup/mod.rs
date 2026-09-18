//! First-run setup: MCP config merging, plugin installation (hooks, skills, commands).
//!
//! Split into submodules:
//! - `config` — Claude Code MCP config read/write/merge
//! - `plugins` — embedded plugin install (skills, slash commands, hooks, example feed script)
//! - `hooks` — tests for the embedded hook scripts (the install path lives in `plugins`)

mod config;
mod hooks;
mod plugins;
pub(crate) mod statusline;

use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::tmux;

pub(crate) use config::dispatch_entry_identifying;
pub use config::{has_dispatch_entry, merge_mcp_config, remove_mcp_config, MergeResult};
pub use plugins::{remove_plugin, seed_feed_epics};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Path to Claude Code's user-global configuration directory (`~/.claude`).
///
/// Every caller resolves this once and then passes the result around — see
/// [`SetupPaths::resolve`], [`UninstallPaths::resolve`] and
/// `runtime::StartupPaths::resolve`. Nothing re-derives it mid-flow, which is
/// what lets a test point a whole flow at a temp directory.
///
/// The directory's name comes from `crate::claude_paths`, the one place it is
/// written down — the spawn constant expands the same token. `$HOME` is the
/// other half of the agreement: the spawn constant names this directory as
/// `~/…`, which the launching shell resolves under the home directory, so the
/// two halves agree only while this lookup reads nothing but `$HOME`. Adding a
/// second input here (an operator-settable configuration directory, say) parts
/// them silently for whoever sets it, and the spawn constant would have to stop
/// naming the location outright in the same change. See
/// docs/specs/dispatch.allium:
/// `SpawnSitesAndStartupNameTheSameConfigurationDirectory`.
pub(super) fn claude_dir() -> Result<PathBuf> {
    Ok(claude_dir_in(&home_dir()?))
}

/// [`claude_dir`] under a given home directory.
pub(super) fn claude_dir_in(home: &std::path::Path) -> PathBuf {
    home.join(crate::claude_paths::claude_dir_name!())
}

/// Path to Claude Code's user-global config file (`~/.claude.json`).
///
/// This is where Claude Code reads user-level MCP servers from — *not*
/// `~/.claude/.mcp.json`, which Claude Code does not consume. It is also
/// Claude Code's trust store, which is what the trust-gated dispatch arms read
/// and write through `TuiRuntime::claude_json_path`; the two consumers share
/// this one resolution rather than each deriving the location. See
/// docs/specs/dispatch.allium:
/// `AnUnavailableHomeDirectoryIsAFailureNotAPath`.
pub(super) fn user_global_config_path() -> Result<PathBuf> {
    Ok(user_global_config_path_in(&home_dir()?))
}

/// [`user_global_config_path`] under a given home directory.
///
/// Beside the configuration directory rather than inside it. Its name is
/// stated here and not among the `crate::claude_paths` tokens because nothing
/// launches a session from it: those tokens exist so the spawn-side `~/`
/// literals and the writer-side joins cannot drift apart, and this file has no
/// spawn-side half to agree with.
pub(super) fn user_global_config_path_in(home: &std::path::Path) -> PathBuf {
    home.join(".claude.json")
}

/// The operator's home directory, from a `$HOME` *value* — `None` when the
/// variable is unset.
///
/// An empty value is unavailable, not the root: `export HOME=` is a shell's
/// other spelling of "unset", and `PathBuf::from("").join(..)` yields a path
/// relative to the process's working directory, which is not the operator's
/// anything. Taking the value as a parameter is what makes that testable —
/// `std::env::set_var` is `unsafe` in edition 2024 and races the test
/// harness's threads regardless (see `src/editor.rs::resolve_editor`, the same
/// shape for the same reason).
pub(super) fn home_dir_from_value(home: Option<&str>) -> Result<PathBuf> {
    home.filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .context("$HOME is not set")
}

/// [`home_dir_from_value`] against the real process environment.
///
/// The one reader of `$HOME` behind every location `SetupPaths`,
/// `UninstallPaths` and `runtime::StartupPaths` are built from, so an absent
/// home directory is reported the same way whichever of them a run asked for.
/// Nothing downstream can name the cause — see docs/specs/dispatch.allium:
/// `AnUnavailableHomeDirectoryIsAFailureNotAPath`.
///
/// Not every `$HOME` reader in the crate: `crate::models::expand_tilde` and
/// `crate::default_db_path` resolve their own, and neither fails — they are
/// string-in/string-out and fall back to a default respectively, so they have
/// no `Result` to report an absence through.
pub(super) fn home_dir() -> Result<PathBuf> {
    home_dir_from_value(std::env::var("HOME").ok().as_deref())
}

pub(super) fn read_json_file(path: &std::path::Path) -> Result<Option<Value>> {
    match fs::read_to_string(path) {
        Ok(content) => {
            let value: Value = serde_json::from_str(&content)
                .with_context(|| format!("Invalid JSON in {}", path.display()))?;
            Ok(Some(value))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("Failed to read {}", path.display())),
    }
}

pub(super) fn write_json_file(path: &std::path::Path, value: &Value) -> Result<()> {
    let content = serde_json::to_string_pretty(value).context("Failed to serialize JSON")?;
    fs::write(path, content + "\n").with_context(|| format!("Failed to write {}", path.display()))
}

/// Whether `path` already holds exactly `content` — the single definition of "this
/// file is already up to date" for everything setup manages.
///
/// Two callers must agree on it or setup contradicts itself:
/// [`write_file_if_changed`], which decides whether to write and what to report,
/// and `plugins::plugin_needs_update_in`, which decides whether to *offer* an
/// update. A file the predicate calls stale and the writer calls unchanged would
/// be reported as needing an update forever.
///
/// **Exact bytes, deliberately.** The statusline settings file previously compared
/// `.trim()`-normalized, which is what made the two disagree. Exact equality also
/// rewrites a file something else edited back to the canonical content, and
/// converges rather than flip-flopping. An unreadable or absent file is not up to
/// date: the writer's own error is a better report than a read error here.
fn file_is_up_to_date(path: &std::path::Path, content: &str) -> bool {
    fs::read_to_string(path).is_ok_and(|existing| existing == content)
}

/// Write `content` at `path`, creating parent directories, and report whether the
/// on-disk bytes actually changed. Every setup-managed file goes through this, so
/// setup can be run repeatedly and only report what it really touched.
///
/// Shared by the plugin installer (`plugins::install_dir_recursive`) and the
/// statusline settings file (`statusline::write_settings_file`) — it lives here
/// rather than in either of them so there is one place to change what writing
/// means. Up-to-dateness is [`file_is_up_to_date`]'s to define.
///
/// Deliberately unmarked rather than `pub(…)`: a private item in this module is
/// already reachable from every child that needs it, and a filesystem write is
/// not something the rest of the crate should be able to name.
fn write_file_if_changed(path: &std::path::Path, content: &str, executable: bool) -> Result<bool> {
    if file_is_up_to_date(path, content) {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    fs::write(path, content).with_context(|| format!("Failed to write {}", path.display()))?;
    if executable {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("Failed to set permissions on {}", path.display()))?;
    }
    Ok(true)
}

// ---------------------------------------------------------------------------
// Confirmation seam (mirrors `ProcessRunner` in src/process.rs)
// ---------------------------------------------------------------------------

/// Seam over interactive prompts so the setup/uninstall orchestration flows
/// (and, via [`prompt_text`](Confirmer::prompt_text), the startup host-label
/// gate — see `docs/specs/startup.allium`: `HostLabelPrompt`) can be driven
/// deterministically in tests. The real implementation ([`StdinConfirmer`])
/// reads from stdin; tests inject a fake that returns queued answers.
pub trait Confirmer {
    /// Prompt defaulting to **Yes** (empty input counts as yes).
    fn confirm(&self, prompt: &str) -> Result<bool>;

    /// Prompt defaulting to **No** — the user must explicitly type "y".
    fn confirm_dangerous(&self, prompt: &str) -> Result<bool>;

    /// Prompt for free text with a pre-filled `default`. Empty input (just
    /// pressing enter) accepts the default rather than being treated as a
    /// blank answer — this is what makes `startup.allium`'s
    /// `HostLabelPrompt` a one-keypress accept when the hostname is fine as
    /// the label.
    fn prompt_text(&self, prompt: &str, default: &str) -> Result<String>;
}

/// Real confirmer backed by stderr prompts and stdin input.
pub struct StdinConfirmer;

impl StdinConfirmer {
    /// Prompt on stderr and read a yes/no answer from stdin. `default_yes`
    /// selects both the displayed hint (`[Y/n]` vs `[y/N]`) and the meaning of
    /// empty input.
    fn prompt(&self, prompt: &str, default_yes: bool) -> Result<bool> {
        let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
        eprint!("{prompt} {hint} ");
        std::io::stderr().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let trimmed = input.trim().to_lowercase();
        Ok(match trimmed.as_str() {
            "" => default_yes,
            "y" | "yes" => true,
            _ => false,
        })
    }
}

impl Confirmer for StdinConfirmer {
    fn confirm(&self, prompt: &str) -> Result<bool> {
        self.prompt(prompt, true)
    }

    fn confirm_dangerous(&self, prompt: &str) -> Result<bool> {
        self.prompt(prompt, false)
    }

    fn prompt_text(&self, prompt: &str, default: &str) -> Result<String> {
        eprint!("{prompt} [{default}] ");
        std::io::stderr().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let trimmed = input.trim();
        Ok(if trimmed.is_empty() {
            default.to_string()
        } else {
            trimmed.to_string()
        })
    }
}

fn count_tasks(db_path: &std::path::Path) -> Result<i64> {
    let conn =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))?;
    Ok(count)
}

pub fn remove_database(db_path: &std::path::Path) -> Result<bool> {
    if !db_path.exists() {
        return Ok(false);
    }

    let parent = db_path
        .parent()
        .context("database path has no parent directory")?;

    for name in ["tasks.db", "tasks.db-wal", "tasks.db-shm", "app.log"] {
        let path = parent.join(name);
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("Failed to remove {}", path.display()))?;
        }
    }

    if parent.exists() && parent.read_dir()?.next().is_none() {
        fs::remove_dir(parent).with_context(|| format!("Failed to remove {}", parent.display()))?;
    }

    Ok(true)
}

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
pub(super) fn apply_mcp_setup(
    target: &Path,
    legacy: &Path,
    port: u16,
    headers_helper: &str,
) -> Result<bool> {
    let mut changed = false;

    let existing = read_json_file(target)?;
    let merged = merge_mcp_config(existing, port, headers_helper);
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
fn display_for(path: &Path) -> String {
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
    /// The `headersHelper` command the MCP entry records: the INSTALLED
    /// dispatch binary, resolved from `PATH`, never the binary running this
    /// check. Fixed per machine like the two paths above, and resolved here
    /// for the same reason — see `startup.allium`'s
    /// `TheHelperPathNamesTheInstalledBinary`.
    pub caller_headers_command: String,
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
    /// configuration check performs no `$HOME` lookup of its own. The three
    /// remaining values are fixed per machine rather than per configuration
    /// directory, so they are resolved here — see
    /// `SnapshotLocationIsFixedNotDerivedFromTheOpenDatabase` and, for the
    /// helper command, `startup.allium`'s
    /// `TheHelperPathNamesTheInstalledBinary`.
    pub fn under(claude_dir: &Path, mcp_path: &Path) -> Result<Self> {
        Ok(Self {
            legacy_mcp_path: claude_dir.join(".mcp.json"),
            statusline_path: statusline::settings_path(claude_dir),
            claude_dir: claude_dir.to_path_buf(),
            mcp_path: mcp_path.to_path_buf(),
            tmux_conf_path: tmux::tmux_conf_path()?,
            budget_snapshot_path: crate::budget_snapshot_path(),
            caller_headers_command: config::caller_headers_command(),
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
                let merge_is_noop = read_json_file(&paths.mcp_path).is_ok_and(|existing| {
                    !merge_mcp_config(existing, ctx.port, &paths.caller_headers_command).changed
                });
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
            Self::McpServerEntry => apply_mcp_setup(
                &paths.mcp_path,
                &paths.legacy_mcp_path,
                ctx.port,
                &paths.caller_headers_command,
            )
            .map(|_| ()),
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
fn apply_feed_scripts(ctx: &ConfigContext<'_>) -> Result<()> {
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
fn apply_focus_events(
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
fn report_plugin_install(plugin_base: &Path) {
    println!(
        "Plugin: installed dispatch plugin to {}/",
        plugin_base.display()
    );
    let skills: Vec<String> = plugins::PLUGIN_DIR
        .get_dir("skills")
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

/// Filesystem locations the uninstall flow removes. Grouped so tests can point
/// the whole flow at temp directories instead of the real `$HOME`.
pub(super) struct UninstallPaths {
    pub mcp_path: PathBuf,
    pub legacy_mcp_path: PathBuf,
    pub plugin_path: PathBuf,
    pub db_path: PathBuf,
    pub statusline_path: PathBuf,
}

impl UninstallPaths {
    /// Resolve the real `$HOME`-derived locations used in production.
    fn resolve() -> Result<Self> {
        let claude_dir = claude_dir()?;
        Ok(Self {
            mcp_path: user_global_config_path()?,
            legacy_mcp_path: claude_dir.join(".mcp.json"),
            plugin_path: plugins::plugin_dir()?,
            db_path: crate::default_db_path(),
            statusline_path: statusline::settings_path(&claude_dir),
        })
    }
}

pub fn run_uninstall(yes: bool, purge: bool) -> Result<()> {
    let paths = UninstallPaths::resolve()?;
    run_uninstall_in(&paths, &StdinConfirmer, yes, purge)
}

/// Injectable core of [`run_uninstall`]. Takes the target filesystem locations
/// and a confirmer so the removal decision matrix can be exercised
/// deterministically in tests.
pub(super) fn run_uninstall_in(
    paths: &UninstallPaths,
    confirmer: &dyn Confirmer,
    yes: bool,
    purge: bool,
) -> Result<()> {
    let UninstallPaths {
        mcp_path,
        legacy_mcp_path,
        plugin_path,
        db_path,
        statusline_path,
    } = paths;

    // Show what will be removed
    eprintln!("This will remove:");
    eprintln!("  Plugin:      {}", plugin_path.display());
    eprintln!(
        "  MCP config:  mcpServers.dispatch from {}",
        mcp_path.display()
    );
    eprintln!(
        "  Legacy MCP:  mcpServers.dispatch from {} (if present)",
        legacy_mcp_path.display()
    );
    eprintln!("  Status line: {} (if present)", statusline_path.display());
    if purge {
        eprintln!("  Database:    {}", db_path.display());
    }

    if !yes && !confirmer.confirm("\nContinue?")? {
        println!("Aborted.");
        return Ok(());
    }

    let mut any_removed = false;

    match remove_plugin(plugin_path) {
        Ok(true) => {
            println!("Removed plugin directory");
            any_removed = true;
        }
        Ok(false) => println!("Plugin directory not found, skipping"),
        Err(e) => eprintln!("Warning: failed to remove plugin: {e}"),
    }

    match remove_mcp_config(mcp_path) {
        Ok(true) => {
            println!("Removed dispatch from MCP config");
            any_removed = true;
        }
        Ok(false) => println!("No dispatch entry in MCP config, skipping"),
        Err(e) => eprintln!("Warning: failed to update MCP config: {e}"),
    }

    // Legacy cleanup: remove any stale entry from ~/.claude/.mcp.json that
    // earlier dispatch versions mistakenly wrote there.
    match remove_mcp_config(legacy_mcp_path) {
        Ok(true) => {
            println!(
                "Removed stale dispatch entry from {}",
                legacy_mcp_path.display()
            );
            any_removed = true;
        }
        Ok(false) => {}
        Err(e) => eprintln!("Warning: failed to clean up legacy MCP config: {e}"),
    }

    // Status line settings file — dispatch-owned, written by the startup
    // configuration check
    // (see src/setup/statusline.rs). Best-effort: a missing file is a no-op.
    match fs::remove_file(statusline_path) {
        Ok(()) => {
            println!("Removed status line settings file");
            any_removed = true;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("Status line settings file not found, skipping")
        }
        Err(e) => eprintln!("Warning: failed to remove status line settings file: {e}"),
    }

    // Note: ~/.claude/settings.json is intentionally not touched. Dispatch no
    // longer manages permissions in that file — it is user-owned config. Users
    // who ran an older `dispatch setup` may have stale mcp__dispatch__* entries
    // in settings.json; those are inert once the MCP server is removed and can
    // be cleaned up manually.

    if purge {
        if db_path.exists() {
            let task_count = count_tasks(db_path).unwrap_or(0);
            eprintln!("\n  Database contains {task_count} task(s). This cannot be undone.");
            if confirmer.confirm_dangerous("Delete database?")? {
                match remove_database(db_path) {
                    Ok(true) => {
                        println!("Removed database");
                        any_removed = true;
                    }
                    Ok(false) => println!("Database not found, skipping"),
                    Err(e) => eprintln!("Warning: failed to remove database: {e}"),
                }
            } else {
                println!("Kept database.");
            }
        } else {
            println!("Database not found, skipping");
        }
    }

    if any_removed {
        println!("Uninstall complete.");
    } else {
        println!("Nothing to remove.");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests for shared helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Test seam
// ---------------------------------------------------------------------------

/// A [`Confirmer`] that returns queued answers instead of reading stdin,
/// mirroring `MockProcessRunner`. Separate queues for the default-yes and
/// default-no (dangerous) prompts so tests assert which kind fired. Panics if a
/// prompt is issued with no queued answer — the same fail-loud contract as
/// `MockProcessRunner`.
///
/// Lives outside `mod tests` because two test modules drive prompts: this one
/// (uninstall) and `crate::startup`'s (the startup consent prompt). A second
/// copy could answer differently from this one and neither would look wrong.
#[cfg(test)]
pub(crate) struct FakeConfirmer {
    confirm_answers: std::sync::Mutex<std::collections::VecDeque<bool>>,
    dangerous_answers: std::sync::Mutex<std::collections::VecDeque<bool>>,
    text_answers: std::sync::Mutex<std::collections::VecDeque<String>>,
    confirm_calls: std::sync::Mutex<usize>,
    dangerous_calls: std::sync::Mutex<usize>,
    text_calls: std::sync::Mutex<usize>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
impl FakeConfirmer {
    pub(crate) fn new(confirm: Vec<bool>, dangerous: Vec<bool>) -> Self {
        Self::with_text(confirm, dangerous, vec![])
    }

    /// Like [`Self::new`], with a queue of text answers for
    /// [`Confirmer::prompt_text`] — `startup.allium`'s `HostLabelPrompt`.
    pub(crate) fn with_text(confirm: Vec<bool>, dangerous: Vec<bool>, text: Vec<String>) -> Self {
        Self {
            confirm_answers: std::sync::Mutex::new(confirm.into()),
            dangerous_answers: std::sync::Mutex::new(dangerous.into()),
            text_answers: std::sync::Mutex::new(text.into()),
            confirm_calls: std::sync::Mutex::new(0),
            dangerous_calls: std::sync::Mutex::new(0),
            text_calls: std::sync::Mutex::new(0),
        }
    }

    /// Confirmer that must never be prompted.
    pub(crate) fn never() -> Self {
        Self::new(vec![], vec![])
    }

    pub(crate) fn confirm_call_count(&self) -> usize {
        *self.confirm_calls.lock().unwrap()
    }

    pub(crate) fn dangerous_call_count(&self) -> usize {
        *self.dangerous_calls.lock().unwrap()
    }

    pub(crate) fn text_call_count(&self) -> usize {
        *self.text_calls.lock().unwrap()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
impl Confirmer for FakeConfirmer {
    fn confirm(&self, _prompt: &str) -> Result<bool> {
        *self.confirm_calls.lock().unwrap() += 1;
        Ok(self
            .confirm_answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("FakeConfirmer: no confirm answer queued"))
    }

    fn confirm_dangerous(&self, _prompt: &str) -> Result<bool> {
        *self.dangerous_calls.lock().unwrap() += 1;
        Ok(self
            .dangerous_answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("FakeConfirmer: no dangerous answer queued"))
    }

    fn prompt_text(&self, _prompt: &str, _default: &str) -> Result<String> {
        *self.text_calls.lock().unwrap() += 1;
        Ok(self
            .text_answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("FakeConfirmer: no text answer queued"))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::db::Database;
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

    // -- Database removal --

    #[test]
    fn remove_database_deletes_db_and_related_files() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("dispatch");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("tasks.db"), "db").unwrap();
        fs::write(data_dir.join("tasks.db-wal"), "wal").unwrap();
        fs::write(data_dir.join("tasks.db-shm"), "shm").unwrap();
        fs::write(data_dir.join("app.log"), "log").unwrap();

        let db_path = data_dir.join("tasks.db");
        let removed = remove_database(&db_path).unwrap();
        assert!(removed);
        assert!(!data_dir.join("tasks.db").exists());
        assert!(!data_dir.join("tasks.db-wal").exists());
        assert!(!data_dir.join("tasks.db-shm").exists());
        assert!(!data_dir.join("app.log").exists());
        assert!(!data_dir.exists());
    }

    #[test]
    fn remove_database_keeps_parent_if_not_empty() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("dispatch");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("tasks.db"), "db").unwrap();
        fs::write(data_dir.join("other.txt"), "keep").unwrap();

        let db_path = data_dir.join("tasks.db");
        let removed = remove_database(&db_path).unwrap();
        assert!(removed);
        assert!(!data_dir.join("tasks.db").exists());
        assert!(data_dir.exists());
        assert!(data_dir.join("other.txt").exists());
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

        let changed = apply_mcp_setup(&target, &legacy, 3142, TEST_HELPER_COMMAND).unwrap();
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

        apply_mcp_setup(&target, &legacy, 3142, TEST_HELPER_COMMAND).unwrap();

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

        let changed = apply_mcp_setup(&target, &legacy, 3142, TEST_HELPER_COMMAND).unwrap();
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

        apply_mcp_setup(&target, &legacy, 3142, TEST_HELPER_COMMAND).unwrap();
        let changed = apply_mcp_setup(&target, &legacy, 3142, TEST_HELPER_COMMAND).unwrap();
        assert!(
            !changed,
            "second apply with no changes must report unchanged"
        );
    }

    #[test]
    fn remove_database_noop_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("dispatch").join("tasks.db");

        let removed = remove_database(&db_path).unwrap();
        assert!(!removed);
    }

    #[test]
    fn setup_does_not_write_settings_json() {
        // Regression guard: the setup flow must not create or modify settings.json.
        // That file is user-owned config; dispatch must not add permissions to it.
        let dir = tempfile::tempdir().unwrap();
        let claude_json = dir.path().join(".claude.json");
        let legacy = dir.path().join(".mcp.json");
        let settings = dir.path().join("settings.json");

        apply_mcp_setup(&claude_json, &legacy, 3142, TEST_HELPER_COMMAND).unwrap();

        assert!(
            !settings.exists(),
            "setup must not create settings.json; permissions are user-managed"
        );
    }

    // -- count_tasks --

    #[tokio::test]
    async fn count_tasks_reports_zero_for_fresh_db() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("dispatch").join("tasks.db");
        fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        // Create the schema, then drop the handle so count_tasks can open it.
        let db = Database::open(&db_path).await.unwrap();
        drop(db);

        let count = count_tasks(&db_path).unwrap();
        assert_eq!(count, 0, "a freshly-created db has no tasks");
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

    #[tokio::test]
    async fn run_uninstall_in_purge_deletes_db_when_dangerous_confirmed() {
        let dir = tempfile::tempdir().unwrap();
        let paths = uninstall_layout(dir.path());
        let db = Database::open(&paths.db_path).await.unwrap();
        drop(db);
        assert!(paths.db_path.exists());

        // confirm "Continue?" -> yes; confirm_dangerous "Delete database?" -> yes.
        let confirmer = FakeConfirmer::new(vec![true], vec![true]);
        run_uninstall_in(&paths, &confirmer, false, true).unwrap();

        assert!(!paths.db_path.exists(), "purge must delete the database");
        assert_eq!(confirmer.dangerous_call_count(), 1);
    }

    #[tokio::test]
    async fn run_uninstall_in_purge_keeps_db_when_dangerous_declined() {
        let dir = tempfile::tempdir().unwrap();
        let paths = uninstall_layout(dir.path());
        let db = Database::open(&paths.db_path).await.unwrap();
        drop(db);

        let confirmer = FakeConfirmer::new(vec![true], vec![false]);
        run_uninstall_in(&paths, &confirmer, false, true).unwrap();

        assert!(
            paths.db_path.exists(),
            "declining the dangerous prompt must keep the database"
        );
    }

    #[tokio::test]
    async fn run_uninstall_in_yes_still_prompts_before_deleting_db() {
        // Regression guard: --yes suppresses "Continue?" but must NOT
        // auto-confirm the irreversible database deletion.
        let dir = tempfile::tempdir().unwrap();
        let paths = uninstall_layout(dir.path());
        let db = Database::open(&paths.db_path).await.unwrap();
        drop(db);

        // No confirm answers queued (would panic if consulted); dangerous -> no.
        let confirmer = FakeConfirmer::new(vec![], vec![false]);
        run_uninstall_in(&paths, &confirmer, true, true).unwrap();

        assert_eq!(confirmer.confirm_call_count(), 0, "--yes skips 'Continue?'");
        assert_eq!(
            confirmer.dangerous_call_count(),
            1,
            "--yes must still prompt before deleting the database"
        );
        assert!(paths.db_path.exists(), "db kept because dangerous declined");
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
            caller_headers_command: TEST_HELPER_COMMAND.to_string(),
        }
    }

    /// Stands in for the installed binary's command, so a test asserting about
    /// the MCP entry does not depend on this machine's `PATH`. The one test
    /// that does read it is `setup_paths_compose_the_shared_configuration_layout`,
    /// which is pinning that `SetupPaths::under` resolves the command at all.
    const TEST_HELPER_COMMAND: &str = "/usr/local/bin/dispatch caller-headers";

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
        assert!(
            paths.caller_headers_command.ends_with(" caller-headers"),
            "the helper command must be resolved as part of the layout, not left \
             to whoever writes the MCP entry, got {}",
            paths.caller_headers_command
        );
        assert!(
            !paths.caller_headers_command.contains(
                std::env::current_exe()
                    .expect("the test binary must have a path")
                    .to_str()
                    .expect("the test binary path must be UTF-8")
            ),
            "docs/specs/startup.allium: TheHelperPathNamesTheInstalledBinary — the \
             running binary must not reach the composed command, got {}",
            paths.caller_headers_command
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
}
