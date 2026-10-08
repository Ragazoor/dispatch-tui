//! The git queries: `git_changes` and `git_branch_commits`.

use super::*;

// ---- git_changes: unstaged work (AgentTreeShowsUnstagedWorkOnly) ------

/// Fork point of HEAD with the LOCAL base branch, in the commit-list rigs.
const LOCAL_FORK: &str = "1111111111111111111111111111111111111111";
/// Fork point of HEAD with the REMOTE-TRACKING base ref.
const REMOTE_FORK: &str = "2222222222222222222222222222222222222222";

/// One `git merge-base` answer, newline-terminated as git writes it.
fn sha(commit: &str) -> Result<std::process::Output> {
    MockProcessRunner::ok_with_stdout(format!("{commit}\n").as_bytes())
}

/// The spec's AgentTreeGitQuery, unstaged form, in full: the working tree
/// against the INDEX — naming no revision — then its counts, then the
/// untracked listing. No branch is resolved, so no merge-base probe runs: a
/// stale or missing base branch cannot fail the tree any more.
#[test]
fn unstaged_work_diffs_the_working_tree_against_the_index_and_resolves_no_branch() {
    let runner = git_rig(&["M", "src/a.rs"], &[]);
    let changes = git_changes(Path::new("/wt"), None, &runner).expect("ok");

    assert_eq!(changes, vec![modified("src/a.rs")]);
    assert_eq!(
        runner.flattened_calls(),
        vec![
            "git -C /wt diff --name-status --no-renames -z".to_string(),
            "git -C /wt diff --numstat --no-renames -z".to_string(),
            "git -C /wt ls-files --others --exclude-standard -z".to_string(),
        ]
    );
}

/// The counts query is a SEPARATE ask against the SAME baseline (the index)
/// and the same rename setting. If the two ever drifted apart, a row's badge
/// and its numbers would be answering different questions.
#[test]
fn git_changes_counts_lines_against_the_same_baseline_as_the_badges() {
    let runner = MockProcessRunner::new(unstaged_out_counted(
        &["M", "src/a.rs"],
        &["12\t3\tsrc/a.rs"],
        &[],
    ));

    let changes = git_changes(Path::new("/wt"), None, &runner).expect("ok");

    assert_eq!(
        changes[0].counts,
        Some(LineCounts {
            added: 12,
            removed: 3
        })
    );
}

/// An untracked file is invisible to a diff against the index, so it comes
/// back with no counts however the query went. The pane must not fill that
/// hole with a zero — see the spec's UntrackedFilesHaveNoLineCounts.
#[test]
fn an_untracked_path_comes_back_with_no_line_counts() {
    let runner = MockProcessRunner::new(unstaged_out_counted(&[], &[], &["brand_new.rs"]));

    let changes = git_changes(Path::new("/wt"), None, &runner).expect("ok");

    assert_eq!(changes, vec![added("brand_new.rs")]);
    assert_eq!(changes[0].counts, None);
}

/// Git C-quotes any path with a non-ASCII byte, and separates the status
/// from the path with a tab, unless `-z` is passed — so `src/é.rs` would
/// arrive as the literal `"src/\303\251.rs"`. Every step of both forms asks
/// for NUL-delimited output.
#[test]
fn every_unstaged_query_asks_for_nul_delimited_output() {
    let runner = git_rig(&[], &[]);
    git_changes(Path::new("/wt"), None, &runner).expect("ok");
    for call in runner.flattened_calls() {
        assert!(
            call.split(' ').any(|arg| arg == "-z"),
            "without -z, quoting breaks non-ASCII names; got {call}"
        );
    }
}

/// The payoff of the flag above: a non-ASCII path survives end to end, from
/// git's stdout to a node the tree can name.
#[test]
fn a_non_ascii_path_survives_parsing_and_tree_building() {
    let runner = git_rig(&["M", "src/é.rs"], &["docs/naïve.md"]);
    let changes = git_changes(Path::new("/wt"), None, &runner).expect("ok");
    assert_eq!(changes, vec![modified("src/é.rs"), added("docs/naïve.md")]);

    let tree = build_tree(&root(), &changes);
    assert_eq!(
        tree.node_at(&["src", "é.rs"]).expect("é.rs").badge,
        Some(FileChange::Modified)
    );
    assert_eq!(
        tree.node_at(&["docs", "naïve.md"]).expect("naïve.md").badge,
        Some(FileChange::Added)
    );
}

