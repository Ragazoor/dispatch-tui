use super::*;
use crate::models::{EpicBuilder, TaskBuilder, TaskStatus};
use proptest::prelude::*;

// --- resolve_editor ---

#[test]
fn visual_wins_over_editor() {
    assert_eq!(resolve_editor(Some("vim"), Some("nano")), vec!["vim"]);
}

#[test]
fn editor_is_used_when_visual_is_unset() {
    assert_eq!(resolve_editor(None, Some("nano")), vec!["nano"]);
}

#[test]
fn falls_back_to_vi_when_neither_is_set() {
    assert_eq!(resolve_editor(None, None), vec![EDITOR_FALLBACK]);
}

/// An exported-but-empty variable is how a shell spells "unset" in practice
/// (`export EDITOR=`), and an empty argv would be unrunnable.
#[test]
fn an_empty_value_counts_as_unset() {
    assert_eq!(resolve_editor(Some(""), Some("nano")), vec!["nano"]);
    assert_eq!(resolve_editor(Some(""), Some("")), vec![EDITOR_FALLBACK]);
    assert_eq!(resolve_editor(Some("   "), None), vec![EDITOR_FALLBACK]);
}

/// The value is argv, not a shell command: it is split on whitespace and
/// executed directly, so flags in `$EDITOR` work and nothing in it is
/// shell-interpreted.
#[test]
fn a_multi_word_value_splits_into_argv() {
    assert_eq!(resolve_editor(Some("vim -p"), None), vec!["vim", "-p"]);
    assert_eq!(
        resolve_editor(Some("  code   -w  "), None),
        vec!["code", "-w"]
    );
}

/// Whatever the ambient environment says, the resolver's contract holds:
/// callers pass the result straight to tmux, where an empty argv is an
/// error.
#[test]
fn editor_from_env_is_never_empty() {
    assert!(!editor_from_env().is_empty());
}

const FEED_INTERVAL_SLOW_SECS: i64 = 300;
const FEED_INTERVAL_MED_SECS: i64 = 120;
const FEED_INTERVAL_FAST_SECS: i64 = 60;

fn make_epic(title: &str, description: &str) -> Epic {
    EpicBuilder::new(1)
        .title(title.to_string())
        .description(description.to_string())
        .auto_dispatch(true)
        .build()
}

#[test]
fn epic_editor_roundtrip_basic() {
    let epic = make_epic("My Epic", "A description");
    let content = format_epic_for_editor(&epic);
    let fields = parse_epic_editor_output(&content);
    assert_eq!(fields.title, "My Epic");
    assert_eq!(fields.description, "A description");
}

#[test]
fn epic_editor_roundtrip_multiline_description() {
    let epic = make_epic("Title", "Line 1\nLine 2\nLine 3");
    let content = format_epic_for_editor(&epic);
    let fields = parse_epic_editor_output(&content);
    assert_eq!(fields.description, "Line 1\nLine 2\nLine 3");
}

#[test]
fn epic_editor_roundtrip_colons_in_title() {
    let epic = make_epic("Fix: auth system", "desc");
    let content = format_epic_for_editor(&epic);
    let fields = parse_epic_editor_output(&content);
    assert_eq!(fields.title, "Fix: auth system");
}

#[test]
fn epic_editor_unknown_section_ignored() {
    let input = "--- TITLE ---\nHello\n--- UNKNOWN ---\nStuff\n--- DESCRIPTION ---\nmy desc\n";
    let fields = parse_epic_editor_output(input);
    assert_eq!(fields.title, "Hello");
    assert_eq!(fields.description, "my desc");
}

#[test]
fn epic_editor_empty_input() {
    let fields = parse_epic_editor_output("");
    assert_eq!(fields.title, "");
    assert_eq!(fields.description, "");
}

fn make_task(
    title: &str,
    description: &str,
    repo_path: &str,
    status: TaskStatus,
    plan: Option<&str>,
) -> Task {
    TaskBuilder::new(1)
        .title(title)
        .description(description)
        .repo_path(repo_path)
        .status(status)
        .plan(plan)
        .build()
}

#[test]
fn editor_roundtrip_basic() {
    let task = make_task(
        "My Task",
        "A description",
        "/repo",
        TaskStatus::Backlog,
        Some("docs/plan.md"),
    );
    let content = format_editor_content(&task);
    let fields = parse_editor_content(&content);
    assert_eq!(fields.title, "My Task");
    assert_eq!(fields.description, "A description");
    assert_eq!(fields.repo_path, "/repo");
    assert_eq!(fields.status, Some(TaskStatus::Backlog));
    assert_eq!(fields.plan, "docs/plan.md");
}

