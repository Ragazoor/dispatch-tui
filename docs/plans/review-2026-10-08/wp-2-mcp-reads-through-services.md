# MCP Reads Through Services

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Route the remaining direct state.db reads in MCP handlers through the task and epic services.

## Context

This work package addresses findings from the code review in `docs/plans/review-2026-10-08/report.md`.

## Findings

### 💡 Direct `state.db` reads (`src/mcp/handlers/poll_ownership.rs:66`, `:80`; `src/mcp/handlers/tasks/dispatch.rs:29`, `:142`, `:225`)

**Issue:** Handlers call `state.db.get_task`/`get_epic` directly, bypassing service-level behaviour. Two read paths exist.

**Fix:** Use `state.task_svc`/`state.epic_svc` reads. Follow `docs/invariants.md` (mutation boundary). Spec first if behaviour changes (it should not).

## Changes

| File | Change |
|------|--------|
| `src/mcp/handlers/poll_ownership.rs` | Read via services |
| `src/mcp/handlers/tasks/dispatch.rs` | Read via services |
| `src/mcp/handlers/*` | `grep state\.db\. src/mcp` for others and convert reads |

## Verification

- [ ] `cargo test` passes; `cargo clippy --all-targets -- -D warnings` clean
- [ ] `grep -rn "state\.db\." src/mcp` shows no non-test reads left
