//! `dispatch-prompt.allium: CoveredTaskAsksItsOrchestrator`
//! (`orchestrators.allium: TaskAgentUnderAnOrchestrator`).

use super::*;
use crate::store::{
    CreateTaskRequest, EpicCrud, HostStore, OrchestratorStore, Store, TaskCrud, TaskRead,
};

async fn task_in(db: &Store, epic_id: EpicId) -> crate::models::Task {
    let id = db
        .create_task(CreateTaskRequest {
            epic_id: Some(epic_id),
            ..CreateTaskRequest::fixture("t", "/repo")
        })
        .await
        .unwrap();
    db.get_task(id).await.unwrap()
}

async fn orchestrated_epic_with_a_sub_epic(db: &Store) -> (EpicId, EpicId) {
    let root = db.create_epic("Root", "", None).await.unwrap();
    let sub = db.create_epic("Sub", "", Some(root.id)).await.unwrap();
    db.switch_orchestrator_on(root.id, None, None)
        .await
        .unwrap();
    (root.id, sub.id)
}

#[tokio::test]
async fn a_covered_task_names_this_machines_orchestrator_of_the_covering_epic() {
    let db = Store::open_in_memory().unwrap();
    let (root, sub) = orchestrated_epic_with_a_sub_epic(&db).await;
    let host = db.host_id().to_string();
    db.ensure_orchestrator(root, &host).await.unwrap();
    let name = db.list_orchestrators().await[0].name.clone();
    let task = task_in(&db, sub).await;

    let ctx = EpicContext::from_db(&task, &db).await.unwrap();

    assert_eq!(ctx.orchestrator.as_deref(), Some(name.as_str()));
}

#[tokio::test]
async fn a_task_of_an_epic_with_no_orchestrator_names_none() {
    let db = Store::open_in_memory().unwrap();
    let epic = db.create_epic("E", "", None).await.unwrap();
    let task = task_in(&db, epic.id).await;

    let ctx = EpicContext::from_db(&task, &db).await.unwrap();

    assert_eq!(ctx.orchestrator, None);
}

#[tokio::test]
async fn a_switched_on_epic_with_no_row_on_this_machine_names_none() {
    let db = Store::open_in_memory().unwrap();
    let (_, sub) = orchestrated_epic_with_a_sub_epic(&db).await;
    let task = task_in(&db, sub).await;

    let ctx = EpicContext::from_db(&task, &db).await.unwrap();

    assert_eq!(ctx.orchestrator, None);
}

fn ctx(orchestrator: Option<&str>) -> EpicContext {
    EpicContext {
        epic_id: EpicId(7),
        epic_title: "E".to_string(),
        under_cve_feed: false,
        orchestrator: orchestrator.map(str::to_string),
    }
}

#[test]
fn the_covered_section_sends_questions_to_the_orchestrator_in_one_shape() {
    let text = ctx(Some("zlata")).prompt_section();
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    for needle in [
        "zlata",
        // AsksTheOrchestratorFirst
        "plan confirmations included",
        "where this prompt or a skill says to ask the user",
        "later tasks of the epic",
        // QuestionsHaveOneShape
        "what you plan, the question, the options, the one you recommend",
        "where in the code or the spec the evidence is",
        "short outcome report when your task finishes",
        // FallsBackToTheOperator
        "not in the agent list",
        "comes back undelivered",
        "ask the operator",
    ] {
        assert!(flat.contains(needle), "missing {needle:?} in:\n{text}");
    }
    assert!(
        !text.contains("mcp__dispatch__"),
        "the section names no MCP tool, got: {text}"
    );
}

#[test]
fn an_uncovered_section_says_nothing_of_an_orchestrator() {
    let text = ctx(None).prompt_section();
    assert!(!text.to_lowercase().contains("orchestrator"), "got: {text}");
}