#[test]
fn format_editor_content_maps_each_field_to_its_own_section() {
    // Regression guard for the positional-format fragility: each field
    // must land under its own section header, not a neighbour's. A
    // reordering mistake in the format! call would move a value to the
    // wrong section and this test would catch it directly (without going
    // through parse, which would silently "fix" a swap between
    // same-shaped string fields).
    let mut task = make_task(
        "field-title",
        "field-description",
        "field-repo-path",
        TaskStatus::Running,
        Some("field-plan"),
    );
    task.tag = Some(crate::models::TaskTag::Bug);
    task.base_branch = "field-base-branch".into();
    task.wrap_up_mode = Some(crate::models::WrapUpMode::Pr);
    task.url = Some(crate::models::TaskUrl::new(
        "field-url",
        crate::models::UrlType::Issue,
    ));

    let content = format_editor_content(&task);
    let sections = parse_sections(&content);

    assert_eq!(
        sections.get("TITLE").map(String::as_str),
        Some("field-title")
    );
    assert_eq!(
        sections.get("DESCRIPTION").map(String::as_str),
        Some("field-description")
    );
    assert_eq!(
        sections.get("REPO_PATH").map(String::as_str),
        Some("field-repo-path")
    );
    assert_eq!(sections.get("STATUS").map(String::as_str), Some("running"));
    assert_eq!(sections.get("PLAN").map(String::as_str), Some("field-plan"));
    assert_eq!(sections.get("TAG").map(String::as_str), Some("bug"));
    assert_eq!(
        sections.get("BASE_BRANCH").map(String::as_str),
        Some("field-base-branch")
    );
    assert_eq!(sections.get("WRAP_UP_MODE").map(String::as_str), Some("pr"));
    assert_eq!(sections.get("URL").map(String::as_str), Some("field-url"));
    assert_eq!(sections.get("URL_TYPE").map(String::as_str), Some("issue"));
}

#[test]
fn editor_includes_base_branch() {
    let task = make_task("T", "D", "/repo", TaskStatus::Backlog, None);
    let content = format_editor_content(&task);
    assert!(
        content.contains("--- BASE_BRANCH ---"),
        "should have BASE_BRANCH section"
    );
    assert!(content.contains("main"), "should contain the branch value");
}

#[test]
fn editor_roundtrip_base_branch() {
    let mut task = make_task("T", "D", "/repo", TaskStatus::Backlog, None);
    task.base_branch = "develop".into();
    let content = format_editor_content(&task);
    let fields = parse_editor_content(&content);
    assert_eq!(fields.base_branch, "develop");
}

#[test]
fn parse_base_branch_from_editor_output() {
    let input = "--- TITLE ---\nT\n--- BASE_BRANCH ---\nstaging\n";
    let fields = parse_editor_content(input);
    assert_eq!(fields.base_branch, "staging");
}

#[test]
fn parse_base_branch_missing_returns_empty() {
    let input = "--- TITLE ---\nT\n--- STATUS ---\nbacklog\n";
    let fields = parse_editor_content(input);
    assert_eq!(fields.base_branch, "");
}

#[test]
fn editor_roundtrip_colons_in_title() {
    let task = make_task("Fix: auth bug", "desc", "/repo", TaskStatus::Backlog, None);
    let content = format_editor_content(&task);
    let fields = parse_editor_content(&content);
    assert_eq!(fields.title, "Fix: auth bug");
}

#[test]
fn editor_roundtrip_colons_in_description() {
    let task = make_task(
        "Title",
        "Step 1: do this\nStep 2: do that",
        "/repo",
        TaskStatus::Backlog,
        None,
    );
    let content = format_editor_content(&task);
    let fields = parse_editor_content(&content);
    assert_eq!(fields.description, "Step 1: do this\nStep 2: do that");
}

#[test]
fn editor_multiline_description() {
    let task = make_task(
        "Title",
        "Line 1\nLine 2\nLine 3",
        "/repo",
        TaskStatus::Done,
        None,
    );
    let content = format_editor_content(&task);
    let fields = parse_editor_content(&content);
    assert_eq!(fields.description, "Line 1\nLine 2\nLine 3");
}

