//! Form input, text-entry, creation/edit/delete-flow handlers.

use std::collections::HashMap;

use crate::models::{EpicId, Task, TaskTag, WrapUpMode};

use super::super::types::*;
use super::super::{filtered_repos, has_new_repo_option, InteractionState};

// Free functions over the fields each form step touches. The `InputMessage`
// route passes `&mut app.input` and `&mut app.status` directly, so none of
// these is an `App` method.

pub(in crate::tui) fn copy_task(
    input: &mut InputState,
    status: &mut StatusState,
    task: &Task,
) -> Vec<Command> {
    let title = format!("Copy of: {}", task.title);
    let description = task.description.clone();
    let repo_path = task.repo_path.clone();
    let tag = task.tag;
    let wrap_up_mode = task.wrap_up_mode;
    // The copy runs the form from InputTag on (CopyTask in
    // docs/specs/tasks.allium): that step is where phoenix is armed, and a
    // copy that skipped it would be the one flow with no way to arm the
    // flag. The source's repo_path rides on the draft rather than in the
    // buffer, because InputRepoPath is now one step further along.
    input.task_draft = Some(TaskDraft {
        title,
        description,
        repo_path,
        tag,
        wrap_up_mode,
        ..Default::default()
    });
    input.clear_buffer();
    input.copy_flow = true;
    input.mode = InputMode::InputTag;
    status.set(crate::tui::ui::tag_prompt(false).to_string());
    vec![]
}

pub(in crate::tui) fn start_new_task(
    input: &mut InputState,
    status: &mut StatusState,
) -> Vec<Command> {
    input.mode = InputMode::InputTitle;
    input.clear_buffer();
    input.task_draft = None;
    input.copy_flow = false;
    status.set("Enter title: ".to_string());
    vec![]
}

pub(in crate::tui) fn cancel_input(
    input: &mut InputState,
    interaction: &mut InteractionState,
    status: &mut StatusState,
    package_count: usize,
) -> Vec<Command> {
    // The new-package title belongs to the link picker: Esc returns there
    // (`work-packages.allium: WorkPackageLinkPicker`).
    if let InputMode::InputWorkPackageTitle(item) = input.mode {
        input.reopen_link_picker(item, package_count);
        return vec![];
    }
    input.mode = InputMode::Normal;
    input.clear_buffer();
    input.task_draft = None;
    input.copy_flow = false;
    input.pending_epic_id = None;
    interaction.pending_g = None;
    status.clear();
    vec![]
}

pub(in crate::tui) fn submit_title(
    input: &mut InputState,
    status: &mut StatusState,
    value: String,
) -> Vec<Command> {
    input.clear_buffer();
    if value.is_empty() {
        input.mode = InputMode::Normal;
        input.task_draft = None;
        status.clear();
    } else {
        input.task_draft = Some(TaskDraft {
            title: value,
            description: String::new(),
            repo_path: String::new(),
            tag: None,
            base_branch: "main".to_string(),
            wrap_up_mode: None,
            phoenix: false,
            work_package: None,
        });
        input.mode = InputMode::InputTag;
        status.set(crate::tui::ui::tag_prompt(false).to_string());
    }
    vec![]
}

/// Enter the repo-path step, prefilling the picker from the draft's own
/// `repo_path`.
///
/// Shared by the two steps that hand over to it — InputDescription (new
/// task) and InputTag (copy) — so the step's entry invariants and its
/// prompt live in one place. The new-task draft carries an empty
/// `repo_path`, so the prefill is a no-op there; the copy's carries the
/// source's path.
fn enter_repo_path_step(input: &mut InputState, status: &mut StatusState) {
    input.enter_repo_path_step();
    status.set("Enter repo path: ".to_string());
}

pub(in crate::tui) fn submit_description(
    input: &mut InputState,
    status: &mut StatusState,
    value: String,
) -> Vec<Command> {
    if let Some(ref mut draft) = input.task_draft {
        draft.description = value;
    }
    enter_repo_path_step(input, status);
    vec![]
}

