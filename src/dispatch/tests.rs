use super::prompts::{
    allium_instruction, build_brainstorm_prompt, build_epic_planning_prompt, build_plan_prompt,
    build_prompt, build_quick_dispatch_prompt, build_tmux_window_name, epic_preamble,
    mcp_tools_instruction, plan_and_attach_instruction, rebase_preamble, task_block,
    tdd_instruction, wrap_up_instruction, EpicContext,
};
use super::worktree::provision_worktree;
use super::*;

use crate::models::{
    EpicId, Learning, LearningKind, LearningScope, LearningStatus, Task, TaskId, TaskStatus,
};
use crate::process::{exit_fail, MockProcessRunner};
use chrono::Utc;
use std::process::Output;

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
fn mcp_tools_instruction_mentions_get_and_update() {
    let instr = mcp_tools_instruction();
    assert!(instr.contains("get_task"));
    assert!(instr.contains("update_task"));
}

#[test]
fn plan_and_attach_instruction_mentions_docs_plans_and_update_task() {
    let instr = plan_and_attach_instruction(TaskId(9));
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

#[test]
fn format_learnings_preamble_returns_none_for_empty_slice() {
    assert!(super::agents::format_learnings_preamble(&[]).is_none());
}

#[test]
fn format_learnings_preamble_procedural_only_has_instructions_section() {
    let learning = make_learning(LearningKind::Procedural, "Always run cargo fmt --check.");
    let preamble = super::agents::format_learnings_preamble(&[learning]).unwrap();
    assert!(
        preamble.contains("# Instructions from past experience"),
        "should have instructions heading"
    );
    assert!(
        preamble.contains("Always run cargo fmt --check."),
        "should include summary"
    );
    assert!(
        !preamble.contains("# Relevant learnings"),
        "should not have relevant learnings heading"
    );
}

#[test]
fn format_learnings_preamble_non_procedural_only_has_relevant_section() {
    let learning = make_learning(
        LearningKind::Convention,
        "Use FieldUpdate for nullable fields.",
    );
    let preamble = super::agents::format_learnings_preamble(&[learning]).unwrap();
    assert!(
        preamble.contains("# Relevant learnings"),
        "should have relevant learnings heading"
    );
    assert!(
        preamble.contains("[Convention]"),
        "should include kind label in brackets"
    );
    assert!(
        preamble.contains("Use FieldUpdate for nullable fields."),
        "should include summary"
    );
    assert!(
        !preamble.contains("# Instructions from past experience"),
        "should not have instructions heading"
    );
}

#[test]
fn format_learnings_preamble_mixed_has_both_sections() {
    let proc_l = make_learning(LearningKind::Procedural, "Run tests before committing.");
    let conv_l = make_learning(LearningKind::Convention, "Use snake_case for file names.");
    let preamble = super::agents::format_learnings_preamble(&[proc_l, conv_l]).unwrap();
    assert!(
        preamble.contains("# Instructions from past experience"),
        "should have instructions section"
    );
    assert!(
        preamble.contains("# Relevant learnings"),
        "should have relevant learnings section"
    );
}

#[test]
fn format_learnings_preamble_procedural_has_no_kind_label() {
    let learning = make_learning(LearningKind::Procedural, "Always rebase before pushing.");
    let preamble = super::agents::format_learnings_preamble(&[learning]).unwrap();
    assert!(
        !preamble.contains("[Procedural]"),
        "procedural learnings should not show a kind label"
    );
    assert!(
        preamble.contains("- Always rebase before pushing."),
        "procedural items should be plain bullets"
    );
}

#[test]
fn format_learnings_preamble_kind_labels_are_human_readable() {
    let pitfall = make_learning(LearningKind::Pitfall, "Watch out for X.");
    let tool_rec = make_learning(LearningKind::ToolRecommendation, "Use ripgrep.");
    let episodic = make_learning(LearningKind::Episodic, "Last time we tried Y.");
    let preference = make_learning(LearningKind::Preference, "User prefers short commits.");

    let preamble =
        super::agents::format_learnings_preamble(&[pitfall, tool_rec, episodic, preference])
            .unwrap();
    assert!(preamble.contains("[Pitfall]"), "Pitfall label");
    assert!(
        preamble.contains("[Tool recommendation]"),
        "ToolRecommendation label"
    );
    assert!(preamble.contains("[Episodic]"), "Episodic label");
    assert!(preamble.contains("[Preference]"), "Preference label");
}

#[test]
fn dispatch_agent_prepends_procedural_learnings_to_prompt() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]);
    let task = make_task(&repo_path);
    let learning = make_learning(
        LearningKind::Procedural,
        "Always run cargo fmt --check before pushing.",
    );
    let result = dispatch_agent(&task, &mock, None, &[learning]).unwrap();
    let written =
        std::fs::read_to_string(format!("{}/.claude-prompt", result.worktree_path)).unwrap();
    assert!(
        written.contains("# Instructions from past experience"),
        "prompt should contain instructions section; got:\n{written}"
    );
    assert!(
        written.contains("Always run cargo fmt --check before pushing."),
        "prompt should contain learning summary"
    );
}