#[test]
fn editor_unknown_section_ignored() {
    let input = "--- TITLE ---\nHello\n--- UNKNOWN ---\nStuff\n--- STATUS ---\nbacklog\n";
    let fields = parse_editor_content(input);
    assert_eq!(fields.title, "Hello");
    assert_eq!(fields.status, Some(TaskStatus::Backlog));
}

#[test]
fn description_editor_roundtrip_empty() {
    let content = format_description_for_editor("");
    let result = parse_description_editor_output(&content);
    assert_eq!(result, "");
}

#[test]
fn description_editor_roundtrip_multiline() {
    let content = format_description_for_editor("Line 1\nLine 2\nLine 3");
    let result = parse_description_editor_output(&content);
    assert_eq!(result, "Line 1\nLine 2\nLine 3");
}

#[test]
fn description_editor_roundtrip_with_dashes() {
    let content = format_description_for_editor("Some --- dashes --- in text");
    let result = parse_description_editor_output(&content);
    assert_eq!(result, "Some --- dashes --- in text");
}

proptest! {
    #[test]
    fn parse_editor_content_never_panics(input in "\\PC{0,2000}") {
        // parse_editor_content should never panic on arbitrary input
        let _ = parse_editor_content(&input);
    }
}

// --- apply_task_editor_fields -----------------------------------------

fn sample_task() -> Task {
    let mut t = make_task(
        "Original title",
        "Original description",
        "/orig/repo",
        TaskStatus::Running,
        Some("docs/plan.md"),
    );
    t.tag = Some(crate::models::TaskTag::Bug);
    t.base_branch = "develop".into();
    t
}

#[test]
fn apply_task_editor_fields_roundtrip() {
    let task = sample_task();
    let content = format_editor_content(&task);
    let fields = parse_editor_content(&content);
    let applied = apply_task_editor_fields(&task, fields);
    assert_eq!(applied.title, task.title);
    assert_eq!(applied.description, task.description);
    assert_eq!(applied.repo_path, task.repo_path);
    assert_eq!(applied.status, task.status);
    assert_eq!(
        applied.plan_path,
        FieldUpdate::Set(task.plan_path.clone().unwrap())
    );
    assert_eq!(applied.tag, task.tag);
    assert_eq!(
        applied.base_branch.as_deref(),
        Some(task.base_branch.as_str())
    );
}

#[test]
fn apply_task_empty_title_preserves_prior() {
    let task = sample_task();
    let fields = EditorFields {
        description: "New desc".into(),
        ..Default::default()
    };
    let applied = apply_task_editor_fields(&task, fields);
    assert_eq!(applied.title, "Original title");
    assert_eq!(applied.description, "New desc");
    // empty repo_path preserves prior
    assert_eq!(applied.repo_path, "/orig/repo");
}

#[test]
fn editor_includes_wrap_up_mode_section() {
    let mut task = make_task("T", "D", "/repo", TaskStatus::Backlog, None);
    task.wrap_up_mode = Some(crate::models::WrapUpMode::Rebase);
    let content = format_editor_content(&task);
    assert!(
        content.contains("--- WRAP_UP_MODE ---"),
        "should have WRAP_UP_MODE section: {content}"
    );
    assert!(
        content.contains("rebase"),
        "should contain wrap-up mode value"
    );
}

#[test]
fn editor_roundtrip_wrap_up_mode() {
    let mut task = make_task("T", "D", "/repo", TaskStatus::Backlog, None);
    task.wrap_up_mode = Some(crate::models::WrapUpMode::Pr);
    let content = format_editor_content(&task);
    let fields = parse_editor_content(&content);
    assert_eq!(fields.wrap_up_mode, Some(crate::models::WrapUpMode::Pr));
}

#[test]
fn editor_wrap_up_mode_none_when_empty() {
    let input = "--- TITLE ---\nT\n--- WRAP_UP_MODE ---\n\n";
    let fields = parse_editor_content(input);
    assert_eq!(fields.wrap_up_mode, None);
}

#[test]
fn apply_task_editor_wrap_up_mode_set() {
    let task = make_task("T", "D", "/repo", TaskStatus::Backlog, None);
    let fields = EditorFields {
        title: "T".into(),
        wrap_up_mode: Some(crate::models::WrapUpMode::Done),
        ..Default::default()
    };
    let applied = apply_task_editor_fields(&task, fields);
    assert_eq!(applied.wrap_up_mode, Some(crate::models::WrapUpMode::Done));
}

