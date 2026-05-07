use super::prompts::{
    allium_instruction, build_epic_planning_prompt, build_fix_task_prompt, build_prompt,
    build_quick_dispatch_prompt, build_tmux_window_name, epic_preamble, learning_tools_instruction,
    mcp_tools_instruction, plan_and_attach_instruction, rebase_preamble, task_block, tdd_instruction,
    wrap_up_instruction, EpicContext, ProjectContext,
};
use super::worktree::provision_worktree;
use super::*;

use crate::models::{AlertKind, EpicId, ProjectId, Task, TaskId, TaskStatus};
use crate::process::{exit_fail, MockProcessRunner};
use crate::tui::FixAgentRequest;
use chrono::Utc;
use std::process::Output;

// -----------------------------------------------------------------------
// Shared helper tests
// -----------------------------------------------------------------------

#[test]
fn task_block_contains_id_title_description() {
    let block = task_block(TaskId(5), "My title", "My description", None, None);
    assert!(block.contains("5"));
    assert!(block.contains("My title"));
    assert!(block.contains("My description"));
}

#[test]
fn task_block_includes_epic_section_when_present() {
    let ctx = EpicContext {
        epic_id: EpicId(3),
        epic_title: "Big Epic".to_string(),
    };
    let block = task_block(TaskId(1), "T", "D", Some(&ctx), None);
    assert!(block.contains("EpicId: 3"));
    assert!(block.contains("Big Epic"));
}

#[test]
fn task_block_includes_project_section_when_present() {
    let ctx = ProjectContext {
        project_id: ProjectId(42),
        project_name: "My Project".to_string(),
    };
    let block = task_block(TaskId(1), "T", "D", None, Some(&ctx));
    assert!(block.contains("ProjectId: 42"), "block was: {block}");
    assert!(block.contains("My Project"), "block was: {block}");
    assert!(
        block.contains("project_id=42"),
        "should tell agent to pass project_id=42"
    );
}

#[test]
fn build_prompt_includes_project_context() {
    let ctx = ProjectContext {
        project_id: ProjectId(7),
        project_name: "Dispatch".to_string(),
    };
    let prompt = build_prompt(TaskId(1), "Task", "Desc", None, None, Some(&ctx));
    assert!(prompt.contains("ProjectId: 7"));
    assert!(prompt.contains("Dispatch"));
}

#[test]
fn learning_tools_instruction_includes_lint_checkpoint() {
    let instr = learning_tools_instruction();
    assert!(
        instr.contains("/lint"),
        "learning_tools_instruction must include the /lint checkpoint, got:\n{instr}"
    );
}

#[test]
fn fix_task_prompt_includes_lint_checkpoint() {
    let prompt = build_fix_task_prompt(TaskId(5), "Fix CVE", "desc", None, None);
    assert!(
        prompt.contains("/lint"),
        "build_fix_task_prompt must include /lint (via learning_tools_instruction), got:\n{prompt}"
    );
}

#[test]
fn tdd_instruction_mentions_tests_first() {
    let instr = tdd_instruction();
    assert!(instr.contains("tests first") || instr.contains("behaviour as tests"));
}

#[test]
fn mcp_tools_instruction_mentions_get_and_update() {
    let instr = mcp_tools_instruction();
    assert!(instr.contains("get_task"));
    assert!(instr.contains("update_task"));
}

#[test]
fn plan_and_attach_instruction_mentions_docs_plans_and_update_task() {
    let instr = plan_and_attach_instruction();
    assert!(instr.contains("docs/plans/"));
    assert!(instr.contains("update_task"));
}

#[test]
fn wrap_up_instruction_mentions_wrap_up_skill() {
    let instr = wrap_up_instruction();
    assert!(instr.contains("/wrap-up"));
}

#[test]
fn allium_instruction_mentions_spec_and_skills() {
    let instr = allium_instruction();
    assert!(instr.contains("docs/specs/"));
    assert!(instr.contains("allium:tend"));
    assert!(instr.contains("allium:weed"));
}

