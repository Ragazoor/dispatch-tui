//! The workspace and super-admin tools (`docs/specs/admin-surfaces.allium`,
//! "the MCP twins"), over an in-memory store. The test user is a member of
//! the default workspace 1 and is not a super-admin until it claims.

use super::*;
use crate::store::PeopleStore as _;

const BOB: &str = "bbbbbbbb00000000000000000000000000000000000000000000000000000001";
const ANN1: &str = "aaaaaaaa00000000000000000000000000000000000000000000000000000001";
const ANN2: &str = "aaaaaaab00000000000000000000000000000000000000000000000000000002";

async fn state_with_people() -> (Arc<McpState>, Arc<Store>) {
    let store = Arc::new(Store::open_in_memory().unwrap());
    store.seed_person(BOB, "Bob");
    store.seed_person(ANN1, "Ann");
    store.seed_person(ANN2, "Ann");
    let db: Arc<dyn store::TaskStore> = store.clone();
    let (state, _) = state_over(
        db,
        Arc::new(MockProcessRunner::new(vec![])),
        None,
        None,
        None,
    );
    (state, store)
}

async fn tool(state: &Arc<McpState>, name: &str, arguments: Value) -> JsonRpcResponse {
    call(
        state,
        "tools/call",
        Some(json!({ "name": name, "arguments": arguments })),
    )
    .await
}

