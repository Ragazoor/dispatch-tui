# Phase 6b: route agent session state through reducers

Task #4906. Routes the twelve remaining shared mutations `db::SHARED_WRITES_ARE_COMPLETE`
lists under "Agent session state": `subagent_start`, `subagent_stop`, `subagent_clear`,
`subagent_clear_and_void_pending_stop`, `shell_start`, `shell_stop`,
`shell_clear_no_drain`, `try_record_stop`, `record_pre_tool_use`, `record_notification`,
`record_user_prompt_submit`, `mark_pr_learnings_gate_shown`.

No domain rule changes. `docs/specs/agent-health.allium`'s guarantees (the counter
arithmetic, the drain predicate, the flip-or-defer decision, session fencing) are
unchanged and are the reference the reducers must reproduce exactly — same end state,
new backend. `epics.allium: EpicStatusRecalculation` already states, generically, that
once a mutation is routed to the store the recalculation it triggers runs there too; no
new spec text is needed for that either. This plan is implementation-only.

## Key decisions

1. **Recalculation moves inside the reducer, same as `patch_task`/`create_task` already
   do.** `try_record_stop` (on flip), `subagent_stop`/`subagent_clear` (on drain-that-
   flips), `shell_stop` (ditto), and `record_user_prompt_submit` (on Review→Running)
   call `recalculate_epic_chain` themselves when they change `status`, inside the same
   transaction. The service layer's existing `if outcome == Flipped { recalculate_epic_for_task }`
   callers (`src/service/tasks/crud.rs`) are **not changed** — on a store-backed board
   they become a second, harmless, idempotent recalculation, exactly the relationship
   `patch_task`-driven status moves already have today (the service always recalculates
   after a status-changing patch, and the store's `patch_task` reducer also recalculates
   internally). Not a new pattern; extending the existing one.

2. **A reducer returns no value. For a known row (not a generated id), the answer is a
   post-transaction read of that same row, not id-matching.** Every one of these twelve
   methods acts on a task that already exists and whose id the caller already has — this
   is simpler than `create_task`'s problem (no id to match on) or `create_epic`'s. The
   `_then` callback's `ctx.db` reflects state after the transaction commits
   (`sdk_connector.rs::create_task`'s doc comment), so `ctx.db.tasks().id().find(id)`
   after the call answers with the row's final `live_subagents`/`live_shells`/`status`/
   `stop_pending` — no subscription widening needed, because a task with a live hook
   firing against it is by definition one this board is already running, hence already
   subscribed.

3. **Ambiguous branches are resolved by refusing, not by guessing — except one pair that a
   post-read genuinely cannot separate, which stays advisory.** `try_record_stop` and
   `record_user_prompt_submit` guard on `status` the same way their SQL does; when the
   guard fails (task not `Running`, or not in `{Running, Review}`), the reducer returns
   `Err` rather than `Ok(())` with nothing changed. That turns "did nothing because the
   precondition failed" into an ordinary `ReducerOutcome::Refused`, read as
   `StopOutcome::NoOp` / `UserPromptOutcome::NoOp`.

   That refusal removes the ambiguity for `try_record_stop`'s Flipped-vs-Deferred (once the
   guard holds, they write genuinely different rows — `status` differs). It does **not**
   remove it for `record_user_prompt_submit`'s Resumed-vs-Refreshed: an adversarial review
   of this plan caught that `apply_to(Review)` and `apply_to(Running)` write the identical
   field set (`status = Running`, `sub_status = active`, `last_pre_tool_use_at =
   activity_at`), so the committed row is byte-for-byte the same whichever branch fired —
   there is no column to read back that tells the two apart. Splitting into two reducers
   doesn't help either: the client still calls one method unconditionally and needs one
   answer.

   The resolution leans on decision 1: `record_user_prompt_submit`'s reducer decides
   `resumed` (prior status was `Review`) from its own unambiguous, race-free view and
   recalculates the epic itself, right there, when `resumed` is true — that is *already*
   correct regardless of what the client goes on to report. So `ReducerWriter` classifies
   Resumed vs. Refreshed for the CALLER's benefit only, from a cheap **pre-read**
   (`self.reads.get_task(id)`, the same `BoardReads` handle already held for the claim's
   candidate-picking) taken before the reducer call: prior status `Review` → `Resumed`,
   else → `Refreshed`, refused → `NoOp`. This is advisory, not authoritative — a stale
   pre-read (another write landing in the gap between the read and the call) can only make
   `record_hook_event`'s existing `if outcome == Resumed { recalculate_epic_for_task }`
   fire an extra idempotent recalculation or skip a redundant one; the actual epic status is
   never wrong, because the reducer's own server-side recalculation already made it right.

   `mark_pr_learnings_gate_shown`'s bool ("did *this* call set it") has no such gap — it is
   exactly the `try_claim_backlog_task` shape — refuse when already set, apply when not —
   so it reuses `ReducerOutcome::won()`, not a read-back at all.

