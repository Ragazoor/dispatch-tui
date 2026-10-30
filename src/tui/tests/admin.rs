//! The admin screen (`docs/specs/admin-surfaces.allium: AdminScreen`).

use super::*;
use crate::models::{
    AdminSnapshot, DirectoryPerson, MemberRole, UnseenItemRow, WorkspaceDetail, WorkspaceMember,
};
use crate::tui::commands::{AdminWrite, PeopleCommand};
use crate::tui::messages::AdminMessage;
use crate::tui::types::{AdminConfirm, AdminPane, AdminPrompt, AdminScreen};
use crossterm::event::KeyCode;

fn member(id: i64, name: &str, role: MemberRole) -> WorkspaceMember {
    WorkspaceMember {
        membership_id: id,
        identity: format!("{name:a<16}"),
        shown_name: name.to_string(),
        role,
    }
}

fn workspace(id: i64, name: &str, members: Vec<WorkspaceMember>) -> WorkspaceDetail {
    WorkspaceDetail {
        id,
        name: name.to_string(),
        members,
    }
}

fn snapshot(super_admin: bool) -> AdminSnapshot {
    AdminSnapshot {
        me: Some("me".to_string()),
        i_am_super_admin: super_admin,
        claimable: false,
        workspaces: vec![
            workspace(
                1,
                "Default",
                vec![
                    member(7, "Ann", MemberRole::Admin),
                    member(8, "Bob", MemberRole::Member),
                ],
            ),
            workspace(2, "Ops", vec![member(9, "Cy", MemberRole::Member)]),
        ],
        directory: vec![
            DirectoryPerson {
                identity: "me".to_string(),
                shown_name: "Me".to_string(),
                is_super_admin: super_admin,
            },
            DirectoryPerson {
                identity: format!("{:a<16}", "Ann"),
                shown_name: "Ann".to_string(),
                is_super_admin: false,
            },
        ],
        unseen: vec![UnseenItemRow {
            kind: "epic".to_string(),
            id: 12,
            workspace_id: 1,
            workspace_name: "Default".to_string(),
            creator: "Zed".to_string(),
            created_at: "2026-10-30T07:00:00Z".to_string(),
        }],
    }
}

/// A screen on a super-admin's snapshot, for the keybinding gate too.
pub(super) fn super_admin_screen() -> AdminScreen {
    AdminScreen {
        snapshot: snapshot(true),
        ..AdminScreen::default()
    }
}

fn open(snapshot: AdminSnapshot) -> App {
    let mut app = make_app();
    app.input.admin = Some(AdminScreen::default());
    app.input.mode = InputMode::Admin;
    app.update(Message::Admin(AdminMessage::Loaded(Box::new(snapshot))));
    app
}

fn press(app: &mut App, c: char) -> Vec<Command> {
    without_usage(app.handle_key(make_key(KeyCode::Char(c))))
}

fn writes(cmds: Vec<Command>) -> Vec<AdminWrite> {
    cmds.into_iter()
        .filter_map(|c| match c {
            Command::People(PeopleCommand::Admin(w)) => Some(w),
            _ => None,
        })
        .collect()
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle_key(make_key(KeyCode::Char(c)));
    }
}

fn pane(app: &App) -> AdminPane {
    app.input.admin.as_ref().unwrap().pane
}

#[test]
fn a_opens_the_screen_and_asks_for_the_snapshot() {
    let mut app = make_app();
    let cmds = press(&mut app, 'A');
    assert_eq!(app.input.mode, InputMode::Admin);
    assert!(
        matches!(cmds.as_slice(), [Command::People(PeopleCommand::LoadAdmin)]),
        "{cmds:?}"
    );
}

#[test]
fn esc_closes_the_screen_and_drops_its_state() {
    let mut app = open(snapshot(true));
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.input.admin.is_none());
}

#[test]
fn a_snapshot_that_arrives_after_the_screen_closed_is_ignored() {
    let mut app = make_app();
    let cmds = app.update(Message::Admin(AdminMessage::Loaded(Box::new(snapshot(
        true,
    )))));
    assert!(cmds.is_empty());
    assert!(app.input.admin.is_none());
}

