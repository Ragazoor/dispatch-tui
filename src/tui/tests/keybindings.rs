//! The keybinding drift gate (`KeybindingDriftGate.press_every_row_key` in
//! `docs/specs/keybindings.allium`) for every board namespace the board's
//! dispatcher receives: with a row's context set up, pressing each of its keys
//! records the row's action id. Plus the dispatch-through-the-table rules
//! (`KeypressRunsItsRowsAction`, `KeyWithoutARowCannotAct`) and the context
//! evaluation the gate and the handler share (`context_holds`).
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::keybindings::{
    default_bindings_in, namespaces_matching, KeyBinding, KeyContext as C, KeyNamespace,
    ANY_OTHER_KEY, KEY_BINDINGS,
};
use crate::models::test_pane_id;
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
    unlisted_key_in(KEY_BINDINGS, ns)
}

/// [`unlisted_key`] for the rows of `table`.
fn unlisted_key_in(table: &[KeyBinding], ns: KeyNamespace) -> &'static str {
    ["w", "Q", "%", "W", "_"]
        .into_iter()
        .find(|k| {
            !table
                .iter()
                .any(|b| b.namespace == ns && b.keys.contains(k))
        })
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
        Some(C::InsideEpicView) | Some(C::InsideNestedView) | Some(C::NestedViewNoSearch) => {
            inside_epic_view()
        }
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
            app.folds
                .toggle(TaskStatus::Running, ColumnSection::NeedsInput);
            app.selection_mut().set_row(2, 0);
            app
        }
        Some(C::OnFoldedEpicGroup) => {
            let mut a = make_task(1, TaskStatus::Running);
            a.epic_id = Some(EpicId(10));
            let mut app = App::new(vec![a]);
            app.board.epics = vec![make_epic(10)];
            app.filter.lens = crate::tui::types::Lens::Flat;
            app.selection_mut().set_column(2);
            app.epic_folds.toggle(crate::tui::types::EpicFoldRef::new(
                TaskStatus::Running,
                ColumnSection::Active,
                EpicId(10),
            ));
            app.selection_mut().set_row(2, 0);
            app
        }
        Some(C::OnEpicCard) => make_app_with_epic_selected(),
        Some(C::OnWorkPackageCard) => app_with_package_selected(),
        Some(C::OnFlattenedTaskCard) => {
            let mut app = app_with_running_task(|t| t.epic_id = Some(EpicId(10)));
            app.board.epics = vec![make_epic(10)];
            app.filter.lens = crate::tui::types::Lens::Flat;
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
            // These record only when they act: give each a card to move or a
            // group to fold.
            "toggle_epic_fold" => app_for(&in_context(C::OnFoldedEpicGroup)),
            "reorder_task_down" | "reorder_task_up" => {
                let mut app = make_app();
                app.selection_mut().set_column(1);
                let row = if action == "reorder_task_down" { 0 } else { 1 };
                app.selection_mut().set_row(1, row);
                app
            }
            "open_pr_url" => app_with_running_task(|t| {
                t.url = Some(crate::models::TaskUrl::new(
                    "https://github.com/example/repo/pull/42",
                    crate::models::UrlType::Pr,
                ));
            }),
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
            app.board.split.right_pane_id = Some(test_pane_id("%42"));
            app.board.split.pinned_task_id = Some(TaskId(7));
            app
        }
        Some(C::TaskWindowSplitOpen) => {
            let mut app = app_with_running_task(|_| {});
            app.board.split.active = true;
            app.board.split.right_pane_id = Some(test_pane_id("%42"));
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

/// A board whose only card is work package #1's, under the cursor.
fn app_with_package_selected() -> App {
    let mut app = App::new(vec![]);
    let at = chrono::DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp");
    app.board.work_packages = vec![crate::models::WorkPackage {
        id: crate::models::WorkPackageId(1),
        title: "Window".to_string(),
        description: String::new(),
        sort_order: None,
        created_by: None,
        created_at: at,
        updated_at: at,
    }];
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    app
}

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
        N::BoardDescriptionEditor => [InputMode::InputDescription, InputMode::InputEpicDescription]
            .into_iter()
            .map(draft_app)
            .collect(),
        N::BoardText => [
            InputMode::InputTitle,
            InputMode::InputEpicTitle,
            InputMode::InputWorkPackageTitle(crate::models::WorkPackageItem::Task(TaskId(1))),
            InputMode::RenameWorkPackage(crate::models::WorkPackageId(1)),
            InputMode::InputDisplayName,
        ]
        .into_iter()
        .map(draft_app)
        .collect(),
        N::BoardFilter => {
            let mut app = make_app();
            // Nine repos so every digit 1-9 toggles one.
            app.board.repo_paths = (1..=9).map(|i| format!("/repo/r{i}")).collect();
            // Epic 11 is a child of 10, so the fold keys have a parent to
            // fold under the cursor.
            let mut child = make_epic(11);
            child.parent_epic_id = Some(EpicId(10));
            app.board.epics = vec![make_epic(10), child];
            app.update(Message::BoardFilter(
                crate::tui::messages::BoardFilterMessage::Start,
            ));
            assert_eq!(app.input.mode, InputMode::BoardFilter);
            // The Epics-tab rows act only there; every other row acts on the
            // Repos tab the overlay opens on.
            if binding.action.starts_with("filter_") && binding.action.ends_with("_epics") {
                app.filter_overlay.tab = crate::tui::types::FilterTab::Epics;
            }
            one(app)
        }
        N::BoardFilterSearch => {
            let mut app = make_app();
            app.board.epics = vec![make_epic(10)];
            app.update(Message::BoardFilter(
                crate::tui::messages::BoardFilterMessage::Start,
            ));
            app.filter_overlay.tab = crate::tui::types::FilterTab::Epics;
            app.update(Message::BoardFilter(
                crate::tui::messages::BoardFilterMessage::StartEpicSearch,
            ));
            assert_eq!(app.input.mode, InputMode::BoardFilterSearch);
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
        N::BoardPickerModel => one(draft_app(InputMode::InputModel)),
        N::BoardPickerQuickDispatch => one(draft_app(InputMode::QuickDispatch)),
        N::BoardPickerMoveToEpic => one(with_move_task_picker()),
        N::BoardPickerReparentEpic => one(with_reparent_picker()),
        N::BoardPickerWorkPackage => {
            let mut app = app_with_package_selected();
            app.input.mode = InputMode::LinkWorkPackage {
                item: crate::models::WorkPackageItem::Task(TaskId(1)),
                cursor: 0,
            };
            one(app)
        }
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
        N::BoardConfirmDeleteWorkPackage => one(draft_app(InputMode::ConfirmDeleteWorkPackage(
            crate::models::WorkPackageId(1),
        ))),
        N::BoardConfirmStartOrchestrator => {
            let mut app = inside_epic_view();
            app.input.mode = InputMode::ConfirmStartOrchestrator(EpicId(10));
            one(app)
        }
        N::BoardConfirmResolveItem => one(draft_app(InputMode::ConfirmResolveItem(1))),
        N::BoardKeyEditor => one(key_editor_app(InputMode::KeyEditor)),
        N::BoardKeyEditorSearch => one(key_editor_app(InputMode::KeyEditorSearch)),
        N::BoardKeyCapture => one(key_editor_app(InputMode::KeyCapture)),
        N::BoardConfirmDiscardKeyEdits => one(key_editor_app(InputMode::ConfirmDiscardKeyEdits)),
        N::AgentTreeTree
        | N::AgentTreeCommits
        | N::AgentTreeAgents
        | N::AgentDiff
        | N::TmuxGlobal => {
            panic!("{} is not received by the board", ns.name())
        }
    }
}

