//! The keybinding drift gate (`KeybindingDriftGate.press_every_row_key` in
//! `docs/specs/keybindings.allium`) for every board namespace the board's
//! dispatcher receives: with a row's context set up, pressing each of its keys
//! records the row's action id. Plus the dispatch-through-the-table rules
//! (`KeypressRunsItsRowsAction`, `KeyWithoutARowCannotAct`) and the context
//! evaluation the gate and the handler share (`context_holds`).
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::keybindings::{
    bindings_in, namespaces_matching, KeyBinding, KeyContext as C, KeyNamespace, ANY_OTHER_KEY,
    KEY_BINDINGS,
};
use crate::models::{ColumnSection, DispatchMode, EpicId, SubStatus, TaskStatus};
use crate::tui::commands::UsageCommand;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The key event for a key as the table writes it. A modifier prefix other
/// than letter-case Shift ("Ctrl+Left") adds that modifier.
fn key_event_for(key: &str) -> KeyEvent {
    if let Some(rest) = key.strip_prefix("Ctrl+") {
        let base = key_event_for(rest);
        return KeyEvent::new(base.code, base.modifiers | KeyModifiers::CONTROL);
    }
    if let Some(rest) = key.strip_prefix("Alt+") {
        let base = key_event_for(rest);
        return KeyEvent::new(base.code, base.modifiers | KeyModifiers::ALT);
    }
    let code = match key {
        "Space" => KeyCode::Char(' '),
        "Enter" => KeyCode::Enter,
        "Esc" => KeyCode::Esc,
        "Tab" => KeyCode::Tab,
        "BackTab" => KeyCode::BackTab,
        "Backspace" => KeyCode::Backspace,
        "Delete" => KeyCode::Delete,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "Left" => KeyCode::Left,
        "Right" => KeyCode::Right,
        "Up" => KeyCode::Up,
        "Down" => KeyCode::Down,
        k if k.chars().count() == 1 => KeyCode::Char(k.chars().next().unwrap()),
        other => panic!("test cannot press {other}"),
    };
    let shift =
        matches!(code, KeyCode::Char(c) if c.is_ascii_uppercase()) || code == KeyCode::BackTab;
    KeyEvent::new(
        code,
        if shift {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        },
    )
}

fn recorded(cmds: &[Command]) -> Vec<(String, Option<String>)> {
    cmds.iter()
        .filter_map(|c| match c {
            Command::Usage(UsageCommand::Record(e)) => Some((e.action.clone(), e.detail.clone())),
            _ => None,
        })
        .collect()
}

/// Press `key` as the table writes it. A chord is its keys in turn; the
/// commands returned are the completing press's.
fn press(app: &mut App, key: &str) -> Vec<Command> {
    if key == "gg" {
        app.handle_key(key_event_for("g"));
        return app.handle_key(key_event_for("g"));
    }
    app.handle_key(key_event_for(key))
}

/// What the usage event records as the key: key_label's form, which is the
/// table's written form except that Space is recorded as " " and modifiers are
/// dropped ("Ctrl+Left" is recorded as "Left").
fn detail_for(key: &str) -> String {
    let base = key
        .strip_prefix("Ctrl+")
        .or_else(|| key.strip_prefix("Alt+"))
        .unwrap_or(key);
    if base == "Space" {
        " ".to_string()
    } else {
        base.to_string()
    }
}

/// A key no row of `ns` lists, to press for its catch-all row (or to show a
/// namespace without one ignores it).
fn unlisted_key(ns: KeyNamespace) -> &'static str {
    ["w", "Q", "%", "W", "_"]
        .into_iter()
        .find(|k| !bindings_in(ns).any(|b| b.keys.contains(k)))
        .unwrap_or_else(|| panic!("{}: every candidate key is listed", ns.name()))
}

// ---------------------------------------------------------------------------
// board.normal contexts
// ---------------------------------------------------------------------------

fn on_running_card(mut app: App) -> App {
    app.selection_mut().set_column(2);
    app.selection_mut().set_row(2, 0);
    app
}

/// A board whose cursor sits on one Running task, adjusted by `f`.
fn app_with_running_task(f: impl FnOnce(&mut Task)) -> App {
    let mut t = make_task(7, TaskStatus::Running);
    f(&mut t);
    on_running_card(App::new(vec![t]))
}

fn feed_epic_app() -> App {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    let mut e = make_epic(10);
    e.feed_command = Some("echo '[]'".to_string());
    app.board.epics = vec![e];
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 1);
    app
}

fn inside_epic_view() -> App {
    let mut app = make_app();
    app.board.epics = vec![make_epic(10)];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(10),
    )));
    assert!(matches!(app.board.view_mode, ViewMode::Epic { .. }));
    app
}

/// A row-less binding standing for "the board with this context set up".
fn in_context(context: C) -> KeyBinding {
    KeyBinding {
        namespace: KeyNamespace::BoardNormal,
        keys: &[],
        context: Some(context),
        action: "",
        description: "",
        note: None,
        records_usage: true,
    }
}

