//! MCP context resources: `resources/list`, `resources/read` and their tool
//! twin `read_context` (`McpContextResources` in
//! `docs/specs/mcp-task-tools.allium`).
//!
//! Five URI shapes are served, nothing else: `skill://<name>/SKILL.md`,
//! `skill://<name>/references/<file>.md`, `dispatch://learnings/<id>`,
//! `dispatch://task/self` and `dispatch://task/<id>` (read by id, never listed). Everything here is read-only: no write, no
//! retrieval record, no notification.
//!
//! Skill text comes from the copy of `plugin/skills` built into the binary
//! (`crate::setup::built_in_skills_dir`), never from the installed copy.

use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::mcp::identity::{CallerIdentity, IdentityError};
use crate::mcp::McpState;
use crate::models::{LearningId, LearningStatus};

use super::types::{parse_args, JsonRpcResponse, INTERNAL_ERROR, INVALID_PARAMS, INVALID_REQUEST};

/// Most entries one `resources/list` page returns
/// (`config.context_list_page_size` in the spec).
const CONTEXT_LIST_PAGE_SIZE: usize = 50;

const MIME_MARKDOWN: &str = "text/markdown";
const MIME_TEXT: &str = "text/plain";
const TASK_SELF_URI: &str = "dispatch://task/self";

// ---------------------------------------------------------------------------
// Built-in skills
// ---------------------------------------------------------------------------

struct BuiltInSkill {
    name: String,
    description: String,
    skill_md: &'static str,
    /// `(file name, text)`, by file name ascending.
    references: Vec<(String, &'static str)>,
}

/// The `name` and `description` of a SKILL.md frontmatter. Handles the plain
/// scalar and the folded (`>-`) description forms.
fn parse_frontmatter(skill_md: &str) -> Option<(String, String)> {
    let body = skill_md.strip_prefix("---\n")?;
    let end = body.find("\n---")?;
    let lines: Vec<&str> = body[..end].lines().collect();
    let mut name = None;
    let mut description = None;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some(v) = line.strip_prefix("name:") {
            name = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("description:") {
            let v = v.trim();
            if v.starts_with('>') || v.starts_with('|') {
                let mut folded = Vec::new();
                while i + 1 < lines.len() && lines[i + 1].starts_with(' ') {
                    i += 1;
                    folded.push(lines[i].trim());
                }
                description = Some(folded.join(" "));
            } else {
                description = Some(v.to_string());
            }
        }
        i += 1;
    }
    Some((name?, description?))
}

fn file_name_of(path: &std::path::Path) -> &str {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("")
}

fn load_built_in_skills() -> Vec<BuiltInSkill> {
    let Some(root) = crate::setup::built_in_skills_dir() else {
        return Vec::new();
    };
    let mut skills = Vec::new();
    for dir in root.dirs() {
        let Some(skill_md) = dir
            .files()
            .find(|f| file_name_of(f.path()) == "SKILL.md")
            .and_then(|f| f.contents_utf8())
        else {
            continue;
        };
        let Some((name, description)) = parse_frontmatter(skill_md) else {
            continue;
        };
        let mut references: Vec<(String, &'static str)> = dir
            .dirs()
            .find(|d| file_name_of(d.path()) == "references")
            .map(|refs| {
                refs.files()
                    .filter_map(|f| {
                        let file = file_name_of(f.path());
                        if !file.ends_with(".md") {
                            return None;
                        }
                        Some((file.to_string(), f.contents_utf8()?))
                    })
                    .collect()
            })
            .unwrap_or_default();
        references.sort_by(|a, b| a.0.cmp(&b.0));
        skills.push(BuiltInSkill {
            name,
            description,
            skill_md,
            references,
        });
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

fn built_in_skills() -> &'static [BuiltInSkill] {
    static SKILLS: OnceLock<Vec<BuiltInSkill>> = OnceLock::new();
    SKILLS.get_or_init(load_built_in_skills)
}

// ---------------------------------------------------------------------------
// URI shapes
// ---------------------------------------------------------------------------

/// A URI that has one of the five recognised shapes. Whether it resolves is a
/// separate question, answered at read time.
#[derive(Debug, PartialEq, Eq)]
enum ContextUri {
    Skill { name: String },
    SkillRef { name: String, file: String },
    Learning(i64),
    OwnTask,
    Task(i64),
}

/// The listing's stable order: own task, skills by name (SKILL.md before its
/// references, by file name), learnings by id. The derived `Ord` follows the
/// variant order.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum SortKey {
    OwnTask,
    Skill(String, Option<String>),
    Learning(i64),
}

/// A single path segment: non-empty, and nothing that could step out of a
/// directory.
fn is_plain_segment(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && !s.contains(['/', '\\', '%', '\0'])
}

/// A canonical decimal id: digits only, no leading zeros, so every readable
/// URI is one the listing (or a task's own link) emits.
fn parse_canonical_id(id: &str) -> Option<i64> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) || id.starts_with('0') {
        return None;
    }
    id.parse().ok()
}