#[test]
fn dispatch_agent_with_no_learnings_omits_preamble() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]);
    let task = make_task(&repo_path);
    let result = dispatch_agent(&task, &mock, None, &[]).unwrap();
    let written =
        std::fs::read_to_string(format!("{}/.claude-prompt", result.worktree_path)).unwrap();
    assert!(
        !written.contains("# Instructions from past experience"),
        "empty learnings should not add instructions section"
    );
    assert!(
        !written.contains("# Relevant learnings"),
        "empty learnings should not add relevant learnings section"
    );
}

#[test]
fn dispatch_agent_relevant_learnings_section_uses_kind_labels() {
    let (_dir, repo_path, _worktree_dir) = make_test_repo_with_worktree("42-fix-bug");
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // tmux new-window
        MockProcessRunner::ok(), // tmux set-option @dispatch_dir
        MockProcessRunner::ok(), // tmux set-hook
        MockProcessRunner::ok(), // tmux send-keys -l
        MockProcessRunner::ok(), // tmux send-keys Enter
    ]);
    let task = make_task(&repo_path);
    let learning = make_learning(LearningKind::Pitfall, "Watch out for X.");
    let result = dispatch_agent(&task, &mock, None, &[learning]).unwrap();
    let written =
        std::fs::read_to_string(format!("{}/.claude-prompt", result.worktree_path)).unwrap();
    assert!(
        written.contains("# Relevant learnings"),
        "non-procedural learning should produce relevant learnings section"
    );
    assert!(
        written.contains("[Pitfall]"),
        "pitfall learning should have [Pitfall] label"
    );
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
        project_id: 1,
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

