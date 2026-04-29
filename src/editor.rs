#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorSection {
    Title,
    Description,
    Plan,
}

pub struct EditorFields {
    pub title: String,
    pub description: String,
    pub repo_path: String,
    pub status: String,
    pub plan: String,
    pub tag: String,
    pub base_branch: String,
}

use crate::models::{Epic, Learning, LearningKind, Task};
use crate::service::FieldUpdate;

pub struct EpicEditorFields {
    pub title: String,
    pub description: String,
    pub repo_path: String,
    pub feed_command: String,       // "" → Clear, non-empty → Set
    pub feed_interval_secs: String, // "" or non-integer → None (don't touch)
}

/// Parse `--- SECTION ---` delimited text into a map of section name → content.
fn parse_sections(input: &str) -> std::collections::HashMap<&str, String> {
    let mut sections = std::collections::HashMap::new();
    let mut current_section: Option<&str> = None;
    let mut current_buf = String::new();

    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("--- ") && trimmed.ends_with(" ---") {
            if let Some(name) = current_section {
                sections.insert(name, current_buf.trim().to_string());
            }
            let section = trimmed.trim_start_matches("--- ").trim_end_matches(" ---");
            current_section = Some(section);
            current_buf = String::new();
            continue;
        }
        if current_section.is_some() {
            if !current_buf.is_empty() {
                current_buf.push('\n');
            }
            current_buf.push_str(line);
        }
    }
    if let Some(name) = current_section {
        sections.insert(name, current_buf.trim().to_string());
    }
    sections
}

pub fn format_description_for_editor(existing: &str) -> String {
    format!("--- DESCRIPTION ---\n{existing}\n")
}

pub fn parse_description_editor_output(input: &str) -> String {
    let mut s = parse_sections(input);
    s.remove("DESCRIPTION").unwrap_or_default()
}

pub fn format_epic_for_editor(epic: &Epic) -> String {
    let feed_cmd = epic.feed_command.as_deref().unwrap_or("");
    let feed_interval = epic
        .feed_interval_secs
        .map(|n| n.to_string())
        .unwrap_or_default();
    format!(
        "--- TITLE ---\n{}\n--- DESCRIPTION ---\n{}\n--- REPO_PATH ---\n{}\n--- FEED_COMMAND ---\n{}\n--- FEED_INTERVAL_SECS ---\n{}\n",
        epic.title, epic.description, epic.repo_path, feed_cmd, feed_interval
    )
}

pub fn parse_epic_editor_output(input: &str) -> EpicEditorFields {
    let mut s = parse_sections(input);
    EpicEditorFields {
        title: s.remove("TITLE").unwrap_or_default(),
        description: s.remove("DESCRIPTION").unwrap_or_default(),
        repo_path: s.remove("REPO_PATH").unwrap_or_default(),
        feed_command: s.remove("FEED_COMMAND").unwrap_or_default(),
        feed_interval_secs: s.remove("FEED_INTERVAL_SECS").unwrap_or_default(),
    }
}

pub fn format_editor_content(task: &Task) -> String {
    let plan = task.plan_path.as_deref().unwrap_or("");
    let tag = task.tag.map(|t| t.as_str()).unwrap_or("");
    format!(
        "--- TITLE ---\n{}\n--- DESCRIPTION ---\n{}\n--- REPO_PATH ---\n{}\n--- STATUS ---\n{}\n--- PLAN ---\n{}\n--- TAG ---\n{}\n--- BASE_BRANCH ---\n{}\n",
        task.title, task.description, task.repo_path, task.status.as_str(), plan, tag, task.base_branch
    )
}

/// Result of merging editor output with an existing [`Task`]. Empty string
/// fields are replaced with the task's prior values; empty plan/tag fields
/// clear the field. Invalid status strings fall back to the task's prior
/// status. Empty base_branch preserves the prior value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskEditApplied {
    pub title: String,
    pub description: String,
    pub repo_path: String,
    pub status: crate::models::TaskStatus,
    pub plan_path: Option<String>,
    pub tag: Option<crate::models::TaskTag>,
    pub base_branch: Option<String>,
}

