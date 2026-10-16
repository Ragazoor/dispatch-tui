# MCP & Feeds

## MCP Notification Flow

When an MCP handler mutates the database, the TUI must refresh to show the change. This is the propagation path:

```
MCP handler (e.g. handle_update_task)
  → mutates via state.task_svc / state.epic_svc   # never state.db — that's a compile error
  → calls state.notify()                          # McpState method
    → sends BoardEvent::Refresh via mpsc::UnboundedSender
      → runtime event loop receives it             # tokio::select! in run_event_loop()
        → calls rt.exec_refresh_from_db(app)
          → reads all tasks/epics from DB
          → calls app.update(Message::Task(TaskMessage::Refresh(tasks)))
            → App replaces its in-memory task list, re-renders
```

Key types in the chain:
- `BoardEvent` (`src/board_event.rs::BoardEvent`) — `Refresh` (catch-all full reload), `TaskChanged(TaskId)` / `EpicChanged(EpicId)` (targeted single-row reloads, preferred when the changed entity is known)
- `McpState::notify()` — fire-and-forget send on the channel
- `TuiRuntime::exec_refresh_from_db()` (`src/runtime/tasks.rs`) — reloads tasks, epics, and usage from DB
- `TaskMessage::Refresh` (`src/tui/messages/task.rs`) — carries the fresh task list into the App, wrapped as `Message::Task`

