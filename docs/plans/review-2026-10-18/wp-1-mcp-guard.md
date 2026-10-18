# MCP Guard Hardening

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Close the gaps in the MCP Host/Origin guard and make MCP error shapes consistent.

## Context

This work package addresses findings from the 2026-10-18 codebase review (`docs/plans/2026-10-18-codebase-review.md`, finding numbers match that report). Verify each finding in the code before fixing; it may have moved.

## Findings

### Medium: Hook route dropped its Content-Type requirement (`src/mcp/handlers/hooks.rs:~40`, `src/mcp/mod.rs:~244`)

**Issue:** `handle_hook_body` takes raw `Bytes`. A cross-origin `text/plain` POST needs no preflight, so a page with a loopback Origin can forge `observe` events for any task.

**Fix:** Reject a non-JSON Content-Type, or take `Json<Value>` and decode afterwards. Test both.

### Medium: Origin guard ignores the port (`src/mcp/middleware.rs:38`)

**Issue:** `Origin: http://localhost:<any port>` passes. The authority parser is loose (`localhost:x@evil.com` reads as `localhost`).

**Fix:** Compare the port with the bound one and parse the authority properly. Add tests for `Origin: null`, `localhost.`, upper case, port mismatch.

### Low: `exit_session` and others return a raw JSON-RPC error for a mistyped target id (`src/mcp/handlers/tasks/wrap_up.rs:353`)

**Issue:** `fetch_caller_task`, `resolve_list_scope` and `create_task` still use the old error shape and say "Unknown caller task N".

**Fix:** Route through `tool_error` like the other handlers; give a message that names the target id.

## Changes

| File | Change |
|------|--------|
| `src/mcp/handlers/hooks.rs`, `src/mcp/mod.rs` | Require JSON Content-Type on the hook route |
| `src/mcp/middleware.rs` | Port-aware loopback check, strict authority parse, tests |
| `src/mcp/handlers/tasks/wrap_up.rs`, `src/mcp/handlers/types.rs` | Unified error shape |

## Verification

- [ ] Run existing tests — all pass (`cargo test --no-fail-fast`, redirect output, do not pipe)
- [ ] New tests for the origin edge cases and a text/plain hook POST
- [ ] Run the repo's verify command from get_task