/// The board with `binding`'s context set up so its action takes effect.
fn app_for(binding: &KeyBinding) -> App {
    let action = binding.action;
    match binding.context {
        Some(C::SearchActive) => {
            let mut app = make_app();
            app.search.query = "task".to_string();
            app
        }
        Some(C::InsideEpicView) | Some(C::EpicViewNoSearch) => inside_epic_view(),
        Some(C::TopLevel) => make_app(),
        Some(C::WithSelection) => {
            let mut app = make_app();
            app.selection_mut().set_column(1);
            app.handle_key(key_event_for("v"));
            app
        }
        Some(C::OnColumnSelectAll) => {
            let mut app = make_app();
            app.selection_mut().set_column(1);
            app.update(Message::NavigateRow(-1));
            app.update(Message::SelectAllColumn);
            app
        }
        Some(C::OnFoldedSection) => {
            let mut a = make_task(1, TaskStatus::Running);
            a.sub_status = SubStatus::NeedsInput;
            let mut app = App::new(vec![a]);
            app.selection_mut().set_column(2);
            app.toggle_section_collapse(TaskStatus::Running, ColumnSection::NeedsInput);
            app.selection_mut().set_row(2, 0);
            app
        }
        Some(C::OnFoldedEpicGroup) => {
            let mut a = make_task(1, TaskStatus::Running);
            a.epic_id = Some(EpicId(10));
            let mut app = App::new(vec![a]);
            app.board.epics = vec![make_epic(10)];
            app.board.flattened = true;
            app.selection_mut().set_column(2);
            app.toggle_epic_fold(TaskStatus::Running, EpicId(10));
            app.selection_mut().set_row(2, 0);
            app
        }
        Some(C::OnEpicCard) => make_app_with_epic_selected(),
        Some(C::OnFlattenedTaskCard) => {
            let mut app = app_with_running_task(|t| t.epic_id = Some(EpicId(10)));
            app.board.epics = vec![make_epic(10)];
            app.board.flattened = true;
            app
        }
        Some(C::OnUnflattenedTaskCard) => {
            let mut app = make_app();
            app.selection_mut().set_column(1);
            app.selection_mut().set_row(1, 0);
            app
        }
        Some(C::OnTaskCard) | Some(C::OffEpicCard) | Some(C::TaskWithWindow) | None => match action
        {
            "refresh_feed" => feed_epic_app(),
            "open_pr_url" => app_with_running_task(|t| {
                t.url = Some(crate::models::TaskUrl::new(
                    "https://github.com/example/repo/pull/42",
                    crate::models::UrlType::Pr,
                ));
            }),
            "open_repo_sync_prompt" => {
                let mut app = on_running_card(make_app());
                let repo = app.selected_task().unwrap().repo_path.clone();
                app.update(Message::RepoSync(
                    crate::tui::messages::RepoSyncMessage::Measured(
                        crate::repo_sync::RepoSyncMeasurement {
                            repo_path: repo,
                            base_branch: "main".to_string(),
                            counts: Some(crate::repo_sync::AheadBehind {
                                ahead: 1,
                                behind: 1,
                            }),
                            fetch_error: None,
                        },
                    ),
                ));
                app
            }
            _ => on_running_card(make_app()),
        },
        Some(C::TaskOnOtherMachine) => {
            let mut app = app_with_running_task(|t| t.host = Some("other-machine".to_string()));
            app.set_local_host_id("this-machine".to_string());
            app
        }
        Some(C::TaskPinnedInSplit) => {
            let mut app = app_with_running_task(|_| {});
            app.board.split.active = true;
            app.board.split.right_pane_id = Some("%42".to_string());
            app.board.split.pinned_task_id = Some(TaskId(7));
            app
        }
        Some(C::TaskWindowSplitOpen) => {
            let mut app = app_with_running_task(|_| {});
            app.board.split.active = true;
            app.board.split.right_pane_id = Some("%42".to_string());
            app.board.split.pinned_task_id = None;
            app
        }
        Some(C::BacklogTask) => {
            let mut app = make_app();
            app.selection_mut().set_column(1);
            app.selection_mut().set_row(1, 0);
            app
        }
        Some(C::StuckTask) => app_with_running_task(|t| {
            t.tmux_window = None;
            t.sub_status = SubStatus::Stale;
        }),
        Some(C::TaskDispatching) => app_with_running_task(|t| {
            t.worktree = None;
            t.tmux_window = None;
            t.last_pre_tool_use_at = Some(chrono::Utc::now());
        }),
        Some(C::TaskWithWorktree) => app_with_running_task(|t| t.tmux_window = None),
        Some(C::TaskWithoutWorktree) => {
            let mut t = make_task(8, TaskStatus::Done);
            t.worktree = None;
            t.tmux_window = None;
            let mut app = App::new(vec![t]);
            app.selection_mut().set_column(4);
            app.selection_mut().set_row(4, 0);
            app
        }
        Some(C::DetailZoomed) => panic!("no board.normal row is guarded by detail_zoomed"),
        Some(C::OnDirectory) | Some(C::OnFile) => {
            panic!(
                "the board has no file tree: {:?} is a pane context",
                binding.context
            )
        }
    }
}

