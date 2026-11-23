use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde_json::{json, Value};

use crate::mcp::identity::{CallerIdentity, IdentityError};
use crate::mcp::trajectory;
use crate::mcp::McpState;
use chrono::Utc;

use super::admin;
use super::context;
use super::epics;
use super::keybindings;
use super::learnings;
use super::managed_feeds;
use super::people;
use super::review_command;
use super::tasks;
use super::types::{
    tool_error, JsonRpcRequest, JsonRpcResponse, INVALID_PARAMS, INVALID_REQUEST, METHOD_NOT_FOUND,
};
use super::work_packages;

/// MCP protocol version this server speaks natively.
const SERVER_PROTOCOL_VERSION: &str = "2025-06-18";

/// Older protocol versions we still understand. If a client offers one of these
/// during `initialize`, we echo it back so the session uses that version
/// (graceful downgrade per spec §Initialization).
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2024-11-05"];

// ---------------------------------------------------------------------------
// Tool registry macro
// ---------------------------------------------------------------------------

/// How a tool touches state, declared beside its name in `mcp_tools!` so a new
/// tool cannot be registered without choosing one. Drives the MCP `annotations`
/// behaviour hints that `tools/list` advertises.
#[derive(Clone, Copy)]
pub(super) enum Access {
    /// Changes nothing.
    ReadOnly,
    /// Changes state; repeating the call may change it again.
    Write,
    /// Changes state; repeating the identical call leaves the same state.
    Idempotent,
    /// Removes or closes something no other call restores.
    Destructive,
}

/// Old tool name → the call that replaces it, as `(old, hint)`. The hint reads
/// after "<old> was renamed: ", e.g. `("get_task", "call task_search with task_id")`.
/// An old name is never registered, so it never appears in `tools/list`.
pub(super) const OLD_TOOL_NAMES: &[(&str, &str)] = &[
    ("get_task", "call task_search with task_id"),
    (
        "list_tasks",
        "call task_search. It no longer scopes to your own epic or hides your own task, so pass \
         epic_id, and it returns 50 rows unless limit says otherwise",
    ),
    ("create_task", "call task_create"),
    ("update_tasks", "call task_update"),
    ("update_task", "call task_update with task_ids: [id]"),
    ("dispatch_task", "call task_dispatch"),
    ("resume_task", "call task_resume"),
    ("wrap_up", "call session_wrap-up"),
    ("exit_session", "call session_exit"),
    ("record_learning", "call learning_record"),
    (
        "query_learnings",
        "call learning_search. It returns 10 entries unless limit says otherwise",
    ),
    (
        "rate_learning",
        "call learning_rate. It takes task_id and a ratings list of {learning_id, verdict}",
    ),
    ("delete_learning", "call learning_delete"),
    ("create_epic", "call epic_create"),
    (
        "list_epics",
        "call epic_search. It returns 50 rows unless limit says otherwise",
    ),
    ("update_epic", "call epic_update"),
    ("create_work_package", "call work-package_create"),
    ("list_work_packages", "call work-package_search"),
    (
        "update_work_package",
        "call work-package_update. To delete a package, call work-package_delete",
    ),
    ("set_verify_command", "call verify-command_set"),
    (
        "set_review_command",
        "call review-command_read to read it, review-command_set (command required) to set it",
    ),
    (
        "managed_feed_config",
        "call feed-config_read to read, feed-config_set (at least one field) to change",
    ),
    ("set_display_name", "call display-name_set"),
    ("list_workspaces", "call workspace_search"),
    ("create_workspace", "call workspace_create"),
    (
        "update_workspace",
        "call workspace_update to rename. To delete a workspace, call workspace_delete",
    ),
    ("set_workspace_member", "call workspace-member_set"),
    ("set_super_admin", "call super-admin_set"),
    ("claim_super_admin", "call super-admin_claim"),
    ("list_unseen_items", "call unseen-item_search"),
    ("delete_unseen_item", "call unseen-item_delete"),
    ("query_usage", "call usage_read"),
    ("read_context", "call context_read"),
    (
        "list_keybindings",
        "call keybinding_search. With no arguments it returns an index of namespaces, so pass namespace or query for keys",
    ),
];

/// The error text for a `tools/call` to an unregistered name. A name in
/// `table` gets a second line naming the tool that replaced it.
pub(super) fn unknown_tool_message(name: &str, table: &[(&str, &str)]) -> String {
    match table.iter().find(|(old, _)| *old == name) {
        Some((old, hint)) => format!("Unknown tool: {name}\n{old} was renamed: {hint}"),
        None => format!("Unknown tool: {name}"),
    }
}

/// The MCP `annotations` object for a tool: a title and the four behaviour
/// hints. `openWorldHint` is false everywhere — no tool reaches outside dispatch.
pub(super) fn annotations(name: &str, access: Access) -> Value {
    let title = name
        .split(['_', '-'])
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ");
    json!({
        "title": title,
        "readOnlyHint": matches!(access, Access::ReadOnly),
        "destructiveHint": matches!(access, Access::Destructive),
        "idempotentHint": matches!(access, Access::ReadOnly | Access::Idempotent),
        "openWorldHint": false
    })
}

/// Register all MCP tools in one place. Expands to:
/// - `fn tool_definitions() -> Value` — the JSON tool list
/// - `async fn dispatch_tool(...)` — the match dispatch
/// - `pub const TOOL_NAMES: &[&str]` — flat list of names
macro_rules! mcp_tools {
    (
        $(
            $kind:ident $access:ident $name:literal => $handler:path,
                $desc:literal,
                $schema:tt
        );+ $(;)?
    ) => {
        pub(super) fn tool_definitions() -> Value {
            json!({
                "tools": [
                    $(
                        {
                            "name": $name,
                            "description": $desc,
                            "inputSchema": $schema,
                            "annotations": annotations($name, mcp_tools!(@access $access))
                        }
                    ),+
                ]
            })
        }

        pub(super) async fn dispatch_tool(
            state: &McpState,
            id: Option<Value>,
            identity: &CallerIdentity,
            tool_name: &str,
            args: Value,
        ) -> JsonRpcResponse {
            match tool_name {
                $(
                    $name => mcp_tools!(@call $kind, $handler, state, id, identity, args),
                )+
                other => JsonRpcResponse::err(id, INVALID_PARAMS, unknown_tool_message(other, OLD_TOOL_NAMES)),
            }
        }

        pub const TOOL_NAMES: &[&str] = &[$($name),+];
    };

    (@access read_only) => { Access::ReadOnly };
    (@access write) => { Access::Write };
    (@access idempotent) => { Access::Idempotent };
    (@access destructive) => { Access::Destructive };

    (@call sync, $handler:path, $state:expr, $id:expr, $identity:expr, $args:expr) => {
        $handler($state, $id, $identity, $args)
    };
    (@call async, $handler:path, $state:expr, $id:expr, $identity:expr, $args:expr) => {
        $handler($state, $id, $identity, $args).await
    };
}

// ---------------------------------------------------------------------------
// Shared enum-schema derivations
//
// Each of these backs an MCP schema `"enum"` array in two separate tool
// definitions below. Factored out so the two call sites can't drift from
// each other, not for the (negligible) cost of computing a handful of
// `&'static str`s twice.
// ---------------------------------------------------------------------------

