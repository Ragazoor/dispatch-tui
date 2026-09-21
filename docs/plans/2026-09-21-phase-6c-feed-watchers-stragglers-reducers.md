# Phase 6c: route feed upsert, watchers and the last stragglers through reducers

Task #4907. Routes the remainder of `db::SHARED_WRITES_ARE_COMPLETE`'s "what is
still unrouted" list:

- Feed ingestion: `upsert_feed_tasks`, `upsert_feed_tasks_additive`,
  `delete_stale_subtree_feed_tasks`, `create_repo_group_sub_epic`,
  `create_managed_role_epic`.
- Task watchers: `create_task_watcher`, `delete_task_watcher`,
  `delete_watches_of_target`, `delete_watches_by_watcher`.
- Stragglers: `batch_patch_sub_status`, `respawn_phoenix_successor`.
- The host registry decision (open in the flag's doc comment, not a leftover):
  resolved with the user and recorded in `host.allium`/`sync.allium` before
  this plan was written. `ensure_host_identity`/`rename_host`/
  `adopt_user_identity` stay local and are **not** added to `SharedWriter` —
  they are local identity facts (the design doc's permanent local exception),
  not shared-table writes. What was missing is a mirror: a new `register_host`
  reducer, pushed on `UserIdentitySettled` (every connect/reconnect) and on a
  live `RenameHost` while connected. This plan implements that mirror as
  separate plumbing, not a `SharedWriter` method — see Decision 6.

`db::SHARED_WRITES_ARE_COMPLETE` flips to `true` at the end of this task.
#4906, #4908 and #4910 (the other three items its doc comment lists as gates)
are already done — confirmed via `get_task` before starting this plan — so
this is the last gate.

No domain rule changes to feed ingest, watcher or phoenix-respawn behaviour:
`feeds.allium`, `tasks.allium`'s watcher rules and the phoenix-respawn spec
text are unchanged. Same end state, new backing, same as Phase 6b.

## Key decisions

1. **`RemovedFeedTask` cannot be read back the way `create_task`'s id can, and
   the fix is predict-then-verify, not a new table.** A SpacetimeDB reducer
   returns no value (#809), and a *deleted* row is gone from `ctx.db` by the
   time any callback reads it — unlike a created row, there is no later state
   to match against. A tombstone/receipt table would solve this exactly but
   is new schema for a single call site; a client-side prediction alone
   (`the rows I can currently see that match the stale predicate`) risks
   **destructively** tearing down a worktree for a task the reducer did NOT
   actually delete, if a concurrent write raced the prediction — feed
   polling is not yet host-scoped (Phase 7), so two hosts running the same
   feed concurrently is not a theoretical case here.

   The resolution: `ReducerWriter` reads its own already-subscribed view
   (`self.reads`, the same handle Phase 6b's pre-read decisions use) for the
   candidate stale rows **before** calling the reducer, using the identical
   predicate the reducer applies (`epic_id` scope, `external_id` set and not
   in the keep list). After the reducer's `_then` fires, it checks each
   candidate against the **post-commit** `ctx.db.tasks()` state and reports
   only the ones now **confirmed absent**. A candidate that turns out to
   still exist (raced) is silently dropped from the report rather than torn
   down — never a false positive, only a possible false negative (a missed
   teardown, which is the existing "best-effort, warn on failure" bargain
   `cleanup_removed_feed_tasks` already documents, not a new risk).

2. **Epic find-or-create is race-free by construction, with no uniqueness
   index needed.** SpacetimeDB reducers execute one at a time; there is no
   concurrent second call to interleave with a check-then-act sequence,
   unlike SQLite's multi-process writers. `create_repo_group_sub_epic` and
   `create_managed_role_epic` become: look up by the domain key
   (`parent_epic_id, title, origin` / `parent_epic_id, feed_role`), unarchive
   or return if found, insert otherwise. The `ON CONFLICT` retry arm in the
   SQLite version has no reducer equivalent to port — there is nothing to
   retry.

3. **Both epic creators must stamp `created_by`, which the SQLite version
   never has.** `created_by` is a module-only column (no SQLite equivalent —
   `Epic` has no `owner` at all, so it's the only way `own_creations`
   finds a fresh epic). The per-epic subscription query is `WHERE id =
   {followed_epic}`, which does **not** cover a brand-new *child* epic's own
   row — only `own_creations` (`WHERE created_by = {identity}`) does. Without
   stamping it, the id-read-back path (`ReducerCaller`, mirroring
   `create_epic`'s `matches_created_epic`) cannot see the row it just
   created or found, ever, on any board. `ReducerWriter` stamps the
   connection's own settled identity, the same `require_identity(...)` call
   `create_epic` already makes.

   **Not fixed here, and flagged rather than silently left:** a *followed*
   epic's board does not automatically see its own sub-epics either, since
   the subscription query for a followed epic covers that epic and its
   *tasks*, not its child epics. That is a pre-existing gap in the Phase 5
   subscription shape, not something this routing task should widen scope to
   fix. Recorded as an open question below.

4. **Feed reducers do not recalculate epic status internally.** Unlike
   `create_task`/`patch_task`, every feed call site already calls the
   *already-routed* `recalculate_epic_status` explicitly afterward
   (`recalculate_epic_status_after_feed`, called from `role_routed.rs`,
   `grouped.rs` and `cycle.rs` after every upsert/delete). Duplicating the
   recalculation inside the new reducers would only be a second, harmless,
   idempotent call — but there is no caller today that needs it, so it is
   left out, matching this plan's "no domain rule changes" scope.

5. **`respawn_phoenix_successor`'s id read-back reuses `create_task`'s
   matching pattern**, with `predecessor` folded in as an extra match field
   where useful for disambiguation (`created_by`/`created_at`/`title`/
   `repo_path`/`owner` already make a tie exceedingly unlikely, and a tie
   here is exactly as benign as `create_task`'s — "the caller's, either way").
   Its caller (`PhoenixRespawn` in `crud.rs`) already calls
   `recalculate_epic` separately on success, so — as with feed ingest — no
   internal recalculation is added.

6. **The host registry mirror is NOT a `SharedWriter` method.** Every other
   method in this task follows the established `if let Some(writer) =
   self.shared_writer() { return writer.foo(...).await } else { <local SQL>
   }` shape — the write happens in exactly one place depending on whether a
   store is attached. `register_host` cannot follow that shape: the local
   write (`ensure_host_identity`/`rename_host`/`adopt_user_identity`) must
   keep happening unconditionally, connected or not — it is the durable
   local credential, not a shared row with one copy. So this is wired as
   separate plumbing that runs *in addition to* the local write, only when a
   connection exists: called from wherever `UserIdentitySettled` fires
   today (the sync connection's identity-settle handling) and from
   `rename_host`'s call site when a writer is attached. See
   `sync.allium: RegisterHostOnConnect` / `RegisterHostOnRename` for the
   spec (already written). Best-effort — a failed push does not fail the
   rename or the connection.

7. **Watchers and `batch_patch_sub_status` need no special handling.** Four
   simple CRUD-shaped reducers over `task_watchers`
   (`create_task_watcher`/`delete_task_watcher`/`delete_watches_of_target`/
   `delete_watches_by_watcher`), all `Result<(), String>`, no id read-back
   needed (none return anything but success/failure). `batch_patch_sub_status`
   takes the whole `Vec<(TaskId, SubStatus)>` in one reducer call — same "one
   reducer, whole-or-nothing" shape `TaskCrud`'s own doc comment already
   promises — and, per `TaskServiceApi`'s own doc comment, carries no
   `recalculate_epic_status` obligation at all (it's derived board state, not
   a status/epic-linkage change).

## Open questions (not blocking this task)

- A followed epic's own sub-epics are not covered by the current
  subscription query shape (Decision 3's flagged gap). Worth a task once
  noticed in practice; out of scope here.
- Cross-host feed polling races (Decision 1) are exactly the case Phase 7
  ("host-scope polling and feeds") exists to close. This plan's predict-
  then-verify approach is safe under that race today; Phase 7 removes the
  race rather than this plan needing to.

## Work

Spec/tests-before-code per group, in this order:

1. `spacetime/module/src/lib.rs`: reducers for feed upsert (3), epic
   creators (2), watchers (4), `batch_patch_sub_status`,
   `respawn_phoenix_successor`, `register_host` — plus the module's own
   `#[cfg(test)]` coverage (existing style near the bottom of the file).
   A new `FeedTaskUpsertItem`-shaped wire struct (stripped `Task` fields:
   `external_id, title, description, repo_path, status, base_branch, tag,
   labels, sort_order, url, url_type, wrap_up_mode`) replaces the SQLite
   `ON CONFLICT DO UPDATE SET` — insert-or-update-preserving-user-fields is
   spelled out per item in the reducer body, reproducing exactly which
   fields the SQLite version updates on conflict (title, description, tag,
   labels, sort_order, url/url_type with the "existing non-null wins"
   precedence) and which it preserves (status, sub_status, repo_path,
   base_branch, wrap_up_mode, completed_at).
2. Regenerate bindings: `./scripts/regenerate-spacetime-bindings.sh`.
3. `src/sync/writes.rs` + `src/sync/sdk_connector.rs`: extend `ReducerCaller`/
   `SdkReducerCaller` with the eleven methods. Feed upsert/stale-delete get
   the predict-then-verify wrapper (Decision 1); the epic creators get
   `created_by` stamping + `matches_*` id read-back (Decision 3); the rest
   are direct `answered_call!`-shaped passthroughs.
4. `src/sync/writes.rs`: extend `ReducerWriter`'s `impl SharedWriter`.
5. `src/db/mod.rs`: add the ten `SharedWriter` methods (everything but
   `register_host`, per Decision 6); update `SHARED_WRITES_ARE_COMPLETE`'s
   doc comment to drop every item this task closes, and flip the constant to
   `true`.
6. `src/db/queries/{tasks,epics}.rs`: add the `if let Some(writer) = ...`
   guard to each of the ten, matching the existing style.
7. Host registry plumbing (Decision 6): `register_host` on `ReducerCaller`/
   `SdkReducerCaller`; call sites wired at the identity-settle point and
   `rename_host`'s call site (wherever `HostStore::rename_host` is invoked
   with a writer attached) — not through `SharedWriter`/`db/queries`.
8. `src/db/tests/shared_writer.rs`: extend `RecordingWriter` with the ten
   methods; fold into the existing coverage tests
   (`every_routed_mutation_reaches_the_writer` /
   `no_routed_mutation_leaves_a_local_row` /
   `a_refusal_never_falls_back_to_the_local_store`).
9. `src/sync/tests/writes.rs`: extend `RecordingCaller`/`Sent`; add tests for
   the predict-then-verify decoding (confirmed-absent vs. raced-still-present
   vs. reducer failure) and the epic-creator id read-back (found vs. created,
   `created_by` stamped).
10. `allium:weed` pass over `feeds.allium`, `sync.allium`, `host.allium`,
    `tasks.allium` (watchers) against the new code; fix whatever it finds.
11. Full verify command; confirm `db::SHARED_WRITES_ARE_COMPLETE = true`
    compiles and every existing store-gated test still passes.