#[test]
fn apply_task_editor_wrap_up_mode_clear() {
    let mut task = make_task("T", "D", "/repo", TaskStatus::Backlog, None);
    task.wrap_up_mode = Some(crate::models::WrapUpMode::Rebase);
    let fields = EditorFields {
        title: "T".into(),
        wrap_up_mode: None,
        ..Default::default()
    };
    let applied = apply_task_editor_fields(&task, fields);
    assert_eq!(
        applied.wrap_up_mode, None,
        "empty wrap_up_mode section should clear it"
    );
}

#[test]
fn apply_task_editor_fields_resolved_plan_path_matches_set() {
    let task = sample_task();
    let fields = EditorFields {
        plan: "docs/new-plan.md".into(),
        ..Default::default()
    };
    let applied = apply_task_editor_fields(&task, fields);
    assert_eq!(
        applied.plan_path,
        FieldUpdate::Set("docs/new-plan.md".into())
    );
    assert_eq!(
        applied.resolved_plan_path.as_deref(),
        Some("docs/new-plan.md")
    );
}

#[test]
fn apply_task_editor_fields_resolved_plan_path_none_when_cleared() {
    let task = sample_task();
    let fields = EditorFields::default();
    let applied = apply_task_editor_fields(&task, fields);
    assert_eq!(applied.plan_path, FieldUpdate::Clear);
    assert_eq!(applied.resolved_plan_path, None);
}

#[test]
fn apply_task_empty_plan_clears_plan() {
    let task = sample_task();
    assert!(task.plan_path.is_some());
    let fields = EditorFields {
        tag: Some(crate::models::TaskTag::Bug),
        ..Default::default()
    };
    let applied = apply_task_editor_fields(&task, fields);
    assert_eq!(
        applied.plan_path,
        FieldUpdate::Clear,
        "empty plan should clear plan"
    );
}

#[test]
fn apply_task_empty_tag_clears_tag() {
    let task = sample_task();
    let fields = EditorFields::default();
    let applied = apply_task_editor_fields(&task, fields);
    assert!(applied.tag.is_none());
}

#[test]
fn parse_invalid_status_records_error_and_preserves_prior_on_apply() {
    let input = "--- TITLE ---\nT\n--- STATUS ---\nnonsense\n";
    let parsed = parse_editor_content(input);
    assert!(parsed.status.is_none());
    assert_eq!(parsed.errors.len(), 1);
    assert_eq!(parsed.errors[0].field, "STATUS");
    assert!(parsed.errors[0].message.contains("nonsense"));

    let task = sample_task();
    let applied = apply_task_editor_fields(&task, parsed);
    assert_eq!(applied.status, task.status);
}

#[test]
fn parse_invalid_tag_records_error_and_clears_on_apply() {
    let input = "--- TITLE ---\nT\n--- TAG ---\nnot-a-real-tag\n";
    let parsed = parse_editor_content(input);
    assert!(parsed.tag.is_none());
    assert_eq!(parsed.errors.len(), 1);
    assert_eq!(parsed.errors[0].field, "TAG");

    let task = sample_task();
    let applied = apply_task_editor_fields(&task, parsed);
    assert!(applied.tag.is_none());
}

#[test]
fn parse_valid_input_has_no_errors() {
    let task = sample_task();
    let parsed = parse_editor_content(&format_editor_content(&task));
    assert!(parsed.errors.is_empty(), "got: {:?}", parsed.errors);
    assert_eq!(parsed.status, Some(task.status));
    assert_eq!(parsed.tag, task.tag);
}

#[test]
fn apply_task_empty_base_branch_preserves_prior() {
    let task = sample_task();
    let fields = EditorFields::default();
    let applied = apply_task_editor_fields(&task, fields);
    // When the field is "" we preserve by returning None so the DB
    // patch does not touch it. The runtime's merger then keeps the
    // prior value.
    assert!(applied.base_branch.is_none());
}

#[test]
fn apply_task_unchanged_content_yields_same_task() {
    // Regression guard: if a user opens the editor and closes without
    // changes, the applied result must equal the original task's values.
    let task = sample_task();
    let content = format_editor_content(&task);
    let fields = parse_editor_content(&content);
    let applied = apply_task_editor_fields(&task, fields);
    assert_eq!(applied.title, task.title);
    assert_eq!(applied.description, task.description);
    assert_eq!(applied.repo_path, task.repo_path);
    assert_eq!(applied.status, task.status);
    assert_eq!(
        applied.plan_path,
        FieldUpdate::Set(task.plan_path.clone().unwrap())
    );
    assert_eq!(applied.tag, task.tag);
    // No url on the task → no url change requested.
    assert_eq!(applied.url, None);
}

