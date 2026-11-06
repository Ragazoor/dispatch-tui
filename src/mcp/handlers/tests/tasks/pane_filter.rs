//! agent-tree.allium: ListsFollowTheBoardFilter. The board narrows the agents a
//! companion pane lists by its saved repo filter and epic hide-list, and says
//! how many it left out.

use super::*;
use crate::hooks::wire::{PaneAgent, PaneView};
use crate::models::TaskId;

async fn pane_view_for(state: &Arc<McpState>, task_id: TaskId) -> PaneView {
    let axum::Json(view) = crate::mcp::handlers::handle_pane_view(
        axum::extract::State(state.clone()),
        axum::Json(crate::hooks::wire::PaneViewRequest { task_id }),
    )
    .await
    .unwrap();
    view
}

fn ids(agents: &[PaneAgent]) -> Vec<TaskId> {
    let mut ids: Vec<_> = agents.iter().map(|a| a.id).collect();
    ids.sort();
    ids
}

async fn save(state: &Arc<McpState>, key: &str, value: &str) {
    state.db.set_setting_string(key, value).await.unwrap();
}

/// A Running task with a worktree and no window.
async fn detached_in(
    state: &Arc<McpState>,
    repo: &str,
    epic: Option<crate::models::EpicId>,
) -> TaskId {
    let id = create_running_task_with_window_in(state, repo, epic).await;
    let patch = crate::store::TaskPatch::new().tmux_window(None);
    state.db_write().patch_task(id, &patch).await.unwrap();
    id
}

#[tokio::test]
async fn with_nothing_saved_the_pane_view_lists_every_agent_and_hides_none() {
    let state = test_state().await;
    let a = create_running_task_with_window_in(&state, "/a", None).await;
    let b = detached_in(&state, "/b", None).await;

    let view = pane_view_for(&state, a).await;

    assert_eq!(ids(&view.live_agents), vec![a]);
    assert_eq!(ids(&view.detached_agents), vec![b]);
    assert_eq!(view.hidden_agents, 0);
}

#[tokio::test]
async fn a_hidden_epic_hides_its_agents_and_its_sub_epics_agents() {
    let state = test_state().await;
    let root = state
        .db_write()
        .create_epic("Root", "", None)
        .await
        .unwrap();
    let sub = state
        .db_write()
        .create_epic("Sub", "", Some(root.id))
        .await
        .unwrap();
    let other = state
        .db_write()
        .create_epic("Other", "", None)
        .await
        .unwrap();
    let in_root = create_running_task_with_window_in(&state, "/r", Some(root.id)).await;
    let in_sub = detached_in(&state, "/r", Some(sub.id)).await;
    let in_other = create_running_task_with_window_in(&state, "/r", Some(other.id)).await;
    let epicless = create_running_task_with_window_in(&state, "/r", None).await;
    save(&state, "hidden_epics", &format!("[{}]", root.id.0)).await;

    let view = pane_view_for(&state, in_other).await;

    assert_eq!(ids(&view.live_agents), vec![in_other, epicless]);
    assert!(view.detached_agents.is_empty());
    assert_eq!(view.hidden_agents, 2);
    let _ = (in_root, in_sub);
}

#[tokio::test]
async fn a_hidden_sub_epic_leaves_its_parents_own_agents_listed() {
    let state = test_state().await;
    let root = state
        .db_write()
        .create_epic("Root", "", None)
        .await
        .unwrap();
    let sub = state
        .db_write()
        .create_epic("Sub", "", Some(root.id))
        .await
        .unwrap();
    let in_root = create_running_task_with_window_in(&state, "/r", Some(root.id)).await;
    let _in_sub = create_running_task_with_window_in(&state, "/r", Some(sub.id)).await;
    save(&state, "hidden_epics", &format!("[{}]", sub.id.0)).await;

    let view = pane_view_for(&state, in_root).await;

    assert_eq!(ids(&view.live_agents), vec![in_root]);
    assert_eq!(view.hidden_agents, 1);
}

#[tokio::test]
async fn an_include_repo_filter_lists_only_the_included_repos_agents() {
    let state = test_state().await;
    let kept = create_running_task_with_window_in(&state, "/kept", None).await;
    let _dropped = create_running_task_with_window_in(&state, "/dropped", None).await;
    let _dropped_detached = detached_in(&state, "/dropped", None).await;
    save(&state, "repo_filter", r#"["/kept"]"#).await;
    save(&state, "repo_filter_mode", "include").await;

    let view = pane_view_for(&state, kept).await;

    assert_eq!(ids(&view.live_agents), vec![kept]);
    assert!(view.detached_agents.is_empty());
    assert_eq!(view.hidden_agents, 2);
}

#[tokio::test]
async fn an_exclude_repo_filter_drops_the_excluded_repos_agents() {
    let state = test_state().await;
    let kept = create_running_task_with_window_in(&state, "/kept", None).await;
    let _dropped = create_running_task_with_window_in(&state, "/dropped", None).await;
    save(&state, "repo_filter", r#"["/dropped"]"#).await;
    save(&state, "repo_filter_mode", "exclude").await;

    let view = pane_view_for(&state, kept).await;

    assert_eq!(ids(&view.live_agents), vec![kept]);
    assert_eq!(view.hidden_agents, 1);
}

/// The pane's own task gets no exception from the filter, but its PaneTask
/// still comes back so the tree and commits sections keep working.
#[tokio::test]
async fn the_asking_panes_own_hidden_agent_is_left_out_but_its_task_is_answered() {
    let state = test_state().await;
    let epic = state
        .db_write()
        .create_epic("Hidden", "", None)
        .await
        .unwrap();
    let own = create_running_task_with_window_in(&state, "/r", Some(epic.id)).await;
    save(&state, "hidden_epics", &format!("[{}]", epic.id.0)).await;

    let view = pane_view_for(&state, own).await;

    assert!(view.live_agents.is_empty());
    assert_eq!(view.hidden_agents, 1);
    assert!(view.task.is_some());
}
