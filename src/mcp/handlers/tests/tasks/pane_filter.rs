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

// -- agent-tree.allium: OrchestratorsAreThisMachinesLiveOnes ------------------

/// An orchestrator of a fresh epic on this machine, its session started when
/// `live`.
async fn orchestrator_of(
    state: &Arc<McpState>,
    title: &str,
    parent: Option<crate::models::EpicId>,
    live: bool,
) -> crate::models::Epic {
    let epic = state
        .db_write()
        .create_epic(title, "", parent)
        .await
        .unwrap();
    state
        .db_write()
        .switch_orchestrator_on(epic.id, None, None)
        .await
        .unwrap();
    let host = state.db.host_id().to_string();
    state.db.ensure_orchestrator(epic.id, &host).await.unwrap();
    if live {
        let row = state
            .db
            .list_orchestrators()
            .await
            .into_iter()
            .find(|o| o.epic_id == epic.id)
            .unwrap();
        state
            .db
            .start_orchestrator_session(row.id, &host, false)
            .await
            .unwrap();
    }
    epic
}

#[tokio::test]
async fn a_live_orchestrator_is_listed_with_its_epic_and_window() {
    let state = test_state().await;
    let root = state
        .db_write()
        .create_epic("Root", "", None)
        .await
        .unwrap();
    let epic = orchestrator_of(&state, "Orchestrated", Some(root.id), true).await;
    let own = create_running_task_with_window_in(&state, "/r", None).await;

    let view = pane_view_for(&state, own).await;

    assert_eq!(view.orchestrators.len(), 1, "{view:?}");
    let listed = &view.orchestrators[0];
    assert_eq!(listed.epic.id, epic.id);
    assert_eq!(listed.epic.breadcrumb, vec!["Orchestrated".to_string()]);
    assert_eq!(listed.epic.ancestors.len(), 1);
    assert_eq!(
        listed.tmux_window,
        crate::models::TmuxWindow::for_orchestrator(epic.id).as_str()
    );
    assert!(!listed.name.is_empty());
}

#[tokio::test]
async fn an_orchestrator_with_no_window_is_not_listed() {
    let state = test_state().await;
    orchestrator_of(&state, "Asleep", None, false).await;
    let own = create_running_task_with_window_in(&state, "/r", None).await;

    let view = pane_view_for(&state, own).await;

    assert!(view.orchestrators.is_empty(), "{view:?}");
}

#[tokio::test]
async fn an_orchestrator_of_a_hidden_epic_is_left_out_and_not_counted() {
    let state = test_state().await;
    let root = state
        .db_write()
        .create_epic("Root", "", None)
        .await
        .unwrap();
    orchestrator_of(&state, "Under root", Some(root.id), true).await;
    let own = create_running_task_with_window_in(&state, "/r", None).await;
    save(&state, "hidden_epics", &format!("[{}]", root.id.0)).await;

    let view = pane_view_for(&state, own).await;

    assert!(view.orchestrators.is_empty(), "{view:?}");
    assert_eq!(view.hidden_agents, 0);
}

#[tokio::test]
async fn the_repo_filter_leaves_an_orchestrator_listed() {
    let state = test_state().await;
    orchestrator_of(&state, "Anywhere", None, true).await;
    let own = create_running_task_with_window_in(&state, "/kept", None).await;
    save(&state, "repo_filter", r#"["/kept"]"#).await;
    save(&state, "repo_filter_mode", "include").await;

    let view = pane_view_for(&state, own).await;

    assert_eq!(view.orchestrators.len(), 1, "{view:?}");
}

#[test]
fn another_machines_orchestrator_is_not_listed() {
    let orchestrator = |host: &str, epic: i64| crate::models::Orchestrator {
        id: epic,
        epic_id: crate::models::EpicId(epic),
        host: host.to_string(),
        name: format!("orch {epic}"),
        tmux_window: Some(crate::models::TmuxWindow::for_orchestrator(
            crate::models::EpicId(epic),
        )),
        started_at: None,
        restarted_once: false,
        session_died: false,
        last_turn_at: None,
    };
    let listed = crate::mcp::handlers::hooks::listed_orchestrators(
        vec![orchestrator("here", 1), orchestrator("there", 2)],
        "here",
        &std::collections::HashSet::new(),
    );
    let epics: Vec<_> = listed.iter().map(|(_, o)| o.epic_id.0).collect();
    assert_eq!(epics, vec![1]);
}
