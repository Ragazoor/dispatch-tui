# Managed Store and Snapshot Tooling

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** The board only stops a store it started, and restore/import never silently lose or overwrite data.

## Context

This work package addresses findings 20 and 57–60 of the 2026-10-17 code
review (`docs/plans/2026-10-17-codebase-review.md`). Specs:
`docs/specs/startup.allium`, `docs/specs/spacetime-seed.allium`,
`docs/specs/spacetime-snapshot.allium`. Do not run `cargo run -- tui`
against `127.0.0.1:3000`; use a throwaway port.

## Findings

### 🟡 Failed managed-store start kills another board's store (`src/spacetime/managed_store.rs:851`, `:741`)

**Issue:** On a failed start, `stop_once` calls `stop_managed_store`, which
SIGTERMs every pid listening on the port. Two boards starting together: B's
start fails with "address in use" and B kills A's healthy store. In the poll
loop (`:674`) a `StoreAnswering` from another board's store can also make B
believe its own dead child succeeded.

**Fix:** On a failed start, signal only the tracked child; use the port scan
only for adopted stores. In the poll loop, require the tracked child to be
alive before accepting `StoreAnswering`.

### 🟢 Low findings

- `src/spacetime/restore.rs:5` — header cites removed `SeedSharedStore` /
  `seed.rs`; `restore()` no longer refuses a non-empty store. Fix the comment
  and decide (spec) whether restore should refuse.
- `src/spacetime/import.rs:221` (`differs_in_creation`) — id-conflict check
  covers tasks, epics, learnings, work packages only; agents, repo rows,
  usage events and retired feed items with a clashing id are silently kept
  as "already there". Compare a content key or refuse.
- `src/spacetime/managed_store.rs:829` (`stop_once`) — returns `true` on
  every later call even if the first stop timed out. Store and return the
  first result.
- `src/spacetime/managed_store.rs:281` — any HTTP 200 on `/v1/ping` is
  adopted as our store and later stopped. Confirm identity before adopting,
  or only stop what this board spawned (partly a documented edge — check
  `startup.allium` first).

## Changes

| File | Change |
|------|--------|
| `src/spacetime/managed_store.rs` | Child-only stop on failed start; alive check; `stop_once` result; adopt check |
| `src/spacetime/restore.rs` | Comment; optional non-empty refusal |
| `src/spacetime/import.rs` | Conflict check for the remaining tables |
| `docs/specs/startup.allium`, `spacetime-seed.allium` | Any behaviour change |

## Verification

- [ ] Tests first: failed start with a foreign listener does not signal it (inject the pid lister); `stop_once` reports a timeout twice
- [ ] Import test with a clashing agent id
- [ ] `cargo test --no-fail-fast` and the task's verify command pass
