//! Plugin install: skills, slash commands, hooks (embedded at compile time),
//! plus the example feed script and feed-epic seeding.

use anyhow::{Context, Result};
use include_dir::{include_dir, Dir};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::db::{Database, EpicCrud, EpicPatch, EpicRead};

// The entire plugin/ directory is embedded at compile time. Any file added to
// plugin/ is automatically picked up — no manual registration required.
pub(super) static PLUGIN_DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/plugin");

// ---------------------------------------------------------------------------
// Plugin installation
// ---------------------------------------------------------------------------

pub(super) fn plugin_dir() -> Result<PathBuf> {
    Ok(plugin_dir_under(&super::claude_dir()?))
}

/// Resolve the plugin install directory beneath an explicit `~/.claude`-style
/// directory. Kept separate from [`plugin_dir`] so orchestration code can
/// inject a temp directory in tests.
pub(super) fn plugin_dir_under(claude_dir: &Path) -> PathBuf {
    claude_dir.join(crate::claude_paths::plugin_dir_rel!())
}

fn is_executable(path: &std::path::Path) -> bool {
    path.starts_with("hooks/scripts")
}

pub(super) fn install_plugin_in(base: &Path) -> Result<bool> {
    let mut changed = false;
    install_dir_recursive(&PLUGIN_DIR, base, &mut changed)?;
    remove_stale_files(base, &mut changed)?;
    Ok(changed)
}

fn install_dir_recursive(dir: &Dir, base: &std::path::Path, changed: &mut bool) -> Result<()> {
    for file in dir.files() {
        let path = base.join(file.path());
        let content = file
            .contents_utf8()
            .with_context(|| format!("Non-UTF-8 plugin file: {}", file.path().display()))?;
        *changed |= super::write_file_if_changed(&path, content, is_executable(file.path()))?;
    }
    for subdir in dir.dirs() {
        install_dir_recursive(subdir, base, changed)?;
    }
    Ok(())
}

fn embedded_path_set() -> std::collections::HashSet<PathBuf> {
    fn collect(dir: &Dir, paths: &mut std::collections::HashSet<PathBuf>) {
        for file in dir.files() {
            paths.insert(file.path().to_path_buf());
        }
        for subdir in dir.dirs() {
            collect(subdir, paths);
        }
    }
    let mut paths = std::collections::HashSet::new();
    collect(&PLUGIN_DIR, &mut paths);
    paths
}

fn remove_stale_files(base: &Path, changed: &mut bool) -> Result<()> {
    if !base.exists() {
        return Ok(());
    }
    let embedded = embedded_path_set();
    remove_stale_recursive(base, base, &embedded, changed)
}

fn remove_stale_recursive(
    base: &Path,
    dir: &Path,
    embedded: &std::collections::HashSet<PathBuf>,
    changed: &mut bool,
) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("Failed to read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            remove_stale_recursive(base, &path, embedded, changed)?;
            if fs::read_dir(&path)?.next().is_none() {
                fs::remove_dir(&path)
                    .with_context(|| format!("Failed to remove {}", path.display()))?;
                *changed = true;
            }
        } else {
            let relative = path.strip_prefix(base).with_context(|| {
                format!(
                    "path {} is not under base {}",
                    path.display(),
                    base.display()
                )
            })?;
            if !embedded.contains(relative) {
                fs::remove_file(&path)
                    .with_context(|| format!("Failed to remove {}", path.display()))?;
                *changed = true;
            }
        }
    }
    Ok(())
}

pub(super) fn plugin_needs_update_in(base: &std::path::Path) -> Result<bool> {
    if needs_update_recursive(&PLUGIN_DIR, base)? {
        return Ok(true);
    }
    has_stale_files(base)
}