// `pub(super)` because the `task_update` schema is generated from the
// `mcp_args!` field list in `tasks::mod` and calls this from there. The helper
// stays here with its siblings rather than following its caller.
pub(super) fn task_tag_enum_values() -> Vec<&'static str> {
    crate::models::TaskTag::ALL
        .iter()
        .map(|t| t.as_str())
        .collect()
}

fn wrap_up_action_enum_values() -> Vec<&'static str> {
    crate::mcp::WrapUpAction::ALL
        .iter()
        .map(|a| a.as_str())
        .collect()
}

fn task_status_enum_values() -> Vec<&'static str> {
    crate::models::TaskStatus::ALL
        .iter()
        .map(|s| s.as_str())
        .collect()
}

// ---------------------------------------------------------------------------
// Tool definitions & dispatch (generated by mcp_tools!)
// ---------------------------------------------------------------------------

mcp_tools! {
    async destructive "task_update" => tasks::handle_update_tasks,
        "Update one or many tasks. Pass task_ids (one id updates one task) plus the fields to set: \
status, sub_status, title, description, repo_path, plan_path, sort_order, url, url_type, tag, \
epic_id, base_branch, wrap_up_mode, model, auto_run_plan, phoenix, schedule, and bookmarked. At least one field \
must be provided; every field left out is untouched. Every named task is updated whatever its status; \
pick many ids with task_search (epic_id, recursive, status, base_branch), and check its reply for a last line saying rows were cut (it returns 50 unless limit says otherwise). With one id the reply is \
\"Task N updated (fields)\" or an error. With several it is not atomic: one line per task, success \
or failure, then a count.",
        (tasks::update_tasks_schema());

    async write "task_create" => tasks::handle_create_task,
        "Create a new task on the kanban board. Tasks always start in 'backlog' status. Your \
identity as caller is established by the transport, not by an argument — there is no \
caller_task_id to pass. epic_id is required: name the epic this task belongs to, or pass null \
if it is deliberately standalone. Dispatched agents: your own epic is in your prompt, and is \
usually the right answer.",
        {
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Task title"
                },
                "repo_path": {
                    "type": "string",
                    "description": "Path to the repository for this task"
                },
                "description": {
                    "type": "string",
                    "description": "Task description (optional, defaults to empty)"
                },
                "plan_path": {
                    "type": "string",
                    "description": "Absolute file path to the implementation plan (optional)."
                },
                "epic_id": {
                    "type": ["integer", "null"],
                    "description": "The epic this task belongs to. Required — there is no default and the caller's own epic is never assumed. Pass null to create a deliberately standalone task with no epic."
                },
                "sort_order": {
                    "type": "integer",
                    "description": "Display order within column (lower values appear first)"
                },
                "tag": {
                    "type": "string",
                    "description": "Task tag: bug, feature, chore, pr-review, research, fix, dependabot, or release-please. Controls dispatch behavior.",
                    "enum": task_tag_enum_values()
                },
                "base_branch": {
                    "type": "string",
                    "description": "The base branch for rebase and PR operations (e.g. 'main', 'develop')."
                },
                "wrap_up_mode": {
                    "type": "string",
                    "description": "Pre-set the wrap-up action: 'rebase' (rebase onto base_branch), 'pr' (create a PR), or 'done' (mark done immediately).",
                    "enum": crate::models::WrapUpMode::ALL.iter().map(|m| m.as_str()).collect::<Vec<_>>()
                },
                "model": {
                    "type": "string",
                    "description": "The model the task's agent launches on: 'haiku', 'sonnet', 'opus' or 'fable'. A task with no model runs on opus. Defaults to opus.",
                    "enum": crate::models::TaskModel::ALL.iter().map(|m| m.as_str()).collect::<Vec<_>>()
                },
                "auto_run_plan": {
                    "type": "boolean",
                    "description": "When true and a plan_path is set, the dispatched agent implements the plan immediately instead of summarizing it and asking for confirmation first. Use only for a plan already reviewed. Defaults to false."
                },
                "phoenix": {
                    "type": "boolean",
                    "description": "When true, completing this task automatically recreates it as a fresh backlog copy carrying the same settings, and the flag moves to that copy. Without a schedule nothing dispatches the copy, you do; with one, the copy runs on it. Defaults to false."
                },
                "schedule": {
                    "type": "string",
                    "description": "Run this task on a schedule: a five-field cron string read in UTC (minute hour day-of-month month day-of-week), e.g. '0 9 * * 1' for Mondays 09:00 UTC. A running board dispatches it when due, or as soon as one starts if the time passed. A phoenix task repeats on it; any other task runs once, so for a one-off give a cron matching that date."
                },
                "signal": {
                    "type": "boolean",
                    "description": "When true, signal the user that a new task arrived: the new task and every epic on the path to it are marked on the board until the user enters the epic. Everyone who can see the epic when the task is created is marked. Ignored when epic_id is null. Defaults to false."
                }
            },
            "required": ["title", "repo_path", "epic_id"]
        };

    async read_only "task_search" => tasks::handle_task_search,
        "Read tasks on the kanban board, in one of two forms. With task_id and no other argument it \
returns that one task in full, a deleted (archived) one included, and your own task too. Without \
task_id it lists tasks, one line per task, with the filters ANDed. The list has no scope of its \
own: nothing is derived from who calls, so a dispatched agent passes epic_id (its own is in its \
prompt) to see its epic, and its own task is listed like any other. Rows are sorted most recently \
updated first, ties by id descending, and at most 50 are returned unless limit says otherwise: a \
last line gives the count shown and how to narrow. Each row shows the base branch, the url \
under its own type label (PR, Issue, Security Alert or Link, with the number appended where there \
is one), the goal cut to about 80 characters on one line, and the updated and completed times \
(ISO 8601 UTC, - when it has not completed). An empty result names the filters it applied. To count \
tasks finished in a period, filter with completed_after and completed_before: the last change time \
also moves on later edits.",
        {
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "integer",
                    "description": "Return this one task in full (archived tasks too). It takes no other argument: every other argument below is for the list form and is refused alongside it."
                },
                "status": {
                    "description": "List form: filter by status. Single string or array of strings.",
                    "oneOf": [
                        { "type": "string", "enum": task_status_enum_values() },
                        { "type": "array", "items": { "type": "string", "enum": task_status_enum_values() } }
                    ]
                },
                "epic_id": {
                    "type": "integer",
                    "description": "List form: filter to only tasks belonging to this epic. Without it tasks of every epic are listed, whoever you are."
                },
                "repo_paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "List form: filter to tasks whose repo_path is in this list."
                },
                "recursive": {
                    "type": "boolean",
                    "description": "List form: also include tasks of every descendant epic of epic_id. Rejected without epic_id."
                },
                "base_branch": {
                    "type": "string",
                    "description": "The base branch for rebase and PR operations (e.g. main, develop). In the list form, lists only tasks whose base_branch is exactly this."
                },
                "query": {
                    "type": "string",
                    "description": "List form: text search over title and description. Split on spaces; every word must appear (any case, as part of a word) in the title or description. Combines with the other filters."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "List form: return at most this many rows. Defaults to 50. When rows are cut, the last line says how many were shown of how many; raise limit to see more."
                },
                "bookmarked": {
                    "type": "boolean",
                    "description": "List form: true lists only tasks the board's person has bookmarked; false only those they have not. Combines with the other filters. A bookmarked task's row says bookmarked after its title."
                },
                "archived": {
                    "type": "boolean",
                    "description": "List form: true lists archived (deleted) tasks instead of live ones, newest first, with their title, epic, agent and a description preview. epic_id, repo_paths, base_branch, query and limit apply; status, recursive, bookmarked and the four time filters are rejected. Defaults to false."
                },
                "updated_after": {
                    "type": "string",
                    "description": "List form: keep only tasks whose last change time is at or after this time, an ISO 8601 time with a zone, like 2026-11-09T00:00:00Z or 2026-11-09T00:00:00+02:00. After includes the bound, before excludes it, so two windows never overlap. Combines with the other filters."
                },
                "updated_before": {
                    "type": "string",
                    "description": "List form: keep only tasks whose last change time is strictly before this time, an ISO 8601 time with a zone, like 2026-11-09T00:00:00Z or 2026-11-09T00:00:00+02:00. After includes the bound, before excludes it, so two windows never overlap. Combines with the other filters."
                },
                "completed_after": {
                    "type": "string",
                    "description": "List form: keep only tasks whose completion time (the last time the task entered done; a task that never completed is excluded) is at or after this time, an ISO 8601 time with a zone, like 2026-11-09T00:00:00Z or 2026-11-09T00:00:00+02:00. After includes the bound, before excludes it, so two windows never overlap. Combines with the other filters."
                },
                "completed_before": {
                    "type": "string",
                    "description": "List form: keep only tasks whose completion time (the last time the task entered done; a task that never completed is excluded) is strictly before this time, an ISO 8601 time with a zone, like 2026-11-09T00:00:00Z or 2026-11-09T00:00:00+02:00. After includes the bound, before excludes it, so two windows never overlap. Combines with the other filters."
                }
            }
        };

    async write "epic_create" => epics::handle_epic_create,
        "Create a new epic on the kanban board. Pass either workspace_id (a root epic) or \
parent_epic_id (a sub-epic), never both and never neither: a root epic needs a workspace_id (a \
refusal lists yours), and a sub-epic takes its parent's workspace and refuses one. An epic groups \
related tasks and derives its status from theirs, so you do not set an epic's status directly. \
visibility is public (every member of the \
workspace sees it, the default) or private (only you do). Epics carry no repo_path — that lives \
on each subtask, and passing it here is rejected.",
        {
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Epic title" },
                "description": { "type": "string", "description": "Epic description" },
                "sort_order": { "type": "integer", "description": "Display order within column (lower values appear first)" },
                "parent_epic_id": { "type": "integer", "description": "Makes this a sub-epic of the epic with this id, in that epic's workspace, so pass no workspace_id with it. Leave it out for a root epic." },
                "workspace_id": { "type": "integer", "description": "The workspace a root epic lives in: required for a root epic (no parent_epic_id), refused for a sub-epic. A refusal lists the workspaces you belong to." },
                "visibility": { "type": "string", "enum": ["public", "private"], "description": "public: every member of the workspace sees it. private: only you do. Default public." }
            },
            "required": ["title"]
        };

    async read_only "epic_search" => epics::handle_epic_search,
        "List epics on the kanban board, one row each, with each sub-epic's parent id. With no \
arguments it lists the 50 most recently updated epics, newest first; a last line says when rows \
were cut and how to get the rest, and an empty result names the filters it applied. Pass parent_epic_id for that epic's direct children, plus \
recursive=true for its whole subtree. Pass query to search titles and descriptions. Use it to find an epic's ID when you only know its title. \
Pass epic_id instead for one epic in full: its description, status, plan, parent, feed settings \
and a count of its subtasks by status, but not the subtasks themselves — call task_search with \
epic_id for those. Each row shows the epic's updated and completed times (ISO 8601 UTC, '-' when it has not completed). Filter them with updated_after, updated_before, completed_after and completed_before.",
        {
            "type": "object",
            "properties": {
                "epic_id": { "type": "integer", "description": "Return this one epic in full instead of a list. Takes no other argument, the time filters included." },
                "parent_epic_id": { "type": "integer", "description": "List only the sub-epics of this epic." },
                "recursive": { "type": "boolean", "description": "With parent_epic_id, list every descendant rather than only direct children." },
                "query": {
                    "type": "string",
                    "description": "Text search over title and description. Split on spaces; every word must appear (any case, as part of a word) in the title or description. Combines with the other filters."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Return at most this many rows. Default 50. Rows are sorted by last update, newest first. When rows are cut, the last line says how many were shown of how many."
                },
                "updated_after": {
                    "type": "string",
                    "description": "Keep only epics whose last change time is at or after this time, an ISO 8601 time with a zone, like 2026-11-09T00:00:00Z or 2026-11-09T00:00:00+02:00. After includes the bound, before excludes it, so two windows never overlap. Combines with the other filters."
                },
                "updated_before": {
                    "type": "string",
                    "description": "Keep only epics whose last change time is strictly before this time, an ISO 8601 time with a zone, like 2026-11-09T00:00:00Z or 2026-11-09T00:00:00+02:00. After includes the bound, before excludes it, so two windows never overlap. Combines with the other filters."
                },
                "completed_after": {
                    "type": "string",
                    "description": "Keep only epics whose completion time (the last time the epic entered done; an epic that never completed is excluded) is at or after this time, an ISO 8601 time with a zone, like 2026-11-09T00:00:00Z or 2026-11-09T00:00:00+02:00. After includes the bound, before excludes it, so two windows never overlap. Combines with the other filters."
                },
                "completed_before": {
                    "type": "string",
                    "description": "Keep only epics whose completion time (the last time the epic entered done; an epic that never completed is excluded) is strictly before this time, an ISO 8601 time with a zone, like 2026-11-09T00:00:00Z or 2026-11-09T00:00:00+02:00. After includes the bound, before excludes it, so two windows never overlap. Combines with the other filters."
                }
            }
        };

    async idempotent "epic_update" => epics::handle_epic_update,
        "Update an epic in place. Accepts title, description, status, plan_path, sort_order, \
parent_epic_id, and the feed settings feed_command, feed_interval_secs, group_by_repo and \
feed_append_only, and visibility (public or private), and workspace_id, which moves a root epic and its whole tree \
(sub-epics, tasks, their learnings) to that workspace: allowed to a super-admin or an admin of both \
the source and the destination workspace, refused for a sub-epic, and it drops work package links \
into the tree and follows by people outside the destination. Every field left out is untouched; the nullable ones take an explicit null to \
clear. Setting status by hand is rarely right — an epic's status is recalculated from its \
subtasks whenever they change. Re-parenting is cycle-checked and rejected if it would create a \
loop.",
        {
            "type": "object",
            "properties": {
                "epic_id": { "type": "integer", "description": "The epic ID" },
                "title": { "type": "string", "description": "New title" },
                "description": { "type": "string", "description": "New description" },
                "status": { "type": "string", "description": "New status: backlog, running, review, or done", "enum": crate::models::TaskStatus::ALL.iter().map(|s| s.as_str()).collect::<Vec<_>>() },
                "plan_path": { "type": "string", "description": "Path to the plan file" },
                "sort_order": { "type": "integer", "description": "Display order within column (lower values appear first)" },
                "feed_command": { "type": ["string", "null"], "description": "Shell command that emits JSON FeedItems to populate tasks. Pass null to clear." },
                "feed_interval_secs": { "type": ["integer", "null"], "description": "Polling interval in seconds (overrides the default). Minimum 60; a lower value is rejected. Pass null to clear (falls back to the 60s default)." },
                "group_by_repo": { "type": "boolean", "description": "When true, group feed tasks by repository path in the TUI. Pass false to disable grouping." },
                "feed_append_only": { "type": "boolean", "description": "When true, this epic's feed only ever adds tasks: one whose item is missing from a later emission is kept, not removed. For a feed whose source emits events that are never retracted (a log scan), where absence means nothing. Close such a task from the TUI once it is done: deleting it retires its external id under this epic forever, so an append-only feed never recreates it. Pass false to restore the default, where the feed mirrors its source and removes what the emission omits." },
                "parent_epic_id": { "type": ["integer", "null"], "description": "Re-parent this epic under another epic by ID. Pass null to make it a root epic. Cycle detection prevents hierarchical loops." },
                "visibility": { "type": "string", "enum": ["public", "private"], "description": "public: every member of the workspace sees it. private: only its creator does. Changeable by the creator or a workspace admin." },
                "workspace_id": { "type": "integer", "description": "Move this root epic and its tree to the workspace with this id (see workspace_search). Super-admin, or admin of both workspaces." }
            },
            "required": ["epic_id"]
        };

    async write "work-package_create" => work_packages::handle_work_package_create,
        "Create a work package: a named group that links to tasks and epics which already exist, \
wherever they sit in the epic tree, so work spread across several epics can be followed as one \
board. A package only links; it never owns its items, and one item can belong to many packages. Pass \
link_task_ids and link_epic_ids to link items at once; every id is checked first, and an unknown \
one creates nothing. Needs a workspace_id (a refusal lists yours) and links only items in that \
workspace, or tasks on your main board. visibility is public (the default) or private (only you \
see it). Returns the new package's id. To group new work under a parent, use epic_create \
instead.",
        {
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Package title, for example the maintenance window it covers. Must not be blank." },
                "description": { "type": "string", "description": "Free-text notes, for example the plan for the window." },
                "link_task_ids": { "type": "array", "items": { "type": "integer" }, "description": "Ids of existing tasks to link." },
                "link_epic_ids": { "type": "array", "items": { "type": "integer" }, "description": "Ids of existing epics to link. A linked epic shows as one card; its subtasks are not linked one by one." },
                "workspace_id": { "type": "integer", "description": "The workspace the package lives in. Required; a refusal lists the workspaces you belong to." },
                "visibility": { "type": "string", "enum": ["public", "private"], "description": "public: every member of the workspace sees it. private: only you do. Default public." }
            },
            "required": ["title", "workspace_id"]
        };

    async read_only "work-package_search" => work_packages::handle_work_package_search,
        "List work packages with their linked items. Read-only. With no arguments it lists every \
package; pass work_package_id for one. Each package shows its title, description and every linked \
task or epic with its id, status and title. A package has no status of its own: read progress from \
the items. A package's sort order is shown when it has been set. Filter by when a package was last changed with updated_after and updated_before. A package has no completion time, so it has no completed filters.",
        {
            "type": "object",
            "properties": {
                "work_package_id": { "type": "integer", "description": "List only this package. Takes no time filter." },
                "updated_after": {
                    "type": "string",
                    "description": "Keep only packages whose last change time is at or after this time, an ISO 8601 time with a zone, like 2026-11-09T00:00:00Z or 2026-11-09T00:00:00+02:00. After includes the bound, before excludes it, so two windows never overlap. Combines with the other filters."
                },
                "updated_before": {
                    "type": "string",
                    "description": "Keep only packages whose last change time is strictly before this time, an ISO 8601 time with a zone, like 2026-11-09T00:00:00Z or 2026-11-09T00:00:00+02:00. After includes the bound, before excludes it, so two windows never overlap. Combines with the other filters."
                }
            }
        };

    async idempotent "work-package_update" => work_packages::handle_work_package_update,
        "Update a work package in place: rename it with title, rewrite description, set sort_order (lower values show first among the package cards), set visibility (public or private), and change \
its links with link_task_ids, unlink_task_ids, link_epic_ids and unlink_epic_ids. Unlinks apply \
before links, linking an item already linked and unlinking one not linked are both silent, and \
the tasks and epics themselves are never changed. Every id is checked before anything is written, \
and an id named in both a link and an unlink list is refused. Every field left out is untouched, \
and at least one must be given. To delete a package, call work-package_delete.",
        {
            "type": "object",
            "properties": {
                "work_package_id": { "type": "integer", "description": "The work package ID" },
                "title": { "type": "string", "description": "New title. Must not be blank." },
                "description": { "type": "string", "description": "New description." },
                "sort_order": { "type": "integer", "description": "Display order within column (lower values appear first)" },
                "visibility": { "type": "string", "enum": ["public", "private"], "description": "public: every member of the workspace sees it. private: only its creator does." },
                "link_task_ids": { "type": "array", "items": { "type": "integer" }, "description": "Ids of existing tasks to link." },
                "unlink_task_ids": { "type": "array", "items": { "type": "integer" }, "description": "Ids of tasks to unlink." },
                "link_epic_ids": { "type": "array", "items": { "type": "integer" }, "description": "Ids of existing epics to link." },
                "unlink_epic_ids": { "type": "array", "items": { "type": "integer" }, "description": "Ids of epics to unlink." }
            },
            "required": ["work_package_id"]
        };

    async destructive "work-package_delete" => work_packages::handle_work_package_delete,
        "Delete a work package and its links once it has served its purpose. Its linked tasks and \
epics stay where they are, with their status unchanged. Takes only the package id; to rename or \
relink a package, call work-package_update.",
        {
            "type": "object",
            "properties": {
                "work_package_id": { "type": "integer", "description": "The work package ID" }
            },
            "required": ["work_package_id"]
        };

    async write "session_wrap-up" => tasks::handle_wrap_up,
        "Wrap up a running or review task with one of three actions (rebase, done, pr) and return an \
exit token for session_exit; the task stays running until then. \
'rebase' rebases onto base_branch and fast-forwards it (blocks until complete). \
'done' and 'pr' perform no git operations — for 'pr' you run git push and gh pr create yourself \
and pass the resulting URL to session_exit. \
A task tagged pr-review, dependabot or release-please is refused.",
        {
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "integer",
                    "description": "The task ID to wrap up"
                },
                "action": {
                    "type": "string",
                    "enum": wrap_up_action_enum_values(),
                    "description": "'rebase' — rebase onto base_branch and fast-forward it. 'done' — no git ops. 'pr' — no git ops; agent has already run gh pr create. All three return an exit token; the task stays in its current status until session_exit closes it."
                }
            },
            "required": ["task_id", "action"]
        };

    async write "task_dispatch" => tasks::handle_dispatch_task,
        "Dispatch a backlog task by ID, then launch an agent in a tmux session. If the task has \
no worktree directory yet, one is CREATED — from the task's base branch, or from the PR's head \
branch for a review task carrying a PR url. If it already exists it is \
REUSED as-is — its branch, its commits and its uncommitted changes are all still there, and the \
agent starts in them. So a crashed or stalled task is restarted in place: set it back to backlog \
with task_update, then task_dispatch it. Tell the new agent in the task what its worktree \
already holds — it is a new agent and has none of the previous session's context. To wake a \
paused agent WITH its context, use task_resume instead. Waits up to 30 \
seconds for the dispatch to complete; the success text says whether the worktree was created or \
reused, and names it along with the tmux window. If provisioning takes longer, it returns the \
dispatch request's id and state instead, and the dispatch carries on. Errors if the task is not in \
backlog status or the dispatch fails.",
        {
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "integer",
                    "description": "The task ID to dispatch"
                }
            },
            "required": ["task_id"]
        };

    async write "task_resume" => tasks::handle_resume_task,
        "Wake a detached (paused) agent WITH its context. Reattaches the task's worktree, starts \
the Claude session so it continues the agent's previous conversation, and puts the task back to \
running — what pressing space on a detached agent in the TUI does. Prefer this over task_update \
status=backlog plus task_dispatch, which starts a NEW agent that has none of the old context. \
The optional message is delivered as the first input after the session resumes; use it to say why \
the agent was woken. Returns once the board has been asked to resume, not when the session is \
up. Errors with 'no resumable session' when the task has no worktree or is in backlog (use \
task_dispatch for those); errors when its agent is already attached (use SendMessage) or its \
worktree is on another machine.",
        {
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "integer",
                    "description": "The task ID of the detached agent to resume"
                },
                "message": {
                    "type": "string",
                    "description": "Optional first message for the resumed session"
                }
            },
            "required": ["task_id"]
        };

    async write "learning_record" => learnings::handle_record_learning,
        "Record a non-obvious, durable finding in the shared knowledge base, following the /learnings skill's authoring rules.",
        {
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "integer",
                    "description": "ID of the task this learning was discovered during"
                },
                "kind": {
                    "type": "string",
                    "description": "Category: pitfall, convention, preference, tool_recommendation, procedural, or landscape",
                    "enum": crate::models::LearningKind::ALL.iter().map(|k| k.as_str()).collect::<Vec<_>>()
                },
                "summary": {
                    "type": "string",
                    "description": "One-sentence description of what was learned (required, non-empty)"
                },
                "scope": {
                    "type": "string",
                    "description": "Scope this learning applies to: user (global), repo, or epic",
                    "enum": crate::models::LearningScope::ALL.iter().map(|s| s.as_str()).collect::<Vec<_>>()
                },
                "detail": {
                    "type": "string",
                    "description": "Optional extended context or explanation"
                },
                "scope_ref": {
                    "type": "string",
                    "description": "Qualifier for the scope (e.g. repo path, epic ID as string). Omit to auto-derive from the calling task — only set explicitly when recording a learning about a different context than the current task."
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional free-form labels for filtering (e.g. [\"rust\", \"testing\"])"
                }
            },
            "required": ["task_id", "kind", "summary", "scope"]
        };

    async read_only "learning_search" => learnings::handle_query_learnings,
        "Query the knowledge base by semantic similarity to your task's context. Results are \
ranked by embedding similarity plus a scope and upvote boost, and entries below the similarity \
threshold are dropped — so an empty result means nothing relevant was found, not that the store is \
empty. The entries already injected into your prompt are not excluded, so \
expect overlap with those. Call it when something is unclear, before guessing or asking.",
        {
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "integer",
                    "description": "ID of the calling task — determines which repo/project/epic to use for scope boosting"
                },
                "query": {
                    "type": "string",
                    "description": "Optional semantic query string. When omitted, the task's title and description are used as the query."
                },
                "tag_filter": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of tags to soft-boost — learnings with matching tags score higher but untagged entries are not excluded"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default 10, max 50). Results are ranked best first; when exactly this many come back there may be more."
                }
            },
            "required": ["task_id"]
        };

    async write "learning_rate" => learnings::handle_rate_learning,
        "Give feedback on knowledge base entries that were surfaced to you this task. \
Call any time you act on retrieved learnings, and rate every entry you have acted on in one call. \
'helped' upvotes an entry (a usefulness signal that boosts ranking); 'wrong' downvotes it (may go \
negative) — neither changes its status, and there is no human review step. You can only rate a \
learning that was surfaced to you (injected into your prompt or returned by learning_search). \
The call is all or nothing: if any pair is refused, none is written and the error names it.",
        {
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "integer",
                    "description": "ID of the task that acted on these entries"
                },
                "ratings": {
                    "type": "array",
                    "minItems": 1,
                    "description": "One {learning_id, verdict} pair per entry. A learning may appear once.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "learning_id": {
                                "type": "integer",
                                "description": "ID of the knowledge base entry to rate"
                            },
                            "verdict": {
                                "type": "string",
                                "enum": crate::models::LearningVerdict::ALL.iter().map(|v| v.as_str()).collect::<Vec<_>>(),
                                "description": "'helped' — the entry applied and was useful (upvotes it). 'wrong' — the entry misled you or is inaccurate (downvotes it; may go negative). 'not_applicable' — you read an injected entry and it did not fit this task (score unchanged; it tells the store the entry was off-topic here). No verdict changes the entry's status."
                            }
                        },
                        "required": ["learning_id", "verdict"]
                    }
                }
            },
            "required": ["task_id", "ratings"]
        };

    async destructive "learning_delete" => learnings::handle_delete_learning,
        "Delete a learning entry from the knowledge base by ID. Permanent and immediate — there \
is no archive, no soft delete and no human review step, so the entry stops reaching other agents \
at once. Use it for an entry that is wrong or has gone stale, rather than only downvoting it with \
learning_rate; downvoting keeps a wrong entry in circulation. Returns an error if the ID does not \
exist.",
        {
            "type": "object",
            "properties": {
                "learning_id": {
                    "type": "integer",
                    "description": "ID of the knowledge base entry to delete"
                }
            },
            "required": ["learning_id"]
        };

    async read_only "review-command_read" => review_command::handle_review_command_read,
        "Read the command a PR review task runs on its PR, for this machine. It defaults to \
/dispatch:preference-review and shows up in the task's prompt, so a change reaches the next launch, \
not an agent already running. Takes no arguments and changes nothing. The reply names the command \
in effect. To change it, call review-command_set.",
        {
            "type": "object",
            "properties": {}
        };

    async idempotent "review-command_set" => review_command::handle_review_command_set,
        "Set the command a PR review task runs on its PR, for this machine, for example a personal \
skill such as /review-pr. It shows up in the task's prompt, so it applies to the next launch, not \
an agent already running. A blank command restores the default, /dispatch:preference-review. \
Repeating the same call changes nothing. The reply names the command now in effect. To only read \
it, call review-command_read.",
        {
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The single-line review command, e.g. /review-pr. Blank restores /dispatch:preference-review."
                }
            },
            "required": ["command"]
        };

    async read_only "feed-config_read" => managed_feeds::handle_feed_config_read,
        "Read the managed-feed configuration: the PR-reviews and CVE feed commands and their poll \
intervals (in seconds). Takes no arguments and changes nothing. Unset values are reported as \
unset, which is how you tell a disabled feed from one running on the default interval. To change \
them, call feed-config_set.",
        {
            "type": "object",
            "properties": {}
        };

    async idempotent "feed-config_set" => managed_feeds::handle_feed_config_set,
        "Change the managed-feed configuration: the PR-reviews and CVE feed commands and their poll \
intervals (in seconds). Pass at least one field; a call with none is refused, and feed-config_read \
reads the current values. Each field is triple-state: omit to leave unchanged, pass null to clear, \
or pass a value to set. The reply is the resulting configuration. A change re-provisions the \
managed epic tree, so enabling a feed creates its epics immediately. Omit an interval to fall back \
to the default feed interval.",
        {
            "type": "object",
            "properties": {
                "reviews_command": { "type": ["string", "null"], "description": "Script emitting the involved-PR feed. Pass null to clear (disables the reviews subtree). Omit to leave unchanged." },
                "reviews_interval_secs": { "type": ["integer", "null"], "description": "Poll cadence for the reviews feed, in seconds. Minimum 60; a lower value is rejected. Pass null to clear (falls back to the 60s default). Omit to leave unchanged." },
                "cve_command": { "type": ["string", "null"], "description": "Script emitting the security/CVE feed. Pass null to clear (disables the CVE epic). Omit to leave unchanged." },
                "cve_interval_secs": { "type": ["integer", "null"], "description": "Poll cadence for the CVE feed, in seconds. Minimum 60; a lower value is rejected. Pass null to clear (falls back to the 60s default). Omit to leave unchanged." }
            }
        };

    async idempotent "display-name_set" => people::handle_display_name_set,
        "Set your display name: the name the board and other tools show beside what you create, \
instead of the short prefix of your identity. It names the person whose board this agent runs \
under, not the agent. The name is trimmed, must not be blank and can be at most 64 characters; \
a blank or longer name is refused with the reason. Repeating the same call changes nothing. \
Returns the saved name.",
        {
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "The display name, 1 to 64 characters after trimming." }
            },
            "required": ["name"]
        };

    async read_only "workspace_search" => admin::handle_workspace_search,
        "List the workspaces you can see, who is in each and in which role, who the super-admins are, \
and the directory of every person by display name and short identity. Use it to find the \
workspace_id and the person to name in epic_create, workspace-member_set and super-admin_set: \
people are named by display name, and the directory tells two people with one name apart by \
identity. A person sees the workspaces they belong to; a super-admin sees all of them. It \
also says when super-admin_claim would work. It returns no epics, tasks or other content.",
        {
            "type": "object",
            "properties": {
                "workspace_id": { "type": "integer", "description": "Show only this workspace and its members, and leave out the directory of people. Omit to list every workspace you can see." }
            }
        };

    async write "workspace_create" => admin::handle_workspace_create,
        "Create a workspace: a place where people collaborate, holding epics and work packages. Only a \
super-admin may. The new workspace has no members, not even you; add people, yourself included, \
with workspace-member_set. Names are unique across the store, trimmed, and at most 64 characters. \
Returns the new workspace's id and name.",
        {
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "The workspace name, 1 to 64 characters after trimming, not already used by another workspace." }
            },
            "required": ["name"]
        };

    async idempotent "workspace_update" => admin::handle_workspace_update,
        "Rename a workspace. A super-admin or an admin of the workspace may. Renaming to the name it \
already has changes nothing. Returns what changed. To delete a workspace, call workspace_delete.",
        {
            "type": "object",
            "properties": {
                "workspace_id": { "type": "integer", "description": "The workspace to rename, from workspace_search." },
                "name": { "type": "string", "description": "The new name, 1 to 64 characters after trimming, not used by another workspace." }
            },
            "required": ["workspace_id", "name"]
        };

    async destructive "workspace_delete" => admin::handle_workspace_delete,
        "Delete a workspace. Only a super-admin may, and only an empty workspace: one still holding an \
epic or work package, private ones included, is refused, so empty it first (unseen-item_search and \
unseen-item_delete reach private items nobody can see). Deleting removes its memberships and its \
learnings and cannot be undone. Returns what was deleted. To rename a workspace, call \
workspace_update.",
        {
            "type": "object",
            "properties": {
                "workspace_id": { "type": "integer", "description": "The workspace to delete, from workspace_search." }
            },
            "required": ["workspace_id"]
        };

    async idempotent "workspace-member_set" => admin::handle_workspace_member_set,
        "Add a person to a workspace, change their role there, or remove them; one call does one of the \
three. Give role to add the person (if they are not a member) or to change their role (if they are); \
give remove=true to take them out. A super-admin or an admin of the workspace may. Removing a \
person does not touch their epics or packages: their public ones stay for the others, their \
private ones are then seen by nobody until they rejoin or a super-admin deletes them. Returns what \
changed.",
        {
            "type": "object",
            "properties": {
                "workspace_id": { "type": "integer", "description": "The workspace, from workspace_search." },
                "person": { "type": "string", "description": "The person's display name, or their identity (or its first 8 or more characters) when two people share a name. A name more than one person has is refused with the matches." },
                "role": { "type": "string", "enum": ["admin", "member"], "description": "The role to give: admin can manage the workspace's members, member cannot. Adds the person when they are not a member. Give this or remove, not both." },
                "remove": { "type": "boolean", "description": "When true, remove the person from the workspace; they must be a member. Give this or role, not both. False is the same as leaving it out." }
            },
            "required": ["workspace_id", "person"]
        };

    async idempotent "super-admin_set" => admin::handle_super_admin_set,
        "Make a person a super-admin or stop them being one. Only a super-admin may. A super-admin \
manages every workspace and may delete private items nobody can see, but cannot read them. \
Repeating a call that already holds changes nothing. Taking the role from the last super-admin is \
refused. To become the first one yourself, use super-admin_claim. Returns the person's name and \
their new standing.",
        {
            "type": "object",
            "properties": {
                "person": { "type": "string", "description": "The person's display name, or their identity (or its first 8 or more characters) when two people share a name." },
                "super_admin": { "type": "boolean", "description": "true makes the person a super-admin; false revokes it." }
            },
            "required": ["person", "super_admin"]
        };

    async write "super-admin_claim" => admin::handle_super_admin_claim,
        "Make yourself the first super-admin. It works only while the store has none, and then once: \
after that it is refused and a super-admin uses super-admin_set to add more. You are the person \
whose board this agent runs under. Takes no arguments. Returns your new standing.",
        {
            "type": "object",
            "properties": {}
        };

    async read_only "unseen-item_search" => admin::handle_unseen_item_search,
        "List private epics and work packages that nobody can see any more because their creator has \
left the workspace. Only a super-admin sees any; for anyone else the list is empty. Each entry is \
its kind, id, workspace, creator and created date, never a title or content. The only thing to \
do with one is unseen-item_delete.",
        {
            "type": "object",
            "properties": {}
        };

    async destructive "unseen-item_delete" => admin::handle_unseen_item_delete,
        "Permanently delete a private epic or work package that nobody can see, chosen from \
unseen-item_search. Only a super-admin may. An epic is deleted with its sub-epics and tasks, and is \
refused while any task in it is not done. A public item, or an id that does not exist, is refused \
in the same words. It cannot be undone, and you never read what you delete.",
        {
            "type": "object",
            "properties": {
                "kind": { "type": "string", "enum": ["epic", "work_package"], "description": "The kind of item, as unseen-item_search names it." },
                "item_id": { "type": "integer", "description": "The item's id from unseen-item_search." }
            },
            "required": ["kind", "item_id"]
        };

    async idempotent "verify-command_set" => tasks::handle_verify_command_set,
        "Set or clear the verify command for a repository path — the check an agent working in \
that repo must run green before a rebase or pr wrap-up (a done wrap-up runs none). It never appears in a dispatch prompt. \
It reaches the agent through the 'Verify command' line in task_search's response, and the agent \
runs it itself in its worktree. \
Pass command=null to clear it.",
        {
            "type": "object",
            "properties": {
                "repo_path": {
                    "type": "string",
                    "description": "Absolute path to the repository root (e.g. the task's repo_path)"
                },
                "command": {
                    "type": ["string", "null"],
                    "description": "Single-line shell command to run for verification. Omit or pass null to clear."
                }
            },
            "required": ["repo_path"]
        };

    async destructive "session_exit" => tasks::handle_exit_session,
        "Close your agent session with the token session_wrap-up returned. Applies the terminal mutation \
atomically with clearing the tmux window: 'rebase'/'done' move the task to Done; 'pr' moves it to \
Review and sets the PR url. The action must match the one passed to session_wrap-up. If the task belongs \
to an epic with auto_dispatch enabled, closing it also starts that epic's next backlog subtask.",
        {
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "integer",
                    "description": "The ID of the task whose session to close"
                },
                "token": {
                    "type": "string",
                    "description": "Exit token obtained from session_wrap-up."
                },
                "action": {
                    "type": "string",
                    "enum": wrap_up_action_enum_values(),
                    "description": "Must match the action passed to session_wrap-up for this token."
                },
                "pr_url": {
                    "type": "string",
                    "description": "Required when action is 'pr'. The GitHub PR URL returned by gh pr create."
                }
            },
            "required": ["task_id", "token", "action"]
        };

    async read_only "usage_read" => tasks::handle_usage_read,
        "Query aggregated feature usage counts. Returns events grouped by category/action/actor, ordered by count ascending (least-used first). Useful for identifying unused keybindings or MCP tools.",
        {
            "type": "object",
            "properties": {
                "category": {
                    "type": "string",
                    "description": "Filter by category: 'keybinding' or 'mcp_tool'",
                    "enum": crate::models::UsageCategory::ALL.iter().map(|c| c.as_str()).collect::<Vec<_>>()
                },
                "actor": {
                    "type": "string",
                    "description": "Filter by actor: 'human' or 'agent'",
                    "enum": crate::models::UsageActor::ALL.iter().map(|a| a.as_str()).collect::<Vec<_>>()
                },
                "since": {
                    "type": "string",
                    "description": "ISO 8601 datetime lower bound, e.g. '2026-01-01T00:00:00Z'"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Max results (default 50, max 500)"
                }
            }
        }
