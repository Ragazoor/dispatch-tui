# Toolchain Guard For The Pre-Push Hook

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Stop the pre-push hook from failing with false clippy errors when the host cargo ignores the pinned toolchain.

## Context

This work package addresses findings from `docs/plans/2026-10-22-codebase-review.md`.

## Findings

### Medium: The pre-push gate cannot run on a distro-rust host (`.githooks/pre-push:8`)

**Issue:** `rust-toolchain.toml` pins 1.96.0. A Fedora `/usr/bin/cargo` is 1.99.0 with no rustup, so the pin is ignored. `cargo clippy --all-targets -- -D warnings` then gives 240 errors, 191 of them `clippy::double_must_use` on every `#[async_trait]` trait (first at `src/agent_tree/pane.rs:158`). An agent sees errors unrelated to its change and bypasses the hook or wastes a session. CI uses 1.96.0, so `main` stays gated.

**Fix:** Add a short `scripts/check-toolchain.sh` that compares `rustc --version` with `rust-toolchain.toml` and fails with "install rustup or use `cargo +1.96.0`". Call it first in the hook. If a script is added, cover it in `tests/ci_gates.rs`.

### Low: CLAUDE.md first-time setup hides the pin (`CLAUDE.md:39`)

**Issue:** It offers `dnf install rust-std-static-wasm32-unknown-unknown` as equal to `rustup target add`. A distro rustc ignores the pin.

**Fix:** Say which path is supported and point at the new check.

### Low: CLAUDE.md wording on unwraps and `spacetime` (`CLAUDE.md:115`, tmux sentence)

**Issue:** Line 115 reads as if production `unwrap()` exist. The tree has none outside `cfg(test)`. The full suite also needs `spacetime` on `PATH` for the live-store targets (`docs/testing.md:229`); only `tmux` is named.

**Fix:** Say "the tree has zero and the hook keeps it there". Add one clause for `spacetime` beside the tmux sentence.

## Changes

| File | Change |
|------|--------|
| `scripts/check-toolchain.sh` | New version check with a clear message |
| `.githooks/pre-push` | Run the check first |
| `tests/ci_gates.rs` | Cover the new script if the gate list requires it |
| `CLAUDE.md` | Toolchain note, unwrap wording, `spacetime` on `PATH` |

## Verification

- [ ] Run existing tests: `cargo test --no-fail-fast > scratch.txt 2>&1; echo $?`
- [ ] On this host the hook now stops with the clear message, not 240 errors
- [ ] Doc checkers pass
