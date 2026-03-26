# TODOS

## Phase 2

### Re-dispatch / kill-agent capability
**What:** Add `k` key to kill the agent's tmux session from the TUI, and allow `d` on InProgress tasks to re-dispatch (kill existing session, create new one).
**Why:** Agents get stuck. You need to restart them without leaving the TUI. This is a day-2 requirement for a tool used daily.
**Pros:** Complete orchestration workflow — create, monitor, kill, restart.
**Cons:** Adds ~30 lines to app.rs + dispatch.rs. Need to handle tmux kill-session cleanup.
**Depends on:** Phase 1 dispatch working correctly.

### Handle git branch conflicts in dispatch
**What:** Before dispatch, check if the branch name already exists (`git branch --list {slug}`) and either auto-increment the suffix or prompt the user. Also handle cases where the worktree exists but the branch was deleted, or vice versa.
**Why:** Prevents dispatch failures from pre-existing branches (from prior dispatches, manual work, or partial cleanup). The slug collision handler only checks the task list, not actual git state.
**Pros:** Robust dispatch with zero user confusion.
**Cons:** ~10 lines in dispatch.rs + a subprocess call to `git branch --list`.

### Investigate JSON vs SQLite for persistence
**What:** Evaluate whether a JSON file (`~/.local/share/orchestrator-tui/tasks.json`) would be simpler than SQLite for Phase 1's 5-20 task scale. If JSON is chosen, remove rusqlite + tokio-rusqlite dependencies.
**Why:** SQLite adds bundled C compilation (~30s first build), async wrapper complexity, and WAL mode configuration. For single-digit concurrent tasks with no concurrent access, JSON + serde may be sufficient.
**Pros (JSON):** Fewer deps, faster builds, simpler code, human-readable state file.
**Cons (JSON):** No query capability, no indexing, file-level locking, harder to migrate to SQLite later.
**Context:** SQLite was chosen for learning value (rusqlite, async patterns) and future-proofing (MCP server in Phase 2 would need concurrent access). If learning is the priority, keep SQLite. If shipping speed is the priority, use JSON.
