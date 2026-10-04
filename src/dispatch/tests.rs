use super::agents::prompt_launch_command;
use super::mock_sequence::{
    pr_view_reply, DispatchScript, FinishRun, PrHead, Step, COMPANION_PANE_ID,
};
use super::prompts::{
    allium_instruction, build_prompt, build_quick_dispatch_prompt, epic_preamble,
    reused_rebase_preamble, spec_first_instruction, task_block, tdd_instruction,
    wrap_up_instruction, EpicContext, LearningInjections, PromptContext,
};
use super::worktree::{
    provision_worktree, BaseRef, StartPoint, FETCH_MAX_ATTEMPTS, PROVISION_MAX_SUBPROCESS_CALLS,
};
use super::*;
use crate::models::test_tmux_window;
use crate::models::TaskBuilder;

use crate::models::{EpicId, Task, TaskId};
use crate::process::{AgentBinaries, MockProcessRunner, SUBPROCESS_TIMEOUT};
use crate::tmux;
use std::time::Duration;

// -----------------------------------------------------------------------
// Shared helper tests
// -----------------------------------------------------------------------

#[test]
fn task_block_contains_id_title_description() {
    let block = task_block(TaskId(5), "My title", "My description", None);
    assert!(block.contains("5"));
    assert!(block.contains("My title"));
    assert!(block.contains("My description"));
}

#[test]
fn task_block_includes_epic_section_when_present() {
    let ctx = EpicContext {
        epic_id: EpicId(3),
        epic_title: "Big Epic".to_string(),
        under_cve_feed: false,
    };
    let block = task_block(TaskId(1), "T", "D", Some(&ctx));
    assert!(block.contains("EpicId: 3"));
    assert!(block.contains("Big Epic"));
}

#[test]
fn tdd_instruction_mentions_tests_first() {
    let instr = tdd_instruction();
    assert!(instr.contains("tests first") || instr.contains("behaviour as tests"));
}

#[test]
fn spec_first_instruction_mentions_docs_plans_and_update_task() {
    let instr = spec_first_instruction();
    assert!(instr.contains("docs/plans/"));
    assert!(instr.contains("update_task"));
}

#[test]
fn allium_instruction_mentions_spec_and_skills() {
    let instr = allium_instruction();
    assert!(instr.contains("docs/specs/"));
    assert!(instr.contains("allium:tend"));
    assert!(instr.contains("allium:weed"));
}

pub(super) fn make_task(repo_path: &str) -> Task {
    TaskBuilder::new(42)
        .title("Fix bug")
        .description("A nasty crash")
        .repo_path(repo_path)
        .build()
}

/// A `git worktree remove` call, if the mock recorded one.
fn worktree_remove_call(calls: &[(String, Vec<String>)]) -> Option<&(String, Vec<String>)> {
    calls.iter().find(|(prog, args)| {
        prog == "git"
            && args.contains(&"worktree".to_string())
            && args.contains(&"remove".to_string())
    })
}

/// The `git worktree add` call, by verb rather than by position.
///
/// `git worktree prune` now runs immediately before it
/// (`StaleAdminRecordIsPrunedBeforeWorktreeAdd` in docs/specs/dispatch.allium),
/// so "the last git call" and "the first git call" no longer name the add.
/// Assertions about the start point read it from here instead, and stay true
/// the next time a call is inserted around it.
fn worktree_add_call(calls: &[(String, Vec<String>)]) -> &(String, Vec<String>) {
    calls
        .iter()
        .find(|(prog, args)| {
            prog == "git"
                && args.contains(&"worktree".to_string())
                && args.contains(&"add".to_string())
        })
        .expect("no git worktree add was recorded")
}

fn find_call_arg(calls: &[(String, Vec<String>)], call_idx: usize, pattern: &str) -> String {
    calls[call_idx]
        .1
        .iter()
        .find(|a| a.contains(pattern))
        .unwrap_or_else(|| panic!("call {call_idx} missing arg matching {pattern:?}"))
        .clone()
}

fn make_test_repo() -> (tempfile::TempDir, String) {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().to_str().unwrap().to_string();
    (dir, path)
}

/// A temp repo with `.worktrees/<slug>` already created, which is what puts
/// `provision_worktree` on its reuse branch.
///
/// `pub(crate)` rather than `pub(super)`: the service-layer dispatch-seam tests
/// need the same precondition, and a second copy of it would mean the day
/// `.worktrees/` moves, one of the two silently starts exercising the
/// fresh-worktree branch instead.
pub(crate) fn make_test_repo_with_worktree(
    slug: &str,
) -> (tempfile::TempDir, String, std::path::PathBuf) {
    let (dir, repo_path) = make_test_repo();
    let worktree_dir = dir.path().join(".worktrees").join(slug);
    std::fs::create_dir_all(&worktree_dir).unwrap();
    (dir, repo_path, worktree_dir)
}

