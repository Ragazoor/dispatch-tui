//! Pure helpers that translate validated `UpdateTaskParams` into a
//! `TaskPatch` for the database layer.
//!
//! Methods that need to read DB state (sub-status legality, epic linkage,
//! etc.) live on `TaskService` in `crud.rs` because they take `&self`.

use crate::db::TaskPatch;
use crate::models::SubStatus;
use crate::service::FieldUpdate;

use super::params::UpdateTaskParams;

/// Build a `TaskPatch` from `UpdateTaskParams`. The expanded repo path and
/// the (already-validated) sub_status are passed in separately because they
/// require either tilde-expansion or a database-bound check before being
/// committed to the patch.
pub(super) fn build_task_patch<'a>(
    params: &'a UpdateTaskParams,
    expanded_repo_path: Option<&'a str>,
    sub_status: Option<SubStatus>,
) -> TaskPatch<'a> {
    let mut patch = TaskPatch::new();
    if let Some(s) = params.status {
        patch = patch.status(s);
    }
    if let Some(p) = params.plan_path.as_deref() {
        patch = patch.plan_path(Some(p));
    }
    if let Some(t) = params.title.as_deref() {
        patch = patch.title(t);
    }
    if let Some(d) = params.description.as_deref() {
        patch = patch.description(d);
    }
    if let Some(r) = expanded_repo_path {
        patch = patch.repo_path(r);
    }
    if let Some(so) = params.sort_order {
        patch = patch.sort_order(Some(so));
    }
    if let Some(update) = params.pr_url.as_ref() {
        patch = match update {
            FieldUpdate::Set(url) => patch.pr_url(Some(url.as_str())),
            FieldUpdate::Clear => patch.pr_url(None),
        };
    }
    if let Some(tag) = params.tag {
        patch = patch.tag(Some(tag));
    }
    if let Some(update) = params.worktree.as_ref() {
        patch = match update {
            FieldUpdate::Set(wt) => patch.worktree(Some(wt.as_str())),
            FieldUpdate::Clear => patch.worktree(None),
        };
    }
    if let Some(update) = params.tmux_window.as_ref() {
        patch = match update {
            FieldUpdate::Set(tw) => patch.tmux_window(Some(tw.as_str())),
            FieldUpdate::Clear => patch.tmux_window(None),
        };
    }
    if let Some(bb) = params.base_branch.as_deref() {
        patch = patch.base_branch(bb);
    }
    if let Some(ss) = sub_status {
        patch = patch.sub_status(ss);
    }
    if let Some(pid) = params.project_id {
        patch = patch.project_id(pid);
    }
    if let Some(ts) = params.last_pre_tool_use_at {
        patch = patch.last_pre_tool_use_at(ts);
    }
    patch
}
