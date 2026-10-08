//! The refresh loop: a failure keeps the last good tree.

use super::*;

// ---- refresh: failure keeps the last good tree ------------------------

/// The spec's AgentTreeGitFailureKeepsLastGoodTree: a failed query leaves
/// the tree exactly as it was and says so. Blanking on a transient index
/// lock — the commonest failure, taken by the agent's own git — would make
/// the pane flicker empty.
#[test]
fn a_failed_git_query_keeps_the_last_good_tree_and_sets_a_notice() {
    let mut state = RenderState::new();
    let mut tree = build_tree(&root(), &[]);

    let good = git_rig(&["M", "src/a.rs"], &[]);
    refresh(&root(), &good, &mut tree, &mut state);
    assert!(tree.node_at(&["src", "a.rs"]).is_some());
    assert!(state.notice.is_none());

    let bad = failing_git_rig("fatal: unable to read index.lock\n");
    refresh(&root(), &bad, &mut tree, &mut state);

    assert!(
        tree.node_at(&["src", "a.rs"]).is_some(),
        "the last good tree must survive"
    );
    let notice = state.notice.as_ref().expect("notice set");
    assert!(matches!(notice, Notice::Git(_)), "got {notice:?}");
    assert!(notice.text().contains("index.lock"), "got {notice:?}");
}

/// The warning must name WHY git failed. anyhow's plain Display prints only
/// the outermost context, so a timeout or a failed spawn logged as `could not
/// run git` and nothing else — task #4928's 2139 identical, undiagnosable cards.
#[tokio::test]
async fn a_git_query_that_could_not_run_logs_its_cause() {
    let log = crate::test_log::logged_during(|| async {
        let mut state = RenderState::new();
        let mut tree = build_tree(&root(), &[]);
        let timeout =
            || Err(anyhow::anyhow!("git timed out after 10s").context("could not run git"));
        let timed_out = MockProcessRunner::new(vec![timeout()]);
        refresh(&root(), &timed_out, &mut tree, &mut state);
    })
    .await;

    assert!(log.contains("git query failed"), "got {log}");
    assert!(log.contains("timed out after 10s"), "got {log}");
}

/// A working git retracts its own complaint on the next tick.
#[test]
fn a_recovering_git_query_clears_its_own_notice() {
    let mut state = RenderState::new();
    let mut tree = build_tree(&root(), &[]);

    let bad = failing_git_rig("fatal: unable to read index.lock\n");
    refresh(&root(), &bad, &mut tree, &mut state);
    assert!(state.notice.is_some());

    let good = git_rig(&["M", "a.rs"], &[]);
    refresh(&root(), &good, &mut tree, &mut state);
    assert!(state.notice.is_none());
}

/// ...but it must not swallow the answer to a keypress the user made half a
/// second ago, nor the commits section's own complaint. The notices share
/// one field and one line of border, so the source is what keeps them apart
/// — see NoticeSource in the spec.
#[test]
fn a_successful_git_query_leaves_other_writers_notices_alone() {
    for other in [
        Notice::diff("could not split the diff pane"),
        Notice::commit_list("not a valid object name main"),
    ] {
        let mut state = RenderState::new();
        let mut tree = build_tree(&root(), &[]);
        state.notice = Some(other.clone());

        let good = git_rig(&["M", "a.rs"], &[]);
        refresh(&root(), &good, &mut tree, &mut state);

        assert_eq!(state.notice, Some(other));
    }
}

/// A revert un-badges the file with no bookkeeping: git stops reporting it,
/// so the node goes. This is the second half of task #4408 — a file that is
/// not modified must not show as modified.
#[test]
fn a_reverted_file_disappears_from_the_tree() {
    let mut state = RenderState::new();
    let mut tree = build_tree(&root(), &[]);

    let dirty = git_rig(&["M", "a.rs"], &[]);
    refresh(&root(), &dirty, &mut tree, &mut state);
    assert!(tree.node_at(&["a.rs"]).is_some());

    let clean = git_rig(&[], &[]);
    refresh(&root(), &clean, &mut tree, &mut state);
    assert!(
        tree.node_at(&["a.rs"]).is_none(),
        "a reverted file must leave the tree"
    );
}

/// RefreshAgentTree asks about the SELECTED source: with a commit selected
/// the tick runs the one-commit form — naming that commit, with no
/// untracked listing — and the tree shows that commit's files.
#[test]
fn a_refresh_with_a_commit_selected_shows_that_commit() {
    let mut state = RenderState::new();
    state.selected_commit = Some(COMMIT.to_string());
    let mut tree = build_tree(&root(), &[]);

    let runner = commit_rig(&["A", "src/new.rs"], &["4\t0\tsrc/new.rs"]);
    refresh(&root(), &runner, &mut tree, &mut state);

    assert_eq!(
        tree.node_at(&["src", "new.rs"]).and_then(|n| n.badge),
        Some(FileChange::Added)
    );
    for call in runner.flattened_calls() {
        assert!(call.contains(COMMIT), "{call}");
        assert!(!call.contains("ls-files"), "{call}");
    }
}