async fn ok_text(state: &Arc<McpState>, name: &str, arguments: Value) -> String {
    let resp = tool(state, name, arguments).await;
    assert!(
        !resp
            .result
            .as_ref()
            .and_then(|r| r.get("isError"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        "{name} was refused: {}",
        extract_response_text(&resp)
    );
    extract_response_text(&resp)
}

fn tool_hints(name: &str) -> Value {
    let defs = tool_definitions();
    defs["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == name)
        .unwrap_or_else(|| panic!("no tool {name}"))["annotations"]
        .clone()
}

#[test]
fn the_tools_declare_their_access_class() {
    for (name, read_only, destructive, idempotent) in [
        ("list_workspaces", true, false, true),
        ("create_workspace", false, false, false),
        ("update_workspace", false, true, false),
        ("set_workspace_member", false, true, false),
        ("set_super_admin", false, false, true),
        ("claim_super_admin", false, false, false),
        ("list_unseen_items", true, false, true),
        ("delete_unseen_item", false, true, false),
    ] {
        let hints = tool_hints(name);
        assert_eq!(hints["readOnlyHint"], json!(read_only), "{name}");
        assert_eq!(hints["destructiveHint"], json!(destructive), "{name}");
        assert_eq!(hints["idempotentHint"], json!(idempotent), "{name}");
    }
}

#[tokio::test]
async fn list_workspaces_shows_members_roles_and_the_directory() {
    let (state, _) = state_with_people().await;
    let text = ok_text(&state, "list_workspaces", json!({})).await;
    assert!(text.contains("Workspace 1: Default"), "{text}");
    assert!(text.contains("Bob (bbbbbbbb)"), "{text}");
    assert!(
        text.contains("claim_super_admin"),
        "no super-admin yet: {text}"
    );
}

#[tokio::test]
async fn a_non_super_admin_is_refused_by_the_store_in_its_words() {
    let (state, _) = state_with_people().await;
    let resp = tool(&state, "create_workspace", json!({"name": "Ops"})).await;
    assert_error(&resp, "only a super-admin");
}

#[tokio::test]
async fn claim_then_create_then_add_a_member_by_display_name() {
    let (state, _) = state_with_people().await;
    ok_text(&state, "claim_super_admin", json!({})).await;
    let made = ok_text(&state, "create_workspace", json!({"name": "Ops"})).await;
    assert!(made.contains("Created workspace 2: Ops"), "{made}");

    let added = ok_text(
        &state,
        "set_workspace_member",
        json!({"workspace_id": 2, "person": "Bob", "role": "member"}),
    )
    .await;
    assert!(added.contains("Added Bob to Ops as member"), "{added}");

    let promoted = ok_text(
        &state,
        "set_workspace_member",
        json!({"workspace_id": 2, "person": "Bob", "role": "admin"}),
    )
    .await;
    assert!(promoted.contains("Bob is admin of Ops"), "{promoted}");

    let listed = ok_text(&state, "list_workspaces", json!({"workspace_id": 2})).await;
    assert!(listed.contains("Bob (bbbbbbbb) admin"), "{listed}");
    assert!(!listed.contains("People (the directory"), "{listed}");

    let removed = ok_text(
        &state,
        "set_workspace_member",
        json!({"workspace_id": 2, "person": "Bob", "remove": true}),
    )
    .await;
    assert!(removed.contains("Removed Bob from Ops"), "{removed}");
}

#[tokio::test]
async fn a_second_claim_is_refused() {
    let (state, _) = state_with_people().await;
    ok_text(&state, "claim_super_admin", json!({})).await;
    let resp = tool(&state, "claim_super_admin", json!({})).await;
    assert_error(&resp, "already exists");
}

#[tokio::test]
async fn a_duplicated_name_is_refused_with_the_matches_and_an_identity_picks_one() {
    let (state, _) = state_with_people().await;
    ok_text(&state, "claim_super_admin", json!({})).await;
    let resp = tool(
        &state,
        "set_workspace_member",
        json!({"workspace_id": 1, "person": "Ann", "role": "member"}),
    )
    .await;
    assert_error(&resp, "Ann (aaaaaaaa)");
    assert_error(&resp, "Ann (aaaaaaab)");

    let added = ok_text(
        &state,
        "set_workspace_member",
        json!({"workspace_id": 1, "person": "aaaaaaab", "role": "member"}),
    )
    .await;
    assert!(added.contains("Added Ann to Default"), "{added}");
}

#[tokio::test]
async fn a_call_must_choose_between_role_and_remove() {
    let (state, _) = state_with_people().await;
    ok_text(&state, "claim_super_admin", json!({})).await;
    for args in [
        json!({"workspace_id": 1, "person": "Bob"}),
        json!({"workspace_id": 1, "person": "Bob", "role": "member", "remove": true}),
    ] {
        assert_error(
            &tool(&state, "set_workspace_member", args).await,
            "exactly one",
        );
    }
    assert_error(
        &tool(&state, "update_workspace", json!({"workspace_id": 1})).await,
        "exactly one",
    );
    assert_error(
        &tool(
            &state,
            "update_workspace",
            json!({"workspace_id": 1, "name": "X", "delete": true}),
        )
        .await,
        "exactly one",
    );
}

#[tokio::test]
async fn removing_someone_who_is_not_a_member_is_refused() {
    let (state, _) = state_with_people().await;
    ok_text(&state, "claim_super_admin", json!({})).await;
    ok_text(&state, "create_workspace", json!({"name": "Ops"})).await;
    let resp = tool(
        &state,
        "set_workspace_member",
        json!({"workspace_id": 2, "person": "Bob", "remove": true}),
    )
    .await;
    assert_error(&resp, "not a member");
}

#[tokio::test]
async fn rename_and_delete_a_workspace() {
    let (state, _) = state_with_people().await;
    ok_text(&state, "claim_super_admin", json!({})).await;
    ok_text(&state, "create_workspace", json!({"name": "Ops"})).await;
    let renamed = ok_text(
        &state,
        "update_workspace",
        json!({"workspace_id": 2, "name": "Operations"}),
    )
    .await;
    assert!(
        renamed.contains("Renamed workspace 2 to Operations"),
        "{renamed}"
    );
    let deleted = ok_text(
        &state,
        "update_workspace",
        json!({"workspace_id": 2, "delete": true}),
    )
    .await;
    assert!(
        deleted.contains("Deleted workspace Operations"),
        "{deleted}"
    );
    let resp = tool(
        &state,
        "update_workspace",
        json!({"workspace_id": 2, "delete": true}),
    )
    .await;
    assert_error(&resp, "no workspace 2");
}

#[tokio::test]
async fn grant_and_revoke_are_one_idempotent_tool_and_the_last_cannot_go() {
    let (state, store) = state_with_people().await;
    ok_text(&state, "claim_super_admin", json!({})).await;
    let granted = ok_text(
        &state,
        "set_super_admin",
        json!({"person": "Bob", "super_admin": true}),
    )
    .await;
    assert!(granted.contains("Bob is a super-admin"), "{granted}");
    ok_text(
        &state,
        "set_super_admin",
        json!({"person": "Bob", "super_admin": true}),
    )
    .await;
    ok_text(
        &state,
        "set_super_admin",
        json!({"person": "Bob", "super_admin": false}),
    )
    .await;
    ok_text(
        &state,
        "set_super_admin",
        json!({"person": "Bob", "super_admin": false}),
    )
    .await;
    // The test user is the only super-admin left, and the last cannot go.
    let me = store.admin_snapshot().await.me.unwrap();
    let resp = tool(
        &state,
        "set_super_admin",
        json!({"person": me, "super_admin": false}),
    )
    .await;
    assert_error(&resp, "last super-admin");
}

#[tokio::test]
async fn an_unseen_private_epic_is_listed_without_a_title_and_deleted() {
    let (state, store) = state_with_people().await;
    ok_text(&state, "claim_super_admin", json!({})).await;
    ok_text(
        &state,
        "create_epic",
        json!({"title": "Secret plans", "workspace_id": 1, "visibility": "private"}),
    )
    .await;
    // Nothing is unseen while its creator is still a member.
    let none = ok_text(&state, "list_unseen_items", json!({})).await;
    assert!(none.contains("No unseen items"), "{none}");

    // The super-admin takes the creator (themselves) out of the workspace.
    let listed = ok_text(&state, "list_workspaces", json!({"workspace_id": 1})).await;
    assert!(listed.contains("membership"), "{listed}");
    let me = store.admin_snapshot().await.me.unwrap();
    ok_text(
        &state,
        "set_workspace_member",
        json!({"workspace_id": 1, "person": me, "remove": true}),
    )
    .await;

    let items = ok_text(&state, "list_unseen_items", json!({})).await;
    assert!(items.contains("epic"), "{items}");
    assert!(items.contains("Default"), "{items}");
    assert!(
        !items.contains("Secret plans"),
        "no title is shown: {items}"
    );

    let id: i64 = items
        .lines()
        .find_map(|l| l.strip_prefix("- epic "))
        .and_then(|l| l.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .expect("an epic row");
    let deleted = ok_text(
        &state,
        "delete_unseen_item",
        json!({"kind": "epic", "item_id": id}),
    )
    .await;
    assert!(deleted.contains(&format!("Deleted epic {id}")), "{deleted}");
    let after = ok_text(&state, "list_unseen_items", json!({})).await;
    assert!(after.contains("No unseen items"), "{after}");
}

#[tokio::test]
async fn a_person_who_is_not_a_super_admin_sees_no_unseen_items() {
    let (state, _) = state_with_people().await;
    let text = ok_text(&state, "list_unseen_items", json!({})).await;
    assert!(text.contains("Only a super-admin"), "{text}");
}

#[tokio::test]
async fn a_public_item_cannot_be_deleted_through_the_hatch() {
    let (state, _) = state_with_people().await;
    ok_text(&state, "claim_super_admin", json!({})).await;
    ok_text(
        &state,
        "create_epic",
        json!({"title": "Open", "workspace_id": 1}),
    )
    .await;
    let resp = tool(
        &state,
        "delete_unseen_item",
        json!({"kind": "epic", "item_id": 1}),
    )
    .await;
    assert_error(&resp, "only a private item");
}
