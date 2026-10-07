use super::*;
use crate::models::{test_tmux_window, SubStatus, TaskId, TaskStatus, TaskTag};
// Palette constants come from the palette, never retyped as literals here: a
// hand-copied RGB goes stale silently when the palette moves, which is the exact
// drift the derived header labels were introduced to stop.
use crate::tui::ui::palette::{BORDER, GREEN, MUTED, PURPLE, RED, YELLOW};
use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use std::time::Instant;

#[tokio::test]
async fn action_hints_backlog_task() {
    let task = make_task(1, TaskStatus::Backlog);
    let hints = ui::action_hints(Some(&task), false, Color::Rgb(122, 162, 247));
    let keys: Vec<&str> = hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect();
    assert!(keys.contains(&"[Space]"), "should have dispatch hint");
    assert!(keys.contains(&"[e]"), "should have edit hint");
    assert!(keys.contains(&"[L]"), "should have move hint");
    assert!(!keys.contains(&"[H]"), "backlog has no back movement");
    assert!(keys.contains(&"[x]"), "should have delete hint");
    assert!(keys.contains(&"[n]"), "should have new hint");
    let text: String = hints.iter().map(|s| s.content.as_ref()).collect();
    // "dispatch" is the one name for starting a task, whether or not a plan is
    // attached. The label used to read "brainstorm" for a no-plan task, naming
    // a design step that no longer exists (see task #4366).
    assert!(
        text.contains("dispatch"),
        "starting a backlog task is always called dispatch, got: {text}"
    );
    assert!(
        !text.contains("brainstorm"),
        "the retired brainstorm label must not come back, got: {text}"
    );
}

#[tokio::test]
async fn action_hints_backlog_task_with_plan() {
    let mut task = make_task(3, TaskStatus::Backlog);
    task.plan_path = Some("plan.md".into());
    let hints = ui::action_hints(Some(&task), false, Color::Rgb(122, 162, 247));
    let keys: Vec<&str> = hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect();
    assert!(keys.contains(&"[Space]"), "should have dispatch hint");
    let text: String = hints.iter().map(|s| s.content.as_ref()).collect();
    assert!(
        text.contains("ispatch"),
        "backlog with plan dispatch means dispatch"
    );
}

#[tokio::test]
async fn action_hints_running_with_window() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some(test_tmux_window("win-4"));
    let hints = ui::action_hints(Some(&task), false, Color::Rgb(122, 162, 247));
    let keys: Vec<&str> = hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect();
    assert!(keys.contains(&"[Space]"), "should have go-to-session hint");
    assert!(
        !keys.contains(&"[d]"),
        "should not have dispatch/resume when window exists"
    );
}

#[tokio::test]
async fn action_hints_running_with_worktree_no_window() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = Some("/tmp/wt".to_string());
    task.tmux_window = None;
    let hints = ui::action_hints(Some(&task), false, Color::Rgb(122, 162, 247));
    let keys: Vec<&str> = hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect();
    assert!(keys.contains(&"[Space]"), "should have resume hint");
    assert!(!keys.contains(&"[d]"), "the d key is no longer bound");
    let text: String = hints.iter().map(|s| s.content.as_ref()).collect();
    assert!(text.contains("resume"), "Space means resume here");
}

/// `@guarantee RetryReachableInPlace` in docs/specs/dispatch.allium — an
/// unprovisioned Running task has nothing to jump to or resume, so Space
/// advertises the kill-and-retry recovery instead.
#[tokio::test]
async fn action_hints_running_no_worktree_no_window() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = None;
    task.tmux_window = None;
    let hints = ui::action_hints(Some(&task), false, Color::Rgb(122, 162, 247));
    let keys: Vec<&str> = hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect();
    assert!(
        !keys.contains(&"[d]"),
        "no dispatch/resume without worktree"
    );
    assert!(keys.contains(&"[Space]"), "Space offers retry");
    let text: String = hints.iter().map(|s| s.content.as_ref()).collect();
    assert!(
        text.contains("retry"),
        "Space means retry here, got {text:?}"
    );
    assert!(keys.contains(&"[e]"), "still has edit");
}

