//! The host-label gate: name this machine before the board draws.

use super::launch::StartupAbort;
use crate::setup::{Confirmer, StdinConfirmer};

/// The host-label gate: `startup.allium`'s `CheckHostLabel` and its
/// label-side rules (`ContinueWhenTheHostIsAlreadyNamed`,
/// `PromptForHostLabelWhenUnnamed`, `NameHostFromStartupPrompt`,
/// `AbortWhenTheHostIsUnnamedAndNoOneCanAnswer`). `CheckHostLabel`'s fourth
/// arm (`AbortWhenTheHostIdentityStoreIsUnusable`) fires earlier — in
/// `TuiRuntime::bootstrap`, before this function is ever reached — so it has
/// no counterpart here. The broken-settings-store condition that rule names
/// has a second face (a persist that does not take), but that face is not
/// this rule either: it fires *after* this function returns `Ok(Some(label))`,
/// raised directly by `startup.allium`'s `NameHostFromStartupPrompt` in the
/// caller's persist step — see `persist_host_label` in `src/runtime/mod.rs`.
///
/// Pure given its inputs: reading the current label from the database and
/// persisting a newly accepted one via `host.allium: RenameHost` are the
/// caller's job — this function only decides, so it can be tested without a
/// database. `current_label` is the host's label as read from settings right
/// now; `None` means unnamed. Returns:
///
/// - `Ok(None)` — the host is already named; nothing to persist
///   (`ContinueWhenTheHostIsAlreadyNamed`).
/// - `Ok(Some(label))` — the operator answered the prompt; the caller must
///   persist `label` via `RenameHost` before drawing
///   (`PromptForHostLabelWhenUnnamed` / `NameHostFromStartupPrompt`), and
///   abort through `AbortWhenTheHostIdentityStoreIsUnusable`'s reason if that
///   write fails rather than treat `Ok` as "resolved".
/// - `Err(StartupAbort::HostUnnamed)` — nobody could be asked, or the one
///   asked could not answer (`AbortWhenTheHostIsUnnamedAndNoOneCanAnswer`).
///
/// `confirmer` is `None` on exactly the same non-interactive condition
/// `resolve_startup_config_in` uses for `operator_can_answer` — a script, a
/// CI job, stdin redirected from nowhere. Unlike that function, an absent or
/// failing confirmer here is fatal
/// (`TheBoardNeverDrawsForAnUnnamedHost`): there is no `reported_only`
/// counterpart for a host with no name.
///
/// The prompt is retried on a blank answer rather than giving up
/// (`HostLabelPrompt`'s `ThereIsNoWayPast`): in practice this never loops
/// against the real `StdinConfirmer`, whose `prompt_text` already substitutes
/// the (always non-empty) hostname default for empty input, but a confirmer
/// that returns blank text directly must still be re-asked rather than
/// treated as a way past the gate.
pub(crate) fn resolve_host_label(
    current_label: Option<&str>,
    hostname: &str,
    confirmer: Option<&dyn Confirmer>,
) -> Result<Option<String>, StartupAbort> {
    if current_label.is_some() {
        return Ok(None);
    }

    let Some(confirmer) = confirmer else {
        return Err(StartupAbort::HostUnnamed);
    };

    eprintln!(
        "This machine has not been named yet. The name is shown on shared boards \
         so teammates can tell whose worktree a task belongs to."
    );

    loop {
        let answer = confirmer
            .prompt_text("Name for this machine", hostname)
            .map_err(|_| StartupAbort::HostUnnamed)?;
        let trimmed = answer.trim();
        if !trimmed.is_empty() {
            return Ok(Some(trimmed.to_string()));
        }
        eprintln!("A name is required — this machine cannot stay unnamed.");
    }
}

/// Real, blocking entry point for the host-label gate: constructs the
/// stdin-backed prompter and this machine's hostname, then delegates to
/// [`resolve_host_label`]. Mirrors [`resolve_startup_config`]'s split from
/// [`resolve_startup_config_in`] — callers on an async runtime must run this
/// on a blocking thread, since it may block on stdin waiting for an answer.
pub fn resolve_host_label_interactively(
    current_label: Option<String>,
    interactive: bool,
) -> Result<Option<String>, StartupAbort> {
    let confirmer = StdinConfirmer;
    resolve_host_label(
        current_label.as_deref(),
        &machine_hostname(),
        interactive.then_some(&confirmer as &dyn Confirmer),
    )
}

/// Best-effort machine hostname, used only as the host-label prompt's
/// pre-filled default (never the Host's id — see host.allium:
/// MintHostIdentity's guidance on why the id is generated, not derived).
/// Reads `/proc/sys/kernel/hostname` directly rather than shelling out to
/// `hostname(1)`: cheaper, and every target this binary ships for is Linux
/// (see CLAUDE.md: "POSIX-only"). Falls back to the `HOSTNAME` environment
/// variable, then to a fixed placeholder — this must never fail startup, and
/// must never be empty, since an empty default would make the prompt's
/// accept-with-enter path indistinguishable from a blank answer.
pub(crate) fn machine_hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "unknown-host".to_string())
}