pub(in crate::tui) fn submit_repo_path(
    input: &mut InputState,
    status: &mut StatusState,
    base_branches: &HashMap<String, Vec<String>>,
    value: String,
) -> Vec<Command> {
    input.clear_buffer();
    if value.is_empty() {
        status.set("Repo path required (no saved paths available)".to_string());
        return vec![];
    }
    // Accepted std::fs-in-handler exception (docs/conventions.md, "No
    // std::fs inside async handlers"): a bare exists()/is_dir() stat, no
    // read or parse, on the low-frequency repo-path submit path.
    if let Err(msg) = crate::dispatch::validate_repo_path(&value) {
        status.set(msg);
        return vec![];
    }
    if let Some(ref mut draft) = input.task_draft {
        draft.repo_path = value.clone();
    }
    let default_base_branch = input
        .task_draft
        .as_ref()
        .map(|d| d.base_branch.clone())
        .unwrap_or_else(|| "main".to_string());
    // PrefillFromHistory (dispatch.allium: BaseBranchPicker): prefer the
    // most-recently-used branch for this repo; fall back to the draft
    // default when the repo has no history yet.
    let remembered = base_branches.get(&value).and_then(|v| v.first()).cloned();
    let base_branch = remembered.clone().unwrap_or(default_base_branch);
    input.set_buffer(base_branch.clone());
    input.repo_cursor = 0;
    input.mode = InputMode::InputBaseBranch;
    status.set("Base branch: ".to_string());
    // No history means the user has never answered this for this repo, so
    // the fallback above is a guess — ask the repository instead
    // (DefaultBaseBranchIsDetectedNotAssumed). Reading it is a subprocess,
    // so the field is already open by the time the answer lands; it names
    // the buffer it is allowed to replace so it cannot overwrite typing.
    // A remembered branch IS the user's answer, and beats origin/HEAD.
    if remembered.is_some() {
        return vec![];
    }
    vec![Command::Settings(
        crate::tui::commands::SettingsCommand::DetectDefaultBranch {
            repo_path: value,
            replacing: base_branch,
        },
    )]
}

/// Apply a repository's detected default branch to the base-branch field,
/// if the field is still waiting for it.
///
/// Implements `DetectedPrefillNeverOverwritesTyping` (docs/specs/dispatch.allium).
/// Both guards matter and neither subsumes the other: the mode check
/// catches a form that has moved on, and the buffer check catches a user
/// who started typing while the repository was being read. An answer that
/// passes neither is dropped in silence — a prefill is a convenience, and
/// announcing a convenience that did not apply is noise.
pub(in crate::tui) fn default_branch_detected(
    input: &mut InputState,
    branch: String,
    replacing: String,
) -> Vec<Command> {
    input.apply_default_branch(branch, &replacing);
    vec![]
}

pub(in crate::tui) fn submit_base_branch(
    input: &mut InputState,
    status: &mut StatusState,
    value: String,
) -> Vec<Command> {
    let base_branch = if value.is_empty() {
        input
            .task_draft
            .as_ref()
            .map(|d| d.base_branch.clone())
            .unwrap_or_else(|| "main".to_string())
    } else {
        value
    };
    if let Some(ref mut draft) = input.task_draft {
        draft.base_branch = base_branch;
    }
    input.clear_buffer();
    input.mode = InputMode::InputWrapUpMode;
    status.set("Wrap-up: [r]ebase  [p]r  [d]one  [Enter] skip".to_string());
    vec![]
}

///
/// The caller then runs `App::finish_task_creation`, which needs the whole
/// app.
pub(in crate::tui) fn apply_wrap_up_mode(input: &mut InputState, mode: Option<WrapUpMode>) {
    // `mode == None` means Enter was pressed without an explicit r/p/d
    // pick — leave the draft's existing value untouched rather than
    // clearing it, so CopyTask's prefilled source-task mode survives an
    // Enter-to-skip instead of being silently dropped.
    if let (Some(ref mut draft), Some(m)) = (input.task_draft.as_mut(), mode) {
        draft.wrap_up_mode = Some(m);
    }
}