fn make_task(repo_path: &str) -> Task {
    Task {
        id: TaskId(42),
        title: "Fix bug".to_string(),
        description: "A nasty crash".to_string(),
        repo_path: repo_path.to_string(),
        status: TaskStatus::Backlog,
        worktree: None,
        tmux_window: None,
        plan_path: None,
        epic_id: None,
        sub_status: crate::models::SubStatus::None,
        pr_url: None,
        tag: None,
        sort_order: None,
        base_branch: "main".to_string(),
        external_id: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        project_id: ProjectId(1),
    }
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

fn make_test_repo_with_worktree(slug: &str) -> (tempfile::TempDir, String, std::path::PathBuf) {
    let (dir, repo_path) = make_test_repo();
    let worktree_dir = dir.path().join(".worktrees").join(slug);
    std::fs::create_dir_all(&worktree_dir).unwrap();
    (dir, repo_path, worktree_dir)
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
    let prompt = build_prompt(TaskId(42), "Fix bug", "A nasty crash", None, None, None);
    assert!(prompt.contains("42"));
    assert!(prompt.contains("Fix bug"));
    assert!(prompt.contains("A nasty crash"));
    assert!(prompt.contains("TDD"));
}

#[test]
fn build_prompt_mentions_tdd() {
    let prompt = build_prompt(TaskId(7), "Title", "Desc", None, None, None);
    assert!(prompt.contains("TDD"));
    assert!(prompt.contains("behaviour as tests first"));
}

#[test]
fn build_prompt_mentions_wrap_up_skill() {
    let prompt = build_prompt(
        TaskId(7),
        "Title",
        "Desc",
        Some("docs/plans/p.md"),
        None,
        None,
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
    // — no-plan agents may end by attaching a plan and need the same finalise
    // step (commit/finalise) as implementing agents.
    let prompt = build_prompt(TaskId(7), "Title", "Desc", None, None, None);
    assert!(
        prompt.contains("/wrap-up"),
        "no-plan prompt should mention /wrap-up (universal, covers plan-attach finish)"
    );
}

#[test]
fn build_prompt_without_plan_includes_planning_instruction() {
    let prompt = build_prompt(TaskId(1), "Task", "Desc", None, None, None);
    assert!(
        prompt.contains("docs/plans/"),
        "no-plan prompt should instruct agent to write a plan"
    );
    assert!(
        prompt.contains("update_task"),
        "no-plan prompt should instruct agent to attach plan via MCP"
    );
    assert!(
        prompt.contains("ask") || prompt.contains("permission") || prompt.contains("proceed"),
        "no-plan prompt should ask for permission before implementing"
    );
}

#[test]
fn build_prompt_without_plan_mentions_brainstorm_if_vague() {
    let prompt = build_prompt(TaskId(1), "Task", "Desc", None, None, None);
    assert!(
        prompt.contains("/brainstorming"),
        "no-plan prompt should mention /brainstorming for vague descriptions"
    );
    assert!(
        prompt.contains("vague"),
        "no-plan prompt should mention vagueness as the condition for brainstorming"
    );
}

#[test]
fn build_prompt_without_plan_mentions_direct_plan_alternative() {
    let prompt = build_prompt(TaskId(1), "Task", "Desc", None, None, None);
    assert!(
        prompt.contains("implementation plan directly"),
        "no-plan prompt should offer writing a plan directly for clear descriptions"
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
        None,
    );
    assert!(prompt.contains("docs/plans/plan.md"));
    assert!(
        prompt.contains("Shall I proceed")
            || prompt.contains("permission")
            || prompt.contains("proceed"),
        "with-plan prompt should ask for permission before implementing"
    );
    assert!(
        !prompt.contains("step by step"),
        "with-plan prompt should not say 'Follow it step by step' — agent reviews first"
    );
}

#[test]
fn build_prompt_mentions_mcp_tools() {
    let prompt = build_prompt(TaskId(1), "Task", "Desc", None, None, None);
    assert!(
        prompt.contains("dispatch MCP tools"),
        "standard dispatch prompt should mention MCP tools"
    );
}

#[test]
fn is_wrappable_running_with_worktree() {
    let task = Task {
        status: TaskStatus::Running,
        worktree: Some("/tmp/wt".to_string()),
        ..make_task("/repo")
    };
    assert!(is_wrappable(&task));
}

#[test]
fn is_wrappable_review_with_worktree() {
    let task = Task {
        status: TaskStatus::Review,
        worktree: Some("/tmp/wt".to_string()),
        ..make_task("/repo")
    };
    assert!(is_wrappable(&task));
}

#[test]
fn is_wrappable_running_without_worktree() {
    let task = Task {
        status: TaskStatus::Running,
        worktree: None,
        ..make_task("/repo")
    };
    assert!(!is_wrappable(&task));
}

#[test]
fn is_wrappable_backlog_with_worktree() {
    let task = Task {
        status: TaskStatus::Backlog,
        worktree: Some("/tmp/wt".to_string()),
        ..make_task("/repo")
    };
    assert!(!is_wrappable(&task));
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

#[test]
fn resume_window_name_matches_dispatch() {
    // The resume window name should use the same naming convention as dispatch
    assert_eq!(build_tmux_window_name(TaskId(42)), "task-42");
}

#[test]
fn build_prompt_includes_plan_path() {
    let prompt = build_prompt(
        TaskId(1),
        "Task",
        "Desc",
        Some("docs/plans/my-plan.md"),
        None,
        None,
    );
    assert!(prompt.contains("Plan: docs/plans/my-plan.md"));
}

#[test]
fn build_prompt_without_plan_omits_plan_section() {
    let prompt = build_prompt(TaskId(1), "Task", "Desc", None, None, None);
    assert!(!prompt.contains("Plan:"));
}

#[test]
fn build_quick_dispatch_prompt_includes_planning_instruction() {
    let prompt = build_quick_dispatch_prompt(TaskId(42), "Quick task", "", None, None);
    assert!(
        prompt.contains("docs/plans/") || prompt.contains("plan"),
        "quick dispatch prompt should instruct agent to write a plan before implementing"
    );
    assert!(
        prompt.contains("ask") || prompt.contains("permission") || prompt.contains("proceed"),
        "quick dispatch prompt should ask for permission before implementing"
    );
}

#[test]
fn build_quick_dispatch_prompt_contains_rename_instruction() {
    let prompt = build_quick_dispatch_prompt(TaskId(42), "Quick task", "", None, None);
    assert!(prompt.contains("42"));
    assert!(prompt.contains("Quick task"));
    assert!(prompt.contains("update_task"));
    assert!(prompt.contains("title"));
    assert!(prompt.contains("placeholder"));
}

#[test]
fn build_quick_dispatch_prompt_mentions_mcp() {
    let prompt = build_quick_dispatch_prompt(TaskId(1), "Quick task", "", None, None);
    assert!(prompt.contains("dispatch MCP tools"));
    assert!(prompt.contains("update_task"));
    assert!(!prompt.contains("add_note"));
}

#[test]
fn build_quick_dispatch_prompt_differs_from_regular() {
    let regular = build_prompt(TaskId(1), "Task", "Desc", None, None, None);
    let quick = build_quick_dispatch_prompt(TaskId(1), "Task", "Desc", None, None);
    assert!(quick.contains("placeholder"));
    assert!(!regular.contains("placeholder"));
}

#[test]
fn build_quick_dispatch_prompt_includes_epic_context() {
    let ctx = EpicContext {
        epic_id: EpicId(7),
        epic_title: "My Epic".to_string(),
    };
    let prompt = build_quick_dispatch_prompt(TaskId(42), "Quick task", "", Some(&ctx), None);
    assert!(prompt.contains("EpicId: 7"), "should include epic ID");
    assert!(prompt.contains("My Epic"), "should include epic title");
    assert!(
        prompt.contains("send_message"),
        "should tell agent how to message sibling agents"
    );
}

#[test]
fn rebase_preamble_prepended_to_all_prompts() {
    let body = build_prompt(TaskId(1), "Task", "Desc", None, None, None);
    let full = format!(
        "{}\n\n\
         Always work from this worktree folder — do not `cd` to the parent repo \
         or other directories.\n\n\
         {body}",
        rebase_preamble("origin/main")
    );
    assert!(full.contains("rebase your branch from origin/main"));
    assert!(full.starts_with("Before starting work"));
    assert!(full.contains("Always work from this worktree folder"));
}

#[test]
fn no_plan_prompts_reference_brainstorming_skill() {
    let standard = build_prompt(TaskId(1), "T", "D", None, None, None);
    let quick = build_quick_dispatch_prompt(TaskId(1), "T", "D", None, None);

    for (name, prompt) in [("standard-no-plan", standard), ("quick", quick)] {
        assert!(
            prompt.contains("/brainstorming"),
            "{name} prompt should reference /brainstorming skill"
        );
    }
}

const SHARED_TRAILING_LINES: &[&str] = &[
    "TDD",                           // tdd_instruction
    "Allium specs in `docs/specs/`", // allium_instruction
    "dispatch MCP tools",            // mcp_tools_instruction
    "/wrap-up",                      // wrap_up_instruction (universal)
];

fn project_ctx() -> ProjectContext {
    ProjectContext {
        project_id: ProjectId(1),
        project_name: "Default".to_string(),
    }
}

fn epic_ctx() -> EpicContext {
    EpicContext {
        epic_id: EpicId(7),
        epic_title: "My Epic".to_string(),
    }
}

fn all_aligned_prompts() -> [(&'static str, String); 4] {
    let project = project_ctx();
    let epic = epic_ctx();
    [
        (
            "standard-no-plan",
            build_prompt(TaskId(1), "Task", "Desc", None, None, None),
        ),
        (
            "standard-with-plan",
            build_prompt(
                TaskId(1),
                "Task",
                "Desc",
                Some("docs/plans/p.md"),
                None,
                None,
            ),
        ),
        (
            "quick-dispatch",
            build_quick_dispatch_prompt(TaskId(1), "Quick task", "", None, None),
        ),
        (
            "epic-planning",
            build_epic_planning_prompt(
                TaskId(42),
                "Plan: My Epic",
                "Planning subtask for epic",
                &epic,
                &project,
            ),
        ),
    ]
}

#[test]
fn every_prompt_includes_shared_trailing_metadata() {
    for (name, prompt) in all_aligned_prompts() {
        for needle in SHARED_TRAILING_LINES {
            assert!(
                prompt.contains(needle),
                "{name} prompt missing shared trailing line: {needle}\n--- prompt ---\n{prompt}"
            );
        }
    }
}

#[test]
fn every_prompt_uses_task_block_format() {
    for (name, prompt) in all_aligned_prompts() {
        assert!(
            prompt.contains("Task:"),
            "{name} prompt should open task block with `Task:` (no `Epic:` header)\n{prompt}"
        );
        assert!(prompt.contains("ID:"), "{name} prompt should have `ID:`");
        assert!(
            prompt.contains("Title:"),
            "{name} prompt should have `Title:`"
        );
        assert!(
            prompt.contains("Description:"),
            "{name} prompt should have `Description:`"
        );
    }
}

#[test]
fn epic_planning_prompt_uses_task_block_not_epic_header() {
    let project = project_ctx();
    let epic = epic_ctx();
    let prompt = build_epic_planning_prompt(
        TaskId(42),
        "Plan: My Epic",
        "Planning subtask",
        &epic,
        &project,
    );
    assert!(
        prompt.starts_with("You are planning an epic."),
        "epic-planning prompt should open with the planning preamble, got: {}",
        prompt.lines().next().unwrap_or("(empty)")
    );
    assert!(
        prompt.contains("Task:"),
        "epic-planning should reuse task_block (Task: header), not custom Epic: header"
    );
    assert!(
        !prompt.contains("\nEpic:\n  ID:"),
        "epic-planning must not use the legacy `Epic:` header in the task block"
    );
    assert!(
        prompt.contains("EpicId: 7"),
        "epic-planning should surface the epic id via the task_block EpicId line"
    );
    assert!(
        prompt.contains("ID: 42"),
        "epic-planning should use the planning subtask's real id, not a placeholder"
    );
}

#[test]
fn epic_planning_prompt_includes_work_package_steps_and_no_implement_guard() {
    let project = project_ctx();
    let epic = epic_ctx();
    let prompt = build_epic_planning_prompt(TaskId(1), "Plan", "Desc", &epic, &project);
    assert!(prompt.contains("create_task"), "should mention create_task");
    assert!(prompt.contains("update_epic"), "should mention update_epic");
    assert!(prompt.contains("sort_order"), "should mention sort_order");
    assert!(prompt.contains("repo_path"), "should mention repo_path");
    assert!(
        prompt.contains("Do NOT") || prompt.contains("do not start implementing"),
        "epic-planning should keep the do-not-implement guard"
    );
}

#[test]
fn quick_dispatch_uses_unconditional_plan_and_attach_instruction() {
    let prompt = build_quick_dispatch_prompt(TaskId(1), "Quick task", "", None, None);
    assert!(
        prompt.contains(plan_and_attach_instruction()),
        "quick-dispatch prompt should embed plan_and_attach_instruction verbatim"
    );
    assert!(
        !prompt.contains("vague or"),
        "quick-dispatch must not use the conditional plan_or_brainstorm wording"
    );
}

#[test]
fn wrap_up_instruction_universal_wording() {
    let text = wrap_up_instruction();
    assert!(
        text.contains("/wrap-up"),
        "wrap_up_instruction should reference the /wrap-up skill"
    );
    assert!(
        text.contains("attaching a plan")
            || text.contains("creating work packages")
            || text.contains("your work is done"),
        "wrap_up_instruction should describe the universal trigger (plan attach / work-packages / impl), got: {text}"
    );
}

#[test]
fn plan_and_attach_instruction_is_concise() {
    let instruction = plan_and_attach_instruction();
    assert!(
        instruction.len() < 200,
        "plan_and_attach_instruction should be concise (< 200 chars), got {} chars",
        instruction.len()
    );
    assert!(instruction.contains("/brainstorming"));
    assert!(instruction.contains("update_task"));
    assert!(instruction.contains("docs/plans/"));
}

#[test]
fn epic_preamble_returns_empty_strings_for_none() {
    let (id_line, section) = epic_preamble(None);
    assert!(id_line.is_empty());
    assert!(section.is_empty());
}

#[test]
fn epic_preamble_returns_id_line_and_section_for_some() {
    let ctx = EpicContext {
        epic_id: EpicId(5),
        epic_title: "Auth Rework".to_string(),
    };
    let (id_line, section) = epic_preamble(Some(&ctx));
    assert!(id_line.contains("EpicId: 5"));
    assert!(section.contains("Auth Rework"));
    assert!(
        section.contains("send_message"),
        "should guide agent to use send_message"
    );
    assert!(
        !section.contains("Sibling tasks:"),
        "should not enumerate sibling tasks"
    );
}

// --- ProcessRunner-based tests ---

#[test]
fn dispatch_reuses_existing_worktree() {
    // Pre-create worktree dir — simulates a re-dispatch where the worktree
    // already exists on disk from a previous dispatch cycle.
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let mock = MockProcessRunner::new(vec![
        // No detect_default_branch call — task.base_branch is used directly
        // git worktree add is skipped (dir exists)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
        MockProcessRunner::ok(), // tmux send-keys -l
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]);

    let task = make_task(&repo_path);
    dispatch_agent(&task, &mock, None, None).unwrap();

    let calls = mock.recorded_calls();
    assert!(
        calls
            .iter()
            .all(|(prog, args)| !(prog == "git" && args.iter().any(|a| a == "worktree"))),
        "git worktree add should be skipped for existing worktree"
    );
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(calls[0].1[0], "new-window");
    assert_eq!(calls[1].0, "tmux");
    assert_eq!(calls[1].1[0], "set-option");
    assert_eq!(calls[2].0, "tmux");
    assert_eq!(calls[2].1[0], "set-hook");
}

#[test]
fn dispatch_sends_claude_command() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let mock = MockProcessRunner::new(vec![
        // No detect_default_branch call — task.base_branch is used directly
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
        MockProcessRunner::ok(), // tmux send-keys -l (the claude command)
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]);

    let task = make_task(&repo_path);
    dispatch_agent(&task, &mock, None, None).unwrap();

    let calls = mock.recorded_calls();
    // The literal send-keys call (index 3) carries the claude invocation
    assert!(
        calls[3].1.iter().any(|a| a.contains("claude")),
        "send-keys should include claude"
    );
    assert!(
        calls[3]
            .1
            .iter()
            .any(|a| a.contains("--permission-mode plan")),
        "dispatch_agent send-keys should use plan mode"
    );
}

#[test]
fn dispatch_agent_uses_plan_mode() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let mock = MockProcessRunner::new(vec![
        // No detect_default_branch call — task.base_branch is used directly
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]);

    let task = make_task(&repo_path);
    dispatch_agent(&task, &mock, None, None).unwrap();

    let calls = mock.recorded_calls();
    let send_keys_arg = find_call_arg(&calls, 3, "claude");
    assert!(
        send_keys_arg.contains("--permission-mode plan"),
        "dispatch_agent should use plan mode, got: {send_keys_arg}"
    );
}

#[test]
fn provision_worktree_creates_new_when_dir_missing() {
    let (_dir, repo_path) = make_test_repo();
    // Do NOT pre-create the worktree dir — test the "create" path

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // git worktree add
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
    ]);

    let task = make_task(&repo_path);
    let result = provision_worktree(&task, &mock, None).unwrap();

    let calls = mock.recorded_calls();
    assert_eq!(calls[0].0, "git", "first call should be git worktree add");
    assert!(calls[0].1.contains(&"worktree".to_string()));
    assert!(calls[0].1.contains(&"add".to_string()));
    assert_eq!(calls[1].0, "tmux");
    assert_eq!(calls[1].1[0], "new-window");

    let expected_path = format!("{repo_path}/.worktrees/42-fix-bug");
    assert_eq!(result.worktree_path, expected_path);
}