#[tokio::test]
async fn action_hints_review_with_window() {
    let mut task = make_task(6, TaskStatus::Review);
    task.tmux_window = Some(test_tmux_window("win-6"));
    let hints = ui::action_hints(Some(&task), false, Color::Rgb(122, 162, 247));
    let keys: Vec<&str> = hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect();
    assert!(
        keys.contains(&"[Space]"),
        "review with window shows go-to-session"
    );
}

#[tokio::test]
async fn action_hints_done_task() {
    let task = make_task(5, TaskStatus::Done);
    let hints = ui::action_hints(Some(&task), false, Color::Rgb(122, 162, 247));
    let keys: Vec<&str> = hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect();
    assert!(keys.contains(&"[e]"), "done has edit");
    assert!(keys.contains(&"[H]"), "done has back");
    assert!(keys.contains(&"[x]"), "done has delete");
    assert!(!keys.contains(&"[L]"), "done has no forward move");
    assert!(!keys.contains(&"[d]"), "done has no dispatch");
}

#[tokio::test]
async fn action_hints_no_task() {
    let hints = ui::action_hints(None, false, Color::Rgb(122, 162, 247));
    let keys: Vec<&str> = hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect();
    assert!(keys.contains(&"[n]"), "no-task shows new");
    assert!(!keys.contains(&"[d]"), "no-task has no dispatch");
    assert!(!keys.contains(&"[e]"), "no-task has no edit");
}

/// The `[I] learnings` footer hint went with the overlay
/// (docs/plans/archive/2026-07-31-3809-keybinding-pruning-implementation.md §3) — the footer must
/// not advertise a key that no longer has a handler.
#[tokio::test]
async fn action_hints_no_longer_advertises_learnings_key() {
    let hints = ui::action_hints(None, false, Color::Rgb(122, 162, 247));
    let keys = hint_keys(&hints);
    assert!(
        !keys.contains(&"[I]"),
        "retired learnings key must not appear"
    );
}

#[tokio::test]
async fn action_hints_backlog_shows_enter_detail() {
    let task = make_task(1, TaskStatus::Backlog);
    let hints = ui::action_hints(Some(&task), false, Color::Rgb(122, 162, 247));
    let keys = hint_keys(&hints);
    assert!(keys.contains(&"[Enter]"), "should show Enter/detail hint");
}

#[tokio::test]
async fn action_hints_shows_filter_help() {
    let task = make_task(1, TaskStatus::Backlog);
    let hints = ui::action_hints(Some(&task), false, Color::Rgb(122, 162, 247));
    let keys = hint_keys(&hints);
    assert!(keys.contains(&"[f]"), "should show filter hint");
    assert!(keys.contains(&"[?]"), "should show help hint");
}

#[tokio::test]
async fn action_hints_shows_copy_and_split() {
    let task = make_task(1, TaskStatus::Backlog);
    let hints = ui::action_hints(Some(&task), false, Color::Rgb(122, 162, 247));
    let keys = hint_keys(&hints);
    assert!(keys.contains(&"[c]"), "should show copy hint");
    assert!(keys.contains(&"[s]"), "should show split hint");
}

#[tokio::test]
async fn render_empty_board_shows_all_column_headers() {
    let mut app = App::new(vec![]);
    let buf = render_to_buffer(&mut app, 100, 20);
    assert!(buffer_contains_ignore_case(&buf, "backlog"));
    assert!(buffer_contains_ignore_case(&buf, "running"));
    assert!(buffer_contains_ignore_case(&buf, "review"));
    assert!(buffer_contains_ignore_case(&buf, "done"));
}

#[tokio::test]
async fn render_shows_task_titles_in_columns() {
    let tasks = vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Running),
        make_task(3, TaskStatus::Review),
    ];
    let mut app = App::new(tasks);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(buffer_contains(&buf, "Task 1"));
    assert!(buffer_contains(&buf, "Task 2"));
    assert!(buffer_contains(&buf, "Task 3"));
}

#[tokio::test]
async fn render_error_popup_shows_message() {
    let mut app = App::new(vec![]);
    app.update(Message::System(crate::tui::messages::SystemMessage::Error(
        "Something went wrong".to_string(),
    )));
    let buf = render_to_buffer(&mut app, 100, 20);
    assert!(buffer_contains(&buf, "Something went wrong"));
}

