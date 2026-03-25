# Interactive Agents Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Change agent dispatch from autonomous headless mode to interactive sessions with jump-to-window and resume support.

**Architecture:** Minimal delta on existing Elm Architecture. Modify dispatch to use interactive claude CLI, add resume_agent(), make Command::Cleanup handle optional tmux_window, extend tick loop to monitor all live windows, add context-sensitive `d` key and `g` key in input handler.

**Tech Stack:** Rust, ratatui, tmux, Claude Code CLI

**Spec:** `docs/plans/2026-03-25-interactive-agents-design.md`

---

### File map

| File | Change | Responsibility |
|---|---|---|
| `src/tmux.rs` | Add `select_window()` | tmux subprocess wrappers |
| `src/dispatch.rs` | Update `cleanup_task()` signature, change `dispatch_agent()` to interactive, add `resume_agent()` | Agent lifecycle |
| `src/tui/mod.rs` | Update `Command::Cleanup`, add `Message::ResumeTask`/`Resumed`, `Command::Resume`/`JumpToTmux`, update `WindowGone`/`Tick`/`DispatchTask`/`MoveTask` handlers | State machine |
| `src/tui/input.rs` | Context-sensitive `d` key, add `g` key | Keybinding logic |
| `src/main.rs` | Handle `Command::Resume`, `Command::JumpToTmux`, update `Command::Cleanup` handler | Command execution |

---

### Task 1: Add `tmux::select_window()`

**Files:**
- Modify: `src/tmux.rs`

- [ ] **Step 1: Write the test for select_window_args helper**

Add to the `#[cfg(test)] mod tests` block in `src/tmux.rs`:

```rust
#[test]
fn select_window_args_correct() {
    let args = select_window_args("task-42");
    assert_eq!(args, vec!["select-window", "-t", "task-42"]);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib tmux::tests::select_window_args_correct`
Expected: FAIL — `select_window_args` not found

- [ ] **Step 3: Implement select_window_args and select_window**

Add the internal helper above `new_window_args` in `src/tmux.rs`:

```rust
fn select_window_args(window: &str) -> Vec<String> {
    vec![
        "select-window".to_string(),
        "-t".to_string(),
        window.to_string(),
    ]
}
```

Add the public function above `new_window`:

```rust
/// Switch the active tmux window to the one with the given name.
pub fn select_window(window: &str) -> Result<()> {
    let args = select_window_args(window);
    let status = Command::new("tmux")
        .args(&args)
        .status()
        .context("failed to spawn tmux select-window")?;
    if !status.success() {
        bail!("tmux select-window failed with status {}", status);
    }
    Ok(())
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib tmux::tests::select_window_args_correct`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/tmux.rs
git commit -m "feat: add tmux::select_window() for jumping to agent windows"
```

---

### Task 2: Make `cleanup_task()` accept optional tmux_window

**Files:**
- Modify: `src/dispatch.rs`

- [ ] **Step 1: Update `cleanup_task` signature and body**

Change the function signature in `src/dispatch.rs` from:

```rust
pub fn cleanup_task(repo_path: &str, worktree_path: &str, tmux_window: &str) -> Result<()> {
```

to:

```rust
pub fn cleanup_task(repo_path: &str, worktree_path: &str, tmux_window: Option<&str>) -> Result<()> {
```

Update the tmux window kill block to wrap the existing logic:

```rust
if let Some(window) = tmux_window {
    match tmux::has_window(window) {
        Ok(true) => {
            tmux::kill_window(window)
                .context("failed to kill tmux window during cleanup")?;
        }
        Ok(false) => {}
        Err(e) => {
            eprintln!("warning: could not check tmux window during cleanup: {e}");
        }
    }
}
```

- [ ] **Step 2: Update the call site in `dispatch_agent` cleanup (inside `execute_commands`)**

This call is in `src/main.rs` inside the `Command::Dispatch` arm at the `dispatch::cleanup_task` call. Change:

```rust
if let Err(e) = dispatch::cleanup_task(&repo_path, wt, tw) {
```

to:

```rust
if let Err(e) = dispatch::cleanup_task(&repo_path, wt, Some(tw.as_str())) {
```

Note: `tw` is `&String` from the pattern match, so `.as_str()` is needed since `Option<&String>` doesn't coerce to `Option<&str>`. This call site is removed entirely in Task 12 step 3.

- [ ] **Step 3: Update the `Command::Cleanup` handler in `main.rs`**

Change the `Command::Cleanup` arm in `execute_commands()`. The current destructure is:

```rust
Command::Cleanup { repo_path, worktree, tmux_window } => {
```

Update the `dispatch::cleanup_task` call inside it:

```rust
if let Err(e) = dispatch::cleanup_task(&repo_path, &worktree, &tmux_window) {
```

to:

```rust
if let Err(e) = dispatch::cleanup_task(&repo_path, &worktree, tmux_window.as_deref()) {
```

- [ ] **Step 4: Update `Command::Cleanup` enum variant in `tui/mod.rs`**

Change the variant from:

```rust
Cleanup { repo_path: String, worktree: String, tmux_window: String },
```

to:

```rust
Cleanup { repo_path: String, worktree: String, tmux_window: Option<String> },
```

- [ ] **Step 5: Update backward-move cleanup emission in `tui/mod.rs`**

In the `MoveTask` handler, change the cleanup match from:

```rust
match (task.worktree.take(), task.tmux_window.take()) {
    (Some(wt), Some(tw)) => Some(Command::Cleanup {
        repo_path: task.repo_path.clone(),
        worktree: wt,
        tmux_window: tw,
    }),
    _ => None,
}
```

to:

```rust
match task.worktree.take() {
    Some(wt) => Some(Command::Cleanup {
        repo_path: task.repo_path.clone(),
        worktree: wt,
        tmux_window: task.tmux_window.take(),
    }),
    None => {
        task.tmux_window.take(); // clear even if no worktree
        None
    },
}
```

- [ ] **Step 6: Run all tests to verify nothing broke**

Run: `cargo test`
Expected: All existing tests pass. The `move_backward_from_running_emits_cleanup` test should still pass since `tmux_window` was `Some` in that test fixture.

- [ ] **Step 7: Commit**

```bash
git add src/dispatch.rs src/tui/mod.rs src/main.rs
git commit -m "refactor: make cleanup_task accept optional tmux_window"
```

---

### Task 3: Change `dispatch_agent()` to interactive mode

**Files:**
- Modify: `src/dispatch.rs`

- [ ] **Step 1: Update the prompt launch in dispatch_agent**

Replace the current lines 69-75 (prompt file write + `claude -p < .claude-prompt` send-keys):

```rust
// 5. Write the prompt file and launch Claude in print mode.
let prompt = build_prompt(task_id, title, description, mcp_port);
let prompt_file = format!("{worktree_path}/.claude-prompt");
fs::write(&prompt_file, &prompt)
    .with_context(|| format!("failed to write {prompt_file}"))?;
tmux::send_keys(&tmux_window, "claude -p < .claude-prompt")
    .context("failed to send keys to tmux window")?;
```

with:

```rust
// 5. Write prompt file and launch Claude in interactive mode.
let prompt = build_prompt(task_id, title, description, mcp_port);
let prompt_file = format!("{worktree_path}/.claude-prompt");
fs::write(&prompt_file, &prompt)
    .with_context(|| format!("failed to write {prompt_file}"))?;
tmux::send_keys(&tmux_window, "claude \"$(cat .claude-prompt)\"")
    .context("failed to send keys to tmux window")?;
```

- [ ] **Step 2: Run existing tests**

Run: `cargo test --lib dispatch::tests`
Expected: All pass (tests don't invoke tmux, they test `build_prompt` and `expand_tilde`)

- [ ] **Step 3: Commit**

```bash
git add src/dispatch.rs
git commit -m "feat: dispatch agents in interactive mode instead of print mode"
```

---

### Task 4: Add `resume_agent()` function

**Files:**
- Modify: `src/dispatch.rs`
- Modify: `src/models.rs`

- [ ] **Step 1: Add ResumeResult to models.rs**

Add after the `DispatchResult` struct in `src/models.rs`:

```rust
#[derive(Debug, Clone)]
pub struct ResumeResult {
    pub tmux_window: String,
}
```

- [ ] **Step 2: Write the test for resume_agent arguments**

We can't test the full function (requires tmux), but we can test that `build_resume_window_name` produces the right name. Add to `src/dispatch.rs` tests:

```rust
#[test]
fn resume_window_name_matches_dispatch() {
    // The resume window name should use the same naming convention as dispatch
    assert_eq!(build_tmux_window_name(42), "task-42");
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test --lib dispatch::tests::resume_window_name_matches_dispatch`
Expected: FAIL — `build_tmux_window_name` not found

- [ ] **Step 4: Extract window name helper and implement resume_agent**

First, extract the tmux window name logic into a helper in `src/dispatch.rs` (currently inlined as `format!("task-{task_id}")`):

```rust
fn build_tmux_window_name(task_id: i64) -> String {
    format!("task-{task_id}")
}
```

Update `dispatch_agent` to use it:

```rust
let tmux_window = build_tmux_window_name(task_id);
```

Then add the `resume_agent` function:

```rust
/// Re-open a tmux window for an existing worktree and resume the most recent
/// Claude conversation with `claude --continue`.
///
/// This function is **synchronous** and should be called via
/// `tokio::task::spawn_blocking` from async contexts.
pub fn resume_agent(
    task_id: i64,
    worktree_path: &str,
) -> Result<ResumeResult> {
    let tmux_window = build_tmux_window_name(task_id);

    // 1. Create a new tmux window at the existing worktree.
    tmux::new_window(&tmux_window, worktree_path)
        .context("failed to create tmux window for resume")?;

    // 2. Launch Claude in continue mode (picks up most recent conversation).
    tmux::send_keys(&tmux_window, "claude --continue")
        .context("failed to send resume keys to tmux window")?;

    Ok(ResumeResult { tmux_window })
}
```

Add `use crate::models::{DispatchResult, ResumeResult, slugify};` at the top (update the existing import).

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib dispatch::tests`
Expected: All pass including new test

- [ ] **Step 6: Commit**

```bash
git add src/dispatch.rs src/models.rs
git commit -m "feat: add resume_agent() for continuing closed sessions"
```

---

### Task 5: Add new Message/Command variants

**Files:**
- Modify: `src/tui/mod.rs`

- [ ] **Step 1: Add new variants to Message enum**

Add to the `Message` enum in `src/tui/mod.rs`:

```rust
ResumeTask(i64),
Resumed { id: i64, tmux_window: String },
```

- [ ] **Step 2: Add new variants to Command enum**

Add to the `Command` enum:

```rust
Resume { task: Task },
JumpToTmux { window: String },
```

- [ ] **Step 3: Add placeholder handlers in App::update()**

Add match arms in `App::update()` before the `Message::Error` arm:

```rust
Message::ResumeTask(id) => {
    if let Some(task) = self.tasks.iter().find(|t| t.id == id) {
        if task.worktree.is_some() && task.tmux_window.is_none() {
            vec![Command::Resume { task: task.clone() }]
        } else {
            vec![]
        }
    } else {
        vec![]
    }
}

Message::Resumed { id, tmux_window } => {
    if let Some(task) = self.tasks.iter_mut().find(|t| t.id == id) {
        task.tmux_window = Some(tmux_window);
        let task_clone = task.clone();
        vec![Command::PersistTask(task_clone)]
    } else {
        vec![]
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: All pass (new variants have handlers, no exhaustiveness errors)

- [ ] **Step 5: Commit**

```bash
git add src/tui/mod.rs
git commit -m "feat: add ResumeTask/Resumed messages and Resume/JumpToTmux commands"
```

---

### Task 6: Update WindowGone handler

**Files:**
- Modify: `src/tui/mod.rs`

- [ ] **Step 1: Write test for new WindowGone behavior**

Add to tests in `src/tui/mod.rs`:

```rust
#[test]
fn window_gone_clears_tmux_window_and_persists() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = Some("/repo/.worktrees/4-task-4".to_string());
    task.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task]);

    let cmds = app.update(Message::WindowGone(4));

    // Task should stay Running
    let task = app.tasks.iter().find(|t| t.id == 4).unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    // tmux_window should be cleared
    assert!(task.tmux_window.is_none());
    // worktree should be preserved
    assert!(task.worktree.is_some());
    // Should emit PersistTask to write cleared tmux_window to DB
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0], Command::PersistTask(t) if t.tmux_window.is_none()));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib tui::tests::window_gone_clears_tmux_window_and_persists`
Expected: FAIL — current handler auto-advances to Review

- [ ] **Step 3: Replace the WindowGone handler**

Replace the current `Message::WindowGone` arm in `App::update()`:

```rust
Message::WindowGone(id) => {
    // Only auto-advance if the task is still Running
    if let Some(task) = self.tasks.iter().find(|t| t.id == id) {
        if task.status == TaskStatus::Running {
            return self.update(Message::MoveTask {
                id,
                direction: MoveDirection::Forward,
            });
        }
    }
    vec![]
}
```

with:

```rust
Message::WindowGone(id) => {
    if let Some(task) = self.tasks.iter_mut().find(|t| t.id == id) {
        task.tmux_window = None;
        let task_clone = task.clone();
        vec![Command::PersistTask(task_clone)]
    } else {
        vec![]
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: New test passes. The old `move_backward_from_running_emits_cleanup` test still passes. Check that no other test relied on WindowGone auto-advancing.

- [ ] **Step 5: Commit**

```bash
git add src/tui/mod.rs
git commit -m "feat: WindowGone clears tmux_window without advancing status"
```

---

### Task 7: Extend tick loop to capture all live windows

**Files:**
- Modify: `src/tui/mod.rs`

- [ ] **Step 1: Write test for tick capturing Review tasks with tmux_window**

Add to tests in `src/tui/mod.rs`:

```rust
#[test]
fn tick_captures_review_task_with_live_window() {
    let mut task = make_task(5, TaskStatus::Review);
    task.tmux_window = Some("task-5".to_string());
    let mut app = App::new(vec![task]);

    let cmds = app.update(Message::Tick);

    assert!(cmds.iter().any(|c| matches!(c, Command::CaptureTmux { id: 5, .. })));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib tui::tests::tick_captures_review_task_with_live_window`
Expected: FAIL — current tick only captures Running tasks

- [ ] **Step 3: Update the Tick handler filter**

In `App::update()`, replace the `Message::Tick` arm's task filter:

```rust
let mut cmds: Vec<Command> = self
    .tasks
    .iter()
    .filter(|t| t.status == TaskStatus::Running)
    .filter_map(|t| {
```

with:

```rust
let mut cmds: Vec<Command> = self
    .tasks
    .iter()
    .filter(|t| t.tmux_window.is_some())
    .filter_map(|t| {
```

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: All pass including old `tick_produces_capture_for_running_tasks_with_window` test (Running tasks with a window still have `tmux_window.is_some()`)

- [ ] **Step 5: Commit**

```bash
git add src/tui/mod.rs
git commit -m "feat: tick loop captures tmux output for any task with live window"
```

---

### Task 8: Add forward-cleanup-on-Done in MoveTask

**Files:**
- Modify: `src/tui/mod.rs`

- [ ] **Step 1: Write test for forward move to Done emitting cleanup**

Add to tests in `src/tui/mod.rs`:

```rust
#[test]
fn move_forward_to_done_emits_cleanup() {
    let mut task = make_task(5, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/5-task-5".to_string());
    task.tmux_window = None; // session closed, but worktree remains
    let mut app = App::new(vec![task]);

    let cmds = app.update(Message::MoveTask {
        id: 5,
        direction: MoveDirection::Forward,
    });

    let task = app.tasks.iter().find(|t| t.id == 5).unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    assert!(task.worktree.is_none());
    // Should have Cleanup + PersistTask
    assert_eq!(cmds.len(), 2);
    assert!(matches!(&cmds[0], Command::Cleanup { tmux_window: None, .. }));
    assert!(matches!(&cmds[1], Command::PersistTask(_)));
}

#[test]
fn move_forward_to_done_with_live_window_emits_cleanup() {
    let mut task = make_task(5, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/5-task-5".to_string());
    task.tmux_window = Some("task-5".to_string());
    let mut app = App::new(vec![task]);

    let cmds = app.update(Message::MoveTask {
        id: 5,
        direction: MoveDirection::Forward,
    });

    assert_eq!(cmds.len(), 2);
    assert!(matches!(&cmds[0], Command::Cleanup { tmux_window: Some(_), .. }));
    assert!(matches!(&cmds[1], Command::PersistTask(_)));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib tui::tests::move_forward_to_done_emits_cleanup`
Expected: FAIL — no cleanup emitted on forward move

- [ ] **Step 3: Add forward-cleanup logic to MoveTask handler**

In the `MoveTask` handler, the current backward-cleanup block runs before `task.status = new_status`. Add a forward-cleanup block right after the backward-cleanup block (but before the status assignment). Replace the section from `let cleanup = if matches!(direction, MoveDirection::Backward)` through the end of the cleanup `let` binding with:

```rust
let cleanup = if matches!(direction, MoveDirection::Backward) {
    match task.worktree.take() {
        Some(wt) => Some(Command::Cleanup {
            repo_path: task.repo_path.clone(),
            worktree: wt,
            tmux_window: task.tmux_window.take(),
        }),
        None => {
            task.tmux_window.take();
            None
        },
    }
} else if new_status == TaskStatus::Done {
    match task.worktree.take() {
        Some(wt) => Some(Command::Cleanup {
            repo_path: task.repo_path.clone(),
            worktree: wt,
            tmux_window: task.tmux_window.take(),
        }),
        None => {
            task.tmux_window.take();
            None
        },
    }
} else {
    None
};
```

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: All pass including both new forward-cleanup tests and existing backward-cleanup tests

- [ ] **Step 5: Commit**

```bash
git add src/tui/mod.rs
git commit -m "feat: emit cleanup when moving task forward to Done"
```

---

### Task 9: Simplify DispatchTask handler to Ready-only

**Files:**
- Modify: `src/tui/mod.rs`

- [ ] **Step 1: Update existing dispatch tests**

The tests `dispatch_from_running_redispatches` and `dispatch_from_review_redispatches` test old behavior. Replace them with tests for the new behavior:

```rust
#[test]
fn dispatch_from_running_is_noop() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = Some("/repo/.worktrees/4-task-4".to_string());
    task.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task]);
    let cmds = app.update(Message::DispatchTask(4));
    assert!(cmds.is_empty());
}

#[test]
fn dispatch_from_review_is_noop() {
    let mut task = make_task(5, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/5-task-5".to_string());
    task.tmux_window = Some("task-5".to_string());
    let mut app = App::new(vec![task]);
    let cmds = app.update(Message::DispatchTask(5));
    assert!(cmds.is_empty());
}
```

- [ ] **Step 2: Simplify the DispatchTask handler**

Replace the current `Message::DispatchTask` arm:

```rust
Message::DispatchTask(id) => {
    if let Some(task) = self.tasks.iter().find(|t| t.id == id) {
        match task.status {
            TaskStatus::Ready | TaskStatus::Running | TaskStatus::Review => {
                return vec![Command::Dispatch { task: task.clone() }];
            }
            _ => {
                self.status_message = Some(
                    "Move task to Ready before dispatching (press m)".to_string(),
                );
            }
        }
    }
    vec![]
}
```

with:

```rust
Message::DispatchTask(id) => {
    if let Some(task) = self.tasks.iter().find(|t| t.id == id) {
        if task.status == TaskStatus::Ready {
            return vec![Command::Dispatch { task: task.clone() }];
        }
    }
    vec![]
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test`
Expected: All pass

- [ ] **Step 4: Commit**

```bash
git add src/tui/mod.rs
git commit -m "refactor: simplify DispatchTask to Ready-only, input.rs owns branching"
```

---

### Task 10: Context-sensitive `d` key in input handler

**Files:**
- Modify: `src/tui/input.rs`

- [ ] **Step 1: Write tests for d-key branching**

Add tests to `src/tui/mod.rs` (where the other input tests live, using `handle_key`):

```rust
#[test]
fn d_key_on_ready_dispatches() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = App::new(vec![make_task(3, TaskStatus::Ready)]);
    app.selected_column = 1; // Ready column
    let cmds = app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    assert!(matches!(&cmds[0], Command::Dispatch { .. }));
}

#[test]
fn d_key_on_running_with_window_shows_warning() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some("task-4".to_string());
    task.worktree = Some("/repo/.worktrees/4-task-4".to_string());
    let mut app = App::new(vec![task]);
    app.selected_column = 2; // Running column
    let cmds = app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    assert!(cmds.is_empty());
    assert!(app.status_message.as_deref().unwrap().contains("already running"));
}

#[test]
fn d_key_on_running_no_window_resumes() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = Some("/repo/.worktrees/4-task-4".to_string());
    task.tmux_window = None;
    let mut app = App::new(vec![task]);
    app.selected_column = 2; // Running column
    let cmds = app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    assert!(matches!(&cmds[0], Command::Resume { .. }));
}

#[test]
fn d_key_on_backlog_shows_warning() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.selected_column = 0; // Backlog column
    let cmds = app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    assert!(cmds.is_empty());
    assert!(app.status_message.is_some());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib tui::tests::d_key_on_running_with_window_shows_warning`
Expected: FAIL — current `d` key doesn't check window state

- [ ] **Step 3: Replace the `d` key handler in input.rs**

Replace the current `KeyCode::Char('d')` arm in `handle_key_normal`:

```rust
KeyCode::Char('d') => {
    if let Some(task) = self.selected_task() {
        let id = task.id;
        self.update(Message::DispatchTask(id))
    } else {
        vec![]
    }
}
```

with:

```rust
KeyCode::Char('d') => {
    if let Some(task) = self.selected_task() {
        let id = task.id;
        match task.status {
            TaskStatus::Ready => {
                self.update(Message::DispatchTask(id))
            }
            TaskStatus::Running | TaskStatus::Review => {
                if task.tmux_window.is_some() {
                    self.status_message = Some(
                        "Agent already running, press g to jump".to_string(),
                    );
                    vec![]
                } else if task.worktree.is_some() {
                    self.update(Message::ResumeTask(id))
                } else {
                    self.status_message = Some(
                        "No worktree to resume, move to Ready and re-dispatch".to_string(),
                    );
                    vec![]
                }
            }
            _ => {
                self.status_message = Some(
                    "Move task to Ready before dispatching (press m)".to_string(),
                );
                vec![]
            }
        }
    } else {
        vec![]
    }
}
```

Add `use crate::models::TaskStatus;` to the imports at the top of `src/tui/input.rs` if not already present.

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: All pass

- [ ] **Step 5: Commit**

```bash
git add src/tui/input.rs src/tui/mod.rs
git commit -m "feat: context-sensitive d key (dispatch/resume/warning)"
```

---

### Task 11: Add `g` key for jump-to-window

**Files:**
- Modify: `src/tui/input.rs`

- [ ] **Step 1: Write tests for g key**

Add to tests in `src/tui/mod.rs`:

```rust
#[test]
fn g_key_with_live_window_jumps() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task]);
    app.selected_column = 2; // Running column
    let cmds = app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert!(matches!(&cmds[0], Command::JumpToTmux { window } if window == "task-4"));
}

#[test]
fn g_key_without_window_shows_message() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.selected_column = 0;
    let cmds = app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert!(cmds.is_empty());
    assert!(app.status_message.as_deref().unwrap().contains("No active session"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib tui::tests::g_key_with_live_window_jumps`
Expected: FAIL — no `g` key handler

- [ ] **Step 3: Add `g` key handler in input.rs**

Add a new arm in `handle_key_normal`, after the `KeyCode::Char('d')` arm:

```rust
KeyCode::Char('g') => {
    if let Some(task) = self.selected_task() {
        if let Some(window) = &task.tmux_window {
            vec![Command::JumpToTmux { window: window.clone() }]
        } else {
            self.status_message = Some("No active session".to_string());
            vec![]
        }
    } else {
        vec![]
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: All pass

- [ ] **Step 5: Commit**

```bash
git add src/tui/input.rs src/tui/mod.rs
git commit -m "feat: add g key to jump to agent tmux window"
```

---

### Task 12: Handle new commands in main.rs

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Add Command::Resume handler**

Add a new arm in `execute_commands()` in `src/main.rs`, after the `Command::Dispatch` arm:

```rust
Command::Resume { task } => {
    let tx = rt.msg_tx.clone();
    let id = task.id;
    let worktree_path = task.worktree.clone().unwrap_or_default();

    tokio::task::spawn_blocking(move || {
        match dispatch::resume_agent(id, &worktree_path) {
            Ok(result) => {
                let _ = tx.send(Message::Resumed {
                    id,
                    tmux_window: result.tmux_window,
                });
            }
            Err(e) => {
                let _ = tx.send(Message::Error(format!("Resume failed: {e:#}")));
            }
        }
    });
}
```

- [ ] **Step 2: Add Command::JumpToTmux handler**

Add another arm after `Command::Resume`:

```rust
Command::JumpToTmux { window } => {
    if let Err(e) = tmux::select_window(&window) {
        app.error_popup = Some(format!("Jump failed: {e:#}"));
    }
}
```

- [ ] **Step 3: Remove old re-dispatch cleanup in Command::Dispatch**

The `Command::Dispatch` handler currently cleans up previous dispatch before re-dispatching. Since `d` on Running/Review no longer triggers `Command::Dispatch`, this cleanup is dead code. Remove the `old_worktree`/`old_tmux_window` variables and the cleanup block inside the `spawn_blocking` closure. The handler simplifies to:

```rust
Command::Dispatch { task } => {
    let tx = rt.msg_tx.clone();
    let id = task.id;
    let title = task.title.clone();
    let description = task.description.clone();
    let repo_path = task.repo_path.clone();
    let port = rt.port;

    tokio::task::spawn_blocking(move || {
        match dispatch::dispatch_agent(id, &title, &description, &repo_path, port) {
            Ok(result) => {
                let _ = tx.send(Message::Dispatched {
                    id,
                    worktree: result.worktree_path,
                    tmux_window: result.tmux_window,
                });
            }
            Err(e) => {
                let _ = tx.send(Message::Error(format!("Dispatch failed: {e:#}")));
            }
        }
    });
}
```

- [ ] **Step 4: Verify it compiles and tests pass**

Run: `cargo test`
Expected: All pass

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat: handle Resume and JumpToTmux commands in main loop"
```

---

### Task 13: Final integration test

**Files:** None (verification only)

- [ ] **Step 1: Run full test suite**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 2: Run clippy**

Run: `cargo clippy`
Expected: No warnings

- [ ] **Step 3: Build release**

Run: `cargo build`
Expected: Compiles cleanly

- [ ] **Step 4: Commit any clippy fixes if needed**

If clippy flagged anything, fix and commit:
```bash
git add -A
git commit -m "fix: address clippy warnings"
```