// ---------------------------------------------------------------------------
// The other board namespaces: put the board into the namespace's input mode
// ---------------------------------------------------------------------------

fn draft_app(mode: InputMode) -> App {
    let mut app = make_app();
    app.board.repo_paths = vec!["/repo/a".to_string(), "/repo/b".to_string()];
    app.input.task_draft = Some(TaskDraft {
        repo_path: "/repo/a".to_string(),
        ..TaskDraft::default()
    });
    app.input.epic_draft = Some(EpicDraft::default());
    app.input.mode = mode;
    app
}

fn with_move_task_picker() -> App {
    let mut app = make_app();
    app.board.epics = vec![make_epic(10), make_epic(11)];
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::StartMoveToEpic(TaskId(1)),
    ));
    assert!(
        matches!(app.input.mode, InputMode::MoveTaskToEpic(_)),
        "the move-to-epic picker did not open: {:?}",
        app.input.mode
    );
    app
}

fn with_reparent_picker() -> App {
    let mut app = make_app();
    app.board.epics = vec![make_epic(10), make_epic(11)];
    app.update(Message::Epic(
        crate::tui::messages::EpicMessage::StartReparent(EpicId(10)),
    ));
    assert!(
        matches!(app.input.mode, InputMode::ReparentEpic(_)),
        "the reparent picker did not open: {:?}",
        app.input.mode
    );
    app
}

/// Every board the namespace is entered on. One for most namespaces;
/// board.text is four input modes sharing one namespace, and each is set up.
fn apps_in(binding: &KeyBinding) -> Vec<App> {
    use KeyNamespace as N;
    let ns = binding.namespace;
    if ns == N::BoardNormal {
        return vec![app_for(binding)];
    }
    match (ns, binding.context) {
        (_, None) | (N::BoardDetail, Some(C::DetailZoomed)) => {}
        (_, Some(c)) => panic!(
            "{}: the gate cannot set up context {c:?} outside board.normal",
            ns.name()
        ),
    }
    let one = |app: App| vec![app];
    match ns {
        N::BoardNormal => unreachable!(),
        N::BoardDetail => {
            let mut app = make_app();
            app.update(Message::Task(
                crate::tui::messages::TaskMessage::OpenDetail(TaskId(3)),
            ));
            match &mut app.board.view_mode {
                ViewMode::TaskDetail { zoomed, .. } => {
                    *zoomed = binding.context == Some(C::DetailZoomed);
                }
                other => panic!("the detail view did not open: {other:?}"),
            }
            one(app)
        }
        N::BoardSearch => {
            let mut app = make_app();
            app.handle_key(key_event_for("/"));
            assert_eq!(app.input.mode, InputMode::SearchTasks);
            one(app)
        }
        N::BoardHelp => {
            let mut app = make_app();
            app.update(Message::System(
                crate::tui::messages::SystemMessage::ToggleHelp,
            ));
            assert_eq!(app.input.mode, InputMode::Help);
            one(app)
        }
        N::BoardError => {
            let mut app = make_app();
            app.status.error_popup = Some("boom".to_string());
            one(app)
        }
        N::BoardText => [
            InputMode::InputTitle,
            InputMode::InputDescription,
            InputMode::InputEpicTitle,
            InputMode::InputEpicDescription,
        ]
        .into_iter()
        .map(draft_app)
        .collect(),
        N::BoardRepoFilter => {
            let mut app = make_app();
            // Nine repos so every digit 1-9 toggles one.
            app.board.repo_paths = (1..=9).map(|i| format!("/repo/r{i}")).collect();
            app.update(Message::RepoFilter(
                crate::tui::messages::RepoFilterMessage::Start,
            ));
            assert_eq!(app.input.mode, InputMode::RepoFilter);
            one(app)
        }
        N::BoardPickerRepoPath => one(draft_app(InputMode::InputRepoPath)),
        N::BoardPickerBaseBranch => {
            let mut app = draft_app(InputMode::InputBaseBranch);
            app.board.repo_base_branches.insert(
                "/repo/a".to_string(),
                vec!["main".to_string(), "dev".to_string()],
            );
            one(app)
        }
        N::BoardPickerTag => one(draft_app(InputMode::InputTag)),
        N::BoardPickerWrapUpMode => one(draft_app(InputMode::InputWrapUpMode)),
        N::BoardPickerQuickDispatch => one(draft_app(InputMode::QuickDispatch)),
        N::BoardPickerMoveToEpic => one(with_move_task_picker()),
        N::BoardPickerReparentEpic => one(with_reparent_picker()),
        N::BoardConfirmQuit => one(draft_app(InputMode::ConfirmQuit)),
        N::BoardConfirmDeleteTask => one(draft_app(InputMode::ConfirmDeleteTask(TaskId(4)))),
        N::BoardConfirmBatchDelete => one(draft_app(InputMode::ConfirmBatchDelete)),
        N::BoardConfirmDeleteEpic => {
            let mut app = make_app_with_epic_selected();
            app.input.mode = InputMode::ConfirmDeleteEpic;
            one(app)
        }
        N::BoardConfirmDone => one(draft_app(InputMode::ConfirmDone)),
        N::BoardConfirmRetry => one(draft_app(InputMode::ConfirmRetry(TaskId(3)))),
        N::BoardConfirmDetachTmux => one(draft_app(InputMode::ConfirmDetachTmux(vec![TaskId(3)]))),
        N::BoardConfirmOverrideFeedOwner => {
            let mut app = draft_app(InputMode::ConfirmOverrideFeedOwner {
                epic_id: EpicId(10),
                other_host: "other-machine".to_string(),
            });
            app.board.epics = vec![make_epic(10)];
            one(app)
        }
        N::BoardConfirmMoveToEpic => {
            let mut app = with_move_task_picker();
            app.input.mode = InputMode::ConfirmMoveTaskToEpic {
                task_id: TaskId(1),
                new_epic: Some(EpicId(10)),
            };
            one(app)
        }
        N::BoardConfirmReparentEpic => {
            let mut app = with_reparent_picker();
            app.input.mode = InputMode::ConfirmReparentEpic {
                epic_id: EpicId(10),
                new_parent: Some(EpicId(11)),
            };
            one(app)
        }
        N::BoardConfirmDeleteRepoPath => {
            let mut app = draft_app(InputMode::ConfirmDeleteRepoPath);
            app.input.repo_cursor = 0;
            one(app)
        }
        N::BoardConfirmTrustRepo => one(draft_app(InputMode::ConfirmTrustRepo {
            task_id: TaskId(1),
            mode: DispatchMode::Dispatch,
        })),
        N::BoardConfirmTrustRepoQuickDispatch => {
            one(draft_app(InputMode::ConfirmTrustRepoQuickDispatch {
                draft: TaskDraft {
                    repo_path: "/repo/a".to_string(),
                    ..TaskDraft::default()
                },
                epic_id: None,
            }))
        }
        N::BoardConfirmRepoSync => one(draft_app(InputMode::ConfirmRepoSync {
            repo_path: "/repo/a".to_string(),
        })),
        N::AgentTreeTree
        | N::AgentTreeCommits
        | N::AgentTreeAgents
        | N::AgentDiff
        | N::TmuxGlobal => {
            panic!("{} is not received by the board", ns.name())
        }
    }
}

