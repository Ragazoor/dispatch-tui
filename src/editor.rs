//! External `$EDITOR` integration: which editor the user wants, and the
//! structured file format the pop-out task/epic editors read back.

/// Editor of last resort when neither `$VISUAL` nor `$EDITOR` names one —
/// `config.editor_fallback` in docs/specs/core.allium.
pub const EDITOR_FALLBACK: &str = "vi";

/// Resolve the editor argv from environment *values*: `$VISUAL`, then
/// `$EDITOR`, then [`EDITOR_FALLBACK`]. Never returns an empty vector.
///
/// One resolver for every surface that launches an editor — today the board's
/// pop-out task/epic editor (`runtime::editor`) — because one `$EDITOR` must not
/// mean two things in one application (docs/specs/core.allium:
/// `editor_fallback`). The agent-tree companion pane used to be the second
/// such surface; it shows diffs now and launches no editor at all.
///
/// Takes the values as parameters rather than reading the process environment,
/// so the resolution order is testable without `std::env::set_var` — which is
/// `unsafe` in edition 2024 and races the test harness's threads either way.
/// [`editor_from_env`] is the one-line adapter that reads them.
///
/// A value is treated as unset when it is empty or all whitespace: `export
/// EDITOR=` is how a shell spells "no editor", and it would otherwise produce an
/// unrunnable empty argv. The value is split on whitespace into argv and
/// executed directly, never through a shell, so `EDITOR="vim -p"` works and
/// nothing in it is expanded, globbed or word-split by anything but this
/// function.
pub fn resolve_editor(visual: Option<&str>, editor: Option<&str>) -> Vec<String> {
    for candidate in [visual, editor] {
        let Some(value) = candidate else { continue };
        let argv: Vec<String> = value.split_whitespace().map(str::to_string).collect();
        if !argv.is_empty() {
            return argv;
        }
    }
    vec![EDITOR_FALLBACK.to_string()]
}

/// [`resolve_editor`] against the real process environment.
pub fn editor_from_env() -> Vec<String> {
    let visual = std::env::var("VISUAL").ok();
    let editor = std::env::var("EDITOR").ok();
    resolve_editor(visual.as_deref(), editor.as_deref())
}

/// A parse failure surfaced by [`parse_editor_content`] /
/// [`parse_epic_editor_output`]. The runtime turns these into a status message
/// so the user knows their input was rejected rather than silently dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorParseError {
    /// The section name as it appears in the editor file (e.g. `"STATUS"`).
    pub field: &'static str,
    /// The raw user-typed value that failed to parse.
    pub raw: String,
    /// Human-readable explanation suitable for a status bar message.
    pub message: String,
}

impl std::fmt::Display for EditorParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

#[derive(Default)]
pub struct EditorFields {
    pub title: String,
    pub description: String,
    pub repo_path: String,
    /// Parsed task status, or `None` if the section was empty or its content
    /// failed to parse. Parse failures are recorded in `errors`.
    pub status: Option<crate::models::TaskStatus>,
    pub plan: String,
    /// Parsed tag, or `None` if the section was empty or unparseable. Parse
    /// failures are recorded in `errors`.
    pub tag: Option<crate::models::TaskTag>,
    pub base_branch: String,
    /// Parsed wrap-up mode, or `None` if the section was empty/absent or
    /// unparseable. `None` is applied as "clear" in `apply_task_editor_fields`.
    pub wrap_up_mode: Option<crate::models::WrapUpMode>,
    /// Raw URL string. Empty means "clear the url".
    pub url: String,
    /// Parsed url_type, or `None` if the section was empty/absent or
    /// unparseable (parse failures are recorded in `errors`). `None` means
    /// "infer from the url, or preserve the prior type if the url is unchanged"
    /// at apply time.
    pub url_type: Option<crate::models::UrlType>,
    /// Parsed phoenix flag, or `None` if the section was empty/absent or
    /// unparseable (parse failures are recorded in `errors`). `None` is applied
    /// as `false` in `apply_task_editor_fields` — the same "clear it" the TAG
    /// and WRAP_UP_MODE sections mean.
    pub phoenix: Option<bool>,
    pub errors: Vec<EditorParseError>,
}