/// `p` at the tag picker (CreateTask: PhoenixArming, in
/// docs/specs/tasks.allium). Arms the flag and re-opens the SAME step, so
/// the operator still picks the task's real tag.
///
/// There is no disarming counterpart: declining the recurrence is not
/// pressing `p`, so an ordinary task costs no keypress at all for it.
/// Nothing seeds the flag either — `CopyTask` deliberately does not carry
/// it (see docs/specs/tasks.allium), so every draft reaches this step
/// unarmed even when the source was a phoenix.
pub(in crate::tui) fn arm_phoenix(
    input: &mut InputState,
    status: &mut StatusState,
) -> Vec<Command> {
    if let Some(ref mut draft) = input.task_draft {
        draft.phoenix = true;
    }
    // Mode is already InputTag; re-advertise the accepted set without `p`.
    status.set(crate::tui::ui::tag_prompt(true).to_string());
    vec![]
}

/// Leaves the tag picker for whichever step follows it: InputDescription
/// for a new task, InputRepoPath for a copy (whose description is already
/// carried). See CopyTask in docs/specs/tasks.allium.
///
/// `tag == None` means Enter was pressed without an explicit pick — leave
/// the draft's existing value alone rather than clearing it, so CopyTask's
/// seeded tag survives the step (CreateTask: EnterKeepsTheDraft). Same
/// rule, and same reason, as `apply_wrap_up_mode` above.
pub(in crate::tui) fn submit_tag(
    input: &mut InputState,
    status: &mut StatusState,
    tag: Option<TaskTag>,
) -> Vec<Command> {
    input.clear_buffer();
    if let (Some(ref mut draft), Some(t)) = (input.task_draft.as_mut(), tag) {
        draft.tag = Some(t);
    }
    // Taken, not read: the marker's only reader is here, so consuming it
    // retires the stale-`true` class outright rather than relying on every
    // exit path from the form remembering to clear it. Same discipline as
    // `pending_epic_id.take()` below.
    if std::mem::take(&mut input.copy_flow) {
        enter_repo_path_step(input, status);
        return vec![];
    }
    input.mode = InputMode::InputDescription;
    status.set("Opening editor for description...".to_string());
    vec![Command::Editor(
        crate::tui::commands::EditorCommand::PopOut(EditKind::Description { is_epic: false }),
    )]
}

pub(in crate::tui) fn start_quick_dispatch_selection(
    input: &mut InputState,
    status: &mut StatusState,
) -> Vec<Command> {
    input.mode = InputMode::QuickDispatch;
    input.repo_cursor = 0;
    input.clear_buffer();
    status.set("Type to filter · ↑/↓ navigate · Enter select · Esc cancel".to_string());
    vec![]
}

/// Close the quick-dispatch picker on row `idx` and return the chosen repo
/// with the epic the dispatch is for. `None` leaves the picker open.
pub(in crate::tui) fn take_quick_dispatch_repo(
    input: &mut InputState,
    status: &mut StatusState,
    repo_paths: &[String],
    idx: usize,
) -> Option<(String, Option<EpicId>)> {
    let repos = filtered_repos(repo_paths, &input.buffer);
    let repo_path = if idx < repos.len() {
        repos[idx].clone()
    } else if has_new_repo_option(&input.buffer, &repos) {
        input.buffer.clone()
    } else {
        return None;
    };
    let epic_id = input.pending_epic_id.take();
    input.mode = InputMode::Normal;
    input.clear_buffer();
    status.clear();
    Some((repo_path, epic_id))
}

pub(in crate::tui) fn cancel_retry(
    input: &mut InputState,
    status: &mut StatusState,
) -> Vec<Command> {
    input.mode = InputMode::Normal;
    status.clear();
    vec![]
}