#[tokio::test]
async fn render_crashed_task_shows_label() {
    let mut task = make_task(1, TaskStatus::Running);
    task.tmux_window = Some(test_tmux_window("win-1"));
    task.sub_status = SubStatus::Crashed;
    let mut app = App::new(vec![task]);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(buffer_contains(&buf, "crashed"));
}

#[tokio::test]
async fn render_stale_task_shows_label() {
    let mut task = make_task(1, TaskStatus::Running);
    task.tmux_window = Some(test_tmux_window("win-1"));
    task.sub_status = SubStatus::Stale;
    let mut app = App::new(vec![task]);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(buffer_contains(&buf, "stale"));
}

#[tokio::test]
async fn running_card_with_worktree_no_window_shows_detached() {
    let mut task = make_task(1, TaskStatus::Running);
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = None;
    let mut app = App::new(vec![task]);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(buffer_contains(&buf, "○ detached"), "expected '○ detached'");
}

#[tokio::test]
async fn running_card_with_window_shows_running_not_detached() {
    let mut task = make_task(1, TaskStatus::Running);
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = Some(test_tmux_window("1-fix"));
    let mut app = App::new(vec![task]);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(buffer_contains(&buf, "◉ running"), "expected '◉ running'");
    assert!(
        !buffer_contains(&buf, "detached"),
        "should not show detached"
    );
}

#[tokio::test]
async fn review_card_with_pr_detached_shows_circle_prefix() {
    let mut task = make_task(1, TaskStatus::Review);
    task.sub_status = SubStatus::AwaitingReview;
    task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/42",
        crate::models::UrlType::Pr,
    ));
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = None;
    let mut app = App::new(vec![task]);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(buffer_contains(&buf, "○ PR #42"), "expected '○ PR #42'");
}

#[tokio::test]
async fn review_card_with_pr_attached_shows_filled_circle() {
    let mut task = make_task(1, TaskStatus::Review);
    task.sub_status = SubStatus::AwaitingReview;
    task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/42",
        crate::models::UrlType::Pr,
    ));
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = Some(test_tmux_window("1-fix"));
    let mut app = App::new(vec![task]);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(buffer_contains(&buf, "● PR #42"), "expected '● PR #42'");
}

#[tokio::test]
async fn render_does_not_panic_on_small_terminal() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    // Very small terminal — should not panic
    let _ = render_to_buffer(&mut app, 20, 5);
}

#[tokio::test]
async fn render_input_mode_shows_prompt() {
    let mut app = App::new(vec![]);
    app.update(Message::Input(
        crate::tui::messages::InputMessage::StartNewTask,
    ));
    let buf = render_to_buffer(&mut app, 100, 20);
    assert!(buffer_contains(&buf, "Title"));
}

#[tokio::test]
async fn truncate_respects_max_length() {
    assert_eq!(ui::truncate("short", 10), "short");
    assert_eq!(
        ui::truncate("hello world this is long", 10).chars().count(),
        10
    );
    assert!(ui::truncate("hello world this is long", 10).ends_with('…'));
}

/// Read back the foreground colour of the text INSIDE a `[badge]` on a card's
/// metadata line. Locates the literal `[text]` run in the buffer and returns
/// the colour of its first inner character — the brackets themselves are not
/// what carries the claim.
fn badge_fg(buf: &ratatui::buffer::Buffer, text: &str) -> Option<Color> {
    let needle = format!("[{text}]");
    let chars: Vec<char> = needle.chars().collect();
    for y in buf.area.top()..buf.area.bottom() {
        'x: for x in buf.area.left()..buf.area.right() {
            if x as usize + chars.len() > buf.area.right() as usize {
                continue;
            }
            for (i, c) in chars.iter().enumerate() {
                if buf[(x + i as u16, y)].symbol() != c.to_string() {
                    continue 'x;
                }
            }
            // First character past the opening bracket.
            return Some(buf[(x + 1, y)].fg);
        }
    }
    None
}