/// Every query is bounded, so a git blocked on an index lock the agent
/// itself holds cannot wedge the renderer's single-threaded loop. A tree
/// tick runs at most three commands (config.agent_tree_git_timeout).
#[test]
fn every_unstaged_query_is_bounded_by_a_timeout() {
    let runner = git_rig(&[], &[]);
    git_changes(Path::new("/wt"), None, &runner).expect("ok");
    assert_eq!(runner.recorded_timeouts(), vec![Some(GIT_TIMEOUT); 3]);
}

#[test]
fn git_changes_reports_untracked_files_as_added() {
    let runner = git_rig(&["M", "a.rs"], &["new.rs", "docs/draft.md"]);
    let changes = git_changes(Path::new("/wt"), None, &runner).expect("ok");
    assert_eq!(
        changes,
        vec![modified("a.rs"), added("new.rs"), added("docs/draft.md")]
    );
}

#[test]
fn git_changes_reports_deletions() {
    let runner = git_rig(&["D", "src/old.rs"], &[]);
    let changes = git_changes(Path::new("/wt"), None, &runner).expect("ok");
    assert_eq!(changes, vec![deleted("src/old.rs")]);
}

#[test]
fn git_changes_on_a_clean_worktree_reports_nothing() {
    let runner = git_rig(&[], &[]);
    assert!(git_changes(Path::new("/wt"), None, &runner)
        .expect("ok")
        .is_empty());
}

/// A failing git surfaces its own first stderr line, because that line is
/// what reaches the user's border and has to say something actionable.
#[test]
fn git_changes_fails_with_gits_own_message() {
    let runner = failing_git_rig("fatal: unable to read index.lock\n");
    let err = git_changes(Path::new("/wt"), None, &runner)
        .expect_err("must fail")
        .to_string();
    assert!(err.contains("index.lock"), "got {err}");
}

/// A failing diff short-circuits: the counts and the listing must not run
/// against a repo that just refused to diff.
#[test]
fn a_failing_diff_does_not_run_the_untracked_listing() {
    let runner = failing_git_rig("fatal: unable to read index.lock\n");
    let _ = git_changes(Path::new("/wt"), None, &runner);
    assert_eq!(runner.recorded_calls().len(), 1);
}

// ---- git_changes: one selected commit (AgentTreeSourceIsOneSelection) --

/// ONE COMMIT: two steps, both the commit against its parent with rename
/// detection off and NUL-delimited output, and no untracked listing — a
/// commit has no untracked files. No branch is resolved for this form
/// either.
#[test]
fn a_selected_commit_is_diffed_in_two_nul_delimited_rename_free_queries() {
    let runner = commit_rig(&["M", "a.rs"], &["3\t1\ta.rs"]);
    git_changes(Path::new("/wt"), Some(COMMIT), &runner).expect("ok");

    let calls = runner.flattened_calls();
    assert_eq!(calls.len(), 2, "two steps, no untracked listing: {calls:?}");
    for call in &calls {
        let args: Vec<&str> = call.split(' ').collect();
        assert!(args.contains(&"-z"), "{call}");
        assert!(args.contains(&"--no-renames"), "{call}");
        assert!(call.contains(COMMIT), "the query names the commit: {call}");
        assert!(!call.contains("ls-files"), "{call}");
        assert!(!call.contains("merge-base"), "{call}");
    }
    assert!(calls[0].contains("--name-status"), "{calls:?}");
    assert!(calls[1].contains("--numstat"), "{calls:?}");
}

/// In the commit form every text file carries counts — there is no
/// untracked path for which a count could be missing.
#[test]
fn a_selected_commits_files_all_carry_counts() {
    let runner = commit_rig(
        &["M", "a.rs", "A", "new.rs"],
        &["3\t1\ta.rs", "5\t0\tnew.rs"],
    );
    let changes = git_changes(Path::new("/wt"), Some(COMMIT), &runner).expect("ok");
    assert_eq!(
        changes,
        vec![
            counted("a.rs", FileChange::Modified, 3, 1),
            counted("new.rs", FileChange::Added, 5, 0),
        ]
    );
}

#[test]
fn every_commit_query_is_bounded_by_a_timeout() {
    let runner = commit_rig(&[], &[]);
    git_changes(Path::new("/wt"), Some(COMMIT), &runner).expect("ok");
    assert_eq!(runner.recorded_timeouts(), vec![Some(GIT_TIMEOUT); 2]);
}