// --- url editing ------------------------------------------------------

use crate::models::{TaskUrl, UrlType};
use crate::service::UrlUpdate;

fn sample_task_with_url(url: TaskUrl) -> Task {
    let mut t = sample_task();
    t.url = Some(url);
    t
}

#[test]
fn editor_includes_url_sections() {
    let task = sample_task_with_url(TaskUrl::new("https://github.com/o/r/pull/9", UrlType::Pr));
    let content = format_editor_content(&task);
    assert!(content.contains("--- URL ---"), "{content}");
    assert!(content.contains("--- URL_TYPE ---"), "{content}");
    assert!(content.contains("https://github.com/o/r/pull/9"));
    assert!(content.contains("pr"));
}

#[test]
fn url_roundtrip_no_url_is_noop() {
    let task = sample_task();
    let fields = parse_editor_content(&format_editor_content(&task));
    let applied = apply_task_editor_fields(&task, fields);
    assert_eq!(applied.url, None);
}

#[test]
fn url_roundtrip_pr_is_noop() {
    let task = sample_task_with_url(TaskUrl::new("https://github.com/o/r/pull/42", UrlType::Pr));
    let fields = parse_editor_content(&format_editor_content(&task));
    let applied = apply_task_editor_fields(&task, fields);
    // Unchanged edit → no url update.
    assert_eq!(applied.url, None);
}

#[test]
fn url_roundtrip_security_alert_preserves_type() {
    // A security_alert url whose string contains neither /pull/ nor
    // /issues/ would infer to Other — round-trip must keep security_alert.
    let task = sample_task_with_url(TaskUrl::new(
        "https://github.com/o/r/security/dependabot/3",
        UrlType::SecurityAlert,
    ));
    let fields = parse_editor_content(&format_editor_content(&task));
    let applied = apply_task_editor_fields(&task, fields);
    assert_eq!(applied.url, None, "unchanged round-trip must be a no-op");
}

#[test]
fn url_security_alert_type_preserved_when_type_section_cleared() {
    // User blanks URL_TYPE but leaves the (unchanged) security-alert url.
    // The prior type must be preserved, not inferred down to Other.
    let url = TaskUrl::new(
        "https://github.com/o/r/security/dependabot/3",
        UrlType::SecurityAlert,
    );
    let task = sample_task_with_url(url.clone());
    let fields = EditorFields {
        url: url.url.clone(),
        url_type: None, // section cleared
        ..Default::default()
    };
    let applied = apply_task_editor_fields(&task, fields);
    // url unchanged → no-op, and the type stayed security_alert.
    assert_eq!(applied.url, None);
}

#[test]
fn url_empty_clears_existing_url() {
    let task = sample_task_with_url(TaskUrl::new("https://github.com/o/r/pull/1", UrlType::Pr));
    let fields = EditorFields {
        url: String::new(),
        ..Default::default()
    };
    let applied = apply_task_editor_fields(&task, fields);
    assert_eq!(applied.url, Some(UrlUpdate::Clear));
}

#[test]
fn url_empty_on_task_without_url_is_noop() {
    let task = sample_task();
    let fields = EditorFields {
        url: String::new(),
        ..Default::default()
    };
    let applied = apply_task_editor_fields(&task, fields);
    assert_eq!(applied.url, None);
}

#[test]
fn apply_task_editor_fields_resolved_url_reflects_new_value_even_when_unchanged() {
    // resolved_url must reflect the post-edit url regardless of whether
    // `url` itself is None (unchanged, no diff to forward to the DB).
    let task = sample_task_with_url(TaskUrl::new("https://github.com/o/r/pull/9", UrlType::Pr));
    let fields = parse_editor_content(&format_editor_content(&task));
    let applied = apply_task_editor_fields(&task, fields);
    assert_eq!(applied.url, None, "unchanged edit is a no-op diff");
    assert_eq!(
        applied.resolved_url,
        Some(TaskUrl::new("https://github.com/o/r/pull/9", UrlType::Pr)),
        "resolved_url must still carry the post-edit value"
    );
}

#[test]
fn apply_task_editor_fields_resolved_url_none_when_cleared() {
    let task = sample_task_with_url(TaskUrl::new("https://github.com/o/r/pull/1", UrlType::Pr));
    let fields = EditorFields {
        url: String::new(),
        ..Default::default()
    };
    let applied = apply_task_editor_fields(&task, fields);
    assert_eq!(applied.url, Some(UrlUpdate::Clear));
    assert_eq!(applied.resolved_url, None);
}