fn board_namespaces() -> Vec<KeyNamespace> {
    KeyNamespace::ALL
        .iter()
        .copied()
        .filter(|n| n.name().starts_with("board."))
        .collect()
}

/// press_every_row_key / RecordedActionMatchesRow, for every board namespace.
/// A catch-all row is pressed with a key no other row of its namespace lists,
/// and records the key actually pressed.
#[test]
fn pressing_each_key_of_each_board_row_records_the_rows_action() {
    let mut failures = Vec::new();
    let mut pressed = 0;
    for ns in board_namespaces() {
        for binding in bindings_in(ns) {
            let keys: Vec<&str> = if binding.keys == [ANY_OTHER_KEY] {
                vec![unlisted_key(ns)]
            } else {
                binding.keys.to_vec()
            };
            for key in keys {
                for (i, mut app) in apps_in(binding).into_iter().enumerate() {
                    let cmds = press(&mut app, key);
                    pressed += 1;
                    let got = recorded(&cmds);
                    let want = if binding.records_usage {
                        vec![(binding.action.to_string(), Some(detail_for(key)))]
                    } else {
                        // UnrecordedRowsRecordNothing: text editing runs but
                        // leaves no usage trace.
                        vec![]
                    };
                    if got != want {
                        failures.push(format!(
                            "{} {key} [{:?}] (board #{i}) expected {want:?}, recorded {got:?}",
                            ns.name(),
                            binding.context
                        ));
                    }
                }
            }
        }
    }
    assert!(pressed > 0);
    assert!(
        failures.is_empty(),
        "rows whose keys do not record their action:\n{}",
        failures.join("\n")
    );
}

/// The gate above is only as strong as the rows it presses: every board
/// namespace must contribute at least one.
#[test]
fn the_gate_presses_a_row_in_every_board_namespace() {
    let unpressed: Vec<&str> = board_namespaces()
        .into_iter()
        .filter(|ns| bindings_in(*ns).next().is_none())
        .map(|ns| ns.name())
        .collect();
    assert!(unpressed.is_empty(), "no rows to press in {unpressed:?}");
}