/// A commit git cannot read fails the query with git's own words
/// (AgentTreeGitFailureKeepsLastGoodTree: "with a commit selected, git
/// could not read that commit").
#[test]
fn a_commit_git_cannot_read_fails_the_query() {
    let runner = MockProcessRunner::new(vec![MockProcessRunner::fail(
        "fatal: bad object 3333333333333333333333333333333333333333\n",
    )]);
    let err = git_changes(Path::new("/wt"), Some(COMMIT), &runner)
        .expect_err("must fail")
        .to_string();
    assert!(err.contains("bad object"), "got {err}");
}

// ---- git_changes against a real repository ------------------------------

fn real_changes(repo: &TestRepo, commit: Option<&str>) -> Vec<GitFileChange> {
    git_changes(repo.root(), commit, &RealProcessRunner::default()).expect("git_changes")
}

/// Staged changes are not shown: a file staged and not changed again is
/// absent (AgentTreeShowsUnstagedWorkOnly).
#[test]
fn a_staged_change_is_not_shown() {
    let repo = TestRepo::new();
    repo.write("seed.txt", "seed\nstaged\n");
    repo.git(&["add", "seed.txt"]);

    assert_eq!(real_changes(&repo, None), vec![]);
}

/// Against the index, a row counts only the UNSTAGED part of a file.
#[test]
fn only_the_unstaged_part_of_a_file_is_counted() {
    let repo = TestRepo::new();
    repo.write("seed.txt", "seed\nstaged\n");
    repo.git(&["add", "seed.txt"]);
    repo.append("seed.txt", "unstaged 1\nunstaged 2\n");

    assert_eq!(
        real_changes(&repo, None),
        vec![counted("seed.txt", FileChange::Modified, 2, 0)]
    );
}

/// Committing empties the default view of everything that went into the
/// commit — the reversal of the old "Committing does not empty the pane".
#[test]
fn committed_work_leaves_the_default_view() {
    let repo = TestRepo::new();
    repo.write("src/a.rs", "fn a() {}\n");
    repo.commit_all("add a");

    assert_eq!(real_changes(&repo, None), vec![]);
}

#[test]
fn an_untracked_file_is_shown_as_added_with_no_counts() {
    let repo = TestRepo::new();
    repo.write("new.rs", "one\ntwo\n");

    assert_eq!(real_changes(&repo, None), vec![added("new.rs")]);
}

/// `git rm --cached` takes a file out of the index and leaves it on disk:
/// to git it is now untracked, so it is badged added with no counts — and
/// named once, since the two listings are disjoint against the index
/// (ChangePrecedence).
#[test]
fn a_file_taken_out_of_the_index_is_untracked_not_deleted() {
    let repo = TestRepo::new();
    repo.git(&["rm", "-q", "--cached", "seed.txt"]);

    assert_eq!(real_changes(&repo, None), vec![added("seed.txt")]);
}

/// A new file staged and then edited again is badged modified, not added:
/// the index already holds it, and what is unstaged is an edit to that.
#[test]
fn a_new_file_staged_then_edited_again_is_modified() {
    let repo = TestRepo::new();
    repo.write("n.rs", "one\n");
    repo.git(&["add", "n.rs"]);
    repo.append("n.rs", "two\n");

    assert_eq!(
        real_changes(&repo, None),
        vec![counted("n.rs", FileChange::Modified, 1, 0)]
    );
}

/// A tracked file removed from disk is badged deleted until the deletion is
/// staged, and then disappears — a staged deletion is staged work.
#[test]
fn a_deletion_is_shown_until_it_is_staged() {
    let repo = TestRepo::new();
    std::fs::remove_file(repo.root().join("seed.txt")).expect("rm");
    assert_eq!(
        real_changes(&repo, None),
        vec![counted("seed.txt", FileChange::Deleted, 0, 1)]
    );

    repo.git(&["rm", "-q", "seed.txt"]);
    assert_eq!(real_changes(&repo, None), vec![]);
}