#[test]
fn url_new_with_empty_type_infers_pr() {
    let task = sample_task();
    let fields = EditorFields {
        url: "https://github.com/o/r/pull/7".into(),
        url_type: None,
        ..Default::default()
    };
    let applied = apply_task_editor_fields(&task, fields);
    assert_eq!(
        applied.url,
        Some(UrlUpdate::Set(TaskUrl::new(
            "https://github.com/o/r/pull/7",
            UrlType::Pr
        )))
    );
}

#[test]
fn url_new_with_empty_type_infers_issue() {
    let task = sample_task();
    let fields = EditorFields {
        url: "https://github.com/o/r/issues/7".into(),
        url_type: None,
        ..Default::default()
    };
    let applied = apply_task_editor_fields(&task, fields);
    assert_eq!(
        applied.url,
        Some(UrlUpdate::Set(TaskUrl::new(
            "https://github.com/o/r/issues/7",
            UrlType::Issue
        )))
    );
}

#[test]
fn url_explicit_type_wins() {
    let task = sample_task();
    let fields = EditorFields {
        url: "https://example.com/x".into(),
        url_type: Some(UrlType::SecurityAlert),
        ..Default::default()
    };
    let applied = apply_task_editor_fields(&task, fields);
    assert_eq!(
        applied.url,
        Some(UrlUpdate::Set(TaskUrl::new(
            "https://example.com/x",
            UrlType::SecurityAlert
        )))
    );
}

#[test]
fn parse_invalid_url_type_records_error_and_falls_back_to_infer() {
    let input = "--- URL ---\nhttps://github.com/o/r/pull/5\n--- URL_TYPE ---\nbogus\n";
    let parsed = parse_editor_content(input);
    assert!(parsed.url_type.is_none());
    assert_eq!(parsed.errors.len(), 1);
    assert_eq!(parsed.errors[0].field, "URL_TYPE");

    let task = sample_task();
    let applied = apply_task_editor_fields(&task, parsed);
    // url still applied, type inferred from the /pull/ path.
    assert_eq!(
        applied.url,
        Some(UrlUpdate::Set(TaskUrl::new(
            "https://github.com/o/r/pull/5",
            UrlType::Pr
        )))
    );
}

#[test]
fn url_changed_string_reinfers_type() {
    // Prior type was security_alert; user replaces the url with a /pull/
    // link and blanks the type → infer pr (prior type must NOT stick to a
    // different url).
    let task = sample_task_with_url(TaskUrl::new(
        "https://github.com/o/r/security/dependabot/3",
        UrlType::SecurityAlert,
    ));
    let fields = EditorFields {
        url: "https://github.com/o/r/pull/9".into(),
        url_type: None,
        ..Default::default()
    };
    let applied = apply_task_editor_fields(&task, fields);
    assert_eq!(
        applied.url,
        Some(UrlUpdate::Set(TaskUrl::new(
            "https://github.com/o/r/pull/9",
            UrlType::Pr
        )))
    );
}

/// `format_editor_content` and `parse_editor_content` each name every
/// section as a string literal, in two lists nothing structural ties
/// together. A section added to one and misspelled in the other compiles
/// fine and silently never round-trips — this is what catches that.
#[test]
fn every_formatted_section_is_a_section_the_parser_reads() {
    let task = make_task("T", "D", "/repo", TaskStatus::Backlog, None);
    let mut formatted: Vec<String> = parse_sections(&format_editor_content(&task))
        .keys()
        .map(|k| k.to_string())
        .collect();
    formatted.sort();
    assert_eq!(
        formatted,
        vec![
            "BASE_BRANCH",
            "DESCRIPTION",
            "PHOENIX",
            "PLAN",
            "REPO_PATH",
            "STATUS",
            "TAG",
            "TITLE",
            "URL",
            "URL_TYPE",
            "WRAP_UP_MODE",
        ],
        "the editor's section set changed — update parse_editor_content to match"
    );
}

// --- FEED_INTERVAL_SECS -----------------------------------------------

#[test]
fn epic_editor_feed_interval_accepts_a_suffixed_literal() {
    let input = "--- TITLE ---\nT\n--- FEED_INTERVAL_SECS ---\n10m\n";
    let parsed = parse_epic_editor_output(input);
    assert_eq!(parsed.feed_interval_secs, Some(600));
    assert!(parsed.errors.is_empty());
}

