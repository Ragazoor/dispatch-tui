use super::*;

#[test]
fn render_validated_knowledge_block_omits_when_empty() {
    assert_eq!(render_validated_knowledge_block(&[]), String::new());
}

#[test]
fn render_validated_knowledge_block_formats_entries() {
    let l = seed(7, LearningScope::Epic, 3);
    let out = render_validated_knowledge_block(&[&l]);
    assert!(out.contains("## Validated knowledge for this task"));
    assert!(out.contains("[#7 epic, \u{2191}3]"));
    assert!(out.contains("learning 7"));
    assert!(
        out.contains("rate_learning"),
        "validated-knowledge block should instruct rate_learning, got: {out}"
    );
    assert!(
        !out.contains("learning_verdicts"),
        "validated-knowledge block should no longer reference wrap_up verdicts, got: {out}"
    );
}

#[test]
fn build_prompt_default_injections_unchanged() {
    // Regression: when no learnings are injected the prompt must not gain
    // any leading whitespace or knowledge-block headers.
    let text = build_prompt(
        TaskId(1),
        "title",
        "desc",
        None,
        None,
        &PromptContext::default(),
    );
    assert!(text.starts_with("Your task is:"));
    assert!(!text.contains("Validated knowledge for this task"));
}

#[test]
fn build_prompt_with_injections_includes_knowledge_block() {
    let procedural_l = {
        let mut l = seed(10, LearningScope::User, 0);
        l.kind = LearningKind::Procedural;
        l.detail = Some("Always run tests before committing.".into());
        l
    };
    let convention_l = seed(11, LearningScope::Repo, 2);
    let injections = LearningInjections {
        ranked: vec![&procedural_l, &convention_l],
    };
    let ctx = PromptContext {
        learnings: injections,
        ..PromptContext::default()
    };
    let text = build_prompt(TaskId(1), "title", "desc", None, None, &ctx);
    // Procedural learnings no longer appear as a verbatim prefix — prompt
    // always starts with the task block.
    assert!(text.starts_with("Your task is:"));
    assert!(text.contains("## Validated knowledge for this task"));
    // Both learnings appear in the validated-knowledge block.
    assert!(text.contains("[#10 user, \u{2191}0]"));
    assert!(text.contains("[#11 repo, \u{2191}2]"));
}

#[test]
fn build_quick_dispatch_prompt_default_injections_unchanged() {
    let text =
        build_quick_dispatch_prompt(TaskId(1), "title", "desc", None, &PromptContext::default());
    assert!(text.starts_with("You are working interactively with the user."));
    assert!(!text.contains("Validated knowledge for this task"));
}