fn make_learning(kind: LearningKind, summary: &str) -> Learning {
    Learning {
        id: 1,
        kind,
        summary: summary.to_string(),
        detail: None,
        scope: LearningScope::User,
        scope_ref: None,
        tags: vec![],
        status: LearningStatus::Approved,
        source_task_id: None,
        confirmed_count: 0,
        last_confirmed_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
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
    let prompt = build_prompt(TaskId(42), "Fix bug", "A nasty crash", None, None);
    assert!(prompt.contains("42"));
    assert!(prompt.contains("Fix bug"));
    assert!(prompt.contains("A nasty crash"));
    assert!(prompt.contains("TDD"));
}

#[test]
fn build_prompt_mentions_tdd() {
    let prompt = build_prompt(TaskId(7), "Title", "Desc", None, None);
    assert!(prompt.contains("TDD"));
    assert!(prompt.contains("behaviour as tests first"));
}

#[test]
fn build_prompt_mentions_wrap_up_skill() {
    // wrap-up instruction only appears when a plan exists (agent is implementing)
    let prompt = build_prompt(TaskId(7), "Title", "Desc", Some("docs/plans/p.md"), None);
    assert!(
        prompt.contains("/wrap-up"),
        "with-plan prompt should tell agent to use /wrap-up skill"
    );
    assert!(
        prompt.contains("rebase") || prompt.contains("PR"),
        "with-plan prompt should mention rebase/PR choice"
    );
}

#[test]
fn build_prompt_without_plan_omits_wrap_up() {
    let prompt = build_prompt(TaskId(7), "Title", "Desc", None, None);
    assert!(
        !prompt.contains("/wrap-up"),
        "no-plan prompt should not mention /wrap-up (agent isn't implementing yet)"
    );
}

#[test]
fn build_prompt_without_plan_includes_planning_instruction() {
    let prompt = build_prompt(TaskId(1), "Task", "Desc", None, None);
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
    let prompt = build_prompt(TaskId(1), "Task", "Desc", None, None);
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
    let prompt = build_prompt(TaskId(1), "Task", "Desc", None, None);
    assert!(
        prompt.contains("implementation plan directly"),
        "no-plan prompt should offer writing a plan directly for clear descriptions"
    );
}

#[test]
fn build_prompt_with_plan_asks_permission_before_implementing() {
    let prompt = build_prompt(TaskId(1), "Task", "Desc", Some("docs/plans/plan.md"), None);
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
    let prompt = build_prompt(TaskId(1), "Task", "Desc", None, None);
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
    );
    assert!(prompt.contains("Plan: docs/plans/my-plan.md"));
}

#[test]
fn build_prompt_without_plan_omits_plan_section() {
    let prompt = build_prompt(TaskId(1), "Task", "Desc", None, None);
    assert!(!prompt.contains("Plan:"));
}

#[test]
fn build_quick_dispatch_prompt_includes_planning_instruction() {
    let prompt = build_quick_dispatch_prompt(TaskId(42), "Quick task", "", None);
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
    let prompt = build_quick_dispatch_prompt(TaskId(42), "Quick task", "", None);
    assert!(prompt.contains("42"));
    assert!(prompt.contains("Quick task"));
    assert!(prompt.contains("update_task"));
    assert!(prompt.contains("title"));
    assert!(prompt.contains("placeholder"));
}

#[test]
fn build_quick_dispatch_prompt_mentions_mcp() {
    let prompt = build_quick_dispatch_prompt(TaskId(1), "Quick task", "", None);
    assert!(prompt.contains("dispatch MCP tools"));
    assert!(prompt.contains("update_task"));
    assert!(!prompt.contains("add_note"));
}

#[test]
fn build_quick_dispatch_prompt_differs_from_regular() {
    let regular = build_prompt(TaskId(1), "Task", "Desc", None, None);
    let quick = build_quick_dispatch_prompt(TaskId(1), "Task", "Desc", None);
    assert!(quick.contains("placeholder"));
    assert!(!regular.contains("placeholder"));
}

#[test]
fn build_quick_dispatch_prompt_includes_epic_context() {
    let ctx = EpicContext {
        epic_id: EpicId(7),
        epic_title: "My Epic".to_string(),
    };
    let prompt = build_quick_dispatch_prompt(TaskId(42), "Quick task", "", Some(&ctx));
    assert!(prompt.contains("EpicId: 7"), "should include epic ID");
    assert!(prompt.contains("My Epic"), "should include epic title");
    assert!(
        prompt.contains("send_message"),
        "should tell agent how to message sibling agents"
    );
}

