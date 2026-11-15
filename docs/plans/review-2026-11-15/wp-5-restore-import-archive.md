# Restore, import and archive: seed paths meet the one-time migrations

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** data that comes back through restore or import goes through the same normalisation as the live store's one-time migrations, and archived records follow their spec.

## Context

Findings K1–K6 from `docs/plans/2026-11-15-codebase-review.md` §3.4. K1 and K2 share one cause: a migration gated on the store's rules stamp does not see rows that a seed reducer writes later. The structural fix is one seed-path normaliser per migration. Spec first (`docs/specs/spacetime-seed.allium`, `core.allium: ArchivedTask`), then tests, then code. Mirror in `src/sync/memory_caller/`.

## Findings

### 🟡 Medium — K1: a restore or import of older data brings task-scoped learnings back, seen by the whole workspace (`spacetime/module/src/seed.rs::seed_learnings`; `lib.rs::stamp_rules_version_on_connect`; `visibility.rs::learning_seen_by`)

**Issue:** the first connection stamps an empty store at the current version, then `seed_learnings` inserts `scope = "task"` rows raw; `restore.rs` and `import.rs` run no retire afterwards. `learning_seen_by` answers `true` for any scope other than `epic`, so a learning about a task in someone's private epic is readable by every workspace member (or by everyone with `workspace_id = 0` and no author).

**Fix:** `seed_learnings` applies the same retire as the `TASK_SCOPE_REMOVED_RULES_VERSION` migration to each row (or refuses `task`). Test: restore a snapshot holding a task-scoped learning; it lands retired.

### 🟡 Medium — K2: old snapshots and stores with an unfit sub_status are refused half-way through a restore (`spacetime/module/src/rules.rs::check_sub_status_write` via `seed.rs::seed_tasks` → `tasks_epics.rs::store_task`; `src/spacetime/restore.rs::restore`; `src/spacetime/import.rs`)

**Issue:** `reset_unfit_sub_statuses` cleans the live store once (6962bc38); a snapshot holding backlog/approved is refused by `seed_tasks` after `burn_id_sequences` and earlier tables have been written, against `NothingIsWrittenBeforeTheChecksPass`.

**Fix:** normalise unfit pairs to `default_sub_status` while preparing the snapshot or import (as the migration does), in one shared function both the migration and the seed path call. Test: restore a snapshot with an unfit pair; it succeeds with the pair reset.

### 🟢 Low — K3: the operator's archive listing misses records (`spacetime/module/src/archive.rs::archived_tasks_for`)

**Issue:** `core.allium: ArchivedTask` says the operator sees every record, and `archived_task_for` agrees, but the list reads only the caller's owner index and listed workspaces.

**Fix:** for the operator, iterate the table.

### 🟢 Low — K4: archived records keep their audience after the epic goes private or moves (`archive.rs::archived_task_visible_to`) — decision item

**Issue:** old-workspace members keep reading archived titles and descriptions after the epic goes private or moves. `core.allium` freezes the audience on purpose; `ClaimsFollowSight` points the other way.

**Fix:** see Decisions.

### 🟢 Low — K5: soft-delete conformance covers one path (`tests/memory_caller_conformance/agents.rs`)

**Issue:** only an epic-less `delete_task` record is compared. Missing: private-chain records (`workspace_id`, `private_holder`), `batch_delete`, `delete_epic_subtree`, the super-admin hatch, reads by member, non-member, holder and operator.

**Fix:** add those cases, read back as two identities.

### 🟢 Low — K6: stale citation of a removed rule (`src/spacetime/import.rs::DroppedIds`; `docs/specs/spacetime-seed.allium: DependentsOfDroppedRowsAreDropped`)

**Issue:** both cite `agents.allium: AgentNameIsNeverReused`, removed by 0171c8d7. Names are reusable now, so an archived record's "Agent: X (archived)" can name a live agent.

**Fix:** update both citations; show the archived agent with its task id.

## Decisions

### K4: whose sight governs an archived record?

**Question:** after a task is archived, should its record's audience stay as it was at archive time, or follow the live epic's sight (private flip, workspace move) while the epic exists?

**Options:** (a) frozen at archive time (today's spec) — simple, but people who lose sight of an epic keep reading its archived tasks; (b) follow the live epic while it exists, frozen once it is deleted — consistent with `ClaimsFollowSight`, costs a read of the epic per record; (c) narrow on a flip or move (rewrite records' `workspace_id`/`private_holder` then) — cheap reads, more writes on those paths.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/spacetime-seed.allium`, `core.allium` | seeds normalise like migrations; K4 decision; citation |
| `spacetime/module/src/{seed,rules,lib,archive}.rs` | shared normalisers; operator listing |
| `src/spacetime/{restore,import}.rs` | call the normaliser or rely on the seed; citation |
| `src/sync/memory_caller/` | mirror |
| `tests/` (restore tests), `tests/memory_caller_conformance/agents.rs` | old-snapshot restores; soft-delete cases |

## Verification

- [ ] `./scripts/verify.sh` passes
- [ ] A restore of a snapshot with a task-scoped learning and an unfit sub_status pair succeeds, retires the learning and resets the pair
