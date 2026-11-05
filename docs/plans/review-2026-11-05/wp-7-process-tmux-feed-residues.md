# Process, tmux and feed script residues

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Close the leftovers of last round's process, tmux and feed fixes: the last unbounded tmux and spacetime calls, a production port race, the askpass override, untagged feed warnings and keep-awake detection.

## Context

Findings P1, P2, P3, P4, P6 and P8 from `docs/plans/2026-11-05-codebase-review.md` §3.5. Each is a residue of a 2026-11-04 fix (H3, T2, N4, M13, N1, keep-awake T2). Never drive tmux without `-L <socket>`. Behaviour that changes a spec starts in the spec (`docs/specs/startup.allium`, `feeds.allium`, `keep-awake.allium`).

## Findings

### 🟡 Medium — P2: `store_import` picks a port by bind-and-drop (`src/cli/store_import.rs::spawn_on_free_port`, `wait_until_answering`) — code path verified, race outcome plausible

**Issue:** `wait_until_answering` accepts any store that answers on the port, so if another store takes the port first, the import dumps that store's data instead of the copy. The doc says a lost race only shows as "never answering". Test-only siblings: `src/spacetime/managed_store.rs::a_port_nothing_listens_on_probes_as_nothing_listening`, `store_pin_tests.rs::nothing_listening_reads_as_none`.

**Fix:** confirm the listener is the process just started (its data directory, as `managed_store_real.rs` now does, or port 0 and read the bound port); fix the test siblings like 513f89df did.

### 🟢 Low–Medium — P1: `tmux::pane_pid` is unbounded (`src/tmux.rs::pane_pid`; `src/startup/retire.rs`)

**Issue:** `runner.run` with no timeout, on the relaunch path; a wedged tmux server hangs the restart. `run_checked_timeout`'s doc says every tmux call goes through it.

**Fix:** use the bounded runner. Consider a test that greps `src/tmux.rs` for a bare `runner.run(` (graduation of H3's class).

### 🟢 Low — P3: fix for N4 → the user's `SSH_ASKPASS_REQUIRE` is overridden (`src/process.rs::non_interactive_env`)

**Issue:** it sets `never` always; the same function keeps an existing `GIT_SSH_COMMAND` for exactly this reason ("the user's ssh config is untouched"). A user with `force` and a keyring askpass can no longer unlock keys in bounded git/feed children.

**Fix:** set it only when unset. Test both cases.

### 🟢 Low — P4: fix for M13 → an unbounded `spacetime call` on every warm start (`SpacetimeCliPublisher::claim_store_owner`) — plausible

**Issue:** `self.runner.run` with no timeout, now on every bring-up; a store that accepts connections but stalls on reducers hangs startup.

**Fix:** `run_with_timeout`; a timeout is a `warn`, as a failed claim is today.

### 🟢 Low — P6: N1's siblings taint every feed cycle (`scripts/fetch-reviews.sh`, `scripts/fetch-dependabot.sh`)

**Issue:** the bots.conf format and duplicate-entry warnings go to stderr without the `notice:` prefix, so a misconfigured file makes every cycle additive and stale cards are never removed. The `fetch-cve` "skipping repo" warnings are correctly taints.

**Fix:** prefix permanent configuration warnings with `notice:`; a `tests/feed_scripts.rs` case per script under a bad config, run through the emission classifier.

### 🟢 Low — P8: keep-awake detection reads only the tmux server's environment (`src/keep_awake.rs::hold_command_for_this_machine`)

**Issue:** a7c40537 reads `XDG_CURRENT_DESKTOP`, `XDG_SESSION_DESKTOP` and `DESKTOP_SESSION`, all from the tmux server's environment, so a server started over ssh still has none; the `OnceLock` keeps the wrong first answer.

**Fix:** try `gnome-session-inhibit` first when it is on `PATH`, or read `tmux show-environment -g`; do not cache a fallback answer. Update `keep-awake.allium` if the choice rule changes.

## Changes

| File | Change |
|------|--------|
| `src/cli/store_import.rs` | confirm the listener; tests |
| `src/spacetime/managed_store.rs`, `store_pin_tests.rs` | no bind-and-drop in tests |
| `src/tmux.rs` | bound `pane_pid` |
| `src/process.rs` | keep a user's `SSH_ASKPASS_REQUIRE` |
| `src/spacetime/` (publisher) | bounded `claim_store_owner` |
| `scripts/fetch-reviews.sh`, `scripts/fetch-dependabot.sh`, `tests/feed_scripts.rs` | `notice:` prefix and tests |
| `src/keep_awake.rs`, `docs/specs/keep-awake.allium` | detection |

## Verification

- [ ] `./scripts/verify.sh` passes