#[test]
fn rebase_preamble_prepended_to_all_prompts() {
    let body = build_prompt(TaskId(1), "Task", "Desc", None, None);
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
fn build_brainstorm_prompt_contains_task_info() {
    let prompt = build_brainstorm_prompt(TaskId(7), "Design auth", "Rework the auth flow", None);
    assert!(prompt.contains("7"));
    assert!(prompt.contains("Design auth"));
    assert!(prompt.contains("Rework the auth flow"));
    assert!(prompt.contains("brainstorm"));
    assert!(prompt.contains("update_task"));
}

#[test]
fn build_plan_prompt_contains_task_info() {
    let prompt = build_plan_prompt(TaskId(8), "Add feature", "Small improvement", None);
    assert!(prompt.contains("8"));
    assert!(prompt.contains("Add feature"));
    assert!(prompt.contains("Small improvement"));
    assert!(prompt.contains("/plan"));
    assert!(prompt.contains("update_task"));
}

#[test]
fn build_plan_prompt_differs_from_brainstorm() {
    let plan = build_plan_prompt(TaskId(1), "T", "D", None);
    let brainstorm = build_brainstorm_prompt(TaskId(1), "T", "D", None);
    assert_ne!(plan, brainstorm);
    assert!(plan.contains("planning"));
    assert!(brainstorm.contains("brainstorm"));
}

#[test]
fn brainstorm_prompt_omits_tdd() {
    let prompt = build_brainstorm_prompt(TaskId(7), "Design auth", "Rework auth", None);
    assert!(
        !prompt.contains("TDD"),
        "brainstorm prompt should not include TDD — no code is written at design stage"
    );
}

#[test]
fn brainstorm_prompt_omits_clarifying_questions_opener() {
    let prompt = build_brainstorm_prompt(TaskId(7), "Design auth", "Rework auth", None);
    assert!(
        !prompt.contains("clarifying questions"),
        "brainstorm prompt should not have a clarifying-questions opener — /brainstorming skill handles it"
    );
}

#[test]
fn all_planning_prompts_reference_brainstorming_skill() {
    let brainstorm = build_brainstorm_prompt(TaskId(1), "T", "D", None);
    let plan = build_plan_prompt(TaskId(1), "T", "D", None);
    let standard = build_prompt(TaskId(1), "T", "D", None, None);
    let quick = build_quick_dispatch_prompt(TaskId(1), "T", "D", None);

    for (name, prompt) in [
        ("brainstorm", brainstorm),
        ("plan", plan),
        ("standard-no-plan", standard),
        ("quick", quick),
    ] {
        assert!(
            prompt.contains("/brainstorming"),
            "{name} prompt should reference /brainstorming skill"
        );
    }
}

#[test]
fn plan_and_attach_instruction_is_concise() {
    let instruction = plan_and_attach_instruction(TaskId(42));
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
    dispatch_agent(&task, &mock, None, &[]).unwrap();

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
    dispatch_agent(&task, &mock, None, &[]).unwrap();

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
    dispatch_agent(&task, &mock, None, &[]).unwrap();

    let calls = mock.recorded_calls();
    let send_keys_arg = find_call_arg(&calls, 3, "claude");
    assert!(
        send_keys_arg.contains("--permission-mode plan"),
        "dispatch_agent should use plan mode, got: {send_keys_arg}"
    );
}

#[test]
fn plan_agent_uses_plan_mode() {
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
    plan_agent(&task, &mock, None, &[]).unwrap();

    let calls = mock.recorded_calls();
    let send_keys_arg = find_call_arg(&calls, 3, "claude");
    assert!(
        send_keys_arg.contains("--permission-mode plan"),
        "plan_agent should use plan mode, got: {send_keys_arg}"
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
    dispatch_agent(&task, &mock, None, &[]).unwrap();

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
    let result = dispatch_agent(&task, &mock, None, &[]);
    assert!(result.is_err());
    let calls = mock.recorded_calls();
    assert_eq!(
        calls.len(),
        2,
        "only git fetch + git worktree add should have been called (no detect_default_branch)"
    );
}

#[test]
fn brainstorm_reuses_existing_worktree() {
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
    brainstorm_agent(&task, &mock, None, &[]).unwrap();

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
fn brainstorm_sends_brainstorm_prompt() {
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
    brainstorm_agent(&task, &mock, None, &[]).unwrap();

    // Verify the prompt file was written with brainstorm content
    let prompt_file = worktree_dir.join(".claude-prompt");
    let prompt = std::fs::read_to_string(prompt_file).unwrap();
    assert!(
        prompt.contains("brainstorm"),
        "prompt should mention brainstorming"
    );
    assert!(
        prompt.contains("/brainstorming"),
        "prompt should reference /brainstorming skill"
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
    quick_dispatch_agent(&task, &mock, None, &[]).unwrap();

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
    quick_dispatch_agent(&task, &mock, None, &[]).unwrap();

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
    let prompt = build_epic_planning_prompt(EpicId(42), "Redesign auth", "Rework the login flow");
    assert!(prompt.contains("42"));
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
    assert_eq!(extract_github_repo("https://jira.company.com/browse/PROJ-123"), None);
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
    let result = dispatch_agent(&task, &mock, None, &[]);
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("Repository path"),
        "error should mention 'Repository path', got: {msg}"
    );
}

// --- create_pr tests ---

#[test]
fn create_pr_happy_path() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // git push
        MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"), // git remote get-url origin
        MockProcessRunner::ok_with_stdout(b"https://github.com/org/repo/pull/42\n"), // gh pr create
    ]);

    let result = create_pr(
        "/repo",
        "42-fix-bug",
        "Fix bug",
        "A nasty crash",
        "main",
        &mock,
    )
    .unwrap();
    assert_eq!(result.pr_url, "https://github.com/org/repo/pull/42");

    let calls = mock.recorded_calls();
    assert_eq!(calls[0].0, "git");
    assert!(calls[0].1.contains(&"push".to_string()));
    assert!(calls[0].1.contains(&"-u".to_string()));
    assert_eq!(calls[1].0, "git");
    assert!(calls[1].1.contains(&"get-url".to_string()));
    assert_eq!(calls[2].0, "gh");
    assert!(calls[2].1.contains(&"create".to_string()));
    assert!(calls[2].1.contains(&"--draft".to_string()));
    assert!(calls[2].1.contains(&"org/repo".to_string()));
    // --head must include owner prefix to avoid gh resolving it in the wrong namespace
    assert!(
        calls[2].1.contains(&"org:42-fix-bug".to_string()),
        "--head must be owner:branch, got: {:?}",
        calls[2].1
    );
}

