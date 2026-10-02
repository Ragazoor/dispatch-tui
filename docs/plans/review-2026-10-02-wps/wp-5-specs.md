# Missing Specs

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Cover the CLI, sync and spacetime store code with Allium specs.

## Context

Findings from `docs/plans/review-2026-10-02/report.md`, "Specs". Use `allium:distill`, then `allium:weed`. Ask the user where code and intent disagree rather than guessing.

## Findings

### ⚠️ No CLI spec (`src/cli/agent_tree.rs`, `statusline.rs`, `caller_headers.rs`, `agent_diff.rs`)

**Fix:** `docs/specs/cli.allium`: subcommands, exit codes, caller-identity headers.

### 💡 Sync is thinly specified (`src/sync/sdk_connector.rs`, `writes.rs`, `rows.rs`, `session.rs`)

**Fix:** Add reconnect, session and write-ordering rules to `sync.allium`.

### 💡 Spacetime store unspecified (`src/spacetime/managed_store.rs`, `snapshot.rs`, `restore.rs`, `dump.rs`, `cli_store.rs`)

**Fix:** `docs/specs/spacetime-snapshot.allium`: snapshot/restore/dump invariants, managed vs cli store selection.

## Changes

| File | Change |
|------|--------|
| `docs/specs/cli.allium` | New |
| `docs/specs/sync.allium` | Extend |
| `docs/specs/spacetime-snapshot.allium` | New |

## Verification

- [ ] `allium check` passes on each spec
- [ ] `./scripts/check-doc-paths.sh`, `check-doc-symbols.sh`, `check-doc-headings.sh` pass
- [ ] `cargo test`