;

    async read_only "context_read" => context::handle_context_read,
        "Return the text of one dispatch context resource, the same text the resources/read method gives for that URI. Pass skill://<name>/SKILL.md for an agent-facing skill or skill://<name>/references/<file>.md for a reference file beside it, dispatch://learnings/<id> for a validated learning from the knowledge base, dispatch://task/self for your own task, or dispatch://task/<id> for any task by id, exactly as task_search renders it with task_id. Use it to pull a skill, a reference or a learning into your context when you only have its URI; it does not search, and it does not list what exists. An unknown URI is refused with an error naming it.",
        {
            "type": "object",
            "properties": {
                "uri": {
                    "type": "string",
                    "description": "The resource URI to read: skill://<name>/SKILL.md, skill://<name>/references/<file>.md, dispatch://learnings/<id>, dispatch://task/self or dispatch://task/<id>."
                }
            },
            "required": ["uri"]
        }
;

    async read_only "keybinding_search" => keybindings::handle_keybinding_search,
        "Search the keys the dispatch board, the agent-tree and diff panes, and tmux answer to, straight from the table the running board loaded, so the user's own keys show where they replaced a default. With no arguments it returns an index, not the table: one entry per input mode (namespace) with its row count and how many rows the user rebound, plus any entries of the user's key override file the board refused. Pass namespace for one mode's keys, or query to find rows across every mode. Each row has its keys, the context it applies under as words and as the literal keybinding_set takes (when it has one), the action id, a description and a note where a key is easy to misread; a rebound row also has its default keys, and a row whose keys change at the next board start has its pending keys. Read-only; change keys with keybinding_set.",
        {
            "type": "object",
            "properties": {
                "namespace": {
                    "type": "string",
                    "description": "Optional filter: one namespace (e.g. 'board.normal', 'agent_diff', 'tmux.global') or a family ('board.confirm', 'board.picker') for all its namespaces. An unknown name is rejected with the list of valid ones."
                },
                "query": {
                    "type": "string",
                    "description": "Optional text. Keeps the rows whose keys, action id or description contain it, ignoring case, and drops a namespace left with none. Combines with namespace."
                }
            }
        }