#[test]
fn epic_editor_feed_interval_still_reads_a_bare_integer_as_seconds() {
    let input = "--- TITLE ---\nT\n--- FEED_INTERVAL_SECS ---\n300\n";
    let parsed = parse_epic_editor_output(input);
    assert_eq!(parsed.feed_interval_secs, Some(300));
    assert!(parsed.errors.is_empty());
}

#[test]
fn epic_editor_feed_interval_rejects_zero() {
    let input = "--- TITLE ---\nT\n--- FEED_INTERVAL_SECS ---\n0\n";
    let parsed = parse_epic_editor_output(input);
    assert_eq!(parsed.feed_interval_secs, None);
    assert_eq!(parsed.errors.len(), 1);
    assert_eq!(parsed.errors[0].field, "FEED_INTERVAL_SECS");
}

#[test]
fn apply_epic_editor_fields_roundtrip() {
    let epic = make_epic("E title", "E desc");
    let content = format_epic_for_editor(&epic);
    let fields = parse_epic_editor_output(&content);
    let applied = apply_epic_editor_fields(&epic, fields);
    assert_eq!(applied.title, epic.title);
    assert_eq!(applied.description, epic.description);
    assert_eq!(applied.feed_command, FieldUpdate::Clear);
    assert_eq!(applied.feed_interval_secs, None);
}

#[test]
fn apply_epic_empty_fields_preserve_prior() {
    let epic = make_epic("E title", "E desc");
    let fields = EpicEditorFields {
        description: "new desc".into(),
        ..Default::default()
    };
    let applied = apply_epic_editor_fields(&epic, fields);
    assert_eq!(applied.title, "E title");
    assert_eq!(applied.description, "new desc");
    assert_eq!(applied.feed_command, FieldUpdate::Clear);
    assert_eq!(applied.feed_interval_secs, None);
}

#[test]
fn epic_editor_includes_feed_command_section() {
    let mut epic = make_epic("T", "D");
    epic.feed_command = Some("scripts/fetch-dependabot.sh".into());
    let content = format_epic_for_editor(&epic);
    assert!(content.contains("--- FEED_COMMAND ---"));
    assert!(content.contains("scripts/fetch-dependabot.sh"));
}

#[test]
fn epic_editor_includes_feed_interval_section() {
    let mut epic = make_epic("T", "D");
    epic.feed_interval_secs = Some(FEED_INTERVAL_SLOW_SECS);
    let content = format_epic_for_editor(&epic);
    assert!(content.contains("--- FEED_INTERVAL_SECS ---"));
    assert!(content.contains("300"));
}

#[test]
fn epic_editor_roundtrip_feed_command_set() {
    let mut epic = make_epic("T", "D");
    epic.feed_command = Some("my-script.sh".into());
    let content = format_epic_for_editor(&epic);
    let fields = parse_epic_editor_output(&content);
    assert_eq!(fields.feed_command, "my-script.sh");
}

#[test]
fn epic_editor_roundtrip_feed_command_empty() {
    let epic = make_epic("T", "D");
    let content = format_epic_for_editor(&epic);
    let fields = parse_epic_editor_output(&content);
    assert_eq!(fields.feed_command, "");
}

#[test]
fn epic_editor_roundtrip_feed_interval_set() {
    let mut epic = make_epic("T", "D");
    epic.feed_interval_secs = Some(FEED_INTERVAL_MED_SECS);
    let content = format_epic_for_editor(&epic);
    let fields = parse_epic_editor_output(&content);
    assert_eq!(fields.feed_interval_secs, Some(FEED_INTERVAL_MED_SECS));
    assert!(fields.errors.is_empty());
}

#[test]
fn parse_epic_invalid_feed_interval_records_error() {
    let input = "--- TITLE ---\nT\n--- FEED_INTERVAL_SECS ---\nnot-a-number\n";
    let parsed = parse_epic_editor_output(input);
    assert_eq!(parsed.feed_interval_secs, None);
    assert_eq!(parsed.errors.len(), 1);
    assert_eq!(parsed.errors[0].field, "FEED_INTERVAL_SECS");
}

#[test]
fn apply_epic_feed_command_set() {
    let epic = make_epic("T", "D");
    let fields = EpicEditorFields {
        title: "T".into(),
        description: "D".into(),
        feed_command: "my-script.sh".into(),
        feed_interval_secs: None,
        errors: Vec::new(),
    };
    let applied = apply_epic_editor_fields(&epic, fields);
    assert_eq!(
        applied.feed_command,
        crate::service::FieldUpdate::Set("my-script.sh".into())
    );
    assert_eq!(applied.feed_interval_secs, None);
}