/// The key editor in `mode`, its cursor on a row in the middle of the
/// table with four staged keys, the second picked: every motion, removal,
/// restore and save has something to act on.
pub(super) fn key_editor_app(mode: InputMode) -> App {
    let mut app = make_app();
    app.key_override_file = Some(std::path::PathBuf::from("/nonexistent/keybindings.json"));
    let mut editor = crate::tui::key_editor::KeyEditor::default();
    let row = KEY_BINDINGS
        .iter()
        .position(|b| b.namespace == KeyNamespace::BoardText && b.action == "text_cursor_word_left")
        .unwrap();
    editor.cursor = editor
        .visible(KEY_BINDINGS)
        .iter()
        .position(|&r| r == row)
        .unwrap();
    editor.stage(
        row,
        ["Ctrl+Left", "Alt+Left", "Alt+B", "F5"]
            .map(String::from)
            .to_vec(),
    );
    editor.picked = 1;
    if mode == InputMode::KeyCapture {
        editor.capture = Some(crate::tui::key_editor::CaptureKind::AddKey);
    }
    app.input.key_editor = Some(Box::new(editor));
    app.input.mode = mode;
    app
}

fn board_namespaces() -> Vec<KeyNamespace> {
    KeyNamespace::ALL
        .iter()
        .copied()
        .filter(|n| n.name().starts_with("board."))
        .collect()
}