#[test]
fn provision_worktree_skips_git_when_dir_exists() {
    let (_dir, repo_path, worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
    ]);

    let task = make_task(&repo_path);
    let result = provision_worktree(&task, &mock, None).unwrap();

    let calls = mock.recorded_calls();
    assert!(
        calls.iter().all(|(prog, _)| prog != "git"),
        "git should be skipped"
    );
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(calls[0].1[0], "new-window");
    assert_eq!(result.worktree_path, worktree_dir.to_str().unwrap());
}

#[test]
fn provision_worktree_with_base_branch_passes_start_point() {
    let (_dir, repo_path) = make_test_repo();

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // git fetch origin 99-prev-task
        MockProcessRunner::ok(), // git worktree add (with origin/99-prev-task)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
    ]);

    let task = make_task(&repo_path);
    let result = provision_worktree(&task, &mock, Some("99-prev-task")).unwrap();

    let calls = mock.recorded_calls();
    // call[0] = fetch
    assert_eq!(calls[0].0, "git");
    assert!(calls[0].1.contains(&"fetch".to_string()));
    assert!(calls[0].1.contains(&"99-prev-task".to_string()));
    // call[1] = worktree add — start point is now origin/<base>
    assert_eq!(calls[1].0, "git");
    let git_args = &calls[1].1;
    assert_eq!(
        git_args.last().unwrap(),
        "origin/99-prev-task",
        "base branch should be origin/99-prev-task as last git arg, got: {git_args:?}"
    );

    let expected_path = format!("{repo_path}/.worktrees/42-fix-bug");
    assert_eq!(result.worktree_path, expected_path);
}