#[test]
fn a_super_admin_visits_four_panes_and_others_two() {
    let mut app = open(snapshot(true));
    let mut seen = vec![pane(&app)];
    for _ in 0..4 {
        app.handle_key(make_key(KeyCode::Tab));
        seen.push(pane(&app));
    }
    assert_eq!(
        seen,
        [
            AdminPane::Workspaces,
            AdminPane::Members,
            AdminPane::SuperAdmins,
            AdminPane::Unseen,
            AdminPane::Workspaces
        ]
    );

    // A workspace admin sees only the workspaces they administer.
    let mut snap = snapshot(false);
    snap.workspaces[0].members[0].identity = "me".to_string();
    let mut app = open(snap);
    let mut seen = vec![pane(&app)];
    for _ in 0..2 {
        app.handle_key(make_key(KeyCode::Tab));
        seen.push(pane(&app));
    }
    assert_eq!(
        seen,
        [
            AdminPane::Workspaces,
            AdminPane::Members,
            AdminPane::Workspaces
        ]
    );
    assert_eq!(app.input.admin.as_ref().unwrap().administered().len(), 1);
}

#[test]
fn a_person_who_administers_nothing_cannot_start_a_write() {
    let mut app = open(snapshot(false));
    for key in ['n', 'r', 'd', 'a'] {
        let cmds = press(&mut app, key);
        assert!(cmds.is_empty(), "{key}: {cmds:?}");
        assert_eq!(app.input.mode, InputMode::Admin, "{key}");
    }
}

#[test]
fn new_workspace_types_a_name_and_sends_it() {
    let mut app = open(snapshot(true));
    press(&mut app, 'n');
    assert_eq!(app.input.mode, InputMode::InputAdminText);
    assert_eq!(
        app.input.admin.as_ref().unwrap().prompt,
        Some(AdminPrompt::NewWorkspace)
    );
    type_text(&mut app, "Support");
    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(
        writes(cmds),
        [AdminWrite::CreateWorkspace("Support".to_string())]
    );
    assert_eq!(app.input.mode, InputMode::Admin);
}

#[test]
fn a_blank_name_and_esc_send_nothing() {
    let mut app = open(snapshot(true));
    press(&mut app, 'n');
    type_text(&mut app, "   ");
    assert!(writes(app.handle_key(make_key(KeyCode::Enter))).is_empty());
    assert_eq!(app.input.mode, InputMode::Admin);

    press(&mut app, 'n');
    type_text(&mut app, "x");
    assert!(writes(app.handle_key(make_key(KeyCode::Esc))).is_empty());
    assert_eq!(app.input.mode, InputMode::Admin);
    assert!(app.input.admin.is_some(), "Esc returns to the screen");
}

#[test]
fn only_a_super_admin_creates_or_deletes_a_workspace() {
    let mut snap = snapshot(false);
    snap.workspaces[0].members[0].identity = "me".to_string();
    let mut app = open(snap);
    assert!(press(&mut app, 'n').is_empty());
    assert!(press(&mut app, 'd').is_empty());
    assert_eq!(app.input.mode, InputMode::Admin);
    // But a workspace admin renames theirs.
    press(&mut app, 'r');
    assert_eq!(app.input.mode, InputMode::InputAdminText);
}

#[test]
fn rename_opens_with_the_current_name() {
    let mut app = open(snapshot(true));
    press(&mut app, 'r');
    assert_eq!(app.input.buffer, "Default");
    app.handle_key(make_key(KeyCode::Backspace));
    type_text(&mut app, "x");
    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(
        writes(cmds),
        [AdminWrite::RenameWorkspace {
            id: 1,
            name: "Defaulx".to_string()
        }]
    );
}

#[test]
fn delete_workspace_asks_first_and_only_y_sends() {
    let mut app = open(snapshot(true));
    press(&mut app, 'j');
    press(&mut app, 'd');
    assert_eq!(app.input.mode, InputMode::ConfirmAdmin);
    assert!(matches!(
        app.input.admin.as_ref().unwrap().confirm,
        Some(AdminConfirm::DeleteWorkspace {
            id: 2,
            members: 1,
            ..
        })
    ));
    assert!(writes(press(&mut app, 'n')).is_empty());
    assert_eq!(app.input.mode, InputMode::Admin);

    press(&mut app, 'd');
    assert_eq!(
        writes(press(&mut app, 'y')),
        [AdminWrite::DeleteWorkspace(2)]
    );
    assert_eq!(app.input.mode, InputMode::Admin);
}