;

    async idempotent "keybinding_set" => keybindings::handle_keybinding_set,
        "Change the user's keys: set the keys of one or more rows of the keybinding table, or restore a row's default keys, in the key override file in the data directory. Name each row the way keybinding_search shows it: namespace, action id, and the context literal where the action has several rows in the namespace. The changes of one call are checked together with the checks the board runs when it loads the file, so swapping two rows' keys is one call, and a call with any change the board would refuse writes nothing. It returns each changed row with the keys it gets. The running board keeps the keys it loaded: a change applies when the board next starts.",
        {
            "type": "object",
            "properties": {
                "overrides": {
                    "type": "array",
                    "minItems": 1,
                    "description": "The rows to change, applied together. Each names one row and gives its new keys.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "namespace": {
                                "type": "string",
                                "description": "The row's namespace, as keybinding_search names it (e.g. 'board.normal')."
                            },
                            "action": {
                                "type": "string",
                                "description": "The row's action id, as keybinding_search shows it."
                            },
                            "context": {
                                "type": "string",
                                "description": "The row's context literal (keybinding_search's context_literal). Needed only where the action has several rows in the namespace; without it the action's row with no context is meant."
                            },
                            "keys": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Every key the row should answer to, replacing its current keys, written as keybinding_search writes keys ('j', 'Enter', 'Ctrl+D', 'gg'). An empty list restores the row's default keys."
                            }
                        },
                        "required": ["namespace", "action", "keys"]
                    }
                }
            },
            "required": ["overrides"]
        }
}