#[test]
fn provision_worktree_fetches_origin_before_create() {
    // Fetch succeeds → worktree add should use origin/<base> as start point
    let (_dir, repo_path) = make_test_repo();

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // git fetch origin main
        MockProcessRunner::ok(), // git worktree add
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
    ]);

    let task = make_task(&repo_path);
    provision_worktree(&task, &mock, Some("main")).unwrap();

    let calls = mock.recorded_calls();
    // call[0] = git fetch origin main
    assert_eq!(calls[0].0, "git");
    assert!(
        calls[0].1.contains(&"fetch".to_string()),
        "expected fetch, got: {:?}",
        calls[0].1
    );
    assert!(calls[0].1.contains(&"origin".to_string()));
    assert!(calls[0].1.contains(&"main".to_string()));
    // call[1] = git worktree add ... origin/main
    assert_eq!(calls[1].0, "git");
    assert!(calls[1].1.contains(&"worktree".to_string()));
    assert_eq!(
        calls[1].1.last().unwrap(),
        "origin/main",
        "worktree add should use origin/main as start point, got: {:?}",
        calls[1].1
    );
}

#[test]
fn provision_worktree_fetch_failure_falls_back_to_local() {
    // Fetch fails → worktree add should use local branch (no error propagated)
    let (_dir, repo_path) = make_test_repo();

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::fail("fatal: 'origin' does not appear to be a git repository"),
        MockProcessRunner::ok(), // git worktree add
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
    ]);

    let task = make_task(&repo_path);
    // Should NOT return an error — soft fail
    provision_worktree(&task, &mock, Some("main")).unwrap();

    let calls = mock.recorded_calls();
    // call[0] = fetch (failed)
    assert_eq!(calls[0].0, "git");
    assert!(calls[0].1.contains(&"fetch".to_string()));
    // call[1] = worktree add using local "main" (not "origin/main")
    assert_eq!(calls[1].0, "git");
    assert!(calls[1].1.contains(&"worktree".to_string()));
    assert_eq!(
        calls[1].1.last().unwrap(),
        "main",
        "fallback should use local main, got: {:?}",
        calls[1].1
    );
}

#[test]
fn provision_worktree_fetch_uses_custom_base_branch() {
    // Custom base_branch is used in both fetch and worktree add
    let (_dir, repo_path) = make_test_repo();

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // git fetch origin develop
        MockProcessRunner::ok(), // git worktree add
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
    ]);

    let task = make_task(&repo_path);
    provision_worktree(&task, &mock, Some("develop")).unwrap();

    let calls = mock.recorded_calls();
    assert!(
        calls[0].1.contains(&"develop".to_string()),
        "fetch should use 'develop', got: {:?}",
        calls[0].1
    );
    assert_eq!(
        calls[1].1.last().unwrap(),
        "origin/develop",
        "worktree add should use origin/develop, got: {:?}",
        calls[1].1
    );
}

#[test]
fn provision_worktree_skips_fetch_when_dir_exists() {
    // Pre-existing worktree dir → no git calls at all (fetch + worktree add both skipped)
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
    ]);

    let task = make_task(&repo_path);
    provision_worktree(&task, &mock, Some("main")).unwrap();

    let calls = mock.recorded_calls();
    assert!(
        calls.iter().all(|(prog, _)| prog != "git"),
        "no git calls expected when worktree dir already exists, got: {calls:?}"
    );
}

#[test]
fn rebase_preamble_with_base_branch() {
    let preamble = rebase_preamble("99-prev-task");
    assert!(
        preamble.contains("99-prev-task"),
        "should reference the base branch"
    );
    assert!(
        !preamble.contains("origin/main"),
        "should not reference origin/main"
    );
}

#[test]
fn rebase_preamble_uses_given_target() {
    let preamble = rebase_preamble("origin/develop");
    assert!(
        preamble.contains("origin/develop"),
        "should use given target, got: {preamble}"
    );
    assert!(
        !preamble.contains("origin/main"),
        "should not contain origin/main"
    );
}

#[test]
fn resume_skips_git_issues_tmux_continue() {
    let (_dir, worktree_path) = make_test_repo();

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
        MockProcessRunner::ok(), // tmux send-keys -l
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]);

    resume_agent(TaskId(42), &worktree_path, &mock).unwrap();

    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 5);
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(calls[0].1[0], "new-window");
    assert_eq!(calls[1].1[0], "set-option");
    assert_eq!(calls[2].1[0], "set-hook");
    assert!(
        calls.iter().all(|(prog, _)| prog != "git"),
        "resume should make no git calls"
    );
    assert!(calls[3].1.iter().any(|a| a.contains("--continue")));
}

#[test]
fn cleanup_kills_window_and_removes_worktree() {
    let mock = MockProcessRunner::new(vec![
        // has_window: list-windows returns the window name in stdout
        MockProcessRunner::ok_with_stdout(b"task-42\n"),
        MockProcessRunner::ok(), // tmux kill-window
        MockProcessRunner::ok(), // git worktree remove
        MockProcessRunner::ok(), // git branch -D (best-effort)
    ]);

    cleanup_task(
        "/repo",
        "/repo/.worktrees/42-fix-bug",
        Some("task-42"),
        &mock,
    )
    .unwrap();

    let calls = mock.recorded_calls();
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(calls[0].1[0], "list-windows");
    assert_eq!(calls[1].0, "tmux");
    assert_eq!(calls[1].1[0], "kill-window");
    assert_eq!(calls[2].0, "git");
    // git worktree remove is invoked with -C <repo>
    assert!(calls[2].1.contains(&"-C".to_string()));
    assert!(calls[2].1.contains(&"remove".to_string()));
}

#[test]
fn cleanup_succeeds_when_worktree_already_removed() {
    // When git says "not a working tree" the archive should still succeed,
    // not surface an error to the user.
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::fail("fatal: '/repo/.worktrees/42-fix-bug' is not a working tree"),
        MockProcessRunner::ok(), // git branch -D (best-effort)
    ]);

    cleanup_task("/repo", "/repo/.worktrees/42-fix-bug", None, &mock).unwrap();
}