use crate::models::{Epic, Task};
use crate::service::FieldUpdate;

#[derive(Default)]
pub struct EpicEditorFields {
    pub title: String,
    pub description: String,
    pub feed_command: String, // "" → Clear, non-empty → Set
    /// Parsed seconds, or `None` if the section was empty or unparseable.
    /// Parse failures are recorded in `errors`.
    pub feed_interval_secs: Option<i64>,
    pub errors: Vec<EditorParseError>,
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
        "--- TITLE ---\n{}\n--- DESCRIPTION ---\n{}\n--- FEED_COMMAND ---\n{}\n--- FEED_INTERVAL_SECS ---\n{}\n",
        epic.title, epic.description, feed_cmd, feed_interval
    )
}

/// Parse a section's raw value with `parser`. Empty input is treated as
/// "section absent" and returns `None` without an error. A non-empty value
/// that fails to parse pushes an [`EditorParseError`] onto `errors` and
/// returns `None`.
fn parse_section<T>(
    raw: String,
    field: &'static str,
    parser: impl FnOnce(&str) -> Option<T>,
    on_fail_message: impl FnOnce(&str) -> String,
    errors: &mut Vec<EditorParseError>,
) -> Option<T> {
    if raw.is_empty() {
        return None;
    }
    match parser(&raw) {
        Some(v) => Some(v),
        None => {
            let message = on_fail_message(&raw);
            errors.push(EditorParseError {
                field,
                raw,
                message,
            });
            None
        }
    }
}

/// The message an interval section shows when its content fails
/// [`crate::models::parse_interval_secs`]. The examples come from
/// [`crate::models::INTERVAL_EXAMPLES`] rather than being retyped, so they
/// cannot drift from the parser.
fn interval_parse_failure_message(raw: &str) -> String {
    format!(
        "not a valid interval: {raw:?} (expected e.g. {})",
        crate::models::INTERVAL_EXAMPLES
    )
}

pub fn parse_epic_editor_output(input: &str) -> EpicEditorFields {
    let mut s = parse_sections(input);
    let mut errors = Vec::new();
    let feed_interval_secs = parse_section(
        s.remove("FEED_INTERVAL_SECS").unwrap_or_default(),
        "FEED_INTERVAL_SECS",
        crate::models::parse_interval_secs,
        interval_parse_failure_message,
        &mut errors,
    );
    EpicEditorFields {
        title: s.remove("TITLE").unwrap_or_default(),
        description: s.remove("DESCRIPTION").unwrap_or_default(),
        feed_command: s.remove("FEED_COMMAND").unwrap_or_default(),
        feed_interval_secs,
        errors,
    }
}

pub fn format_editor_content(task: &Task) -> String {
    let plan = task.plan_path.as_deref().unwrap_or("");
    let tag = task.tag.map(|t| t.as_str()).unwrap_or("");
    let wrap_up_mode = task.wrap_up_mode.map(|m| m.as_str()).unwrap_or("");
    let url = task.url.as_ref().map(|u| u.url.as_str()).unwrap_or("");
    let url_type = task.url.as_ref().map(|u| u.url_type.as_str()).unwrap_or("");
    let phoenix = if task.phoenix { "true" } else { "false" };
    format!(
        "--- TITLE ---\n{title}\n\
         --- DESCRIPTION ---\n{description}\n\
         --- REPO_PATH ---\n{repo_path}\n\
         --- STATUS ---\n{status}\n\
         --- PLAN ---\n{plan}\n\
         --- TAG ---\n{tag}\n\
         --- BASE_BRANCH ---\n{base_branch}\n\
         --- WRAP_UP_MODE ---\n{wrap_up_mode}\n\
         --- PHOENIX ---\n{phoenix}\n\
         --- URL ---\n{url}\n\
         --- URL_TYPE ---\n{url_type}\n",
        title = task.title,
        description = task.description,
        repo_path = task.repo_path,
        status = task.status.as_str(),
        base_branch = task.base_branch,
    )
}

