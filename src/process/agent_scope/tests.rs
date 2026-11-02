use super::*;
use crate::process::MockProcessRunner;

const SHOW: [&str; 6] = ["--user", "show", AGENT_SLICE, "-p", "MemoryMax", "--value"];

const SET: [&str; 7] = [
    "--user",
    "set-property",
    "--runtime",
    AGENT_SLICE,
    "MemoryHigh=80%",
    "MemoryMax=85%",
    "MemorySwapMax=2G",
];

fn recorded(mock: &MockProcessRunner) -> Vec<(String, Vec<String>)> {
    mock.recorded_calls()
}

fn call(args: &[&str]) -> (String, Vec<String>) {
    (
        "systemctl".to_string(),
        args.iter().map(|a| (*a).to_string()).collect(),
    )
}

#[test]
fn an_uncapped_slice_gets_the_default_cap() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"infinity\n"),
        MockProcessRunner::ok(),
    ]);

    let scope = prepare_agent_scope(&mock);

    assert_eq!(scope, Some(AgentScope::new(AGENT_SLICE)));
    assert_eq!(recorded(&mock), vec![call(&SHOW), call(&SET)]);
}

#[test]
fn a_capped_slice_keeps_its_cap() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"30000000000\n")]);

    let scope = prepare_agent_scope(&mock);

    assert_eq!(scope, Some(AgentScope::new(AGENT_SLICE)));
    assert_eq!(recorded(&mock), vec![call(&SHOW)]);
}

#[test]
fn an_unreachable_service_manager_launches_unwrapped() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail(
        "Failed to connect to bus: No medium found",
    )]);

    assert_eq!(prepare_agent_scope(&mock), None);
}

#[test]
fn a_missing_systemctl_launches_unwrapped() {
    let mock = MockProcessRunner::new(vec![Err(anyhow::anyhow!("failed to run systemctl"))]);

    assert_eq!(prepare_agent_scope(&mock), None);
}

#[test]
fn a_failed_cap_write_launches_unwrapped() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"infinity\n"),
        MockProcessRunner::fail("Access denied"),
    ]);

    assert_eq!(prepare_agent_scope(&mock), None);
}

#[test]
fn wrap_runs_the_command_in_a_scope_that_survives_an_oom_kill() {
    let scope = AgentScope::new("dispatch-agents.slice");

    assert_eq!(
        scope.wrap("bash -c 'x' claude"),
        "systemd-run --user --scope --quiet --collect --slice=dispatch-agents.slice \
         -p OOMPolicy=continue -- bash -c 'x' claude"
    );
}

#[test]
fn mock_runner_reports_no_agent_scope_by_default() {
    assert_eq!(MockProcessRunner::new(vec![]).agent_scope(), None);
}

#[test]
fn mock_runner_reports_no_board_cargo_build_jobs_by_default() {
    assert_eq!(
        MockProcessRunner::new(vec![]).board_cargo_build_jobs(),
        None
    );
}