#[test]
fn create_pr_head_ref_includes_owner_prefix() {
    // Regression: gh pr create --head branch (no owner) causes GitHub to resolve the
    // branch in the authenticated user's namespace instead of the --repo owner's namespace,
    // producing "Head sha can't be blank" errors. The fix is to always pass owner:branch.
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(),
        MockProcessRunner::ok_with_stdout(b"https://github.com/myorg/myrepo.git\n"),
        MockProcessRunner::ok_with_stdout(b"https://github.com/myorg/myrepo/pull/1\n"),
    ]);

    create_pr("/repo", "99-my-feature", "Feature", "desc", "main", &mock).unwrap();

    let calls = mock.recorded_calls();
    let gh_args = &calls[2].1;
    let head_idx = gh_args
        .iter()
        .position(|a| a == "--head")
        .expect("--head flag must be present");
    assert_eq!(
        gh_args[head_idx + 1],
        "myorg:99-my-feature",
        "--head value must be owner:branch"
    );
}

#[test]
fn create_pr_with_master_base() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // git push
        MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"), // git remote get-url origin
        MockProcessRunner::ok_with_stdout(b"https://github.com/org/repo/pull/42\n"), // gh pr create
    ]);

    create_pr("/repo", "42-fix-bug", "Fix bug", "desc", "master", &mock).unwrap();

    let calls = mock.recorded_calls();
    let gh_call = calls.iter().find(|c| c.0 == "gh").unwrap();
    assert!(gh_call.1.contains(&"master".to_string()));
}

