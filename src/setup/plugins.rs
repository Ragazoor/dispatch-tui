//! Plugin install: skills, slash commands, hooks (embedded at compile time),
//! plus the example feed script and feed-epic seeding.

use anyhow::{Context, Result};
use include_dir::{include_dir, Dir};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

// The entire plugin/ directory is embedded at compile time. Any file added to
// plugin/ is automatically picked up — no manual registration required.
pub(super) static PLUGIN_DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/plugin");

/// Directories under `plugin/` that are embedded by `include_dir!` but are
/// local development fixtures, never installed. `plugin/evals/` holds the
/// `claude plugin eval` cases (see "Agent-behaviour evals" in docs/testing.md).
const DEV_ONLY_DIRS: &[&str] = &["evals"];

pub(super) fn is_dev_only(path: &Path) -> bool {
    DEV_ONLY_DIRS.iter().any(|d| path.starts_with(d))
}

/// The built-in copy of `plugin/skills`, for serving skills as MCP resources.
/// Read from the embedded tree, never from the installed copy.
pub(crate) fn built_in_skills_dir() -> Option<&'static Dir<'static>> {
    PLUGIN_DIR.get_dir("skills")
}

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
    for subdir in dir.dirs().filter(|d| !is_dev_only(d.path())) {
        install_dir_recursive(subdir, base, changed)?;
    }
    Ok(())
}

