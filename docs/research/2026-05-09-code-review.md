# Code Review — dispatch (2026-05-09)

Reviewed branch: `677-quick-task` against the worktree state at HEAD `3441e1c`.

## Executive Summary

- **Architecture is clean and well-layered.** Clear separation of `models` → `db` (with narrow per-domain traits) → `service` → `mcp/handlers` + `tui`/`runtime`, mediated by an explicit `Message`/`Command` cycle. The conventions are unusually well documented in `CLAUDE.md`.
- **Test coverage is broad and balanced.** ~41k lines of test code vs ~34k lines of production code (≈1.2:1) across unit, integration, scenario, and snapshot tiers. `MockProcessRunner` and `Database::open_in_memory()` keep tests fast and deterministic.
- **Two real complexity hotspots.** `src/tui/ui/kanban.rs` (2,476 LOC, ten functions over 80 lines, one of 264) and `src/tui/types.rs` (1,294 LOC; `Message` enum has 144 variants, `Command` has 48). Both are kitchen-sink modules that have outgrown their files.
- **Magic-wand priority:** split `Message`/`Command`, decompose `kanban.rs` rendering, and replace the SQLite `Mutex<Connection>` with a connection pool (or move to `tokio-rusqlite`) to remove the synchronous bottleneck behind every async handler.
- **Code smells are minor and isolated.** No `TODO/FIXME/HACK` markers, no `panic!`/`unimplemented!`/`todo!` in production paths, only one `#[allow(dead_code)]`-shaped suppression, and `unwrap_used`/`expect_used` are clippy-warned via `Cargo.toml`.

---

## 1. Architecture & Patterns

**Pattern:** A pragmatic Elm-Architecture-style core (`Message → update → Command`) wrapped around a layered backend.

```
TUI (input → Message → App.update → Command)
    ↕
Runtime (executes Commands; tokio::select! over tmux + MCP + ticks)
    ↕
Service layer (TaskService / EpicService / LearningService) — business rules
    ↕
DB layer (narrow traits: TaskAndEpicStore, EpicCrud, PrStore, …; TaskStore is supertrait)
    ↕
SQLite (rusqlite, single connection guarded by Mutex)
```

