//! The admin screen (`docs/specs/admin-surfaces.allium: AdminScreen`): the
//! workspaces the person administers, the members of the highlighted one and,
//! for a super-admin, the super-admins and the private items nobody can see.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{BorderType, Paragraph},
    Frame,
};

use crate::keybindings::{hint_key, hint_keys, yes_no_hint, KeyNamespace as N};
use crate::models::MemberRole;
use crate::palette::{CYAN, FG, GREEN, MUTED, RED, YELLOW};
use crate::tui::types::{AdminConfirm, AdminPane, AdminPrompt, AdminScreen};
use crate::tui::ui::shared::{caret_line, centered_rect, open_overlay, titled_block, truncate};
use crate::tui::{App, InputMode};

/// Draw the screen while it, or a prompt or confirmation over it, is open.
pub(in crate::tui::ui::kanban) fn render_admin_overlay(frame: &mut Frame, app: &App, area: Rect) {
    if !matches!(
        app.input.mode,
        InputMode::Admin | InputMode::InputAdminText | InputMode::ConfirmAdmin
    ) {
        return;
    }
    let Some(screen) = app.input.admin.as_ref() else {
        return;
    };
    let width = (area.width * 80 / 100).clamp(40, 100);
    let height = (area.height * 80 / 100).clamp(14, 40);
    let block = titled_block(
        CYAN,
        BorderType::Rounded,
        " Workspaces and people ".to_string(),
    );
    let inner = open_overlay(frame, centered_rect(area, width, height), block);
    let text_width = usize::from(inner.width);

    let mut lines = admin_lines(screen, text_width);
    lines.push(Line::raw(""));
    lines.extend(footer_lines(app, screen, text_width));
    frame.render_widget(Paragraph::new(lines), inner);
}

fn heading(text: &str, active: bool) -> Line<'static> {
    let style = if active {
        Style::default().fg(CYAN).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(MUTED)
    };
    Line::from(Span::styled(text.to_string(), style))
}

fn row(text: String, selected: bool, pane_active: bool) -> Line<'static> {
    let marker = if selected && pane_active {
        "\u{25b8} "
    } else {
        "  "
    };
    let style = if selected && pane_active {
        Style::default().fg(FG).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(FG)
    };
    Line::from(Span::styled(format!("{marker}{text}"), style))
}

fn muted(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("  {text}"),
        Style::default().fg(MUTED),
    ))
}

/// The body: one section per pane. Public to the module's tests.
pub(super) fn admin_lines(screen: &AdminScreen, width: usize) -> Vec<Line<'static>> {
    let snapshot = &screen.snapshot;
    let mut lines = Vec::new();

    let administered = screen.administered();
    lines.push(heading("Workspaces", screen.pane == AdminPane::Workspaces));
    if administered.is_empty() {
        lines.push(muted("You administer no workspace."));
        if snapshot.claimable {
            lines.push(muted("No super-admin yet: press c to claim it."));
        }
    }
    for (i, w) in administered.iter().enumerate() {
        let count = w.members.len();
        let text = format!(
            "{}  ({count} member{})",
            truncate(&w.name, width.saturating_sub(20)),
            if count == 1 { "" } else { "s" }
        );
        lines.push(row(
            text,
            i == screen.workspace,
            screen.pane == AdminPane::Workspaces,
        ));
    }

    if let Some(workspace) = screen.current_workspace() {
        lines.push(Line::raw(""));
        lines.push(heading(
            &format!("Members of {}", workspace.name),
            screen.pane == AdminPane::Members,
        ));
        if workspace.members.is_empty() {
            lines.push(muted("No members. Press a to add one."));
        }
        for (i, m) in workspace.members.iter().enumerate() {
            let prefix: String = m.identity.chars().take(8).collect();
            let role = match m.role {
                MemberRole::Admin => "admin",
                MemberRole::Member => "member",
            };
            lines.push(row(
                format!("{} ({prefix})  {role}", m.shown_name),
                i == screen.member,
                screen.pane == AdminPane::Members,
            ));
        }
    }

    if snapshot.i_am_super_admin {
        lines.push(Line::raw(""));
        lines.push(heading(
            "Super-admins",
            screen.pane == AdminPane::SuperAdmins,
        ));
        for (i, p) in snapshot.super_admins().enumerate() {
            lines.push(row(
                p.labelled(),
                i == screen.super_admin,
                screen.pane == AdminPane::SuperAdmins,
            ));
        }
        lines.push(Line::raw(""));
        lines.push(heading(
            "Unseen items (private, creator gone; no titles are shown)",
            screen.pane == AdminPane::Unseen,
        ));
        if snapshot.unseen.is_empty() {
            lines.push(muted("None."));
        }
        for (i, item) in snapshot.unseen.iter().enumerate() {
            let created = item.created_at.get(..10).unwrap_or(&item.created_at);
            lines.push(row(
                format!(
                    "{} #{}  {}  by {}  {created}",
                    item.kind, item.id, item.workspace_name, item.creator
                ),
                i == screen.unseen,
                screen.pane == AdminPane::Unseen,
            ));
        }
    }
    lines
}

