use super::*;

/// The decompose-review skill and its plan template with every run of
/// whitespace folded to one space, so a phrase the Markdown wraps across
/// lines still matches.
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// /decompose-review turns review findings into an epic of subtasks. Each
/// guarantee of `DecomposeReviewSession` in
/// `docs/specs/decompose-review-skill.allium` that the skill text can hold is
/// pinned here by the words in `SKILL.md` (or the plan template) that hold
/// it; the skill is followed by an agent, so the text is the behaviour a
/// test can pin. The pre-existing tests in `skill_workflow.rs` pin
/// EveryFindingIsTaken, ConfirmationNamesLeftOutFindings,
/// DecisionItemsReachThePlan and the wrap-up mode as well.
#[test]
fn decompose_review_skill_text_holds_each_session_guarantee() {
    let skill = flat(skill_body("decompose-review"));
    let template = flat(skill_file(
        "decompose-review",
        "references/plan-template.md",
    ));
    for (guarantee, needle) in [
        (
            "AnnouncesItself",
            "I'm using the decompose-review skill to create work packages from the code review.",
        ),
        ("EveryFindingIsTaken", "Take **every** finding, Low ones included"),
        (
            "EveryFindingIsTaken",
            "tell the user there is nothing to decompose and stop",
        ),
        ("GroupedByFilesTouched", "Group findings by the files they touch"),
        (
            "GroupedByFilesTouched",
            "Group by directory prefix (first 2 path components",
        ),
        (
            "GroupedByFilesTouched",
            "references files in multiple groups, merge those groups",
        ),
        (
            "PackagesAreOrderedBySeverity",
            "A Low finding stays in its group when that group also has a higher-severity finding",
        ),
        (
            "PackagesAreOrderedBySeverity",
            "Order work packages by maximum severity",
        ),
        (
            "PackagesAreOrderedBySeverity",
            "\"Cleanups\" goes last",
        ),
        (
            "DecisionItemsStayAndAreNotDecided",
            "A decision item stays in its work package. Do not leave it out, and do not pick an option for the user.",
        ),
        (
            "ConfirmationNamesLeftOutFindings",
            "Write \"Not included: none\"",
        ),
        (
            "NothingCreatedBeforeConfirmation",
            "Nothing is created until the user confirms.",
        ),
        (
            "NothingCreatedBeforeConfirmation",
            "ask with `AskUserQuestion` whether to confirm all, drop or merge some",
        ),
        (
            "RepoPathIsTheMainRoot",
            "git rev-parse --path-format=absolute --git-common-dir",
        ),
        (
            "RepoPathIsTheMainRoot",
            "`repo_path` on tasks must point to the main repo root",
        ),
        (
            "EpicCarriesNoRepoPath",
            "`epic_create` rejects the field outright",
        ),
        (
            "PlanDirectoryIsNeverReused",
            "Check first whether it already exists, before you create it, and if it does, do not write into it",
        ),
        (
            "PlanDirectoryIsNeverReused",
            "Say which directory you used when you report results",
        ),
        (
            "PlanHoldsEveryFinding",
            "All findings for this work package with their severity, file:line, description, and fix",
        ),
        (
            "PlanHoldsEveryFinding",
            "A changes table listing every file and what to change",
        ),
        (
            "PlanHoldsEveryFinding",
            "A verification section with test commands",
        ),
        (
            "DecisionItemsReachThePlan",
            "A Decisions section for each decision item",
        ),
        (
            "EverySubtaskCarriesAPlan",
            "`plan_path`: absolute path to the plan file just written",
        ),
        (
            "EverySubtaskCarriesAPlan",
            "`epic_id`: the epic ID from Step 5",
        ),
        (
            "AutoRunPlanSkipsStartNotDecisions",
            "`auto_run_plan`: `true`",
        ),
        (
            "AutoRunPlanSkipsStartNotDecisions",
            "not the questions in the plan's Decisions section",
        ),
        ("SubtasksRebaseAndKeepSeverityOrder", "`tag`: `\"chore\"`"),
        (
            "SubtasksRebaseAndKeepSeverityOrder",
            "`sort_order`: N",
        ),
        (
            "SubtasksRebaseAndKeepSeverityOrder",
            "`wrap_up_mode`: `\"rebase\"`",
        ),
        ("SubtasksAreMarkedReady", "`ready`: `true`"),
        (
            "SubtasksAreMarkedReady",
            "the user asked for this work when they confirmed the packages in Step 3",
        ),
        (
            "ReportNamesEpicAndTasks",
            "Display the created epic and all task IDs",
        ),
        (
            "ReportNamesEpicAndTasks",
            "Dispatch the epic from the TUI",
        ),
    ] {
        assert!(
            skill.contains(&flat(needle)),
            "decompose-review skill must hold {guarantee}: missing {needle:?}"
        );
    }
    for (guarantee, needle) in [
        ("DecisionItemsReachThePlan", "## Decisions"),
        (
            "DecisionItemsReachThePlan",
            "If the user does not answer, leave the finding unimplemented and say so at wrap-up",
        ),
        ("DecisionItemsReachThePlan", "**Question:**"),
        ("DecisionItemsReachThePlan", "**Options:**"),
        (
            "DecisionItemsReachThePlan",
            "Ask the user with `AskUserQuestion`",
        ),
        ("PlanHoldsEveryFinding", "## Findings"),
        ("PlanHoldsEveryFinding", "## Changes"),
        ("PlanHoldsEveryFinding", "## Verification"),
    ] {
        assert!(
            template.contains(&flat(needle)),
            "plan-template.md must hold {guarantee}: missing {needle:?}"
        );
    }
}

/// Decomposition.stage: the announcement, then extract, group, confirm,
/// resolve the repo path, create the epic, write plans and tasks, report.
/// The confirmation comes before anything is created.
#[test]
fn decompose_review_skill_confirms_before_it_creates() {
    let content = skill_body("decompose-review");
    let order = [
        "**Announce at start:**",
        "## Step 1: Extract Review Findings",
        "## Step 2: Group into Work Packages",
        "## Step 3: Confirm the Work Packages",
        "## Step 4: Resolve Repository Path",
        "## Step 5: Create Epic",
        "## Step 6: Write Plan Files and Create Tasks",
        "## Step 7: Report Results",
    ];
    let positions: Vec<usize> = order
        .iter()
        .map(|n| {
            content
                .find(n)
                .unwrap_or_else(|| panic!("decompose-review skill must have {n}"))
        })
        .collect();
    assert!(
        positions.windows(2).all(|w| w[0] < w[1]),
        "decompose-review must follow the Decomposition stage order: {order:?}"
    );
}

/// The plan is written before its subtask is created, so the subtask is
/// created with `plan_path` pointing at a file that exists
/// (WritePlanAndCreateTask).
#[test]
fn decompose_review_skill_writes_the_plan_before_the_task() {
    let step6 = skill_body("decompose-review")
        .split("## Step 6: Write Plan Files and Create Tasks")
        .nth(1)
        .and_then(|s| s.split("\n## Step 7").next())
        .expect("decompose-review skill must have a Step 6 section");
    let write = step6.find("### 6a. Write the plan file");
    let create = step6.find("### 6b. Create the subtask");
    assert!(
        matches!((write, create), (Some(w), Some(c)) if w < c),
        "Step 6 must write the plan (6a) before it creates the subtask (6b): {step6:?}"
    );
}