#[test]
fn apply_epic_feed_command_clear() {
    let mut epic = make_epic("T", "D");
    epic.feed_command = Some("old-script.sh".into());
    let fields = EpicEditorFields {
        title: "T".into(),
        description: "D".into(),
        feed_command: String::new(),
        feed_interval_secs: None,
        errors: Vec::new(),
    };
    let applied = apply_epic_editor_fields(&epic, fields);
    assert_eq!(applied.feed_command, crate::service::FieldUpdate::Clear);
}

#[test]
fn apply_epic_feed_interval_valid() {
    let epic = make_epic("T", "D");
    let fields = EpicEditorFields {
        title: "T".into(),
        description: "D".into(),
        feed_command: String::new(),
        feed_interval_secs: Some(FEED_INTERVAL_SLOW_SECS),
        errors: Vec::new(),
    };
    let applied = apply_epic_editor_fields(&epic, fields);
    assert_eq!(applied.feed_interval_secs, Some(FEED_INTERVAL_SLOW_SECS));
}

#[test]
fn apply_epic_editor_fields_full_roundtrip() {
    let mut epic = make_epic("E title", "E desc");
    epic.feed_command = Some("scripts/fetch-dependabot.sh".into());
    epic.feed_interval_secs = Some(FEED_INTERVAL_FAST_SECS);
    let content = format_epic_for_editor(&epic);
    let fields = parse_epic_editor_output(&content);
    let applied = apply_epic_editor_fields(&epic, fields);
    assert_eq!(applied.title, "E title");
    assert_eq!(
        applied.feed_command,
        crate::service::FieldUpdate::Set("scripts/fetch-dependabot.sh".into())
    );
    assert_eq!(applied.feed_interval_secs, Some(FEED_INTERVAL_FAST_SECS));
}

fn task_with_phoenix(phoenix: bool) -> Task {
    let mut t = sample_task();
    t.phoenix = phoenix;
    t
}

fn parse_phoenix(section: &str) -> (Option<bool>, Vec<EditorParseError>) {
    let input = format!("--- TITLE ---\nT\n--- PHOENIX ---\n{section}\n");
    let fields = parse_editor_content(&input);
    (fields.phoenix, fields.errors)
}

#[test]
fn the_section_round_trips_the_flag() {
    assert!(format_editor_content(&task_with_phoenix(true)).contains("--- PHOENIX ---\ntrue"));
    assert!(format_editor_content(&task_with_phoenix(false)).contains("--- PHOENIX ---\nfalse"));
}

/// The wide spelling set is the mitigation for the clear-on-unparseable
/// rule below: it makes an accidentally cleared recurrence something a
/// human has to work at. See EditTask in docs/specs/tasks.allium.
#[test]
fn every_documented_spelling_parses_case_insensitively() {
    for raw in ["true", "TRUE", "True", "yes", "YES", "on", "1"] {
        assert_eq!(parse_phoenix(raw).0, Some(true), "{raw:?} should be true");
    }
    for raw in ["false", "FALSE", "no", "NO", "off", "0"] {
        assert_eq!(parse_phoenix(raw).0, Some(false), "{raw:?} should be false");
    }
}

/// PHOENIX joins the enum-ish sections under their one parse rule: empty
/// and unparseable are treated identically — no value — and unparseable
/// additionally records an error.
#[test]
fn an_empty_section_yields_no_value_and_no_error() {
    let (value, errors) = parse_phoenix("");
    assert_eq!(value, None);
    assert!(errors.is_empty());
}

#[test]
fn an_unparseable_section_yields_no_value_and_records_an_error() {
    let (value, errors) = parse_phoenix("maybe");
    assert_eq!(value, None);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].field, "PHOENIX");
}

/// "No value" means false for this field — the same "clear it" the TAG and
/// WRAP_UP_MODE sections mean.
#[test]
fn no_value_clears_the_flag() {
    let applied = apply_task_editor_fields(
        &task_with_phoenix(true),
        parse_editor_content("--- TITLE ---\nT\n--- PHOENIX ---\n\n"),
    );
    assert!(!applied.phoenix);
}

#[test]
fn a_true_section_arms_the_flag() {
    let applied = apply_task_editor_fields(
        &task_with_phoenix(false),
        parse_editor_content("--- TITLE ---\nT\n--- PHOENIX ---\nyes\n"),
    );
    assert!(applied.phoenix);
}