/// scroll_help is one of the rows the gate presses for board.help.
#[test]
fn the_gate_presses_scroll_help() {
    let binding = bindings_in(KeyNamespace::BoardHelp)
        .find(|b| b.action == "scroll_help")
        .expect("board.help has a scroll_help row");
    for key in ["j", "Down", "k", "Up"] {
        assert!(binding.keys.contains(&key), "{key}");
        let mut app = apps_in(binding).remove(0);
        let cmds = press(&mut app, key);
        assert_eq!(
            recorded(&cmds),
            vec![("scroll_help".to_string(), Some(key.to_string()))]
        );
        assert_eq!(
            app.input.mode,
            InputMode::Help,
            "{key} keeps the overlay open"
        );
    }
}

// ---------------------------------------------------------------------------
// KeyWithoutARowCannotAct
// ---------------------------------------------------------------------------

/// In a namespace with no catch-all, a key no row lists runs nothing and
/// records nothing. Typing modes are excluded: there a printable key is text
/// entry, which is outside the rule.
#[test]
fn a_key_with_no_row_runs_nothing_and_records_nothing() {
    use KeyNamespace as N;
    let typing = [
        N::BoardSearch,
        N::BoardText,
        N::BoardPickerRepoPath,
        N::BoardPickerBaseBranch,
        N::BoardPickerQuickDispatch,
    ];
    let mut checked = 0;
    for ns in board_namespaces() {
        if typing.contains(&ns) || bindings_in(ns).any(|b| b.keys.contains(&ANY_OTHER_KEY)) {
            continue;
        }
        let Some(sample) = bindings_in(ns).next() else {
            continue;
        };
        let key = unlisted_key(ns);
        let unguarded = KeyBinding {
            context: None,
            ..*sample
        };
        for mut app in apps_in(if ns == N::BoardNormal {
            sample
        } else {
            &unguarded
        }) {
            let mode_before = app.input.mode.clone();
            let cmds = press(&mut app, key);
            assert!(
                cmds.is_empty(),
                "{} {key}: a key with no row produced {} commands",
                ns.name(),
                cmds.len()
            );
            assert_eq!(
                app.input.mode,
                mode_before,
                "{} {key} changed the mode",
                ns.name()
            );
            checked += 1;
        }
    }
    assert!(checked >= 5, "only {checked} namespaces were checked");
}

// ---------------------------------------------------------------------------
// Rebinding is an edit to one row (KeypressRunsItsRowsAction)
// ---------------------------------------------------------------------------

/// The real table with board.normal's delete_task row answering `W` instead
/// of `x`, and nothing else changed.
fn table_with_delete_rebound_to_w() -> &'static [KeyBinding] {
    let rows: Vec<KeyBinding> = KEY_BINDINGS
        .iter()
        .map(|b| {
            if b.namespace == KeyNamespace::BoardNormal && b.action == "delete_task" {
                assert_eq!(
                    b.keys,
                    &["x"],
                    "the fixture assumes x is delete_task's only key"
                );
                KeyBinding { keys: &["W"], ..*b }
            } else {
                *b
            }
        })
        .collect();
    assert!(
        !rows.iter().any(|b| b.namespace == KeyNamespace::BoardNormal
            && b.keys.contains(&"W")
            && b.action != "delete_task"),
        "W must be free in board.normal for this fixture"
    );
    Box::leak(rows.into_boxed_slice())
}

#[test]
fn a_rebound_key_runs_the_rows_action() {
    let mut app = on_running_card(make_app());
    app.set_key_table(table_with_delete_rebound_to_w());
    let cmds = press(&mut app, "W");
    assert_eq!(
        recorded(&cmds),
        vec![("delete_task".to_string(), Some("W".to_string()))]
    );
    assert_eq!(
        app.input.mode,
        InputMode::ConfirmDone,
        "W on a Running task runs delete_task, which asks to move it to Done"
    );
}

