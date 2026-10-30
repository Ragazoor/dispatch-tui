//! Epic creation, take-over prompt and reparent flow, as free functions over
//! the fields each step touches. The `EpicMessage` route passes the fields
//! in, so none of these is an `App` method.

use super::super::types::*;
use super::super::{truncate_title, InteractionState, TITLE_DISPLAY_LENGTH};
use crate::models::{Epic, EpicId};

/// `epics.allium: EditEpic`'s take-over prompt. Shows a y/n confirmation
/// naming the conflicting owner; the `feed_command` edit that triggered
/// this has already applied by the time this fires — see
/// `TuiRuntime::finalize_epic_edit`, which is the only emitter.
pub(in crate::tui) fn feed_owner_takeover_offered(
    input: &mut InputState,
    status: &mut StatusState,
    epic_id: EpicId,
    other_host: String,
) -> Vec<Command> {
    status.set(format!(
        "This feed is currently owned by host {other_host} — take over polling? [y/N]"
    ));
    input.mode = InputMode::ConfirmOverrideFeedOwner {
        epic_id,
        other_host,
    };
    vec![]
}

pub(in crate::tui) fn start_new_epic(
    input: &mut InputState,
    status: &mut StatusState,
    view_mode: &ViewMode,
) -> Vec<Command> {
    input.mode = InputMode::InputEpicTitle;
    input.clear_buffer();
    let parent_epic_id = if let ViewMode::Epic { epic_id, .. } = *view_mode {
        Some(epic_id)
    } else {
        None
    };
    input.epic_draft = Some(EpicDraft {
        parent_epic_id,
        ..Default::default()
    });
    status.set("Epic title: ".to_string());
    // A sub-epic takes its parent's workspace, so only a root epic needs the
    // person's workspaces for the placement step.
    if parent_epic_id.is_none() {
        vec![Command::People(
            crate::tui::commands::PeopleCommand::LoadWorkspaces,
        )]
    } else {
        vec![]
    }
}

pub(in crate::tui) fn submit_epic_title(
    input: &mut InputState,
    status: &mut StatusState,
    value: String,
) -> Vec<Command> {
    input.clear_buffer();
    if value.is_empty() {
        input.mode = InputMode::Normal;
        status.clear();
        vec![]
    } else {
        let parent_epic_id = input.epic_draft.as_ref().and_then(|d| d.parent_epic_id);
        input.epic_draft = Some(EpicDraft {
            title: value,
            description: String::new(),
            parent_epic_id,
            ..Default::default()
        });
        input.mode = InputMode::InputEpicDescription;
        status.set("Opening editor for description...".to_string());
        vec![Command::Editor(
            crate::tui::commands::EditorCommand::PopOut(EditKind::Description { is_epic: true }),
        )]
    }
}

pub(in crate::tui) fn apply_epic_description(input: &mut InputState, value: String) {
    input.clear_buffer();
    if let Some(ref mut draft) = input.epic_draft {
        draft.description = value;
    }
}

// -----------------------------------------------------------------------
// Reparent epic handlers
// -----------------------------------------------------------------------

pub(in crate::tui) fn start_reparent(
    input: &mut InputState,
    interaction: &mut InteractionState,
    epic_id: EpicId,
    items: Vec<tui_tree_widget::TreeItem<'static, String>>,
    current_parent: Option<EpicId>,
) {
    let tree_state = crate::tui::ui::tree_state_at(&items, current_parent);
    interaction.reparent_picker = Some(crate::tui::ReparentPickerState {
        epic_id,
        tree_state: std::cell::RefCell::new(tree_state),
        items,
    });
    input.mode = InputMode::ReparentEpic(epic_id);
}

pub(in crate::tui) fn reparent_navigate(interaction: &InteractionState, nav: TreeNav) -> bool {
    if let Some(picker) = &interaction.reparent_picker {
        picker.navigate(nav);
        return true;
    }
    false
}

pub(in crate::tui) fn reparent_confirm(
    input: &mut InputState,
    interaction: &InteractionState,
    status: &mut StatusState,
    epics: &[Epic],
) {
    let epic_id = match input.mode {
        InputMode::ReparentEpic(id) => id,
        _ => return,
    };

    let selected_id: Option<String> = interaction
        .reparent_picker
        .as_ref()
        .and_then(|p| p.selected_id());

    let new_parent: Option<EpicId> = match selected_id.as_deref() {
        Some(s) if s != crate::tui::types::REPARENT_NO_PARENT_SENTINEL => s
            .strip_prefix("epic:")
            .and_then(|n| n.parse::<i64>().ok())
            .map(EpicId),
        _ => None,
    };

    let moving_title = epics
        .iter()
        .find(|e| e.id == epic_id)
        .map(|e| truncate_title(&e.title, TITLE_DISPLAY_LENGTH))
        .unwrap_or_default();

    let msg = match new_parent {
        None => format!("Make {moving_title} a root epic? [y/n]"),
        Some(pid) => {
            let parent_label = epics
                .iter()
                .find(|e| e.id == pid)
                .map(|e| truncate_title(&e.title, TITLE_DISPLAY_LENGTH))
                .unwrap_or_else(|| format!("\"epic #{}\"", pid.0));
            format!("Reparent {moving_title} under {parent_label}? [y/n]")
        }
    };

    input.mode = InputMode::ConfirmReparentEpic {
        epic_id,
        new_parent,
    };
    status.set(msg);
}

fn clear_reparent_state(
    input: &mut InputState,
    interaction: &mut InteractionState,
    status: &mut StatusState,
) {
    input.mode = InputMode::Normal;
    interaction.reparent_picker = None;
    status.clear();
}

pub(in crate::tui) fn reparent_execute(
    input: &mut InputState,
    interaction: &mut InteractionState,
    status: &mut StatusState,
) -> Vec<Command> {
    let (epic_id, new_parent) = match input.mode {
        InputMode::ConfirmReparentEpic {
            epic_id,
            new_parent,
        } => (epic_id, new_parent),
        _ => return vec![],
    };
    clear_reparent_state(input, interaction, status);
    vec![Command::Epic(crate::tui::commands::EpicCommand::Reparent {
        id: epic_id,
        new_parent,
    })]
}

/// Cancel the reparent flow entirely (Esc/q from ConfirmReparentEpic),
/// returning to Normal mode and clearing the picker.
pub(in crate::tui) fn reparent_cancel_all(
    input: &mut InputState,
    interaction: &mut InteractionState,
    status: &mut StatusState,
) -> Vec<Command> {
    clear_reparent_state(input, interaction, status);
    vec![]
}

pub(in crate::tui) fn reparent_cancel(
    input: &mut InputState,
    interaction: &mut InteractionState,
    status: &mut StatusState,
) -> Vec<Command> {
    match input.mode {
        InputMode::ConfirmReparentEpic { epic_id, .. } => {
            input.mode = InputMode::ReparentEpic(epic_id);
            status.clear();
        }
        InputMode::ReparentEpic(_) => {
            input.mode = InputMode::Normal;
            interaction.reparent_picker = None;
        }
        _ => {}
    }
    vec![]
}
