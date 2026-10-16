use super::*;

use crate::setup::FakeConfirmer;
use serde_json::Value;

// -- Report section (ReportShippedFeedScripts) --

fn report(name: &str, outcome: ShippedScriptOutcome) -> ShippedScriptReport {
    ShippedScriptReport {
        name: name.to_string(),
        path: PathBuf::from("/data/scripts").join(name),
        outcome,
        backup_path: None,
        error: None,
    }
}

/// The slice of `content` that follows `anchor`, ending at the next Markdown
/// heading of any depth — so promoting or demoting a heading cannot silently
/// widen a scoped assertion to the rest of the document. `None` if `anchor`
/// is absent, letting each caller phrase its own "this copy is gone" panic.
fn section_after(content: &str, anchor: &str) -> Option<String> {
    let (_, section) = content.split_once(anchor)?;
    Some(
        section
            .split_once("\n#")
            .map_or(section, |(block, _)| block)
            .to_string(),
    )
}

/// Every embedded file under a skill's `references/` directory, as
/// `(file name, contents)`, in a stable order. Empty when the skill has no
/// references directory.
fn skill_references(skill: &str) -> Vec<(String, &'static str)> {
    let Some(dir) = PLUGIN_DIR.get_dir(format!("skills/{skill}/references")) else {
        return Vec::new();
    };
    let mut files: Vec<(String, &'static str)> = dir
        .files()
        .map(|f| {
            let name = f
                .path()
                .file_name()
                .expect("a reference file has a name")
                .to_string_lossy()
                .into_owned();
            let body = f
                .contents_utf8()
                .unwrap_or_else(|| panic!("{} must be UTF-8", f.path().display()));
            (name, body)
        })
        .collect();
    files.sort();
    files
}

/// The concatenated text of a skill's reference files — where branch
/// detail lives under the just-in-time skill-copy rule ("Skill copy" at
/// the head of docs/specs/mcp-task-tools.allium).
fn references_text(skill: &str) -> String {
    skill_references(skill)
        .into_iter()
        .map(|(_, body)| body)
        .collect::<Vec<_>>()
        .join("\n")
}

/// `SKILL.md` followed by every reference file: the whole copy an agent can
/// reach from the skill, for assertions about content that may live on
/// either side of the core-flow / branch-detail split.
fn skill_text(skill: &str) -> String {
    format!("{}\n{}", skill_body(skill), references_text(skill))
}

/// The reference file of `skill` whose contents include `anchor`, with the
/// name SKILL.md must cite it by. Panics naming `what` when none does.
fn reference_containing(skill: &str, anchor: &str, what: &str) -> (String, &'static str) {
    skill_references(skill)
        .into_iter()
        .find(|(_, body)| body.contains(anchor))
        .unwrap_or_else(|| {
            panic!("{skill}: {what} must live in a references/ file containing {anchor:?}")
        })
}

/// The wrap-up skill's Step 7 closing sequence, up to the next `## ` heading.
fn wrap_up_closing_sequence() -> &'static str {
    let body = skill_body("wrap-up");
    body.split_once("## Step 7")
        .map(|(_, rest)| rest.split("\n## ").next().unwrap_or(rest))
        .expect("wrap-up skill must have a Step 7 closing sequence")
}

/// Read an embedded skill's `SKILL.md` by skill name, for tests that assert
/// on skill copy.
fn skill_body(skill: &str) -> &'static str {
    skill_file(skill, "SKILL.md")
}

/// Read any embedded file from a skill directory. `SKILL.md` is the common
/// case ([`skill_body`]); allium-loop also ships the `prompt.md` that its
/// per-iteration agents actually run from, and that copy needs the same
/// deletion-is-a-regression protection.
fn skill_file(skill: &str, file: &str) -> &'static str {
    let path = format!("skills/{skill}/{file}");
    PLUGIN_DIR
        .get_file(&path)
        .unwrap_or_else(|| panic!("{path} must be embedded"))
        .contents_utf8()
        .unwrap_or_else(|| panic!("{path} must be UTF-8"))
}

mod feed_scripts;
mod plugin_install;
mod reference_files;
mod retro_skill;
mod skill_workflow;
mod wrap_up_skill;