/// A selected commit shows exactly that commit against its parent, with
/// counts, whatever the working tree and the index hold meanwhile.
#[test]
fn a_selected_commit_shows_exactly_that_commit_against_its_parent() {
    let repo = TestRepo::new();
    repo.write("a.rs", "a1\n");
    repo.commit_all("first");
    repo.write("a.rs", "a1\na2\n");
    repo.write("b.rs", "b\n");
    let second = repo.commit_all("second");
    // Work in progress the commit view must not see.
    repo.write("a.rs", "changed again\n");
    repo.write("untracked.rs", "x\n");

    let status_before = repo.git(&["status", "--porcelain"]);

    assert_eq!(
        real_changes(&repo, Some(&second)),
        vec![
            counted("a.rs", FileChange::Modified, 1, 0),
            counted("b.rs", FileChange::Added, 1, 0),
        ]
    );
    // ReadOnlyObservation: showing a commit checks nothing out.
    assert_eq!(repo.head(), second);
    assert_eq!(repo.git(&["status", "--porcelain"]), status_before);
}

/// A commit with no parent is diffed against the empty tree, so every file
/// it holds is badged added.
#[test]
fn a_root_commit_is_diffed_against_the_empty_tree() {
    let repo = TestRepo::new();
    let root_commit = repo.git(&["rev-list", "--max-parents=0", "HEAD"]);

    assert_eq!(
        real_changes(&repo, Some(root_commit.trim())),
        vec![counted("seed.txt", FileChange::Added, 1, 0)]
    );
}

/// A merge commit is shown against its FIRST parent: what it brought onto
/// this branch.
#[test]
fn a_merge_commit_is_shown_against_its_first_parent() {
    let repo = TestRepo::new();
    repo.commit_on_main("from_main.txt", "upstream work");
    repo.write("t.rs", "task\n");
    repo.commit_all("task work");
    repo.git(&["merge", "-q", "--no-ff", "main", "-m", "merge main"]);
    let merge = repo.head();

    assert_eq!(
        real_changes(&repo, Some(&merge)),
        vec![counted("from_main.txt", FileChange::Added, 1, 0)]
    );
}

// ---- git_branch_commits: the commits section's read --------------------

/// The fork-point resolution is KEPT, for this list alone: both refs the
/// base branch can denote are probed, and the listing is taken from the
/// fork point they agree on (AgentTreeBaselineIsTaskBaseBranch).
#[test]
fn the_commit_list_resolves_the_fork_point_from_both_base_refs() {
    let runner = MockProcessRunner::new(vec![
        sha(LOCAL_FORK),
        sha(LOCAL_FORK),
        MockProcessRunner::ok_with_stdout(b""),
    ]);
    let commits = git_branch_commits(Path::new("/wt"), "main", &runner).expect("ok");

    assert!(commits.is_empty());
    let calls = runner.flattened_calls();
    assert_eq!(
        &calls[..2],
        [
            "git -C /wt merge-base HEAD main".to_string(),
            "git -C /wt merge-base HEAD origin/main".to_string(),
        ]
    );
    assert_eq!(calls.len(), 3, "one listing after the probes: {calls:?}");
    assert!(calls[2].contains(LOCAL_FORK), "{calls:?}");
}

/// Both probes are built from the task's own base branch name.
#[test]
fn the_commit_list_uses_the_tasks_own_base_branch() {
    let runner = MockProcessRunner::new(vec![
        sha(LOCAL_FORK),
        sha(LOCAL_FORK),
        MockProcessRunner::ok_with_stdout(b""),
    ]);
    git_branch_commits(Path::new("/wt"), "develop", &runner).expect("ok");
    assert_eq!(
        &runner.flattened_calls()[..2],
        [
            "git -C /wt merge-base HEAD develop".to_string(),
            "git -C /wt merge-base HEAD origin/develop".to_string(),
        ]
    );
}

/// Neither ref resolving fails the read, with the LOCAL branch's message —
/// the name the user put on the task — and nothing further runs.
#[test]
fn neither_base_ref_resolving_fails_the_commit_list_with_the_local_branchs_message() {
    let runner = MockProcessRunner::new(vec![
        MockProcessRunner::fail("fatal: Not a valid object name nosuchbranch\n"),
        MockProcessRunner::fail("fatal: Not a valid object name origin/nosuchbranch\n"),
    ]);
    let err = git_branch_commits(Path::new("/wt"), "nosuchbranch", &runner)
        .expect_err("must fail")
        .to_string();
    assert!(err.contains("nosuchbranch"), "got {err}");
    assert!(!err.contains("origin/"), "got {err}");
    assert_eq!(runner.recorded_calls().len(), 2);
}