/// Classify a URI by its shape alone (`context_uri_kind`).
fn parse_context_uri(uri: &str) -> Option<ContextUri> {
    if uri == TASK_SELF_URI {
        return Some(ContextUri::OwnTask);
    }
    if let Some(id) = uri.strip_prefix("dispatch://learnings/") {
        return parse_canonical_id(id).map(ContextUri::Learning);
    }
    if let Some(id) = uri.strip_prefix("dispatch://task/") {
        return parse_canonical_id(id).map(ContextUri::Task);
    }
    let rest = uri.strip_prefix("skill://")?;
    let (name, path) = rest.split_once('/')?;
    if !is_plain_segment(name) {
        return None;
    }
    if path == "SKILL.md" {
        return Some(ContextUri::Skill {
            name: name.to_string(),
        });
    }
    let file = path.strip_prefix("references/")?;
    if !is_plain_segment(file) {
        return None;
    }
    Some(ContextUri::SkillRef {
        name: name.to_string(),
        file: file.to_string(),
    })
}

impl ContextUri {
    fn sort_key(&self) -> SortKey {
        match self {
            ContextUri::OwnTask => SortKey::OwnTask,
            // Never listed, so a cursor naming one resumes where the own
            // task would sit.
            ContextUri::Task(_) => SortKey::OwnTask,
            ContextUri::Skill { name } => SortKey::Skill(name.clone(), None),
            ContextUri::SkillRef { name, file } => SortKey::Skill(name.clone(), Some(file.clone())),
            ContextUri::Learning(id) => SortKey::Learning(*id),
        }
    }
}

// ---------------------------------------------------------------------------
// Cursor
// ---------------------------------------------------------------------------

/// An opaque cursor: the hex of the last returned entry's URI. It records a
/// position in the stable order, not an offset.
fn encode_cursor(uri: &str) -> String {
    uri.bytes().map(|b| format!("{b:02x}")).collect()
}

fn decode_cursor(cursor: &str) -> Option<SortKey> {
    if cursor.is_empty() || !cursor.len().is_multiple_of(2) || !cursor.is_ascii() {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..cursor.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&cursor[i..i + 2], 16).ok())
        .collect();
    let uri = String::from_utf8(bytes?).ok()?;
    parse_context_uri(&uri).map(|u| u.sort_key())
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

enum ReadError {
    /// The URI does not resolve (`RejectUnresolvedContextUri`).
    Unresolved(String),
    /// A resolvable resource's text could not be produced.
    Internal(String),
}

impl ReadError {
    fn into_response(self, id: Option<Value>) -> JsonRpcResponse {
        match self {
            ReadError::Unresolved(m) => JsonRpcResponse::err(id, INVALID_PARAMS, m),
            ReadError::Internal(m) => JsonRpcResponse::err(id, INTERNAL_ERROR, m),
        }
    }
}

fn unresolved_message(uri: &str) -> String {
    format!("Unknown resource: {uri}")
}

fn no_own_task_message(uri: &str) -> String {
    format!(
        "Unknown resource: {uri}. The caller has no own task to read: it is not a dispatched \
         agent, or its task no longer exists"
    )
}

fn learning_text(summary: &str, detail: Option<&str>) -> String {
    match detail {
        Some(d) if !d.trim().is_empty() => format!("{summary}\n\n{d}"),
        _ => summary.to_string(),
    }
}

