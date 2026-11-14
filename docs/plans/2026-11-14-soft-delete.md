# Soft delete (task 99036)

Deleting a task keeps a read-only archive record, queryable over MCP.
Agreed with the operator on 2026-11-14.

## Decisions

- **Separate table, not a status.** `archived_tasks` holds a snapshot. The
  `tasks` table and every board read stay as they are; the retired `archived`
  task status stays retired (old-store rows carrying it are still dropped).
- **Delete is otherwise unchanged.** Done-gate, confirmation, teardown
  (window, worktree, branch), feed retirement and every side effect run as
  today. The snapshot is written in the same reducer as the row delete.
- **Which removals archive.** `delete_task`, `batch_delete` (plain tasks and
  epic subtrees), `delete_epic` subtrees, and the super-admin unseen-item
  delete (it runs the epic-subtree delete). Not archived: the feed stale
  delete (never-worked items) and the empty repo-group cleanup (no tasks).
- **Record fields.** The task's own descriptive fields (id, title,
  description, repo_path, base_branch, tag, labels, url, url_type, plan_path,
  external_id, created_at, completed_at, created_by, owner) plus: archived_at,
  archived_by, epic_id, epic_title, agent_name (agents are freed on delete),
  workspace_id, private_holder (the epic chain's private holder at archive
  time, "" for none). Live-session columns are not kept.
- **Visibility is frozen at archive time.** Epic-less: owner only. In an epic:
  a member of `workspace_id`, and `private_holder` empty or the caller. The
  operator sees all.
- **Kept forever.** No expiry, no purge, no restore. The id is never reused.
- **Learnings stay linked.** `source_task_id` keeps the archived id; retrieval
  and verdict rows are kept. (Feed stale deletes still detach and cascade.)
- **Read path.** Boards do not subscribe to the archive. Two procedures read it
  on demand: `archived_task(id)` and `archived_tasks(filter)`.
- **MCP.** `get_task` falls back to the archive and marks the record archived.
  `list_tasks` takes `archived: true` with the usual filters. `resume_task`,
  `dispatch_task`, `update_tasks` on an archived id refuse with "archived".
- **Backup/restore.** Snapshots carry the new table; the task id ceiling counts
  archived ids so a restore never reissues one.

## Steps

1. Spec: `core.allium` (ArchivedTask), `tasks.allium` (DeleteTask,
   BatchDelete), `epics.allium` (DeleteEpic), `learnings.allium`,
   `mcp-task-tools.allium`, `spacetime-seed.allium`.
2. Failing tests: module-tests for each delete path writing a record and
   keeping learnings; conformance tests for `MemoryReducerCaller`; MCP handler
   tests for get/list/refusals; snapshot/restore round-trip.
3. Module: table, write helper, procedures, visibility; bump `RULES_VERSION`
   and `BOARD_RULES_VERSION`; rebuild wasm; regenerate bindings.
4. Host: mirror in `src/sync/memory_caller/`, store API for the reads, MCP
   handlers, snapshot/restore.
5. Weed, verify, wrap up.