// ---------------------------------------------------------------------------
// MCP handler
// ---------------------------------------------------------------------------

/// The MCP server instructions: the conventions that span several tools and
/// that no one tool's schema owns. Claude Code loads them into every connected
/// session's system prompt, so dispatch prompts do not restate them. Repository
/// policy and per-task rules stay in the prompt — see
/// `InstructionsCoverOnlyConventionsNoSchemaStates` in
/// `docs/specs/mcp-task-tools.allium`.
const SERVER_INSTRUCTIONS: &str = "Each dispatched task runs a Claude Code agent with its own \
agent name, shown in the task's details and in task listings. Wherever these tools take a task \
id, that agent name works too. The agent's session carries the same name, so reach another \
task's agent with ListAgents and SendMessage by its agent name. Tasks in the same epic are \
siblings.";

/// The `"initialize"` arm of [`handle_mcp`]. Per spec: if the client offers a
/// version we support, echo it back to use that version for the session;
/// otherwise reply with our latest supported version (the client may then
/// abort).
fn negotiate_initialize(id: Option<Value>, params: Option<Value>) -> JsonRpcResponse {
    let client_version = params
        .as_ref()
        .and_then(|p| p.get("protocolVersion"))
        .and_then(Value::as_str);
    let negotiated = match client_version {
        Some(v) if SUPPORTED_PROTOCOL_VERSIONS.contains(&v) => v,
        _ => SERVER_PROTOCOL_VERSION,
    };
    JsonRpcResponse::ok(
        id,
        json!({
            "protocolVersion": negotiated,
            "capabilities": {
                "tools": {},
                "resources": {}
            },
            "serverInfo": {
                "name": "dispatch",
                "version": env!("CARGO_PKG_VERSION")
            },
            "instructions": SERVER_INSTRUCTIONS
        }),
    )
}