#[tokio::test]
async fn ci_status_badge_is_coloured_by_state() {
    // board-visuals.allium "Card label badges": labels are muted grey, and the three
    // CI-status texts are the one exception — they take the same state colours
    // the card indicator uses for the same three meanings. The exception is by
    // exact text match, so an unrecognised `ci:` value stays muted rather than
    // guessing a colour.
    let mut tasks = Vec::new();
    for (i, label) in [
        "ci:pass",
        "ci:fail",
        "ci:pending",
        "ci:flaky", // not in the vocabulary
    ]
    .iter()
    .enumerate()
    {
        let mut task = make_task(i as i64 + 1, TaskStatus::Backlog);
        // A plain label alongside the CI one, to prove ordinary badges are
        // unaffected on the very same card.
        task.labels = vec![label.to_string(), "dispatch".to_string()];
        tasks.push(task);
    }
    let mut app = App::new(tasks);
    let buf = render_to_buffer(&mut app, 160, 30);

    assert_eq!(badge_fg(&buf, "ci:pass"), Some(GREEN), "[ci:pass] is green");
    assert_eq!(badge_fg(&buf, "ci:fail"), Some(RED), "[ci:fail] is red");
    assert_eq!(
        badge_fg(&buf, "ci:pending"),
        Some(YELLOW),
        "[ci:pending] is yellow"
    );
    assert_eq!(
        badge_fg(&buf, "ci:flaky"),
        Some(MUTED),
        "an unrecognised ci: value renders as an ordinary muted badge"
    );
    assert_eq!(
        badge_fg(&buf, "dispatch"),
        Some(MUTED),
        "an ordinary label stays muted"
    );
}

#[tokio::test]
async fn render_v2_task_card_shows_stripe() {
    // board-visuals.allium "Card stripe": every card carries the quarter block ▎
    // (U+258E), the cursor card included. The superseded behaviour swapped in a
    // half block ▌ (U+258C) for the cursor — stripe weight no longer moves with
    // the cursor, because selection is carried by the hued frame and bold title.
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Backlog),
    ]);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "\u{258e}"),
        "task cards must carry the quarter-block stripe"
    );
    assert!(
        !buffer_contains(&buf, "\u{258c}"),
        "the half-block cursor stripe is superseded and must not be rendered"
    );
}

#[tokio::test]
async fn render_v2_backlog_task_shows_status_icon() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "\u{25e6}"),
        "backlog task should show \u{25e6} icon"
    );
}

#[tokio::test]
async fn render_v2_running_task_shows_status_icon() {
    let mut task = make_task(1, TaskStatus::Running);
    task.tmux_window = Some(test_tmux_window("win-1"));
    let mut app = App::new(vec![task]);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "\u{25c9}"),
        "running task should show \u{25c9} icon"
    );
}

#[tokio::test]
async fn render_v2_focused_column_shows_arrow() {
    let mut app = App::new(vec![]);
    let buf = render_to_buffer(&mut app, 120, 20);
    // Default focus is on first column (Backlog), should show \u{25b8}
    assert!(
        buffer_contains(&buf, "\u{25b8}"),
        "focused column should show \u{25b8} indicator"
    );
}

#[tokio::test]
async fn render_v2_unfocused_columns_show_dot() {
    let mut app = App::new(vec![]);
    let buf = render_to_buffer(&mut app, 120, 20);
    // Unfocused columns should show \u{25e6}
    assert!(
        buffer_contains(&buf, "\u{25e6}"),
        "unfocused columns should show \u{25e6} indicator"
    );
}

#[tokio::test]
async fn render_task_detail_overlay_shows_metadata() {
    // The old fixed detail panel is replaced by the TaskDetail overlay (Task 6).
    // Placeholder: verify that opening the overlay does not crash the renderer.
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::OpenDetail(TaskId(1)),
    ));
    let _buf = render_to_buffer(&mut app, 120, 20);
}

#[tokio::test]
async fn render_v2_done_task_shows_checkmark() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Done)]);
    // Navigate to Done column (index 3)
    for _ in 0..3 {
        app.update(Message::NavigateColumn(1));
    }
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "\u{2713}"),
        "done task should show \u{2713} icon"
    );
}

