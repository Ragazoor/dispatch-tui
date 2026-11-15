# Store layer: usage retention cap and drift

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** the module owns the usage retention rule so no caller can prune other people's events, and `src/store/` says what it does.

## Context

Findings V3 and M5 from `docs/plans/2026-11-15-codebase-review.md` §3.2 and §3.7. CLAUDE.md "Store Layering": data rules belong in the module. Mirror module changes in `src/sync/memory_caller/`; bump both rules versions together.

## Findings

### 🟡 Medium — V3: any caller sets the usage-retention cap and prunes everyone's events (`spacetime/module/src/config.rs::record_usage_event`, `::prune_usage_events`; `src/store/queries/usage.rs::record_usage_event_with_cap`; `src/store/mod.rs::UsageCap`)

**Issue:** the module accepts any `cap > 0` and deletes every `usage_events` row with `id <= highest - cap`, across all people. `record_usage_event(row, 1)` wipes all telemetry, which the keybinding analysis reads as "unused". Each event (one per keypress) scans the whole table in the reducer.

**Fix:** a module constant for the cap; drop the argument (or clamp it to at least the constant); prune by id range rather than a full scan. Remove `UsageCap` from the board. Conformance case: a call with cap 1 leaves earlier events.

### 🟢 Low — M5: small store-layer drift (`src/store/queries/epics.rs::create_epic_configured`; `src/store/queries/settings.rs::refuse_identity_key`; `src/store/mod.rs` docs; `src/atomic_write.rs::write_with`, `::create_new_bytes`)

**Issue:** `create_epic_configured` returns the row as sent, but `visibility.rs::settle_new_epic` rewrites `workspace_id`, `created_by` and `feed_command_set_by` (the MCP reply can name the wrong workspace). Identity setting keys are refused on the board only; the module's `save_setting` accepts them. `src/store/mod.rs` docs talk about SQLite, "migration v35" and cycle detection in the service layer; `SessionRequestStore` claims every call names this host (`request_exit_session` sends none). `atomic_write` does not fsync the parent directory after the rename, so a power loss right after identity adoption can lose the credential.

**Fix:** read the created epic back; move the identity-key refusal into the module's `save_setting` (spec `settings.allium`); correct the docs; sync the parent directory after rename.

## Changes

| File | Change |
|------|--------|
| `spacetime/module/src/config.rs` | cap constant; id-range prune; refuse identity keys |
| `src/sync/memory_caller/config.rs` | mirror |
| `src/store/mod.rs`, `src/store/queries/{usage,epics,settings}.rs` | drop `UsageCap`; read back; docs |
| `src/atomic_write.rs` | fsync parent directory |
| `docs/specs/settings.allium` | the module refuses identity keys |
| `tests/memory_caller_conformance/` | cap case; identity-key case |
| `spacetime/module/src/rules.rs`, `src/sync/rules_gate.rs` | bump together |

## Verification

- [ ] `./scripts/verify.sh` passes
- [ ] The cap conformance case fails before the fix