/// The `"tools/call"` arm of [`handle_mcp`]: resolves caller identity,
/// dispatches the tool, and records trajectory/usage as a side effect.
async fn handle_tools_call(
    state: &McpState,
    id: Option<Value>,
    identity_result: &Result<CallerIdentity, IdentityError>,
    params: Option<Value>,
) -> JsonRpcResponse {
    let identity = match identity_result.as_ref() {
        Ok(i) => i,
        Err(e) => return JsonRpcResponse::err(id, INVALID_REQUEST, e.to_string()),
    };

    let params = params.unwrap_or(Value::Null);
    let tool_name = params.get("name").and_then(Value::as_str).unwrap_or("");
    // MCP lets a caller omit `arguments`; a missing or null one is "no arguments".
    let mut args = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(v) => v.clone(),
    };

    // The name of one of a task's agents is a second name for its id
    // (`AgentNameResolvesToTask`); swap it for the id before any handler
    // parses the arguments. The agent list is read only when a call actually
    // names one.
    let resolution = if super::task_refs::holds_name(&args) {
        let agents = state.db.list_agents().await;
        super::task_refs::resolve_task_refs(&mut args, &agents)
    } else {
        Ok(())
    };

    let start = std::time::Instant::now();
    let start_utc = Utc::now();
    // A name that resolves to nothing is a tool failure like any other: it is
    // recorded below and answered as an `isError` result, not a protocol error.
    let resp = match resolution {
        Ok(()) => dispatch_tool(state, id, identity, tool_name, args.clone()).await,
        Err(bad) => tool_error(id, bad.message()),
    };

    if let CallerIdentity::Task(task_id) = identity {
        let duration_ms = start.elapsed().as_millis() as u64;
        let result_value = match &resp.error {
            Some(err) => serde_json::json!({"error": err.message}),
            None => resp.result.clone().unwrap_or(Value::Null),
        };
        let entry = trajectory::TrajectoryEntry {
            timestamp: start_utc,
            task_id: task_id.0,
            method: tool_name.to_string(),
            args,
            result: result_value,
            duration_ms,
        };
        let data_dir = state.data_dir.clone();
        let bg_done = state.test_hooks.bg_write_done_tx.clone();
        tokio::spawn(async move {
            trajectory::append_entry(&data_dir, &entry).await;
            if let Some(tx) = &bg_done {
                let _ = tx.send(crate::mcp::BackgroundWrite::Trajectory);
            }
        });
    }

    // Fire-and-forget usage recording. Skipped when no tool name was
    // resolved (e.g. malformed request) to avoid recording empty rows.
    if !tool_name.is_empty() {
        let actor = match identity {
            CallerIdentity::Task(_) => crate::models::UsageActor::Agent,
            CallerIdentity::Session => crate::models::UsageActor::Human,
        };
        let db = Arc::clone(&state.db);
        // Only registered names are recorded as themselves; a caller-supplied
        // unknown name would otherwise mint a usage row per distinct string.
        let tool = if TOOL_NAMES.contains(&tool_name) {
            tool_name.to_string()
        } else {
            "unknown".to_string()
        };
        let bg_done = state.test_hooks.bg_write_done_tx.clone();
        tokio::spawn(async move {
            crate::service::record_usage_event_logged(
                db.as_ref(),
                &crate::models::UsageEvent {
                    category: crate::models::UsageCategory::McpTool,
                    action: tool,
                    detail: None,
                    actor,
                },
            )
            .await;
            if let Some(tx) = &bg_done {
                let _ = tx.send(crate::mcp::BackgroundWrite::Usage);
            }
        });
    }

    // Per MCP spec, tool-execution failures belong in `result` with
    // `isError: true` — not as a JSON-RPC protocol error. Re-wrap any
    // error returned by the tool dispatch path.
    match resp.error {
        Some(err) => tool_error(resp.id, err.message),
        None => resp,
    }
}