#[tokio::test]
async fn render_columns_appear_left_to_right() {
    let mut app = App::new(vec![]);
    let buf = render_to_buffer(&mut app, 120, 30);

    // Find the leftmost x-position where each header appears
    let headers = ["BACKLOG", "RUNNING", "REVIEW", "DONE"];
    let mut positions: Vec<Option<u16>> = Vec::new();
    for header in &headers {
        let mut found = None;
        for y in 0..2u16 {
            for x in 0..120u16 {
                let remaining = (120 - x) as usize;
                if remaining < header.len() {
                    continue;
                }
                let segment: String = (0..header.len() as u16)
                    .map(|dx| buf[(x + dx, y)].symbol().to_string())
                    .collect();
                if segment == *header {
                    found = Some(x);
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        positions.push(found);
    }

    // All headers must render
    for (i, header) in headers.iter().enumerate() {
        assert!(
            positions[i].is_some(),
            "column header '{header}' not found in rendered output"
        );
    }

    // Verify strict left-to-right ordering
    let xs: Vec<u16> = positions.into_iter().flatten().collect();
    for pair in xs.windows(2) {
        assert!(
            pair[0] < pair[1],
            "columns must be ordered left to right, got positions: {xs:?}"
        );
    }
}

#[tokio::test]
async fn render_columns_fill_terminal_width() {
    // Regression test: columns must use the full terminal width, not leave a gap on the right.
    // A previous bug reserved a 34-char right sidebar in the column content area.
    let mut app = App::new(vec![make_task(1, TaskStatus::Done)]);
    let width: u16 = 120;
    let buf = render_to_buffer(&mut app, width, 20);

    // Find the rightmost x-position where "done" header text appears
    let header = "DONE";
    let mut header_x = None;
    'outer: for y in 0..3u16 {
        for x in (0..width).rev() {
            let remaining = (width - x) as usize;
            if remaining < header.len() {
                continue;
            }
            let segment: String = (0..header.len() as u16)
                .map(|dx| buf[(x + dx, y)].symbol().to_string())
                .collect();
            if segment == header {
                header_x = Some(x);
                break 'outer;
            }
        }
    }
    let done_col_x = header_x.expect("'done' column header not found");

    // The "done" column header should be centered in the last quarter of the terminal.
    // With 4 columns at width=120, each column is 30 chars wide, so the last column
    // starts at x=90. The header should be somewhere after x=90.
    // If the old bug exists (34-char sidebar), each column is only ~21 chars and the
    // header would be well before x=90.
    let expected_min_x = width * 3 / 4;
    assert!(
        done_col_x >= expected_min_x,
        "last column header 'done' at x={done_col_x}, expected >= {expected_min_x} — \
         columns are not filling the terminal width"
    );
}

/// Open the help overlay and render it into a `height`-row terminal.
fn help_buffer(height: u16) -> ratatui::buffer::Buffer {
    let mut app = App::new(vec![]);
    app.update(Message::System(
        crate::tui::messages::SystemMessage::ToggleHelp,
    ));
    render_to_buffer(&mut app, 100, height)
}

/// The `[C] feed config` help line went with the popup
/// (docs/plans/archive/2026-07-31-3809-keybinding-pruning-implementation.md §6) — the help overlay
/// must not teach a key that no longer has a handler.
#[tokio::test]
async fn render_help_overlay_no_longer_teaches_feed_config_key() {
    let buf = help_buffer(40);
    assert!(
        !buffer_contains(&buf, "[C]"),
        "retired feed-config key must not appear in the help overlay"
    );
    assert!(
        !buffer_contains(&buf, "feed config"),
        "retired feed-config help text must not appear in the help overlay"
    );
}

#[tokio::test]
async fn render_1x1_terminal_does_not_panic() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Running)]);
    let _ = render_to_buffer(&mut app, 1, 1);
}

#[tokio::test]
async fn stress_large_task_list_navigation() {
    let tasks: Vec<_> = (1..=1000)
        .map(|i| make_task(i, TaskStatus::Backlog))
        .collect();
    let mut app = App::new(tasks);

    assert_eq!(app.board.tasks.len(), 1000);

    // Navigate through all rows
    for _ in 0..999 {
        app.update(Message::NavigateRow(1));
    }
    assert_eq!(app.selected_row()[0], 999);

    // Navigate back
    for _ in 0..999 {
        app.update(Message::NavigateRow(-1));
    }
    assert_eq!(app.selected_row()[0], 0);
}