/// `get_task`'s text for a task; a missing task is refused with `missing`.
async fn task_text(
    state: &McpState,
    id: crate::models::TaskId,
    missing: impl FnOnce() -> String,
) -> Result<(&'static str, String), ReadError> {
    match state.task_svc.get_task(id).await {
        Ok(task) => Ok((
            MIME_TEXT,
            super::tasks::task_detail_text(state, &task).await,
        )),
        Err(crate::service::ServiceError::NotFound(_)) => Err(ReadError::Unresolved(missing())),
        Err(e) => Err(ReadError::Internal(e.to_string())),
    }
}

/// Resolve `uri` for `identity` and produce `(mimeType, text)`.
async fn resolve_context(
    state: &McpState,
    identity: &CallerIdentity,
    uri: &str,
) -> Result<(&'static str, String), ReadError> {
    let unresolved = || ReadError::Unresolved(unresolved_message(uri));
    let parsed = parse_context_uri(uri).ok_or_else(unresolved)?;
    match parsed {
        ContextUri::Skill { name } => built_in_skills()
            .iter()
            .find(|s| s.name == name)
            .map(|s| (MIME_MARKDOWN, s.skill_md.to_string()))
            .ok_or_else(unresolved),
        ContextUri::SkillRef { name, file } => built_in_skills()
            .iter()
            .find(|s| s.name == name)
            .and_then(|s| s.references.iter().find(|(f, _)| *f == file))
            .map(|(_, text)| (MIME_MARKDOWN, text.to_string()))
            .ok_or_else(unresolved),
        ContextUri::Learning(id) => {
            let learning = state
                .db
                .get_learning(LearningId(id))
                .await
                .map_err(|e| ReadError::Internal(e.to_string()))?;
            match learning {
                Some(l) if l.status == LearningStatus::Approved => {
                    Ok((MIME_TEXT, learning_text(&l.summary, l.detail.as_deref())))
                }
                _ => Err(unresolved()),
            }
        }
        ContextUri::Task(id) => {
            task_text(state, crate::models::TaskId(id), || unresolved_message(uri)).await
        }
        ContextUri::OwnTask => {
            let CallerIdentity::Task(task_id) = identity else {
                return Err(ReadError::Unresolved(no_own_task_message(uri)));
            };
            task_text(state, *task_id, || no_own_task_message(uri)).await
        }
    }
}

// ---------------------------------------------------------------------------
// Listing
// ---------------------------------------------------------------------------

/// A learning's short title: its summary's first line, cut at 80 characters.
fn learning_title(summary: &str) -> String {
    const MAX: usize = 80;
    let first = summary.lines().next().unwrap_or("").trim();
    if first.chars().count() <= MAX {
        return first.to_string();
    }
    let cut: String = first.chars().take(MAX).collect();
    format!("{}…", cut.trim_end())
}

/// Every listable entry for the caller, in the stable order.
async fn list_entries(
    state: &McpState,
    identity: &CallerIdentity,
) -> Result<Vec<(SortKey, Value)>, String> {
    let mut entries: Vec<(SortKey, Value)> = Vec::new();

    if let CallerIdentity::Task(task_id) = identity {
        if let Some(task) = state
            .db
            .get_task(*task_id)
            .await
            .map_err(|e| e.to_string())?
        {
            entries.push((
                SortKey::OwnTask,
                json!({
                    "uri": TASK_SELF_URI,
                    "name": "Your task",
                    "description": task.title,
                    "mimeType": MIME_TEXT,
                }),
            ));
        }
    }

    for skill in built_in_skills() {
        entries.push((
            SortKey::Skill(skill.name.clone(), None),
            json!({
                "uri": format!("skill://{}/SKILL.md", skill.name),
                "name": skill.name,
                "description": skill.description,
                "mimeType": MIME_MARKDOWN,
            }),
        ));
        for (file, _) in &skill.references {
            entries.push((
                SortKey::Skill(skill.name.clone(), Some(file.clone())),
                json!({
                    "uri": format!("skill://{}/references/{file}", skill.name),
                    "name": format!("{}: {file}", skill.name),
                    "mimeType": MIME_MARKDOWN,
                }),
            ));
        }
    }

    let mut learnings = state
        .db
        .list_learnings(crate::db::LearningFilter {
            status: Some(LearningStatus::Approved),
            ..Default::default()
        })
        .await
        .map_err(|e| e.to_string())?;
    learnings.sort_by_key(|l| l.id.0);
    for l in learnings {
        entries.push((
            SortKey::Learning(l.id.0),
            json!({
                "uri": format!("dispatch://learnings/{}", l.id.0),
                "name": learning_title(&l.summary),
                "description": l.summary.lines().next().unwrap_or("").trim(),
                "mimeType": MIME_TEXT,
            }),
        ));
    }
    Ok(entries)
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

fn require_identity<'a>(
    id: &Option<Value>,
    identity: &'a Result<CallerIdentity, IdentityError>,
) -> Result<&'a CallerIdentity, JsonRpcResponse> {
    identity
        .as_ref()
        .map_err(|e| JsonRpcResponse::err(id.clone(), INVALID_REQUEST, e.to_string()))
}

