//! mcp-task-tools.allium: "MCP context resources" and the
//! `McpContextResources` surface — `resources/list`, `resources/read` and their
//! tool twin `read_context`.
//!
//! Every test drives the JSON-RPC surface through `handle_mcp`, so nothing here
//! depends on how the handlers are factored. The skill fixtures are read from
//! `plugin/skills` on disk, which is the source the built-in copy is embedded
//! from at compile time (SkillsComeFromTheBuiltInCopy).

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use super::*;

/// config.context_list_page_size's declared default. The config has no
/// test-settable seam, so the pagination tests seed enough learnings to
/// overflow the default rather than shrinking it.
const CONTEXT_LIST_PAGE_SIZE: usize = 50;

/// Upper bound on pages a listing walk follows, so a cursor that never ends
/// fails the test instead of hanging it.
const MAX_PAGES: usize = 100;

const TASK_SELF: &str = "dispatch://task/self";

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

struct SkillOnDisk {
    name: String,
    description: String,
    skill_md: String,
    /// `(file name, text)`, by file name ascending.
    references: Vec<(String, String)>,
}

impl SkillOnDisk {
    fn skill_uri(&self) -> String {
        format!("skill://{}/SKILL.md", self.name)
    }
    fn reference_uri(&self, file: &str) -> String {
        format!("skill://{}/references/{file}", self.name)
    }
}

fn skills_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("plugin/skills")
}

/// The frontmatter `name` and `description` of a SKILL.md. Handles the plain
/// scalar and the folded (`>-`) form, which are the two the skills use.
fn frontmatter(skill_md: &str) -> (String, String) {
    let body = skill_md
        .strip_prefix("---\n")
        .expect("SKILL.md must open with frontmatter");
    let end = body.find("\n---").expect("frontmatter must close");
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
    (
        name.expect("frontmatter must carry a name"),
        description.expect("frontmatter must carry a description"),
    )
}