/// Turn `<base>/.worktrees/<slug>` into a real LINKED worktree.
///
/// Re-exported from [`crate::worktree_admin::tests`], which is where the
/// function that READS this shape lives. One encoding of what git writes, in
/// the same module as the code that parses it, so the two cannot drift the day
/// the pointer's shape changes.
pub(crate) use crate::worktree_admin::tests::make_linked_worktree;

/// A `~/.claude.json` holding exactly the entry the startup configuration
/// check writes.
pub(crate) fn claude_json_with_dispatch_entry(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("claude.json");
    std::fs::write(
        &path,
        serde_json::to_string(&crate::setup::merge_mcp_config(None, crate::DEFAULT_PORT).value)
            .unwrap(),
    )
    .unwrap();
    path
}

#[test]
fn find_call_arg_returns_matching_arg() {
    let calls = vec![
        (
            "git".to_string(),
            vec!["worktree".to_string(), "add".to_string()],
        ),
        (
            "tmux".to_string(),
            vec!["new-window".to_string(), "-d".to_string()],
        ),
    ];
    let arg = find_call_arg(&calls, 1, "new-window");
    assert_eq!(arg, "new-window");
}

#[test]
#[should_panic(expected = "call 0 missing arg matching \"nonexistent\"")]
fn find_call_arg_panics_with_message_on_missing() {
    let calls = vec![("git".to_string(), vec!["status".to_string()])];
    find_call_arg(&calls, 0, "nonexistent");
}

#[test]
fn make_test_repo_returns_live_directory() {
    let (dir, repo_path) = make_test_repo();
    assert!(dir.path().exists());
    assert_eq!(repo_path, dir.path().to_str().unwrap());
}

