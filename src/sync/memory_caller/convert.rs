//! `module::*` <-> `bindings::*` row conversions.

use dispatch_spacetime_module as module;

use crate::spacetime::bindings;

// ---------------------------------------------------------------------------
// module <-> bindings conversions
// ---------------------------------------------------------------------------
//
// `spacetime/module`'s table/patch structs and `src/spacetime/bindings`'s
// generated ones are structurally identical (bindings are generated FROM the
// module's schema) but are two distinct Rust types in two distinct crates.
// `mirror!` writes both `From` impls from one field list; a field the module
// adds and this list omits is a compile error in the generated `Self { .. }`
// literal (neither struct derives `Default`), which is what keeps this honest
// as the module's schema grows.

macro_rules! mirror {
    ($module:ty, $bindings:ty { $($field:ident),+ $(,)? }) => {
        impl From<$bindings> for $module {
            fn from(v: $bindings) -> Self {
                Self { $($field: v.$field),+ }
            }
        }
        impl From<$module> for $bindings {
            fn from(v: $module) -> Self {
                Self { $($field: v.$field),+ }
            }
        }
    };
}

mirror!(
    module::Task,
    bindings::Task {
        id,
        title,
        description,
        repo_path,
        status,
        worktree,
        tmux_window,
        plan_path,
        epic_id,
        sub_status,
        tag,
        sort_order,
        created_at,
        updated_at,
        base_branch,
        external_id,
        labels,
        last_pre_tool_use_at,
        last_notification_at,
        wrap_up_mode,
        url,
        url_type,
        pr_learnings_gate_shown_at,
        auto_run_plan,
        live_subagents,
        stop_pending,
        stop_pending_at,
        live_shells,
        oldest_live_shell_started_at,
        last_peer_message_sent_at,
        last_peer_message_received_at,
        phoenix,
        host,
        owner,
        completed_at,
        created_by,
    }
);

mirror!(
    module::TaskPatch,
    bindings::TaskPatch {
        title,
        description,
        repo_path,
        status,
        worktree,
        tmux_window,
        plan_path,
        epic_id,
        sub_status,
        tag,
        sort_order,
        base_branch,
        external_id,
        labels,
        last_pre_tool_use_at,
        last_notification_at,
        wrap_up_mode,
        url,
        url_type,
        pr_learnings_gate_shown_at,
        auto_run_plan,
        live_subagents,
        stop_pending,
        stop_pending_at,
        last_peer_message_sent_at,
        last_peer_message_received_at,
        phoenix,
        host,
        owner,
        completed_at,
    }
);

mirror!(
    module::Epic,
    bindings::Epic {
        id,
        title,
        description,
        status,
        plan_path,
        sort_order,
        created_at,
        updated_at,
        auto_dispatch,
        parent_epic_id,
        feed_command,
        feed_interval_secs,
        group_by_repo,
        feed_role,
        origin,
        feed_append_only,
        completed_at,
        created_by,
    }
);

mirror!(
    module::EpicPatch,
    bindings::EpicPatch {
        title,
        description,
        status,
        plan_path,
        sort_order,
        auto_dispatch,
        parent_epic_id,
        feed_command,
        feed_interval_secs,
        group_by_repo,
        feed_role,
        origin,
        feed_append_only,
        completed_at,
    }
);

mirror!(
    module::RepoPath,
    bindings::RepoPath {
        id,
        path,
        last_used,
        verify_command
    }
);

mirror!(
    module::RepoBaseBranch,
    bindings::RepoBaseBranch {
        id,
        repo_path,
        branch,
        last_used
    }
);

mirror!(
    module::Subscription,
    bindings::Subscription {
        id,
        epic_id,
        subscriber
    }
);

mirror!(
    module::Setting,
    bindings::Setting {
        id,
        host,
        key,
        value
    }
);

mirror!(
    module::UsageEvent,
    bindings::UsageEvent {
        id,
        recorded_at,
        category,
        action,
        detail,
        actor,
    }
);

mirror!(
    module::TaskWatcher,
    bindings::TaskWatcher {
        id,
        watcher_task_id,
        target_task_id,
        created_at
    }
);

mirror!(
    module::PollOwner,
    bindings::PollOwner {
        id,
        scope,
        scope_id,
        host,
        claimed_at
    }
);

mirror!(module::Host, bindings::Host { id, label, owner });

mirror!(
    module::RetiredFeedItem,
    bindings::RetiredFeedItem {
        id,
        feed_epic_id,
        external_id,
        retired_at
    }
);

mirror!(
    module::FeedTaskUpsertItem,
    bindings::FeedTaskUpsertItem {
        external_id,
        title,
        description,
        repo_path,
        status,
        sub_status,
        base_branch,
        tag,
        labels,
        sort_order,
        url,
        url_type,
        wrap_up_mode,
    }
);

mirror!(
    module::SubStatusUpdate,
    bindings::SubStatusUpdate {
        task_id,
        sub_status
    }
);

mirror!(
    module::Learning,
    bindings::Learning {
        id,
        kind,
        summary,
        detail,
        scope,
        scope_ref,
        tags,
        status,
        source_task_id,
        upvote_count,
        last_upvoted_at,
        created_at,
        updated_at,
        embedding,
    }
);

mirror!(
    module::LearningPatch,
    bindings::LearningPatch {
        status,
        summary,
        embedding,
    }
);

mirror!(
    module::LearningRetrieval,
    bindings::LearningRetrieval {
        id,
        task_id,
        learning_id,
        source,
        retrieved_at,
    }
);
