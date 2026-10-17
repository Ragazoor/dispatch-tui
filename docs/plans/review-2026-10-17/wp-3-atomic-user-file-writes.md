# Atomic Writes for User Files

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Every write dispatch makes to a user-owned file is atomic, keeps the file mode, and does not reorder keys.

## Context

This work package addresses findings 3, 24, 37 and 41 of the 2026-10-17 code
review (`docs/plans/2026-10-17-codebase-review.md`). `src/host_file/mod.rs::write_whole`
already does temp file + `persist` with mode 0600 — reuse or generalise it.
Do not run `dispatch setup` or the TUI to verify; it rewrites real config
under `$HOME`. Use the tests' temp directories.

## Findings

### 🔴 `~/.claude.json` rewritten non-atomically and re-sorted (`src/setup/mod.rs:132`, `src/dispatch/trust.rs:36`)

**Issue:** `write_json_file` (used by `src/setup/config_update.rs:22`) and
`trust_at` do read-modify-`fs::write` on `~/.claude.json`, which Claude Code
rewrites constantly. `fs::write` truncates in place: a concurrent update is
lost, a crash leaves an empty file. `serde_json = "1"` (`Cargo.toml:29`) lacks
`preserve_order`, so the first rewrite re-sorts every key.

**Fix:** One shared helper: write to `NamedTempFile::new_in(parent)`, copy the
original's permissions, `persist`. Enable serde_json's `preserve_order`
feature (check nothing relies on sorted output — snapshot tests may). Keep the
read-to-write window short.

### 🟡 Snapshot dump truncates the target first (`src/main.rs:665`)

**Issue:** `write_snapshot` does `File::create(out)` then streams into it. A
failure mid-write destroys the previous backup. Default umask on a full board
dump.

**Fix:** Temp file in the same directory, `persist`, mode 0600.

### 🟢 Low findings

- `src/startup/store_pin.rs:49`, `src/startup/launch.rs:191` — `fs::write`
  for the pin and the record; a truncated pin aborts every launch with
  `StoreSwitched`, an unreadable one fails open. Use the helper.
- `src/host_file/mod.rs:34` — `HostIdentity` has no `#[serde(flatten)]`
  catch-all, so an older binary drops fields a newer one added. Add
  `extra: serde_json::Map<String, Value>`.

## Changes

| File | Change |
|------|--------|
| new helper (e.g. in `src/host_file/` or a small `src/atomic_write.rs`) | temp file + keep mode + persist |
| `src/setup/mod.rs` | `write_json_file` via the helper |
| `src/dispatch/trust.rs` | `trust_at` via the helper |
| `src/main.rs` | `write_snapshot` via the helper, 0600 |
| `src/startup/store_pin.rs`, `src/startup/launch.rs` | via the helper |
| `src/host_file/mod.rs` | flatten catch-all |
| `Cargo.toml` | `serde_json` `preserve_order` |

## Verification

- [ ] Tests first: a rewrite keeps the original key order and file mode; a failing serializer/writer leaves the original file intact
- [ ] Test: `HostIdentity` round-trips an unknown field
- [ ] `cargo test --no-fail-fast` and the task's verify command pass