#[derive(Deserialize)]
struct ListParams {
    #[serde(default)]
    cursor: Option<Value>,
}

/// `resources/list`.
pub(super) async fn handle_resources_list(
    state: &McpState,
    id: Option<Value>,
    identity: &Result<CallerIdentity, IdentityError>,
    params: Option<Value>,
) -> JsonRpcResponse {
    let identity = match require_identity(&id, identity) {
        Ok(i) => i,
        Err(resp) => return resp,
    };
    let params: ListParams = match parse_args(&id, params.unwrap_or_else(|| json!({}))) {
        Ok(p) => p,
        Err(resp) => return resp,
    };
    let after = match params.cursor {
        None | Some(Value::Null) => None,
        Some(Value::String(c)) => match decode_cursor(&c) {
            Some(key) => Some(key),
            None => return JsonRpcResponse::err(id, INVALID_PARAMS, "Invalid cursor"),
        },
        Some(_) => return JsonRpcResponse::err(id, INVALID_PARAMS, "Invalid cursor"),
    };

    let entries = match list_entries(state, identity).await {
        Ok(e) => e,
        Err(m) => return JsonRpcResponse::err(id, INTERNAL_ERROR, m),
    };
    let mut remaining: Vec<Value> = entries
        .into_iter()
        .filter(|(key, _)| after.as_ref().is_none_or(|a| key > a))
        .map(|(_, v)| v)
        .collect();
    let has_more = remaining.len() > CONTEXT_LIST_PAGE_SIZE;
    remaining.truncate(CONTEXT_LIST_PAGE_SIZE);

    let next_cursor = has_more
        .then(|| {
            remaining
                .last()
                .and_then(|e| e["uri"].as_str())
                .map(encode_cursor)
        })
        .flatten();
    let mut result = json!({ "resources": remaining });
    if let Some(c) = next_cursor {
        result["nextCursor"] = json!(c);
    }
    JsonRpcResponse::ok(id, result)
}

#[derive(Deserialize)]
struct ReadParams {
    uri: String,
}

/// `resources/read`.
pub(super) async fn handle_resources_read(
    state: &McpState,
    id: Option<Value>,
    identity: &Result<CallerIdentity, IdentityError>,
    params: Option<Value>,
) -> JsonRpcResponse {
    let identity = match require_identity(&id, identity) {
        Ok(i) => i,
        Err(resp) => return resp,
    };
    let params: ReadParams = match parse_args(&id, params.unwrap_or(Value::Null)) {
        Ok(p) => p,
        Err(resp) => return resp,
    };
    match resolve_context(state, identity, &params.uri).await {
        Ok((mime, text)) => JsonRpcResponse::ok(
            id,
            json!({ "contents": [{ "uri": params.uri, "mimeType": mime, "text": text }] }),
        ),
        Err(e) => e.into_response(id),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReadContextArgs {
    uri: String,
}

/// `read_context`, the tool twin of `resources/read`. A refusal is returned as
/// a JSON-RPC error here; `tools/call` re-wraps it as an `isError` result.
pub(crate) async fn handle_read_context(
    state: &McpState,
    id: Option<Value>,
    identity: &CallerIdentity,
    args: Value,
) -> JsonRpcResponse {
    let args: ReadContextArgs = match parse_args(&id, args) {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    tracing::info!(uri = %args.uri, "MCP read_context");
    match resolve_context(state, identity, &args.uri).await {
        Ok((_, text)) => {
            JsonRpcResponse::ok(id, json!({ "content": [{ "type": "text", "text": text }] }))
        }
        Err(e) => e.into_response(id),
    }
}
