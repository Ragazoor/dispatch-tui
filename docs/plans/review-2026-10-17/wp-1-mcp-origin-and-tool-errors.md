# MCP Port Origin Check and Tool Errors

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Stop non-loopback origins from reaching the MCP port, and make tool failures come back as errors in one consistent shape.

## Context

This work package addresses findings from the 2026-10-17 code review
(`docs/plans/2026-10-17-codebase-review.md`, findings 1, 28–31, 43–49).
Behaviour changes start in the spec (`docs/specs/mcp-task-tools.allium`,
`docs/specs/agent-health.allium` for hooks) — update it first, then tests,
then code. The caller-check findings in the review's "Open question: the trust
model" section are out of scope here.

## Findings

### 🔴 No Host/Origin check or secret on the MCP port (`src/mcp/mod.rs:237`)

**Issue:** `router_over` has no Host/Origin validation and no secret; caller
identity is a client-supplied header (`src/mcp/middleware.rs:23`). A
DNS-rebinding page reaching `127.0.0.1:<port>` can POST with
`X-Caller-Kind: session` and call `update_epic(feed_command)`,
`managed_feed_config`, `set_verify_command`, `dispatch_task`. Rebinding makes
the request same-origin, so no CORS preflight blocks it.

**Fix:** Add a middleware before `extract_caller_identity` that rejects
requests whose `Host` is not `127.0.0.1:<port>` / `localhost:<port>`, and any
request carrying a non-loopback `Origin` (the MCP streamable-HTTP spec
requires this). Apply it to `/mcp`, the hook path and the pane-view path.
Check what Host header Claude Code and the hook client actually send before
choosing the allow list. A per-board bearer token is a possible follow-up;
not required here.

### 🟡 Failed "mark done" returns a success-shaped result (`src/mcp/handlers/tasks/crud.rs:305`, `:184`)

**Issue:** When `close_task` fails, `handle_mark_task_done` returns
`JsonRpcResponse::ok` with "could NOT be moved to done" and no `isError`. In
the multi-id loop the line still counts toward "Updated N of N".

**Fix:** Return an error response on `Err(e)` (re-wrapped as `isError` by
`handle_tools_call`), keep the text, and do not count it as updated.

### 🟡 Omitted `arguments` rejected (`src/mcp/handlers/dispatch.rs:744`)

**Issue:** `params.get("arguments")...unwrap_or(Value::Null)` makes every
struct parse fail for `tools/call {name:"list_tasks"}`. MCP allows omitting
`arguments`; tests always pass `{}`.

**Fix:** Default a missing or explicit `null` to `json!({})`. Add a test with
no `arguments` key.

### 🟡 Inconsistent MCP error shapes (`src/mcp/handlers/dispatch.rs:750` vs `:815`)

**Issue:** Agent-name resolution and identity failures return raw JSON-RPC
errors and skip trajectory/usage recording; everything from `dispatch_tool`
becomes `isError` with the code dropped. `get_task("nope")` and
`get_task(99999)` answer differently.

**Fix:** Route the resolution failure through `tool_error` too, after
recording. Update `docs/mcp.md` if it promises `-32001` through `tools/call`.

### 🟡 `set_verify_command` schema forbids the documented null (`src/mcp/handlers/dispatch.rs:592`)

**Issue:** Schema says `"type":"string"`, description says "pass null to clear".

**Fix:** `"type": ["string","null"]`.

### 🟢 Low findings

- `src/mcp/handlers/tasks/mod.rs:201`, `src/mcp/handlers/epics.rs:44` —
  `limit` is a plain `NonZeroUsize`; accept numeric strings like the other
  integer args (`deserialize_optional_flexible_i64` pattern).
- `src/mcp/handlers/learnings.rs:219` — `limit.unwrap_or(50).min(50) as usize`
  turns negatives into `usize::MAX`; reject `< 1` or clamp to `1..=50`.
- `src/mcp/handlers/types.rs:491` (`fetch_caller_task`) — reused for target
  ids in `exit_session` and `record_learning`, so a typo says "Unknown caller
  task N"; use the standard not-found message for target ids.
- `src/mcp/handlers/task_refs.rs:29` — `is_id` trims, `FlexibleI64Visitor`
  does not; make them agree.
- `src/mcp/handlers/tasks/crud.rs:458` — `list_tasks` description says an
  explicit `epic_id`/`repo_paths` disables auto-scoping, but the caller's own
  task is still excluded; fix the description or the behaviour (spec decides).
- `src/mcp/handlers/dispatch.rs:787` — usage rows are recorded for any tool
  name; record only names in `TOOL_NAMES`, bucket the rest.
- `src/hooks/mod.rs:131` — a newer hook binary's unknown event gets a 422 and
  fails in the agent's terminal; add a `#[serde(other)]` catch-all or answer
  undecodable bodies with a JSON `Failed` outcome.

## Changes

| File | Change |
|------|--------|
| `docs/specs/mcp-task-tools.allium` | Record the origin rule, error shape, omitted-arguments default |
| `src/mcp/mod.rs`, `src/mcp/middleware.rs` | Host/Origin middleware ahead of identity extraction |
| `src/mcp/handlers/dispatch.rs` | Default `arguments`; route resolution errors through `tool_error`; nullable schema; usage filter |
| `src/mcp/handlers/tasks/crud.rs` | Error response for failed close; description fix |
| `src/mcp/handlers/tasks/mod.rs`, `src/mcp/handlers/epics.rs`, `src/mcp/handlers/learnings.rs` | Flexible / bounded `limit` |
| `src/mcp/handlers/types.rs`, `src/mcp/handlers/task_refs.rs` | Not-found message; trim consistency |
| `src/hooks/mod.rs`, `src/hooks/wire.rs` | Unknown-event tolerance |
| `docs/mcp.md` | Error-code wording if it changes |

## Verification

- [ ] Tests first: a request with `Host: evil.example` and one with `Origin: http://evil.example` are rejected; a loopback request passes
- [ ] Test: `tools/call` without `arguments` succeeds for `list_tasks`
- [ ] Test: a failed close yields `isError` and is not counted
- [ ] `cargo test --no-fail-fast` and the task's verify command pass