#[test]
fn dispatch_uses_task_base_branch_in_prompt() {
    let (_dir, repo_path, worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let mock = MockProcessRunner::new(vec![
        // No detect_default_branch call — task.base_branch is used directly
        // git worktree add is skipped (dir exists)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]);

    let mut task = make_task(&repo_path);
    task.base_branch = "master".to_string();
    dispatch_agent(&task, &mock, None, None).unwrap();

    // Verify the prompt uses task.base_branch directly — no symbolic-ref call needed
    let prompt_file = worktree_dir.join(".claude-prompt");
    let prompt = std::fs::read_to_string(prompt_file).unwrap();
    assert!(
        prompt.contains("rebase your branch from master"),
        "prompt should reference task.base_branch (master), got: {prompt}"
    );
    assert!(
        !prompt.contains("rebase your branch from main"),
        "prompt should not reference main when task.base_branch is master"
    );
}

#[test]
fn dispatch_fails_fast_if_git_fails() {
    let (_dir, repo_path) = make_test_repo();

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(),                   // git fetch origin main (succeeds)
        MockProcessRunner::fail("not a git repo"), // git worktree add fails
    ]);

    let task = make_task(&repo_path);
    let result = dispatch_agent(&task, &mock, None, None);
    assert!(result.is_err());
    let calls = mock.recorded_calls();
    assert_eq!(
        calls.len(),
        2,
        "only git fetch + git worktree add should have been called (no detect_default_branch)"
    );
}

#[test]
fn quick_dispatch_reuses_existing_worktree() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let mock = MockProcessRunner::new(vec![
        // No detect_default_branch call — task.base_branch is used directly
        // git worktree add is skipped (dir exists)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
        MockProcessRunner::ok(), // tmux send-keys -l
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]);

    let task = make_task(&repo_path);
    quick_dispatch_agent(&task, &mock, None, None).unwrap();

    let calls = mock.recorded_calls();
    assert!(
        calls
            .iter()
            .all(|(prog, args)| !(prog == "git" && args.iter().any(|a| a == "worktree"))),
        "git worktree add should be skipped for existing worktree"
    );
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(calls[0].1[0], "new-window");
}

#[test]
fn quick_dispatch_sends_rename_prompt() {
    let (_dir, repo_path, worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let mock = MockProcessRunner::new(vec![
        // No detect_default_branch call — task.base_branch is used directly
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook (after-split-window)
        MockProcessRunner::ok(), // tmux send-keys -l
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]);

    let task = make_task(&repo_path);
    quick_dispatch_agent(&task, &mock, None, None).unwrap();

    let prompt_file = worktree_dir.join(".claude-prompt");
    let prompt = std::fs::read_to_string(prompt_file).unwrap();
    assert!(
        prompt.contains("placeholder"),
        "prompt should mention placeholder title"
    );
    assert!(
        prompt.contains("update_task"),
        "prompt should mention update_task for rename"
    );
}

// --- finish_task tests ---

#[test]
fn epic_planning_prompt_contains_epic_context() {
    let project = ProjectContext {
        project_id: ProjectId(3),
        project_name: "My Project".to_string(),
    };
    let epic = EpicContext {
        epic_id: EpicId(42),
        epic_title: "Redesign auth".to_string(),
    };
    let prompt = build_epic_planning_prompt(
        TaskId(99),
        "Plan: Redesign auth",
        "Rework the login flow",
        &epic,
        &project,
    );
    assert!(prompt.contains("EpicId: 42"));
    assert!(prompt.contains("Redesign auth"));
    assert!(prompt.contains("Rework the login flow"));
    assert!(prompt.contains("Do NOT start implementing"));
    // Work package instructions
    assert!(
        prompt.contains("create_task"),
        "prompt should instruct using create_task"
    );
    assert!(
        prompt.contains("sort_order"),
        "prompt should explain sort_order for ordering"
    );
    assert!(
        prompt.contains("update_epic"),
        "prompt should instruct attaching plan to epic"
    );
    assert!(
        prompt.contains("repo_path"),
        "prompt should explain repo_path for parallelization"
    );
    assert!(
        prompt.contains("epic_id=42"),
        "update_epic call should include the resolved epic id"
    );
    assert!(
        prompt.contains("/brainstorm"),
        "prompt should direct agent to use the brainstorm skill"
    );
    assert!(
        prompt.contains("work package"),
        "prompt should use 'work package' terminology"
    );
    assert!(
        prompt.contains("ProjectId: 3"),
        "prompt should include the ProjectId line"
    );
    assert!(
        prompt.contains("project_id=3"),
        "prompt should tell agent to set project_id on work packages"
    );
}

#[test]
fn finish_task_happy_path() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // rev-parse HEAD
        MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"), // remote get-url origin
        MockProcessRunner::ok(),                                             // git pull origin main
        MockProcessRunner::ok(), // git rebase main (from worktree)
        MockProcessRunner::ok(), // git merge --ff-only (fast-forward main)
        // Only tmux kill (worktree preserved for archival):
        MockProcessRunner::ok_with_stdout(b"task-42\n"), // tmux list-windows (has_window)
        MockProcessRunner::ok(),                         // tmux kill-window
    ]);

    finish_task(
        "/repo",
        "/repo/.worktrees/42-fix-bug",
        "42-fix-bug",
        "main",
        Some("task-42"),
        &mock,
    )
    .unwrap();

    let calls = mock.recorded_calls();
    assert!(calls.iter().any(|c| c.1.contains(&"rebase".to_string())));
    assert!(calls.iter().any(|c| c.1.contains(&"--ff-only".to_string())));
    // No worktree removal
    assert!(!calls.iter().any(|c| c.1.contains(&"remove".to_string())));
}

#[test]
fn finish_task_with_master_default_branch() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"master\n"), // rev-parse HEAD
        MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"), // remote get-url origin
        MockProcessRunner::ok(), // git pull origin master
        MockProcessRunner::ok(), // git rebase master (from worktree)
        MockProcessRunner::ok(), // git merge --ff-only
    ]);

    finish_task(
        "/repo",
        "/repo/.worktrees/42-fix-bug",
        "42-fix-bug",
        "master",
        None,
        &mock,
    )
    .unwrap();

    let calls = mock.recorded_calls();
    // pull should reference "master" not "main"
    let pull_call = calls
        .iter()
        .find(|c| c.1.contains(&"pull".to_string()))
        .unwrap();
    assert!(pull_call.1.contains(&"master".to_string()));
    // rebase should reference "master"
    let rebase_call = calls
        .iter()
        .find(|c| c.1.contains(&"rebase".to_string()))
        .unwrap();
    assert!(rebase_call.1.contains(&"master".to_string()));
}

#[test]
fn finish_task_not_on_default_branch() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"feature-branch\n"), // rev-parse HEAD
    ]);

    let result = finish_task(
        "/repo",
        "/repo/.worktrees/42-fix-bug",
        "42-fix-bug",
        "main",
        None,
        &mock,
    );
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(err, FinishError::NotOnDefaultBranch { .. }));
    assert!(err.to_string().contains("feature-branch"));
}

#[test]
fn finish_task_rebase_conflict() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"),
        MockProcessRunner::fail(""),                         // remote get-url (no remote)
        Ok(Output {
            status: exit_fail(),
            stdout: b"".to_vec(),
            stderr: b"CONFLICT (content): Merge conflict in src/main.rs\nerror: could not apply abc1234\n".to_vec(),
        }),
        MockProcessRunner::ok(),                             // git rebase --abort
    ]);

    let result = finish_task(
        "/repo",
        "/repo/.worktrees/42-fix-bug",
        "42-fix-bug",
        "main",
        None,
        &mock,
    );
    assert!(matches!(
        result.unwrap_err(),
        FinishError::RebaseConflict(_)
    ));
    let calls = mock.recorded_calls();
    assert!(calls.last().unwrap().1.contains(&"--abort".to_string()));
}

#[test]
fn finish_task_pull_fails() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"),
        MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"), // remote get-url origin
        MockProcessRunner::fail("fatal: unable to access remote"),           // git pull fails
    ]);

    let result = finish_task(
        "/repo",
        "/repo/.worktrees/42-fix-bug",
        "42-fix-bug",
        "main",
        None,
        &mock,
    );
    assert!(matches!(result.unwrap_err(), FinishError::Other(_)));
}