fn needs_update_recursive(dir: &Dir, base: &std::path::Path) -> Result<bool> {
    for file in dir.files() {
        let path = base.join(file.path());
        let content = file.contents_utf8().unwrap_or("");
        // Same predicate the installer writes by, so "needs an update" and "was
        // actually written" can never disagree.
        if !super::file_is_up_to_date(&path, content) {
            return Ok(true);
        }
    }
    for subdir in dir.dirs() {
        if needs_update_recursive(subdir, base)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn has_stale_files(base: &Path) -> Result<bool> {
    if !base.exists() {
        return Ok(false);
    }
    let embedded = embedded_path_set();
    has_stale_recursive(base, base, &embedded)
}

fn has_stale_recursive(
    base: &Path,
    dir: &Path,
    embedded: &std::collections::HashSet<PathBuf>,
) -> Result<bool> {
    for entry in fs::read_dir(dir).with_context(|| format!("Failed to read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if has_stale_recursive(base, &path, embedded)? {
                return Ok(true);
            }
        } else {
            let relative = path.strip_prefix(base).with_context(|| {
                format!(
                    "path {} is not under base {}",
                    path.display(),
                    base.display()
                )
            })?;
            if !embedded.contains(relative) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub fn remove_plugin(plugin_path: &std::path::Path) -> Result<bool> {
    if !plugin_path.exists() {
        return Ok(false);
    }
    fs::remove_dir_all(plugin_path)
        .with_context(|| format!("Failed to remove {}", plugin_path.display()))?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// Shipped feed scripts + epic seeding
// ---------------------------------------------------------------------------
//
// Two classes of file land in `<data_dir>/scripts/`, with deliberately
// opposite rules. See `InstallShippedFeedScripts` and
// `InstallShippedFeedConfigs` in docs/specs/feeds.allium.
//
// The split is not arbitrary. Every shipped script ships with EMPTY config
// placeholders (`REPOS=()`, `ORGS=()`, `BOT_AUTHORS=()`, `LOG_FILE=""`) — the
// whole edit surface is the `.conf` files beside them. So a script on disk is
// expected to stay byte-identical to what dispatch shipped, and a config file
// is expected to diverge the moment the user configures anything.

/// One file dispatch ships into `<data_dir>/scripts/`, paired with the body
/// embedded for it at compile time.
///
/// Name and body live in one table entry rather than in a name list beside a
/// lookup: a mismatch between the two is then a compile error instead of a
/// user-visible "no embedded content" line at install time.
#[derive(Debug, Clone, Copy)]
pub struct ShippedFile {
    pub name: &'static str,
    pub content: &'static str,
}

const fn shipped(name: &'static str, content: &'static str) -> ShippedFile {
    ShippedFile { name, content }
}

/// Dispatch-owned scripts. Installed by the startup configuration check,
/// executable, and kept current thereafter through the provenance manifest.
///
/// The whole shipped set, not a curated subset: before this existed only
/// `fetch-dependabot.sh` was installed, so a fix to any of the other four
/// reached a deployment only if the user copied it out of a git checkout —
/// and a release-binary install has no checkout to copy from.
pub const SHIPPED_SCRIPTS: [ShippedFile; 5] = [
    shipped(
        "fetch-dependabot.sh",
        include_str!("../../scripts/fetch-dependabot.sh"),
    ),
    shipped(
        "fetch-reviews.sh",
        include_str!("../../scripts/fetch-reviews.sh"),
    ),
    shipped("fetch-cve.sh", include_str!("../../scripts/fetch-cve.sh")),
    shipped(
        "fetch-security.sh",
        include_str!("../../scripts/fetch-security.sh"),
    ),
    shipped(
        "fetch-log-warnings.sh",
        include_str!("../../scripts/fetch-log-warnings.sh"),
    ),
];

/// User-owned config files. Created if absent and never touched again.
///
/// `bots.conf` is read by both `fetch-dependabot.sh` and `fetch-reviews.sh`'s
/// bot-author pass — one list, so a deployment does not spell its bots twice.
pub const SHIPPED_SCRIPT_CONFIGS: [ShippedFile; 4] = [
    shipped("repos.conf", include_str!("../../scripts/repos.conf")),
    shipped("bots.conf", include_str!("../../scripts/bots.conf")),
    shipped("org.conf", include_str!("../../scripts/org.conf")),
    shipped(
        "log-warnings.conf",
        include_str!("../../scripts/log-warnings.conf"),
    ),
];

/// Provenance manifest, written alongside the scripts it describes so a moved
/// or copied `<data_dir>` carries its provenance with it.
pub const SCRIPT_MANIFEST_NAME: &str = ".install-manifest.json";

/// Suffix for the pre-overwrite copy taken on the one destructive branch.
pub const SCRIPT_BACKUP_SUFFIX: &str = ".bak";

/// Where a shipped script, config file or the manifest lives under `data_dir`.
pub fn installed_script_path(data_dir: &Path, name: &str) -> PathBuf {
    data_dir.join("scripts").join(name)
}

/// The mode every shipped script is installed with. The feed runner executes
/// them, so bytes alone are not enough to call one installed.
const SCRIPT_MODE: u32 = 0o755;

/// Whether `path` already carries [`SCRIPT_MODE`].
///
/// One definition, shared by the drift predicate and the installer. Two copies
/// could disagree, and a disagreement here is silent in both directions: a
/// script reported stale forever, or one left unexecutable forever.
fn has_script_mode(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|meta| meta.permissions().mode() & 0o777 == SCRIPT_MODE)
}

/// Lowercase hex SHA-256 of `content` — the exact string stored as a manifest
/// value.
pub fn script_digest(content: &str) -> String {
    use std::fmt::Write;
    hmac_sha256::Hash::hash(content.as_bytes()).iter().fold(
        String::with_capacity(64),
        |mut acc, b| {
            // Writing to a String cannot fail.
            let _ = write!(acc, "{b:02x}");
            acc
        },
    )
}

/// The pre-overwrite copy's path: the script's own path plus
/// [`SCRIPT_BACKUP_SUFFIX`]. There is only ever one per script — it is a
/// safety net for the overwrite just approved, not an archive.
fn script_backup_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(SCRIPT_BACKUP_SUFFIX);
    path.with_file_name(name)
}

/// What happened to one shipped script during an install pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShippedScriptOutcome {
    /// Already the shipped content. Nothing written, nothing reported.
    InSync,
    /// Nothing was at the path.
    Installed,
    /// Brought up to the shipped content — silently on proven provenance, or
    /// after an explicit yes on unknown provenance.
    Updated,
    /// Left as the user had it. The one outcome that leaves the deployment
    /// running something other than what dispatch ships.
    Kept,
    /// The backup, write or chmod raised. Never fails the run.
    Failed,
}

/// One script's line in the `Feed scripts:` report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShippedScriptReport {
    pub name: String,
    pub path: PathBuf,
    pub outcome: ShippedScriptOutcome,
    /// Set iff a `.bak` copy was taken.
    pub backup_path: Option<PathBuf>,
    /// Non-null exactly when `outcome` is [`ShippedScriptOutcome::Failed`].
    pub error: Option<String>,
}

impl ShippedScriptReport {
    fn new(name: &str, path: PathBuf, outcome: ShippedScriptOutcome) -> Self {
        Self {
            name: name.to_string(),
            path,
            outcome,
            backup_path: None,
            error: None,
        }
    }

    fn failed(name: &str, path: PathBuf, error: impl std::fmt::Display) -> Self {
        Self {
            name: name.to_string(),
            path,
            outcome: ShippedScriptOutcome::Failed,
            backup_path: None,
            error: Some(error.to_string()),
        }
    }
}

/// Read the provenance manifest as a name -> digest map.
///
/// A manifest that is MISSING, UNREADABLE OR MALFORMED reads as EMPTY and
/// never raises. It exists to let an untouched copy be updated without asking;
/// losing it costs prompts, not correctness, because every script then falls
/// to the conservative unknown-provenance branch. A setup that refused to run
/// because a JSON file it wrote itself was truncated would trade the user's
/// whole install on a cache.
fn read_script_manifest(data_dir: &Path) -> std::collections::BTreeMap<String, String> {
    let path = installed_script_path(data_dir, SCRIPT_MANIFEST_NAME);
    let Ok(raw) = fs::read_to_string(&path) else {
        return std::collections::BTreeMap::new();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

/// Persist the provenance manifest. A write failure is logged and swallowed
/// for the same reason a read failure is: the manifest is a cache of consent,
/// not the install itself.
fn write_script_manifest(data_dir: &Path, manifest: &std::collections::BTreeMap<String, String>) {
    let path = installed_script_path(data_dir, SCRIPT_MANIFEST_NAME);
    let encoded = match serde_json::to_string_pretty(manifest) {
        Ok(encoded) => encoded,
        Err(e) => {
            tracing::warn!("failed to encode {}: {e}", path.display());
            return;
        }
    };
    if let Err(e) = fs::write(&path, encoded) {
        tracing::warn!("failed to write {}: {e}", path.display());
    }
}

/// Give `path` mode 0755 unless it already has it.
///
/// Separate from [`write_shipped_script`] because the in-sync branch needs the
/// mode half without the write half: a script whose bytes are right and whose
/// mode is not is not in sync with what dispatch ships.
fn ensure_executable(path: &Path) -> Result<()> {
    if has_script_mode(path) {
        return Ok(());
    }
    fs::set_permissions(path, fs::Permissions::from_mode(SCRIPT_MODE))
        .with_context(|| format!("Failed to set permissions on {}", path.display()))
}

/// Write `content` to `path` and mark it executable.
fn write_shipped_script(path: &Path, content: &str) -> Result<()> {
    // `write_file_if_changed` also creates the parent directory and is setup's
    // one writer; it can return false here only if the content already matched,
    // in which case the chmod below is still the half we want.
    super::write_file_if_changed(path, content, true)?;
    ensure_executable(path)
}

/// Whether `<data_dir>/scripts/` already holds what this build would write.
///
/// **Reads only** — the startup drift check calls this, and
/// `startup.allium`'s `InspectWritesNothing` forbids creating so much as an
/// empty directory from an inspect. In particular it does NOT touch the
/// provenance manifest: the manifest decides HOW an out-of-date script is
/// updated (silently or with a prompt), never WHETHER it is out of date.
///
/// A script counts as current only if its bytes AND its mode match. A file the
/// feed runner cannot execute is not in sync with what dispatch ships, however
/// right its content is.
pub fn shipped_scripts_are_current(data_dir: &Path) -> bool {
    SHIPPED_SCRIPTS.iter().all(|file| {
        let path = installed_script_path(data_dir, file.name);
        // `file_is_up_to_date` is setup's single definition of "already up to
        // date"; a second copy here could disagree with the writer and report
        // a file stale forever.
        super::file_is_up_to_date(&path, file.content) && has_script_mode(&path)
    }) && SHIPPED_SCRIPT_CONFIGS
        .iter()
        .all(|file| installed_script_path(data_dir, file.name).exists())
}

/// Install and update every script in [`SHIPPED_SCRIPTS`] under `data_dir`.
///
/// `interactive` is true when setup may ask the user a question; it is false
/// under `--yes`. It is a capability rather than the flag itself because the
/// decision below turns on "can I ask?", not on how the invocation was spelled.
///
/// Exactly one branch fires per script:
///
///   1. ABSENT -> write, mark executable, record the digest. `Installed`.
///   2. PRESENT AND IDENTICAL -> nothing to write. The digest is recorded
///      anyway: the file demonstrably IS the shipped content, so claiming
///      provenance over it asserts nothing untrue, and it lets the NEXT
///      release update silently instead of prompting. This is how a
///      deployment that predates the manifest heals itself. `InSync`.
///   3. DIFFERS, ON-DISK DIGEST == RECORDED DIGEST -> dispatch wrote this file
///      and nobody has touched it since; a newer version now ships. Overwrite,
///      re-record. No prompt, no backup — the content being replaced is
///      byte-for-byte a previous release's, recoverable from that release, and
///      backing it up would litter every upgrade with `.bak` files nobody
///      asked for. `Updated`.
///   4. DIFFERS, DIGEST MATCHES NO RECORDED DIGEST -> provenance is UNKNOWN.
///      Covers both "no manifest entry at all" (a pre-manifest deployment or a
///      hand-copied file) and "the user edited it". Interactive: prompt
///      through `confirm_dangerous`, which defaults to NO, and on yes copy to
///      `<name>.bak` BEFORE writing. Non-interactive: keep it. `--yes` means
///      "do not stop to ask about the safe things", not "destroy a file of
///      unknown provenance while nobody is watching" — a script-invoked setup
///      in CI must be incapable of eating a local edit.
///
/// Branch 4 is the only branch that can destroy user content, and it needs
/// both an interactive session and an explicit yes.
///
/// A backup, write or chmod failure for ONE script yields `Failed` for that
/// script and does not fail the run — the remaining scripts, the config files
/// and everything downstream still happen. A failed script's digest is never
/// recorded: claiming provenance over a file dispatch did not successfully
/// write would convert this failure into next run's silent overwrite.
pub fn install_shipped_feed_scripts(
    data_dir: &Path,
    confirmer: Option<&dyn super::Confirmer>,
) -> Result<Vec<ShippedScriptReport>> {
    let scripts_dir = data_dir.join("scripts");
    if let Err(e) = fs::create_dir_all(&scripts_dir) {
        // Nothing can be written, but the caller decides what that means for
        // the run; here it is five failures with one cause.
        let error = format!("Failed to create {}: {e}", scripts_dir.display());
        return Ok(SHIPPED_SCRIPTS
            .iter()
            .map(|file| {
                ShippedScriptReport::failed(
                    file.name,
                    installed_script_path(data_dir, file.name),
                    &error,
                )
            })
            .collect());
    }

    let mut manifest = read_script_manifest(data_dir);
    let mut reports = Vec::with_capacity(SHIPPED_SCRIPTS.len());

    for file in SHIPPED_SCRIPTS {
        let ShippedFile {
            name,
            content: shipped,
        } = file;
        let path = installed_script_path(data_dir, name);

        // One read answers all three cases. Only NotFound is an empty slot: a
        // file that exists but cannot be read (wrong permissions, not UTF-8) is
        // unknown provenance, and treating it as absent would overwrite it with
        // no prompt and no backup.
        let report = match fs::read_to_string(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                match write_shipped_script(&path, shipped) {
                    Ok(()) => {
                        manifest.insert(name.to_string(), script_digest(shipped));
                        ShippedScriptReport::new(name, path, ShippedScriptOutcome::Installed)
                    }
                    Err(e) => ShippedScriptReport::failed(name, path, e),
                }
            }
            Ok(ref current) if current == shipped => {
                // The content is right, so there is nothing to write — but the
                // MODE may still be wrong. `write_shipped_script` writes before
                // it chmods, so a chmod that raised on an earlier run left the
                // right bytes behind a mode the feed runner cannot execute.
                // Matching on content alone would adopt that file as in_sync
                // and record a digest for it, and the failure would never be
                // reported again. Repair it here instead.
                match ensure_executable(&path) {
                    Ok(()) => {
                        manifest.insert(name.to_string(), script_digest(shipped));
                        ShippedScriptReport::new(name, path, ShippedScriptOutcome::InSync)
                    }
                    Err(e) => ShippedScriptReport::failed(name, path, e),
                }
            }
            current => {
                // `recorded == digest(on_disk)` is false when there is no entry
                // AND when the file changed, so one test covers both meanings
                // of unknown provenance. An unreadable file has no digest at
                // all and lands here too.
                let proven = current.ok().is_some_and(|c| {
                    manifest
                        .get(name)
                        .is_some_and(|rec| *rec == script_digest(&c))
                });
                if proven {
                    match write_shipped_script(&path, shipped) {
                        Ok(()) => {
                            manifest.insert(name.to_string(), script_digest(shipped));
                            ShippedScriptReport::new(name, path, ShippedScriptOutcome::Updated)
                        }
                        Err(e) => ShippedScriptReport::failed(name, path, e),
                    }
                } else {
                    install_unknown_provenance_script(
                        name,
                        path,
                        shipped,
                        confirmer,
                        &mut manifest,
                    )?
                }
            }
        };
        reports.push(report);
    }

    write_script_manifest(data_dir, &manifest);
    Ok(reports)
}

/// Branch 4: the file differs and dispatch cannot prove it wrote it.
///
/// The backup is taken BEFORE the write, not after and not "if the write
/// succeeds", so a crash mid-write cannot leave the user with neither copy. A
/// backup that fails abandons the script before any write — the fallback on
/// "cannot back up" is to not destroy, never to destroy without a copy.
fn install_unknown_provenance_script(
    name: &str,
    path: PathBuf,
    shipped: &str,
    confirmer: Option<&dyn super::Confirmer>,
    manifest: &mut std::collections::BTreeMap<String, String>,
) -> Result<ShippedScriptReport> {
    // The ABSENCE of a confirmer is how "nobody can answer" is expressed, the
    // same way the startup check expresses it — so there is no flag a caller
    // could set inconsistently, and no path that reads a yes out of silence.
    let approved = match confirmer {
        None => false,
        Some(confirmer) => confirmer.confirm_dangerous(&format!(
            "{} differs from the version dispatch ships. Overwrite it? \
             (the current file is saved to {})",
            path.display(),
            script_backup_path(&path).display()
        ))?,
    };
    if !approved {
        // No digest recorded: the provenance is still unknown, and recording
        // one would license a silent overwrite next run.
        return Ok(ShippedScriptReport::new(
            name,
            path,
            ShippedScriptOutcome::Kept,
        ));
    }

    let backup = script_backup_path(&path);
    if let Err(e) = fs::copy(&path, &backup) {
        return Ok(ShippedScriptReport::failed(
            name,
            path,
            format!("Failed to back up to {}: {e}", backup.display()),
        ));
    }
    let mut report = match write_shipped_script(&path, shipped) {
        Ok(()) => {
            manifest.insert(name.to_string(), script_digest(shipped));
            ShippedScriptReport::new(name, path, ShippedScriptOutcome::Updated)
        }
        // The backup was already taken, so it is on disk whether or not the
        // write landed — and a failure here is precisely when the user needs
        // to find it. `backup_path` is set below for both arms.
        Err(e) => ShippedScriptReport::failed(name, path, e),
    };
    report.backup_path = Some(backup);
    Ok(report)
}

/// Create each file in [`SHIPPED_SCRIPT_CONFIGS`] if and only if nothing
/// exists at its path.
///
/// The user-owned half. If something is already there, setup does not read it,
/// diff it, prompt about it, back it up or record it in the manifest. There is
/// no branch that writes over a `.conf` file — not with `--yes`, not with a
/// prompt, not ever. It deliberately takes no confirmer: there is no question
/// to ask.
pub fn install_shipped_feed_configs(data_dir: &Path) -> Result<()> {
    let scripts_dir = data_dir.join("scripts");
    fs::create_dir_all(&scripts_dir)
        .with_context(|| format!("Failed to create {}", scripts_dir.display()))?;

    for file in SHIPPED_SCRIPT_CONFIGS {
        install_if_absent(&scripts_dir.join(file.name), file.content)?;
    }
    Ok(())
}

/// The indented detail lines of the `Feed scripts:` section. The caller prints
/// the header.
///
/// Failures come FIRST, then the changes, each group in [`SHIPPED_SCRIPTS`]
/// order so repeated runs print stably. Putting a failure last risks it
/// scrolling off behind four successes. `InSync` scripts print nothing — a
/// section that lists all five every run is noise the user learns to skip,
/// which costs them the one line that is not noise.
///
/// When nothing is reportable the section is a single "already up to date"
/// line rather than nothing at all: a missing section is ambiguous between
/// "nothing to do" and "this version of dispatch does not do that yet".
pub fn feed_scripts_section_lines(reports: &[ShippedScriptReport]) -> Vec<String> {
    let shipped_order = |report: &ShippedScriptReport| {
        SHIPPED_SCRIPTS
            .iter()
            .position(|f| f.name == report.name)
            .unwrap_or(usize::MAX)
    };
    let (mut failures, mut changes): (Vec<_>, Vec<_>) = reports
        .iter()
        .filter(|r| r.outcome != ShippedScriptOutcome::InSync)
        .partition(|r| r.outcome == ShippedScriptOutcome::Failed);
    failures.sort_by_key(|r| shipped_order(r));
    changes.sort_by_key(|r| shipped_order(r));

    if failures.is_empty() && changes.is_empty() {
        return vec!["  → already up to date".to_string()];
    }
    failures
        .into_iter()
        .chain(changes)
        .filter_map(script_report_line)
        .collect()
}

/// One report's line, or `None` for an outcome the section does not print.
fn script_report_line(report: &ShippedScriptReport) -> Option<String> {
    let path = report.path.display();
    Some(match report.outcome {
        // Not reported: a section listing all five every launch is noise the
        // operator learns to skip, which costs them the line that is not noise.
        ShippedScriptOutcome::InSync => return None,
        ShippedScriptOutcome::Failed => {
            let error = report.error.as_deref().unwrap_or("unknown error");
            match &report.backup_path {
                Some(backup) => format!(
                    "  → failed {path}: {error} (your previous copy is at {})",
                    backup.display()
                ),
                None => format!("  → failed {path}: {error}"),
            }
        }
        ShippedScriptOutcome::Installed => format!("  → installed {path}"),
        ShippedScriptOutcome::Updated => match &report.backup_path {
            Some(backup) => format!(
                "  → updated {path} (your previous copy saved to {})",
                backup.display()
            ),
            None => format!("  → updated {path}"),
        },
        ShippedScriptOutcome::Kept => format!(
            "  → kept {path} (differs from the version dispatch ships; \
             re-run without --yes to update it)"
        ),
    })
}

/// Create `path` with `content` only if it does not already exist. Preserves
/// user edits across repeated startup configuration updates.
fn install_if_absent(path: &std::path::Path, content: &str) -> Result<()> {
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => file
            .write_all(content.as_bytes())
            .with_context(|| format!("Failed to write {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => {
            Err(anyhow::Error::new(e).context(format!("Failed to create {}", path.display())))
        }
    }
}

/// Seed exactly one example feed epic ("Dependabot") wired to the installed
/// example script. Idempotent: re-running does not duplicate the epic.
///
/// Installs nothing itself — [`install_shipped_feed_scripts`] and
/// [`install_shipped_feed_configs`] own every file under `<data_dir>/scripts/`.
/// If `fetch-dependabot.sh` was the script that failed to write, seeding still
/// proceeds: the path is the right one to seed against either way, and a later
/// setup retries the write. Skipping would make the example epic depend on an
/// unrelated filesystem error that no later run notices.
pub async fn seed_feed_epics(db: &Database, data_dir: &Path) -> Result<()> {
    let script_path = installed_script_path(data_dir, "fetch-dependabot.sh");
    let cmd = script_path
        .to_str()
        .context("example script path is not valid UTF-8")?;

    let already_seeded = db
        .list_epics()
        .await?
        .iter()
        .any(|e| e.feed_command.as_deref() == Some(cmd));
    if already_seeded {
        return Ok(());
    }

    let epic = db.create_epic("Dependabot", "", None).await?;
    db.patch_epic(
        epic.id,
        &EpicPatch::new()
            .feed_command(Some(cmd))
            .feed_interval_secs(Some(300))
            .sort_order(Some(0)),
    )
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::setup::FakeConfirmer;
    use serde_json::Value;

    // -- seed_feed_epics --

    #[tokio::test]
    async fn seed_feed_epics_creates_single_example_epic() {
        let db = Database::open_in_memory().await.unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        seed_feed_epics(&db, data_dir.path()).await.unwrap();

        let epics = db.list_epics().await.unwrap();
        assert_eq!(
            epics.len(),
            1,
            "setup must seed exactly one example feed epic"
        );

        let epic = &epics[0];
        assert_eq!(epic.title, "Dependabot");
        assert_eq!(epic.sort_order, Some(0));
        assert_eq!(epic.feed_interval_secs, Some(300));

        let expected_path = data_dir.path().join("scripts").join("fetch-dependabot.sh");
        assert_eq!(
            epic.feed_command.as_deref(),
            Some(expected_path.to_str().unwrap())
        );
    }

    #[tokio::test]
    async fn seed_feed_epics_is_idempotent() {
        let db = Database::open_in_memory().await.unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        seed_feed_epics(&db, data_dir.path()).await.unwrap();
        seed_feed_epics(&db, data_dir.path()).await.unwrap();

        let epics = db.list_epics().await.unwrap();
        assert_eq!(epics.len(), 1, "Dependabot epic must not be duplicated");
    }

    /// feeds.allium: SeedExampleFeedEpic — "Seeding OWNS NO FILES." Getting
    /// the script onto disk belongs to InstallShippedFeedScripts.
    #[tokio::test]
    async fn seed_feed_epics_writes_no_files() {
        let db = Database::open_in_memory().await.unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        seed_feed_epics(&db, data_dir.path()).await.unwrap();

        assert!(
            !installed_script_path(data_dir.path(), "fetch-dependabot.sh").exists(),
            "seeding only wires an epic to a path the install rules guarantee; it must not \
             install the script itself"
        );
        assert!(
            !installed_script_path(data_dir.path(), "repos.conf").exists(),
            "seeding must not install config files either"
        );
    }

    /// feeds.allium: SeedExampleFeedEpic — "If fetch-dependabot.sh is the
    /// script that FAILED to write, seeding proceeds unchanged."
    #[tokio::test]
    async fn seed_feed_epics_seeds_even_when_the_dependabot_script_failed_to_write() {
        let db = Database::open_in_memory().await.unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let dir = data_dir.path().join("scripts");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();

        let reports = install_shipped_feed_scripts(data_dir.path(), None)
            .expect("a per-script write failure must not fail the run");
        let seeded = seed_feed_epics(&db, data_dir.path()).await;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(
            reports
                .iter()
                .find(|r| r.name == "fetch-dependabot.sh")
                .map(|r| r.outcome),
            Some(ShippedScriptOutcome::Failed),
            "the write was arranged to fail, so this test is only meaningful if it did"
        );
        seeded.expect("seeding must not depend on the outcome of a filesystem error");
        let epics = db.list_epics().await.unwrap();
        assert_eq!(
            epics.len(),
            1,
            "the epic is keyed on the path, not on the file's existence: once a run has \
             seeded nothing there is no later run that notices it should have"
        );
        assert_eq!(
            epics[0].feed_command.as_deref(),
            installed_script_path(data_dir.path(), "fetch-dependabot.sh").to_str(),
            "a later setup retries the write to that same path"
        );
    }

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
        let backup = scripts_dir(data_dir.path())
            .join(format!("fetch-log-warnings.sh{SCRIPT_BACKUP_SUFFIX}"));
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
        let backup =
            scripts_dir(data_dir.path()).join(format!("fetch-cve.sh{SCRIPT_BACKUP_SUFFIX}"));
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

    // -- Report section (ReportShippedFeedScripts) --

    fn report(name: &str, outcome: ShippedScriptOutcome) -> ShippedScriptReport {
        ShippedScriptReport {
            name: name.to_string(),
            path: PathBuf::from("/data/scripts").join(name),
            outcome,
            backup_path: None,
            error: None,
        }
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

    // -- Plugin metadata --

    #[test]
    fn plugin_json_is_valid() {
        let content = PLUGIN_DIR
            .get_file(".claude-plugin/plugin.json")
            .expect("plugin.json must be embedded")
            .contents_utf8()
            .expect("plugin.json must be UTF-8");
        let value: Value = serde_json::from_str(content).expect("plugin.json is invalid JSON");
        assert_eq!(value["name"], "dispatch");
    }

    #[test]
    fn plugin_embeds_required_files() {
        let required = [
            ".claude-plugin/plugin.json",
            "hooks/hooks.json",
            "hooks/scripts/task-status-hook",
            "hooks/scripts/pr-learnings-hook",
            "skills/wrap-up/SKILL.md",
            "skills/retro/SKILL.md",
            "skills/decompose-review/SKILL.md",
            "skills/decompose-review/references/plan-template.md",
            "skills/learnings/SKILL.md",
            "skills/summarize/SKILL.md",
            "skills/grill/SKILL.md",
            "skills/allium-loop/SKILL.md",
            "skills/allium-loop/prompt.md",
        ];
        for path in required {
            assert!(
                PLUGIN_DIR.get_file(path).is_some(),
                "{path} must be embedded in PLUGIN_DIR"
            );
        }
    }

    #[test]
    fn wrap_up_skill_uses_simplify_not_code_simplifier() {
        let content = skill_body("wrap-up");
        assert!(
            !content.contains("code-simplifier"),
            "wrap-up skill must not reference the old 'code-simplifier' skill"
        );
        assert!(
            content.contains("\"simplify\""),
            "wrap-up skill must reference the 'simplify' skill"
        );
    }

    /// The embedded copy of the wrap-up skill is what agents actually read, so
    /// it is the only thing that catches a regression here: epic chaining is a
    /// server-side effect of `exit_session` and there is no `dispatch_next` tool
    /// left for the skill to name.
    #[test]
    fn wrap_up_skill_does_not_instruct_calling_dispatch_next() {
        let content = skill_body("wrap-up");
        assert!(
            !content.contains("dispatch_next"),
            "wrap-up skill must not tell the agent to call dispatch_next — \
             exit_session chains the next epic subtask automatically"
        );
    }

    /// A regression in this guidance is silent: an agent that reads a non-error
    /// response as a completed close leaves a live tmux window and a task stuck
    /// in its old status, with no error anywhere to notice.
    #[test]
    fn wrap_up_skill_warns_a_successful_exit_session_can_report_a_failed_close() {
        assert!(
            failed_close_guidance().contains("success"),
            "failed-close guidance must say the response is a *successful* one, \
             not an error — that is the whole trap"
        );
    }

    /// The one reaction that must survive any rewording of the failed-close
    /// guidance: the exit token is consumed before the terminal write is
    /// attempted, so neither retrying `exit_session` nor taking a fresh token
    /// from `wrap_up` can work. Asserted positively — a negative "must not
    /// contain 'retry'" would also pass if the guidance were deleted outright,
    /// which is the regression that matters.
    #[test]
    fn wrap_up_skill_tells_the_agent_not_to_retry_a_failed_close() {
        let section = failed_close_guidance();
        assert!(
            section.contains("not retry") || section.contains("n't retry"),
            "failed-close guidance must tell the agent not to retry exit_session"
        );
        assert!(
            section.contains("wrap_up")
                && (section.contains("again") || section.contains("fresh token")),
            "failed-close guidance must tell the agent not to call wrap_up again \
             for a fresh token"
        );
    }

    /// Regression guard for the stale instruction that survived unnoticed until
    /// task #3769: the skill told the agent to record the PR URL via
    /// `update_task`, when the URL actually travels with `exit_session`. It
    /// drifted precisely because nothing pinned it. Scoped to lines that pair
    /// `update_task` with a URL, so an unrelated future use of the tool is not
    /// blocked.
    #[test]
    fn wrap_up_skill_does_not_record_the_pr_url_via_update_task() {
        for line in skill_body("wrap-up").lines() {
            let lower = line.to_lowercase();
            assert!(
                !(lower.contains("update_task") && lower.contains("url")),
                "wrap-up skill must not tell the agent to record the PR URL via \
                 update_task — the URL travels with exit_session: {line}"
            );
        }
    }

    /// The "Draft the title and body" step of the PR path, isolated from its
    /// neighbouring steps. Not built on `section_after`: that helper ends a
    /// section at the next line starting with `#`, but this step's own
    /// Markdown example contains fenced `## Summary` / `## Test plan` headings
    /// as literal example text, which would truncate the section before the
    /// part these tests need to inspect.
    fn pr_body_draft_section() -> String {
        let content = skill_body("wrap-up");
        let (_, after) = content
            .split_once("### Draft the title and body")
            .expect("wrap-up skill must have a 'Draft the title and body' step");
        let (section, _) = after.split_once("### Push and create the draft PR").expect(
            "wrap-up skill must have a 'Push and create the draft PR' step \
                 after drafting",
        );
        section.to_string()
    }

    /// Distilled from the shared PR-description knowledge base (learning #36):
    /// PR bodies must describe the user-visible change and why, not the
    /// implementation — so the template must never invite a function, class,
    /// file, or variable name.
    #[test]
    fn wrap_up_skill_pr_body_uses_categorized_summary_like_coderabbit() {
        let section = pr_body_draft_section();
        for label in ["**Breaking Changes**", "**New Features**", "**Bug Fixes**"] {
            assert!(
                section.contains(label),
                "PR body template must group changes under bold category labels \
                 (CodeRabbit-style), missing: {label}"
            );
        }
        assert!(
            section.to_lowercase().contains("coderabbit"),
            "PR body template should name CodeRabbit as the format it mirrors, \
             so a future editor knows why the categories look like this"
        );
    }

    /// Distilled from the shared PR-description knowledge base (learnings
    /// #146 and #154): dispatch task IDs must never appear in a PR body —
    /// GitHub auto-links `#N` to an unrelated issue/PR in the target repo.
    /// The old template's `Implements #{task_id}.` line was a live
    /// contradiction of both learnings until this test locked it out.
    #[test]
    fn wrap_up_skill_pr_body_omits_task_references() {
        let section = pr_body_draft_section();
        assert!(
            !section.contains("Implements #{task_id}"),
            "PR body template must not instruct the agent to write \
             \"Implements #{{task_id}}\" into the body — GitHub auto-links #N \
             to an unrelated issue/PR in this repo"
        );
        assert!(
            section
                .to_lowercase()
                .contains("do not reference the dispatch task"),
            "PR body template must explicitly warn against referencing the \
             dispatch task (task IDs, \"Implements #N\")"
        );
    }

    /// Distilled from the shared PR-description knowledge base (learning
    /// #188): the Test plan section is opt-in for dangerous/breaking PRs,
    /// not a default part of every PR body.
    #[test]
    fn wrap_up_skill_pr_body_test_plan_is_opt_in_not_default() {
        let section = pr_body_draft_section();
        assert!(
            !section.contains("- [ ] {how to verify"),
            "PR body template must not show an unconditional Test plan checklist \
             in its default example"
        );
        let lower = section.to_lowercase();
        assert!(
            lower.contains("test plan")
                && (lower.contains("dangerous") || lower.contains("breaking")),
            "PR body template must say the Test plan section is only for \
             dangerous or breaking PRs, omitted by default"
        );
    }

    /// The wrap-up skill's failed-close guidance block, lowercased: the heading
    /// section telling the agent that a *successful* `exit_session` response can
    /// still report that the close did not take effect.
    ///
    /// Scoped to that one section deliberately — "do not retry" also appears in
    /// the neighbouring `exit_session` *errors* section, so a whole-document
    /// check would still pass with this section's retry guidance deleted. The
    /// section is anchored on the phrase below and ends at the next heading of
    /// any depth (so promoting or demoting the heading cannot silently widen it
    /// to the rest of the file); if you reword the heading, re-anchor it here.
    fn failed_close_guidance() -> String {
        let content = skill_body("wrap-up").to_lowercase();
        section_after(&content, "did not take effect").expect(
            "wrap-up skill must document that a successful exit_session response \
             can still report the close did not take effect",
        )
    }

    /// The slice of `content` that follows `anchor`, ending at the next Markdown
    /// heading of any depth — so promoting or demoting a heading cannot silently
    /// widen a scoped assertion to the rest of the document. `None` if `anchor`
    /// is absent, letting each caller phrase its own "this copy is gone" panic.
    fn section_after(content: &str, anchor: &str) -> Option<String> {
        let (_, section) = content.split_once(anchor)?;
        Some(
            section
                .split_once("\n#")
                .map_or(section, |(block, _)| block)
                .to_string(),
        )
    }

    /// A loop iteration does rebase, tend, propagate, red check, implement,
    /// verify and weed, and repeats up to the loop's configured maximum — so an
    /// iteration agent left on the session model (Opus) multiplies that cost by
    /// the iteration count. Nothing in the loop's own output reveals which model
    /// ran, so dropping this instruction would regress silently.
    #[test]
    fn allium_loop_dispatches_iteration_agents_on_sonnet() {
        let section = allium_loop_dispatch_instruction();
        assert!(
            section.contains("sonnet"),
            "allium-loop's dispatch step must name the sonnet model for \
             iteration agents"
        );
    }

    /// The no-fork rule and the model override are load-bearing together: a
    /// `fork` ignores `model` entirely and runs on the session model, so losing
    /// the no-fork constraint would silently undo the sonnet pin even with the
    /// override still written down.
    #[test]
    fn allium_loop_dispatch_still_forbids_fork() {
        let section = allium_loop_dispatch_instruction();
        assert!(
            section.contains("fork"),
            "allium-loop's dispatch step must keep forbidding `fork` — a fork \
             ignores the model override and runs on the session model"
        );
    }

    /// The allium-loop skill's per-iteration dispatch instruction: the "Each
    /// Iteration" section.
    ///
    /// Scoped to that one section deliberately — the kickoff section also names
    /// the model (it resolves and records the loop parameter), so a
    /// whole-document check would still pass with the dispatch step's override
    /// deleted. Re-anchor here if the heading is reworded.
    fn allium_loop_dispatch_instruction() -> String {
        section_after(skill_body("allium-loop"), "### Each Iteration")
            .expect("allium-loop skill must have an 'Each Iteration' section")
    }

    /// Both halves of the loop's convergence gate, added after two iterations of
    /// one run reported `CONVERGED: yes` while their own prose named an
    /// unresolved item. The driver acts on the label, not the prose, so a
    /// self-contradicting report ends the loop with work still outstanding —
    /// and nothing in the loop's output reveals that, which is what makes
    /// losing this copy a silent regression rather than a visible one.
    #[test]
    fn allium_loop_convergence_gate_rejects_deferred_and_uncovered_work() {
        let section = section_after(
            skill_file("allium-loop", "prompt.md"),
            "Emit `CONVERGED: yes` ONLY when ALL hold:",
        )
        .expect("allium-loop prompt must state its CONVERGED criteria");
        assert!(
            section.contains("pending a decision"),
            "the convergence gate must keep disqualifying work left pending a \
             later run — a flagged-but-unresolved item is divergence"
        );
        assert!(
            section.contains("name the surviving test"),
            "the convergence gate must keep requiring a deleted test's \
             replacement to be named, not asserted in the abstract"
        );
    }

    /// Read an embedded skill's `SKILL.md` by skill name, for tests that assert
    /// on skill copy.
    fn skill_body(skill: &str) -> &'static str {
        skill_file(skill, "SKILL.md")
    }

    /// Read any embedded file from a skill directory. `SKILL.md` is the common
    /// case ([`skill_body`]); allium-loop also ships the `prompt.md` that its
    /// per-iteration agents actually run from, and that copy needs the same
    /// deletion-is-a-regression protection.
    fn skill_file(skill: &str, file: &str) -> &'static str {
        let path = format!("skills/{skill}/{file}");
        PLUGIN_DIR
            .get_file(&path)
            .unwrap_or_else(|| panic!("{path} must be embedded"))
            .contents_utf8()
            .unwrap_or_else(|| panic!("{path} must be UTF-8"))
    }

    /// A lowercased section of the retro skill body, via [`section_after`]:
    /// from the first occurrence of `anchor` up to the next Markdown heading of
    /// any depth. Anchors passed here must therefore be lowercase.
    ///
    /// Scoped per-section deliberately. Retro repeats words like "task",
    /// "spec" and "fix" across its steps, so a whole-document `contains` can
    /// still pass after the instruction under test has been deleted. If you
    /// reword an anchor heading, re-anchor it here.
    fn retro_section(anchor: &str) -> String {
        let content = skill_body("retro").to_lowercase();
        section_after(&content, anchor).unwrap_or_else(|| {
            panic!("retro skill must contain the section anchored on {anchor:?}")
        })
    }

    #[test]
    fn retro_admission_test_is_next_agent_benefit_not_doc_accuracy() {
        // The old Step 2 asked whether CLAUDE.md or a spec was "stale or
        // wrong" — a correctness question every trivial nit passes, which is
        // how retro came to file 38 one-line doc chores. The bar is now
        // whether the *next* agent would do better, and each finding must
        // trace to a concrete moment this session actually lost time on.
        let section = retro_section("## step 2:");
        assert!(
            section.contains("would the next agent do better"),
            "retro's admission test must be whether the next agent benefits, \
             not whether a sentence is inaccurate"
        );
        assert!(
            section.contains("concrete moment"),
            "retro must require every finding to trace to a concrete moment \
             from Step 1 rather than to a hypothetical"
        );
        assert!(
            !section.contains("stale or wrong"),
            "retro must not frame its check as a documentation-accuracy audit"
        );
    }

    #[test]
    fn retro_first_step_asks_whether_user_corrected_or_steered_you() {
        // Task #4326: the old Step 1 was entirely "context turned out wrong"
        // shaped (a bad CLAUDE.md assumption, a convention found by reading
        // source, a stale spec, a guessed rule, a failing command) — none of
        // it about the user. #4316's code-review feedback (test suffix,
        // one-class-per-file, ADTs over `require()`, and more) went
        // uncaptured until the human explicitly asked whether it had been
        // saved as learnings. Step 1 now asks specifically whether the user
        // corrected or steered the agent, and must still say "nothing
        // notable" is a real answer. (The Step-1-to-Step-2 linkage is
        // asserted separately, by the "concrete moment" check in
        // `retro_admission_test_is_next_agent_benefit_not_doc_accuracy`.)
        let section = retro_section("## step 1:");
        assert!(
            section.contains("correct") && section.contains("steer"),
            "retro's first step must ask whether the user corrected or \
             steered the agent this session"
        );
        assert!(
            section.contains("nothing notable"),
            "retro must state that an empty reflection is a real answer, so a \
             smooth session is not pressured into inventing findings"
        );
    }

    #[test]
    fn retro_first_step_scopes_out_self_discovered_gaps() {
        // A stale spec is `weed`'s job (spec-code alignment), and a command
        // that failed until the right invocation was found is a tooling
        // problem — neither is feedback from the user, so Step 1 must not
        // invite them back in as findings.
        let section = retro_section("## step 1:");
        assert!(
            section.contains("weed"),
            "retro's first step must route spec-code alignment to the weed \
             skill rather than treating a stale spec as its own finding"
        );
        assert!(
            section.contains("tooling problem"),
            "retro's first step must name a failing command as a tooling \
             problem, not a correction from the user"
        );
    }

    #[test]
    fn retro_first_step_names_code_review_and_design_examples() {
        // Step 1's checklist must give the agent concrete shapes of user
        // feedback to look for, and must say this counts even when it cost
        // no time in the moment — design/style feedback rarely does.
        let section = retro_section("## step 1:");
        assert!(
            section.contains("code review"),
            "retro's first step must prompt for feedback given during a \
             code review pass"
        );
        assert!(
            section.contains("design choice") || section.contains("design decision"),
            "retro's first step must ask about a design choice the user \
             made or corrected this session"
        );
        assert!(
            section.contains("even if it didn't") || section.contains("even if it did not"),
            "retro must state this category counts even when it didn't cost \
             time in the moment, since design/style feedback often doesn't \
             slow the agent down"
        );
    }

    #[test]
    fn retro_relationship_section_says_not_to_wait_to_be_asked_for_feedback() {
        // #4316's design/style feedback only became learnings after the
        // human asked "did you save learnings about the feedback I gave
        // you" — retro must not rely on being asked.
        let section = retro_section("## relationship to other skills");
        assert!(
            section.contains("don't wait") || section.contains("do not wait"),
            "retro's relationship-to-learnings note must say to record \
             design/style feedback without waiting to be asked"
        );
    }

    #[test]
    fn retro_skill_permits_fixing_small_context_drift_in_session() {
        // The old Step 3 said "Do not edit files yourself", which turned every
        // one-line doc correction into a task + worktree + agent dispatch. The
        // agent that just did the work has the context and is already in a
        // worktree whose next step is a commit; it should make the fix.
        let content = skill_body("retro").to_lowercase();
        assert!(
            !content.contains("do not edit files yourself"),
            "retro must no longer ban editing outright — fixing small context \
             drift in place is now its job"
        );
        let section = retro_section("## step 3:");
        assert!(
            section.contains("fix it yourself"),
            "retro must tell the agent to fix small context drift in this session"
        );
        assert!(
            section.contains("small and self-evident"),
            "retro's edit licence must be bounded to small, self-evident \
             corrections that need no design judgement"
        );
    }

    #[test]
    fn retro_skill_forbids_speccing_unimplemented_behaviour() {
        // A spec edit describing behaviour the session already implemented is
        // documentation catching up. One describing behaviour the code lacks is
        // a design change, and this repo runs those spec -> tests -> code with
        // their own dispatch — so retro must file it, not write it.
        let section = retro_section("## step 3:");
        assert!(
            section.contains("already implemented"),
            "retro may only edit a spec to describe behaviour this session \
             already implemented"
        );
        assert!(
            section.contains("spec → tests → code"),
            "retro must route a spec change for not-yet-implemented behaviour \
             to a task, naming the spec -> tests -> code loop as the reason"
        );
    }

    #[test]
    fn retro_skill_does_not_file_feature_tasks() {
        // Every archived retro-created task was a speculative refactor dressed
        // as an enhancement — "this invariant is enforced by convention, so the
        // same omission could recur", "this could be a single atomic insert".
        // feature leaves retro's vocabulary entirely.
        let section = retro_section("### what you may file");
        assert!(
            section.contains("never file a `feature`"),
            "retro must explicitly refuse to file feature tasks"
        );
        assert!(
            section.contains("speculative refactor"),
            "retro must name speculative refactors as a non-finding, since that \
             is the shape of every retro task that got archived"
        );
        // Scoped to the whole skill body, not just this section: the string
        // this guards against previously lived in Step 3's tag list, a
        // section this assertion does not otherwise cover. A future agent
        // restoring "`feature` for an enhancement idea" to Step 3 is the
        // likeliest regression, and no other section legitimately contains
        // this string, so widening the scope here is safe.
        let whole_skill = skill_body("retro").to_lowercase();
        assert!(
            !whole_skill.contains("`feature` for"),
            "retro must not still describe when to use the feature tag, in any section"
        );
    }

    #[test]
    fn retro_skill_requires_a_duplicate_check_before_filing() {
        // Two findings were each filed twice: one stale sentence that appeared
        // in two documents, and one recurring shape nobody recognised. Nothing
        // in the skill told the agent to look first.
        let section = retro_section("### before you file");
        assert!(
            section.contains("list_tasks"),
            "retro must check for an existing task with list_tasks before filing"
        );
        assert!(
            section.contains("one task per finding"),
            "retro must collapse a finding that spans several files into one task"
        );
    }

    #[test]
    fn retro_skill_states_zero_findings_is_the_normal_outcome() {
        // The old skill buried this under three steps of checklist-shaped
        // instructions, which read as a quota to fill rather than a bar to
        // clear.
        let section = retro_section("### before you file");
        assert!(
            section.contains("zero tasks is the normal outcome"),
            "retro must state outright that filing nothing is the expected result"
        );
    }

    #[test]
    fn retro_step_2_asks_whether_root_cause_is_outside_this_repo() {
        // Task #4256: retro found the Bash sandbox blocks Gradle daemon
        // startup, wrote a workaround note to user-scala's CLAUDE.md, and
        // stopped — the sandbox defect itself was never filed. Nothing in
        // Step 2 asked where the root cause actually lived, so this passed
        // retro's own rubric as closed. Step 2 must now ask that question for
        // every finding, in addition to the next-agent-benefit test.
        let section = retro_section("## step 2:");
        assert!(
            section.contains("root cause"),
            "retro's Step 2 must ask whether a finding's root cause lies \
             outside this repo"
        );
        assert!(
            section.contains("sandbox") && section.contains("claude code"),
            "retro's root-cause question must name the sandbox, dispatch \
             itself, and claude code as tool/environment loci, not just \
             gesture at 'somewhere else'"
        );
    }

    #[test]
    fn retro_root_cause_subsection_requires_filing_not_just_a_workaround() {
        // A local doc workaround treats the symptom, not the defect. Retro
        // must not let it read as full closure when the root cause is the
        // tool/environment running the agent rather than this repo — and
        // must route the filing itself based on who owns that root cause:
        // this task's own repo (file directly) or a different one (flag to
        // the user rather than filing unprompted onto a foreign board).
        let section = retro_section("### when the root cause is the tool or environment");
        let flattened = section.replace('\n', " ");
        assert!(
            section.contains("workaround"),
            "the new subsection must address the local workaround explicitly"
        );
        assert!(
            flattened.contains("is not") && flattened.contains("sufficient closure"),
            "the new subsection must state that a workaround alone is not \
             sufficient closure for a tool/environment root cause"
        );
        assert!(
            section.contains("create_task"),
            "retro must file a same-repo root-cause defect with create_task"
        );
        assert!(
            section.contains("not file") || section.contains("don't file silently"),
            "retro must not file a cross-repo root-cause defect silently"
        );
        assert!(
            section.contains("flag"),
            "retro must flag a cross-repo root-cause defect explicitly instead"
        );
    }

    #[test]
    fn retro_output_template_has_a_root_cause_line() {
        // The #4256 gap wasn't just a missing filing rule — retro's own
        // output never said anything, so nothing forced the question in
        // front of the user before the session closed.
        //
        // Not scoped via retro_section("## step 4:"): the output template is
        // a fenced code block containing its own "## Session Retrospective"
        // heading, which retro_section's "next heading of any depth" cutoff
        // would treat as the section boundary and truncate before this line.
        let content = skill_body("retro").to_lowercase();
        assert!(
            content.contains("root-cause issues flagged"),
            "retro's output template must include a line surfacing \
             root-cause issues flagged elsewhere"
        );
    }

    #[test]
    fn decompose_review_skill_defaults_wrap_up_mode_to_rebase() {
        // Review work packages are small and land on main — one draft PR per
        // package is noise. The skill pre-sets wrap_up_mode purely to skip
        // wrap-up's AskUserQuestion step, and 'rebase' is the right value.
        let content = skill_body("decompose-review");
        assert!(
            content.contains("`wrap_up_mode`: `\"rebase\"`"),
            "decompose-review skill must set wrap_up_mode to \"rebase\""
        );
        assert!(
            !content.contains("`wrap_up_mode`: `\"pr\"`"),
            "decompose-review skill must not set wrap_up_mode to \"pr\" — \
             a decomposed review epic would open one draft PR per work package"
        );
    }

    #[test]
    fn decompose_review_skill_does_not_pass_repo_path_to_create_epic() {
        // Epics carry no repo_path — create_epic rejects the field outright
        // ("unknown field `repo_path`"). Step 6's subtasks are where the repo
        // path belongs, so this assertion is scoped to the Step 5 section:
        // a whole-document check would trip over Step 6's legitimate uses.
        let content = skill_body("decompose-review");
        let step5 = content
            .split("## Step 5: Create Epic")
            .nth(1)
            .expect("decompose-review skill must have a 'Step 5: Create Epic' section")
            .split("## Step 6")
            .next()
            .expect("split always yields at least one element");
        assert!(
            !step5.contains("`repo_path`:"),
            "decompose-review must not tell the agent to pass repo_path to \
             create_epic — the tool rejects it (found in Step 5: {step5:?})"
        );
    }

    /// Every sub-skill wrap-up invokes is a place the agent can mistake the
    /// sub-skill's end for the session's end and go idle — retro and simplify
    /// both needed explicit guards for exactly that. Summarize sat in the
    /// closing sequence, between `wrap_up` and `exit_session`, which is the
    /// worst possible place to stall: the rebase has already fast-forwarded
    /// base_branch and the task is stuck in its old status. It bought a recap
    /// the user rarely needed, so it is gone rather than guarded (task #4505).
    #[test]
    fn wrap_up_skill_does_not_invoke_summarize() {
        let content = skill_body("wrap-up");
        assert!(
            !content.contains(r#"skill: "summarize""#),
            "wrap-up must not invoke the summarize skill — a sub-skill call \
             between wrap_up and exit_session is a stall point on the one path \
             where stalling has already touched base_branch"
        );
    }

    /// A preset `wrap_up_mode` is the user's answer, given earlier. Suppressing
    /// the question without carrying the wrap-up through leaves the agent idle
    /// with the question it was told not to ask unanswered, and nothing reaches
    /// base_branch until a human notices (task #4505, observed twice).
    #[test]
    fn wrap_up_skill_carries_a_preset_mode_through_without_asking_or_stopping() {
        let content = skill_body("wrap-up");
        let step_2 = content
            .split("## Step 2")
            .nth(1)
            .and_then(|s| s.split("## Step 3").next())
            .expect("wrap-up skill must have a Step 2 that reads the task");
        assert!(
            step_2.contains("Wrap-up mode:"),
            "Step 2 must read the preset mode off the line get_task actually \
             prints (`Wrap-up mode: <mode>`) — the response is prose, not JSON, \
             so an agent told to look for a `wrap_up_mode` field can find \
             nothing and fall through to the question it was meant to skip"
        );
        assert!(
            step_2.contains("Step 4"),
            "Step 2 must say what a preset mode does to the question step \
             (Step 4), or the agent has to infer it"
        );
        let lowered = step_2.to_lowercase();
        assert!(
            lowered.contains("do not stop") || lowered.contains("don't stop"),
            "a preset mode must come with an explicit instruction not to stop \
             part-way — suppressing the question alone is what left agents idle \
             at a blank prompt"
        );
    }

    #[test]
    fn summarize_skill_does_not_claim_unconditional_finality() {
        // summarize is usually run standalone (/summarize), but any skill may
        // invoke it mid-flow as a sub-step. An unconditional "this is always
        // the final step" claim reads as an instruction to stop the whole
        // session right there, stranding whatever the caller had left to do.
        // wrap-up used to be that caller and stalled exactly this way, which
        // is why its call is gone — see wrap_up_skill_does_not_invoke_summarize.
        let content = skill_body("summarize");
        assert!(
            !content.contains("always the final step"),
            "summarize skill must not unconditionally claim to be the final step, \
             since wrap-up invokes it as a mid-flow sub-step"
        );
    }

    #[test]
    fn retro_skill_tells_agent_to_resume_the_caller() {
        // wrap-up invokes retro pre-commit, before its commit step. Without an
        // explicit instruction to resume the caller's remaining steps, an
        // agent that just finished following retro's own steps has nothing
        // telling it to continue — that's how wrap-up gets stuck after retro
        // and never reaches the commit, the user's action choice, or
        // exit_session. Retro's own edits are among what that commit carries,
        // so stopping here loses them.
        let content = skill_body("retro");
        assert!(
            content.contains("do not stop here") || content.contains("Do not stop here"),
            "retro skill must explicitly instruct the agent to resume the \
             calling skill's next step instead of stopping"
        );
    }

    #[test]
    fn wrap_up_skill_runs_retro_between_the_action_choice_and_the_commit() {
        // Retro is bracketed on both sides, and each bound fixes a real defect:
        //
        //  • After the action choice, because retro decides fix-vs-file from the
        //    action. On `done` there is no rebase and no push, so a fix would be
        //    stranded — and retro cannot know that while the action is unsettled.
        //  • Before the commit, because that commit is what carries the fixes it
        //    does make. Invoked after wrap_up instead, they are stranded anyway:
        //    the rebase path has already fast-forwarded base_branch, so a later
        //    commit sits on a branch nobody merges, and the PR path has pushed.
        let content = skill_body("wrap-up");
        let choice_at = content
            .find("## Step 4: Ask the user to choose")
            .expect("wrap-up skill must have an action-choice step to anchor retro after");
        let retro_at = content
            .find("Skill({ skill: \"retro\" })")
            .expect("wrap-up skill must invoke the retro skill");
        let commit_at = content
            .find("## Step 6: Commit uncommitted changes")
            .expect("wrap-up skill must have a commit step to anchor retro before");
        assert!(
            choice_at < retro_at,
            "wrap-up must settle the action before invoking retro, so retro can \
             tell whether a fix it makes can reach the base branch at all"
        );
        assert!(
            retro_at < commit_at,
            "wrap-up must invoke retro before its commit step, so retro's \
             context fixes are committed with the session's work"
        );
        assert_eq!(
            content.matches("Skill({ skill: \"retro\" })").count(),
            1,
            "wrap-up must invoke retro exactly once — a leftover call in the \
             closing sequence would run it twice and re-file its findings"
        );
        assert!(
            !content.to_lowercase().contains("run `/retro`"),
            "wrap-up must not still invoke retro from the closing sequence \
             between wrap_up and exit_session"
        );
    }

    /// The "Do NOT record" list must name the internal-code-citation failure
    /// mode explicitly (task #4152 — learning #401 carried a stale
    // allow-phantom-symbol: the actual stale citation learning #401 carried
    /// `src/feed/cycle.rs::run_feed_cycle` citation that no gate ever caught).
    /// Scoped to that one section: the rest of the skill mentions plenty of
    /// backticked identifiers (tool names) that would make a whole-document
    /// check pass even with this specific rule missing.
    #[test]
    fn learnings_skill_forbids_code_citations() {
        let section = section_after(skill_body("learnings"), "### Do NOT record:")
            .expect("learnings skill must have a 'Do NOT record' section");
        assert!(
            section.contains("path.rs::symbol") || section.contains("path.rs"),
            "the Do NOT record list must name the path.rs::symbol citation shape: {section}"
        );
        assert!(
            section.to_lowercase().contains("rot"),
            "the rule must explain WHY (silent rot, no re-check) not just state a ban: {section}"
        );
    }

    /// The two skills that both tell an agent to rate a learning must not
    /// disagree about *when*. `learnings` says at the moment you act on the
    /// entry, explicitly "not deferred to wrap-up"; wrap-up's own copy used to
    /// permit "at the latest before you wrap up", which is that deferral. An
    /// agent reads whichever it loaded, so the looser wording wins whenever
    /// wrap-up is the one in context — and rating a batch at the end is the
    /// behaviour the immediate rule exists to prevent.
    #[test]
    fn wrap_up_and_learnings_agree_that_rating_is_not_deferred() {
        let learnings = skill_body("learnings");
        assert!(
            learnings.contains("not deferred to wrap-up"),
            "the learnings skill must keep the rate-immediately rule this test pins wrap-up to"
        );
        let wrap_up = skill_body("wrap-up");
        assert!(
            !wrap_up.contains("at the latest before you wrap up"),
            "wrap-up must not permit the deferral the learnings skill forbids"
        );
    }

    /// The validator only catches shapes prose never produces — a bare
    /// PascalCase or short snake_case name walks straight through it (task
    /// #4402: no regex separates `TuiRuntime` from `GitHub`). The skill copy
    /// is therefore the only thing keeping those out, so it has to state the
    /// rule in full rather than deferring to what `record_learning` rejects.
    #[test]
    fn learnings_skill_forbids_naming_implementation_detail_a_validator_misses() {
        let section = section_after(skill_body("learnings"), "### Do NOT record:")
            .expect("learnings skill must have a 'Do NOT record' section");
        for term in ["function", "type", "macro", "fixture", "file"] {
            assert!(
                section.contains(term),
                "the Do NOT record list must name '{term}' among the things an entry \
                 may not name — the validator does not catch all of them: {section}"
            );
        }
        assert!(
            section.contains("rejects only"),
            "the list must tell the agent the validator is narrower than the rule, \
             or an agent will read a successful call as approval: {section}"
        );
    }

    /// Ported from the `kognic-knowledge` plugin's capture triage (task
    /// #4402). Without it, "Do NOT record" is a list of categories an agent
    /// has to pattern-match against; with it there is one question that
    /// decides, and it routes a whole class of would-be entries to a lint
    /// instead of to prose.
    #[test]
    fn learnings_skill_carries_the_machine_check_triage() {
        let content = skill_body("learnings").to_lowercase();
        assert!(
            content.contains("failing check"),
            "the skill must ask whether a failing check could be written from the \
             source alone: {content}"
        );
        assert!(
            content.contains("lint"),
            "the triage's yes-branch must route to a lint rule rather than prose: {content}"
        );
        assert!(
            content.contains("smell"),
            "the triage's third branch — wrongly-shaped code is a smell, and the fix \
             is a refactor not a sentence — must survive: {content}"
        );
    }

    /// `record_learning` enforces only that a procedural entry HAS a detail.
    /// What the detail must contain — the case where the agent stops and asks
    /// a human — is convention, and this copy is where it lives (task #4402,
    /// ported from OKF's required `# Escalate` section).
    #[test]
    fn learnings_skill_requires_a_boundary_on_procedural_entries() {
        let content = skill_body("learnings").to_lowercase();
        assert!(
            content.contains("procedural") && content.contains("ask a human"),
            "the skill must say a procedural entry's detail names when to stop and \
             ask a human: {content}"
        );
    }

    /// The summary-writing guidance used to illustrate "name the specific
    /// thing" with an example naming a Rust type — the exact habit task #4402
    /// exists to break. An example teaches harder than a rule, so a stale one
    /// here would quietly undo the section above it.
    #[test]
    fn learnings_skill_summary_guidance_names_no_symbol() {
        let section = section_after(skill_body("learnings"), "### Writing a good summary")
            .expect("learnings skill must have a summary-writing section");
        assert!(
            !section.contains("TaskPatch"),
            "the summary example must not name a type — it models the banned habit: {section}"
        );
    }

    /// The scope table must not offer `project` — LearningScope has only
    /// user/repo/epic/task (task #4152: the row was stale, and an agent
    /// passing scope="project" gets a deserialization error).
    #[test]
    fn learnings_skill_scope_table_has_no_project_row() {
        let content = skill_body("learnings");
        assert!(
            !content.contains("| `project` |"),
            "the scope table must not offer a project scope row: {content}"
        );
    }

    /// The `wrong` verdict bullet must not claim a human-review step exists
    /// (learnings.allium: no human gate, no needs_review state, no status
    /// change on either verdict).
    #[test]
    fn learnings_skill_wrong_verdict_does_not_claim_human_review() {
        // The correct text says "there is no human review step", which itself
        // contains the substring "human review" — so this checks for the old
        // false CLAIM (routing an entry TO review) rather than the substring.
        let content = skill_body("learnings").to_lowercase();
        assert!(
            !content.contains("routes an approved entry"),
            "the learnings skill must not claim a wrong verdict routes an entry \
             for human review — learnings.allium: no human gate, no status \
             change on either verdict: {content}"
        );
        assert!(
            content.contains("no human review"),
            "the learnings skill should state plainly that there is no human \
             review step: {content}"
        );
    }

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

    /// Task #4193: the verify command must reach the agent, and be acted on,
    /// before `wrap_up` is ever called — not just via the response's
    /// after-the-fact "Verify before exiting" line, which for the rebase path
    /// arrives after the base branch has already been fast-forwarded.
    #[test]
    fn wrap_up_skill_runs_verification_before_calling_wrap_up() {
        let content = skill_body("wrap-up");
        let commit_at = content
            .find("## Step 6: Commit uncommitted changes")
            .expect("wrap-up skill must have a commit step to anchor verification after");
        let verify_at = content
            .find("## Step 7: Run verification")
            .expect("wrap-up skill must have a verification step between commit and closing");
        let closing_at = content
            .find("## Step 8: The closing sequence")
            .expect("wrap-up skill's closing sequence must be Step 8, after verification");
        assert!(
            commit_at < verify_at,
            "verification step must come after the commit step"
        );
        assert!(
            verify_at < closing_at,
            "verification step must come before the closing sequence (and so before wrap_up \
             is ever called)"
        );
        assert!(
            !content.contains("## Step 7: The closing sequence"),
            "the closing sequence must be renumbered to Step 8, not left as Step 7"
        );

        let verify_section = section_after(content, "## Step 7: Run verification")
            .expect("wrap-up skill must have a verification step to anchor the section on");
        assert!(
            verify_section.contains("Verify command") && verify_section.contains("Step 2"),
            "verification step must tell the agent to read the Verify command shown by \
             get_task in Step 2, got: {verify_section}"
        );
        assert!(
            verify_section.to_lowercase().contains("before")
                && verify_section.contains("`wrap_up`"),
            "verification step must instruct running verification before calling wrap_up, \
             got: {verify_section}"
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
}