/// Result of merging editor output with an existing [`Task`].
///
/// The "absent" convention is encoded in the field *type* rather than in
/// scattered empty-string checks, so each field's intent is explicit:
///
/// - **Non-nullable, keep-prior** (`title`, `description`, `repo_path`):
///   plain `String` already resolved against the prior value — an empty
///   section yields the task's prior value.
/// - **`status`**: a resolved [`TaskStatus`](crate::models::TaskStatus);
///   invalid/empty input falls back to the prior status.
/// - **`base_branch`**: `Option<String>` where `None` = keep the prior value
///   (the column is non-nullable, so it is never cleared from the editor).
/// - **Clearable fields** (`plan_path`, `tag`, `wrap_up_mode`): the editor
///   always states a definite intent (the section is always present), so an
///   empty section means *clear* and a filled section means *set*. `plan_path`
///   uses [`FieldUpdate`] (`Set`/`Clear`); the rest use `Option` where
///   `None` = clear.
/// - **`url`**: `Option<`[`UrlUpdate`](crate::service::UrlUpdate)`>` — `None`
///   leaves the field untouched (the edited url equals the prior url, a no-op);
///   `Some(Set/Clear)` is forwarded to the service only when it differs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskEditApplied {
    pub title: String,
    pub description: String,
    pub repo_path: String,
    pub status: crate::models::TaskStatus,
    pub plan_path: FieldUpdate,
    /// Post-edit plan path resolved once here, so callers building an
    /// in-memory snapshot (e.g. `finalize_task_edit`) consume it directly
    /// rather than re-deriving it from `plan_path`.
    pub resolved_plan_path: Option<String>,
    pub tag: Option<crate::models::TaskTag>,
    pub base_branch: Option<String>,
    pub wrap_up_mode: Option<crate::models::WrapUpMode>,
    pub url: Option<crate::service::UrlUpdate>,
    /// Post-edit url resolved once here (the value `url` diffs against the
    /// prior task to decide whether a DB write is needed), so callers get
    /// the true post-edit value even when `url` itself is `None` (no-op diff).
    pub resolved_url: Option<crate::models::TaskUrl>,
    pub phoenix: bool,
}

/// Resolve the desired `Option<TaskUrl>` from the parsed URL string and
/// (already-parsed) url_type, given the task's prior url. An empty url clears
/// the field. A present url with no explicit type preserves the prior type when
/// the url is unchanged (so a `security_alert` is never downgraded — `infer`
/// can only yield Pr/Issue/Other), otherwise infers the type from the url.
fn resolve_edited_url(
    raw_url: &str,
    explicit_type: Option<crate::models::UrlType>,
    prior: Option<&crate::models::TaskUrl>,
) -> Option<crate::models::TaskUrl> {
    use crate::models::{TaskUrl, UrlType};

    if raw_url.is_empty() {
        return None;
    }

    let url_type = explicit_type.unwrap_or_else(|| match prior {
        Some(p) if p.url == raw_url => p.url_type,
        _ => UrlType::infer(raw_url),
    });

    Some(TaskUrl::new(raw_url.to_string(), url_type))
}

/// Return `edited` unless it is empty, in which case fall back to `prior`.
/// The keep-prior convention for the non-nullable string fields.
fn keep_prior_if_empty(edited: String, prior: &str) -> String {
    if edited.is_empty() {
        prior.to_string()
    } else {
        edited
    }
}

