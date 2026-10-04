use super::*;

// ---------------------------------------------------------------------------
// DetectedPrefillNeverOverwritesTyping — docs/specs/dispatch.allium, surface
// BaseBranchPicker. A repo with no remembered branches is exactly the repo the
// user has never answered this question for, so the field asks the repository
// rather than falling back to the literal "main". Reading the repository is a
// subprocess, so the answer arrives after the field is already open.
// ---------------------------------------------------------------------------

#[test]
fn submit_repo_path_without_history_asks_the_repo_for_its_default() {
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/tmp".to_string()];
    // No entry in repo_base_branches for "/tmp".
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: "D".to_string(),
        ..Default::default()
    });

    let cmds = app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitRepoPath("/tmp".to_string()),
    ));

    assert_eq!(app.input.mode, InputMode::InputBaseBranch);
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Settings(crate::tui::commands::SettingsCommand::DetectDefaultBranch {
                repo_path,
                replacing,
            }) if repo_path == "/tmp" && replacing == &app.input.buffer
        )),
        "a repo with no history must be asked for its default, naming the prefill \
         the answer is allowed to replace: {cmds:?}"
    );
}

#[test]
fn submit_repo_path_with_history_does_not_ask_the_repo() {
    // History is the user's own previous answer for this repo. It beats
    // whatever origin/HEAD says, and asking would be wasted work.
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/tmp".to_string()];
    app.board.repo_base_branches =
        std::collections::HashMap::from([("/tmp".to_string(), vec!["develop".to_string()])]);
    app.input.mode = InputMode::InputRepoPath;
    app.input.task_draft = Some(TaskDraft {
        title: "T".to_string(),
        description: "D".to_string(),
        ..Default::default()
    });

    let cmds = app.update(Message::Input(
        crate::tui::messages::InputMessage::SubmitRepoPath("/tmp".to_string()),
    ));

    assert_eq!(app.input.buffer, "develop");
    assert!(
        !cmds.iter().any(|c| matches!(
            c,
            Command::Settings(crate::tui::commands::SettingsCommand::DetectDefaultBranch { .. })
        )),
        "the repo's remembered branch settles it; nothing should be asked: {cmds:?}"
    );
}

#[test]
fn a_detected_default_branch_replaces_an_untouched_prefill() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputBaseBranch;
    app.input.set_buffer("main".to_string());

    app.update(Message::Input(
        crate::tui::messages::InputMessage::DefaultBranchDetected {
            branch: "master".to_string(),
            replacing: "main".to_string(),
        },
    ));

    assert_eq!(
        app.input.buffer, "master",
        "an untouched prefill is the field's own value, so the repo's answer replaces it"
    );
}

#[test]
fn a_detected_default_branch_is_dropped_once_the_user_has_typed() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputBaseBranch;
    app.input.set_buffer("main".to_string());
    app.update(Message::Input(
        crate::tui::messages::InputMessage::InputChar('x'),
    ));
    let typed = app.input.buffer.clone();

    app.update(Message::Input(
        crate::tui::messages::InputMessage::DefaultBranchDetected {
            branch: "master".to_string(),
            replacing: "main".to_string(),
        },
    ));

    assert_eq!(
        app.input.buffer, typed,
        "a late answer must never overwrite what the user typed"
    );
}

#[test]
fn a_detected_default_branch_is_dropped_after_the_step_moved_on() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputWrapUpMode;
    app.input.set_buffer("main".to_string());

    app.update(Message::Input(
        crate::tui::messages::InputMessage::DefaultBranchDetected {
            branch: "master".to_string(),
            replacing: "main".to_string(),
        },
    ));

    assert_eq!(
        app.input.buffer, "main",
        "the answer is for the base-branch step; another step's buffer is not its business"
    );
}