/// A ranking probe that could not answer fails the read rather than
/// silently keeping the local fork point.
#[test]
fn an_ancestry_probe_that_cannot_answer_fails_the_commit_list() {
    let runner = MockProcessRunner::new(vec![
        sha(LOCAL_FORK),
        sha(REMOTE_FORK),
        MockProcessRunner::fail_with_code(128, "fatal: unable to read pack\n"),
    ]);
    let err = git_branch_commits(Path::new("/wt"), "main", &runner)
        .expect_err("must fail")
        .to_string();
    assert!(err.contains("unable to read pack"), "got {err}");
    assert_eq!(runner.recorded_calls().len(), 3);
}

/// The two probes disagree, and the local fork point is an ancestor of the
/// remote one: the remote one is nearer HEAD and wins.
#[test]
fn a_local_base_behind_its_remote_lists_from_the_remote_fork_point() {
    let runner = MockProcessRunner::new(vec![
        sha(LOCAL_FORK),
        sha(REMOTE_FORK),
        MockProcessRunner::ok(),
        MockProcessRunner::ok_with_stdout(b""),
    ]);
    git_branch_commits(Path::new("/wt"), "main", &runner).expect("ok");
    let calls = runner.flattened_calls();
    assert_eq!(
        calls[2],
        format!("git -C /wt merge-base --is-ancestor {LOCAL_FORK} {REMOTE_FORK}")
    );
    assert!(calls[3].contains(REMOTE_FORK), "{calls:?}");
    assert!(!calls[3].contains(LOCAL_FORK), "{calls:?}");
}

/// The mirror image, and the truly-diverged case: the local ref wins.
#[test]
fn a_local_base_ahead_of_its_remote_lists_from_the_local_fork_point() {
    let runner = MockProcessRunner::new(vec![
        sha(LOCAL_FORK),
        sha(REMOTE_FORK),
        MockProcessRunner::fail_with_code(1, ""),
        MockProcessRunner::ok_with_stdout(b""),
    ]);
    git_branch_commits(Path::new("/wt"), "main", &runner).expect("ok");
    let calls = runner.flattened_calls();
    assert!(calls[3].contains(LOCAL_FORK), "{calls:?}");
}

/// A base branch never checked out locally is ordinary: the read keeps
/// working on the remote ref alone, with no ranking probe.
#[test]
fn a_missing_local_base_branch_still_lists_from_the_remote_ref() {
    let runner = MockProcessRunner::new(vec![
        MockProcessRunner::fail("fatal: Not a valid object name main\n"),
        sha(REMOTE_FORK),
        MockProcessRunner::ok_with_stdout(b""),
    ]);
    git_branch_commits(Path::new("/wt"), "main", &runner).expect("ok");
    let calls = runner.flattened_calls();
    assert_eq!(calls.len(), 3, "{calls:?}");
    assert!(calls[2].contains(REMOTE_FORK), "{calls:?}");
}

/// A repo with no remote-tracking ref lists from the local branch alone.
#[test]
fn a_missing_remote_base_ref_still_lists_from_the_local_branch() {
    let runner = MockProcessRunner::new(vec![
        sha(LOCAL_FORK),
        MockProcessRunner::fail("fatal: Not a valid object name origin/main\n"),
        MockProcessRunner::ok_with_stdout(b""),
    ]);
    git_branch_commits(Path::new("/wt"), "main", &runner).expect("ok");
    let calls = runner.flattened_calls();
    assert_eq!(calls.len(), 3, "{calls:?}");
    assert!(calls[2].contains(LOCAL_FORK), "{calls:?}");
}

/// A probe that exits zero but names no commit is not a candidate.
#[test]
fn a_probe_that_returns_no_commit_is_not_a_fork_point_candidate() {
    let runner = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"\n"),
        sha(REMOTE_FORK),
        MockProcessRunner::ok_with_stdout(b""),
    ]);
    git_branch_commits(Path::new("/wt"), "main", &runner).expect("ok");
    assert!(runner.flattened_calls()[2].contains(REMOTE_FORK));
}

/// A commits-section read runs up to four commands, every one bounded.
#[test]
fn every_commit_list_query_is_bounded_by_a_timeout() {
    let runner = MockProcessRunner::new(vec![
        sha(LOCAL_FORK),
        sha(REMOTE_FORK),
        MockProcessRunner::ok(),
        MockProcessRunner::ok_with_stdout(b""),
    ]);
    git_branch_commits(Path::new("/wt"), "main", &runner).expect("ok");
    assert_eq!(runner.recorded_timeouts(), vec![Some(GIT_TIMEOUT); 4]);
}

