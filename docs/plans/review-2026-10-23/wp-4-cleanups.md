# Review Cleanups (2026-10-23)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix six small issues: learning verdicts missing from snapshots, unowned repo paths, editor section parsing, repo-scoped learning matching, the embedding thread's panic and the CLI's store-pin check.

## Context

This work package addresses findings from the code review in `docs/plans/2026-10-23-codebase-review.md` (W1, W2, F1, F2, F3, F4). Two are decision items (W2, F4); ask before implementing them. Any module or rule change needs its twin in `src/sync/memory_caller/`, a conformance case, and `RULES_VERSION` / `BOARD_RULES_VERSION` moved together.

## Findings

### 🟢 Low: `learning_verdicts` is not in the store snapshot (`src/spacetime/tests/module_schema.rs`, `spacetime/module/src/tables.rs` `LearningVerdict`)

**Issue:** `SharedTable` leaves `learning_verdicts` out, so `store import` and restores drop it while keeping learnings and their scores. After a restore, a task that already rated a learning rates it again and the score moves twice. The `LearningVerdict` doc comment reads as if that were the reason to leave it out.

**Fix:** Add the table to the snapshot (and its seed path), or reword the comment and list the gap in `docs/specs/learnings.allium`. Prefer adding it.

### 🟢 Low: Repo paths saved before the `host` column are never pruned (`src/sync/rows.rs::repo_paths_saved_by`)

**Issue:** It skips `host == ""`; `save_repo_path` adopts a row only when it is saved again. A path deleted from disk before the upgrade stays in the repo picker forever (the board's manual delete still works). The spec states this choice. **Decision item** — see below.

**Fix:** Per the decision.

### 🟢 Low: Description lines shaped like section headers eat the rest of the description (`src/editor.rs::parse_sections`)

**Issue:** Any `--- X ---` line starts a new section. A description containing `--- Notes ---` loses every line after it; one containing `--- STATUS ---` overrides the real status. `.trim()` also strips indentation from a section's first line on every round trip.

**Fix:** Recognise only the known section names; trim only the surrounding newlines. Add tests for both scenarios.

### 🟢 Low: Some repo-scoped learnings never match their repo (`src/embeddings.rs::repo_ref_covers`, `src/mcp/handlers/learnings.rs::resolve_scope_ref`)

**Issue:** An explicit `scope_ref` is stored as given and compared untrimmed. A learning recorded with `/a/repo/` or `~/a/repo` is never injected for tasks in `/a/repo`.

**Fix:** Expand `~` and trim a trailing `/` in `resolve_scope_ref` when recording; make `repo_ref_covers` tolerant of a trailing `/` for rows already stored. Tests for both spellings.

### 🟢 Low: One panic disables learning injection until restart (`src/embeddings.rs::EmbeddingService::new`)

**Issue:** One model thread serves every request. A panic in fastembed, or `vecs.remove(0)` on an empty result, kills it; every later `embed` fails with "channel closed", logged only as a warning, and prompts silently get no learnings.

**Fix:** Replace `vecs.remove(0)` with a checked take that returns an error, and wrap each message's work in `std::panic::catch_unwind` so one bad input answers with an error and the thread lives on.

### 🟢 Low: One-shot CLI commands skip the store-identity pin (`src/store_connection/mod.rs::open_cli_store`)

**Issue:** Only the board runs `check_store_identity`. A stale `DISPATCH_SPACETIME_SERVER` in the shell (the cause of task #28710) makes `dispatch repo set-verify`, `plan` or `prune-repo-paths` write into the wrong store. The specs (`docs/specs/cli.allium`, `docs/specs/startup.allium`) are silent. **Decision item** — see below.

**Fix:** Per the decision. Spec first.

## Decisions

### Unowned repo paths (`src/sync/rows.rs::repo_paths_saved_by`)

**Question:** Should `prune-repo-paths` ever remove a row with no `host`?

**Options:**
- (a) Leave as is (the spec's current choice). No work; stale pre-upgrade rows stay until deleted on the board.
- (b) Adopt unowned rows on the first prune (treat `host == ""` as this host's). Cleans them up; on a multi-host store the first host to prune judges every old row by its own disk — the original A1 hazard, once.
- (c) Add an `--include-unowned` flag. Explicit and safe; one more flag and spec line.

### CLI store pin (`src/store_connection/mod.rs::open_cli_store`)

**Question:** Should one-shot CLI commands check the store-identity pin the board checks?

**Options:**
- (a) Check it in `open_cli_store` and refuse on mismatch with a message naming the expected and found store. Prevents writes to the wrong store; a CLI run in a shell with a different server now fails until fixed.
- (b) Document in `docs/specs/cli.allium` that the CLI trusts its environment. No code; the #28710 failure mode stays.

Ask the user with `AskUserQuestion` before implementing these findings. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `src/spacetime/tests/module_schema.rs` (+ snapshot/seed code) | Include `learning_verdicts` |
| `spacetime/module/src/tables.rs` | Fix the `LearningVerdict` comment |
| `src/sync/rows.rs`, `src/cli/commands.rs`, `docs/specs/cli.allium` | W2 per decision |
| `src/editor.rs` | Known section names only; gentler trim |
| `src/mcp/handlers/learnings.rs`, `src/embeddings.rs` | Normalise `scope_ref`; tolerant compare; panic-safe thread |
| `src/store_connection/mod.rs`, `docs/specs/cli.allium` | F4 per decision |

## Verification

- [ ] Run the task's verify command — all pass
- [ ] `cargo test --no-fail-fast` green
- [ ] `cargo clippy --all-targets -- -D warnings` clean; the doc checkers pass