Agent-to-agent messaging (task #4098) no longer has an `BoardEvent` of its own —
agents call Claude Code's native `SendMessage` directly, and dispatch observes
it via the Claude Code hook pipeline rather than through this MCP-server
notification channel. The observed timestamps
(`Task.last_peer_message_sent_at`/`last_peer_message_received_at`) ride the
ordinary DB-refresh path above instead: `detect_task_transition_notifications`
(`src/tui/update/agent.rs`) diffs them against the previous in-memory row on
each refresh and flashes `AgentTracking::message_flash_sent`/`message_flash`
accordingly. See `HookPeerMessageSent` in `docs/specs/agent-health.allium`.

Hooks deliberately do **not** reach that channel. Every Claude Code hook now
posts to `/hook`, served by the same Axum router as `/mcp`
(`src/mcp/handlers/hooks.rs::handle_hook`), but an applied event emits no
`BoardEvent`: a `PreToolUse` arrives on every tool call of every live session, so
a notification each would cost one extra row read and one full repaint per tool
call, in the process that also draws the board. The tick-driven refresh that
picked these writes up when hooks wrote to the database directly still does, at
a rate that does not scale with how busy the agents are. See `HookDelivery` in
`docs/specs/agent-health.allium` for what the endpoint guarantees, and
`src/hooks/` for the client side.

## MCP State Machines

`McpState` holds no multi-call handshake state. The wrap-up handshake that used to live in an in-memory map there is a pair of request rows in the shared store (store-split M5), so it survives a board restart.

**`wrap_up` → `exit_session` handoff** (`src/mcp/handlers/tasks/wrap_up.rs`; `docs/specs/store-split.allium`: `RequestWrapUp`, `RequestExitSession`, W1-W13):

`wrap_up(task_id, action)` runs its **host-local checks first** — the task is wrappable (running or review, a worktree, not review-tagged), the worktree is committed, and for `rebase` the worktree names a branch — so a refusal writes no row and issues no token. Then it writes a request through `TaskService::request_wrap_up`: one `session_requests` row of kind `wrap_up` naming this host and recording the action (`rebase` | `done` | `pr`). The module re-checks wrappability, review tags, that the worktree is on this host, and that the task holds no other open session request (an `INVALID_PARAMS` refusal in the module's words). The task's `status` is unchanged by `wrap_up` in every case — it stays whatever it was (`running`) until the closing call.

**The board does not run the verify command.** The agent runs it in its own worktree (the wrap-up skill's Step 6). After a `rebase` that moved the worktree's HEAD, and only when the repo has a verify command, the response adds a line telling the agent to re-run it before `exit_session`. See `AgentsRunTheVerifyCommand` in `docs/specs/mcp-task-tools.allium`.

- `done` and `pr` have no host work: the module writes the row already done, and the tool answers at once.
- `rebase` writes a `requested` row and **waits** on it (`McpState::wrap_up_wait_cap`, `WRAP_UP_WAIT_CAP` = the 120s rebase deadline plus the 10s backstop grace), waking on the subscription's change signal. This board's session reactor does the rebase (see below). A done row answers with the success text; a failed row is an `INTERNAL_ERROR` `wrap_up failed: <reason>` (a conflict's reason names the files, and the module has set the Conflict sub_status; `timed out`, `deadline passed` and `host restarted` read the same way); a row still open when the wait ends is an error naming the request and saying no token was issued.

**The exit token is the id of the done `wrap_up` row** (`Exit token: <id>` in the response). Only the task's most recent done wrap_up row is valid, each is consumed once, a later dispatch of the task voids it, and it survives a board restart.

The `/retro` skill (the mandatory reflection step — there is no in-handler reflection prompt anymore) does **not** run between these two calls. The `/wrap-up` skill invokes it earlier, before its commit step and ahead of `wrap_up`, so any agent-context fix retro makes is committed with the session's work rather than stranded after the rebase or the push.

`exit_session(task_id, token, action, pr_url?)` is a **single call**. The tool refuses a missing token or action itself (with today's text); everything else is one `TaskService::request_exit_session` call, which is ONE module transaction:
1. The module checks the token (a done wrap_up row of this task, the most recent, not voided by a later dispatch, not consumed), that `action` matches the row's (mismatch → error naming both actions), that `task.tmux_window` is still set ("no active session" otherwise), and that `pr_url` is present iff `action = "pr"`. An unparseable token names no row and is refused as `invalid exit token`.
2. It applies the terminal mutation: `rebase`/`done` → `status = Done`; `pr` → `status = Review`, `url` set to the pr-typed URL; the window is cleared, the epic status rolled up, a phoenix successor respawned.
3. It runs the **epic chain**: if the task has an `epic_id` and that epic has `auto_dispatch` on, the first backlog subtask the module accepts is requested as an `epic_chain` dispatch request naming this host (`epics.allium: AutoDispatchNextSubtask`; phoenix subtasks are passed over). The request is the claim, so two concurrent closes can never launch two agents for one subtask — and so does every other entry point (`TaskService::request_dispatch`). There is deliberately no agent-facing tool for it.
4. It writes the `exit_session` row, naming the window for the teardown and the chained subtask for the response, and consumes the token.

The close is whole or absent: a refused call changes nothing and **leaves the token valid**, so the agent fixes the cause and calls again. The state the old board-side close had — token spent, terminal write failed, a successful response telling the agent to stop — no longer exists. The response is `Session closed.`, or `Session closed. Dispatching next epic subtask #<id> '<title>'.` from the row's `chained_task`. **The tool does not wait for the teardown**: it kills this agent's own window, so a reply that waited would never be read, and a failed teardown does not undo the close. A call that got no answer from the store is settled from the rows — an `exit_session` row that consumed the token means the close took effect and the accepted response is returned; none means it did not, and the error says so and that the token is still valid.

**The host's side** (`src/runtime/session_host.rs`, `TaskService::serve_session_request` in `src/service/tasks/session.rs`): this board's reactor claims each `requested` row naming its host (the module's claim clears a Conflict sub_status), and under the row's own deadline (`DeadlineRunner`) either runs `finish_task` (a `wrap_up` rebase) or kills the window the row names (an `exit_session` teardown), then reports done or failed. A genuine rebase conflict is reported with `conflict = true` and the module sets the sub_status; the host writes no task field. For a close into Done of a task carrying a PR url the host also runs `gh pr close` (`ClosePrOnDone`). A failed teardown never reverts the close. See `docs/invariants.md` ("Dispatch seam", "Session seam").

This closes a race that existed when `wrap_up("pr")` used to set `status = review` immediately: that armed PR-merge polling (`PollPrStatus`) before the session was actually closed, so a merge/close could null `tmux_window` while the agent was still working. Now a PR task never becomes poll-visible until the exact same transaction that also ends the session.

A crash before the closing call leaves no stranded state: the task simply hasn't transitioned yet, and the token (a row) stays valid until a newer wrap-up, a dispatch or the close replaces it. A board that restarts mid-rebase fails that row `host restarted` at startup and does not resume it; the agent's waiting `wrap_up` died with the board and it calls `wrap_up` again. `update_tasks(status="done")` closes in the module too (`close_task`, store-split M7): no token and no chain, and a `close` row for the host's teardown when the task has a window or a PR.

Do not add new ad-hoc state machines on `McpState` without documenting them here.

## MCP Error Codes

MCP handlers in `src/mcp/handlers/` return JSON-RPC error objects using two codes (the context resource methods also answer `-32600` for a missing or malformed caller identity):

| Code | Meaning | When to use |
|------|---------|-------------|
| `-32602` | Invalid params | Validation failure, missing required field, unknown tool name — maps to `ServiceError::Validation` |
| `-32603` | Internal error | Unexpected DB error, I/O failure — maps to `ServiceError::Internal` or `anyhow` errors |

Use `JsonRpcResponse::err(id, -32602, msg)` for anything the caller can fix; use `-32603` for anything they can't.

## Context resources

The server offers read-only context as MCP resources (`resources/list`, `resources/read`), declared beside `tools` in `initialize` (no `subscribe`, no `listChanged`, no skills extension). Logic lives in `src/mcp/handlers/context.rs`; the behaviour is `McpContextResources` in `docs/specs/mcp-task-tools.allium`.

| URI | Serves |
|-----|--------|
| `skill://<name>/SKILL.md` | the skill file from the copy built into the binary (`src/setup/plugins.rs::built_in_skills_dir`), never the installed copy |
| `skill://<name>/references/<file>.md` | a file from that skill's `references/` directory |
| `dispatch://learnings/<id>` | an approved learning's summary then detail |
| `dispatch://task/self` | the caller's own task, rendered as `get_task` does (`src/mcp/handlers/tasks/mod.rs::task_detail_text`) |

**Tool twin.** Claude Code does not pull resources into an agent's context by itself, so `read_context(uri)` returns the same text. Listing is paged (50 entries, opaque position-based `nextCursor`) in a fixed order: own task, skills by name, learnings by id.

**Error envelopes.** A missing or malformed identity is `-32600` on `resources/list`, `resources/read` and `read_context` (`initialize` and `tools/list` stay identity-free). A URI that does not resolve is `-32602` naming the URI on `resources/read`, and the same message as an `isError` tool result on `read_context`. An invalid cursor is `-32602`; an internal failure is `-32603` (`isError` on `read_context`).

**Never served.** No URI reaches an epic, a trajectory, usage data, `host.json`, the store, or a repository file; unapproved or deleted learnings read as unknown. Reads record no retrieval and write nothing, so `rate_learning` still requires a retrieval.

## Notifications

JSON-RPC 2.0 §4.1 forbids replying to a Notification (a request with no `id`). The MCP streamable-HTTP transport maps this to `HTTP 202 Accepted` with an empty body. `handle_mcp` short-circuits any request where `id.is_none()` to a 202 — including unknown methods. Claude Code sends `notifications/initialized` after every `initialize`; replying to it (even with an error) makes its strict response schema reject `id: null` and aborts the MCP session.

## Debugging MCP handlers

The MCP server listens on port 3142 by default (override with `DISPATCH_PORT`), on the `/mcp` path — posting to the bare origin gives a 404. `tools/call`, `resources/list` and `resources/read` also need a caller-identity header: exactly one of `X-Caller-Task-Id` or `X-Caller-Kind`, never both. When a handler misbehaves you can reproduce it without going through Claude Code:

```bash
# Tail server logs while the TUI runs (logs go to stderr; redirect when launching)
RUST_LOG=dispatch=debug cargo run -- tui 2> /tmp/dispatch.log
tail -f /tmp/dispatch.log

# Send a manual JSON-RPC request to a tool (e.g. list_tasks)
curl -s -X POST http://127.0.0.1:3142/mcp \
  -H 'Content-Type: application/json' \
  -H 'X-Caller-Task-Id: 42' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_tasks","arguments":{}}}' \
  | jq

# Reproduce a failing update — substitute the offending arguments
curl -s -X POST http://127.0.0.1:3142/mcp \
  -H 'Content-Type: application/json' \
  -H 'X-Caller-Task-Id: 42' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"update_tasks","arguments":{"task_ids":[42],"status":"done"}}}' \
  | jq
```

`tools/list` returns the tool schemas — useful when the argument shape isn't obvious.

## Feed Epics

Feed epics are epics whose tasks are populated externally by a shell command rather than by a human. When an epic has a `feed_command` set, the runtime runs it periodically (`feed_interval_secs`) and calls the `ingest_feed` reducer to sync the results. Each feed task has an `external_id` that is used as the upsert key — tasks are created on first appearance and updated (but not deleted) on subsequent runs.

Feed tasks appear in their own column on the kanban board (`SubStatus::Feed`). The schema is part of the store module. See `docs/specs/feeds.allium` for the full specification.

## Knowledge Base Flow

The Knowledge Base lets dispatched agents record knowledge entries that are automatically injected into future dispatch prompts.

### End-to-end lifecycle

1. **Agent records** — calls `record_learning(task_id, kind, summary, scope, ...)` during a task or at wrap-up. The entry is immediately active and will appear in future dispatch prompts for agents working in the matching scope.
2. **Future dispatches** — when an agent is launched, `dispatch_with_prompt()` queries approved entries for the task's context and prepends them to the prompt (see `docs/specs/learnings.allium`).
3. **Agent rates** — calls `rate_learning(learning_id, task_id, verdict)` when it acts on a retrieved entry. `helped` increments `upvote_count` (raising the entry's priority in future results); `wrong` decrements `upvote_count` (a downvote; may go negative) and leaves the status unchanged. Only entries surfaced to the task (injected or returned by `query_learnings`) can be rated. There is no human-approval step and no human-facing curation surface: entries land approved, and curation happens through MCP (`rate_learning`, `delete_learning`) plus a background job that archives approved entries with a non-positive score that have gone stale (see `docs/specs/learnings.allium`: `ArchiveStaleLearning`).

### Scope model

Each learning has a `scope` that determines which tasks receive it:

| Scope | Included when | `scope_ref` |
|-------|---------------|-------------|
| `user` | Always | `null` |
| `repo` | Task's repo path matches | `repo_path` |
| `epic` | Task belongs to this epic | `str(epic_id)` |
| `task` | Only via explicit `query_learnings` | `str(task_id)` |

`scope_ref` is auto-derived from the task context when omitted. `task`-scoped entries are excluded from auto-injection (they capture task-specific outcomes and must be fetched on demand).

### Ordering

Candidate entries are fetched from the store ordered by kind (`procedural` first), then scope specificity (`epic` → `repo` → `user`), then `upvote_count DESC`. That ordering only selects the candidate set: the injected block itself is RAG-ranked by relevance to the task, so `kind` confers no precedence in the final prompt. Procedural entries are **not** prepended as a verbatim prefix — every retrieved entry, procedural included, goes into the single ranked block.

### Status lifecycle

```
approved → archived (terminal)
```

Approved entries affect dispatch; archived entries do not. `rejected` remains a valid `LearningStatus` because existing rows still carry it, but nothing writes it any more — the reject path was removed with the TUI overlay.

### Implementation references

- `src/mcp/handlers/learnings.rs` — MCP tool handlers
- `src/service/learnings.rs` — `LearningService` (create, query, retrieval/verdicts, stale sweep, delete)
- `src/store/` — `LearningStore` trait, `LearningPatch`, `LearningFilter`
- `src/dispatch/agents.rs` — prompt augmentation in `dispatch_with_prompt()`
- `docs/specs/learnings.allium` — full domain specification