#[test]
fn create_pr_push_fails() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::fail("fatal: no remote"), // git push fails
    ]);

    let result = create_pr("/repo", "42-fix-bug", "Fix bug", "desc", "main", &mock);
    assert!(matches!(result, Err(PrError::PushFailed(_))));
}

#[test]
fn create_pr_gh_create_fails() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // git push succeeds
        MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"), // git remote get-url
        MockProcessRunner::fail("error: pull request already exists"), // gh pr create
    ]);

    let result = create_pr("/repo", "42-fix-bug", "Fix bug", "desc", "main", &mock);
    assert!(matches!(result, Err(PrError::CreateFailed(_))));
}

#[test]
fn create_pr_returns_existing_url_when_pr_already_exists() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // git push
        MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"), // git remote get-url
        MockProcessRunner::fail(
            "a pull request for branch '42-fix-bug' already exists:\nhttps://github.com/org/repo/pull/7",
        ), // gh pr create — already exists
    ]);

    let result = create_pr("/repo", "42-fix-bug", "Fix bug", "desc", "main", &mock);
    assert!(
        result.is_ok(),
        "create_pr should succeed when PR already exists, got: {result:?}"
    );
    assert_eq!(result.unwrap().pr_url, "https://github.com/org/repo/pull/7");
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

// --- parse_repo_slug tests ---

#[test]
fn parse_repo_slug_ssh() {
    assert_eq!(
        parse_repo_slug("git@github.com:org/repo.git"),
        Some("org/repo".to_string()),
    );
}

#[test]
fn parse_repo_slug_https() {
    assert_eq!(
        parse_repo_slug("https://github.com/org/repo.git"),
        Some("org/repo".to_string()),
    );
}

#[test]
fn parse_repo_slug_no_git_suffix() {
    assert_eq!(
        parse_repo_slug("https://github.com/org/repo"),
        Some("org/repo".to_string()),
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
fn create_pr_uses_explicit_base_branch_not_auto_detected() {
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // git push
        MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"), // git remote get-url
        MockProcessRunner::ok_with_stdout(b"https://github.com/org/repo/pull/1\n"), // gh pr create
    ]);

    create_pr("/repo", "42-fix-bug", "Fix bug", "desc", "develop", &mock).unwrap();

    let calls = mock.recorded_calls();
    // No symbolic-ref call
    assert!(
        !calls
            .iter()
            .any(|c| c.0 == "git" && c.1.iter().any(|a| a == "symbolic-ref")),
        "symbolic-ref must not be called when base_branch is explicit"
    );
    let gh_call = calls.iter().find(|c| c.0 == "gh").unwrap();
    assert!(
        gh_call.1.contains(&"develop".to_string()),
        "gh pr create should use explicit base_branch"
    );
}

#[test]
fn create_pr_pushes_from_push_dir_not_repo_root() {
    // push_dir (the worktree) is used for git push so the pre-push hook runs in the
    // worktree's working directory where main-repo dirty files are invisible.
    let mock = MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // git push
        MockProcessRunner::ok_with_stdout(b"git@github.com:org/repo.git\n"), // git remote get-url
        MockProcessRunner::ok_with_stdout(b"https://github.com/org/repo/pull/1\n"), // gh pr create
    ]);

    create_pr(
        "/repo/.worktrees/42-fix-bug",
        "42-fix-bug",
        "Fix bug",
        "desc",
        "main",
        &mock,
    )
    .unwrap();

    let calls = mock.recorded_calls();
    let push_call = calls
        .iter()
        .find(|c| c.1.contains(&"push".to_string()))
        .unwrap();
    assert!(
        push_call
            .1
            .contains(&"/repo/.worktrees/42-fix-bug".to_string()),
        "git push must use push_dir (worktree), got: {:?}",
        push_call.1
    );
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
    dispatch_agent(&task, &mock, None, &[]).unwrap();

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
    dispatch_agent(&task, &mock, None, &[]).unwrap();

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
