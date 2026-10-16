use super::*;

#[test]
fn pr_rebase_preamble_targets_pr_branch_on_demand() {
    let text = pr_rebase_preamble("renovate/serde-1.x");
    assert!(
        text.contains("git fetch origin renovate/serde-1.x"),
        "should fetch the PR branch, got: {text}"
    );
    assert!(
        text.contains("git rebase origin/renovate/serde-1.x"),
        "should rebase onto origin/<pr-branch>, got: {text}"
    );
    assert!(
        !text.contains("rebase main"),
        "must not rebase onto the base branch, got: {text}"
    );
}

#[test]
fn reused_preamble_targets_a_local_start_point() {
    let sp = StartPoint::Local {
        base: "develop".to_string(),
    };
    let text = reused_rebase_preamble(&sp);
    assert!(text.contains("git fetch origin develop"), "got: {text}");
    assert!(text.contains("git rebase develop"), "got: {text}");
    assert!(
        !text.contains("git rebase origin/develop"),
        "must not drag a local-based branch back onto origin: {text}"
    );
    assert!(
        text.contains("git status"),
        "tells the agent to inspect first"
    );
    assert!(!text.contains("main"), "no literal main: {text}");
}

#[test]
fn reused_preamble_targets_a_remote_start_point() {
    let sp = StartPoint::Remote {
        base: "develop".to_string(),
    };
    let text = reused_rebase_preamble(&sp);
    assert!(text.contains("git fetch origin develop"), "got: {text}");
    assert!(text.contains("git rebase origin/develop"), "got: {text}");
}

#[test]
fn select_preamble_is_empty_for_a_fresh_worktree() {
    let sp = StartPoint::Remote {
        base: "main".to_string(),
    };
    assert_eq!(select_preamble(None, Some(&sp), false), "");
}

#[test]
fn select_preamble_uses_reuse_wording_for_a_reused_worktree() {
    let sp = StartPoint::Local {
        base: "main".to_string(),
    };
    let text = select_preamble(None, Some(&sp), true);
    assert!(
        text.contains("reused from a previous attempt"),
        "got: {text}"
    );
    assert!(
        text.contains("git rebase main"),
        "mirrors the start point: {text}"
    );
}

#[test]
fn select_preamble_prefers_the_pr_branch_regardless_of_reuse() {
    let sp = StartPoint::Remote {
        base: "renovate/serde-1.x".to_string(),
    };
    for reused in [true, false] {
        let text = select_preamble(Some("renovate/serde-1.x"), Some(&sp), reused);
        assert!(
            text.contains("git rebase origin/renovate/serde-1.x"),
            "reused={reused}, got: {text}"
        );
        assert!(
            !text.contains("reused from a previous attempt"),
            "reused={reused}"
        );
    }
}

#[test]
fn prompt_head_carries_the_warning_even_with_no_preamble() {
    // The fresh + no-origin-ref case: nothing to rebase onto, but the agent
    // must still be told its base is local-only.
    let head = compose_prompt_head("", Some("origin has no branch main"));
    assert!(
        head.contains("Note: origin has no branch main"),
        "got: {head}"
    );
    assert!(!head.starts_with('\n'), "no leading blank line: {head:?}");
}

#[test]
fn prompt_head_is_empty_when_there_is_nothing_to_say() {
    assert_eq!(compose_prompt_head("", None), "");
}

#[test]
fn prompt_head_combines_preamble_and_warning() {
    let head = compose_prompt_head("REBASE", Some("stale"));
    assert!(head.starts_with("REBASE"), "got: {head}");
    assert!(head.contains("Note: stale"), "got: {head}");
    assert!(head.ends_with("\n\n"), "separates from the body: {head:?}");
}

#[test]
fn fresh_worktree_with_no_origin_ref_gets_the_note_and_no_preamble() {
    // The decoupled case: nothing to rebase onto, but the agent must still be
    // told its base is local-only. An earlier design attached the warning to
    // the preamble, which silently dropped it on exactly this row.
    let sp = StartPoint::Local {
        base: "main".to_string(),
    };
    let preamble = select_preamble(None, Some(&sp), false);
    let head = compose_prompt_head(&preamble, Some("origin has no branch main"));

    assert!(
        preamble.is_empty(),
        "fresh worktree emits no preamble: {preamble:?}"
    );
    assert!(
        head.contains("Note: origin has no branch main"),
        "got: {head}"
    );
    assert!(
        !head.contains("git rebase"),
        "nothing to rebase onto: {head}"
    );
    assert!(
        !head.contains("reused from a previous attempt"),
        "got: {head}"
    );
}