#[tokio::test]
async fn stress_large_task_list_rendering() {
    let mut tasks: Vec<_> = (1..=200)
        .map(|i| make_task(i, TaskStatus::Backlog))
        .collect();
    // Spread tasks across all columns
    for (i, task) in tasks.iter_mut().enumerate() {
        task.status = match i % 4 {
            0 => TaskStatus::Backlog,
            1 => TaskStatus::Running,
            2 => TaskStatus::Review,
            _ => TaskStatus::Done,
        };
    }
    let mut app = App::new(tasks);

    // Render at various sizes — must not panic
    for width in [40, 80, 120, 200] {
        for height in [10, 24, 50] {
            let _ = render_to_buffer(&mut app, width, height);
        }
    }
}

#[tokio::test]
async fn stress_rapid_status_transitions() {
    let tasks = vec![make_task(1, TaskStatus::Backlog)];
    let mut app = App::new(tasks);

    // Rapidly move task through all statuses and back.
    // Moving forward will stop at Review because Done requires confirmation.
    for _ in 0..100 {
        app.update(Message::Task(crate::tui::messages::TaskMessage::Move {
            id: TaskId(1),
            direction: MoveDirection::Forward,
        }));
    }
    // Should be at Review (blocked by Done confirmation)
    assert_eq!(app.board.tasks[0].status, TaskStatus::Review);
    assert_eq!(app.input.mode, InputMode::ConfirmDone);

    // Confirm the Done transition
    app.update(Message::Input(
        crate::tui::messages::InputMessage::ConfirmDone,
    ));
    assert_eq!(app.board.tasks[0].status, TaskStatus::Done);

    for _ in 0..100 {
        app.update(Message::Task(crate::tui::messages::TaskMessage::Move {
            id: TaskId(1),
            direction: MoveDirection::Backward,
        }));
    }
    // Should be at Backlog (clamped)
    assert_eq!(app.board.tasks[0].status, TaskStatus::Backlog);
}

#[tokio::test]
async fn stress_db_with_many_tasks() {
    let db = crate::store::Database::open_in_memory().await.unwrap();
    use crate::store::{CreateTaskRequest, TaskCrud, TaskRead};
    for i in 0..500 {
        db.create_task(CreateTaskRequest {
            title: &format!("Task {i}"),
            description: "stress test",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();
    }
    let tasks = db.list_all().await.unwrap();
    assert_eq!(tasks.len(), 500);

    // Create app from DB tasks and verify navigation works
    let mut app = App::new(tasks);
    for _ in 0..499 {
        app.update(Message::NavigateRow(1));
    }
    assert_eq!(app.selected_row()[0], 499);
}

#[tokio::test]
async fn split_focused_defaults_to_true() {
    let app = make_app();
    assert!(app.split_focused());
}

#[tokio::test]
async fn focus_changed_updates_split_focused_when_split_active() {
    let mut app = make_app();
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("pane1".to_string());

    let cmds = app.update(Message::System(
        crate::tui::messages::SystemMessage::FocusChanged(false),
    ));
    assert!(cmds.is_empty());
    assert!(!app.split_focused());

    let cmds = app.update(Message::System(
        crate::tui::messages::SystemMessage::FocusChanged(true),
    ));
    assert!(cmds.is_empty());
    assert!(app.split_focused());
}

#[tokio::test]
async fn render_shows_border_when_split_active_and_focused() {
    let mut app = make_app();
    app.board.split.active = true;
    app.board.split.focused = true;
    app.board.split.right_pane_id = Some("pane1".to_string());

    let buf = render_to_buffer(&mut app, 80, 24);
    // Top-left corner should be a border character (╭ — rounded)
    assert_eq!(
        buf[(0, 0)].symbol(),
        "╭",
        "Expected border corner when split active"
    );
}

#[tokio::test]
async fn render_no_border_when_split_inactive() {
    let mut app = make_app();
    assert!(!app.split_active());

    let buf = render_to_buffer(&mut app, 80, 24);
    // Top-left corner should NOT be a border character
    assert_ne!(
        buf[(0, 0)].symbol(),
        "┌",
        "No border expected when split inactive"
    );
}

#[tokio::test]
async fn truncate_title_short() {
    assert_eq!(super::truncate_title("Fix bug", 30), "\"Fix bug\"");
}

#[tokio::test]
async fn truncate_title_exact_limit() {
    let title = "a".repeat(30);
    assert_eq!(super::truncate_title(&title, 30), format!("\"{}\"", title));
}

#[tokio::test]
async fn truncate_title_over_limit() {
    let title = "Refactor the authentication middleware system";
    assert_eq!(
        super::truncate_title(title, 30),
        "\"Refactor the authentication...\""
    );
}

#[tokio::test]
async fn truncate_title_multibyte_chars() {
    // Multi-byte UTF-8 characters must not panic on truncation
    let title = "Fix the caf\u{00e9} rendering bug now";
    // 31 chars, should truncate at char boundary not byte boundary
    assert!(super::truncate_title(title, 10).ends_with("...\""));
}

#[tokio::test]
async fn focused_column_ground_is_distinct_from_unfocused() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Running),
    ]);
    // Use a wider terminal so every column has room for content.
    // Columns use Ratio constraints (3/18, 2/18, ...) so they aren't equal width.
    let buf = render_to_buffer(&mut app, 240, 30);

    // board-visuals.allium "Focus is intensity, not colour-vs-absence": the focused
    // column's ground is one step lighter than an unfocused column's, and that
    // step is neutral. Check a row well below the cursor card so the assertion
    // reads column ground rather than card surface.
    let focused_bg = ui::column_bg_color(TaskStatus::Backlog, true);
    let cell = &buf[(1, 15)];
    // Backlog is 3/18 of 240 = 40px. Check well past that at x=120 (middle of board).
    let cell2 = &buf[(120, 15)];

    assert_eq!(
        cell.bg, focused_bg,
        "Focused column should carry the focused ground"
    );
    assert_ne!(
        cell2.bg, focused_bg,
        "An unfocused column's ground must differ from the focused one's"
    );
    assert_ne!(
        cell2.bg,
        Color::Rgb(26, 27, 38),
        "The board ground is painted, not left at the bare terminal background"
    );
}