fn embedded_path_set() -> std::collections::HashSet<PathBuf> {
    fn collect(dir: &Dir, paths: &mut std::collections::HashSet<PathBuf>) {
        for file in dir.files() {
            paths.insert(file.path().to_path_buf());
        }
        for subdir in dir.dirs().filter(|d| !is_dev_only(d.path())) {
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
        // `file_type` does not follow symlinks: a symlink is unlinked as a
        // stale entry, never walked into its target.
        if entry.file_type()?.is_dir() {
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
    for subdir in dir.dirs().filter(|d| !is_dev_only(d.path())) {
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
        if entry.file_type()?.is_dir() {
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
pub const SHIPPED_SCRIPT_CONFIGS: [ShippedFile; 5] = [
    shipped("repos.conf", include_str!("../../scripts/repos.conf")),
    shipped("bots.conf", include_str!("../../scripts/bots.conf")),
    shipped("org.conf", include_str!("../../scripts/org.conf")),
    shipped("teams.conf", include_str!("../../scripts/teams.conf")),
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

/// The operator's recorded refusals to overwrite an edited shipped script,
/// beside the manifest (feeds.allium: InstallShippedFeedScripts, DECLINED).
pub const SCRIPT_DECLINED_NAME: &str = ".declined-overwrites.json";

/// One recorded "no": the digest of the content the operator kept and of the
/// content dispatch shipped when they said it. While both still match, the
/// question has already been answered.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct DeclinedOverwrite {
    kept: String,
    shipped: String,
}

impl DeclinedOverwrite {
    fn covers(&self, on_disk: &str, shipped: &str) -> bool {
        self.kept == script_digest(on_disk) && self.shipped == script_digest(shipped)
    }
}

type DeclinedOverwrites = std::collections::BTreeMap<String, DeclinedOverwrite>;

/// Read the declined-overwrite record. Missing, unreadable or malformed reads
/// as EMPTY, on the manifest's terms: the cost is one more question, never a
/// write.
fn read_declined_overwrites(data_dir: &Path) -> DeclinedOverwrites {
    let path = installed_script_path(data_dir, SCRIPT_DECLINED_NAME);
    let Ok(raw) = fs::read_to_string(&path) else {
        return DeclinedOverwrites::new();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

/// Persist the declined-overwrite record; a failure is logged and swallowed,
/// as for the manifest. An empty record removes the file.
fn write_declined_overwrites(data_dir: &Path, declined: &DeclinedOverwrites) {
    let path = installed_script_path(data_dir, SCRIPT_DECLINED_NAME);
    if declined.is_empty() {
        if let Err(e) = fs::remove_file(&path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("failed to remove {}: {e}", path.display());
            }
        }
        return;
    }
    let encoded = match serde_json::to_string_pretty(declined) {
        Ok(encoded) => encoded,
        Err(e) => {
            tracing::warn!("failed to encode {}: {e}", path.display());
            return;
        }
    };
    if let Err(e) = crate::atomic_write::write_bytes(
        &path,
        crate::atomic_write::Mode::Fixed(0o644),
        encoded.as_bytes(),
    ) {
        tracing::warn!("failed to write {}: {e}", path.display());
    }
}

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
    crate::models::hex::encode(&hmac_sha256::Hash::hash(content.as_bytes()))
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
    if let Err(e) = crate::atomic_write::write_bytes(
        &path,
        crate::atomic_write::Mode::Fixed(0o644),
        encoded.as_bytes(),
    ) {
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
/// right its content is. A script the operator has declined to overwrite also
/// counts as current while the refusal still matches both the content on disk
/// and the content shipped (feeds.allium: ADeclinedOverwriteIsNotDrift): the
/// installer then writes nothing for it either. The record is only read here.
pub fn shipped_scripts_are_current(data_dir: &Path) -> bool {
    let declined = read_declined_overwrites(data_dir);
    SHIPPED_SCRIPTS.iter().all(|file| {
        let path = installed_script_path(data_dir, file.name);
        // `file_is_up_to_date` is setup's single definition of "already up to
        // date"; a second copy here could disagree with the writer and report
        // a file stale forever.
        (super::file_is_up_to_date(&path, file.content) && has_script_mode(&path))
            || still_declined(&declined, file, &path)
    }) && SHIPPED_SCRIPT_CONFIGS
        .iter()
        .all(|file| installed_script_path(data_dir, file.name).exists())
}

/// Whether the operator already declined to overwrite `path` with exactly
/// this shipped content, and the file still holds what they kept.
fn still_declined(declined: &DeclinedOverwrites, file: &ShippedFile, path: &Path) -> bool {
    declined.get(file.name).is_some_and(|record| {
        fs::read_to_string(path).is_ok_and(|on_disk| record.covers(&on_disk, file.content))
    })
}

/// Install and update every script in [`SHIPPED_SCRIPTS`] under `data_dir`.
///
/// `confirmer` is `None` when nobody can answer (a scripted launch). Its
/// absence is the input, so no path can read a yes out of silence.
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
///      hand-copied file) and "the user edited it". A refusal already
///      recorded for exactly this pair keeps it without asking. Otherwise,
///      with someone to ask: prompt through `confirm_dangerous`, which
///      defaults to NO; on yes copy to `<name>.bak` BEFORE writing, on no
///      record the refusal. With nobody to ask: keep it and record nothing —
///      a scripted launch in CI must be incapable of eating a local edit.
///
/// Branch 4 is the only branch that can destroy user content, and it needs
/// both someone to ask and an explicit yes.
///
/// A backup, write, chmod or prompt failure for ONE script yields `Failed`
/// for that script and does not fail the run — the remaining scripts, the config files
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
    let mut declined = read_declined_overwrites(data_dir);
    let mut reports = Vec::with_capacity(SHIPPED_SCRIPTS.len());

    for file in SHIPPED_SCRIPTS {
        let report =
            install_one_shipped_script(data_dir, file, confirmer, &mut manifest, &mut declined)?;
        // A script that now holds what dispatch wrote or ships needs no
        // refusal on record any more.
        if matches!(
            report.outcome,
            ShippedScriptOutcome::Installed
                | ShippedScriptOutcome::InSync
                | ShippedScriptOutcome::Updated
        ) {
            declined.remove(file.name);
        }
        reports.push(report);
    }

    write_script_manifest(data_dir, &manifest);
    write_declined_overwrites(data_dir, &declined);
    Ok(reports)
}

/// One script's pass through the four branches of
/// [`install_shipped_feed_scripts`].
fn install_one_shipped_script(
    data_dir: &Path,
    file: ShippedFile,
    confirmer: Option<&dyn super::Confirmer>,
    manifest: &mut std::collections::BTreeMap<String, String>,
    declined: &mut DeclinedOverwrites,
) -> Result<ShippedScriptReport> {
    let ShippedFile {
        name,
        content: shipped,
    } = file;
    let path = installed_script_path(data_dir, name);

    // One read answers all three cases. Only NotFound is an empty slot: a
    // file that exists but cannot be read (wrong permissions, not UTF-8) is
    // unknown provenance, and treating it as absent would overwrite it with
    // no prompt and no backup.
    let current = match fs::read_to_string(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(record_shipped_script(
                write_shipped_script(&path, shipped),
                file,
                path,
                ShippedScriptOutcome::Installed,
                manifest,
            ));
        }
        current => current,
    };
    if current.as_deref().is_ok_and(|c| c == shipped) {
        // The content is right, so there is nothing to write — but the
        // MODE may still be wrong. `write_shipped_script` writes before
        // it chmods, so a chmod that raised on an earlier run left the
        // right bytes behind a mode the feed runner cannot execute.
        // Matching on content alone would adopt that file as in_sync
        // and record a digest for it, and the failure would never be
        // reported again. Repair it here instead.
        return Ok(record_shipped_script(
            ensure_executable(&path),
            file,
            path,
            ShippedScriptOutcome::InSync,
            manifest,
        ));
    }

    // `recorded == digest(on_disk)` is false when there is no entry
    // AND when the file changed, so one test covers both meanings
    // of unknown provenance. An unreadable file has no digest at
    // all and lands here too.
    let current = current.ok();
    let recorded = manifest.get(name);
    let proven = current
        .as_deref()
        .is_some_and(|c| recorded.is_some_and(|rec| *rec == script_digest(c)));
    if !proven {
        return install_unknown_provenance_script(
            file, path, current, confirmer, manifest, declined,
        );
    }
    Ok(record_shipped_script(
        write_shipped_script(&path, shipped),
        file,
        path,
        ShippedScriptOutcome::Updated,
        manifest,
    ))
}

/// Report `outcome` and record the shipped digest when `step` succeeded; a
/// failed step reports `Failed` and records nothing, so dispatch never claims
/// provenance over a file it did not successfully write.
fn record_shipped_script(
    step: Result<()>,
    file: ShippedFile,
    path: PathBuf,
    outcome: ShippedScriptOutcome,
    manifest: &mut std::collections::BTreeMap<String, String>,
) -> ShippedScriptReport {
    match step {
        Ok(()) => {
            manifest.insert(file.name.to_string(), script_digest(file.content));
            ShippedScriptReport::new(file.name, path, outcome)
        }
        Err(e) => ShippedScriptReport::failed(file.name, path, e),
    }
}

/// Branch 4: the file differs and dispatch cannot prove it wrote it.
///
/// The backup is taken BEFORE the write, not after and not "if the write
/// succeeds", so a crash mid-write cannot leave the user with neither copy. A
/// backup that fails abandons the script before any write — the fallback on
/// "cannot back up" is to not destroy, never to destroy without a copy.
fn install_unknown_provenance_script(
    file: ShippedFile,
    path: PathBuf,
    on_disk: Option<String>,
    confirmer: Option<&dyn super::Confirmer>,
    manifest: &mut std::collections::BTreeMap<String, String>,
    declined: &mut DeclinedOverwrites,
) -> Result<ShippedScriptReport> {
    let ShippedFile {
        name,
        content: shipped,
    } = file;
    // The operator already said no to exactly this pair: keep it unasked.
    if let Some(on_disk) = &on_disk {
        if declined
            .get(name)
            .is_some_and(|record| record.covers(on_disk, shipped))
        {
            return Ok(ShippedScriptReport::new(
                name,
                path,
                ShippedScriptOutcome::Kept,
            ));
        }
    }
    // The ABSENCE of a confirmer is how "nobody can answer" is expressed, the
    // same way the startup check expresses it — so there is no flag a caller
    // could set inconsistently, and no path that reads a yes out of silence.
    let approved = match confirmer {
        None => false,
        Some(confirmer) => match confirmer.confirm_dangerous(&format!(
            "{} differs from the version dispatch ships. Overwrite it? \
             (the current file is saved to {})",
            path.display(),
            script_backup_path(&path).display()
        )) {
            Ok(answer) => answer,
            // An unanswerable prompt (closed stdin) is a failure of this
            // script only: the run goes on and the manifest is still written.
            Err(e) => return Ok(ShippedScriptReport::failed(name, path, e)),
        },
    };
    if !approved {
        // No digest recorded: the provenance is still unknown, and recording
        // one would license a silent overwrite next run. A no from someone
        // who was asked IS recorded, so the same question is not asked again
        // (an unreadable file has no digest to record it against).
        if let (Some(_), Some(on_disk)) = (confirmer, &on_disk) {
            declined.insert(
                name.to_string(),
                DeclinedOverwrite {
                    kept: script_digest(on_disk),
                    shipped: script_digest(shipped),
                },
            );
        }
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
    let written = write_shipped_script(&path, shipped);
    let mut report =
        record_shipped_script(written, file, path, ShippedScriptOutcome::Updated, manifest);
    // The backup was already taken, so it is on disk whether or not the
    // write landed — and a failure here is precisely when the user needs
    // to find it.
    report.backup_path = Some(backup);
    Ok(report)
}

/// Create each file in [`SHIPPED_SCRIPT_CONFIGS`] if and only if nothing
/// exists at its path.
///
/// The user-owned half. If something is already there, setup does not read it,
/// diff it, prompt about it, back it up or record it in the manifest. There is
/// no branch that writes over a `.conf` file — not with a
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
            "  → kept {path} (differs from the version dispatch ships and \
             dispatch did not write it; you are asked again after a further \
             edit or a new release)"
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

#[cfg(test)]
mod tests;