// --- extract_github_repo tests ---

#[test]
fn extract_github_repo_pr_url() {
    assert_eq!(
        extract_github_repo("https://github.com/org/repo/pull/42"),
        Some("org/repo"),
    );
}

#[test]
fn extract_github_repo_issue_url() {
    assert_eq!(
        extract_github_repo("https://github.com/org/repo/issues/5"),
        Some("org/repo"),
    );
}

#[test]
fn extract_github_repo_root_url() {
    assert_eq!(
        extract_github_repo("https://github.com/org/repo"),
        Some("org/repo"),
    );
}

#[test]
fn extract_github_repo_root_url_with_trailing_slash() {
    assert_eq!(
        extract_github_repo("https://github.com/org/repo/"),
        Some("org/repo"),
    );
}

#[test]
fn extract_github_repo_tree_url() {
    assert_eq!(
        extract_github_repo("https://github.com/org/repo/tree/main"),
        Some("org/repo"),
    );
}

#[test]
fn extract_github_repo_non_github_url() {
    assert_eq!(
        extract_github_repo("https://jira.company.com/browse/PROJ-123"),
        None
    );
}

#[test]
fn extract_github_repo_empty_string() {
    assert_eq!(extract_github_repo(""), None);
}

#[test]
fn extract_github_repo_only_one_segment() {
    assert_eq!(extract_github_repo("https://github.com/org"), None);
}

#[test]
fn extract_github_repo_malformed_url() {
    assert_eq!(extract_github_repo("not-a-url"), None);
}

// --- dispatch guard tests ---

#[test]
fn dispatch_agent_fails_fast_with_empty_repo_path() {
    let mock = MockProcessRunner::new(vec![]);
    let mut task = make_task("/some/repo");
    task.repo_path = "".to_string();
    let result = dispatch_agent(&task, &mock, None, None);
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("Repository path"),
        "error should mention 'Repository path', got: {msg}"
    );
}

// --- check_pr_status tests ---

#[test]
fn check_pr_status_open() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        b"OPEN\nREVIEW_REQUIRED\n",
    )]);
    let result = check_pr_status("https://github.com/org/repo/pull/42", &mock).unwrap();
    assert_eq!(result.state, PrState::Open);
    assert_eq!(result.review_decision, Some(ReviewDecision::ReviewRequired));
}

#[test]
fn check_pr_status_merged() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"MERGED\n")]);
    let result = check_pr_status("https://github.com/org/repo/pull/42", &mock).unwrap();
    assert_eq!(result.state, PrState::Merged);
    assert_eq!(result.review_decision, None);
}

#[test]
fn check_pr_status_closed() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"CLOSED\n")]);
    let result = check_pr_status("https://github.com/org/repo/pull/42", &mock).unwrap();
    assert_eq!(result.state, PrState::Closed);
    assert_eq!(result.review_decision, None);
}

#[test]
fn check_pr_status_open_approved() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(b"OPEN\nAPPROVED\n")]);
    let result = check_pr_status("https://github.com/org/repo/pull/42", &mock).unwrap();
    assert_eq!(result.state, PrState::Open);
    assert_eq!(result.review_decision, Some(ReviewDecision::Approved));
}

#[test]
fn check_pr_status_open_changes_requested() {
    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        b"OPEN\nCHANGES_REQUESTED\n",
    )]);
    let result = check_pr_status("https://github.com/org/repo/pull/42", &mock).unwrap();
    assert_eq!(result.state, PrState::Open);
    assert_eq!(
        result.review_decision,
        Some(ReviewDecision::ChangesRequested)
    );
}

#[test]
fn finish_task_no_remote_skips_pull() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // rev-parse HEAD
        MockProcessRunner::fail(""),                  // remote get-url (no remote)
        MockProcessRunner::ok(),                      // git rebase main (from worktree)
        MockProcessRunner::ok(),                      // git merge --ff-only (fast-forward)
                                                      // No tmux window, no cleanup
    ]);

    finish_task(
        "/repo",
        "/repo/.worktrees/42-fix-bug",
        "42-fix-bug",
        "main",
        None,
        &mock,
    )
    .unwrap();
    let calls = mock.recorded_calls();
    // Should not have a "pull" call
    assert!(!calls.iter().any(|c| c.1.contains(&"pull".to_string())));
}

// --- new TDD tests for explicit base_branch ---

#[test]
fn finish_task_uses_explicit_base_branch_not_auto_detected() {
    // "develop" is passed explicitly; no symbolic-ref (detect_default_branch) call
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"develop\n"), // rev-parse HEAD → on develop
        MockProcessRunner::fail(""),                     // remote get-url (no remote)
        MockProcessRunner::ok(),                         // git rebase develop
        MockProcessRunner::ok(),                         // git merge --ff-only develop
    ]);

    finish_task(
        "/repo",
        "/repo/.worktrees/42-fix-bug",
        "42-fix-bug",
        "develop",
        None,
        &mock,
    )
    .unwrap();

    let calls = mock.recorded_calls();
    // No symbolic-ref call — branch was provided explicitly
    assert!(
        !calls
            .iter()
            .any(|c| c.0 == "git" && c.1.iter().any(|a| a == "symbolic-ref")),
        "symbolic-ref must not be called when base_branch is explicit"
    );
    // Rebase should target "develop"
    let rebase = calls
        .iter()
        .find(|c| c.1.contains(&"rebase".to_string()))
        .unwrap();
    assert!(rebase.1.contains(&"develop".to_string()));
}

#[test]
fn dispatch_agent_uses_task_base_branch_in_prompt() {
    let (_dir, repo_path, worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    // No detect_default_branch call expected — task.base_branch is used directly
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]);

    let mut task = make_task(&repo_path);
    task.base_branch = "develop".to_string();
    dispatch_agent(&task, &mock, None, None).unwrap();

    let prompt_file = worktree_dir.join(".claude-prompt");
    let prompt = std::fs::read_to_string(prompt_file).unwrap();
    assert!(
        prompt.contains("rebase your branch from develop"),
        "prompt should reference task.base_branch (develop), got: {prompt}"
    );
    // No symbolic-ref call
    let calls = mock.recorded_calls();
    assert!(
        !calls
            .iter()
            .any(|c| c.0 == "git" && c.1.iter().any(|a| a == "symbolic-ref")),
        "dispatch_agent must not call symbolic-ref when task.base_branch is set"
    );
}

// --- dispatch_review_agent tests ---

fn review_req(
    repo_path: &str,
    number: i64,
    head_ref: &str,
    is_dependabot: bool,
) -> crate::tui::ReviewAgentRequest {
    crate::tui::ReviewAgentRequest {
        repo: repo_path.to_string(),
        github_repo: "acme/app".to_string(),
        number,
        head_ref: head_ref.to_string(),
        is_dependabot,
    }
}

#[test]
fn review_agent_returns_early_when_window_exists() {
    let (dir, repo_path) = make_test_repo();
    let repo_short = dir.path().file_name().unwrap().to_str().unwrap();
    let tmux_window = format!("review-{repo_short}-99");

    let mock = MockProcessRunner::new(vec![
        // has_window: list-windows returns the window name
        MockProcessRunner::ok_with_stdout(tmux_window.as_bytes()),
    ]);

    let result =
        dispatch_review_agent(&review_req(&repo_path, 99, "feature-branch", false), &mock).unwrap();

    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1, "only list-windows should be called");
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(calls[0].1[0], "list-windows");
    assert_eq!(result.tmux_window, tmux_window);
    let expected_worktree = format!("{repo_path}/.worktrees/review-99");
    assert_eq!(result.worktree_path, expected_worktree);
}