/// Every skill in plugin/skills, by frontmatter name ascending.
fn skills_on_disk() -> Vec<SkillOnDisk> {
    let mut skills = Vec::new();
    for entry in std::fs::read_dir(skills_root()).unwrap() {
        let dir = entry.unwrap().path();
        let skill_md_path = dir.join("SKILL.md");
        if !skill_md_path.is_file() {
            continue;
        }
        let skill_md = std::fs::read_to_string(&skill_md_path).unwrap();
        let (name, description) = frontmatter(&skill_md);
        let mut references = Vec::new();
        let refs_dir = dir.join("references");
        if refs_dir.is_dir() {
            for r in std::fs::read_dir(&refs_dir).unwrap() {
                let p = r.unwrap().path();
                let file = p.file_name().unwrap().to_str().unwrap().to_string();
                if p.is_file() && file.ends_with(".md") {
                    references.push((file, std::fs::read_to_string(&p).unwrap()));
                }
            }
        }
        references.sort_by(|a, b| a.0.cmp(&b.0));
        skills.push(SkillOnDisk {
            name,
            description,
            skill_md,
            references,
        });
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    assert!(
        skills.iter().any(|s| !s.references.is_empty()),
        "fixture expects at least one skill with a references/ file"
    );
    skills
}

/// The skill section of the listing: each SKILL.md, then its references.
fn expected_skill_uris() -> Vec<String> {
    let mut uris = Vec::new();
    for s in skills_on_disk() {
        uris.push(s.skill_uri());
        for (file, _) in &s.references {
            uris.push(s.reference_uri(file));
        }
    }
    uris
}

async fn new_task(state: &Arc<McpState>, title: &str) -> crate::models::TaskId {
    new_task_in(state, title, TaskStatus::Running).await
}

/// A task that existed and has been deleted: only a Done task can be deleted,
/// so it is created Done.
async fn deleted_task(state: &Arc<McpState>, title: &str) -> crate::models::TaskId {
    let task = new_task_in(state, title, TaskStatus::Done).await;
    state.db_write().delete_task(task).await.unwrap();
    task
}

async fn new_task_in(
    state: &Arc<McpState>,
    title: &str,
    status: TaskStatus,
) -> crate::models::TaskId {
    state
        .db_write()
        .create_task(CreateTaskRequest {
            title,
            description: "context resources fixture task",
            repo_path: "/repo/context",
            plan: None,
            status,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap()
}

async fn new_learning(
    state: &Arc<McpState>,
    summary: &str,
    detail: Option<&str>,
    scope: crate::models::LearningScope,
    scope_ref: Option<&str>,
    tags: &[&str],
    status: crate::models::LearningStatus,
) -> crate::models::LearningId {
    let tag_strings: Vec<String> = tags.iter().map(|s| s.to_string()).collect();
    let emb: Vec<u8> = serialize_embedding(&vec![0.1f32; 384]);
    let id = state
        .db
        .create_learning(CreateLearningRow {
            kind: crate::models::LearningKind::Convention,
            summary,
            detail,
            scope,
            scope_ref,
            tags: &tag_strings,
            source_task_id: None,
            embedding: Some(&emb),
        })
        .await
        .unwrap();
    state
        .db
        .patch_learning(id, &crate::db::LearningPatch::new().status(status))
        .await
        .unwrap();
    id
}

async fn approved_learning(state: &Arc<McpState>, summary: &str) -> crate::models::LearningId {
    new_learning(
        state,
        summary,
        None,
        crate::models::LearningScope::User,
        None,
        &[],
        crate::models::LearningStatus::Approved,
    )
    .await
}

async fn set_learning_status(
    state: &Arc<McpState>,
    id: crate::models::LearningId,
    status: crate::models::LearningStatus,
) {
    state
        .db
        .patch_learning(id, &crate::db::LearningPatch::new().status(status))
        .await
        .unwrap();
}

fn learning_uri(id: crate::models::LearningId) -> String {
    format!("dispatch://learnings/{}", id.0)
}

/// The approved learnings' URIs, by id ascending — the learnings section.
async fn expected_learning_uris(state: &Arc<McpState>) -> Vec<String> {
    let mut learnings = state
        .db
        .list_learnings(crate::db::LearningFilter {
            status: Some(crate::models::LearningStatus::Approved),
            ..Default::default()
        })
        .await
        .unwrap();
    learnings.sort_by_key(|l| l.id.0);
    learnings.into_iter().map(|l| learning_uri(l.id)).collect()
}

/// The full expected listing for a caller, in the spec's order.
async fn expected_listing(state: &Arc<McpState>, lists_own_task: bool) -> Vec<String> {
    let mut uris = Vec::new();
    if lists_own_task {
        uris.push(TASK_SELF.to_string());
    }
    uris.extend(expected_skill_uris());
    uris.extend(expected_learning_uris(state).await);
    uris
}

// ---------------------------------------------------------------------------
// Entry-point helpers
// ---------------------------------------------------------------------------

type Identity = Result<CallerIdentity, IdentityError>;

async fn list_page(
    state: &Arc<McpState>,
    identity: Identity,
    cursor: Option<Value>,
) -> JsonRpcResponse {
    let params = match cursor {
        Some(c) => json!({ "cursor": c }),
        None => json!({}),
    };
    call_with_identity(state, "resources/list", Some(params), identity).await
}

/// The `resources` array of a successful list page, and its nextCursor.
fn page_entries(resp: &JsonRpcResponse) -> (Vec<Value>, Option<Value>) {
    let result = resp
        .result
        .as_ref()
        .unwrap_or_else(|| panic!("resources/list must succeed, got error: {:?}", resp.error));
    let entries = result["resources"]
        .as_array()
        .unwrap_or_else(|| panic!("resources/list result must carry a resources array: {result}"))
        .clone();
    let next = result.get("nextCursor").filter(|c| !c.is_null()).cloned();
    (entries, next)
}

/// Walk every page. Returns each page's entries.
async fn list_all_pages(state: &Arc<McpState>, identity: CallerIdentity) -> Vec<Vec<Value>> {
    let mut pages = Vec::new();
    let mut cursor = None;
    for _ in 0..MAX_PAGES {
        let resp = list_page(state, Ok(identity.clone()), cursor.take()).await;
        let (entries, next) = page_entries(&resp);
        pages.push(entries);
        match next {
            Some(c) => cursor = Some(c),
            None => return pages,
        }
    }
    panic!("resources/list did not terminate within {MAX_PAGES} pages");
}

async fn list_all(state: &Arc<McpState>, identity: CallerIdentity) -> Vec<Value> {
    list_all_pages(state, identity)
        .await
        .into_iter()
        .flatten()
        .collect()
}

fn uris(entries: &[Value]) -> Vec<String> {
    entries
        .iter()
        .map(|e| {
            e["uri"]
                .as_str()
                .unwrap_or_else(|| panic!("listed entry must carry a uri: {e}"))
                .to_string()
        })
        .collect()
}

async fn resources_read(state: &Arc<McpState>, identity: Identity, uri: &str) -> JsonRpcResponse {
    call_with_identity(
        state,
        "resources/read",
        Some(json!({ "uri": uri })),
        identity,
    )
    .await
}

async fn read_context(state: &Arc<McpState>, identity: Identity, uri: &str) -> JsonRpcResponse {
    call_with_identity(
        state,
        "tools/call",
        Some(json!({ "name": "read_context", "arguments": { "uri": uri } })),
        identity,
    )
    .await
}

/// The one content item of a successful resources/read.
fn read_content(resp: &JsonRpcResponse, uri: &str) -> Value {
    let result = resp.result.as_ref().unwrap_or_else(|| {
        panic!(
            "resources/read({uri}) must succeed, got error: {:?}",
            resp.error
        )
    });
    let contents = result["contents"]
        .as_array()
        .unwrap_or_else(|| panic!("resources/read({uri}) must carry contents: {result}"));
    assert_eq!(
        contents.len(),
        1,
        "resources/read({uri}) carries one content item, got {contents:?}"
    );
    let item = contents[0].clone();
    assert_eq!(
        item["uri"],
        json!(uri),
        "content item must name the URI read"
    );
    item
}

/// The outcome of one read, comparable across the two entry points.
#[derive(Debug, PartialEq)]
enum ReadOutcome {
    Text(String),
    /// A JSON-RPC error: code and message.
    RpcError(i64, String),
    /// A tools/call `isError` result carrying the error text.
    ToolError(String),
}

/// What read_context must answer, given what resources/read answered for the
/// same URI and caller (TheToolTwinServesTheSameText): the same text on
/// success; the same -32600 JSON-RPC error for a missing or malformed
/// identity; and for -32602 / -32603 an `isError` tool result carrying the
/// same message.
fn expected_twin_outcome(via_read: &ReadOutcome) -> ReadOutcome {
    match via_read {
        ReadOutcome::Text(t) => ReadOutcome::Text(t.clone()),
        ReadOutcome::RpcError(-32600, m) => ReadOutcome::RpcError(-32600, m.clone()),
        ReadOutcome::RpcError(-32602 | -32603, m) => ReadOutcome::ToolError(m.clone()),
        other => panic!("resources/read answered outside the spec: {other:?}"),
    }
}

fn resources_read_outcome(resp: &JsonRpcResponse) -> ReadOutcome {
    if let Some(err) = &resp.error {
        return ReadOutcome::RpcError(err.code.into(), err.message.clone());
    }
    let result = resp.result.as_ref().unwrap();
    ReadOutcome::Text(
        result["contents"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("resources/read result must carry text: {result}"))
            .to_string(),
    )
}

fn read_context_outcome(resp: &JsonRpcResponse) -> ReadOutcome {
    if let Some(err) = &resp.error {
        return ReadOutcome::RpcError(err.code.into(), err.message.clone());
    }
    let result = resp.result.as_ref().unwrap();
    let text = result["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string();
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        return ReadOutcome::ToolError(text);
    }
    ReadOutcome::Text(text)
}

/// Assert `resp` is the JSON-RPC error `code`, echoes request id 1, and
/// return its message.
fn expect_rpc_error(resp: &JsonRpcResponse, code: i64, what: &str) -> String {
    let err = resp.error.as_ref().unwrap_or_else(|| {
        panic!(
            "{what}: expected JSON-RPC error {code}, got {:?}",
            resp.result
        )
    });
    assert_eq!(
        i64::from(err.code),
        code,
        "{what}: wrong code, message {:?}",
        err.message
    );
    assert_eq!(
        resp.id,
        Some(json!(1)),
        "{what}: error must echo the request id"
    );
    err.message.clone()
}

/// Both entry points refuse `uri` with one message naming it: resources/read
/// as JSON-RPC -32602, read_context as an `isError` tool result.
async fn assert_unknown_on_both(
    state: &Arc<McpState>,
    identity: CallerIdentity,
    uri: &str,
) -> String {
    let via_read = resources_read(state, Ok(identity.clone()), uri).await;
    let msg = expect_rpc_error(&via_read, -32602, &format!("resources/read({uri})"));
    assert!(
        msg.contains(uri),
        "refusal must name the URI {uri:?}, got {msg:?}"
    );
    let via_tool = read_context(state, Ok(identity.clone()), uri).await;
    assert_eq!(
        read_context_outcome(&via_tool),
        ReadOutcome::ToolError(msg.clone()),
        "read_context({uri}) must refuse with an isError result carrying resources/read's message"
    );
    msg
}

fn malformed_identities() -> Vec<(&'static str, IdentityError)> {
    vec![
        ("missing", IdentityError::Missing),
        ("conflict", IdentityError::Conflict),
        (
            "unknown kind",
            IdentityError::UnknownKind("robot".to_string()),
        ),
        (
            "invalid task id",
            IdentityError::InvalidTaskId("abc".to_string()),
        ),
    ]
}

// ---------------------------------------------------------------------------
// DeclareResourcesCapability / ResourcesCapabilityIsDeclared
// ---------------------------------------------------------------------------

/// rule-success.DeclareResourcesCapability: initialize declares tools and
/// resources; resources declares neither subscribe nor listChanged
/// (ContextResourcesAreReadOnly); the skills extension is not declared.
#[tokio::test]
async fn initialize_declares_resources_beside_tools_without_notifications_or_skills_extension() {
    let state = test_state().await;
    let resp = call(&state, "initialize", None).await;
    let result = resp.result.expect("initialize must succeed");
    let caps = &result["capabilities"];
    assert!(
        caps["tools"].is_object(),
        "tools must stay declared: {caps}"
    );
    let resources = caps["resources"]
        .as_object()
        .unwrap_or_else(|| panic!("initialize must declare the resources capability: {caps}"));
    for flag in ["subscribe", "listChanged"] {
        assert!(
            resources.get(flag).and_then(Value::as_bool) != Some(true),
            "resources must not declare {flag}: {caps}"
        );
    }
    assert!(
        !result
            .to_string()
            .contains("io.modelcontextprotocol/skills"),
        "the skills extension must not be declared: {result}"
    );
}

/// IdentityIsRequired / DeclareResourcesCapability: initialize stays
/// identity-free — the declared capabilities do not depend on the caller.
#[tokio::test]
async fn initialize_without_identity_still_declares_resources() {
    let state = test_state().await;
    for (label, err) in malformed_identities() {
        let resp = call_with_identity(&state, "initialize", None, Err(err)).await;
        let result = resp
            .result
            .unwrap_or_else(|| panic!("initialize ({label} identity) must succeed"));
        assert!(
            result["capabilities"]["resources"].is_object(),
            "initialize ({label} identity) must declare resources: {result}"
        );
    }
}

/// ContextResourcesAreReadOnly: there is no resource write method and no
/// subscription method.
#[tokio::test]
async fn no_resource_subscribe_or_write_method_is_answered() {
    let state = test_state().await;
    for method in [
        "resources/subscribe",
        "resources/unsubscribe",
        "resources/write",
    ] {
        let resp = call(&state, method, Some(json!({ "uri": TASK_SELF }))).await;
        expect_rpc_error(&resp, -32601, method);
    }
}

// ---------------------------------------------------------------------------
// read_context in tools/list
// ---------------------------------------------------------------------------

/// ReadContextViaMcp: read_context is a listed tool and `uri` is required.
/// tools/list stays identity-free, so it is asserted without one too.
#[tokio::test]
async fn tools_list_advertises_read_context_with_a_required_uri() {
    let state = test_state().await;
    for identity in [Ok(CallerIdentity::Session), Err(IdentityError::Missing)] {
        let resp = call_with_identity(&state, "tools/list", None, identity).await;
        let result = resp.result.expect("tools/list must succeed");
        let tool = result["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "read_context")
            .unwrap_or_else(|| panic!("tools/list must advertise read_context"))
            .clone();
        let schema = &tool["inputSchema"];
        assert!(
            schema["properties"]["uri"].is_object(),
            "read_context must declare a uri argument: {schema}"
        );
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap_or_else(|| panic!("read_context must declare required arguments: {schema}"))
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(
            required.contains(&"uri"),
            "uri must be required, got {required:?}"
        );
        // Description standard: it names the URI shapes it reads.
        let desc = tool["description"].as_str().unwrap();
        for shape in ["skill://", "dispatch://learnings/", TASK_SELF] {
            assert!(
                desc.contains(shape),
                "read_context description must name the {shape} URI shape, got: {desc}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// IdentityIsRequired
// ---------------------------------------------------------------------------

/// rule-failure.ListContextResourcesViaMcp.1, rule-failure.ReadContextResourceViaMcp.1,
/// rule-failure.ReadContextViaMcp.1: all three entry points refuse a missing or
/// malformed caller identity with -32600, echoing the request id, before the
/// URI is looked at (a resolvable URI is refused too).
#[tokio::test]
async fn every_context_entry_point_refuses_a_missing_or_malformed_identity() {
    let state = test_state().await;
    let skill_uri = expected_skill_uris().remove(0);
    for (label, err) in malformed_identities() {
        let resp = list_page(&state, Err(err.clone()), None).await;
        expect_rpc_error(&resp, -32600, &format!("resources/list ({label})"));

        let resp = resources_read(&state, Err(err.clone()), &skill_uri).await;
        let read_msg = expect_rpc_error(&resp, -32600, &format!("resources/read ({label})"));

        let resp = read_context(&state, Err(err.clone()), &skill_uri).await;
        let tool_msg = expect_rpc_error(&resp, -32600, &format!("read_context ({label})"));
        assert_eq!(
            read_msg, tool_msg,
            "both reads refuse a {label} identity alike"
        );
    }
}

// ---------------------------------------------------------------------------
// ListContextResourcesViaMcp — contents and order
// ---------------------------------------------------------------------------

/// rule-success.ListContextResourcesViaMcp / ListingPagesHaveNoGapsOrRepeats:
/// a dispatched agent sees its own task first, then skills by name (SKILL.md
/// before its references, by file name), then approved learnings by id.
#[tokio::test]
async fn task_caller_listing_is_own_task_then_skills_then_learnings_in_order() {
    let state = test_state().await;
    let task = new_task(&state, "Caller task").await;
    // Created out of summary order, so id order and alphabetical order differ.
    approved_learning(&state, "Zebra learning").await;
    approved_learning(&state, "Alpha learning").await;
    approved_learning(&state, "Middle learning").await;

    let listed = uris(&list_all(&state, CallerIdentity::Task(task)).await);
    assert_eq!(listed, expected_listing(&state, true).await);
}

/// A Session caller has no own task, so dispatch://task/self is not listed.
#[tokio::test]
async fn session_caller_listing_omits_task_self() {
    let state = test_state().await;
    new_task(&state, "Someone else's task").await;
    approved_learning(&state, "A learning").await;

    let listed = uris(&list_all(&state, CallerIdentity::Session).await);
    assert_eq!(listed, expected_listing(&state, false).await);
}

/// has_own_task requires the task to exist: a caller whose task was deleted
/// does not see dispatch://task/self.
#[tokio::test]
async fn caller_whose_task_was_deleted_is_not_listed_task_self() {
    let state = test_state().await;
    let task = deleted_task(&state, "Doomed").await;

    let listed = uris(&list_all(&state, CallerIdentity::Task(task)).await);
    assert_eq!(listed, expected_listing(&state, false).await);
}

/// Only validated learnings are listed, and all of them — whatever their
/// scope. Archived and rejected learnings are not.
#[tokio::test]
async fn listing_carries_every_approved_learning_whatever_its_scope_and_no_other() {
    let state = test_state().await;
    let task = new_task(&state, "Caller").await;
    use crate::models::{LearningScope as S, LearningStatus as St};
    let user = new_learning(
        &state,
        "User scoped",
        None,
        S::User,
        None,
        &[],
        St::Approved,
    )
    .await;
    let other_repo = new_learning(
        &state,
        "Other repo scoped",
        None,
        S::Repo,
        Some("/somewhere/else"),
        &[],
        St::Approved,
    )
    .await;
    let other_task = new_learning(
        &state,
        "Other task scoped",
        None,
        S::Task,
        Some("99999"),
        &[],
        St::Approved,
    )
    .await;
    let archived = new_learning(&state, "Archived", None, S::User, None, &[], St::Archived).await;
    let rejected = new_learning(&state, "Rejected", None, S::User, None, &[], St::Rejected).await;

    let listed = uris(&list_all(&state, CallerIdentity::Task(task)).await);
    for id in [user, other_repo, other_task] {
        assert!(
            listed.contains(&learning_uri(id)),
            "approved {id:?} must be listed"
        );
    }
    for id in [archived, rejected] {
        assert!(
            !listed.contains(&learning_uri(id)),
            "unvalidated {id:?} must not be listed"
        );
    }
}

/// The entry metadata the listing guidance fixes: names, descriptions and
/// mime types per kind.
#[tokio::test]
async fn listed_entries_carry_the_specified_names_descriptions_and_mime_types() {
    let state = test_state().await;
    let task = new_task(&state, "The caller's title").await;
    let learning = new_learning(
        &state,
        "First line of the summary\nsecond line",
        Some("Detail text"),
        crate::models::LearningScope::User,
        None,
        &[],
        crate::models::LearningStatus::Approved,
    )
    .await;

    let entries = list_all(&state, CallerIdentity::Task(task)).await;
    let by_uri: HashMap<String, Value> = entries
        .iter()
        .map(|e| (e["uri"].as_str().unwrap().to_string(), e.clone()))
        .collect();

    let own = &by_uri[TASK_SELF];
    assert_eq!(own["mimeType"], "text/plain");
    assert_eq!(own["name"], "Your task");
    assert_eq!(own["description"], "The caller's title");

    for skill in skills_on_disk() {
        let e = &by_uri[&skill.skill_uri()];
        assert_eq!(e["mimeType"], "text/markdown", "{}", skill.name);
        assert_eq!(e["name"], json!(skill.name));
        assert_eq!(e["description"], json!(skill.description), "{}", skill.name);
        for (file, _) in &skill.references {
            let e = &by_uri[&skill.reference_uri(file)];
            assert_eq!(e["mimeType"], "text/markdown");
            assert_eq!(e["name"], json!(format!("{}: {file}", skill.name)));
            assert!(
                e.get("description").is_none_or(Value::is_null),
                "a reference entry carries no description: {e}"
            );
        }
    }

    let e = &by_uri[&learning_uri(learning)];
    assert_eq!(e["mimeType"], "text/plain");
    assert_eq!(e["description"], "First line of the summary");
    let name = e["name"].as_str().expect("a learning entry carries a name");
    let stem = name.trim_end_matches(['…', '.']).trim_end();
    assert!(
        !stem.is_empty() && "First line of the summary".starts_with(stem),
        "a learning's name is a short title cut from its summary, got {name:?}"
    );
}

// ---------------------------------------------------------------------------
// Pagination
// ---------------------------------------------------------------------------

/// Enough approved learnings that the listing overflows one page.
async fn seed_overflowing_learnings(state: &Arc<McpState>) -> Vec<crate::models::LearningId> {
    let mut ids = Vec::new();
    for i in 0..(CONTEXT_LIST_PAGE_SIZE + 10) {
        ids.push(approved_learning(state, &format!("Paged learning {i:03}")).await);
    }
    ids
}

/// config-default.context_list_page_size / ListContextResourcesViaMcp: a page
/// holds at most 50 entries, a full page carries nextCursor, the last page
/// carries none, and following the cursors returns the whole listing once.
#[tokio::test]
async fn listing_pages_hold_at_most_the_default_page_size_and_chain_by_next_cursor() {
    let state = test_state().await;
    let task = new_task(&state, "Pager").await;
    seed_overflowing_learnings(&state).await;
    let expected = expected_listing(&state, true).await;
    assert!(expected.len() > CONTEXT_LIST_PAGE_SIZE);

    let first = list_page(&state, Ok(CallerIdentity::Task(task)), None).await;
    let (first_entries, next) = page_entries(&first);
    assert_eq!(
        first_entries.len(),
        CONTEXT_LIST_PAGE_SIZE,
        "the first page of an overflowing listing holds exactly page_size entries"
    );
    assert!(
        next.is_some(),
        "a page with more remaining must carry nextCursor"
    );

    let pages = list_all_pages(&state, CallerIdentity::Task(task)).await;
    assert!(pages.len() >= 2);
    for page in &pages {
        assert!(
            page.len() <= CONTEXT_LIST_PAGE_SIZE,
            "page of {} entries",
            page.len()
        );
    }
    let all: Vec<Value> = pages.into_iter().flatten().collect();
    assert_eq!(
        uris(&all),
        expected,
        "paging returns every entry exactly once, in order"
    );
}

/// A listing that fits one page carries no nextCursor.
#[tokio::test]
async fn a_listing_that_fits_one_page_carries_no_next_cursor() {
    let state = test_state().await;
    approved_learning(&state, "Just one").await;
    let resp = list_page(&state, Ok(CallerIdentity::Session), None).await;
    let (entries, next) = page_entries(&resp);
    assert!(
        next.is_none(),
        "single-page listing must not carry nextCursor"
    );
    assert_eq!(uris(&entries), expected_listing(&state, false).await);
}

/// ListingPagesHaveNoGapsOrRepeats: the cursor records a position, not an
/// offset. Removing entries already returned, and adding new ones, between
/// pages skips and repeats nothing that existed throughout.
#[tokio::test]
async fn a_cursor_resumes_after_its_last_entry_when_learnings_change_between_pages() {
    let state = test_state().await;
    let task = new_task(&state, "Pager").await;
    let me = CallerIdentity::Task(task);
    let seeded = seed_overflowing_learnings(&state).await;

    let first = list_page(&state, Ok(me.clone()), None).await;
    let (page1, cursor) = page_entries(&first);
    let cursor = cursor.expect("an overflowing listing must carry nextCursor");
    let page1_uris = uris(&page1);
    let on_page1: Vec<_> = seeded
        .iter()
        .copied()
        .filter(|id| page1_uris.contains(&learning_uri(*id)))
        .collect();
    assert!(
        on_page1.len() >= 3,
        "fixture needs learnings on the first page"
    );

    // Between pages: delete one already-returned learning, archive another,
    // and record a new one.
    state.db.delete_learning(on_page1[0]).await.unwrap();
    set_learning_status(&state, on_page1[1], crate::models::LearningStatus::Archived).await;
    let added = approved_learning(&state, "Recorded between pages").await;
    let throughout: BTreeSet<String> = expected_listing(&state, true)
        .await
        .into_iter()
        .filter(|u| *u != learning_uri(added))
        .collect();

    let mut rest = Vec::new();
    let mut next = Some(cursor);
    for _ in 0..MAX_PAGES {
        let Some(c) = next.take() else { break };
        let resp = list_page(&state, Ok(me.clone()), Some(c)).await;
        let (entries, n) = page_entries(&resp);
        rest.extend(uris(&entries));
        next = n;
    }
    assert!(next.is_none(), "listing must terminate");

    let mut seen: Vec<String> = page1_uris.clone();
    seen.extend(rest);
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for u in &seen {
        *counts.entry(u.as_str()).or_default() += 1;
    }
    for u in &throughout {
        assert_eq!(
            counts.get(u.as_str()).copied().unwrap_or(0),
            1,
            "{u} existed throughout and must appear exactly once across pages"
        );
    }
    assert!(
        counts.values().all(|&n| n == 1),
        "no entry may repeat across pages: {counts:?}"
    );
}

/// rule-failure.ListContextResourcesViaMcp.2: a cursor the server did not
/// issue, or cannot decode, is refused with -32602.
#[tokio::test]
async fn a_cursor_the_server_did_not_issue_is_invalid_params() {
    let state = test_state().await;
    for bad in [
        json!("%%% not a cursor %%%"),
        json!(42),
        json!({ "offset": 1 }),
    ] {
        let resp = list_page(&state, Ok(CallerIdentity::Session), Some(bad.clone())).await;
        expect_rpc_error(&resp, -32602, &format!("resources/list cursor {bad}"));
    }
}

// ---------------------------------------------------------------------------
// ResolveContextUri — what each kind serves
// ---------------------------------------------------------------------------

/// rule-success.ResolveContextUri (skill): SKILL.md byte for byte, frontmatter
/// included, as text/markdown — from the built-in copy of plugin/skills.
#[tokio::test]
async fn reading_a_skill_serves_its_skill_md_byte_for_byte() {
    let state = test_state().await;
    for skill in skills_on_disk() {
        let uri = skill.skill_uri();
        let resp = resources_read(&state, Ok(CallerIdentity::Session), &uri).await;
        let item = read_content(&resp, &uri);
        assert_eq!(item["mimeType"], "text/markdown", "{uri}");
        assert_eq!(
            item["text"],
            json!(skill.skill_md),
            "{uri} must be served byte for byte"
        );
    }
}

/// rule-success.ResolveContextUri (skill_ref): each references/ file byte for
/// byte, as text/markdown.
#[tokio::test]
async fn reading_a_skill_reference_serves_the_file_byte_for_byte() {
    let state = test_state().await;
    for skill in skills_on_disk() {
        for (file, text) in &skill.references {
            let uri = skill.reference_uri(file);
            let resp = resources_read(&state, Ok(CallerIdentity::Session), &uri).await;
            let item = read_content(&resp, &uri);
            assert_eq!(item["mimeType"], "text/markdown", "{uri}");
            assert_eq!(
                item["text"],
                json!(text),
                "{uri} must be served byte for byte"
            );
        }
    }
}

/// rule-success.ResolveContextUri (learning): summary then detail, as
/// text/plain, and nothing else of the row.
#[tokio::test]
async fn reading_a_learning_serves_its_summary_then_detail_and_nothing_else() {
    let state = test_state().await;
    let id = new_learning(
        &state,
        "Summary of the learning",
        Some("Detail of the learning"),
        crate::models::LearningScope::Repo,
        Some("/repo/hidden-scope-ref"),
        &["secret-tag"],
        crate::models::LearningStatus::Approved,
    )
    .await;
    let uri = learning_uri(id);
    let resp = resources_read(&state, Ok(CallerIdentity::Session), &uri).await;
    let item = read_content(&resp, &uri);
    assert_eq!(item["mimeType"], "text/plain");
    let text = item["text"].as_str().unwrap();
    assert!(
        text.starts_with("Summary of the learning"),
        "summary first: {text:?}"
    );
    let s = text.find("Summary of the learning").unwrap();
    let d = text
        .find("Detail of the learning")
        .unwrap_or_else(|| panic!("detail must follow the summary: {text:?}"));
    assert!(s < d);
    for leaked in [
        "/repo/hidden-scope-ref",
        "secret-tag",
        "convention",
        "approved",
    ] {
        assert!(
            !text.contains(leaked),
            "learning text must not carry {leaked:?}: {text:?}"
        );
    }
}

/// A learning without detail is served as its summary alone.
#[tokio::test]
async fn reading_a_learning_without_detail_serves_its_summary() {
    let state = test_state().await;
    let id = approved_learning(&state, "Only a summary").await;
    let uri = learning_uri(id);
    let resp = resources_read(&state, Ok(CallerIdentity::Session), &uri).await;
    let item = read_content(&resp, &uri);
    assert_eq!(item["text"].as_str().unwrap().trim(), "Only a summary");
}

/// rule-success.ResolveContextUri (own_task): the same detail get_task
/// renders for the caller's own task, as text/plain.
#[tokio::test]
async fn reading_task_self_serves_get_tasks_rendering_of_the_callers_task() {
    let state = test_state().await;
    let task = new_task(&state, "My own task").await;
    let me = CallerIdentity::Task(task);

    let get = call_as(
        &state,
        "tools/call",
        Some(json!({ "name": "get_task", "arguments": { "task_id": task.0 } })),
        me.clone(),
    )
    .await;
    let expected = extract_response_text(&get);

    let resp = resources_read(&state, Ok(me.clone()), TASK_SELF).await;
    let item = read_content(&resp, TASK_SELF);
    assert_eq!(item["mimeType"], "text/plain");
    assert_eq!(item["text"], json!(expected));
}

/// SelfIsAlwaysTheCaller: dispatch://task/self resolves from transport
/// identity, so two callers reading the same URI get their own tasks.
#[tokio::test]
async fn task_self_always_names_the_calling_task() {
    let state = test_state().await;
    let a = new_task(&state, "Task Alpha-7f3").await;
    let b = new_task(&state, "Task Bravo-9c1").await;
    for (me, mine, theirs) in [
        (a, "Task Alpha-7f3", "Task Bravo-9c1"),
        (b, "Task Bravo-9c1", "Task Alpha-7f3"),
    ] {
        let resp = resources_read(&state, Ok(CallerIdentity::Task(me)), TASK_SELF).await;
        let text = read_content(&resp, TASK_SELF)["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            text.contains(mine),
            "task/self for {me:?} must be its own task: {text}"
        );
        assert!(
            !text.contains(theirs),
            "task/self for {me:?} must not show another task"
        );
    }
}

// ---------------------------------------------------------------------------
// ListedMeansReadable / TheToolTwinServesTheSameText
// ---------------------------------------------------------------------------

/// ListedMeansReadable: every listed URI resolves on both entry points for
/// the same caller, and read_context returns exactly resources/read's text.
#[tokio::test]
async fn every_listed_uri_reads_the_same_text_through_both_entry_points() {
    let state = test_state().await;
    let task = new_task(&state, "Reader").await;
    approved_learning(&state, "Readable learning").await;
    new_learning(
        &state,
        "Learning with detail",
        Some("and its detail"),
        crate::models::LearningScope::User,
        None,
        &[],
        crate::models::LearningStatus::Approved,
    )
    .await;
    let me = CallerIdentity::Task(task);

    let listed = uris(&list_all(&state, me.clone()).await);
    assert_eq!(listed, expected_listing(&state, true).await);
    for uri in &listed {
        let via_read = resources_read_outcome(&resources_read(&state, Ok(me.clone()), uri).await);
        assert!(
            matches!(via_read, ReadOutcome::Text(_)),
            "listed {uri} must be readable: {via_read:?}"
        );
        let via_tool = read_context_outcome(&read_context(&state, Ok(me.clone()), uri).await);
        assert_eq!(
            via_tool, via_read,
            "read_context({uri}) must equal resources/read"
        );
    }
}

/// TheToolTwinServesTheSameText: for every URI and caller identity — present,
/// missing or malformed — read_context succeeds exactly when resources/read
/// does, with the same text; it fails with the same -32600 for identity, and
/// otherwise with an isError result carrying resources/read's message.
#[tokio::test]
async fn read_context_fails_exactly_when_resources_read_does_with_the_same_error() {
    let state = test_state().await;
    let task = new_task(&state, "Twin").await;
    let ok = approved_learning(&state, "Served").await;
    let archived = approved_learning(&state, "Archived later").await;
    set_learning_status(&state, archived, crate::models::LearningStatus::Archived).await;

    let mut sample: Vec<String> = vec![
        TASK_SELF.to_string(),
        learning_uri(ok),
        learning_uri(archived),
        "dispatch://learnings/987654".to_string(),
        "skill://no-such-skill/SKILL.md".to_string(),
        "skill://wrap-up/references/../../../Cargo.toml".to_string(),
        "https://example.com/".to_string(),
    ];
    sample.push(expected_skill_uris().remove(0));

    let mut identities: Vec<(String, Identity)> = vec![
        ("task".to_string(), Ok(CallerIdentity::Task(task))),
        ("session".to_string(), Ok(CallerIdentity::Session)),
    ];
    for (label, err) in malformed_identities() {
        identities.push((label.to_string(), Err(err)));
    }

    for uri in &sample {
        for (label, identity) in &identities {
            let via_read =
                resources_read_outcome(&resources_read(&state, identity.clone(), uri).await);
            let via_tool = read_context_outcome(&read_context(&state, identity.clone(), uri).await);
            assert!(
                !matches!(via_read, ReadOutcome::RpcError(-32601, _)),
                "resources/read must be answered ({uri}, {label}): {via_read:?}"
            );
            assert_eq!(
                via_tool,
                expected_twin_outcome(&via_read),
                "{uri} as {label}: the twin must agree"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// RejectUnresolvedContextUri / UnknownUrisAreInvalidParams
// ---------------------------------------------------------------------------

/// rule-success.RejectUnresolvedContextUri: an unrecognised URI — another
/// scheme, another dispatch:// path, a malformed id, a skill or reference
/// file the built-in copy lacks — is -32602 naming the URI, on both entry
/// points.
#[tokio::test]
async fn unrecognised_or_missing_uris_are_invalid_params_naming_the_uri() {
    let state = test_state().await;
    let task = new_task(&state, "Asker").await;
    for uri in [
        "https://example.com/SKILL.md",
        "dispatch://learnings/",
        "dispatch://learnings/abc",
        "dispatch://learnings/-1",
        "dispatch://learnings/1.5",
        "dispatch://learnings/424242",
        "dispatch://task/",
        "dispatch://task/self/extra",
        "dispatch://task/abc",
        "dispatch://task/-1",
        "dispatch://task/007",
        "dispatch://task/1.5",
        "dispatch://task/999999",
        "dispatch://nothing",
        "skill://no-such-skill/SKILL.md",
        "skill://wrap-up/references/no-such-file.md",
        "skill://wrap-up/skill.md",
        "skill://wrap-up/",
        "skill://allium-loop/prompt.md",
        "",
    ] {
        assert_unknown_on_both(&state, CallerIdentity::Task(task), uri).await;
    }
}

/// A built-in skill whose frontmatter the listing cannot parse would vanish
/// silently, so every directory under plugin/skills must be listed.
#[tokio::test]
async fn every_built_in_skill_directory_is_listed() {
    let state = test_state().await;
    let uris: Vec<String> = list_all(&state, CallerIdentity::Session)
        .await
        .iter()
        .map(|r| r["uri"].as_str().unwrap().to_string())
        .collect();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("plugin/skills");
    for entry in std::fs::read_dir(dir).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().to_string();
        let uri = format!("skill://{name}/SKILL.md");
        assert!(uris.contains(&uri), "{uri} missing from listing: {uris:?}");
    }
}

/// ListedMeansReadable: a learning is readable only at the URI the listing
/// emits, so a zero-padded id is not an alias for it.
#[tokio::test]
async fn a_zero_padded_learning_id_is_not_an_alias_of_the_listed_uri() {
    let state = test_state().await;
    let id = approved_learning(&state, "Padded").await;
    let padded = format!("dispatch://learnings/00{}", id.0);
    assert_unknown_on_both(&state, CallerIdentity::Session, &padded).await;
    let listed = learning_uri(id);
    let resp = resources_read(&state, Ok(CallerIdentity::Session), &listed).await;
    read_content(&resp, &listed);
}

/// SkillsComeFromTheBuiltInCopy / NeverServedDataIsUnreachable: a skill path
/// that escapes its skill's directory is unrecognised, not followed.
#[tokio::test]
async fn skill_paths_escaping_the_skill_directory_are_invalid_params() {
    let state = test_state().await;
    for uri in [
        "skill://wrap-up/../retro/SKILL.md",
        "skill://wrap-up/references/../SKILL.md",
        "skill://wrap-up/references/../../retro/SKILL.md",
        "skill://wrap-up/references/../../../../Cargo.toml",
        "skill://wrap-up/references/../../../../docs/specs/core.allium",
        "skill://wrap-up/references/%2e%2e/SKILL.md",
        "skill://wrap-up/references//etc/passwd",
        "skill://wrap-up/references/sub/pr.md",
        "skill://../docs/testing.md",
        "skill:///etc/passwd",
        "skill://wrap-up/references/../../../../../../../../etc/passwd",
    ] {
        assert_unknown_on_both(&state, CallerIdentity::Session, uri).await;
    }
}

/// rule-failure.ResolveContextUri.1: archived, rejected and deleted learnings
/// read as unknown — including one listed before it was deleted (resolution
/// is evaluated at read time).
#[tokio::test]
async fn unvalidated_or_deleted_learnings_are_invalid_params() {
    let state = test_state().await;
    let archived = approved_learning(&state, "Will be archived").await;
    let rejected = approved_learning(&state, "Will be rejected").await;
    let deleted = approved_learning(&state, "Will be deleted").await;

    let listed = uris(&list_all(&state, CallerIdentity::Session).await);
    for id in [archived, rejected, deleted] {
        assert!(
            listed.contains(&learning_uri(id)),
            "{id:?} is listed while approved"
        );
    }

    set_learning_status(&state, archived, crate::models::LearningStatus::Archived).await;
    set_learning_status(&state, rejected, crate::models::LearningStatus::Rejected).await;
    state.db.delete_learning(deleted).await.unwrap();

    for id in [archived, rejected, deleted] {
        assert_unknown_on_both(&state, CallerIdentity::Session, &learning_uri(id)).await;
    }
}

/// RejectUnresolvedContextUri: an unvalidated learning and one that never
/// existed get the same answer, apart from the URI it names.
#[tokio::test]
async fn an_unvalidated_learning_is_refused_like_one_that_never_existed() {
    let state = test_state().await;
    let archived = approved_learning(&state, "Archived").await;
    set_learning_status(&state, archived, crate::models::LearningStatus::Archived).await;
    let never = crate::models::LearningId(archived.0 + 1000);

    let a = learning_uri(archived);
    let n = learning_uri(never);
    let msg_archived = assert_unknown_on_both(&state, CallerIdentity::Session, &a).await;
    let msg_never = assert_unknown_on_both(&state, CallerIdentity::Session, &n).await;
    assert_eq!(
        msg_archived.replace(&a, "<URI>"),
        msg_never.replace(&n, "<URI>"),
        "the refusal must not reveal which learnings exist unserved"
    );
}

/// RejectUnresolvedContextUri: task/self from a caller without a task is
/// -32602 naming the URI, and says plainly the caller has no task.
#[tokio::test]
async fn task_self_without_a_caller_task_says_the_caller_has_no_task() {
    let state = test_state().await;
    let msg = assert_unknown_on_both(&state, CallerIdentity::Session, TASK_SELF).await;
    let lower = msg.to_lowercase();
    assert!(
        lower.contains("not a dispatched agent")
            || lower.contains("no own task")
            || lower.contains("no task"),
        "the refusal must say the caller has no task of its own, got {msg:?}"
    );
}

/// rule-success.ResolveContextUri (task): any caller reads any task by id and
/// gets get_task's rendering, as text/plain, on both entry points.
#[tokio::test]
async fn reading_a_task_by_id_serves_get_tasks_rendering_to_any_caller() {
    let state = test_state().await;
    let me = new_task(&state, "Asker 1c4").await;
    let other = new_task(&state, "Other task title 5be2").await;
    let uri = format!("dispatch://task/{}", other.0);

    let get = call_as(
        &state,
        "tools/call",
        Some(json!({ "name": "get_task", "arguments": { "task_id": other.0 } })),
        CallerIdentity::Task(me),
    )
    .await;
    let expected = extract_response_text(&get);

    for identity in [CallerIdentity::Task(me), CallerIdentity::Session] {
        let resp = resources_read(&state, Ok(identity.clone()), &uri).await;
        let item = read_content(&resp, &uri);
        assert_eq!(item["mimeType"], "text/plain");
        assert_eq!(item["text"], json!(expected));
        assert!(!item["text"].as_str().unwrap().contains("Asker 1c4"));

        let twin = call_as(
            &state,
            "tools/call",
            Some(json!({ "name": "read_context", "arguments": { "uri": uri } })),
            identity,
        )
        .await;
        assert_eq!(extract_response_text(&twin), expected);
    }
}

/// A task read by id is not listed, for any caller: the listing offers only
/// the caller's own task.
#[tokio::test]
async fn tasks_by_id_are_not_listed() {
    let state = test_state().await;
    let me = new_task(&state, "Me").await;
    let other = new_task(&state, "Other task title 5be2").await;
    for identity in [CallerIdentity::Task(me), CallerIdentity::Session] {
        let listed = uris(&list_all(&state, identity).await);
        assert!(
            !listed.contains(&format!("dispatch://task/{}", other.0)),
            "task by id must not be listed"
        );
    }
}

/// RejectUnresolvedContextUri: a task that was deleted reads as unknown.
#[tokio::test]
async fn a_deleted_task_by_id_is_invalid_params() {
    let state = test_state().await;
    let me = new_task(&state, "Asker").await;
    let gone = deleted_task(&state, "Gone soon").await;
    assert_unknown_on_both(
        &state,
        CallerIdentity::Task(me),
        &format!("dispatch://task/{}", gone.0),
    )
    .await;
}

/// context_uri_resolves is evaluated at read time: a caller whose task has
/// been deleted reads task/self as unknown.
#[tokio::test]
async fn task_self_for_a_deleted_task_is_invalid_params() {
    let state = test_state().await;
    let task = deleted_task(&state, "Gone soon").await;
    assert_unknown_on_both(&state, CallerIdentity::Task(task), TASK_SELF).await;
}

// ---------------------------------------------------------------------------
// ContextReadsAreNotRetrievals / ContextResourcesAreReadOnly
// ---------------------------------------------------------------------------

/// ContextReadsAreNotRetrievals: listing and reading a learning on both entry
/// points records no Retrieval and touches no counter, so rate_learning still
/// refuses it.
#[tokio::test]
async fn reading_a_learning_records_no_retrieval_and_leaves_rate_learning_guarded() {
    let state = test_state().await;
    let task = new_task(&state, "Rater").await;
    let me = CallerIdentity::Task(task);
    let id = approved_learning(&state, "Read but not retrieved").await;
    let uri = learning_uri(id);
    let before = state.db.get_learning(id).await.unwrap().unwrap();

    let listed = uris(&list_all(&state, me.clone()).await);
    assert!(listed.contains(&uri));
    let r = resources_read(&state, Ok(me.clone()), &uri).await;
    read_content(&r, &uri);
    let t = read_context(&state, Ok(me.clone()), &uri).await;
    assert!(
        matches!(read_context_outcome(&t), ReadOutcome::Text(_)),
        "read_context must serve {uri}"
    );

    let retrievals = state.db.list_retrievals_for_task(task).await.unwrap();
    assert!(
        retrievals.is_empty(),
        "context reads must record no retrieval: {retrievals:?}"
    );
    let after = state.db.get_learning(id).await.unwrap().unwrap();
    assert_eq!(after.upvote_count, before.upvote_count);
    assert_eq!(after.last_upvoted_at, before.last_upvoted_at);
    assert_eq!(after.updated_at, before.updated_at);

    let rate = call_as(
        &state,
        "tools/call",
        Some(json!({
            "name": "rate_learning",
            "arguments": { "learning_id": id.0, "task_id": task.0, "verdict": "helped" }
        })),
        me.clone(),
    )
    .await;
    assert_error(&rate, "retriev");
}

/// ContextResourcesAreReadOnly: listing and reading every kind writes no
/// task, epic or learning.
#[tokio::test]
async fn listing_and_reading_leave_tasks_epics_and_learnings_untouched() {
    let state = test_state().await;
    let task = new_task(&state, "Read-only").await;
    let me = CallerIdentity::Task(task);
    approved_learning(&state, "Unchanged").await;
    state
        .db_write()
        .create_epic("Epic", "", None)
        .await
        .unwrap();

    let snapshot = |s: Arc<McpState>| async move {
        let tasks = s.db.list_all().await.unwrap();
        let learnings =
            s.db.list_learnings(crate::db::LearningFilter::default())
                .await
                .unwrap();
        let epics = s.db.list_epics().await.unwrap();
        format!("{tasks:?}\n{learnings:?}\n{epics:?}")
    };
    let before = snapshot(state.clone()).await;

    let listed = uris(&list_all(&state, me.clone()).await);
    assert!(listed.contains(&TASK_SELF.to_string()));
    for uri in &listed {
        read_content(&resources_read(&state, Ok(me.clone()), uri).await, uri);
        assert!(matches!(
            read_context_outcome(&read_context(&state, Ok(me.clone()), uri).await),
            ReadOutcome::Text(_)
        ));
    }

    assert_eq!(
        snapshot(state.clone()).await,
        before,
        "no list or read may write domain state"
    );
}

// ---------------------------------------------------------------------------
// NeverServedDataIsUnreachable
// ---------------------------------------------------------------------------

/// NeverServedDataIsUnreachable: no URI reaches an epic, a
/// trajectory, usage data, the host identity, a store path or a repository
/// file — whatever the caller.
#[tokio::test]
async fn never_served_data_is_unreachable_under_any_uri() {
    let state = test_state().await;
    let me = new_task(&state, "Me").await;
    let other = new_task(&state, "Other task title 5be2").await;
    let epic = state
        .db_write()
        .create_epic("Hidden epic 77d", "", None)
        .await
        .unwrap();
    let data_dir = state.data_dir.display().to_string();

    let probes = vec![
        format!("dispatch://tasks/{}", other.0),
        "dispatch://tasks".to_string(),
        format!("dispatch://epic/{}", epic.id.0),
        format!("dispatch://epics/{}", epic.id.0),
        format!("dispatch://trajectory/{}", me.0),
        format!("dispatch://trajectories/{}", other.0),
        "dispatch://usage".to_string(),
        "dispatch://host".to_string(),
        "dispatch://host.json".to_string(),
        "dispatch://identity".to_string(),
        "dispatch://db".to_string(),
        "dispatch://learnings".to_string(),
        format!("file://{data_dir}/host.json"),
        format!("file://{data_dir}/tasks.db"),
        "file:///etc/passwd".to_string(),
        format!(
            "file://{}/docs/specs/core.allium",
            env!("CARGO_MANIFEST_DIR")
        ),
        "docs://specs/core.allium".to_string(),
        "repo://src/main.rs".to_string(),
        "skill://wrap-up/references/../../../../docs/plans".to_string(),
        "skill://allium-loop/prompt.md".to_string(),
    ];
    for identity in [CallerIdentity::Task(me), CallerIdentity::Session] {
        for uri in &probes {
            assert_unknown_on_both(&state, identity.clone(), uri).await;
        }
    }

    // The listing offers nothing outside the listable shapes, and no other task.
    let listed = list_all(&state, CallerIdentity::Task(me)).await;
    for entry in &listed {
        let uri = entry["uri"].as_str().unwrap();
        let shaped = uri == TASK_SELF
            || uri.starts_with("dispatch://learnings/")
            || (uri.starts_with("skill://") && !uri.contains(".."));
        assert!(
            shaped,
            "listed URI {uri} is outside the listable resource shapes"
        );
        assert!(
            !entry.to_string().contains("Other task title 5be2"),
            "another task leaked into the listing: {entry}"
        );
        assert!(
            !entry.to_string().contains("Hidden epic 77d"),
            "an epic leaked: {entry}"
        );
    }
}