/// press_every_row_key / RecordedActionMatchesRow over `table`, for every
/// board namespace: the failures, and how many presses were made. A
/// catch-all row is pressed with a key no other row of its namespace lists,
/// and records the key actually pressed.
fn press_every_board_row_key(table: &'static [KeyBinding]) -> (Vec<String>, usize) {
    let mut failures = Vec::new();
    let mut pressed = 0;
    for ns in board_namespaces() {
        for binding in table.iter().filter(|b| b.namespace == ns) {
            let keys: Vec<&str> = if binding.keys == [ANY_OTHER_KEY] {
                vec![unlisted_key_in(table, ns)]
            } else {
                binding.keys.to_vec()
            };
            for key in keys {
                for (i, mut app) in apps_in(binding).into_iter().enumerate() {
                    app.set_key_table(table);
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
    (failures, pressed)
}

#[test]
fn pressing_each_key_of_each_board_row_records_the_rows_action() {
    let (failures, pressed) = press_every_board_row_key(KEY_BINDINGS);
    assert!(pressed > 0);
    assert!(
        failures.is_empty(),
        "rows whose keys do not record their action:\n{}",
        failures.join("\n")
    );
}

/// RemappedTableDrivesDispatch: the same gate over the remapped table, where
/// every pair of opposite rows trades keys and every value family rotates
/// them. Each key records the row that lists it now.
#[test]
fn under_the_remapped_table_each_key_records_the_row_that_lists_it_now() {
    let (failures, pressed) = press_every_board_row_key(crate::keybindings::remapped_table());
    assert!(pressed > 0);
    assert!(
        failures.is_empty(),
        "remapped rows whose keys do not record their action:\n{}",
        failures.join("\n")
    );
}

/// What a press did to the board, for comparing two presses: the state a
/// motion or a pick moves (the view, the input mode, field and draft, the
/// cursors and scroll offsets, the tree pickers' cursors), the screen (drawn
/// with the default table, so the help overlay's title, which names the
/// table's keys, does not differ) and the commands other than usage.
fn effect_of(app: &mut App, cmds: &[Command]) -> String {
    let tree = |state: &std::cell::RefCell<tui_tree_widget::TreeState<String>>| {
        let state = state.borrow();
        let mut opened: Vec<_> = state.opened().iter().cloned().collect();
        opened.sort();
        format!("{:?} {opened:?}", state.selected())
    };
    let pickers = format!(
        "{:?} {:?}",
        app.interaction
            .move_task_picker
            .as_ref()
            .map(|p| tree(&p.tree_state)),
        app.interaction
            .reparent_picker
            .as_ref()
            .map(|p| tree(&p.tree_state))
    );
    let state = format!(
        "{:?}\n{:?}\n{:?} {} {}\n{:?}\n{}\n{pickers}",
        app.board.view_mode,
        app.input.mode,
        app.input.buffer,
        app.input.caret,
        app.input.repo_cursor,
        app.input.task_draft,
        app.interaction.help_scroll,
    );
    let state = format!(
        "{state}\n{:?}",
        app.input
            .key_editor
            .as_ref()
            .map(|e| (e.cursor, e.picked, &e.staged, &e.query, e.capture))
    );
    let table = app.key_table;
    app.set_key_table(KEY_BINDINGS);
    let buf = render_to_buffer(app, 120, 40);
    app.set_key_table(table);
    let area = buf.area();
    let screen: Vec<String> = (area.top()..area.bottom())
        .map(|y| buffer_line(&buf, y))
        .collect();
    let commands: Vec<String> = cmds
        .iter()
        .filter(|c| !matches!(c, Command::Usage(_)))
        .map(|c| format!("{c:?}"))
        .collect();
    // Fixture tasks are stamped with the time they were made, so two boards
    // built a moment apart differ only there.
    static STAMP: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\d{4}-\d\d-\d\dT[\d:.]+Z").unwrap());
    let effect = format!("{state}\n{}\n{commands:?}", screen.join("\n"));
    STAMP.replace_all(&effect, "<time>").into_owned()
}

/// A tree picker over a parent epic with a child and a third epic, its
/// cursor on the first row so each motion, expand and collapse has
/// something to move.
fn tree_picker_app(ns: KeyNamespace) -> App {
    let mut app = make_app();
    let mut child = make_epic(11);
    child.parent_epic_id = Some(EpicId(10));
    app.board.epics = vec![make_epic(10), child, make_epic(12)];
    if ns == KeyNamespace::BoardPickerMoveToEpic {
        app.update(Message::Task(
            crate::tui::messages::TaskMessage::StartMoveToEpic(TaskId(1)),
        ));
    } else {
        app.update(Message::Epic(
            crate::tui::messages::EpicMessage::StartReparent(EpicId(12)),
        ));
    }
    assert_eq!(app.key_namespace(), ns);
    // The tree widget moves over the rows it drew last, so draw around the
    // press as the real loop does.
    render_to_buffer(&mut app, 120, 40);
    press(&mut app, "j");
    render_to_buffer(&mut app, 120, 40);
    app
}

/// The boards the effect check presses on: the gate's, with enough to move
/// for a motion to show — a detail scrolled into its middle, a text field
/// with words either side of its caret, a tree with a child epic and a task waiting to be moved to Done.
fn effect_apps(binding: &KeyBinding) -> Vec<App> {
    use KeyNamespace as N;
    let ns = binding.namespace;
    let mut apps = match ns {
        N::BoardPickerMoveToEpic | N::BoardPickerReparentEpic => vec![tree_picker_app(ns)],
        _ => apps_in(binding),
    };
    for app in &mut apps {
        match ns {
            N::BoardDetail => {
                if let ViewMode::TaskDetail {
                    scroll, max_scroll, ..
                } = &mut app.board.view_mode
                {
                    *scroll = 2;
                    *max_scroll = 10;
                }
            }
            N::BoardText => {
                app.input.set_buffer("one two three".to_string());
                app.input.caret = 5;
            }
            // A query every candidate matches, so the list keeps rows for
            // the cursor to move over.
            N::BoardPickerRepoPath | N::BoardPickerQuickDispatch => {
                app.input.set_buffer("/repo/".to_string());
                app.input.caret = 3;
            }
            N::BoardPickerBaseBranch => {
                app.board.repo_base_branches.insert(
                    "/repo/a".to_string(),
                    vec!["main".to_string(), "main-next".to_string()],
                );
                app.input.set_buffer("main".to_string());
                app.input.caret = 2;
            }
            // A task waiting to be confirmed, so yes has one to move.
            N::BoardConfirmDone => app.prompt_move_to_done(vec![TaskId(3)]),
            _ => {}
        }
    }
    apps
}

/// RemappedTableDrivesDispatch, by effect: under the remapped table each key
/// of a moved row does what the row's default key does under the default
/// table, which is the row's own direction or value. A handler that read the
/// pressed key would go the way the key used to go. Each moved row's effect
/// is also checked to be visible from its setup for at least one row of
/// every pair or family, so the comparison is not vacuous.
#[test]
fn under_the_remapped_table_each_key_goes_the_way_its_row_says_on_the_board() {
    use crate::keybindings::{moved_keys, remap_group, remapped_table};
    let remapped = remapped_table();
    let mut failures = Vec::new();
    let mut moved = std::collections::BTreeSet::new();
    let mut visible = std::collections::BTreeSet::new();
    // What each row's boards look like before any press, built once a row.
    let mut untouched: std::collections::HashMap<*const KeyBinding, Vec<String>> =
        std::collections::HashMap::new();
    for ns in board_namespaces() {
        for (row, key, default_key) in moved_keys(remapped, ns) {
            let group = format!("{} {}", ns.name(), remap_group(row.action));
            moved.insert(group.clone());
            let before = untouched.entry(row).or_insert_with(|| {
                effect_apps(row)
                    .iter_mut()
                    .map(|app| effect_of(app, &[]))
                    .collect()
            });
            for (i, before) in before.iter().enumerate() {
                let mut by_default = effect_apps(row).remove(i);
                let default_cmds = press(&mut by_default, default_key);
                let want = effect_of(&mut by_default, &default_cmds);

                let mut by_remap = effect_apps(row).remove(i);
                by_remap.set_key_table(remapped);
                let remap_cmds = press(&mut by_remap, key);
                let got = effect_of(&mut by_remap, &remap_cmds);

                let actions = |cmds: &[Command]| -> Vec<String> {
                    recorded(cmds).into_iter().map(|(a, _)| a).collect()
                };
                if got != want || actions(&remap_cmds) != actions(&default_cmds) {
                    let differs = got
                        .lines()
                        .zip(want.lines())
                        .find(|(g, w)| g != w)
                        .map(|(g, w)| format!("\n    got:  {g}\n    want: {w}"))
                        .unwrap_or_default();
                    failures.push(format!(
                        "{} {}: {key} under the remap did not do what {default_key} does by default{differs}",
                        ns.name(),
                        row.action
                    ));
                }
                // A pair or family is one group: an "up" at the top of a list
                // changes nothing, and its "down" shows the pair is read the
                // right way round.
                if want != *before {
                    visible.insert(group.clone());
                }
            }
        }
    }
    assert!(!moved.is_empty());
    assert!(
        failures.is_empty(),
        "keys that did not follow their row:\n{}",
        failures.join("\n")
    );
    let unseen: Vec<&String> = moved.difference(&visible).collect();
    assert!(
        unseen.is_empty(),
        "moved pairs and families with no visible effect from their setup: {unseen:?}"
    );
}

/// A chord the table moves works on the board like the shipped one, and
/// `gg` no longer arms once no row lists it (KeypressRunsItsRowsAction: a
/// character is the first half of a chord exactly when a row of the
/// namespace lists a chord starting with it).
#[test]
fn a_chord_moved_to_another_row_key_works_on_the_board() {
    let free = ['&', '%', '_', '!']
        .into_iter()
        .find(|c| {
            !default_bindings_in(KeyNamespace::BoardNormal)
                .any(|b| b.keys.iter().any(|k| k.starts_with(*c)))
        })
        .expect("a character board.normal does not use");
    let chord: &'static str = Box::leak(format!("{free}{free}").into_boxed_str());
    let first: &'static str = Box::leak(free.to_string().into_boxed_str());
    let mut app = make_app();
    app.set_key_table(crate::keybindings::table_with_keys(
        KeyNamespace::BoardNormal,
        "navigate_row_first",
        Box::leak(vec!["[", chord].into_boxed_slice()),
    ));
    app.selection_mut().set_column(1);
    press(&mut app, "]");
    let last = app.selection().row(1);
    assert!(last > 0, "the column has more than one card");

    press(&mut app, "g");
    let cmds = press(&mut app, "g");
    assert_eq!(app.selection().row(1), last, "gg is no row's key any more");
    assert!(recorded(&cmds).is_empty(), "{cmds:?}");

    let cmds = press(&mut app, first);
    assert!(recorded(&cmds).is_empty(), "the first half records nothing");
    let cmds = app.handle_key(key_event_for(first));
    assert_eq!(app.selection().row(1), 0);
    assert_eq!(
        recorded(&cmds),
        vec![("navigate_row_first".to_string(), Some(chord.to_string()))]
    );
}

/// The gate above is only as strong as the rows it presses: every board
/// namespace must contribute at least one.
#[test]
fn the_gate_presses_a_row_in_every_board_namespace() {
    let unpressed: Vec<&str> = board_namespaces()
        .into_iter()
        .filter(|ns| default_bindings_in(*ns).next().is_none())
        .map(|ns| ns.name())
        .collect();
    assert!(unpressed.is_empty(), "no rows to press in {unpressed:?}");
}

/// scroll_help_down and scroll_help_up are rows the gate presses for
/// board.help.
#[test]
fn the_gate_presses_scroll_help() {
    for (action, keys) in [
        ("scroll_help_down", ["j", "Down"]),
        ("scroll_help_up", ["k", "Up"]),
    ] {
        let binding = default_bindings_in(KeyNamespace::BoardHelp)
            .find(|b| b.action == action)
            .unwrap_or_else(|| panic!("board.help has a {action} row"));
        for key in keys {
            assert!(binding.keys.contains(&key), "{key}");
            let mut app = apps_in(binding).remove(0);
            let cmds = press(&mut app, key);
            assert_eq!(
                recorded(&cmds),
                vec![(action.to_string(), Some(key.to_string()))]
            );
            assert_eq!(
                app.input.mode,
                InputMode::Help,
                "{key} keeps the overlay open"
            );
        }
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
        N::BoardFilterSearch,
        N::BoardText,
        N::BoardPickerRepoPath,
        N::BoardPickerBaseBranch,
        N::BoardPickerQuickDispatch,
        N::BoardKeyEditorSearch,
        // CaptureTakesThePress: a key with no row there is the key captured.
        N::BoardKeyCapture,
    ];
    let mut checked = 0;
    for ns in board_namespaces() {
        if typing.contains(&ns) || default_bindings_in(ns).any(|b| b.keys.contains(&ANY_OTHER_KEY))
        {
            continue;
        }
        let Some(sample) = default_bindings_in(ns).next() else {
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

/// The real table with board.normal's delete_task row answering `V` instead
/// of `x`, and nothing else changed.
fn table_with_delete_rebound_to_o() -> &'static [KeyBinding] {
    let rows: Vec<KeyBinding> = KEY_BINDINGS
        .iter()
        .map(|b| {
            if b.namespace == KeyNamespace::BoardNormal && b.action == "delete_task" {
                assert_eq!(
                    b.keys,
                    &["x"],
                    "the fixture assumes x is delete_task's only key"
                );
                KeyBinding { keys: &["V"], ..*b }
            } else {
                *b
            }
        })
        .collect();
    assert!(
        !rows.iter().any(|b| b.namespace == KeyNamespace::BoardNormal
            && b.keys.contains(&"V")
            && b.action != "delete_task"),
        "V must be free in board.normal for this fixture"
    );
    Box::leak(rows.into_boxed_slice())
}

#[test]
fn a_rebound_key_runs_the_rows_action() {
    let mut app = on_running_card(make_app());
    app.set_key_table(table_with_delete_rebound_to_o());
    let cmds = press(&mut app, "V");
    assert_eq!(
        recorded(&cmds),
        vec![("delete_task".to_string(), Some("V".to_string()))]
    );
    assert_eq!(
        app.input.mode,
        InputMode::ConfirmDone,
        "V on a Running task runs delete_task, which asks to move it to Done"
    );
}

#[test]
fn the_key_a_row_no_longer_lists_does_nothing() {
    let mut app = on_running_card(make_app());
    app.set_key_table(table_with_delete_rebound_to_o());
    let cmds = press(&mut app, "x");
    assert!(
        cmds.is_empty(),
        "x has no row any more: {} commands",
        cmds.len()
    );
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// The table the board loads from the override file in its data directory
/// (`LoadKeyTable`), installed the way startup installs it.
fn app_loaded_from(file: &str) -> (App, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(crate::keybindings::KEY_OVERRIDE_FILE_NAME),
        file,
    )
    .unwrap();
    let mut app = on_running_card(make_app());
    app.install_key_table(&crate::keybindings::load_key_table(
        dir.path(),
        crate::keybindings::KeyTableReader::Board,
    ));
    (app, dir)
}

const DELETE_ON_O: &str =
    r#"[{"namespace": "board.normal", "action": "delete_task", "keys": ["V"]}]"#;

#[test]
fn the_board_answers_a_key_from_the_override_file() {
    // Under the default table, x on a Running task asks to move it to Done.
    let mut by_default = on_running_card(make_app());
    press(&mut by_default, "x");
    assert_eq!(by_default.input.mode, InputMode::ConfirmDone);

    let (mut app, _dir) = app_loaded_from(DELETE_ON_O);
    assert_eq!(app.error_popup(), None, "a good file reports nothing");
    let cmds = press(&mut app, "V");
    assert_eq!(
        recorded(&cmds),
        vec![("delete_task".to_string(), Some("V".to_string()))]
    );
    assert_eq!(app.input.mode, InputMode::ConfirmDone, "V has x's effect");

    let (mut app, _dir) = app_loaded_from(DELETE_ON_O);
    let cmds = press(&mut app, "x");
    assert!(cmds.is_empty(), "x is no row's key any more: {cmds:?}");
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn a_refused_entry_is_shown_and_the_rest_of_the_file_applies() {
    let (mut app, dir) = app_loaded_from(
        r#"[{"namespace": "board.normal", "action": "delete_task", "keys": ["V"]},
            {"namespace": "board.normal", "action": "navigate_row_last", "keys": ["ctrl+e"]}]"#,
    );
    let shown = app
        .error_popup()
        .expect("the load error is shown")
        .to_string();
    assert!(shown.contains("write \"Ctrl+E\""), "{shown}");
    let path = dir
        .path()
        .join(crate::keybindings::KEY_OVERRIDE_FILE_NAME)
        .display()
        .to_string();
    assert!(shown.contains(&path), "{shown}");
    let last = app
        .key_table
        .iter()
        .find(|b| b.namespace == KeyNamespace::BoardNormal && b.action == "navigate_row_last")
        .unwrap();
    let default = default_bindings_in(KeyNamespace::BoardNormal)
        .find(|b| b.action == "navigate_row_last")
        .unwrap();
    assert_eq!(last.keys, default.keys, "the refused row keeps its keys");

    press(&mut app, "q");
    assert_eq!(app.error_popup(), None, "any key dismisses the error");
    press(&mut app, "V");
    assert_eq!(
        app.input.mode,
        InputMode::ConfirmDone,
        "the good entry applies"
    );
}

/// Every refused entry's message, and the dismiss hint, fit an 80x24
/// board, even when one message lists a whole namespace's actions.
#[test]
fn the_key_load_errors_fit_a_small_board() {
    let (mut app, _dir) = app_loaded_from(
        r#"[{"namespace": "board.normal", "action": "jump_rope", "keys": ["V"]},
            {"namespace": "board.normal", "action": "navigate_row_last", "keys": ["ctrl+e"]}]"#,
    );
    let buf = render_to_buffer(&mut app, 80, 24);
    let area = buf.area();
    let text: String = (area.top()..area.bottom())
        .map(|y| {
            (area.left()..area.right())
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Press any key to dismiss"), "{text}");
    assert!(text.contains("Ctrl+E"), "{text}");

    // Many refusals of the longest kind still leave the hint on screen.
    let many: Vec<String> = (0..10)
        .map(|n| {
            format!(
                r#"{{"namespace": "board.normal", "action": "no_such_action_{n}", "keys": ["V"]}}"#
            )
        })
        .collect();
    let (mut app, _dir) = app_loaded_from(&format!("[{}]", many.join(",")));
    let buf = render_to_buffer(&mut app, 80, 24);
    let text: String = (area.top()..area.bottom())
        .map(|y| {
            (area.left()..area.right())
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Press any key to dismiss"), "{text}");
    assert!(text.contains("7 more"), "{text}");
}

/// A later error does not replace the key-load notice: the error shows
/// first, and the notice once that is dismissed.
#[test]
fn a_later_error_does_not_hide_the_key_load_errors() {
    let (mut app, _dir) = app_loaded_from(
        r#"[{"namespace": "board.normal", "action": "navigate_row_last", "keys": ["ctrl+e"]}]"#,
    );
    app.handle_error("dispatch failed".to_string());
    assert_eq!(app.error_popup(), Some("dispatch failed"));
    press(&mut app, "q");
    assert!(app.error_popup().unwrap().contains("Ctrl+E"));
    press(&mut app, "q");
    assert_eq!(app.error_popup(), None);
}

#[test]
fn an_unreadable_file_leaves_the_board_on_its_default_keys() {
    let (mut app, _dir) = app_loaded_from("not json");
    assert!(app.error_popup().unwrap().contains("no override applies"));
    assert!(std::ptr::eq(app.key_table, KEY_BINDINGS));
    press(&mut app, "q");
    press(&mut app, "x");
    assert_eq!(app.input.mode, InputMode::ConfirmDone);
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
    let cmds = press(&mut app, "V");
    assert!(cmds.is_empty(), "V has no row in the real table");
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
            Some(C::NestedViewNoSearch),
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
    let esc = [C::SearchActive, C::NestedViewNoSearch, C::WithSelection];
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
    for binding in default_bindings_in(KeyNamespace::BoardNormal) {
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
            let probe = probe(ns);
            assert!(!apps_in(&probe).is_empty(), "{}", ns.name());
        }
    }
}

// ---------------------------------------------------------------------------
// Mode-bar hints
// ---------------------------------------------------------------------------

/// The hint keys inside every `[...]` of `line`, split on `/`: `[q/Esc]`
/// gives `q` and `Esc`, `[/]` gives `/`. Checkbox and tree marks (`[x]`, `[ ]`,
/// `[-]`, `[~]`; `[x]` only where x is not a key of the mode) and the board's badges (`[flat]`, `[active]`, `[/query]`,
/// `[hidden: …]`, `[2/9 repos]`) are not hints. An empty `[]` is kept as the
/// key `[]`: a hint built from a row that is not there, which must fail.
fn bracketed_keys(line: &str, is_key: impl Fn(&str) -> bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find(']') else {
            break;
        };
        let inner = &rest[open + 1..open + close];
        rest = &rest[open + close + 1..];
        let badge = matches!(inner, "flat" | "active")
            || inner.starts_with("hidden:")
            || inner.ends_with(" repos")
            || (inner.starts_with('/') && inner.len() > 1);
        let mark = matches!(inner.trim(), "x" | "-" | "~") && !is_key(inner);
        if badge || mark || inner == " " {
            continue;
        }
        match inner {
            "" => out.push("[]".to_string()),
            "/" => out.push("/".to_string()),
            _ => out.extend(inner.split('/').map(|k| k.trim().to_string())),
        }
    }
    out
}

/// The keys a line names without brackets ("Enter to select", "j/k scroll ·
/// q/Esc/Enter close", "q/Esc:cancel"): each word, split on `/` and `:`,
/// that `is_key` accepts.
fn bare_keys(line: &str, is_key: impl Fn(&str) -> bool) -> Vec<String> {
    line.split_whitespace()
        .flat_map(|w| w.split([':', '/']))
        .map(|w| w.trim_matches(|c: char| ",.;?·()".contains(c)))
        .filter(|k| !k.is_empty() && is_key(k))
        .map(str::to_string)
        .collect()
}

/// A row-less binding standing for "the mode `ns` with nothing else set up".
fn probe(ns: KeyNamespace) -> KeyBinding {
    KeyBinding {
        namespace: ns,
        keys: &[],
        context: None,
        action: "",
        description: "",
        note: None,
        records_usage: true,
    }
}

/// The table form of a key as a hint draws it: `Del`, `↑` and `↓` are the
/// hints' short forms of Delete, Up and Down.
fn table_form(hint: &str) -> &str {
    crate::keybindings::SHORT_KEY_FORMS
        .iter()
        .find(|(_, short)| *short == hint)
        .map_or(hint, |(long, _)| long)
}

/// The modes whose hints name their keys without brackets.
const BARE_HINT_NAMESPACES: [KeyNamespace; 3] = [
    KeyNamespace::BoardPickerMoveToEpic,
    KeyNamespace::BoardPickerReparentEpic,
    KeyNamespace::BoardConfirmDeleteRepoPath,
];

/// One line of hints a mode drew: the namespace, which of its boards, the
/// row of the screen, the line and the keys read from it.
struct HintLine {
    ns: KeyNamespace,
    board: usize,
    y: u16,
    line: String,
    keys: Vec<String>,
}

/// The rows of its own namespace a board's hints were built from: the
/// namespace, which of its boards, and the actions asked for.
type HintRows = Vec<(KeyNamespace, usize, Vec<String>)>;

/// Every board each mode of HintsNameOnlyTableKeys is read on.
fn hint_boards(ns: KeyNamespace) -> Vec<App> {
    match ns {
        // One board per row's context, so every status bar the cursor's
        // card can draw is read.
        KeyNamespace::BoardNormal | KeyNamespace::BoardDetail => default_bindings_in(ns)
            .flat_map(|b| match ns {
                KeyNamespace::BoardNormal => vec![app_for(b)],
                _ => apps_in(b),
            })
            .collect(),
        KeyNamespace::BoardFilter => [FilterTab::Repos, FilterTab::Epics, FilterTab::Packages]
            .into_iter()
            .flat_map(|tab| {
                apps_in(&probe(ns)).into_iter().map(move |mut app| {
                    app.filter_overlay.tab = tab;
                    app
                })
            })
            .collect(),
        _ => apps_in(&probe(ns)),
    }
}

/// HintsNameOnlyTableKeys: the hint lines every board mode draws under
/// `table`. board.normal's are its status bar and the indicator row above
/// the board (the two rows at either edge, framed or not); board.detail's are its status bar and the detail's footer;
/// every other mode's are the lines it draws over the plain board (its popup,
/// form prompt, overlay footer and status bar).
fn read_hints(table: &'static [KeyBinding]) -> Vec<HintLine> {
    read_hints_in(table, None).0
}

/// [`read_hints`] over every namespace, or only the given boards of one,
/// with the rows each board's hints were built from.
fn read_hints_in(
    table: &'static [KeyBinding],
    only: Option<(KeyNamespace, &[usize])>,
) -> (Vec<HintLine>, HintRows) {
    // Wide enough that no status bar is cut off, so a longer user key
    // cannot push a hint out of the frame.
    const W: u16 = 300;
    const H: u16 = 50;
    let mut out = Vec::new();
    let mut rows = Vec::new();
    for ns in board_namespaces() {
        if matches!(ns, KeyNamespace::BoardHelp | KeyNamespace::BoardError)
            || only.is_some_and(|(o, _)| o != ns)
        {
            continue;
        }
        let is_key = |k: &str| {
            let k = table_form(k);
            [KEY_BINDINGS, table]
                .iter()
                .any(|t| t.iter().any(|b| b.namespace == ns && b.keys.contains(&k)))
        };
        for (board, mut app) in hint_boards(ns).into_iter().enumerate() {
            if only.is_some_and(|(_, boards)| !boards.contains(&board)) {
                continue;
            }
            app.set_key_table(table);
            let (buf, asked) =
                crate::keybindings::record_hint_rows(|| render_to_buffer(&mut app, W, H));
            let mut asked: Vec<String> = asked
                .into_iter()
                .filter(|(n, _)| *n == ns)
                .map(|(_, action)| action)
                .collect();
            asked.sort();
            asked.dedup();
            rows.push((ns, board, asked));
            let line = |y: u16| buffer_line(&buf, y);
            let mut push = |y: u16, keys: Vec<String>| {
                out.push(HintLine {
                    ns,
                    board,
                    y,
                    line: line(y).trim().to_string(),
                    keys,
                });
            };
            match ns {
                KeyNamespace::BoardNormal => {
                    // With the split pane open the board is framed, so its
                    // indicator row and status bar sit one row in.
                    for y in [0, 1, H - 2, H - 1] {
                        push(y, bracketed_keys(&line(y), is_key));
                    }
                }
                KeyNamespace::BoardDetail => {
                    // The footer is the detail's bottom border: the last line
                    // the detail draws over the board it was opened from.
                    app.board.view_mode = ViewMode::Board(Default::default());
                    let board_buf = render_to_buffer(&mut app, W, H);
                    let footer = (0..H - 1)
                        .rev()
                        .find(|&y| line(y) != buffer_line(&board_buf, y))
                        .expect("the detail draws over the board");
                    push(footer, bare_keys(&line(footer), is_key));
                    push(H - 1, bracketed_keys(&line(H - 1), is_key));
                }
                _ => {
                    // The board drawn under the mode (its header's `[N]` bell,
                    // its cards) is board.normal's, not the mode's: a line the
                    // mode leaves as the plain board draws it is not its hint.
                    app.input.mode = InputMode::Normal;
                    let board_buf = render_to_buffer(&mut app, W, H);
                    for y in 0..H {
                        let text = line(y);
                        if text == buffer_line(&board_buf, y) {
                            continue;
                        }
                        let keys = if BARE_HINT_NAMESPACES.contains(&ns) {
                            bare_keys(&text, is_key)
                        } else {
                            bracketed_keys(&text, is_key)
                        };
                        if !keys.is_empty() {
                            push(y, keys);
                        }
                    }
                }
            }
        }
    }
    (out, rows)
}

/// Every key read is a key of its mode's namespace in `table` (or stands for
/// its catch-all row), and every mode read names at least one key.
fn hint_failures(table: &'static [KeyBinding], read: &[HintLine]) -> Vec<String> {
    let mut failures = Vec::new();
    for ns in board_namespaces() {
        if matches!(ns, KeyNamespace::BoardHelp | KeyNamespace::BoardError) {
            continue;
        }
        if !read.iter().any(|h| h.ns == ns && !h.keys.is_empty()) {
            failures.push(format!("{}: the mode drew no hint", ns.name()));
        }
    }
    for h in read {
        let rows: Vec<&KeyBinding> = table.iter().filter(|b| b.namespace == h.ns).collect();
        for hint in &h.keys {
            let key = table_form(hint);
            let known = rows
                .iter()
                .any(|b| b.keys.contains(&key) || b.keys.contains(&ANY_OTHER_KEY));
            if hint == "[]" || !known {
                failures.push(format!("{}: `{hint}` in {:?}", h.ns.name(), h.line));
            }
        }
    }
    failures
}

/// HintsNameOnlyTableKeys under the default table: every key hint anywhere
/// in the frame a board mode draws names a key of that mode's namespace.
#[test]
fn every_bracketed_hint_a_mode_draws_is_a_key_of_that_mode() {
    let read = read_hints(KEY_BINDINGS);
    let checked: usize = read.iter().map(|h| h.keys.len()).sum();
    assert!(checked > 100, "the test read too few hints ({checked})");
    let failures = hint_failures(KEY_BINDINGS, &read);
    assert!(
        failures.is_empty(),
        "hints naming a key the mode does not have:\n{}",
        failures.join("\n")
    );
}

/// HintsNameOnlyTableKeys under the remapped table of
/// RemappedTableDrivesDispatch: every hint names a key its mode answers now,
/// and each key stands where its row's default key stood, so `[j/k] move`
/// reads `[k/j] move` once the two rows trade keys. A hint written by hand
/// passes the first check whenever the remap only swaps keys; the second
/// catches it.
#[test]
fn under_the_remapped_table_every_hint_names_the_users_key() {
    let remapped = crate::keybindings::remapped_table();
    let by_default = read_hints(KEY_BINDINGS);
    let by_remap = read_hints(remapped);
    let mut failures = hint_failures(remapped, &by_remap);
    assert_eq!(
        by_default.len(),
        by_remap.len(),
        "the two reads drew different boards"
    );
    let mut moved = 0;
    for (d, r) in by_default.iter().zip(&by_remap) {
        assert_eq!((d.ns, d.board, d.y), (r.ns, r.board, r.y));
        if d.keys.len() != r.keys.len() {
            failures.push(format!("{}: {:?} became {:?}", d.ns.name(), d.line, r.line));
            continue;
        }
        for (dk, rk) in d.keys.iter().zip(&r.keys) {
            let (dk, rk) = (table_form(dk), table_form(rk));
            // The rows whose default keys list the key drawn by default; a
            // catch-all's stand-in (`n` of `[y/n]`) has none.
            let rows: Vec<usize> = (0..KEY_BINDINGS.len())
                .filter(|&i| {
                    KEY_BINDINGS[i].namespace == d.ns && KEY_BINDINGS[i].keys.contains(&dk)
                })
                .collect();
            if rows.is_empty() {
                continue;
            }
            if dk != rk {
                moved += 1;
            }
            if !rows.iter().any(|&i| remapped[i].keys.contains(&rk)) {
                failures.push(format!(
                    "{}: `{dk}` of {:?} became `{rk}` in {:?}, not a key of its row",
                    d.ns.name(),
                    d.line,
                    r.line
                ));
            }
        }
    }
    assert!(moved > 20, "the remap moved too few drawn keys ({moved})");
    assert!(
        failures.is_empty(),
        "hints that do not follow the remap:\n{}",
        failures.join("\n")
    );
}

/// HintsNameOnlyTableKeys, one row at a time: for every row a drawn hint is
/// built from, a table with only that row rebound draws the new key on that
/// board. The remapped table leaves most rows where they were; this reaches
/// the rest, so a hint written by hand beside a row the remap does not move
/// fails too.
#[test]
fn each_hint_names_its_row_when_that_row_alone_is_rebound() {
    let (drawn, asked) = read_hints_in(KEY_BINDINGS, None);
    let mut failures = Vec::new();
    // A hint written by hand asks no row: every drawn key that is a key of
    // its mode must be a key of a row the board's hints were built from.
    for h in &drawn {
        let built_from = asked
            .iter()
            .find(|(ns, board, _)| *ns == h.ns && *board == h.board)
            .map_or(&[][..], |(_, _, actions)| actions.as_slice());
        for hint in &h.keys {
            let key = table_form(hint);
            let rows: Vec<&KeyBinding> = KEY_BINDINGS
                .iter()
                .filter(|b| b.namespace == h.ns && b.keys.contains(&key))
                .collect();
            if !rows.is_empty()
                && !rows
                    .iter()
                    .any(|b| built_from.iter().any(|a| a == b.action))
            {
                failures.push(format!(
                    "{} (board #{}): `{hint}` in {:?} is built from no row",
                    h.ns.name(),
                    h.board,
                    h.line
                ));
            }
        }
    }
    let mut by_row: std::collections::BTreeMap<(&str, String), (KeyNamespace, Vec<usize>)> =
        std::collections::BTreeMap::new();
    for (ns, board, actions) in &asked {
        for action in actions {
            by_row
                .entry((ns.name(), action.clone()))
                .or_insert((*ns, Vec::new()))
                .1
                .push(*board);
        }
    }
    assert!(
        by_row.len() > 100,
        "too few rows were drawn ({})",
        by_row.len()
    );
    for ((_, action), (ns, boards)) in &by_row {
        let listed = |k: &str| {
            KEY_BINDINGS
                .iter()
                .any(|b| b.namespace == *ns && b.keys.contains(&k))
        };
        let (key, keys): (&str, &'static [&'static str]) = if !listed("Ctrl+T") {
            ("Ctrl+T", &["Ctrl+T"])
        } else {
            ("Ctrl+Y", &["Ctrl+Y"])
        };
        let table = crate::keybindings::table_with_keys(*ns, action, keys);
        let (lines, _) = read_hints_in(table, Some((*ns, boards)));
        for board in boards {
            let shown = lines
                .iter()
                .any(|h| h.board == *board && h.keys.iter().any(|k| k == key));
            if !shown {
                failures.push(format!(
                    "{} {action} (board #{board}): rebound to {key}, no hint names it",
                    ns.name()
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "hints that do not follow their own row:\n{}",
        failures.join("\n")
    );
}

/// HintsNameOnlyTableKeys for the prompts a handler sets as the status
/// message on opening a mode (the drawn frames above read the status bar's
/// fallback): under the remapped table they name the user's keys.
#[test]
fn prompts_set_on_opening_a_mode_name_the_users_keys() {
    use crate::keybindings::keys_of;
    let remapped = crate::keybindings::remapped_table();
    let first = |ns, action| keys_of(remapped, ns, action)[0];
    let delete = first(KeyNamespace::BoardNormal, "delete_task");

    let mut done_card = app_for(&in_context(C::TaskWithoutWorktree));
    done_card.set_key_table(remapped);
    press(&mut done_card, delete);
    assert!(matches!(
        done_card.input.mode,
        InputMode::ConfirmDeleteTask(_)
    ));
    let yes = first(KeyNamespace::BoardConfirmDeleteTask, "confirm_delete_yes");
    assert_ne!(yes, "y", "the remap must move the yes key");
    let message = done_card.status_message().unwrap_or_default().to_string();
    assert!(message.ends_with(&format!("[{yes}/n]")), "{message}");

    let mut running = on_running_card(make_app());
    running.set_key_table(remapped);
    press(&mut running, delete);
    assert_eq!(running.input.mode, InputMode::ConfirmDone);
    let yes = first(KeyNamespace::BoardConfirmDone, "confirm_done_yes");
    let message = running.status_message().unwrap_or_default().to_string();
    assert!(message.ends_with(&format!("[{yes}/n]")), "{message}");

    let mut stale = app_with_running_task(|t| {
        t.tmux_window = None;
        t.sub_status = SubStatus::Stale;
    });
    stale.set_key_table(remapped);
    stale.handle_kill_and_retry(TaskId(7));
    let resume = first(KeyNamespace::BoardConfirmRetry, "confirm_retry_resume");
    assert_ne!(resume, "r", "the remap must move the resume key");
    let message = stale.status_message().unwrap_or_default().to_string();
    assert!(message.contains(&format!("[{resume}] Resume")), "{message}");
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
    let binding = default_bindings_in(KeyNamespace::BoardPickerTag)
        .find(|b| b.action == "tag_picker_select_bug")
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
        if !default_bindings_in(ns).any(|b| b.keys.contains(&ANY_OTHER_KEY)) {
            continue;
        }
        let binding = default_bindings_in(ns)
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

/// ModifiedPressDoesNotBorrowTheBareRow: the Ctrl form of every unmodified
/// key of every board row (letters of either case, digits, punctuation,
/// Space and the named keys), when no row lists that Ctrl form, runs nothing
/// and records nothing: no command at all, and the mode is unchanged.
#[test]
fn a_modified_bare_key_with_no_row_does_nothing() {
    let mut seen = std::collections::HashSet::new();
    let mut failures = Vec::new();
    // CaptureTakesThePress: there a modified press is the key captured.
    for ns in board_namespaces()
        .into_iter()
        .filter(|ns| *ns != KeyNamespace::BoardKeyCapture)
    {
        for binding in default_bindings_in(ns) {
            for key in binding.keys {
                if *key == ANY_OTHER_KEY || *key == "gg" || key.contains('+') {
                    continue;
                }
                let base = key_event_for(key);
                let ctrl = KeyEvent::new(base.code, base.modifiers | KeyModifiers::CONTROL);
                let name = crate::keybindings::key_name(ctrl);
                if default_bindings_in(ns).any(|b| b.keys.contains(&name.as_str()))
                    || !seen.insert((ns, binding.action, binding.context, name.clone()))
                {
                    continue;
                }
                for (i, mut app) in apps_in(binding).into_iter().enumerate() {
                    let mode = app.input.mode.clone();
                    let cmds = app.handle_key(ctrl);
                    if !cmds.is_empty() || app.input.mode != mode {
                        failures.push(format!(
                            "{} {name} (row {} #{i}): {cmds:?}, mode {:?} -> {:?}",
                            ns.name(),
                            binding.action,
                            mode,
                            app.input.mode
                        ));
                    }
                }
            }
        }
    }
    assert!(seen.len() > 150, "{}", seen.len());
    assert!(
        failures.is_empty(),
        "modified presses that acted:\n{}",
        failures.join("\n")
    );
}

/// A modified letter is never typed into a text field.
#[test]
fn a_modified_letter_is_not_typed_into_a_text_field() {
    let binding = default_bindings_in(KeyNamespace::BoardText).next().unwrap();
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
    let binding = default_bindings_in(KeyNamespace::BoardText).next().unwrap();
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

/// UnrecordedRowsRecordNothing, for every records_usage false row of every
/// board namespace: each key of the row, pressed with "ab cd ef" in the field
/// and the caret between `c` and `d`, edits the text or moves the caret as
/// the row's action says, and records nothing. board.search,
/// board.filter_search and board.key_editor_search have no caret; their
/// Backspace drops the last char.
#[test]
fn every_unrecorded_row_takes_its_effect_and_records_nothing() {
    use KeyNamespace as N;
    const SEED: &str = "ab cd ef";
    let mut checked = 0;
    let mut failures = Vec::new();
    for ns in board_namespaces() {
        for binding in default_bindings_in(ns).filter(|b| !b.records_usage) {
            for key in binding.keys {
                for (i, mut app) in apps_in(binding).into_iter().enumerate() {
                    // (text, caret) after the press; caret None for a field
                    // without one.
                    let got = match ns {
                        N::BoardSearch => {
                            app.search.query = SEED.to_string();
                            let cmds = press(&mut app, key);
                            (app.search.query.clone(), None, cmds)
                        }
                        N::BoardFilterSearch => {
                            app.filter_overlay.epic_query = SEED.to_string();
                            let cmds = press(&mut app, key);
                            (app.filter_overlay.epic_query.clone(), None, cmds)
                        }
                        N::BoardKeyEditorSearch => {
                            let query =
                                |app: &App| app.input.key_editor.as_ref().unwrap().query.clone();
                            app.input.key_editor.as_mut().unwrap().query = SEED.to_string();
                            let cmds = press(&mut app, key);
                            (query(&app), None, cmds)
                        }
                        _ => {
                            app.input.set_buffer(SEED.to_string());
                            app.input.caret = 4;
                            let cmds = press(&mut app, key);
                            (app.input.buffer.clone(), Some(app.input.caret), cmds)
                        }
                    };
                    let want: (&str, Option<usize>) = match (ns, binding.action) {
                        (
                            N::BoardSearch | N::BoardFilterSearch | N::BoardKeyEditorSearch,
                            "text_backspace",
                        ) => ("ab cd e", None),
                        (_, "text_backspace") => ("ab d ef", Some(3)),
                        (_, "text_delete_forward") => ("ab c ef", Some(4)),
                        (_, "text_cursor_left") => (SEED, Some(3)),
                        (_, "text_cursor_right") => (SEED, Some(5)),
                        (_, "text_cursor_home") => (SEED, Some(0)),
                        (_, "text_cursor_end") => (SEED, Some(8)),
                        (_, "text_cursor_word_left") => (SEED, Some(3)),
                        (_, "text_cursor_word_right") => (SEED, Some(6)),
                        (_, other) => panic!(
                            "{} {other}: no expected effect for this unrecorded row",
                            ns.name()
                        ),
                    };
                    checked += 1;
                    let (text, caret, cmds) = got;
                    if (text.as_str(), caret) != want || !no_recorded_usage(&cmds) {
                        failures.push(format!(
                            "{} {key} {} (#{i}): want {want:?}, got ({text:?}, {caret:?}), recorded {:?}",
                            ns.name(),
                            binding.action,
                            recorded(&cmds)
                        ));
                    }
                }
            }
        }
    }
    assert!(checked >= 40, "only {checked} presses checked");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Backspace in the search bar edits the query and records nothing.
#[test]
fn backspace_in_the_search_bar_edits_the_query_silently() {
    let binding = default_bindings_in(KeyNamespace::BoardSearch)
        .next()
        .unwrap();
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

/// A board.normal key with nothing to act on records nothing: on the
/// select-all row there is no card to select, copy or reorder and no
/// section or epic group to fold (KeyInput's navigation-only exception).
#[test]
fn card_keys_with_nothing_under_the_cursor_record_nothing() {
    for key in ["v", "c", "J", "K", "z", "Z"] {
        let mut app = make_app();
        app.selection_mut().set_column(1);
        app.update(Message::NavigateRow(-1));
        assert!(app.context_holds(C::OnColumnSelectAll));
        let mode = app.input.mode.clone();
        let cmds = press(&mut app, key);
        assert!(recorded(&cmds).is_empty(), "{key}: {:?}", recorded(&cmds));
        assert_eq!(app.input.mode, mode, "{key}");
    }
}

/// `v` on a work package card selects nothing, so it records nothing.
#[test]
fn toggle_select_on_a_work_package_card_records_nothing() {
    let mut app = app_for(&in_context(C::OnWorkPackageCard));
    assert!(app.context_holds(C::OnWorkPackageCard));
    let cmds = press(&mut app, "v");
    assert!(recorded(&cmds).is_empty(), "{:?}", recorded(&cmds));
    assert!(!app.has_selection());
}

/// Enter in the quick-dispatch picker with no repo under the cursor and no new
/// path typed selects nothing, so it records nothing and the picker stays open.
#[test]
fn quick_dispatch_select_with_no_repo_records_nothing() {
    let mut app = make_app();
    app.input.mode = InputMode::QuickDispatch;
    app.input.clear_buffer();
    assert!(app.board.repo_paths.is_empty());
    let cmds = press(&mut app, "Enter");
    assert!(recorded(&cmds).is_empty(), "{:?}", recorded(&cmds));
    assert_eq!(app.input.mode, InputMode::QuickDispatch);
}

/// In the filter overlay's Epics tab, `z` on a leaf and `Z` with no epic
/// that has children fold nothing, so they record nothing.
#[test]
fn filter_folds_with_nothing_to_fold_record_nothing() {
    let binding = default_bindings_in(KeyNamespace::BoardFilter)
        .find(|b| b.action == "filter_fold_epics")
        .unwrap();
    let mut app = apps_in(binding).remove(0);
    app.board.epics = vec![make_epic(10)];
    app.filter_overlay.tab = FilterTab::Epics;
    app.filter_overlay.epic_cursor = 0;
    for key in ["z", "Z"] {
        let cmds = press(&mut app, key);
        assert!(recorded(&cmds).is_empty(), "{key}: {:?}", recorded(&cmds));
    }
}

/// While the Epics tab's title search is typed, the overlay's footer names
/// the search's own keys, not the tab's.
#[test]
fn the_filter_search_footer_names_the_search_keys() {
    let binding = default_bindings_in(KeyNamespace::BoardFilterSearch)
        .next()
        .unwrap();
    let mut app = apps_in(binding).remove(0);
    let buf = render_to_buffer(&mut app, 160, 50);
    assert!(buffer_contains_ignore_case(&buf, "[Backspace] delete"));
    assert!(!buffer_contains_ignore_case(&buf, "[z] fold"));
}

#[test]
fn every_board_normal_action_has_a_handler_arm() {
    use crate::keybindings::{default_bindings_in, KeyNamespace};
    let mut missing = Vec::new();
    for b in default_bindings_in(KeyNamespace::BoardNormal) {
        let mut app = App::new(vec![]);
        if app.try_run_normal(b, "x").is_none() {
            missing.push(b.action);
        }
    }
    assert!(
        missing.is_empty(),
        "actions with no handler arm: {missing:?}"
    );
}