pub async fn handle_mcp(
    State(state): State<Arc<McpState>>,
    Extension(identity_result): Extension<Result<CallerIdentity, IdentityError>>,
    Json(req): Json<JsonRpcRequest>,
) -> Response {
    // JSON-RPC 2.0 §4.1: a Notification (no `id`) must never receive a reply.
    // MCP streamable-HTTP maps this to HTTP 202 Accepted with no body. Replying
    // to `notifications/initialized` (which Claude Code always sends after
    // `initialize`) with `id: null` breaks the client's response schema and
    // aborts the session.
    if req.id.is_none() {
        tracing::debug!(method = %req.method, "received MCP notification (no reply)");
        return StatusCode::ACCEPTED.into_response();
    }

    let id = req.id;
    let response = match req.method.as_str() {
        "initialize" => negotiate_initialize(id, req.params),
        "ping" => JsonRpcResponse::ok(id, json!({})),
        "tools/list" => JsonRpcResponse::ok(id, tool_definitions()),
        "resources/list" => {
            context::handle_resources_list(&state, id, &identity_result, req.params).await
        }
        "resources/read" => {
            context::handle_resources_read(&state, id, &identity_result, req.params).await
        }
        "tools/call" => handle_tools_call(&state, id, &identity_result, req.params).await,
        other => JsonRpcResponse::err(id, METHOD_NOT_FOUND, format!("Method not found: {other}")),
    };

    (StatusCode::OK, Json(response)).into_response()
}
