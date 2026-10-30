//! The workspace and visibility step that ends the new-epic form
//! (`workspaces.allium: NameWorkspaceAndVisibilityFromBoard`).

use super::*;
use crate::models::{EpicId, Visibility, WorkspaceRef};
use crate::tui::commands::EpicCommand;
use crate::tui::messages::InputMessage;
use crossterm::event::KeyCode;

fn workspaces(names: &[&str]) -> Vec<WorkspaceRef> {
    names
        .iter()
        .enumerate()
        .map(|(i, name)| WorkspaceRef {
            id: i as i64 + 1,
            name: (*name).to_string(),
        })
        .collect()
}

/// An app that has finished the epic description, with `names` as the
/// person's workspaces.
fn at_the_placement_step(names: &[&str], parent: Option<EpicId>) -> App {
    let mut app = make_app();
    app.update(Message::Input(InputMessage::WorkspacesLoaded(workspaces(
        names,
    ))));
    app.input.mode = InputMode::InputEpicDescription;
    app.input.epic_draft = Some(EpicDraft {
        title: "My Epic".to_string(),
        parent_epic_id: parent,
        ..Default::default()
    });
    let cmds = app.update(Message::Epic(
        crate::tui::messages::EpicMessage::SubmitDescription("d".to_string()),
    ));
    assert!(
        cmds.is_empty(),
        "nothing is created before the step: {cmds:?}"
    );
    app
}

fn inserted(cmds: Vec<Command>) -> EpicDraft {
    match without_usage(cmds).as_slice() {
        [Command::Epic(EpicCommand::Insert(draft))] => draft.clone(),
        other => panic!("expected one Insert, got {other:?}"),
    }
}

#[test]
fn the_epic_form_ends_with_the_placement_step() {
    let app = at_the_placement_step(&["Core"], None);
    assert_eq!(app.input.mode, InputMode::InputPlacement);
}

#[test]
fn one_workspace_is_used_without_asking_and_the_epic_is_public() {
    let mut app = at_the_placement_step(&["Core"], None);
    let cmds = app.handle_key(make_key(KeyCode::Enter));
    let draft = inserted(cmds);
    assert_eq!(draft.workspace_id, 1);
    assert_eq!(draft.visibility, Visibility::Public);
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn space_h_and_l_toggle_the_visibility() {
    let mut app = at_the_placement_step(&["Core"], None);
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert_eq!(
        app.input.placement.as_ref().unwrap().visibility,
        Visibility::Private
    );
    app.handle_key(make_key(KeyCode::Char('h')));
    assert_eq!(
        app.input.placement.as_ref().unwrap().visibility,
        Visibility::Public
    );
    app.handle_key(make_key(KeyCode::Char('l')));
    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(inserted(cmds).visibility, Visibility::Private);
}

#[test]
fn several_workspaces_start_on_the_first_and_j_k_move_between_them() {
    let mut app = at_the_placement_step(&["Core", "Data", "Ops"], None);
    assert_eq!(app.input.placement.as_ref().unwrap().workspace, 0);
    app.handle_key(make_key(KeyCode::Char('j')));
    app.handle_key(make_key(KeyCode::Char('j')));
    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(
        app.input.placement.as_ref().unwrap().workspace,
        2,
        "stops at the last"
    );
    app.handle_key(make_key(KeyCode::Char('k')));
    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(inserted(cmds).workspace_id, 2);
}

#[test]
fn a_sub_epic_names_no_workspace() {
    let mut app = at_the_placement_step(&["Core", "Data"], Some(EpicId(5)));
    app.handle_key(make_key(KeyCode::Char('j')));
    let cmds = app.handle_key(make_key(KeyCode::Enter));
    let draft = inserted(cmds);
    assert_eq!(draft.workspace_id, 0, "the parent's is used");
    assert_eq!(draft.parent_epic_id, Some(EpicId(5)));
}

#[test]
fn escape_at_the_step_creates_nothing() {
    let mut app = at_the_placement_step(&["Core"], None);
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert!(without_usage(cmds).is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.input.epic_draft.is_none());
    assert!(app.input.placement.is_none());
}

#[test]
fn a_person_in_no_workspace_is_told_so_and_the_form_closes() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicTitle;
    app.input.epic_draft = Some(EpicDraft::default());
    app.update(Message::Input(InputMessage::WorkspacesLoaded(vec![])));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.input.epic_draft.is_none());
    let shown = app.status.error_popup.clone().unwrap_or_default();
    assert!(shown.contains("You belong to no workspace"), "{shown}");
}

#[test]
fn a_late_empty_load_does_not_close_an_unrelated_mode() {
    let mut app = make_app();
    app.update(Message::Input(InputMessage::WorkspacesLoaded(vec![])));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.status.error_popup.is_none());
}

#[test]
fn opening_a_root_epic_form_asks_for_the_workspaces() {
    let mut app = make_app();
    let cmds = app.update(Message::Epic(crate::tui::messages::EpicMessage::StartNew));
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::People(crate::tui::commands::PeopleCommand::LoadWorkspaces)
    )));
}