#[test]
fn add_member_sends_what_was_typed_for_the_highlighted_workspace() {
    let mut app = open(snapshot(true));
    press(&mut app, 'j');
    press(&mut app, 'a');
    type_text(&mut app, "Bob");
    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(
        writes(cmds),
        [AdminWrite::AddMember {
            workspace_id: 2,
            person: "Bob".to_string()
        }]
    );
}

#[test]
fn m_flips_the_highlighted_members_role_in_one_key() {
    let mut app = open(snapshot(true));
    app.handle_key(make_key(KeyCode::Tab));
    // Ann is the admin, first in the list.
    assert_eq!(
        writes(press(&mut app, 'm')),
        [AdminWrite::SetRole {
            membership_id: 7,
            role: MemberRole::Member
        }]
    );
    press(&mut app, 'j');
    assert_eq!(
        writes(press(&mut app, 'm')),
        [AdminWrite::SetRole {
            membership_id: 8,
            role: MemberRole::Admin
        }]
    );
}

#[test]
fn remove_member_names_the_person_and_workspace_then_sends_on_y() {
    let mut app = open(snapshot(true));
    app.handle_key(make_key(KeyCode::Tab));
    press(&mut app, 'j');
    press(&mut app, 'x');
    assert_eq!(
        app.input.admin.as_ref().unwrap().confirm,
        Some(AdminConfirm::RemoveMember {
            membership_id: 8,
            person: "Bob".to_string(),
            workspace: "Default".to_string()
        })
    );
    assert_eq!(writes(press(&mut app, 'y')), [AdminWrite::RemoveMember(8)]);
}

#[test]
fn grant_and_revoke_super_admin_work_from_their_pane() {
    let mut app = open(snapshot(true));
    app.handle_key(make_key(KeyCode::Tab));
    app.handle_key(make_key(KeyCode::Tab));
    assert_eq!(pane(&app), AdminPane::SuperAdmins);
    press(&mut app, 'g');
    type_text(&mut app, "Ann");
    assert_eq!(
        writes(app.handle_key(make_key(KeyCode::Enter))),
        [AdminWrite::SetSuperAdmin {
            person: "Ann".to_string(),
            granted: true
        }]
    );
    press(&mut app, 'v');
    assert_eq!(
        writes(press(&mut app, 'y')),
        [AdminWrite::SetSuperAdmin {
            person: "me".to_string(),
            granted: false
        }]
    );
}

#[test]
fn claim_works_only_while_no_super_admin_exists() {
    let mut snap = snapshot(false);
    snap.claimable = true;
    let mut app = open(snap);
    assert_eq!(writes(press(&mut app, 'c')), [AdminWrite::Claim]);

    let mut app = open(snapshot(true));
    assert!(press(&mut app, 'c').is_empty());
}

#[test]
fn deleting_an_unseen_item_asks_first() {
    let mut app = open(snapshot(true));
    for _ in 0..3 {
        app.handle_key(make_key(KeyCode::Tab));
    }
    assert_eq!(pane(&app), AdminPane::Unseen);
    press(&mut app, 'D');
    assert_eq!(
        app.input.admin.as_ref().unwrap().confirm,
        Some(AdminConfirm::DeleteUnseen {
            kind: "epic".to_string(),
            id: 12
        })
    );
    assert_eq!(
        writes(press(&mut app, 'y')),
        [AdminWrite::DeleteUnseen {
            kind: "epic".to_string(),
            id: 12
        }]
    );
}

#[test]
fn a_reload_keeps_the_highlighted_workspace_by_id() {
    let mut app = open(snapshot(true));
    press(&mut app, 'j'); // Ops
    let mut snap = snapshot(true);
    snap.workspaces.insert(0, workspace(0, "Alpha", vec![]));
    app.update(Message::Admin(AdminMessage::Loaded(Box::new(snap))));
    let screen = app.input.admin.as_ref().unwrap();
    assert_eq!(screen.current_workspace().map(|w| w.id), Some(2));
}

#[test]
fn a_reload_clamps_the_highlight_when_its_row_is_gone() {
    let mut app = open(snapshot(true));
    press(&mut app, 'j');
    let mut snap = snapshot(true);
    snap.workspaces.truncate(1);
    app.update(Message::Admin(AdminMessage::Loaded(Box::new(snap))));
    let screen = app.input.admin.as_ref().unwrap();
    assert_eq!(screen.current_workspace().map(|w| w.id), Some(1));
}