#[test]
fn the_key_a_row_no_longer_lists_does_nothing() {
    let mut app = on_running_card(make_app());
    app.set_key_table(table_with_delete_rebound_to_w());
    let cmds = press(&mut app, "x");
    assert!(
        cmds.is_empty(),
        "x has no row any more: {} commands",
        cmds.len()
    );
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn the_default_table_is_key_bindings() {
    let mut app = on_running_card(make_app());
    let cmds = press(&mut app, "x");
    assert_eq!(
        recorded(&cmds),
        vec![("delete_task".to_string(), Some("x".to_string()))]
    );
    assert_eq!(app.input.mode, InputMode::ConfirmDone);

    let mut app = on_running_card(make_app());
    let cmds = press(&mut app, "W");
    assert!(cmds.is_empty(), "W has no row in the real table");
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// Rebinding works in a namespace other than board.normal too: the table is
/// the dispatcher for every board mode, not just the board.
#[test]
fn a_rebound_confirm_key_runs_the_rows_action() {
    let rows: Vec<KeyBinding> = KEY_BINDINGS
        .iter()
        .map(|b| {
            if b.namespace == KeyNamespace::BoardConfirmQuit && b.action == "confirm_quit_yes" {
                KeyBinding {
                    keys: &["Enter"],
                    ..*b
                }
            } else {
                *b
            }
        })
        .collect();
    let table: &'static [KeyBinding] = Box::leak(rows.into_boxed_slice());

    let mut app = draft_app(InputMode::ConfirmQuit);
    app.set_key_table(table);
    let cmds = press(&mut app, "Enter");
    assert_eq!(
        recorded(&cmds),
        vec![("confirm_quit_yes".to_string(), Some("Enter".to_string()))]
    );
    assert!(app.should_quit);

    // y is no longer listed, so the catch-all answers it.
    let mut app = draft_app(InputMode::ConfirmQuit);
    app.set_key_table(table);
    let cmds = press(&mut app, "y");
    assert_eq!(
        recorded(&cmds),
        vec![("confirm_quit_no".to_string(), Some("y".to_string()))]
    );
    assert!(!app.should_quit);
}

// ---------------------------------------------------------------------------
// q and Esc inside an epic view
// ---------------------------------------------------------------------------

fn inside_epic_view_with_search() -> App {
    let mut app = inside_epic_view();
    app.search.query = "task".to_string();
    app
}

#[test]
fn q_inside_an_epic_with_a_search_active_still_exits_the_epic() {
    let mut app = inside_epic_view_with_search();
    let cmds = press(&mut app, "q");
    assert_eq!(
        recorded(&cmds),
        vec![("exit_epic".to_string(), Some("q".to_string()))]
    );
    assert!(!matches!(app.board.view_mode, ViewMode::Epic { .. }));
}

#[test]
fn esc_inside_an_epic_with_a_search_active_clears_the_search() {
    let mut app = inside_epic_view_with_search();
    let cmds = press(&mut app, "Esc");
    assert_eq!(
        recorded(&cmds),
        vec![("clear_search".to_string(), Some("Esc".to_string()))]
    );
    assert!(app.search.query.is_empty());
    assert!(
        matches!(app.board.view_mode, ViewMode::Epic { .. }),
        "clearing the search leaves the epic view open"
    );
}

#[test]
fn esc_inside_an_epic_with_no_search_exits_the_epic() {
    let mut app = inside_epic_view();
    let cmds = press(&mut app, "Esc");
    assert_eq!(
        recorded(&cmds),
        vec![("exit_epic".to_string(), Some("Esc".to_string()))]
    );
    assert!(!matches!(app.board.view_mode, ViewMode::Epic { .. }));
}

// ---------------------------------------------------------------------------
// context_holds: the contexts for one key are mutually exclusive on real boards
// ---------------------------------------------------------------------------

fn holding(app: &App, among: &[C]) -> Vec<C> {
    among
        .iter()
        .copied()
        .filter(|c| app.context_holds(*c))
        .collect()
}

const LADDER: [C; 9] = [
    C::TaskOnOtherMachine,
    C::TaskPinnedInSplit,
    C::TaskWindowSplitOpen,
    C::TaskWithWindow,
    C::BacklogTask,
    C::StuckTask,
    C::TaskDispatching,
    C::TaskWithWorktree,
    C::TaskWithoutWorktree,
];

/// Space's activation ladder: on each rung's board, that rung and no other
/// holds.
#[test]
fn exactly_one_rung_of_the_activation_ladder_holds() {
    for rung in LADDER {
        let app = app_for(&in_context(rung));
        assert_eq!(
            holding(&app, &LADDER),
            vec![rung],
            "board set up for {rung:?}"
        );
    }
}

const CURSOR_LEVEL: [C; 5] = [
    C::OnColumnSelectAll,
    C::OnFoldedSection,
    C::OnFoldedEpicGroup,
    C::OnEpicCard,
    C::OnTaskCard,
];

/// Enter, Space, m, H and L: the cursor is on at most one of these.
#[test]
fn exactly_one_cursor_level_context_holds() {
    for ctx in CURSOR_LEVEL {
        let app = app_for(&in_context(ctx));
        assert_eq!(
            holding(&app, &CURSOR_LEVEL),
            vec![ctx],
            "board set up for {ctx:?}"
        );
    }
    // Every rung of the ladder is a board with the cursor on a task card.
    for rung in LADDER {
        let app = app_for(&in_context(rung));
        assert_eq!(
            holding(&app, &CURSOR_LEVEL),
            vec![C::OnTaskCard],
            "board set up for {rung:?}"
        );
    }
}

/// q's rows (inside_epic_view, top_level) and Esc's rows (search_active,
/// epic_view_no_search, with_selection), evaluated on every view-level board.
#[test]
fn q_and_esc_contexts_are_mutually_exclusive() {
    let top_with_search_and_selection = || {
        let mut app = app_for(&in_context(C::WithSelection));
        app.search.query = "task".to_string();
        app
    };
    let boards: Vec<(&str, App, C, Option<C>)> = vec![
        ("top-level", make_app(), C::TopLevel, None),
        (
            "top-level, search",
            app_for(&in_context(C::SearchActive)),
            C::TopLevel,
            Some(C::SearchActive),
        ),
        (
            "epic view",
            inside_epic_view(),
            C::InsideEpicView,
            Some(C::EpicViewNoSearch),
        ),
        (
            "epic view, search",
            inside_epic_view_with_search(),
            C::InsideEpicView,
            Some(C::SearchActive),
        ),
        (
            "top-level, selection",
            app_for(&in_context(C::WithSelection)),
            C::TopLevel,
            Some(C::WithSelection),
        ),
        (
            "top-level, select-all row",
            app_for(&in_context(C::OnColumnSelectAll)),
            C::TopLevel,
            Some(C::WithSelection),
        ),
        (
            "top-level, search and selection",
            top_with_search_and_selection(),
            C::TopLevel,
            Some(C::SearchActive),
        ),
    ];
    let q = [C::InsideEpicView, C::TopLevel];
    let esc = [C::SearchActive, C::EpicViewNoSearch, C::WithSelection];
    for (name, app, want_q, want_esc) in boards {
        assert_eq!(holding(&app, &q), vec![want_q], "q on {name}");
        assert_eq!(
            holding(&app, &esc),
            want_esc.into_iter().collect::<Vec<_>>(),
            "Esc on {name}"
        );
    }
}

/// The gate's own setups agree with context_holds: a row's board is one on
/// which the row's context holds. Without this, the gate could pass on a board
/// where the handler reached the row's action by some other path.
#[test]
fn each_board_normal_rows_setup_satisfies_its_context() {
    for binding in bindings_in(KeyNamespace::BoardNormal) {
        if let Some(ctx) = binding.context {
            let app = app_for(binding);
            assert!(
                app.context_holds(ctx),
                "{} {:?}: the board set up for it does not satisfy {ctx:?}",
                binding.action,
                binding.keys
            );
        }
    }
}

/// detail_zoomed holds exactly when the detail view is zoomed.
#[test]
fn detail_zoomed_holds_only_when_zoomed() {
    let row = |zoomed: bool| KeyBinding {
        namespace: KeyNamespace::BoardDetail,
        keys: &[],
        context: zoomed.then_some(C::DetailZoomed),
        action: "",
        description: "",
        note: None,
        records_usage: true,
    };
    assert!(apps_in(&row(true))[0].context_holds(C::DetailZoomed));
    assert!(!apps_in(&row(false))[0].context_holds(C::DetailZoomed));
}

// ---------------------------------------------------------------------------
// Families
// ---------------------------------------------------------------------------

/// Every board.confirm.* and board.picker.* namespace is one the gate can set
/// up (apps_in panics on a namespace it does not know).
#[test]
fn every_family_namespace_can_be_set_up() {
    for family in ["board.confirm", "board.picker"] {
        for ns in namespaces_matching(family).unwrap() {
            let probe = KeyBinding {
                namespace: ns,
                keys: &[],
                context: None,
                action: "",
                description: "",
                note: None,
                records_usage: true,
            };
            assert!(!apps_in(&probe).is_empty(), "{}", ns.name());
        }
    }
}

// ---------------------------------------------------------------------------
// ModifiedPressMatchesOnlyItsOwnRow
// ---------------------------------------------------------------------------

fn no_recorded_usage(cmds: &[Command]) -> bool {
    recorded(cmds).is_empty()
}

/// Ctrl+c is not `c`: it runs nothing in board.normal (it used to copy the
/// task).
#[test]
fn ctrl_c_on_the_board_does_not_copy_the_task() {
    let mut app = on_running_card(make_app());
    let before = app.input.mode.clone();
    let cmds = press(&mut app, "Ctrl+C");
    assert!(cmds.is_empty(), "{cmds:?}");
    assert_eq!(app.input.mode, before);
}

/// Ctrl+b at the tag step does not pick Bug.
#[test]
fn ctrl_b_at_the_tag_picker_does_not_pick_bug() {
    let binding = bindings_in(KeyNamespace::BoardPickerTag)
        .find(|b| b.action == "tag_picker_select")
        .unwrap();
    let mut app = apps_in(binding).remove(0);
    let cmds = press(&mut app, "Ctrl+B");
    assert!(cmds.is_empty(), "{cmds:?}");
    assert_eq!(app.input.mode, InputMode::InputTag);
}

/// A modified press is never answered by a catch-all: Ctrl+x in the error
/// popup leaves it open, and in a y/Y dialog leaves the dialog open.
#[test]
fn a_modified_press_is_not_answered_by_a_catch_all() {
    let mut checked = 0;
    for ns in board_namespaces() {
        if !bindings_in(ns).any(|b| b.keys.contains(&ANY_OTHER_KEY)) {
            continue;
        }
        let binding = bindings_in(ns)
            .find(|b| b.keys.contains(&ANY_OTHER_KEY))
            .unwrap();
        for mut app in apps_in(binding) {
            let mode = app.input.mode.clone();
            let popup = app.status.error_popup.clone();
            let cmds = press(&mut app, "Ctrl+X");
            assert!(no_recorded_usage(&cmds), "{}: {cmds:?}", ns.name());
            assert_eq!(app.input.mode, mode, "{}", ns.name());
            assert_eq!(app.status.error_popup, popup, "{}", ns.name());
            checked += 1;
        }
    }
    assert!(checked > 0);
}

/// A modified form of every bare key of every board row, that no row lists,
/// runs nothing and records nothing.
#[test]
fn a_modified_bare_key_with_no_row_does_nothing() {
    let mut checked = 0;
    for ns in board_namespaces() {
        for binding in bindings_in(ns) {
            for key in binding.keys {
                let bare = key.chars().count() == 1 && key.chars().all(|c| c.is_ascii_lowercase());
                let modified = format!("Ctrl+{}", key.to_uppercase());
                if !bare || bindings_in(ns).any(|b| b.keys.contains(&modified.as_str())) {
                    continue;
                }
                for mut app in apps_in(binding) {
                    let cmds = press(&mut app, &modified);
                    assert!(
                        no_recorded_usage(&cmds),
                        "{} {modified}: {cmds:?}",
                        ns.name()
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 50, "{checked}");
}

/// A modified letter is never typed into a text field.
#[test]
fn a_modified_letter_is_not_typed_into_a_text_field() {
    let binding = bindings_in(KeyNamespace::BoardText).next().unwrap();
    let mut app = apps_in(binding).remove(0);
    let before = app.input.buffer.clone();
    press(&mut app, "Ctrl+X");
    press(&mut app, "Alt+X");
    assert_eq!(app.input.buffer, before);
    press(&mut app, "x");
    assert_eq!(app.input.buffer, format!("{before}x"));
}

// ---------------------------------------------------------------------------
// Text-editing rows
// ---------------------------------------------------------------------------

/// Backspace, Delete, Home, End and the caret and word motions are rows, run
/// their action and record nothing.
#[test]
fn text_editing_keys_edit_the_field_and_record_nothing() {
    let binding = bindings_in(KeyNamespace::BoardText).next().unwrap();
    let mut app = apps_in(binding).remove(0);
    for c in "hello world".chars() {
        press(&mut app, &c.to_string());
    }
    let buf = |app: &App| app.input.buffer.clone();
    assert_eq!(buf(&app), "hello world");
    assert!(no_recorded_usage(&press(&mut app, "Backspace")));
    assert_eq!(buf(&app), "hello worl");
    for key in [
        "Left",
        "Home",
        "Right",
        "End",
        "Ctrl+Left",
        "Alt+B",
        "Alt+Left",
        "Ctrl+Right",
        "Alt+F",
        "Alt+Right",
    ] {
        assert!(no_recorded_usage(&press(&mut app, key)), "{key}");
    }
    // Caret is at the end; word-left then Delete removes from there.
    press(&mut app, "Ctrl+Left");
    press(&mut app, "Delete");
    assert_eq!(buf(&app), "hello orl");
    press(&mut app, "Home");
    press(&mut app, "Delete");
    assert_eq!(buf(&app), "ello orl");
}

/// Backspace in the search bar edits the query and records nothing.
#[test]
fn backspace_in_the_search_bar_edits_the_query_silently() {
    let binding = bindings_in(KeyNamespace::BoardSearch).next().unwrap();
    let mut app = apps_in(binding).remove(0);
    app.search.query = "abc".to_string();
    let cmds = press(&mut app, "Backspace");
    assert!(no_recorded_usage(&cmds));
    assert_eq!(app.search.query, "ab");
}

// ---------------------------------------------------------------------------
// H/L off a card
// ---------------------------------------------------------------------------

/// off_epic_card is the complement of on_epic_card, on every board the gate
/// can build.
#[test]
fn off_epic_card_is_the_complement_of_on_epic_card() {
    for ctx in [
        C::OnEpicCard,
        C::OnTaskCard,
        C::OnColumnSelectAll,
        C::TopLevel,
    ] {
        let app = app_for(&in_context(ctx));
        assert_eq!(
            app.context_holds(C::OffEpicCard),
            !app.context_holds(C::OnEpicCard),
            "{ctx:?}"
        );
    }
}

/// With the cursor on a column's select-all row and a selection, L still
/// moves the selection and records move_task_forward.
#[test]
fn l_on_the_select_all_row_moves_the_selection() {
    let mut app = app_for(&in_context(C::WithSelection));
    app.update(Message::NavigateRow(-1));
    assert!(app.context_holds(C::OnColumnSelectAll));
    assert!(app.context_holds(C::OffEpicCard));
    let cmds = press(&mut app, "L");
    assert_eq!(
        recorded(&cmds),
        vec![("move_task_forward".to_string(), Some("L".to_string()))]
    );
}

/// H with no selection and the cursor on no card moves nothing, so it records
/// nothing.
#[test]
fn h_with_nothing_to_move_records_nothing() {
    let mut app = make_app();
    app.selection_mut().set_column(1);
    app.update(Message::NavigateRow(-1));
    assert!(app.context_holds(C::OnColumnSelectAll));
    let cmds = press(&mut app, "H");
    assert!(recorded(&cmds).is_empty(), "{:?}", recorded(&cmds));
}
