//! The startup configuration check: resolve drift before the board draws.

use crate::process::{ProcessRunner, RealProcessRunner};
use crate::setup::{
    apply_config_update_in, inspect_config_drift_in, ConfigArtefact, ConfigContext, ConfigDrift,
    Confirmer, SetupPaths, StdinConfirmer,
};

/// How the startup configuration check ended.
/// `startup.allium`'s `StartupConfigOutcome`.
///
/// Every value is non-fatal: the board draws after all four
/// (`ConfigurationDriftNeverBlocksTheBoard`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupConfigOutcome {
    /// Nothing was out of date; nothing was said.
    AlreadyCurrent,
    /// The operator agreed and the artefacts were written.
    Updated,
    /// The operator was asked and said no.
    Declined,
    /// No one could be asked, so nothing was written.
    ReportedOnly,
}

/// The line shown to the operator when configuration is stale, whether they
/// are about to be asked or merely told.
///
/// Names every stale artefact: `ConfigUpdatePrompt`'s `NamesWhatWillChange`
/// forbids asking for approval of an unnamed set of writes.
pub(super) fn describe_drift(drift: &ConfigDrift) -> String {
    format!(
        "Dispatch configuration is out of date: {}.",
        list_artefacts(&drift.items)
    )
}

/// The artefact labels as the operator reads them. One formatting of the list,
/// used by the drift line and by the partial-failure warning, so the two cannot
/// punctuate the same set differently.
fn list_artefacts(items: &[ConfigArtefact]) -> String {
    items
        .iter()
        .map(|a| a.label())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Resolve configuration drift on the way to the board, against the real
/// `$HOME`-derived locations.
///
/// Synchronous and blocking: it reads a dozen files, walks the installed plugin
/// tree, spawns a tmux subprocess, and may block on stdin waiting for an
/// answer. Callers on an async runtime must run it on a blocking thread.
pub fn resolve_startup_config(
    paths: &SetupPaths,
    data_dir: &std::path::Path,
    port: u16,
    interactive: bool,
) -> StartupConfigOutcome {
    let confirmer = StdinConfirmer;
    resolve_startup_config_in(
        paths,
        data_dir,
        port,
        &RealProcessRunner::default(),
        interactive.then_some(&confirmer as &dyn Confirmer),
    )
}

/// Injectable core of [`resolve_startup_config`]. The three-way branch is
/// `startup.allium`'s `CheckStartupConfigWhenNothingIsStale` /
/// `PromptToUpdateStaleConfig` / `ReportStaleConfigWhenNoOneCanAnswer`.
///
/// Returns an outcome rather than a `Result` on purpose. Every internal failure
/// is folded into the report it already prints, so there is no `Err` for a
/// caller to propagate and therefore no way to make configuration drift fatal —
/// `ConfigurationDriftNeverBlocksTheBoard` holds by type rather than by every
/// call site remembering to swallow the error.
///
/// `confirmer` is `None` when nobody can answer — a script, a CI job, stdin
/// redirected from nowhere. The absence is the input, not a flag beside one:
/// there is then no code path that could read a queued "yes" out of silence.
pub(crate) fn resolve_startup_config_in(
    paths: &SetupPaths,
    data_dir: &std::path::Path,
    port: u16,
    runner: &dyn ProcessRunner,
    confirmer: Option<&dyn Confirmer>,
) -> StartupConfigOutcome {
    let ctx = ConfigContext {
        paths,
        port,
        runner,
        data_dir,
        confirmer,
    };
    let drift = inspect_config_drift_in(&ctx);

    if drift.is_clean() {
        // Silent on purpose. A line reporting four current artefacts on every
        // launch trains the operator to skip the region of the screen where the
        // one line that matters appears.
        return StartupConfigOutcome::AlreadyCurrent;
    }

    let Some(confirmer) = confirmer else {
        // Treating silence as consent is how a scripted run rewrites the home
        // directory of whoever happened to start it.
        eprintln!(
            "{} Nothing was written — run `dispatch tui` from a terminal to update it.",
            describe_drift(&drift)
        );
        return StartupConfigOutcome::ReportedOnly;
    };

    // One prompt for the whole report: the artefacts are named in it, but they
    // are not separable decisions — a half-updated installation is a
    // configuration nobody chose and nothing tests.
    eprintln!("{}", describe_drift(&drift));
    match confirmer.confirm("Update it now?") {
        Ok(true) => {}
        Ok(false) => {
            eprintln!(
                "Configuration left as it is. The board will start; \
                 you will be asked again next launch."
            );
            return StartupConfigOutcome::Declined;
        }
        Err(e) => {
            // An unreadable stdin is an operator who could not answer, not one
            // who said yes.
            eprintln!("Warning: could not read an answer ({e:#}). Nothing was written.");
            return StartupConfigOutcome::ReportedOnly;
        }
    }

    let failed = apply_config_update_in(&drift, &ctx);
    if !failed.is_empty() {
        eprintln!(
            "Warning: could not update {}. The board will start; \
             this is retried next launch.",
            list_artefacts(&failed)
        );
    }
    StartupConfigOutcome::Updated
}