/// Apply parsed editor fields on top of the task's existing values using
/// the rules documented in `tasks.allium::EditTask`.
pub fn apply_task_editor_fields(task: &Task, fields: EditorFields) -> TaskEditApplied {
    let title = if fields.title.is_empty() {
        task.title.clone()
    } else {
        fields.title
    };
    let description = if fields.description.is_empty() {
        task.description.clone()
    } else {
        fields.description
    };
    let repo_path = if fields.repo_path.is_empty() {
        task.repo_path.clone()
    } else {
        fields.repo_path
    };
    let status = crate::models::TaskStatus::parse(&fields.status).unwrap_or(task.status);
    let plan_path = if fields.plan.is_empty() {
        None
    } else {
        Some(fields.plan)
    };
    let tag = if fields.tag.is_empty() {
        None
    } else {
        crate::models::TaskTag::parse(&fields.tag)
    };
    let base_branch = if fields.base_branch.is_empty() {
        None
    } else {
        Some(fields.base_branch)
    };
    TaskEditApplied {
        title,
        description,
        repo_path,
        status,
        plan_path,
        tag,
        base_branch,
    }
}

/// Result of merging editor output with an existing [`Epic`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpicEditApplied {
    pub title: String,
    pub description: String,
    pub repo_path: String,
    pub feed_command: FieldUpdate,
    pub feed_interval_secs: Option<i64>,
}

/// Apply parsed epic editor fields on top of the epic's existing values.
/// Empty fields preserve the prior value.
pub fn apply_epic_editor_fields(epic: &Epic, fields: EpicEditorFields) -> EpicEditApplied {
    EpicEditApplied {
        title: if fields.title.is_empty() {
            epic.title.clone()
        } else {
            fields.title
        },
        description: if fields.description.is_empty() {
            epic.description.clone()
        } else {
            fields.description
        },
        repo_path: if fields.repo_path.is_empty() {
            epic.repo_path.clone()
        } else {
            fields.repo_path
        },
        feed_command: if fields.feed_command.is_empty() {
            FieldUpdate::Clear
        } else {
            FieldUpdate::Set(fields.feed_command)
        },
        feed_interval_secs: fields.feed_interval_secs.parse::<i64>().ok(),
    }
}

pub fn parse_editor_content(input: &str) -> EditorFields {
    let mut s = parse_sections(input);
    EditorFields {
        title: s.remove("TITLE").unwrap_or_default(),
        description: s.remove("DESCRIPTION").unwrap_or_default(),
        repo_path: s.remove("REPO_PATH").unwrap_or_default(),
        status: s.remove("STATUS").unwrap_or_default(),
        plan: s.remove("PLAN").unwrap_or_default(),
        tag: s.remove("TAG").unwrap_or_default(),
        base_branch: s.remove("BASE_BRANCH").unwrap_or_default(),
    }
}

pub struct LearningEditorFields {
    pub summary: String,
    pub kind: Option<LearningKind>,
    /// `None` = section absent (don't change).
    /// `Some(None)` = section present but empty (clear to NULL).
    /// `Some(Some(v))` = section present with text (set value).
    pub detail: Option<Option<String>>,
    /// `None` = section absent (don't change). `Some(vec![])` = clear tags.
    pub tags: Option<Vec<String>>,
}

pub fn format_learning_for_editor(learning: &Learning) -> String {
    let tags = learning.tags.join(", ");
    let detail = learning.detail.as_deref().unwrap_or("");
    format!(
        "--- SUMMARY ---\n{}\n--- KIND ---\n{}\n--- TAGS ---\n{}\n--- DETAIL ---\n{}\n",
        learning.summary,
        learning.kind.as_str(),
        tags,
        detail,
    )
}