4. **Event time, not write time — and why using the client's clock for it is safe here,
   not merely convenient.** `last_pre_tool_use_at`, `last_notification_at`, and
   `stop_pending_at` are the CLIENT's clock (the instant the hook fired), passed as
   reducer arguments — mirroring how `created_at` is the client's clock elsewhere.
   `updated_at` alone is `now(ctx)` (the store's clock), because it is bookkeeping about
   the write, not a fact about the agent.

   This is not optional even after the move to a store: `HookUserPromptSubmit`'s guidance
   is explicit that the tie-break between a deferred `Stop` and the prompt superseding it
   compares EVENT times, and that deriving the comparison from write/commit order instead
   "inherits the race the rule is trying to resolve" — using the STORE's clock
   (`ctx.timestamp`, assigned when the server processes the call) would reintroduce exactly
   that hazard, since network queueing at the server can reorder commits relative to when
   the hooks actually fired, the same way OS scheduling could on one machine.

   The remaining question the review raised is cross-HOST clock skew: could
   `stop_pending_at` (written by one host's `try_record_stop`) and the prompt time it's
   compared against (written by a different host's `record_user_prompt_submit`) come from
   two machines whose clocks disagree? No — a task's tmux session, and therefore every hook
   firing against it, lives on exactly one machine for the task's whole running lifetime
   (`host.allium`'s locality rules; a claim from a second host is refused). So every
   timestamp in this comparison, for a given task, is always the SAME host's clock — the
   identical guarantee the single-machine SQLite version had, carried over rather than
   newly assumed.

5. **`subagent_start`/`shell_start`'s `i64` return (the live count) is informational only
   — no caller branches on it** (`src/service/tasks/crud.rs` discards it; only
   `src/db/tests/subagents.rs`/`shells.rs` assert it, against the *local* SQL path, which
   is untouched). The store path answers it with a plain post-transaction read of
   `live_subagents`/`live_shells`, no ambiguity.

6. **`ReducerCaller` gets bespoke return types per method, following the existing
   `create_task -> Result<TaskId>` / `create_epic -> Result<i64>` precedent** rather than
   forcing everything through `ReducerOutcome`. `awaiting_answer` is generalised to a
   payload type parameter (it is hard-coded to `ReducerOutcome` today); `ReducerOutcome`
   stays the currency for the methods that only need applied/refused, exactly as now.

7. **Module tables `task_shells`/`task_subagents` have no primary key, unlike their SQLite
   counterparts.** `task_subagents`/`task_shells` in SQLite carry `PRIMARY KEY (task_id,
   agent_id)`/`(task_id, shell_id)` (migrations v81/v85), which is what makes `INSERT OR
   REPLACE` a safe, idempotent upsert. The module's tables (`spacetime/module/src/lib.rs`)
   have only a plain btree index on `task_id` — an adversarial review of this plan flagged
   that `delete_agent_state_for`, the existing precedent decision 7 first cited, never has
   to enforce that per-agent/per-shell uniqueness: it unconditionally deletes every row for
   a `task_id` and proves nothing about correctly replacing one. So `subagent_start` and
   `shell_start` must each explicitly find-and-delete-by-value the row matching
   `(task_id, agent_id)` / `(task_id, shell_id)` — not just the session fence — **before**
   inserting the fresh one, or repeated starts for the same still-live agent/shell
   accumulate duplicate rows, inflate the live count, and can permanently strand a
   `stop_pending` task that can now never observe the count reach exactly zero.

## Work

Spec/tests-before-code per method, in this order:

1. `spacetime/module/src/lib.rs`: twelve reducers + a shared `apply_pending_stop_if_drained`
   helper (mirrors `src/db/queries/mod.rs`'s), plus the module's own `#[cfg(test)]`
   coverage (same style as the existing `TaskPatch` tests near the bottom of the file).
2. Regenerate bindings: `./scripts/regenerate-spacetime-bindings.sh`.
3. `src/sync/writes.rs`: extend `ReducerCaller` with the twelve methods (bespoke return
   types per (5) above) and generalise `awaiting_answer`. Extend `SdkReducerCaller`
   (`src/sync/sdk_connector.rs`) to call them and decode the post-transaction row.
4. `src/sync/writes.rs`: extend `ReducerWriter`'s `impl SharedWriter` with the twelve
   methods, translating domain args to `bindings` rows/patches and `ReducerCaller`
   answers back to `SubagentDrain`/`ShellDrain`/`StopOutcome`/`UserPromptOutcome`/`i64`/
   `bool`/`()`.
5. `src/db/mod.rs`: add the twelve methods to `SharedWriter`; update the
   `SHARED_WRITES_ARE_COMPLETE` doc comment to drop "Agent session state" from what's
   still unrouted. The constant itself stays `false` (feed/watcher task #4907 also gates
   it).
6. `src/db/queries/{tasks,subagents,shells}.rs`: add the `if let Some(writer) = ...`
   guard to each of the twelve, matching the existing style exactly.
7. `src/db/tests/shared_writer.rs`: extend `RecordingWriter` with the twelve methods and
   fold them into `every_routed_mutation_reaches_the_writer` / `no_routed_mutation_leaves_a_local_row`
   / `a_refusal_never_falls_back_to_the_local_store`.
8. `src/sync/tests/writes.rs`: extend `RecordingCaller`/`Sent` with the twelve methods;
   add tests for `ReducerWriter`'s decoding (flip vs. defer vs. refused-as-NoOp from the
   post-read; resume vs. refresh vs. refused-as-NoOp from the pre-read; won vs. refused for
   the PR gate). The resume/refresh tests need a seeded `BoardReads` (see
   `writer_over`/`SharedRows` in the existing file) so the pre-read has something to see.
9. `allium:weed` pass over `agent-health.allium` and `epics.allium` against the new code;
   fix whatever it finds before calling this done.

`db::SHARED_WRITES_ARE_COMPLETE` is not flipped by this task.