/// A listing git refuses fails the read; it must not read as "no commits".
#[test]
fn a_failed_listing_fails_the_commit_list() {
    let runner = MockProcessRunner::new(vec![
        sha(LOCAL_FORK),
        sha(LOCAL_FORK),
        MockProcessRunner::fail("fatal: your current branch appears to be broken\n"),
    ]);
    let err = git_branch_commits(Path::new("/wt"), "main", &runner)
        .expect_err("a failed listing must not read as an empty list")
        .to_string();
    assert!(err.contains("broken"), "got {err}");
}

fn real_commits(repo: &TestRepo) -> Vec<AgentCommit> {
    git_branch_commits(repo.root(), "main", &RealProcessRunner::default()).expect("commits")
}

fn agent_commit(id: &str, subject: &str) -> AgentCommit {
    AgentCommit {
        id: id.to_string(),
        subject: subject.to_string(),
    }
}

/// The agent's commits since the fork point, newest first, as (full id,
/// subject) pairs.
#[test]
fn the_commit_list_is_the_agents_commits_newest_first() {
    let repo = TestRepo::new();
    repo.write("a.rs", "a\n");
    let first = repo.commit_all("first change");
    repo.write("b.rs", "b\n");
    let second = repo.commit_all("second change: naïve résumé");

    assert_eq!(
        real_commits(&repo),
        vec![
            agent_commit(&second, "second change: naïve résumé"),
            agent_commit(&first, "first change"),
        ]
    );
    assert_eq!(second.len(), 40, "selection is keyed by the FULL id");
}

/// An agent that has not committed yet lists nothing — the honest answer.
#[test]
fn an_agent_with_no_commits_lists_none() {
    let repo = TestRepo::new();
    assert_eq!(real_commits(&repo), vec![]);
}

/// Other people's work on the base branch never appears: commits that land
/// on it after the fork are not the agent's.
#[test]
fn commits_landing_on_the_base_branch_after_the_fork_are_not_listed() {
    let repo = TestRepo::new();
    repo.commit_on_main("upstream.txt", "upstream work");
    repo.write("a.rs", "a\n");
    let mine = repo.commit_all("my work");

    assert_eq!(real_commits(&repo), vec![agent_commit(&mine, "my work")]);
}

/// Merging the base branch in moves the fork point forward, so the base
/// branch's commits never appear — while the merge commit the agent made
/// is the agent's own, and is listed.
#[test]
fn merging_the_base_branch_in_never_lists_its_commits() {
    let repo = TestRepo::new();
    repo.commit_on_main("upstream.txt", "upstream work");
    repo.write("a.rs", "a\n");
    let mine = repo.commit_all("my work");
    repo.git(&["merge", "-q", "--no-ff", "main", "-m", "merge main"]);
    let merge = repo.head();

    let ids: Vec<String> = real_commits(&repo).into_iter().map(|c| c.id).collect();
    assert_eq!(ids, vec![merge, mine]);
}

/// Only the newest config.agent_tree_commits_max_listed commits are listed;
/// older ones are not, and no marker says so.
#[test]
fn only_the_newest_fifty_commits_are_listed() {
    let repo = TestRepo::new();
    for n in 1..=55 {
        repo.git(&["commit", "-q", "--allow-empty", "-m", &format!("c{n}")]);
    }

    let commits = real_commits(&repo);
    assert_eq!(commits.len(), crate::agent_tree::changes::MAX_LISTED);
    assert_eq!(commits.first().map(|c| c.subject.as_str()), Some("c55"));
    assert_eq!(commits.last().map(|c| c.subject.as_str()), Some("c6"));
}

/// The stale-local-base case the two-ref resolution exists for: the
/// worktree was branched from the remote-tracking ref, which is ahead of a
/// local base the human never pulled. The upstream commit between the two
/// is not the agent's and must not be listed.
#[test]
fn a_remote_base_ahead_of_a_stale_local_one_moves_the_fork_point() {
    let repo = TestRepo::new();
    let upstream = repo.commit_on_main("upstream.txt", "upstream work");
    repo.git_main(&["update-ref", "refs/remotes/origin/main", &upstream]);
    repo.git_main(&["reset", "-q", "--hard", "HEAD~1"]);
    repo.git(&["reset", "-q", "--hard", "origin/main"]);
    repo.write("a.rs", "a\n");
    let mine = repo.commit_all("my work");

    assert_eq!(real_commits(&repo), vec![agent_commit(&mine, "my work")]);
}