#[test]
fn make_test_repo_with_worktree_creates_directory() {
    let (dir, _repo_path, worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    assert!(worktree_dir.exists());
    assert_eq!(
        worktree_dir,
        dir.path().join(".worktrees").join("42-fix-bug")
    );
}

#[test]
fn resolve_repo_path_matches_directory_name() {
    let paths = vec![
        "/home/user/projects/frontend".to_string(),
        "/home/user/projects/backend".to_string(),
    ];
    assert_eq!(
        resolve_repo_path("org/backend", &paths),
        Some("/home/user/projects/backend".to_string()),
    );
}

#[test]
fn resolve_repo_path_returns_none_when_no_match() {
    let paths = vec!["/home/user/projects/frontend".to_string()];
    assert_eq!(resolve_repo_path("org/backend", &paths), None);
}

#[test]
fn resolve_repo_path_handles_empty_paths() {
    assert_eq!(resolve_repo_path("org/repo", &[]), None);
}

#[test]
fn build_prompt_contains_task_info() {
    let prompt = build_prompt(
        TaskId(42),
        "Fix bug",
        "A nasty crash",
        None,
        None,
        &PromptContext::default(),
    );
    assert!(prompt.contains("42"));
    assert!(prompt.contains("Fix bug"));
    assert!(prompt.contains("A nasty crash"));
}

/// Every implementation prompt states test-first, but not in the same words:
/// the spec-first path states it as steps 3-4 ("confirm they fail before you
/// write any code"), and the with-plan and brainstorm paths state it as the
/// `tdd_instruction` line. Asserting the acronym would only pin the second.
#[test]
fn every_implementation_prompt_states_test_first() {
    let no_plan = build_prompt(
        TaskId(7),
        "Title",
        "Desc",
        None,
        None,
        &PromptContext::default(),
    );
    assert!(
        no_plan.contains("confirm they fail before you write any code"),
        "spec-first path must state test-first, got: {no_plan}"
    );

    let with_plan = build_prompt(
        TaskId(7),
        "Title",
        "Desc",
        Some("/tmp/p.md"),
        None,
        &PromptContext::default(),
    );
    assert!(
        with_plan.contains("behaviour as tests first"),
        "with-plan path must state test-first, got: {with_plan}"
    );
}

#[test]
fn build_prompt_mentions_wrap_up_skill() {
    let prompt = build_prompt(
        TaskId(7),
        "Title",
        "Desc",
        Some("docs/plans/p.md"),
        None,
        &PromptContext::default(),
    );
    assert!(
        prompt.contains("/wrap-up"),
        "with-plan prompt should tell agent to use /wrap-up skill"
    );
    assert!(
        prompt.contains("finalise the task"),
        "with-plan prompt should use the universal wrap-up wording"
    );
}

#[test]
fn build_prompt_without_plan_includes_wrap_up_universally() {
    // wrap_up_instruction is universal across every dispatched-agent prompt
    // — no-plan agents must implement the plan they attach before calling
    // /wrap-up, and need the same finalise step (commit/finalise) as any
    // other implementing agent.
    let prompt = build_prompt(
        TaskId(7),
        "Title",
        "Desc",
        None,
        None,
        &PromptContext::default(),
    );
    assert!(
        prompt.contains("/wrap-up"),
        "no-plan prompt should mention /wrap-up (universal, reached after implementing)"
    );
}

#[test]
fn build_prompt_without_plan_says_where_an_optional_plan_goes() {
    let prompt = build_prompt(
        TaskId(1),
        "Task",
        "Desc",
        None,
        None,
        &PromptContext::default(),
    );
    assert!(
        prompt.contains("docs/plans/"),
        "no-plan prompt should say where a plan goes if the agent writes one"
    );
    assert!(
        prompt.contains("update_task"),
        "no-plan prompt should say how to attach that plan via MCP"
    );
}

#[test]
fn build_prompt_without_plan_sends_the_agent_to_elicit_a_spec() {
    let prompt = build_prompt(
        TaskId(1),
        "Task",
        "Desc",
        None,
        None,
        &PromptContext::default(),
    );
    assert!(
        prompt.contains("allium:elicit"),
        "no-plan prompt should name the interview skill"
    );
    assert!(
        prompt.contains("docs/specs/"),
        "no-plan prompt should name the Allium spec as the design artefact"
    );
}

/// The design step no longer *requires* a plan doc — it is offered as an
/// option, conditional on the size of the implementation.
#[test]
fn build_prompt_without_plan_makes_the_plan_doc_optional() {
    let prompt = build_prompt(
        TaskId(1),
        "Task",
        "Desc",
        None,
        None,
        &PromptContext::default(),
    );
    assert!(
        prompt.contains("only if the implementation is big enough"),
        "no-plan prompt should condition the plan doc on implementation size"
    );
    assert!(
        !prompt.contains("implementation plan directly"),
        "no-plan prompt should no longer instruct writing a plan as the design output"
    );
}

#[test]
fn build_prompt_with_plan_asks_permission_before_implementing() {
    let prompt = build_prompt(
        TaskId(1),
        "Task",
        "Desc",
        Some("docs/plans/plan.md"),
        None,
        &PromptContext::default(),
    );
    assert!(prompt.contains("docs/plans/plan.md"));
    assert!(
        prompt.contains("ask the user to confirm") && prompt.contains("Make no changes"),
        "with-plan prompt should ask for confirmation and gate changes on it"
    );
    assert!(
        !prompt.contains("step by step"),
        "with-plan prompt should not say 'Follow it step by step' — agent reviews first"
    );
}

/// The prose tool notice's absence is checked for every prompt by
/// `SHARED_ABSENT_LINES`. What this covers is the routing the tool schema
/// cannot carry, which must survive that removal.
///
/// Quick dispatch's rename is the one surviving case, and the only example
/// `ThePromptNamesNoToolMerelyToSayItExists` still gives: nothing in
/// `update_task`'s schema can imply that *this* task arrived with a
/// placeholder title and needs renaming off it before anything else.
///
/// It used to assert `query_learnings` in the no-plan prompt instead, on the
/// guarantee's second example — that only the prompt said WHEN to call it.
/// That premise did not survive `query_learnings`'s own description gaining
/// "Call it when something is unclear, before guessing or asking", so the
/// nudge stopped naming any tool and this test moved to the case that is
/// still real. The trailing block's freedom from tool names is now pinned
/// from the other side by `prompt_trailing_lines_name_no_mcp_tool`.
#[test]
fn build_prompt_names_the_tools_it_actually_needs_the_agent_to_call() {
    let prompt =
        build_quick_dispatch_prompt(TaskId(1), "Quick task", "", None, &PromptContext::default());
    assert!(
        prompt.contains("call `update_task` with a descriptive `title`"),
        "quick dispatch must name the specific rename call its schema cannot \
imply, got: {prompt}"
    );
}

#[test]
fn validate_repo_path_existing_dir() {
    assert!(validate_repo_path("/tmp").is_ok());
}

#[test]
fn validate_repo_path_nonexistent() {
    let result = validate_repo_path("/nonexistent/path");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("does not exist"));
}

#[test]
fn validate_repo_path_not_a_dir() {
    let result = validate_repo_path("/etc/hostname");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Not a directory"));
}

mod agent_launch;
mod companion_pane;
mod finish_and_guards;
mod pr_review;
mod pr_status;
mod process_runner;

// Helpers other test files (here and in `mock_sequence`) reach through this module.
pub(in crate::dispatch) use finish_and_guards::run_finish;
pub(in crate::dispatch) use pr_review::pr_review_task;
