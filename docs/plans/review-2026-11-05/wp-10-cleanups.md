# Cleanups

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Write the rules the code already enforces into the specs and docs, fix two small tmux-conf and hint-test gaps, and settle how the toolchain pin follows this machine.

## Context

Findings P7 and P5 from `docs/plans/2026-11-05-codebase-review.md` §3.5. Use `allium:tend` for the spec edits.

## Findings

### 🟢 Low — P7: rules the code enforces that no spec states

**Issue:**
- H2 (4bd2dce3): a failed resume kills its half-created window — `docs/specs/dispatch.allium: ResumeFailed` is silent.
- N8 (299ade2e): the scope probe is skipped when less than 30 s are left (`PREPARE_SCOPE_WORST_CASE`, not cached) and the sccache start is bounded by the time left — `dispatch.allium` still says only "past its deadline" and "bounded at 5 seconds".
- H5/H6 (4bd2dce3): split mode targets the board's own pane; teardown skips the window rename when `$TMUX_PANE` is absent — not in `split-pane.allium`/`startup.allium`.
- `src/process.rs::non_interactive_env`'s variables appear in no spec.
- `docs/reference.md` ("Verify Command") still says the command is stored per `repo_path` row; 8376c7e9 made it shared by all rows with the same GitHub path.
- `src/tmux.rs::contains_focus_events` requires exactly `set -g focus-events on`: `set -s focus-events on` or a trailing comment is not recognised, so a duplicate line is appended to `~/.tmux.conf`.
- a0e568fc's hint test covers only the four `*_action_hints` bars; the hardcoded mode bars in `src/tui/ui/status_bar.rs` are unchecked.

**Fix:** spec and doc text for the first five; match `set(-option)? -[gs]+ focus-events on` ignoring a trailing comment; extend the hint test to the mode bars.

### 🟢 Low — P5: the toolchain pin follows the distro (`rust-toolchain.toml`, `scripts/check-toolchain.sh`, `CLAUDE.md`, `scripts/verify.sh`)

**Issue:** 513f89df moved the pin to Fedora's 1.99.1. `verify.sh` runs `check-toolchain.sh`, so the next `dnf` rustc update turns every agent's verify red until someone bumps the pin. CLAUDE.md and `verify.sh` say verify "fails on a distro toolchain"; it fails only on a version mismatch.

**Fix:** see Decisions.

## Decisions

### P5: rustup or a distro-tracking pin

**Question:** how should this machine's toolchain and the repo pin stay in step?

**Options:** (a) install rustup here (CLAUDE.md's first-time setup) so the pin governs the toolchain — the intended setup, one-time cost, CI unchanged; (b) keep the distro toolchain and reword CLAUDE.md/`verify.sh` to say verify fails on a version mismatch, accepting a red verify after each `dnf` update until the pin is bumped; (c) exclude `dnf` upgrades of rust (`dnf versionlock`) — stable, but drifts from Fedora updates.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/dispatch.allium`, `split-pane.allium`, `startup.allium` | H2, N8, H5/H6, environment rules |
| `docs/reference.md` | Verify Command storage |
| `src/tmux.rs` | `contains_focus_events` |
| `src/tui/ui/status_bar.rs` tests | mode-bar hints |
| `CLAUDE.md`, `scripts/verify.sh` (per decision) | toolchain wording |

## Verification

- [ ] `./scripts/verify.sh` passes
- [ ] `allium check` on every spec touched