/// What the keys of the active pane do, or the prompt or confirmation open.
fn footer_lines(app: &App, screen: &AdminScreen, width: usize) -> Vec<Line<'static>> {
    let hint = |text: &str| Line::from(Span::styled(text.to_string(), Style::default().fg(MUTED)));
    let t = app.key_table;
    let yes_no = yes_no_hint(t, N::BoardConfirmAdmin, "n");
    let key = |action, shown| format!("[{}]", hint_key(t, N::BoardAdmin, None, action, shown));
    match app.input.mode {
        InputMode::InputAdminText => {
            let label = match screen.prompt {
                Some(AdminPrompt::NewWorkspace) => "New workspace: ",
                Some(AdminPrompt::RenameWorkspace(_)) => "Rename to: ",
                Some(AdminPrompt::AddMember(_)) => "Add (display name or identity): ",
                Some(AdminPrompt::GrantSuperAdmin) => "Super-admin (display name or identity): ",
                None => "",
            };
            vec![
                caret_line(
                    label.to_string(),
                    &app.input.buffer,
                    app.input.caret,
                    width.saturating_sub(label.len()).max(1),
                    Style::default().fg(YELLOW),
                ),
                hint(&format!(
                    "[{}] send  [{}] cancel",
                    hint_key(t, N::BoardText, None, "submit_input", "Enter"),
                    hint_key(t, N::BoardText, None, "cancel_input", "Esc"),
                )),
            ]
        }
        InputMode::ConfirmAdmin => {
            let (text, color) = match &screen.confirm {
                Some(AdminConfirm::DeleteWorkspace { name, members, .. }) => (
                    format!("Delete workspace {name} and its {members} membership(s)? {yes_no}"),
                    RED,
                ),
                Some(AdminConfirm::RemoveMember {
                    person, workspace, ..
                }) => (
                    format!(
                        "Remove {person} from {workspace}? Their private items there stay unseen. {yes_no}"
                    ),
                    RED,
                ),
                Some(AdminConfirm::RevokeSuperAdmin { identity }) => {
                    let name = screen
                        .snapshot
                        .directory
                        .iter()
                        .find(|p| &p.identity == identity)
                        .map_or_else(|| identity.clone(), |p| p.shown_name.clone());
                    (format!("Revoke super-admin from {name}? {yes_no}"), YELLOW)
                }
                Some(AdminConfirm::DeleteUnseen { kind, id }) => (
                    format!(
                        "Delete {kind} {id} for good? An epic goes with its subtree. You cannot read it first. {yes_no}"
                    ),
                    RED,
                ),
                None => (String::new(), YELLOW),
            };
            vec![Line::from(Span::styled(text, Style::default().fg(color)))]
        }
        _ => {
            let keys = match screen.pane {
                AdminPane::Workspaces if screen.snapshot.i_am_super_admin => format!(
                    "{} new  {} rename  {} delete  {} add member",
                    key("admin_new_workspace", "n"),
                    key("admin_rename_workspace", "r"),
                    key("admin_delete_workspace", "d"),
                    key("admin_add_member", "a"),
                ),
                AdminPane::Workspaces => format!(
                    "{} rename  {} add member",
                    key("admin_rename_workspace", "r"),
                    key("admin_add_member", "a"),
                ),
                AdminPane::Members => format!(
                    "{} add  {} admin/member  {} remove",
                    key("admin_add_member", "a"),
                    key("admin_toggle_role", "m"),
                    key("admin_remove_member", "x"),
                ),
                AdminPane::SuperAdmins => format!(
                    "{} grant  {} revoke",
                    key("admin_grant_super_admin", "g"),
                    key("admin_revoke_super_admin", "v"),
                ),
                AdminPane::Unseen => {
                    format!("{} delete item", key("admin_delete_unseen_item", "D"))
                }
            };
            let claim = if screen.snapshot.claimable {
                format!(
                    "  {} claim super-admin",
                    key("admin_claim_super_admin", "c")
                )
            } else {
                String::new()
            };
            let moves = hint_keys(
                t,
                N::BoardAdmin,
                &[("admin_navigate_down", "j"), ("admin_navigate_up", "k")],
            );
            vec![Line::from(vec![
                Span::styled(keys, Style::default().fg(GREEN)),
                Span::styled(
                    format!(
                        "{claim}  {} pane  [{moves}] move  {} close",
                        key("admin_next_pane", "Tab"),
                        key("admin_close", "Esc"),
                    ),
                    Style::default().fg(MUTED),
                ),
            ])]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        AdminSnapshot, DirectoryPerson, UnseenItemRow, WorkspaceDetail, WorkspaceMember,
    };

    fn text(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn screen(super_admin: bool) -> AdminScreen {
        AdminScreen {
            snapshot: AdminSnapshot {
                me: Some("me".into()),
                i_am_super_admin: super_admin,
                claimable: false,
                workspaces: vec![WorkspaceDetail {
                    id: 1,
                    name: "Default".into(),
                    members: vec![WorkspaceMember {
                        membership_id: 7,
                        identity: "aaaaaaaa11".into(),
                        shown_name: "Ann".into(),
                        role: MemberRole::Admin,
                    }],
                }],
                directory: vec![DirectoryPerson {
                    identity: "me".into(),
                    shown_name: "Me".into(),
                    is_super_admin: super_admin,
                }],
                unseen: vec![UnseenItemRow {
                    kind: "epic".into(),
                    id: 12,
                    workspace_id: 1,
                    workspace_name: "Default".into(),
                    creator: "Bob".into(),
                    created_at: "2026-10-30T07:00:00Z".into(),
                }],
            },
            ..AdminScreen::default()
        }
    }

    #[test]
    fn a_super_admin_sees_all_four_sections() {
        let out = text(&admin_lines(&screen(true), 80));
        for want in [
            "Workspaces",
            "Default  (1 member)",
            "Members of Default",
            "Ann (aaaaaaaa)  admin",
            "Super-admins",
            "epic #12  Default  by Bob  2026-10-30",
        ] {
            assert!(out.contains(want), "missing {want:?} in:\n{out}");
        }
    }

    #[test]
    fn a_person_who_administers_nothing_is_told_so_and_sees_no_unseen_items() {
        let mut s = screen(false);
        s.snapshot.workspaces.clear();
        s.snapshot.claimable = true;
        let out = text(&admin_lines(&s, 80));
        assert!(out.contains("You administer no workspace."), "{out}");
        assert!(out.contains("press c to claim"), "{out}");
        assert!(!out.contains("Unseen items"), "{out}");
        assert!(!out.contains("Super-admins"), "{out}");
    }

    #[test]
    fn an_unseen_row_shows_no_title() {
        let out = text(&admin_lines(&screen(true), 80));
        assert!(out.contains("no titles are shown"), "{out}");
    }
}
