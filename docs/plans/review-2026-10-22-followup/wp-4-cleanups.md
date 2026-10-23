# Review Cleanups (2026-10-22 follow-up)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix five small issues from the follow-up review: one atomic-write copy, a dead retry, a long ingest function and two doc lines.

## Context

This work package addresses findings from the code review in `docs/plans/2026-10-22-codebase-review-followup.md` (S1, S2, C1, §2.6-1, §2.6-3).

## Findings

### 🟢 Low: A third atomic-write copy with a fixed temp name (`src/agent_tree/open_set.rs:75`)

**Issue:** `write_atomic` writes `path.with_extension("json.tmp")` then renames. Two agent-tree panes saving at once share that temp file, so one rename can move the other's half-written file into place. `src/atomic_write.rs` already does this safely (5 callers).

**Fix:** Call `crate::atomic_write` from `open_set.rs` and delete the local helper. Leave `src/cli/statusline.rs::write_atomically` alone (unique name on purpose) unless it also folds in cleanly.

### 🟢 Low: The `store import` port retry can never fire (`src/cli/store_import.rs:107`, `:174`)

**Issue:** `spawn_on_free_port` retries on `ErrorKind::AddrInUse` from `spawner.spawn`, but `SpacetimeStartSpawner::spawn` (`src/spacetime/managed_store.rs:506`) only runs `Command::spawn`; the child binds later, so the error never comes. A lost port race waits the full start timeout and fails.

**Fix:** Either detect the bind failure after spawn (child exited and its log says address in use) in `wait_until_answering` and pick a new port there, or delete `PORT_ATTEMPTS`, the dead match arm and the doc sentence promising the retry. Prefer the real retry if it stays small; otherwise delete.

### 🟢 Low: `ingest_role_routed` is a 120-line, five-phase function (`spacetime/module/src/feed_ingest.rs:362`)

**Issue:** Ensure roles, index existing, route and move, upsert, delete stale and recalculate all sit in one body; the phase order the comments insist on is hard to see.

**Fix:** Extract the index and route phases as named helpers; keep the order visible in the caller. Pure refactor; rebuild `module.wasm` (`./scripts/build-managed-module.sh`). No `RULES_VERSION` change.

### 🟢 Low: CLAUDE.md unwrap sentence reads as if production unwraps exist (`CLAUDE.md:115`)

**Issue:** The tree has zero `unwrap()`/`expect()` outside `cfg(test)` code; the sentence invites an agent to audit ~314 test-module calls.

**Fix:** Say the tree has none outside `cfg(test)`, and clippy's `unwrap_used`/`expect_used` under `-D warnings` keeps it so.

### 🟢 Low: Nothing says which store tables are per-host (`docs/module-map.md`)

**Issue:** `RepoPath` reads like host-local config but is global; nothing lists which tables are per-host, which is how the prune bug (WP-1) went unnoticed.

**Fix:** Add one line (in `docs/module-map.md` near the store summary, or `docs/specs/sync.allium`) naming the per-host tables and saying every other table is shared across hosts.

## Changes

| File | Change |
|------|--------|
| `src/agent_tree/open_set.rs` | Use `crate::atomic_write`; delete `write_atomic` |
| `src/cli/store_import.rs` | Real retry or remove the dead one |
| `spacetime/module/src/feed_ingest.rs` | Extract helpers from `ingest_role_routed` |
| `spacetime/module/module.wasm` (or wherever the build writes it) | Rebuild |
| `CLAUDE.md` | Rewrite line 115 |
| `docs/module-map.md` | Per-host tables line |

## Verification

- [ ] Run the task's verify command — all pass
- [ ] `cargo test --no-fail-fast` green; module-tests crate green
- [ ] `./scripts/build-managed-module.sh --check` and the doc checkers pass
