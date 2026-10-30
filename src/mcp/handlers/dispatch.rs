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
enum Access {
    /// Changes nothing.
    ReadOnly,
    /// Changes state; repeating the call may change it again.
    Write,
    /// Changes state; repeating the identical call leaves the same state.
    Idempotent,
    /// Removes or closes something no other call restores.
    Destructive,
}

/// The MCP `annotations` object for a tool: a title and the four behaviour
/// hints. `openWorldHint` is false everywhere — no tool reaches outside dispatch.
fn annotations(name: &str, access: Access) -> Value {
    let title = name
        .split('_')
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
                other => JsonRpcResponse::err(id, INVALID_PARAMS, format!("Unknown tool: {other}")),
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

// `pub(super)` because the `update_tasks` schema is generated from the
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
    async idempotent "update_tasks" => tasks::handle_update_tasks,
        "Update one or many tasks. Pass task_ids (one id updates one task) plus the fields to set: \
status, sub_status, title, description, repo_path, plan_path, sort_order, url, url_type, tag, \
epic_id, base_branch, wrap_up_mode, model, auto_run_plan, phoenix, and schedule. At least one field \
must be provided; every field left out is untouched. Every named task is updated whatever its status; \
pick many ids with list_tasks (epic_id, recursive, status, base_branch). With one id the reply is \
\"Task N updated (fields)\" or an error. With several it is not atomic: one line per task, success \
or failure, then a count.",
        (tasks::update_tasks_schema());

    async read_only "get_task" => tasks::handle_get_task,
        "Get the details of a task by ID.",
        {
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "integer",
                    "description": "The task ID"
                }
            },
            "required": ["task_id"]
        };

    async write "create_task" => tasks::handle_create_task,
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
                    "description": "The model the task's agent launches on: 'haiku', 'sonnet', 'opus' or 'fable'. A task with no model runs on sonnet. Defaults to sonnet.",
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
                    "description": "When true, signal the user that a new task arrived: the new task and every epic on the path to it are marked on the board until the user enters the epic. Only the people following the epic when the task is created are marked. Ignored when epic_id is null. Defaults to false."
                }
            },
            "required": ["title", "repo_path", "epic_id"]
        };

    async read_only "list_tasks" => tasks::handle_list_tasks,
        "List tasks on the kanban board. Filters are ANDed. When called by a dispatched agent, results auto-scope to the agent's epic and exclude the agent's own task; passing explicit epic_id/repo_paths disables the epic auto-scoping, but your own task is still excluded. When called from a non-dispatched session, no auto-scoping. Output includes each task's base branch, its url under its own type label — 'PR', 'Issue', 'Security Alert' or 'Link', with the number appended where there is one — and the plan goal, when available.",
        {
            "type": "object",
            "properties": {
                "status": {
                    "description": "Filter by status. Single string or array of strings.",
                    "oneOf": [
                        { "type": "string", "enum": task_status_enum_values() },
                        { "type": "array", "items": { "type": "string", "enum": task_status_enum_values() } }
                    ]
                },
                "epic_id": {
                    "type": "integer",
                    "description": "Filter to only tasks belonging to this epic."
                },
                "repo_paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Filter to tasks whose repo_path is in this list."
                },
                "recursive": {
                    "type": "boolean",
                    "description": "With an epic scope (epic_id, or your own epic when auto-scoped), also include tasks of every descendant epic. Rejected when no epic scope is in effect."
                },
                "base_branch": {
                    "type": "string",
                    "description": "The base branch for rebase and PR operations (e.g. 'main', 'develop'). Lists only tasks whose base_branch is exactly this."
                },
                "query": {
                    "type": "string",
                    "description": "Text search over title and description. Split on spaces; every word must appear (any case, as part of a word) in the title or description. Combines with the other filters."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Return at most this many rows. No default: without it every match is returned. When rows are cut, the last line says how many were shown of how many."
                }
            }
        };

    async write "create_epic" => epics::handle_create_epic,
        "Create a new epic on the kanban board. An epic groups related tasks and derives its \
status from theirs, so you do not set an epic's status directly. Pass parent_epic_id to create it \
as a sub-epic of an existing one. A root epic needs a workspace_id (a refusal lists yours); a \
sub-epic takes its parent's workspace and refuses one. visibility is public (every member of the \
workspace sees it, the default) or private (only you do). Epics carry no repo_path — that lives \
on each subtask, and passing it here is rejected.",
        {
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Epic title" },
                "description": { "type": "string", "description": "Epic description" },
                "sort_order": { "type": "integer", "description": "Display order within column (lower values appear first)" },
                "parent_epic_id": { "type": "integer", "description": "Optional parent epic ID. When set, this epic becomes a sub-epic of the specified parent." },
                "workspace_id": { "type": "integer", "description": "The workspace a root epic lives in. Required without parent_epic_id; refused with it. A refusal lists the workspaces you belong to." },
                "visibility": { "type": "string", "enum": ["public", "private"], "description": "public: every member of the workspace sees it. private: only you do. Default public." }
            },
            "required": ["title"]
        };

    async read_only "list_epics" => epics::handle_list_epics,
        "List epics on the kanban board, one row each, with each sub-epic's parent id. With no \
arguments it lists every epic. Pass parent_epic_id for that epic's direct children, plus \
recursive=true for its whole subtree. Pass query to search titles and descriptions. Use it to find an epic's ID when you only know its title. \
Pass epic_id instead for one epic in full: its description, status, plan, parent, feed settings \
and a count of its subtasks by status, but not the subtasks themselves — call list_tasks with \
epic_id for those.",
        {
            "type": "object",
            "properties": {
                "epic_id": { "type": "integer", "description": "Return this one epic in full instead of a list. Takes no other argument." },
                "parent_epic_id": { "type": "integer", "description": "List only the sub-epics of this epic." },
                "recursive": { "type": "boolean", "description": "With parent_epic_id, list every descendant rather than only direct children." },
                "query": {
                    "type": "string",
                    "description": "Text search over title and description. Split on spaces; every word must appear (any case, as part of a word) in the title or description. Combines with the other filters."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Return at most this many rows. No default: without it every match is returned. When rows are cut, the last line says how many were shown of how many."
                }
            }
        };

    async idempotent "update_epic" => epics::handle_update_epic,
        "Update an epic in place. Accepts title, description, status, plan_path, sort_order, \
parent_epic_id, and the feed settings feed_command, feed_interval_secs, group_by_repo and \
feed_append_only, and visibility (public or private; a workspace never changes). Every field left out is untouched; the nullable ones take an explicit null to \
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
                "visibility": { "type": "string", "enum": ["public", "private"], "description": "public: every member of the workspace sees it. private: only its creator does. Changeable by the creator or a workspace admin." }
            },
            "required": ["epic_id"]
        };

    async write "create_work_package" => work_packages::handle_create_work_package,
        "Create a work package: a named group that links to tasks and epics which already exist, \
wherever they sit in the epic tree, so work spread across several epics can be followed as one \
board. A package only links; it never owns its items, and one item can belong to many packages. Pass \
link_task_ids and link_epic_ids to link items at once; every id is checked first, and an unknown \
one creates nothing. Needs a workspace_id (a refusal lists yours) and links only items in that \
workspace, or tasks on your main board. visibility is public (the default) or private (only you \
see it). Returns the new package's id. To group new work under a parent, use create_epic \
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

    async read_only "list_work_packages" => work_packages::handle_list_work_packages,
        "List work packages with their linked items. Read-only. With no arguments it lists every \
package; pass work_package_id for one. Each package shows its title, description and every linked \
task or epic with its id, status and title. A package has no status of its own: read progress from \
the items. A package's sort order is shown when it has been set.",
        {
            "type": "object",
            "properties": {
                "work_package_id": { "type": "integer", "description": "List only this package." }
            }
        };

    async destructive "update_work_package" => work_packages::handle_update_work_package,
        "Update a work package in place: rename it with title, rewrite description, set sort_order (lower values show first among the package cards), set visibility (public or private), and change \
its links with link_task_ids, unlink_task_ids, link_epic_ids and unlink_epic_ids. Unlinks apply \
before links, linking an item already linked and unlinking one not linked are both silent, and \
the tasks and epics themselves are never changed. Every id is checked before anything is written, \
and an id named in both a link and an unlink list is refused. Every field left out is untouched, \
and at least one must be given. Pass delete=true, and nothing else, to delete the package once it \
has served its purpose: its linked tasks and epics stay where they are, with their status unchanged.",
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
                "unlink_epic_ids": { "type": "array", "items": { "type": "integer" }, "description": "Ids of epics to unlink." },
                "delete": { "type": "boolean", "description": "When true, delete the package and its links. Must be the only field besides work_package_id. False is the same as leaving it out." }
            },
            "required": ["work_package_id"]
        };

    async write "wrap_up" => tasks::handle_wrap_up,
        "Wrap up a running or review task with one of three actions (rebase, done, pr) and return an \
exit token for exit_session; the task stays running until then. \
'rebase' rebases onto base_branch and fast-forwards it (blocks until complete). \
'done' and 'pr' perform no git operations — for 'pr' you run git push and gh pr create yourself \
and pass the resulting URL to exit_session. \
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
                    "description": "'rebase' — rebase onto base_branch and fast-forward it. 'done' — no git ops. 'pr' — no git ops; agent has already run gh pr create. All three return an exit token; the task stays in its current status until exit_session closes it."
                }
            },
            "required": ["task_id", "action"]
        };

    async write "dispatch_task" => tasks::handle_dispatch_task,
        "Dispatch a backlog task by ID, then launch an agent in a tmux session. If the task has \
no worktree directory yet, one is CREATED — from the task's base branch, or from the PR's head \
branch for a review task carrying a PR url. If it already exists it is \
REUSED as-is — its branch, its commits and its uncommitted changes are all still there, and the \
agent starts in them. So a crashed or stalled task is restarted in place: set it back to backlog \
with update_tasks, then dispatch_task it. Tell the new agent in the task what its worktree \
already holds — it is a new agent and has none of the previous session's context. To wake a \
paused agent WITH its context, use resume_task instead. Waits up to 30 \
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

    async write "resume_task" => tasks::handle_resume_task,
        "Wake a detached (paused) agent WITH its context. Reattaches the task's worktree, starts \
the Claude session so it continues the agent's previous conversation, and puts the task back to \
running — what pressing space on a detached agent in the TUI does. Prefer this over update_tasks \
status=backlog plus dispatch_task, which starts a NEW agent that has none of the old context. \
The optional message is delivered as the first input after the session resumes; use it to say why \
the agent was woken. Returns once the board has been asked to resume, not when the session is \
up. Errors with 'no resumable session' when the task has no worktree or is in backlog (use \
dispatch_task for those), and when its agent is already attached (use SendMessage).",
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

    async write "record_learning" => learnings::handle_record_learning,
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
                    "description": "Scope this learning applies to: user (global), repo, epic, or task",
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

    async read_only "query_learnings" => learnings::handle_query_learnings,
        "Query the knowledge base by semantic similarity to your task's context. Results are \
ranked by embedding similarity plus a scope and upvote boost, and entries below the similarity \
threshold are dropped — so an empty result means nothing relevant was found, not that the store is \
empty. Task-scoped entries are excluded; the entries already injected into your prompt are not, so \
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
                    "description": "Maximum number of results to return (default 50, max 50)"
                }
            },
            "required": ["task_id"]
        };

    async write "rate_learning" => learnings::handle_rate_learning,
        "Give feedback on a knowledge base entry that was surfaced to you this task. \
Call any time you act on a retrieved learning. 'helped' upvotes it (a usefulness signal that \
boosts ranking); 'wrong' downvotes it (may go negative) — neither changes its status, and \
there is no human review step. You can only rate a learning that was surfaced to you (injected \
into your prompt or returned by query_learnings).",
        {
            "type": "object",
            "properties": {
                "learning_id": {
                    "type": "integer",
                    "description": "ID of the knowledge base entry to rate"
                },
                "task_id": {
                    "type": "integer",
                    "description": "ID of the task that acted on this entry"
                },
                "verdict": {
                    "type": "string",
                    "enum": crate::models::LearningVerdict::ALL.iter().map(|v| v.as_str()).collect::<Vec<_>>(),
                    "description": "'helped' — the entry applied and was useful (upvotes it). 'wrong' — the entry misled you or is inaccurate (downvotes it; may go negative). 'not_applicable' — you read an injected entry and it did not fit this task (score unchanged; it tells the store the entry was off-topic here). No verdict changes the entry's status."
                }
            },
            "required": ["learning_id", "task_id", "verdict"]
        };

    async destructive "delete_learning" => learnings::handle_delete_learning,
        "Delete a learning entry from the knowledge base by ID. Permanent and immediate — there \
is no archive, no soft delete and no human review step, so the entry stops reaching other agents \
at once. Use it for an entry that is wrong or has gone stale, rather than only downvoting it with \
rate_learning; downvoting keeps a wrong entry in circulation. Returns an error if the ID does not \
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

    async idempotent "managed_feed_config" => managed_feeds::handle_managed_feed_config,
        "Read or change the managed-feed configuration: the PR-reviews and CVE feed commands and \
their poll intervals (in seconds). Call it with no arguments to only read the current values; \
that call changes nothing. Pass any field to change it: each field is triple-state — omit to \
leave unchanged, pass null to clear, or pass a value to set. Either way the reply is the \
resulting configuration, and unset values are reported as unset, which is how you tell a \
disabled feed from one running on the default interval. A change re-provisions the managed epic \
tree, so enabling a feed creates its epics immediately. Omit an interval to fall back to the \
default feed interval.",
        {
            "type": "object",
            "properties": {
                "reviews_command": { "type": ["string", "null"], "description": "Script emitting the involved-PR feed. Pass null to clear (disables the reviews subtree). Omit to leave unchanged." },
                "reviews_interval_secs": { "type": ["integer", "null"], "description": "Poll cadence for the reviews feed, in seconds. Minimum 60; a lower value is rejected. Pass null to clear (falls back to the 60s default). Omit to leave unchanged." },
                "cve_command": { "type": ["string", "null"], "description": "Script emitting the security/CVE feed. Pass null to clear (disables the CVE epic). Omit to leave unchanged." },
                "cve_interval_secs": { "type": ["integer", "null"], "description": "Poll cadence for the CVE feed, in seconds. Minimum 60; a lower value is rejected. Pass null to clear (falls back to the 60s default). Omit to leave unchanged." }
            }
        };

    async idempotent "set_display_name" => people::handle_set_display_name,
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

    async read_only "list_workspaces" => admin::handle_list_workspaces,
        "List the workspaces you can see, who is in each and in which role, who the super-admins are, \
and the directory of every person by display name and short identity. Use it to find the \
workspace_id and the person to name in create_epic, set_workspace_member and set_super_admin: \
people are named by display name, and the directory tells two people with one name apart by \
identity. A person sees the workspaces they belong to; a super-admin sees all of them. It \
also says when claim_super_admin would work. It returns no epics, tasks or other content.",
        {
            "type": "object",
            "properties": {
                "workspace_id": { "type": "integer", "description": "Show only this workspace and its members, and leave out the directory of people. Omit to list every workspace you can see." }
            }
        };

    async write "create_workspace" => admin::handle_create_workspace,
        "Create a workspace: a place where people collaborate, holding epics and work packages. Only a \
super-admin may. The new workspace has no members, not even you; add people, yourself included, \
with set_workspace_member. Names are unique across the store, trimmed, and at most 64 characters. \
Returns the new workspace's id and name.",
        {
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "The workspace name, 1 to 64 characters after trimming, not already used by another workspace." }
            },
            "required": ["name"]
        };

    async destructive "update_workspace" => admin::handle_update_workspace,
        "Rename a workspace with name, or delete it with delete=true; give exactly one. A super-admin or \
an admin of the workspace may rename it. Only a super-admin may delete, and only an empty \
workspace: one still holding an epic or work package, private ones included, is refused, so empty \
it first (list_unseen_items and delete_unseen_item reach private items nobody can see). Deleting \
removes its memberships and its learnings and cannot be undone. Returns what changed.",
        {
            "type": "object",
            "properties": {
                "workspace_id": { "type": "integer", "description": "The workspace to rename or delete, from list_workspaces." },
                "name": { "type": "string", "description": "The new name, 1 to 64 characters after trimming, not used by another workspace. Give this or delete, not both." },
                "delete": { "type": "boolean", "description": "When true, delete the workspace. Give this or name, not both. False is the same as leaving it out." }
            },
            "required": ["workspace_id"]
        };

    async destructive "set_workspace_member" => admin::handle_set_workspace_member,
        "Add a person to a workspace, change their role there, or remove them; one call does one of the \
three. Give role to add the person (if they are not a member) or to change their role (if they are); \
give remove=true to take them out. A super-admin or an admin of the workspace may. Removing a \
person does not touch their epics or packages: their public ones stay for the others, their \
private ones are then seen by nobody until they rejoin or a super-admin deletes them. Returns what \
changed.",
        {
            "type": "object",
            "properties": {
                "workspace_id": { "type": "integer", "description": "The workspace, from list_workspaces." },
                "person": { "type": "string", "description": "The person's display name, or their identity (or its first 8 or more characters) when two people share a name. A name more than one person has is refused with the matches." },
                "role": { "type": "string", "enum": ["admin", "member"], "description": "The role to give: admin can manage the workspace's members, member cannot. Adds the person when they are not a member. Give this or remove, not both." },
                "remove": { "type": "boolean", "description": "When true, remove the person from the workspace; they must be a member. Give this or role, not both. False is the same as leaving it out." }
            },
            "required": ["workspace_id", "person"]
        };

    async idempotent "set_super_admin" => admin::handle_set_super_admin,
        "Make a person a super-admin or stop them being one. Only a super-admin may. A super-admin \
manages every workspace and may delete private items nobody can see, but cannot read them. \
Repeating a call that already holds changes nothing. Taking the role from the last super-admin is \
refused. To become the first one yourself, use claim_super_admin. Returns the person's name and \
their new standing.",
        {
            "type": "object",
            "properties": {
                "person": { "type": "string", "description": "The person's display name, or their identity (or its first 8 or more characters) when two people share a name." },
                "super_admin": { "type": "boolean", "description": "true makes the person a super-admin; false revokes it." }
            },
            "required": ["person", "super_admin"]
        };

    async write "claim_super_admin" => admin::handle_claim_super_admin,
        "Make yourself the first super-admin. It works only while the store has none, and then once: \
after that it is refused and a super-admin uses set_super_admin to add more. You are the person \
whose board this agent runs under. Takes no arguments. Returns your new standing.",
        {
            "type": "object",
            "properties": {}
        };

    async read_only "list_unseen_items" => admin::handle_list_unseen_items,
        "List private epics and work packages that nobody can see any more because their creator has \
left the workspace. Only a super-admin sees any; for anyone else the list is empty. Each entry is \
its kind, id, workspace, creator and created date, never a title or content. The only thing to \
do with one is delete_unseen_item.",
        {
            "type": "object",
            "properties": {}
        };

    async destructive "delete_unseen_item" => admin::handle_delete_unseen_item,
        "Permanently delete a private epic or work package that nobody can see, chosen from \
list_unseen_items. Only a super-admin may. An epic is deleted with its sub-epics and tasks, and is \
refused while any task in it is not done. A public item, or an id that does not exist, is refused \
in the same words. It cannot be undone, and you never read what you delete.",
        {
            "type": "object",
            "properties": {
                "kind": { "type": "string", "enum": ["epic", "work_package"], "description": "The kind of item, as list_unseen_items names it." },
                "item_id": { "type": "integer", "description": "The item's id from list_unseen_items." }
            },
            "required": ["kind", "item_id"]
        };

    async idempotent "set_verify_command" => tasks::handle_set_verify_command,
        "Set or clear the verify command for a repository path — the check an agent working in \
that repo must run green before declaring work complete. It never appears in a dispatch prompt. \
It reaches the agent through the 'Verify command' line in get_task's response, and the agent \
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

    async destructive "exit_session" => tasks::handle_exit_session,
        "Close your agent session with the token wrap_up returned. Applies the terminal mutation \
atomically with clearing the tmux window: 'rebase'/'done' move the task to Done; 'pr' moves it to \
Review and sets the PR url. The action must match the one passed to wrap_up. If the task belongs \
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
                    "description": "Exit token obtained from wrap_up."
                },
                "action": {
                    "type": "string",
                    "enum": wrap_up_action_enum_values(),
                    "description": "Must match the action passed to wrap_up for this token."
                },
                "pr_url": {
                    "type": "string",
                    "description": "Required when action is 'pr'. The GitHub PR URL returned by gh pr create."
                }
            },
            "required": ["task_id", "token", "action"]
        };

    async read_only "query_usage" => tasks::handle_query_usage,
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
                    "description": "Max results (default 50, max 500)"
                }
            }
        }
;

    async read_only "read_context" => context::handle_read_context,
        "Return the text of one dispatch context resource, the same text the resources/read method gives for that URI. Pass skill://<name>/SKILL.md for an agent-facing skill or skill://<name>/references/<file>.md for a reference file beside it, dispatch://learnings/<id> for a validated learning from the knowledge base, dispatch://task/self for your own task, or dispatch://task/<id> for any task by id, exactly as get_task renders it. Use it to pull a skill, a reference or a learning into your context when you only have its URI; it does not search, and it does not list what exists. An unknown URI is refused with an error naming it.",
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

    async read_only "list_keybindings" => keybindings::handle_list_keybindings,
        "List every key the dispatch board, the agent-tree and diff panes, and tmux answer to, grouped by input mode (namespace), straight from the table the key handlers read. Each row has its keys, the context it applies under (when it has one), the action id, a description and a note where a key is easy to misread. Read-only.",
        {
            "type": "object",
            "properties": {
                "namespace": {
                    "type": "string",
                    "description": "Optional filter: one namespace (e.g. 'board.normal', 'agent_diff', 'tmux.global') or a family ('board.confirm', 'board.picker') for all its namespaces. Omit for every namespace. An unknown name is rejected with the list of valid ones."
                }
            }
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
