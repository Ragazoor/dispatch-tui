# 0002. MCP tool names are `<noun>_<verb>`, one tool per capability

## Status

Accepted, 2026-11-16 (task 99256). The renames themselves are follow-up
tasks, one per surface, in the epic named at the end.

## Context

Dispatch's MCP tools grew one at a time and are named `<verb>_<noun>`
(`get_task`, `list_tasks`, `create_epic`), with exceptions (`wrap_up`,
`managed_feed_config`, `read_context`). Agents see them as
`mcp__dispatch__<tool>`.

Kognic's AI Core team settled two ADRs for every MCP server behind its
gateway (`kognic-internal/ai-core-team`, `docs/adr/platform/`):

- **2026-11-10, tool names are `<target>_<noun>_<verb>`.** The noun comes
  first so related tools sort and group together in tool lists and tool
  search. The noun is singular, even when the tool returns many. Noun and
  verb are one lowercase word each, with `-` inside a word if needed. The
  verb names the capability, not the risk; the annotations carry the risk,
  so they must be true, and a tool that reads in one call and writes in
  another should be split. No aliases for old names: two names for one tool
  double the list the model reads.
- **2026-11-06, what we build tools for.** One tool per capability, not per
  endpoint and not per workflow.

Dispatch is not behind that gateway, and that ADR's non-goals exclude it. We
adopt the same rule anyway, so an agent that uses both servers sees one
pattern. The server name already gives the target, so dispatch registers
`<noun>_<verb>` and never a `dispatch_` prefix.

The trajectory logs (474 `list_tasks` calls since 2026-10-01) also show that
the main read tool works badly:

- There is no default `limit`. A call with no arguments returned 258 KB; the
  90th percentile reply is 5.5 KB.
- Each row carries a 200-character goal excerpt with its line breaks, so one
  row spreads over several lines.
- 98 calls got a bare "No tasks found", which does not say which filter
  emptied the list. 33 of them passed no `epic_id` or `repo_paths`, so for a
  dispatched agent the hidden scope to its own epic did the filtering.
- `list_epics` has the same gap: a call with no arguments returned over
  20 KB, 26 times.
- `get_task` (1805 calls) and `list_tasks` (797) are two tools for one
  capability, reading tasks.

## Decision

1. **Names are `<noun>_<verb>`** and match `^[a-z0-9-]+_[a-z0-9-]+$`. The
   noun is singular. There is no exception like the gateway's `overview`.
2. **Read verbs.** `search` reads a collection: filters give a list, an id
   gives one item in full. `read` reads one fixed thing, such as a config
   value or the caller's context.
3. **One tool per capability.** `get_task` and `list_tasks` merge into
   `task_search`, as `get_epic` already merged into `list_epics`.
4. **Each tool has one honest access class.** A tool that reads with no
   arguments and writes with some is split into a `_read` and a `_set`. A
   delete flag on an update tool becomes its own `_delete` tool. This
   reverses the merges recorded in `docs/mcp-tool-design.md` for tasks 45073
   and 53252.
5. **`task_search` fixes the list problems.** It has a default `limit` of
   50, and its sort order is stated, since the order decides which 50 an
   agent sees. It has no silent epic scope: the caller passes `epic_id` (an agent
   finds its own epic in its prompt). Each row is one line, with the goal cut
   to one short line. An empty result names the filters it applied. The
   `task_id` form returns the full view `get_task` returns today, archived
   tasks and the Verify command line included.
6. **Old names get a helpful error, not an alias.** An old name is not in
   `tools/list`. A call to it gets an unknown-tool error that names the new
   tool, so a running agent or an installed skill with the old name recovers
   in one step.

### The mapping

| Now | New |
|---|---|
| `get_task`, `list_tasks` | `task_search` |
| `create_task`, `update_tasks`, `dispatch_task`, `resume_task` | `task_create`, `task_update`, `task_dispatch`, `task_resume` |
| `wrap_up`, `exit_session` | `session_wrap-up`, `session_exit` |
| `create_epic`, `list_epics`, `update_epic` | `epic_create`, `epic_search`, `epic_update` |
| `create_work_package`, `list_work_packages`, `update_work_package` | `work-package_create`, `work-package_search`, `work-package_update`, plus `work-package_delete` split out |
| `record_learning`, `query_learnings`, `rate_learning`, `delete_learning` | `learning_record`, `learning_search`, `learning_rate`, `learning_delete` |
| `set_verify_command` | `verify-command_set` |
| `set_review_command` | `review-command_read`, `review-command_set` |
| `managed_feed_config` | `feed-config_read`, `feed-config_set` |
| `set_display_name` | `display-name_set` |
| `list_workspaces`, `create_workspace`, `update_workspace`, `set_workspace_member` | `workspace_search`, `workspace_create`, `workspace_update`, `workspace-member_set`, plus `workspace_delete` split out |
| `set_super_admin`, `claim_super_admin` | `super-admin_set`, `super-admin_claim` |
| `list_unseen_items`, `delete_unseen_item` | `unseen-item_search`, `unseen-item_delete` |
| `query_usage`, `read_context`, `list_keybindings` | `usage_read`, `context_read`, `keybinding_search` |

The same default limit and empty-result wording apply to every `_search`
tool that can return a long list (`epic_search` above all).

A follow-up task may change a name in its own surface when the work shows a
better one, as long as it keeps the rules above and updates this table.

## Consequences

- Every skill under `plugin/skills/`, every prompt template, the server
  instructions, the specs and the docs that name a tool change with it. Old
  plans under `docs/plans/` keep the old names; they are dated snapshots.
- Stored learnings and trajectory logs keep the old names. The helpful error
  covers an agent that copies a name from them.
- The operator's own Claude Code rules outside this repo that name
  `mcp__dispatch__<old name>` need a hand edit.
- The tool title in the annotations is built by splitting the name on `_`.
  A hyphenated word must still give a readable title.
- `docs/mcp-tool-design.md` ("Namespacing" and "Consolidation") is updated
  to point here.