#[test]
fn review_agent_skips_worktree_add_when_dir_exists() {
    // Pre-create the review worktree directory
    let (_dir, repo_path, worktree_dir) = make_test_repo_with_worktree("review-99");

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"other-window\n"), // has_window: no match
        MockProcessRunner::ok(),                              // git worktree prune
        MockProcessRunner::ok(),                              // git fetch origin feature-branch
        // git worktree add is skipped (dir exists)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux send-keys -l
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]);

    let result =
        dispatch_review_agent(&review_req(&repo_path, 99, "feature-branch", false), &mock).unwrap();

    let calls = mock.recorded_calls();
    assert!(
        calls
            .iter()
            .all(|(prog, args)| !(prog == "git" && args.iter().any(|a| a == "add"))),
        "git worktree add should be skipped when dir exists"
    );
    // git fetch should still happen
    assert_eq!(calls[2].0, "git");
    assert!(calls[2].1.contains(&"fetch".to_string()));
    assert!(calls[2].1.contains(&"feature-branch".to_string()));
    assert_eq!(result.worktree_path, worktree_dir.to_str().unwrap());
}

#[test]
fn review_agent_happy_path_writes_prompt_and_launches_claude() {
    // Pre-create worktree dir (simulates a previous fetch or existing
    // worktree — the mock git worktree add can't create dirs on disk)
    let (dir, repo_path, worktree_dir) = make_test_repo_with_worktree("review-99");

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"other-window\n"), // has_window: no match
        MockProcessRunner::ok(),                              // git worktree prune
        MockProcessRunner::ok(),                              // git fetch origin feature-branch
        // git worktree add skipped (dir exists)
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux send-keys -l
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]);

    let result =
        dispatch_review_agent(&review_req(&repo_path, 99, "feature-branch", false), &mock).unwrap();

    let calls = mock.recorded_calls();
    // Verify git fetch
    assert_eq!(calls[2].0, "git");
    assert!(calls[2].1.contains(&"fetch".to_string()));
    assert!(calls[2].1.contains(&"feature-branch".to_string()));
    // Verify tmux new-window
    assert_eq!(calls[3].0, "tmux");
    assert_eq!(calls[3].1[0], "new-window");
    // Verify send-keys includes claude command
    assert!(
        calls[4].1.iter().any(|a| a.contains("claude")),
        "send-keys should include claude command"
    );

    // Verify prompt file content
    let prompt_file = worktree_dir.join(".claude-prompt");
    let prompt = std::fs::read_to_string(prompt_file).unwrap();
    assert!(
        prompt.contains("PR #99"),
        "prompt should reference PR number"
    );
    assert!(
        prompt.contains("/anthropic-review-pr:review-pr 99"),
        "prompt should invoke fully qualified /anthropic-review-pr:review-pr skill"
    );
    assert!(
        prompt.contains("update_review_status"),
        "prompt should reference MCP tool"
    );

    let repo_short = dir.path().file_name().unwrap().to_str().unwrap();
    assert_eq!(result.tmux_window, format!("review-{repo_short}-99"));
}

#[test]
fn review_agent_calls_worktree_add_when_dir_missing() {
    let (_dir, repo_path) = make_test_repo();
    // Do NOT pre-create the review worktree directory

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"other-window\n"), // has_window: no match
        MockProcessRunner::ok(),                              // git worktree prune
        MockProcessRunner::ok(),                              // git fetch origin feature-branch
        MockProcessRunner::ok(),                              // git worktree add
        MockProcessRunner::ok(),                              // tmux new-window
                                                              // fs::write will fail (mock worktree add doesn't create dir),
                                                              // but we can still verify the calls made so far
    ]);

    // The function will error at fs::write since mock doesn't create the dir
    let result = dispatch_review_agent(&review_req(&repo_path, 99, "feature-branch", false), &mock);
    assert!(result.is_err());

    let calls = mock.recorded_calls();
    // Verify git worktree add was called with correct args
    let wt_call = calls.iter().find(|(prog, args)| {
        prog == "git" && args.contains(&"add".to_string()) && args.contains(&"worktree".to_string())
    });
    assert!(
        wt_call.is_some(),
        "git worktree add should be called when dir is missing"
    );
    let (_, args) = wt_call.unwrap();
    assert!(args.iter().any(|a| a == "origin/feature-branch"));
}

#[test]
fn review_agent_fails_when_git_fetch_fails() {
    let (_dir, repo_path) = make_test_repo();

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"\n"), // has_window: no match
        MockProcessRunner::ok(),                  // git worktree prune
        MockProcessRunner::fail("fatal: couldn't find remote ref"), // git fetch fails
    ]);

    let result = dispatch_review_agent(&review_req(&repo_path, 99, "nonexistent", false), &mock);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("git fetch failed"));
}

#[test]
fn review_agent_uses_accept_edits_mode() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("review-99");

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"\n"), // has_window: no match
        MockProcessRunner::ok(),                  // git worktree prune
        MockProcessRunner::ok(),                  // git fetch
        // worktree exists, skip add
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux send-keys -l
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]);

    dispatch_review_agent(&review_req(&repo_path, 99, "feature-branch", false), &mock).unwrap();

    let calls = mock.recorded_calls();
    let send_keys_arg = find_call_arg(&calls, 4, "claude");
    assert!(
        send_keys_arg.contains("--permission-mode acceptEdits"),
        "review agent should use acceptEdits mode, got: {send_keys_arg}"
    );
}

// --- plugin-dir tests ---

#[test]
fn dispatch_agent_includes_plugin_dir() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");

    let mock = MockProcessRunner::new(vec![
        // No detect_default_branch call — task.base_branch is used directly
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]);

    let task = make_task(&repo_path);
    dispatch_agent(&task, &mock, None, None).unwrap();

    let calls = mock.recorded_calls();
    let send_keys_arg = find_call_arg(&calls, 3, "claude");
    assert!(
        send_keys_arg.contains("--plugin-dir"),
        "dispatch_agent should include --plugin-dir, got: {send_keys_arg}"
    );
    assert!(
        send_keys_arg.contains(".claude/plugins/local/dispatch"),
        "plugin-dir should point to local dispatch plugin, got: {send_keys_arg}"
    );
}

#[test]
fn resume_agent_includes_plugin_dir() {
    let (_dir, worktree_path) = make_test_repo();

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]);

    resume_agent(TaskId(42), &worktree_path, &mock).unwrap();

    let calls = mock.recorded_calls();
    let send_keys_arg = find_call_arg(&calls, 3, "claude");
    assert!(
        send_keys_arg.contains("--plugin-dir"),
        "resume_agent should include --plugin-dir, got: {send_keys_arg}"
    );
    assert!(
        send_keys_arg.contains(".claude/plugins/local/dispatch"),
        "plugin-dir should point to local dispatch plugin, got: {send_keys_arg}"
    );
}

#[test]
fn review_agent_includes_plugin_dir() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("review-99");

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"\n"), // has_window: no match
        MockProcessRunner::ok(),                  // git worktree prune
        MockProcessRunner::ok(),                  // git fetch
        MockProcessRunner::ok(),                  // tmux new-window
        MockProcessRunner::ok(),                  // tmux send-keys -l
        MockProcessRunner::ok(),                  // tmux send-keys Enter
    ]);

    dispatch_review_agent(&review_req(&repo_path, 99, "feature-branch", false), &mock).unwrap();

    let calls = mock.recorded_calls();
    let send_keys_arg = find_call_arg(&calls, 4, "claude");
    assert!(
        send_keys_arg.contains("--plugin-dir"),
        "review_agent should include --plugin-dir, got: {send_keys_arg}"
    );
    assert!(
        send_keys_arg.contains(".claude/plugins/local/dispatch"),
        "plugin-dir should point to local dispatch plugin, got: {send_keys_arg}"
    );
}

