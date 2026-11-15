# Toolchain: stop the distro rustc from turning verify red

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** a `dnf` update to rustc no longer turns every agent's verify command red until someone bumps `rust-toolchain.toml`.

## Context

Standing item P5, raised in the 2026-11-05 and 2026-11-15 reviews (`docs/plans/2026-11-15-codebase-review.md` §5). The user chose on 2026-11-15 to give it its own task. This machine runs Fedora's `/usr/bin/rustc` and `/usr/bin/cargo` (1.99.1) with no rustup; `rust-toolchain.toml` pins `1.99.1` and `scripts/check-toolchain.sh` (run by `verify.sh` and the pre-push hook) fails on a mismatch. e9ffea03 reworded the docs only.

## Findings

### 🟢 Low — P5: the toolchain pin follows the distro (`rust-toolchain.toml`; `scripts/check-toolchain.sh`; CLAUDE.md "First-time setup")

**Issue:** CLAUDE.md says "use rustup", but the operator's machine does not; the next `dnf upgrade` of rustc makes `check-toolchain.sh` fail in every worktree at once.

**Fix:** see Decisions.

## Decisions

### P5: how should this machine get its toolchain?

**Question:** install rustup (so the pin governs the toolchain), or keep the distro rustc and make the mismatch cheap to resolve?

**Options:** (a) install rustup on this machine (`sudo dnf install rustup`, `rustup-init`, `rustup target add wasm32-unknown-unknown`) and remove the distro `rust`/`cargo` packages or put `~/.cargo/bin` first on `PATH` — the documented setup, one-time effort, touches the user's machine so the user runs or approves each step; (b) keep the distro rustc and exclude `rust`/`cargo` from automatic `dnf` upgrades (`excludepkgs` in `dnf.conf`) — no new tool, the user bumps both together by hand; (c) keep both as they are and make `check-toolchain.sh`'s failure message say exactly how to bump the pin — no machine change, the red verify still happens once per rustc update.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. Machine changes outside the worktree need the user's explicit approval for each command. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| (machine) | per decision |
| `scripts/check-toolchain.sh` | clearer failure message (option c) |
| `CLAUDE.md`, `docs/testing.md` | match what was chosen |

## Verification

- [ ] `rustc --version` equals the pin; `./scripts/verify.sh` passes
