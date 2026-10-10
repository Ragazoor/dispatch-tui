# The split between the TUI and the SpacetimeDB module

**Status:** approved by ragge on 2026-10-10
**Task:** #53254, epic #314
**Diagrams:** https://claude.ai/artifact/EHZgksU8SEnzy9DGM43S15 (private; the
iteration history of this design)
**Spec:** [`docs/specs/store-split.allium`](../../specs/store-split.allium)

## TL;DR

The TUI process should handle the store connection, worktrees and the UI, and
little else. Today it also holds most of the domain logic, in the services
(`src/service/`). This design moves every rule about the shared data into the
SpacetimeDB module and leaves the TUI with only the work that needs the
operator's machine.

## Today

Everything except the store runs in one `dispatch tui` process: the TUI, the
Axum `/mcp` and `/hook` server, the task, epic and learning services, the feed
runner, the PR poller, embeddings, and the store client (`SharedRows` for
reads, `ReducerCaller` for writes). Agents reach the store only through that
process.

The rules are split between two places:

| In the module (reducers) | In the TUI process (services, runtime) |
| --- | --- |
| A task belongs to an epic or an owner | MCP parameter checks, sub-status legality |
| Status is a known value | Dispatch order: claim → prologue → mode → provision → record |
| `completed_at` / `updated_at` from the store clock | Wrap-up rebase, conflict sub-status, tmux teardown |
| Epic status rollup up the parent chain | Watcher notices: who to tell, file write, tmux nudge |
| One claim per backlog task | Repo grouping, reparent cycle check |
| Feed upsert and stale-subtree delete | Feed routing to a role, PR state from `gh` |
| Poll owner, host registration, subagent bookkeeping | Prompt building, RAG, embeddings |
| Learning verdicts and stale archive | Board layout, filters, folds |

A rule in the right-hand column is one that only a client linking the dispatch
crate gets, and one that two boards on different builds can disagree about.

## Decisions

**S1 — The split rule.** A piece of behaviour stays in the TUI process only if
it needs the operator's machine: git, tmux, launching `claude`, `gh`, local
repo files, the local embedding model. Every rule about the shared data's
shape, consistency or state transitions belongs in a reducer.

**S2 — Host work stays in the TUI process, thin.** The MCP and hook endpoint,
feed scripts and PR polling stay in the TUI process. There is no separate
daemon, so agents still need a board running. Each of these does only the
local call and forwards the raw result through one reducer call. None decides
a state change.

**S3 — The module drives multi-step work.** Dispatch, wrap-up and
exit-session start as request rows that any client can write. The module sets
each state (requested → provisioning on host X → running or failed). The host
that owns the work sees the row through its subscription, does the local
work, and reports the outcome by reducer call. A scheduled reducer fails work
that a host never reports. It replaces the 14-minute dispatch watchdog
(`DISPATCH_WATCHDOG_TIMEOUT`), which lives in TUI memory and is lost when the
board stops.

Trade-off accepted for S3: dispatch becomes asynchronous. The gain is that the
TUI never decides a state change, and any client (a teammate's board, a future
web client) can request work through the same path.

## The target loop

1. A requester (agent via MCP, hook, key press, feed runner, PR poller) makes
   one reducer call with what was asked or what was observed.
2. The module validates, applies, and, if local work is needed, writes a work
   row naming the host.
3. That host's subscription delivers the row. The host runs git, tmux,
   `claude` or `gh`.
4. The host reports the outcome by reducer call. The module moves the state
   on.
5. A scheduled reducer fails work whose host never reported.

## Migration

Each step leaves the system working and is one task in the epic created for
this migration.

| Step | Change |
| --- | --- |
| M2 | Pure data checks into reducers: sub-status legality, parameter checks, reparent cycle check, repo grouping. Services become pass-through. |
| M3 | Feed routing and reconcile into the module. `FeedRunner` runs the script and posts raw items. |
| M4 | Dispatch as request rows, with a scheduled timeout. Removes the TUI watchdog. Proves the pattern. |
| M5 | Wrap-up and exit-session as request rows. The module owns conflict and done; the host runs the rebase and the teardown. |
| M6 | Watcher "notice due" rows, delivered by the watcher's host. PR state derived in the module from the raw `gh` status the host posts. |
| M7 | Remove what is left in the services. MCP handlers call reducers directly. |

(M1 was this design and the spec file.)

## Risk

A board on an old build applies old rules against a newer module, and the
half-moved state looks fine while it corrupts quietly. Every step that moves a
rule needs a hard startup version gate, not a comment.

## Open questions

Tracked as `open question` entries in `docs/specs/store-split.allium`, for the
step that meets them: the timeout's duration, what the version gate compares,
whether request rows are a new table or task columns, and how an offline host
catches up on a request row written while it was away.