#[tokio::test]
async fn on_select_all_defaults_to_false() {
    let app = make_app();
    assert!(!app.on_select_all());
}

#[tokio::test]
async fn select_all_column_selects_all_tasks_in_column() {
    let mut app = make_app();
    // Cursor is on Backlog (column 0) which has tasks 1, 2
    app.update(Message::SelectAllColumn);
    assert!(app.select.tasks.contains(&TaskId(1)));
    assert!(app.select.tasks.contains(&TaskId(2)));
    assert_eq!(app.select.tasks.len(), 2);
}

#[tokio::test]
async fn select_all_column_deselects_when_all_selected() {
    let mut app = make_app();
    app.update(Message::SelectAllColumn);
    assert_eq!(app.select.tasks.len(), 2);

    app.update(Message::SelectAllColumn);
    assert!(app.select.tasks.is_empty());
}

#[tokio::test]
async fn select_all_column_selects_remaining_when_partially_selected() {
    let mut app = make_app();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleSelect(TaskId(1)),
    ));
    assert_eq!(app.select.tasks.len(), 1);

    app.update(Message::SelectAllColumn);
    assert!(app.select.tasks.contains(&TaskId(1)));
    assert!(app.select.tasks.contains(&TaskId(2)));
    assert_eq!(app.select.tasks.len(), 2);
}

#[tokio::test]
async fn select_all_column_noop_on_empty_column() {
    let mut app = make_app();
    // Navigate to Review column (empty in make_app)
    app.update(Message::NavigateColumn(2));
    app.update(Message::SelectAllColumn);
    assert!(app.select.tasks.is_empty());
}

/// Every position in `buf` whose symbol is exactly `sym`.
pub(super) fn cells_with_symbol<'a>(
    buf: &'a ratatui::buffer::Buffer,
    sym: &'a str,
) -> Vec<&'a ratatui::buffer::Cell> {
    let mut out = Vec::new();
    for y in buf.area.top()..buf.area.bottom() {
        for x in buf.area.left()..buf.area.right() {
            let cell = &buf[(x, y)];
            if cell.symbol() == sym {
                out.push(cell);
            }
        }
    }
    out
}

mod palette_and_frames;
mod selection_reorder_and_flash;
