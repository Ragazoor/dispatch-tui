# MCP Tool Design

How dispatch writes tools for agents. The references are Anthropic's "Define tools" page (platform.claude.com, under
agents-and-tools) and its "Writing tools for agents" engineering post
(anthropic.com/engineering).
The mechanical steps for adding a tool are in [how-to.md](how-to.md); the rules
a test can hold are in `docs/specs/mcp-task-tools.allium` ("Tool descriptions"
and "Tool definitions follow the authoring standard"). This page is the *why*
and the checklist.

## Checklist for a new tool

1. **Design by capability, not by endpoint.** One tool per thing an agent wants
   to get done. If the operation is an update over existing state, extend the
   update tool; do not add a delete or reset twin. Avoid overlapping tools —
   they cost the agent a choice and no benefit. Watch for pairs that are one
   capability: a single and a batch form, an on and an off, a get and a set of
   the same config. Keep create and update apart: their required fields differ.
2. **Agents are first-class users.** A capability the TUI offers needs an MCP
   tool too.
3. **Describe it like a briefing for a new colleague.** At least three
   sentences: what it does, when to use it (and when to reach for a neighbour
   instead), what it returns or does not return. The floor is enforced by
   `every_tool_description_meets_the_detail_floor`; the spec lists what *not* to
   put there (next steps, refusal reasons, code names).
4. **Describe every argument.** Say what it means and how it changes the call.
   Use unambiguous names (`epic_id`, not `epic`). A shared argument uses the one
   shared wording. Enforced by `every_argument_carries_a_description`.
5. **Let the schema carry constraints.** Use `enum` for closed sets, `required`
   for mandatory fields, and prefer required arguments over silent fallbacks.
6. **Declare an access class.** `read_only`, `write`, `idempotent` or
   `destructive` goes next to the name in `mcp_tools!`. It becomes the MCP
   `annotations` hints (`readOnlyHint`, `destructiveHint`, `idempotentHint`,
   `openWorldHint`, `title`). Pick `destructive` only when no other call undoes
   it.
7. **Return high-signal text.** Names and titles beside ids. No internal
   handles. Put the *next step* in the response, where it arrives just in time,
   not in the description.
8. **Bound the response.** Give list tools filters and a `limit` with a default.
   When truncating, say how to narrow the request.
9. **Write errors that fix the call.** Say what was wrong and what a valid call
   looks like. Use the named codes in `src/mcp/handlers/types.rs`. No raw
   internals.
10. **Test with real agent behaviour.** The per-task trajectory log records every
    call with arguments and result. Read it to find confusion, repeated errors
    and redundant calls, then edit the description and measure again. Small
    wording changes move behaviour.

## Audit of the current tools (task 45068)

Checked every tool in `mcp_tools!` against the guidance above.

| Practice | State | Action |
|---|---|---|
| Detailed descriptions (3+ sentences) | Met. Enforced by test. `get_task` and `record_learning` are one sentence on purpose (spec). | None |
| Every argument described | Met for all 25 tools. | Added a test so it stays met |
| Closed sets as `enum` | Met (`status`, `tag`, `wrap_up_mode`, `url_type`). | None |
| Consolidation | **Partly met.** Three pairs overlapped: `update_task`/`update_tasks`, `subscribe_to_task`/`unsubscribe_from_task` (merged into `watch_task`), `get_managed_feed_config`/`set_managed_feed_config`. `create_task` and `update_task` stay apart: their required fields differ, and an upsert turns a wrong id into a new task. | Tasks 45071, 45072, 45073 |
| Search over list-everything | **Gap.** `list_tasks` and `list_epics` have filters but no text query. One search tool across tasks and epics was rejected: they return different shapes. | Task 45074 |
| Namespacing | Met by the server. Agents see `mcp__dispatch__<tool>`; names inside it are verb plus noun. | None |
| Next step in the response, not the description | Met (`wrap_up` → `exit_session`). | None |
| Tool annotations | **Missing on every tool.** | Added. `mcp_tools!` now takes an access class per tool |
| Authoring guidance in the repo | **Missing.** `how-to.md` had the mechanics but no design guidance. | This page; linked from `how-to.md` and `CLAUDE.md` |
| `input_examples` <!-- allow-phantom-symbol: Anthropic API field, not ours --> | Not used. The MCP protocol has no field for it; a worked example in a description is banned by the spec. | None |
| `response_format` <!-- allow-phantom-symbol: Anthropic guidance, not ours --> (concise/detailed) | Not offered. List tools already return one compact row per item and `get_*` return the full view. | Revisit if trajectories show agents over-fetching |
| Evaluation against real tasks | Not automated. The trajectory log is the source. | Follow-up if wanted |