pub fn parse_learning_editor_output(input: &str) -> LearningEditorFields {
    let mut s = parse_sections(input);
    let summary = s.remove("SUMMARY").unwrap_or_default();
    let kind = s.remove("KIND").and_then(|k| LearningKind::parse(&k));
    let tags = s.remove("TAGS").map(|t| {
        if t.trim().is_empty() {
            vec![]
        } else {
            t.split(',')
                .map(|tag| tag.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect()
        }
    });
    let detail = s.remove("DETAIL").map(|d| {
        if d.trim().is_empty() {
            None
        } else {
            Some(d)
        }
    });
    LearningEditorFields { summary, kind, tags, detail }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EpicId, TaskId, TaskStatus};
    use chrono::Utc;
    use proptest::prelude::*;

    fn make_epic(title: &str, description: &str, repo_path: &str) -> Epic {
        Epic {
            id: EpicId(1),
            title: title.to_string(),
            description: description.to_string(),
            repo_path: repo_path.to_string(),
            status: TaskStatus::Backlog,
            plan_path: None,
            sort_order: None,
            auto_dispatch: true,
            parent_epic_id: None,
            feed_command: None,
            feed_interval_secs: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            project_id: 1,
        }
    }

    #[test]
    fn epic_editor_roundtrip_basic() {
        let epic = make_epic("My Epic", "A description", "/repo");
        let content = format_epic_for_editor(&epic);
        let fields = parse_epic_editor_output(&content);
        assert_eq!(fields.title, "My Epic");
        assert_eq!(fields.description, "A description");
        assert_eq!(fields.repo_path, "/repo");
    }

    #[test]
    fn epic_editor_roundtrip_multiline_description() {
        let epic = make_epic("Title", "Line 1\nLine 2\nLine 3", "/repo");
        let content = format_epic_for_editor(&epic);
        let fields = parse_epic_editor_output(&content);
        assert_eq!(fields.description, "Line 1\nLine 2\nLine 3");
    }

    #[test]
    fn epic_editor_roundtrip_colons_in_title() {
        let epic = make_epic("Fix: auth system", "desc", "/repo");
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
        assert_eq!(fields.repo_path, "");
    }

    fn make_task(
        title: &str,
        description: &str,
        repo_path: &str,
        status: TaskStatus,
        plan: Option<&str>,
    ) -> Task {
        Task {
            id: TaskId(1),
            title: title.to_string(),
            description: description.to_string(),
            repo_path: repo_path.to_string(),
            status,
            worktree: None,
            tmux_window: None,
            plan_path: plan.map(|s| s.to_string()),
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
        assert_eq!(fields.status, "backlog");
        assert_eq!(fields.plan, "docs/plan.md");
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
        task.base_branch = "develop".to_string();
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
        assert_eq!(fields.status, "backlog");
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
        t.base_branch = "develop".to_string();
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
        assert_eq!(applied.plan_path.as_deref(), task.plan_path.as_deref());
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
            title: String::new(),
            description: "New desc".into(),
            repo_path: String::new(),
            status: String::new(),
            plan: String::new(),
            tag: String::new(),
            base_branch: String::new(),
        };
        let applied = apply_task_editor_fields(&task, fields);
        assert_eq!(applied.title, "Original title");
        assert_eq!(applied.description, "New desc");
        // empty repo_path preserves prior
        assert_eq!(applied.repo_path, "/orig/repo");
    }

    #[test]
    fn apply_task_empty_plan_clears_plan() {
        let task = sample_task();
        assert!(task.plan_path.is_some());
        let fields = EditorFields {
            title: String::new(),
            description: String::new(),
            repo_path: String::new(),
            status: String::new(),
            plan: String::new(),
            tag: "bug".into(),
            base_branch: String::new(),
        };
        let applied = apply_task_editor_fields(&task, fields);
        assert!(applied.plan_path.is_none(), "empty plan should clear plan");
    }

    #[test]
    fn apply_task_empty_tag_clears_tag() {
        let task = sample_task();
        let fields = EditorFields {
            title: String::new(),
            description: String::new(),
            repo_path: String::new(),
            status: String::new(),
            plan: String::new(),
            tag: String::new(),
            base_branch: String::new(),
        };
        let applied = apply_task_editor_fields(&task, fields);
        assert!(applied.tag.is_none());
    }

    #[test]
    fn apply_task_invalid_status_preserves_prior() {
        let task = sample_task();
        let fields = EditorFields {
            title: String::new(),
            description: String::new(),
            repo_path: String::new(),
            status: "nonsense".into(),
            plan: String::new(),
            tag: String::new(),
            base_branch: String::new(),
        };
        let applied = apply_task_editor_fields(&task, fields);
        assert_eq!(applied.status, task.status);
    }

    #[test]
    fn apply_task_invalid_tag_clears_tag() {
        let task = sample_task();
        let fields = EditorFields {
            title: String::new(),
            description: String::new(),
            repo_path: String::new(),
            status: String::new(),
            plan: String::new(),
            tag: "not-a-real-tag".into(),
            base_branch: String::new(),
        };
        let applied = apply_task_editor_fields(&task, fields);
        // parse returns None → applied.tag = None. Documented behaviour.
        assert!(applied.tag.is_none());
    }

    #[test]
    fn apply_task_empty_base_branch_preserves_prior() {
        let task = sample_task();
        let fields = EditorFields {
            title: String::new(),
            description: String::new(),
            repo_path: String::new(),
            status: String::new(),
            plan: String::new(),
            tag: String::new(),
            base_branch: String::new(),
        };
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
        assert_eq!(applied.plan_path, task.plan_path);
        assert_eq!(applied.tag, task.tag);
    }

    // --- apply_epic_editor_fields -----------------------------------------

    #[test]
    fn apply_epic_editor_fields_roundtrip() {
        let epic = make_epic("E title", "E desc", "/repo");
        let content = format_epic_for_editor(&epic);
        let fields = parse_epic_editor_output(&content);
        let applied = apply_epic_editor_fields(&epic, fields);
        assert_eq!(applied.title, epic.title);
        assert_eq!(applied.description, epic.description);
        assert_eq!(applied.repo_path, epic.repo_path);
        assert_eq!(applied.feed_command, FieldUpdate::Clear);
        assert_eq!(applied.feed_interval_secs, None);
    }

    #[test]
    fn apply_epic_empty_fields_preserve_prior() {
        let epic = make_epic("E title", "E desc", "/repo");
        let fields = EpicEditorFields {
            title: String::new(),
            description: "new desc".into(),
            repo_path: String::new(),
            feed_command: String::new(),
            feed_interval_secs: String::new(),
        };
        let applied = apply_epic_editor_fields(&epic, fields);
        assert_eq!(applied.title, "E title");
        assert_eq!(applied.description, "new desc");
        assert_eq!(applied.repo_path, "/repo");
        assert_eq!(applied.feed_command, FieldUpdate::Clear);
        assert_eq!(applied.feed_interval_secs, None);
    }

    #[test]
    fn epic_editor_includes_feed_command_section() {
        let mut epic = make_epic("T", "D", "/repo");
        epic.feed_command = Some("scripts/fetch-dependabot.sh".into());
        let content = format_epic_for_editor(&epic);
        assert!(content.contains("--- FEED_COMMAND ---"));
        assert!(content.contains("scripts/fetch-dependabot.sh"));
    }

    #[test]
    fn epic_editor_includes_feed_interval_section() {
        let mut epic = make_epic("T", "D", "/repo");
        epic.feed_interval_secs = Some(300);
        let content = format_epic_for_editor(&epic);
        assert!(content.contains("--- FEED_INTERVAL_SECS ---"));
        assert!(content.contains("300"));
    }

    #[test]
    fn epic_editor_roundtrip_feed_command_set() {
        let mut epic = make_epic("T", "D", "/repo");
        epic.feed_command = Some("my-script.sh".into());
        let content = format_epic_for_editor(&epic);
        let fields = parse_epic_editor_output(&content);
        assert_eq!(fields.feed_command, "my-script.sh");
    }

    #[test]
    fn epic_editor_roundtrip_feed_command_empty() {
        let epic = make_epic("T", "D", "/repo");
        let content = format_epic_for_editor(&epic);
        let fields = parse_epic_editor_output(&content);
        assert_eq!(fields.feed_command, "");
    }

    #[test]
    fn epic_editor_roundtrip_feed_interval_set() {
        let mut epic = make_epic("T", "D", "/repo");
        epic.feed_interval_secs = Some(120);
        let content = format_epic_for_editor(&epic);
        let fields = parse_epic_editor_output(&content);
        assert_eq!(fields.feed_interval_secs, "120");
    }

    #[test]
    fn apply_epic_feed_command_set() {
        let epic = make_epic("T", "D", "/repo");
        let fields = EpicEditorFields {
            title: "T".into(),
            description: "D".into(),
            repo_path: "/repo".into(),
            feed_command: "my-script.sh".into(),
            feed_interval_secs: "".into(),
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
        let mut epic = make_epic("T", "D", "/repo");
        epic.feed_command = Some("old-script.sh".into());
        let fields = EpicEditorFields {
            title: "T".into(),
            description: "D".into(),
            repo_path: "/repo".into(),
            feed_command: "".into(),
            feed_interval_secs: "".into(),
        };
        let applied = apply_epic_editor_fields(&epic, fields);
        assert_eq!(applied.feed_command, crate::service::FieldUpdate::Clear);
    }

    #[test]
    fn apply_epic_feed_interval_valid() {
        let epic = make_epic("T", "D", "/repo");
        let fields = EpicEditorFields {
            title: "T".into(),
            description: "D".into(),
            repo_path: "/repo".into(),
            feed_command: "".into(),
            feed_interval_secs: "300".into(),
        };
        let applied = apply_epic_editor_fields(&epic, fields);
        assert_eq!(applied.feed_interval_secs, Some(300));
    }

    #[test]
    fn apply_epic_feed_interval_invalid_preserves_none() {
        let epic = make_epic("T", "D", "/repo");
        let fields = EpicEditorFields {
            title: "T".into(),
            description: "D".into(),
            repo_path: "/repo".into(),
            feed_command: "".into(),
            feed_interval_secs: "not-a-number".into(),
        };
        let applied = apply_epic_editor_fields(&epic, fields);
        assert_eq!(applied.feed_interval_secs, None);
    }

    #[test]
    fn apply_epic_editor_fields_full_roundtrip() {
        let mut epic = make_epic("E title", "E desc", "/repo");
        epic.feed_command = Some("scripts/fetch-dependabot.sh".into());
        epic.feed_interval_secs = Some(60);
        let content = format_epic_for_editor(&epic);
        let fields = parse_epic_editor_output(&content);
        let applied = apply_epic_editor_fields(&epic, fields);
        assert_eq!(applied.title, "E title");
        assert_eq!(
            applied.feed_command,
            crate::service::FieldUpdate::Set("scripts/fetch-dependabot.sh".into())
        );
        assert_eq!(applied.feed_interval_secs, Some(60));
    }

    mod learning_editor_tests {
        use super::*;

        fn make_learning() -> Learning {
            use crate::models::{LearningScope, LearningStatus};
            Learning {
                id: 1,
                kind: LearningKind::Convention,
                summary: "Use LearningService not raw db".to_string(),
                detail: Some("Ensures validation runs.".to_string()),
                scope: LearningScope::Repo,
                scope_ref: Some("/repo".to_string()),
                tags: vec!["arch".to_string(), "service".to_string()],
                status: LearningStatus::Proposed,
                source_task_id: None,
                confirmed_count: 0,
                last_confirmed_at: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }
        }

        #[test]
        fn format_round_trips_all_fields() {
            let l = make_learning();
            let s = format_learning_for_editor(&l);
            let f = parse_learning_editor_output(&s);
            assert_eq!(f.summary, l.summary);
            assert_eq!(f.kind, Some(l.kind));
            assert_eq!(f.tags, Some(l.tags.clone()));
            assert_eq!(f.detail, Some(Some(l.detail.clone().unwrap())));
        }

        #[test]
        fn format_round_trips_no_detail() {
            let mut l = make_learning();
            l.detail = None;
            let s = format_learning_for_editor(&l);
            let f = parse_learning_editor_output(&s);
            // Empty DETAIL section → Some(None) meaning "clear"
            assert_eq!(f.detail, Some(None));
        }

        #[test]
        fn parse_empty_summary_returns_empty_string() {
            let input = "--- SUMMARY ---\n\n--- KIND ---\nconvention\n--- TAGS ---\n\n--- DETAIL ---\n\n";
            let f = parse_learning_editor_output(input);
            assert_eq!(f.summary, "");
        }

        #[test]
        fn parse_unknown_kind_returns_none() {
            let input = "--- SUMMARY ---\nfoo\n--- KIND ---\nnot_a_kind\n--- TAGS ---\n\n--- DETAIL ---\n\n";
            let f = parse_learning_editor_output(input);
            assert_eq!(f.kind, None);
        }

        #[test]
        fn parse_missing_sections_return_none() {
            let f = parse_learning_editor_output("--- SUMMARY ---\nsome text\n");
            assert_eq!(f.kind, None);
            assert_eq!(f.tags, None);
            assert_eq!(f.detail, None);
        }

        #[test]
        fn parse_tags_trims_whitespace() {
            let input = "--- SUMMARY ---\nfoo\n--- KIND ---\npitfall\n--- TAGS ---\n async , rust \n--- DETAIL ---\n\n";
            let f = parse_learning_editor_output(input);
            assert_eq!(f.tags, Some(vec!["async".to_string(), "rust".to_string()]));
        }
    }
}