/// Apply parsed editor fields on top of the task's existing values using
/// the rules documented in `tasks.allium::EditTask`.
pub fn apply_task_editor_fields(task: &Task, fields: EditorFields) -> TaskEditApplied {
    // Non-nullable keep-prior fields: an empty section restores the prior value.
    let title = keep_prior_if_empty(fields.title, &task.title);
    let description = keep_prior_if_empty(fields.description, &task.description);
    let repo_path = keep_prior_if_empty(fields.repo_path, &task.repo_path);
    // None covers both empty-section and unparseable input — both fall back
    // to the prior value. Unparseable input is also surfaced via
    // `fields.errors` for the runtime to render as a status message.
    let status = fields.status.unwrap_or(task.status);
    // Clearable plan: empty section clears, filled section sets.
    let plan_path = FieldUpdate::from_string(fields.plan);
    // Clearable tag/wrap_up_mode: `None` (empty/unparseable section) clears.
    let tag = fields.tag;
    let wrap_up_mode = fields.wrap_up_mode;
    // base_branch is non-nullable: empty preserves the prior value (`None`
    // tells the service not to touch the column).
    let base_branch = if fields.base_branch.is_empty() {
        None
    } else {
        Some(fields.base_branch)
    };
    // Diff the desired url against the prior so an unchanged edit is a true
    // no-op (no spurious write, no `was_pr_finalisation` read).
    let desired_url = resolve_edited_url(&fields.url, fields.url_type, task.url.as_ref());
    let url = if desired_url == task.url {
        None
    } else if let Some(ref u) = desired_url {
        Some(crate::service::UrlUpdate::Set(u.clone()))
    } else {
        Some(crate::service::UrlUpdate::Clear)
    };
    // Non-nullable boolean: `None` (empty/unparseable section) means false, the
    // same "clear it" tag and wrap_up_mode take from a missing section.
    let phoenix = fields.phoenix.unwrap_or(false);
    let resolved_plan_path = plan_path.as_option().map(str::to_string);
    TaskEditApplied {
        title,
        description,
        repo_path,
        status,
        plan_path,
        resolved_plan_path,
        tag,
        base_branch,
        wrap_up_mode,
        url,
        resolved_url: desired_url,
        phoenix,
    }
}

/// Result of merging editor output with an existing [`Epic`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpicEditApplied {
    pub title: String,
    pub description: String,
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
        feed_command: FieldUpdate::from_string(fields.feed_command),
        feed_interval_secs: fields.feed_interval_secs,
    }
}

pub fn parse_editor_content(input: &str) -> EditorFields {
    let mut s = parse_sections(input);
    let mut errors = Vec::new();

    let status = parse_section(
        s.remove("STATUS").unwrap_or_default(),
        "STATUS",
        crate::models::TaskStatus::parse,
        |raw| format!("unknown status: {raw:?}"),
        &mut errors,
    );

    let tag = parse_section(
        s.remove("TAG").unwrap_or_default(),
        "TAG",
        crate::models::TaskTag::parse,
        |raw| format!("unknown tag: {raw:?}"),
        &mut errors,
    );

    let wrap_up_mode = parse_section(
        s.remove("WRAP_UP_MODE").unwrap_or_default(),
        "WRAP_UP_MODE",
        crate::models::WrapUpMode::parse,
        |raw| format!("unknown wrap-up mode: {raw:?} (valid: rebase, pr, done)"),
        &mut errors,
    );

    let url_type = parse_section(
        s.remove("URL_TYPE").unwrap_or_default(),
        "URL_TYPE",
        crate::models::UrlType::parse,
        |raw| format!("unknown url type: {raw:?} (valid: pr, security_alert, issue, other)"),
        &mut errors,
    );

    let phoenix = parse_section(
        s.remove("PHOENIX").unwrap_or_default(),
        "PHOENIX",
        parse_editor_bool,
        |raw| format!("not a yes/no value: {raw:?} (valid: true, false, yes, no, on, off, 1, 0)"),
        &mut errors,
    );

    EditorFields {
        title: s.remove("TITLE").unwrap_or_default(),
        description: s.remove("DESCRIPTION").unwrap_or_default(),
        repo_path: s.remove("REPO_PATH").unwrap_or_default(),
        status,
        plan: s.remove("PLAN").unwrap_or_default(),
        tag,
        base_branch: s.remove("BASE_BRANCH").unwrap_or_default(),
        wrap_up_mode,
        url: s.remove("URL").unwrap_or_default(),
        url_type,
        phoenix,
        errors,
    }
}

/// The PHOENIX section's value grammar.
///
/// Deliberately wider than `true`/`false`. The section shares the enum-ish
/// sections' one parse rule — empty and unparseable are treated identically, so
/// an unparseable value CLEARS the flag — and accepting every spelling a human
/// would reach for is what stops that rule quietly ending a recurrence. See
/// EditTask in `docs/specs/tasks.allium`.
fn parse_editor_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
