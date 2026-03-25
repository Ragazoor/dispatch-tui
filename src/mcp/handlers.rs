use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::models::{NoteSource, TaskStatus};

use super::McpState;

// ---------------------------------------------------------------------------
// JSON-RPC request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

impl JsonRpcResponse {
    fn ok(id: Option<Value>, result: Value) -> Self {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn err(id: Option<Value>, code: i32, message: impl Into<String>) -> Self {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Tool definitions returned by tools/list
// ---------------------------------------------------------------------------

fn tool_definitions() -> Value {
    json!({
        "tools": [
            {
                "name": "update_task",
                "description": "Update the status of a task",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "task_id": {
                            "type": "integer",
                            "description": "The task ID"
                        },
                        "status": {
                            "type": "string",
                            "description": "New status: backlog, ready, running, review, or done",
                            "enum": ["backlog", "ready", "running", "review", "done"]
                        }
                    },
                    "required": ["task_id", "status"]
                }
            },
            {
                "name": "add_note",
                "description": "Add a note to a task",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "task_id": {
                            "type": "integer",
                            "description": "The task ID"
                        },
                        "note": {
                            "type": "string",
                            "description": "The note content"
                        }
                    },
                    "required": ["task_id", "note"]
                }
            },
            {
                "name": "get_task",
                "description": "Get details about a task",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "task_id": {
                            "type": "integer",
                            "description": "The task ID"
                        }
                    },
                    "required": ["task_id"]
                }
            }
        ]
    })
}

// ---------------------------------------------------------------------------
// MCP handler
// ---------------------------------------------------------------------------

pub async fn handle_mcp(
    State(state): State<Arc<McpState>>,
    Json(req): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    let id = req.id;
    let response = match req.method.as_str() {
        "initialize" => {
            JsonRpcResponse::ok(id, json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "task-orchestrator",
                    "version": "0.1.0"
                }
            }))
        }

        "tools/list" => JsonRpcResponse::ok(id, tool_definitions()),

        "tools/call" => {
            let params = req.params.unwrap_or(Value::Null);
            let tool_name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);

            match tool_name {
                "update_task" => handle_update_task(&state, id, &args),
                "add_note" => handle_add_note(&state, id, &args),
                "get_task" => handle_get_task(&state, id, &args),
                other => JsonRpcResponse::err(id, -32602, format!("Unknown tool: {other}")),
            }
        }

        other => JsonRpcResponse::err(id, -32601, format!("Method not found: {other}")),
    };

    (StatusCode::OK, Json(response))
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

fn handle_update_task(state: &McpState, id: Option<Value>, args: &Value) -> JsonRpcResponse {
    let task_id = match args.get("task_id").and_then(Value::as_i64) {
        Some(v) => v,
        None => return JsonRpcResponse::err(id, -32602, "Missing or invalid task_id"),
    };
    let status_str = match args.get("status").and_then(Value::as_str) {
        Some(v) => v,
        None => return JsonRpcResponse::err(id, -32602, "Missing or invalid status"),
    };
    let status = match TaskStatus::from_str(status_str) {
        Some(s) => s,
        None => {
            return JsonRpcResponse::err(
                id,
                -32602,
                format!("Unknown status: {status_str}. Valid values: backlog, ready, running, review, done"),
            )
        }
    };

    match state.db.update_status(task_id, status) {
        Ok(()) => JsonRpcResponse::ok(
            id,
            json!({
                "content": [{"type": "text", "text": format!("Task {task_id} updated to {status_str}")}]
            }),
        ),
        Err(e) => JsonRpcResponse::err(id, -32603, format!("Database error: {e}")),
    }
}

fn handle_add_note(state: &McpState, id: Option<Value>, args: &Value) -> JsonRpcResponse {
    let task_id = match args.get("task_id").and_then(Value::as_i64) {
        Some(v) => v,
        None => return JsonRpcResponse::err(id, -32602, "Missing or invalid task_id"),
    };
    let note = match args.get("note").and_then(Value::as_str) {
        Some(v) => v,
        None => return JsonRpcResponse::err(id, -32602, "Missing or invalid note"),
    };

    match state.db.add_note(task_id, note, NoteSource::Agent) {
        Ok(note_id) => JsonRpcResponse::ok(
            id,
            json!({
                "content": [{"type": "text", "text": format!("Note {note_id} added to task {task_id}")}]
            }),
        ),
        Err(e) => JsonRpcResponse::err(id, -32603, format!("Database error: {e}")),
    }
}

fn handle_get_task(state: &McpState, id: Option<Value>, args: &Value) -> JsonRpcResponse {
    let task_id = match args.get("task_id").and_then(Value::as_i64) {
        Some(v) => v,
        None => return JsonRpcResponse::err(id, -32602, "Missing or invalid task_id"),
    };

    match state.db.get_task(task_id) {
        Ok(Some(task)) => {
            let text = format!(
                "Task {id}: {title}\nStatus: {status}\nRepo: {repo}\nDescription: {desc}",
                id = task.id,
                title = task.title,
                status = task.status.as_str(),
                repo = task.repo_path,
                desc = task.description,
            );
            JsonRpcResponse::ok(
                id,
                json!({
                    "content": [{"type": "text", "text": text}]
                }),
            )
        }
        Ok(None) => JsonRpcResponse::err(id, -32602, format!("Task {task_id} not found")),
        Err(e) => JsonRpcResponse::err(id, -32603, format!("Database error: {e}")),
    }
}
