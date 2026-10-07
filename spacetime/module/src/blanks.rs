//! The one blank row per table. `pub` for the blanks the in-process twin and the tests build their rows from.

use super::*;

/// The one blank task row. `pub` so the in-process twin and the tests build
/// their rows from it instead of restating the ~40 fields.
pub fn blank_task() -> Task {
    Task {
        id: 0,
        title: String::new(),
        description: String::new(),
        repo_path: String::new(),
        status: BACKLOG.into(),
        worktree: String::new(),
        tmux_window: String::new(),
        plan_path: String::new(),
        epic_id: 0,
        sub_status: "none".into(),
        tag: String::new(),
        sort_order: None,
        created_at: String::new(),
        updated_at: String::new(),
        base_branch: "main".into(),
        external_id: String::new(),
        labels: "[]".into(),
        last_pre_tool_use_at: String::new(),
        last_notification_at: String::new(),
        wrap_up_mode: String::new(),
        url: String::new(),
        url_type: String::new(),
        pr_learnings_gate_shown_at: String::new(),
        auto_run_plan: false,
        live_subagents: 0,
        stop_pending: false,
        stop_pending_at: String::new(),
        live_shells: 0,
        oldest_live_shell_started_at: String::new(),
        last_peer_message_sent_at: String::new(),
        last_peer_message_received_at: String::new(),
        phoenix: false,
        host: String::new(),
        // A blank has no epic, so the invariant requires an owner. This one is
        // never a real person: the row exists to be generated and thrown away
        // by `burn_id_sequence`, and the only copy that outlives a call is the
        // `probe_generated_task_id` row an operator deletes by hand.
        owner: SCRATCH_OWNER.into(),
        completed_at: String::new(),
        created_by: String::new(),
    }
}

/// The owner on a throwaway row. Not a `UserIdentity` and not shaped like one,
/// so a scratch row that escapes into a real board is obvious rather than
/// plausible.
pub const SCRATCH_OWNER: &str = "module-scratch";

/// The one blank epic row; see [`blank_task`].
pub fn blank_epic() -> Epic {
    Epic {
        id: 0,
        title: String::new(),
        description: String::new(),
        status: BACKLOG.into(),
        plan_path: String::new(),
        sort_order: None,
        created_at: String::new(),
        updated_at: String::new(),
        auto_dispatch: false,
        parent_epic_id: 0,
        feed_command: String::new(),
        feed_interval_secs: 0,
        group_by_repo: false,
        feed_role: "none".into(),
        origin: "manual".into(),
        feed_append_only: false,
        completed_at: String::new(),
        created_by: String::new(),
    }
}

pub(crate) fn blank_todo() -> Todo {
    Todo {
        id: 0,
        title: String::new(),
        done: false,
        sort_order: 0,
        created_at: String::new(),
        task_id: 0,
        epic_id: 0,
        parent_id: 0,
        owner: String::new(),
    }
}

pub(crate) fn blank_watcher() -> TaskWatcher {
    TaskWatcher {
        id: 0,
        watcher_task_id: 0,
        target_task_id: 0,
        created_at: String::new(),
    }
}

pub(crate) fn blank_repo_path() -> RepoPath {
    RepoPath {
        id: 0,
        path: String::new(),
        last_used: String::new(),
        verify_command: String::new(),
    }
}

pub(crate) fn blank_repo_base_branch() -> RepoBaseBranch {
    RepoBaseBranch {
        id: 0,
        repo_path: String::new(),
        branch: String::new(),
        last_used: String::new(),
    }
}

pub(crate) fn blank_poll_owner() -> PollOwner {
    PollOwner {
        id: 0,
        scope: String::new(),
        scope_id: 0,
        host: String::new(),
        claimed_at: String::new(),
    }
}

pub(crate) fn blank_learning() -> Learning {
    Learning {
        id: 0,
        kind: String::new(),
        summary: String::new(),
        detail: None,
        scope: String::new(),
        scope_ref: None,
        tags: String::new(),
        status: String::new(),
        source_task_id: None,
        upvote_count: 0,
        last_upvoted_at: None,
        created_at: String::new(),
        updated_at: String::new(),
        embedding: None,
    }
}

pub(crate) fn blank_learning_retrieval() -> LearningRetrieval {
    LearningRetrieval {
        id: 0,
        task_id: 0,
        learning_id: 0,
        source: String::new(),
        retrieved_at: String::new(),
    }
}

pub(crate) fn blank_usage_event() -> UsageEvent {
    UsageEvent {
        id: 0,
        recorded_at: String::new(),
        category: String::new(),
        action: String::new(),
        detail: None,
        actor: String::new(),
    }
}

pub(crate) fn blank_retired_feed_item() -> RetiredFeedItem {
    RetiredFeedItem {
        id: 0,
        feed_epic_id: 0,
        external_id: String::new(),
        retired_at: String::new(),
    }
}