**Strengths**
- **Explicit boundaries via traits.** `TaskService` holds `Arc<dyn TaskAndEpicStore>`, `EpicService` holds `Arc<dyn EpicCrud>`, runtimes/MCP hold the wide `Arc<dyn TaskStore>` and let trait-object upcasting (Rust 1.86+) coerce. Conventionally documented in `CLAUDE.md` § "DB trait narrowing".
- **Patch builders for nullable fields.** `TaskPatch`/`EpicPatch` use `Option<Option<T>>` (None = don't touch / Some(None) = NULL / Some(Some) = set), bridged from the service-layer `FieldUpdate` enum. Avoids the sentinel-empty-string anti-pattern.
- **Inline-mutation convention is clearly written down.** Pure UI state mutates inline in `tui/input.rs`; anything with a side effect returns a `Command`. The line is documented and the codebase honours it.
- **Error taxonomy.** Three intentional layers — `anyhow::Result` (IO), `ServiceError::{Validation,NotFound,Internal}` (request shape / business rules), and domain errors (`FinishError`, `PrError`) when callers must branch on the variant. Each layer has a clear use site.
- **Spec-as-source-of-truth.** `docs/specs/*.allium` define the lifecycle/rules; `allium:tend` and `allium:weed` skills enforce alignment.

**Friction points**
- **`App.update()` lives behind a 144-variant `Message` enum.** `dispatcher.rs` is just one big match arm fan-out. Splitting `Message` into per-domain enums (e.g. `TaskMessage`, `EpicMessage`, `InputMessage`, …) carried inside a smaller outer `Message` would localise change and make exhaustiveness checks meaningful.
- **`Command` has 48 variants** with the same shape; same recommendation.
- **Synchronous SQLite under async.** `Database` wraps a `Connection` in `std::sync::Mutex` — every MCP handler is async (Axum) but holds the lock synchronously. Under load you serialise all readers behind one writer. `tokio-rusqlite` or `r2d2_sqlite` with read-only connections would lift that ceiling.

---

## 2. Test Coverage

| Tier | Lines | Examples |
|------|------:|---------|
| Inline unit tests | ~7k | `service/tasks/mod.rs` (60 tests), `db/tests/*` (8 split files) |
| TUI scenario tests | ~14k | `src/tui/tests/{epics,navigation,input_handlers,dispatch,…}.rs` |
| Snapshot tests | (separate dir) | `src/tui/tests/snapshots/` rendered on a 120×40 `TestBackend` |
| End-to-end integration | ~4k | `tests/{lifecycle,epic_lifecycle,review_agent_lifecycle,project_delete,…}.rs` |
| Property tests | inline | `proptest` round-trips on `TaskPatch`, `EpicPatch`, `FieldUpdate` (commit `3441e1c`) |

**What works well**
- **Behaviour over implementation.** Service tests assert on `get_task` / `list_*` outcomes after mutations rather than mocking internal collaborators. Snapshot tests pin user-visible buffers, not widget call sequences.
- **No DB mocking.** Every test uses `Database::open_in_memory()` — a sound choice given how thin the DB layer is. Migrations themselves are tested in `db/tests/migrations.rs` (47 tests, schema-version assertion `fresh_db_has_latest_schema_version`).
- **`MockProcessRunner`** keeps every shell-out (git, tmux) deterministic. `ProcessRunner` is the right seam.
- **Coverage is wired in CI** via `cargo-tarpaulin` — but intentionally not in the pre-push hook (slow). Trend is observable via the CI artifact.

**Gaps**
- **`tui/ui/kanban.rs` (2,476 LOC) only has snapshot coverage.** The render helpers (`render_status_bar` 264L, `render_repo_filter_overlay` 223L, `render_help_overlay` 164L, `render_summary` 131L) compute layout-and-content branching; if a regression doesn't change the rendered buffer it won't surface, and snapshot diffs at 120×40 are coarse.
- **MCP handler tests focus on happy paths and validation.** Concurrent-write semantics — see the documented TOCTOU window in `TaskService::update_task` — aren't asserted anywhere. Acceptable since the policy says "user error", but worth a single regression test that demonstrates last-write-wins.
- **No fuzz/contract tests for the JSON-RPC surface.** `deserialize_flexible_i64` is critical (Claude Code sometimes sends ints as strings) and only has the implicit coverage from handler tests.

---

## 3. Complexity Hotspots

Top files by LOC (production only, tests excluded):

| File | LOC | Notes |
|------|----:|-------|
| `src/tui/ui/kanban.rs` | 2,476 | rendering god-module; 10 functions >80 lines |
| `src/service/tasks/mod.rs` | 1,979 | (~970 of those are tests) |
| `src/models/mod.rs` | 1,596 | re-exports + shared model tests; lots of inline tests |
| `src/tui/types.rs` | 1,294 | 12 enums incl. `Message` (144 variants), `Command` (48), `InputMode` (31), `ViewMode` (4) |
| `src/mcp/handlers/tasks.rs` | 1,202 | 12 handler fns; 4 fns >75 lines |
| `src/tui/mod.rs` | 1,105 | `App` struct + lifecycle |
| `src/db/migrations.rs` | 1,086 | 39 versioned migrations (intentional — never squashed) |
| `src/models/review.rs` | 1,054 | review/security agent state machine |
| `src/tmux.rs` | 998 | tmux IPC abstraction |

**Long functions (>80 lines, production code):**

```
src/tui/ui/kanban.rs:1798  render_status_bar           264 lines
src/tui/ui/kanban.rs:1573  render_repo_filter_overlay  223 lines
src/tui/ui/kanban.rs:1407  render_help_overlay         164 lines
src/tui/ui/kanban.rs:194   render_summary              131 lines
src/tui/ui/kanban.rs:1070  render_task_detail_overlay  124 lines
src/mcp/handlers/tasks.rs:452  handle_list_tasks       123 lines
src/mcp/handlers/tasks.rs:719  handle_exit_session     118 lines
src/tui/ui/kanban.rs:715   render_task_column          114 lines
src/tui/ui/kanban.rs:615   render_columns               94 lines
src/mcp/handlers/tasks.rs:295  handle_update_task       94 lines
src/tui/ui/kanban.rs:2066  action_hints                 91 lines
src/tui/ui/kanban.rs:1317  render_tips_overlay          88 lines
src/tui/ui/kanban.rs:532   build_task_list_item         81 lines
```

**Concentration:** seven of the top thirteen are render helpers in `kanban.rs`. The file mixes top-level `render()` orchestration, every overlay (help, tips, repo filter, task detail), per-column rendering, status bar, and action hints. This is the single highest-leverage refactor target.

**Cyclomatic complexity proxy:** `match` keyword count is 424 across non-test code, `if let`/`while let` 394 — heavy pattern-matching, which is idiomatic Rust but concentrates branching in the "god enums" (`Message`, `Command`, `InputMode`).

---

## 4. Code Smells

**Mostly absent or already mitigated**

- ✅ **No `TODO/FIXME/XXX/HACK` markers** in production code. (`grep` returned zero.)
- ✅ **No `panic!`/`unimplemented!`/`todo!`** in production paths (15 hits — all in `#[cfg(test)]` blocks).
- ✅ **`unwrap_used` and `expect_used` are clippy-warned** at the crate level (`Cargo.toml` `[lints.clippy]`). Pre-push runs `cargo clippy --all-targets -- -D warnings`. The handful of `#[allow(clippy::unwrap_used)]` exceptions all have justification comments (test helpers, `learnings.rs:270` render path, `runtime/mod.rs:595` documented DB invariant).
- ✅ **`let _ =` is documented and bounded.** 92 occurrences in production; convention is documented in CLAUDE.md ("Intentional `let _ =`").
- ✅ **No `#[allow(dead_code)]`** in production code (CLAUDE.md explicitly forbids it).
- ✅ **DRY model conversions.** `TaskStatus`, `LearningKind`, `LearningScope`, `LearningStatus`, `TipsShowMode` all use `FromStr` consistently — no scattered `match s { "x" => ... }` blocks.

**Real smells**

1. **`Message` enum is a god-type (144 variants).** Adds friction to every change in the TUI module: every new feature appends to a 1,000-line enum, every `update()` arm grows the dispatcher, every test that constructs a `Message` reaches across domains. The `// Input routing messages` comment block already shows the implicit grouping — formalise it.

2. **`tui/ui/kanban.rs` is a kitchen-sink module.** It owns column rendering, overlays for help/tips/repo-filter/task-detail, status bar, and action hints. Every new overlay inflates the file; nothing forces them to share an abstraction. Each overlay is plausibly its own `tui/ui/overlays/<name>.rs` exposing a `render(frame, app, area)` and a private `layout()` helper.

3. **Long parameter lists slipping in.** Two `#[allow(clippy::too_many_arguments)]` (`db/queries/learnings.rs:52`, `db/mod.rs:422`). Both look like genuine "wrap in a `Params` struct" candidates. The patch-builder convention exists; extending it to query inputs would close the gap.

4. **Inline tests inflate file size.** `service/tasks/mod.rs` is 1,979 LOC but ~970 of that is the inline `mod tests`. The CLAUDE.md test-placement table says "inline in `src/service/tasks/{mod,crud,params,validators}.rs`", so this is conscious — but at this size, splitting tests by concern (status changes / claim / list / epic linkage) into sibling files (e.g. `src/service/tasks/tests/`) would make them easier to navigate. (Same applies to `tui/types.rs`'s 1,214–1,294-line tests block.)

5. **`McpEvent::Refresh` triggers a full DB reload.** `exec_refresh_from_db` reloads all tasks + epics + usage on every mutation. Fine today but quadratic when a feed epic syncs 200 tasks and emits 200 `Refresh` events. Targeted invalidation (`McpEvent::TaskChanged(TaskId)` / `EpicChanged(EpicId)`) would match the existing `MessageSent(task_id)` pattern.

6. **TOCTOU on sub-status validation** is documented and accepted (CLAUDE.md). Worth a single test demonstrating the accepted behaviour so it doesn't accidentally regress into a transaction-wrapped fix nobody asked for.

---

## 5. Magic Wand: Top 3 Changes

### 1. Split `Message` and `Command` along domain lines
**Where:** `src/tui/types.rs:179` (`Message`, 144 variants) and `:403` (`Command`, 48). `src/tui/dispatcher.rs` and `src/runtime/commands.rs`.

**What:** Replace each god-enum with a small outer enum carrying domain-specific inner enums:
```rust
pub enum Message {
    Task(TaskMessage),
    Epic(EpicMessage),
    Learning(LearningMessage),
    Input(InputMessage),
    System(SystemMessage),
    // ...
}
```
Each per-domain dispatcher (`tui/update/tasks.rs`, `update/epics.rs`, …) already exists — they would become the natural homes.

**Impact:**
- *Productivity:* every TUI change touches a smaller surface.
- *Maintainability:* exhaustiveness checks become useful again. Today, `match self.update(msg)` over 144 variants discourages developers from caring about new variants.
- *Bugs:* eliminates whole classes of "forgot to add a branch in `dispatcher.rs`" mistakes.

### 2. Decompose `tui/ui/kanban.rs`
**Where:** `src/tui/ui/kanban.rs` (2,476 LOC, ten >80-line functions).

**What:** Move every overlay into its own file under `src/tui/ui/overlays/`:
```
overlays/help.rs               <- render_help_overlay (164L)
overlays/repo_filter.rs        <- render_repo_filter_overlay (223L)
overlays/tips.rs               <- render_tips_overlay (88L)
overlays/task_detail.rs        <- render_task_detail_overlay (124L)
status_bar.rs                  <- render_status_bar (264L)
columns.rs                     <- render_columns / render_task_column / build_task_list_item
```
`kanban.rs` keeps `render()`, `column_color`, and `cursor_bg_color` — its actual responsibility.

**Impact:**
- *Productivity:* the overlay you're editing is the file open in your editor.
- *Maintainability:* per-overlay tests become viable (each one becomes ≤300 LOC, testable as a unit).
- *Bugs:* surfaces shared structure that's currently copy-pasted (border colour, dim label patterns); collapsing duplication is a follow-up that becomes obvious only after the split.

### 3. Lift the synchronous-SQLite bottleneck
**Where:** `src/db/mod.rs` (`Database` wraps `Connection` in `Mutex`) and every async MCP handler in `src/mcp/handlers/`.

**What:** Replace `Mutex<Connection>` with `tokio-rusqlite` (executes on a dedicated thread, async API) or `r2d2_sqlite` (sync pool with `tokio::task::spawn_blocking`). Reads can scale (SQLite WAL allows multiple readers); writes still serialise.

**Impact:**
- *Productivity:* removes the awkward `self.conn()?` pattern that exists today only because the lock is sync. Handlers stop blocking the Tokio worker thread.
- *Maintainability:* aligns the DB layer with the rest of the async stack.
- *Bugs:* eliminates the lock-poisoning failure mode entirely (currently mitigated by `self.conn()?` returning `Result` instead of `unwrap`-panicking — see CLAUDE.md "`conn()`").

---

## 6. CLAUDE.md Improvements

`CLAUDE.md` is exceptional — already covers architecture, conventions, error taxonomy, the inline-mutation rule, the `FieldUpdate`/`TaskPatch` patterns, MCP error codes, the projects model, and how-to guides for the most common changes. Suggestions are incremental:

**Missing context that would speed up onboarding**
- **A "where state lives" diagram.** The flow `MCP handler → notify → McpEvent → runtime → exec_refresh → Message::RefreshTasks → App.update` is documented in prose, but a one-glance ASCII diagram next to it would beat re-reading the chain. (You already have one in §"MCP Notification Flow" — promote it to a full sequence with dotted lines for the `MessageSent` branch.)
- **Performance characteristics & known limits.** The synchronous SQLite bottleneck and the full-reload-on-`Refresh` trade-off aren't mentioned. New contributors will discover them only by reading `runtime/tasks.rs::exec_refresh_from_db`.
- **Allium spec → code map.** The CLAUDE.md says specs are the source of truth but doesn't say which spec corresponds to which area of code. A small table (`tasks.allium → src/service/tasks/, src/models/tasks.rs, src/mcp/handlers/tasks.rs`) closes the loop.

**Conventions that are followed but not documented**
- **Naming pattern for runtime exec helpers** (`exec_refresh_from_db`, `exec_refresh_projects_from_db`, `exec_quick_dispatch`) — the projects section mentions it in passing for one helper but the rule is consistent across the file.
- **`#[serde(deserialize_with = "deserialize_flexible_i64")]`** is documented for one tool but not stated as the project-wide rule for any integer arg coming from Claude Code.
- **TUI `pub(in crate::tui)` visibility convention** is documented (good!) but the parallel rule for `pub(in crate::service)`/`pub(in crate::db)` could be stated explicitly — the codebase follows it.

**Implicit assumptions worth surfacing**
- **What "fast tests" means in this project.** Snapshot tests at 120×40 are pinned in the doc, but the implicit `cargo test` budget (a few seconds for unit, more for integration) isn't stated. New contributors won't know whether a 30-second test is acceptable.
- **MCP port allocation.** The doc says 3142 by default with `DISPATCH_PORT`. It doesn't say what happens when two `dispatch tui` instances run at once (presumably the second `bind` fails). One sentence prevents debugging time.

**Workflows worth a short section**
- *"Editing a feed-epic command from the TUI"* and *"Adding a new feed source"* — `scripts/fetch-*.sh` and the `feed_command`/`feed_interval_secs` knobs are documented, but the workflow that ties them to the TUI seeded epic isn't.

---

## Prioritised Action Items

### Quick wins (≤1 day each)
- Add `proptest` round-trip for `deserialize_flexible_i64` covering the int-as-string case end-to-end (closes the "no fuzz on JSON-RPC surface" gap).
- Add a regression test demonstrating the accepted TOCTOU window on `update_task` so it doesn't drift.
- Promote the MCP notification flow to an ASCII sequence diagram in CLAUDE.md.
- Replace the two `#[allow(clippy::too_many_arguments)]` with `Params` structs (`db/queries/learnings.rs:52`, `db/mod.rs:422`).
- Extract `render_status_bar`, `render_help_overlay`, `render_repo_filter_overlay`, `render_task_detail_overlay`, `render_tips_overlay` into `src/tui/ui/overlays/*.rs`. Each move is mechanical; together they shave ~860 LOC off `kanban.rs`.

### Medium-effort (1–3 days)
- Split `tui/types.rs` along domain lines: keep `Message`/`Command` outer enums minimal, push variants into `messages/{tasks,epics,learnings,input,system}.rs`. Migrate `dispatcher.rs` and `runtime/commands.rs` to per-domain match arms in lockstep (single PR per domain).
- Move `service/tasks/mod.rs`'s inline tests into a sibling `tests/` folder split by concern (claim, lifecycle, epic linkage, list/filter). The current file is 50 % tests by line count.

### Larger efforts (>3 days)
- Migrate `Database` to `tokio-rusqlite` or an `r2d2_sqlite` pool (read-heavy paths benefit immediately under WAL). Audit every `self.conn()?` call site; the sync-to-async transition will surface a few handler signatures that need `.await`.
- Replace bulk `McpEvent::Refresh` with targeted invalidations (`TaskChanged(TaskId)` / `EpicChanged(EpicId)` / `ProjectChanged`). The TUI then merges deltas instead of reloading the world.

---

*Reviewed via repo introspection only — `cargo build` succeeded; `cargo test` and `cargo tarpaulin` not executed for this report.*