// --- fix_req helper ---

fn fix_req(repo_path: &str, number: i64) -> FixAgentRequest {
    FixAgentRequest {
        repo: repo_path.to_string(),
        github_repo: "acme/app".to_string(),
        number,
        kind: AlertKind::Dependabot,
        title: "CVE-2024-1234".to_string(),
        description: "A known vulnerability".to_string(),
        package: Some("some-crate".to_string()),
        fixed_version: Some("1.2.3".to_string()),
    }
}

// --- dispatch_fix_agent tests ---

#[test]
fn fix_agent_returns_early_when_window_exists() {
    let (dir, repo_path) = make_test_repo();
    let repo_short = dir.path().file_name().unwrap().to_str().unwrap();
    let tmux_window = format!("fix-{repo_short}-1");

    let mock = MockProcessRunner::new(vec![MockProcessRunner::ok_with_stdout(
        tmux_window.as_bytes(),
    )]);

    let result = dispatch_fix_agent(fix_req(&repo_path, 1), &mock).unwrap();

    let calls = mock.recorded_calls();
    assert_eq!(calls.len(), 1, "only list-windows should be called");
    assert_eq!(calls[0].0, "tmux");
    assert_eq!(calls[0].1[0], "list-windows");
    assert_eq!(result.tmux_window, tmux_window);
    let expected_worktree = format!("{repo_path}/.worktrees/fix-vuln-1");
    assert_eq!(result.worktree_path, expected_worktree);
}

#[test]
fn fix_agent_calls_worktree_add_with_new_branch_when_dir_missing() {
    let (_dir, repo_path) = make_test_repo();
    // Do NOT pre-create the worktree dir — git worktree add must be called.

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"\n"), // has_window: no match
        MockProcessRunner::ok(),                  // git worktree prune
        MockProcessRunner::ok_with_stdout(b"refs/remotes/origin/main\n"), // detect_default_branch
        MockProcessRunner::ok(),                  // git fetch origin main
        MockProcessRunner::ok(),                  // git worktree add -b
        MockProcessRunner::ok(),                  // tmux new-window
                                                  // fs::write will fail (mock worktree add doesn't create dir on disk)
    ]);

    // The function errors at fs::write — we still verify the calls made before that.
    let result = dispatch_fix_agent(fix_req(&repo_path, 1), &mock);
    assert!(result.is_err());

    let calls = mock.recorded_calls();
    let wt_call = calls.iter().find(|(prog, args)| {
        prog == "git" && args.contains(&"add".to_string()) && args.contains(&"worktree".to_string())
    });
    assert!(
        wt_call.is_some(),
        "git worktree add should be called when dir is missing"
    );
    let (_, args) = wt_call.unwrap();
    assert!(
        args.contains(&"-b".to_string()),
        "NewBranch strategy should pass -b flag, got: {args:?}"
    );
    assert!(
        args.iter().any(|a| a.contains("fix/vuln")),
        "branch name should contain fix/vuln, got: {args:?}"
    );
}

#[test]
fn fix_agent_with_existing_dir_skips_worktree_add_and_uses_accept_edits() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("fix-vuln-1");

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"\n"), // has_window: no match
        MockProcessRunner::ok(),                  // git worktree prune
        MockProcessRunner::ok_with_stdout(b"refs/remotes/origin/main\n"), // detect_default_branch
        MockProcessRunner::ok(),                  // git fetch origin main
        MockProcessRunner::ok(),                  // tmux new-window
        MockProcessRunner::ok(),                  // tmux send-keys -l
        MockProcessRunner::ok(),                  // tmux send-keys Enter
    ]);

    dispatch_fix_agent(fix_req(&repo_path, 1), &mock).unwrap();

    let calls = mock.recorded_calls();
    assert!(
        calls
            .iter()
            .all(|(prog, args)| !(prog == "git" && args.iter().any(|a| a == "add"))),
        "git worktree add should be skipped when dir exists, got: {calls:?}"
    );
    let send_keys_arg = find_call_arg(&calls, 5, "claude");
    assert!(
        send_keys_arg.contains("--permission-mode acceptEdits"),
        "fix agent should use acceptEdits mode, got: {send_keys_arg}"
    );
}

// --- provision_and_dispatch error paths ---

#[test]
fn provision_and_dispatch_worktree_add_fails() {
    let (_dir, repo_path) = make_test_repo();
    // Do NOT pre-create worktree dir — git worktree add will be attempted.

    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"\n"), // has_window: no match
        MockProcessRunner::ok(),                  // git worktree prune
        MockProcessRunner::ok(),                  // git fetch origin feature-x
        MockProcessRunner::fail("fatal: '/path' already exists"), // git worktree add — fails
    ]);

    let result = dispatch_review_agent(&review_req(&repo_path, 5, "feature-x", false), &mock);

    assert!(
        result.is_err(),
        "worktree add failure should propagate as error"
    );
    let calls = mock.recorded_calls();
    assert!(
        calls
            .iter()
            .all(|(prog, args)| !(prog == "tmux" && args.iter().any(|a| a == "new-window"))),
        "tmux new-window should not be called when worktree add fails, got: {calls:?}"
    );
}

// --- provision_worktree error path ---

#[test]
fn provision_worktree_git_add_fails_returns_error() {
    let (_dir, repo_path) = make_test_repo();
    // No base_branch → no fetch; first runner call is git worktree add.
    let mock = MockProcessRunner::new(vec![MockProcessRunner::fail("fatal: not a git repository")]);

    let task = make_task(&repo_path);
    let result = provision_worktree(&task, &mock, None);

    assert!(result.is_err(), "git worktree add failure should propagate");
}

// --- cleanup_task edge cases ---

#[test]
fn cleanup_skips_kill_when_window_not_found() {
    // tmux_window is Some but has_window returns false (window already gone).
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"\n"), // has_window: empty → false
        MockProcessRunner::ok(),                  // git worktree remove
        MockProcessRunner::ok(),                  // git branch -D (best-effort)
    ]);

    cleanup_task(
        "/repo",
        "/repo/.worktrees/42-fix-bug",
        Some("task-42"),
        &mock,
    )
    .unwrap();

    let calls = mock.recorded_calls();
    assert!(
        calls
            .iter()
            .all(|(prog, args)| !(prog == "tmux" && args.iter().any(|a| a == "kill-window"))),
        "kill-window should not be called when window not found, got: {calls:?}"
    );
}

// --- finish_task edge cases ---

#[test]
fn finish_task_skips_kill_when_tmux_window_not_found() {
    // The tmux window has already disappeared before finish_task runs.
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"main\n"), // rev-parse HEAD
        MockProcessRunner::fail(""),                  // remote get-url (no remote)
        MockProcessRunner::ok(),                      // git rebase main (from worktree)
        MockProcessRunner::ok(),                      // git merge --ff-only
        MockProcessRunner::ok_with_stdout(b"\n"),     // tmux list-windows — no match
    ]);

    finish_task(
        "/repo",
        "/repo/.worktrees/42-fix-bug",
        "42-fix-bug",
        "main",
        Some("task-42"),
        &mock,
    )
    .unwrap();

    let calls = mock.recorded_calls();
    assert!(
        calls
            .iter()
            .all(|(prog, args)| !(prog == "tmux" && args.iter().any(|a| a == "kill-window"))),
        "kill-window should not be called when window not found, got: {calls:?}"
    );
}
