use super::*;

// --- check_pr_status error-path tests ---

#[test]
fn check_pr_status_unknown_state_returns_error() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        b"FUNKY_NEW_STATE\n",
    )]);
    let result = check_pr_status("https://github.com/org/repo/pull/42", &mock);
    assert!(result.is_err(), "unknown PR state should return an error");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("FUNKY_NEW_STATE"),
        "error should include the unknown state, got: {msg}"
    );
}

#[test]
fn check_pr_status_empty_output_returns_error() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"")]);
    let result = check_pr_status("https://github.com/org/repo/pull/42", &mock);
    assert!(
        result.is_err(),
        "empty output from gh pr view should return an error"
    );
}

// --- check_pr_status failure classification ---
//
// Every stderr string below is a verbatim `gh` failure taken from a real
// app.log, which is the only reason to trust a string-matching classifier at
// all: gh offers no machine-readable failure kind, so the spec's permanent /
// transient split (core.allium: PrCheckOutcome) is stated in terms of these
// messages.

/// Table-driven so adding a newly-observed gh failure is one row, and so the
/// permanent and transient cases cannot drift apart in how they are asserted.
#[test]
fn check_pr_status_classifies_gh_failures() {
    let permanent = [
        "GraphQL: Could not resolve to a Repository with the name 'annotell/storage-api-scala'. (repository)",
        "HTTP 401: Bad credentials (https://api.github.com/graphql)",
        "HTTP 401: Requires authentication (https://api.github.com/graphql)",
        "no pull requests found for branch \"https://github.com/annotell/airflow-dags/issues/12\"",
    ];
    for stderr in permanent {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::fail(stderr)]);
        let failure = check_pr_status("https://github.com/org/repo/pull/42", &mock)
            .expect_err("gh failed, so this must be an error");
        assert!(
            failure.is_permanent(),
            "should classify as permanent: {stderr}"
        );
    }

    let transient = [
        "error connecting to api.github.com",
        "Post \"https://api.github.com/graphql\": net/http: TLS handshake timeout",
        "Post \"https://api.github.com/graphql\": dial tcp 140.82.121.5:443: i/o timeout",
        "HTTP 504: 504 Gateway Timeout (https://api.github.com/graphql)",
        "HTTP 502: 502 Bad Gateway (https://api.github.com/graphql)",
        "GraphQL: API rate limit already exceeded for user ID 1234.",
        "stream error: stream ID 5; CANCEL; received from peer",
        "Post \"https://api.github.com/graphql\": unexpected EOF",
    ];
    for stderr in transient {
        let mock = MockProcessRunner::new(vec![MockProcessRunner::fail(stderr)]);
        let failure = check_pr_status("https://github.com/org/repo/pull/42", &mock)
            .expect_err("gh failed, so this must be an error");
        assert!(
            !failure.is_permanent(),
            "should classify as transient: {stderr}"
        );
    }
}

/// A failure nobody has catalogued yet must retry rather than strand the task.
/// Getting this backwards is the worse error: a misclassified transient failure
/// permanently marks a task pr_unreachable, while a misclassified permanent one
/// costs only a slowing trickle of doomed calls.
#[test]
fn check_pr_status_classifies_an_unrecognised_failure_as_transient() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail(
        "gh: something nobody has seen before",
    )]);
    let failure = check_pr_status("https://github.com/org/repo/pull/42", &mock)
        .expect_err("gh failed, so this must be an error");
    assert!(
        !failure.is_permanent(),
        "an unknown failure must default to transient"
    );
}

/// A state gh reports that dispatch cannot map is permanent: retrying returns
/// the same unmappable answer, and the underlying cause is a code change.
#[test]
fn check_pr_status_classifies_an_unknown_pr_state_as_permanent() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        b"FUNKY_NEW_STATE\n",
    )]);
    let failure = check_pr_status("https://github.com/org/repo/pull/42", &mock)
        .expect_err("an unmappable state must be an error");
    assert!(failure.is_permanent(), "unknown state should be permanent");
}

// --- pr_head_branch tests ---

#[test]
fn pr_head_branch_returns_head_ref_for_same_repo_pr() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(&pr_view_reply(
        "renovate/serde-1.x",
        false,
    ))]);
    let branch = pr_head_branch("https://github.com/org/repo/pull/7", &mock);
    assert_eq!(branch.as_deref(), Some("renovate/serde-1.x"));

    let calls = mock.recorded_calls();
    assert_eq!(calls[0].0, "gh");
    assert!(calls[0].1.contains(&"pr".to_string()));
    assert!(calls[0].1.contains(&"view".to_string()));
    assert!(
        calls[0]
            .1
            .contains(&"headRefName,isCrossRepository".to_string()),
        "should request headRefName and isCrossRepository, got: {:?}",
        calls[0].1
    );
}

#[test]
fn pr_head_branch_returns_none_for_fork_pr() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(&pr_view_reply(
        "patch-1", true,
    ))]);
    assert_eq!(
        pr_head_branch("https://github.com/org/repo/pull/7", &mock),
        None,
        "fork (isCrossRepository=true) PR should fall back to base branch"
    );
}

#[test]
fn pr_head_branch_returns_none_on_command_failure() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("gh: not authenticated")]);
    assert_eq!(
        pr_head_branch("https://github.com/org/repo/pull/7", &mock),
        None
    );
}

#[test]
fn pr_head_branch_returns_none_on_empty_output() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"")]);
    assert_eq!(
        pr_head_branch("https://github.com/org/repo/pull/7", &mock),
        None
    );
}

// A hung `gh` must not park the calling thread forever (#4202).
#[test]
fn pr_head_branch_is_bounded_by_subprocess_timeout() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(&pr_view_reply(
        "renovate/serde-1.x",
        false,
    ))]);
    pr_head_branch("https://github.com/org/repo/pull/7", &mock);
    assert_eq!(mock.recorded_timeouts(), vec![Some(SUBPROCESS_TIMEOUT)]);
}
