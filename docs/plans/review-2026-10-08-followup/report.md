# Codebase Review — dispatch

**Date:** 2026-10-08
**Reviewed at:** `main` @ `3e10bfdc`
**Scope:** whole repository, with emphasis on what changed since the
2026-08-28 review (`docs/plans/2026-08-28-codebase-review.md`) — chiefly the
SQLite → SpacetimeDB migration (`src/sync/`, `src/store/`, `src/spacetime/`).
**Method:** `cargo clippy --all-targets -D warnings`, `cargo fmt --check`, the
four gate scripts (all green); a brace-depth parser over 246 production files
(2,671 functions; bindings, tests and `test-support` code excluded); a
cross-module `crate::X` reference count over production code; the CI coverage
figure re-measured today (`.github/workflows/ci.yml`, coverage job comment);
targeted reading of the seams in `docs/invariants.md`.

---

## 1. Executive summary

- **Discipline is still high and gated.** Clippy, fmt and every gate script
  are green. 5,545 in-crate tests plus 282 integration tests. No production
  `unwrap`/`expect` slips through; no `#[allow(dead_code)]` in `src/`.
- **Most hotspots the last review named are fixed.** `handle_mcp`,
  `feed::tick`, `spawn_refresh_epic`, `provision_worktree` and
  `handle_list_tasks` were flattened; `Task` has `Default` + `TaskBuilder`;
  key handling is the declarative `KEY_BINDINGS` table.
- **The migration left a three-layer store stack for one backend.** 14
  domain traits on `Store` → forwarding bodies → 5 port traits, each with one
  production impl over the same `SharedRows`. About 60 production comments
  still describe SQLite as live.
- **Coverage fell from 91.56% to 86.16%** (floor 85). The SDK glue
  (`src/sync/sdk_connector/{callers,connect,wiring}.rs`) only runs against a
  live SpacetimeDB, which always skips under tarpaulin — and the live skip is
  silent in CI, unlike the tmux one.
- **`App` is unchanged as a god type** (346 non-test methods, 23 `impl App`
  blocks). Still the largest structural debt.

---

## 2. Architecture & patterns

### Pattern

Layered, with an Elm-style core for the TUI (`App::update` → `Command` →
`runtime` executes effects). Storage is now SpacetimeDB, reached through
`crate::store::Store`, a router over ports implemented in `src/sync/`.

### Is it consistently applied?

The core boundaries hold. Production cross-module references:

| Edge | Count | Verdict |
|---|---:|---|
| `tui→store`, `tui→sync`, `tui→tmux`, `mcp→tui`, `service→tui` | 0 | Correct |
| `runtime→tui` | 138 | Expected (runtime owns `App`) |
| `tui→models` / `service→models` | 97 / 96 | Correct direction |
| `sync→models` | 51 | Correct |
| `runtime→sync` | 43 | Wiring |
| `mcp→service` | 34 | Correct |

The mutation boundary is intact: `McpState.db` and `TuiRuntime.database` are
`Arc<dyn TaskReadStore>`; writes go through the services, `FeedRunner`, or
`cli/commands.rs` (which wraps `Store` in `TaskService`).

### Findings

**A1. Three-layer store stack for one backend (high).**
`src/store/mod.rs` (1,845 lines) declares 14 domain traits (`TaskRead`,
`TaskCrud`, `EpicRead`, `EpicCrud`, `SettingsStore`, `HostStore`,
`LearningStore`, `PollOwnershipStore`, `UsageStore`, …). Each has one
production impl, `Store`, and that impl only forwards — e.g.
`src/store/queries/tasks.rs`: `self.shared_reader()?.get_task(id)`. The calls
then go through 5 port traits (`SharedReader`, `SharedWriter`,
`SharedLearningReader`, `SharedUsageReader`, `SharedRetiredFeedItemReader`),
each with one production impl in `src/sync/`. Every store method is written
about three times. The port split follows migration phases, not real seams.
*Fix:* collapse domain traits and ports into one set of narrow traits
implemented directly by the `sync` adapters; drop the forwarding layer.

**A2. `Store` ports are `Option` (medium-high).** `Store` holds five
`Option<Arc<dyn …>>`. The only production setter (`with_shared_store`) sets
all five together, yet `runtime::placeholder_database` still builds
`Store::unattached()` in production. A missing port surfaces only at runtime
("no shared store attached"). *Fix:* one non-optional `SharedStorePorts`;
unattached handles only under `cfg(test)`.

**A3. Stale SQLite comments (medium).** No SQLite is linked
(`tests/no_legacy_database.rs`), but 62 production comment lines still
mention it. Examples: `src/sync/mod.rs` ("the board still reads and writes its
local store … a later change"; lists 3 of 16 submodules),
`src/store/queries/tasks.rs` ("local SQLite fallback below"),
`src/store/mod.rs::SharedWriter`, `src/sync/board_reads.rs`,
`src/runtime/mod.rs::bootstrap_with`, `src/mcp/mod.rs`. `src/spacetime/snapshot.rs`
(L452, L688) cites `dump::is_sqlite_backed` and `super::dump_from_sqlite`,
which do not exist. `docs/module-map.md` cites `db::queries::row_to_task`.
*Fix:* one comment sweep; extend `check-doc-symbols.sh` to rustdoc
intra-doc links in `src/` (or enable `rustdoc::broken_intra_doc_links` in CI).

**A4. Lower layers depend upward or sideways (medium).**
`feed→runtime` (`runtime::poll_ownership`), `feed→mcp` (`McpEvent`),
`mcp→cli` (`src/mcp/handlers/hooks.rs` → `cli::pane_key_event`),
`host_file→startup` and `spacetime→startup` (`StartupAbort`),
`sync→service` (`service::Clock`), and a `service↔dispatch` cycle via
`dispatch/prompts.rs` → `service::embeddings`. *Fix:* move
`poll_ownership`, `Clock`, `pane_key_event`, `StartupAbort` to a leaf module;
move `embeddings` out of `service`.

**A5. "Store" names four unrelated things (medium).** `store::Store` (router),
`spacetime::store::SharedStore` (backup/restore trait), `sync::SyncStore`
(session trait), `spacetime/managed_store.rs` (process lifecycle). Wiring
(`StoreParts`, `CliStore`, `open_cli_store`) lives in `src/runtime/mod.rs`, so
`cli/store_import.rs` imports `runtime`. *Fix:* rename
`spacetime::store::SharedStore` → `SnapshotTarget`; move store wiring to its
own module.

**A6. Board reads through two traits to one object (low-medium).**
`TuiRuntime.database` and `TuiRuntime.board_reads` both end at
`SubscriptionBoardReads`; `get_task`, `list_epics` and others exist on both
`BoardReads` and `SharedReader`. Which one a caller uses matters for redraw
tracking, enforced only by convention. *Fix:* make `BoardReads` the TUI's
only card read path.

### Dependency injection

Constructor-injected `Arc<dyn Trait>` throughout — consistent in mechanism,
inconsistent in shape:

- `EpicService::new(db, learnings)` takes two handles that are the same
  object at every call site (`src/mcp/mod.rs:181`, `src/runtime/mod.rs:1418`,
  `src/service/api.rs:629`).
- `TaskService` resolves the host id lazily (`db.ensure_host_identity()`);
  `ReducerWriter` gets `host_id` up front.
- MCP (`McpState::new`) and the TUI (`runtime/mod.rs` ~L1412) each build their
  own `TaskService`/`EpicService`/`LearningService` — two sets per process.

*Fix:* build services once in the composition root and share them; pass
`host_id` explicitly.

### Module placement

The agent-tree feature is spread over 7 modules: top-level
`src/agent_tree.rs`, `src/agent_tree_open_set.rs`,
`src/agent_tree_diff_pane.rs`, and `src/cli/{agent_tree,agent_diff,
agent_tree_agents,agent_tree_commits}.rs`; `cli` also imports
`tui::ui::palette`. *Fix:* one `src/agent_tree/` module, `cli` as thin entry
points.

---

## 3. Test coverage

### Numbers

- **86.16%** line coverage (19,430 / 22,552, llvm engine, bindings excluded),
  re-measured 2026-10-08 in CI. Floor 85. Was 91.56% on 2026-08-28.
- 5,545 `#[test]`/`#[tokio::test]` in `src/`, 282 in `tests/` (~95% in-crate).
- One `#[ignore]` (`src/tui/tests/repo_filter.rs:877`, needs a real TTY).
- Two deadline-bounded polls, both annotated `allow-test-sleep`.

### Untested critical paths

**T1. SDK glue only runs against a live store (high).**
`tests/common/spacetime_instance.rs::spacetime_available_or_skip` returns
false under `cfg!(tarpaulin)`, so `tests/spacetime_module.rs` (~37 tests) and
`tests/memory_caller_conformance.rs` never run in the coverage job. Code
reached only there:

- `src/sync/sdk_connector/callers.rs` (873 lines) — every reducer write,
  including `claim_poll_owner`, `override_poll_owner`, `register_host`.
- `src/sync/sdk_connector/connect.rs::open_connection`.
- `src/sync/sdk_connector/wiring.rs::{wire_rows, wire_tables, wire_subtree_walk}`.

The *logic* is covered in memory (`src/sync/tests/{connection,reconnect,
identity,startup_connect,writes}.rs`, `src/mcp/handlers/tests/poll_ownership.rs`);
the glue is not. This likely accounts for much of the 5-point drop.
*Fix:* run the live files in a separate `cargo llvm-cov` job and merge, or
exclude these three files explicitly and document it beside the floor.

**T2. Live-store skips are silent in CI (high, cheap).**
`tests/tmux_harness/mod.rs::tmux_available_or_skip` hard-fails under CI.
`spacetime_available_or_skip` and
`src/spacetime/tests/managed_store_real.rs:361` only `eprintln!` and pass. If
the "Install spacetime CLI" CI step breaks, the conformance gate goes green
with nothing run. *Fix:* copy the tmux helper's CI hard-fail; keep only the
tarpaulin skip.

**T3. Smaller gaps.** `src/startup/launch.rs`: `enter_session`,
`restart_in_session`, `inside_tmux_session`, `read_launch_context`,
`beside_data_dir` are never named in a test. `src/spacetime/restore.rs`:
`schema_refusal`, `implausible_ceiling`, `burn_id_sequences` likewise.

### Behaviour vs implementation

Mostly behavioural. TUI tests assert on the `Command`s `App::update` returns
(the observable output of an Elm reducer); MCP handler tests go through
JSON-RPC; service tests use a real in-memory `Store`. `src/dispatch/mock_sequence.rs`
is a call-sequence mock but deliberately de-brittled (step indices derived
from one declaration). Residual brittleness: 329 `recorded_calls()` argv
assertions, 12 of them in service-level `epic_in_epic.rs` where task-state
assertions would do.

### Fixture duplication (new hotspot)

The `Task` fixture problem is fixed (`TaskBuilder`). The pain moved:

- `CreateTaskRequest` (`src/store/mod.rs:127`, no `Default`) — **212** full
  struct literals in tests (37 in `src/store/tests/tasks.rs`, 23 in
  `wrap_up.rs`, 23 in `crud/sub_status.rs`).
- `Epic` — no builder, ~11 full 16-field literals.
- `CreateEpicParams` 36×, `CreateTaskParams` 49×, `McpState::new(` hand-built
  25× despite `test_state()`, ~19 `make_app*` variants.
- `src/tui/tests/dispatch.rs`: 100 `App::new(` setups, 131 fully qualified
  `crate::tui::messages::…` paths, repeated `task.tmux_window = Some(…)`
  after `make_task`.

---

## 4. Complexity hotspots

### Largest production files (code lines, comments/blanks excluded)

| File | Code | Suggested split |
|---|---:|---|
| `src/keybindings.rs` | 1,633 | ~1,280 lines are the `KEY_BINDINGS` table → `keybindings/table.rs` |
| `src/runtime/mod.rs` | 1,113 | store bootstrap → `runtime/bootstrap.rs`; `LoopEvent`/`run_loop` → `runtime/event_loop.rs` |
| `src/cli/agent_tree.rs` | 1,073 | git plumbing → `src/git.rs`; keys → `agent_tree/keys.rs` |
| `src/tui/types.rs` | 843 | ~50 types → `types/{state,fold,layout}.rs` |
| `src/store/mod.rs` | 770 | resolves itself with A1 |
| `src/models/tasks.rs` | 737 | agent-event types (L1146–1470) → `models/agent_events.rs` |

### Longest functions

| Function | Lines | Nest |
|---|---:|---:|
| `service/tasks/crud.rs::update_task` | 116 | 1 |
| `runtime/mod.rs::run_tui` | 112 | 1 |
| `runtime/commands.rs::dispatch_task` | 109 | 4 |
| `mcp/handlers/tasks/dispatch.rs::auto_dispatch_next` | 108 | 4 |
| `runtime/mod.rs::bootstrap_inner` | 105 | 1 |
| `runtime/editor.rs::exec_pop_out_editor` | 104 | 2 |
| `sync/sdk_connector/wiring.rs::wire_tables` | 101 | 3 |
| `tui/ui/kanban/cards.rs::classify_card_indicator` | 101 | 3 |

Flat field plumbing at nest 1 is fine. The two worth work are
`dispatch_task` and `auto_dispatch_next` — both on the dispatch seam at nest 4.

### Deepest nesting

- Nest 6: `setup/plugins.rs::install_shipped_feed_scripts`,
  `spacetime/import.rs::prepare`.
- Nest 5: `mcp/handlers/epics.rs::{handle_get_epic, handle_list_epics}` (on
  the never-panic MCP surface), `runtime/mod.rs::apply_loop_event`,
  `feed/ingest/routing.rs::route_and_group_entries`,
  `runtime/tasks.rs::exec_cleanup`, `tui/input/normal.rs::handle_key_delete_item`,
  `tui/update/epics.rs::handle_move_epic_status`,
  `tui/selection.rs::sync_board_selection`.

### Parameters / fields

Four functions at 7 parameters (clippy's limit, no allows):
`feed/ingest/grouped.rs::upsert_sub_epic_and_recalc`,
`runtime/mod.rs::{build_runtime, run_loop}`,
`tui/ui/kanban/cards.rs::render_epic_item`. Structs: `Task` 29 fields,
`App` 26, `UpdateTaskParams` 20, `TuiRuntime` 18, `Epic` 17.

### God type

`App`: 346 non-test methods, 23 `impl App` blocks (was 354 / 22). Biggest:
`tui/mod.rs` 77, `tui/columns.rs` 26, `update/epics.rs` 26,
`update/forms.rs` 24.

---

## 5. Code smells

**S1. Git plumbing has no shared helper (medium).** `run_git` / `git_error`
live in `src/cli/agent_tree.rs:581/594`; `cli/agent_diff.rs` imports them from
there. Meanwhile `dispatch/worktree.rs` (8 sites), `repo_sync.rs` (6) and
`dispatch/finish.rs` (5) hand-build `run_with_timeout("git", &["-C", …])` and
check status manually; `worktree prune` is duplicated at `worktree.rs:627/871`,
`rebase/merge --abort` at `finish.rs:202` / `repo_sync.rs:320`. `tmux.rs`
already has the right shape (`run_checked`, `run_checked_stdout`).
*Fix:* move both helpers to `src/git.rs`, route the 19 sites through them.

**S2. `UpdateEpicParams` has no constructor.** Written field by field (mostly
`None`) at 6 production sites (`runtime/epics.rs:87,168,194,296`,
`runtime/editor.rs:407`, `mcp/handlers/epics.rs:212`). `UpdateTaskParams`
has `for_task(id)`. *Fix:* `UpdateEpicParams::for_epic(id)` + `..`.

**S3. Dead code.** No callers anywhere: `tmux.rs::current_session_name`
(L576), `tui/mod.rs::App::input_buffer` (L456). Test-only but not gated:
`TaskTag::short_label`, `RepoSyncState::{is_diverged, is_measured}`,
`tmux::current_window_name`, `ConnectionStatus::is_healthy`,
`App::{tasks_by_status, status_message}`,
`FieldUpdate::from_optional_string`, `TmuxWindow::into_string`,
`spacetime/store.rs::next_generated_id`, `snapshot.rs::canonical_rows`.
*Fix:* delete the two; gate the rest `#[cfg(test)]` / `test-support` (or
drop the Allium `derived` if `is_measured` is genuinely unused).

**S4. Primitive obsession at the edges.** `task_id: i64` at ~31 sites in
`hooks/wire.rs`, `cli/mod.rs`, `main.rs`, `cli/agent_tree.rs`,
`agent_tree_diff_pane.rs` — `TaskId` serialises identically, so this is free.
`sync/sdk_connector/outcome.rs::is_review` compares `status == "review"`
instead of `TaskStatus`. `repo_path`/`worktree`/`base_branch` remain `String`
(costly to change; low priority).

**S5. Small duplicates.** Identical list-cursor code in
`cli/agent_tree_agents.rs:81–100` and `cli/agent_tree_commits.rs:101–120`;
shared guard-and-find block in `tui/update/lifecycle.rs::{handle_dispatch_task,
handle_trust_and_dispatch}`; `tui/columns.rs:293` `unreachable!` could return
`stats` directly.

**S6. Test reimplementation of the module (note).** `src/sync/memory_caller/`
(~2.9k lines) re-implements the reducers in `spacetime/module/src` (~4.6k).
Deliberate and conformance-checked, but the largest drift risk the migration
added — and see T2: its conformance gate can silently skip.

`#[allow]` audit: 12 production sites, all justified (commented invariants or
test-gated helpers).

---

## 6. Magic wand: top 3 changes

1. **Collapse the store stack (A1 + A2).** One set of narrow traits
   implemented by the `sync` adapters, no `Option` ports, no forwarding
   `Store`. Every new store method stops being a three-file edit; the
   "unattached store" runtime error class disappears. Highest maintainability
   payoff, and the migration just ended, so the shape is fresh.
2. **Make the live store a hard CI requirement and measure it (T1 + T2).**
   Hard-fail the spacetime skip in CI and run the live files under
   `cargo llvm-cov`. That turns the conformance suite from "green unless
   something broke" into a real gate, and recovers the coverage signal on the
   reducer write path — the code most likely to cause data bugs.
3. **Split `App` by sub-state.** Forms, epics, selection and columns each
   own a struct and their methods. Still the biggest drag on productivity in
   `src/tui/`; 346 methods share every field.

---

## 7. CLAUDE.md improvements

`CLAUDE.md` is in good shape (142 lines, the verify-command drift from last
review is fixed). Suggestions:

- **Add the store layering in one line**: "`store::Store` routes to ports in
  `src/sync/`; `spacetime/` is the server lifecycle, bindings and snapshot
  tooling; `sync/memory_caller/` is the in-memory test double, conformance
  checked by `tests/memory_caller_conformance.rs`." The four meanings of
  "store" (A5) cost a reader time today.
- **Say that the coverage job never runs the live SpacetimeDB tests**, so new
  `sdk_connector` code adds uncovered lines by construction.
- **Note that rustdoc comments are not checked** by the doc checkers, so
  stale citations in `src/` comments survive (A3).
- **Timing drift:** "lib target ~10s, cold full run ~80s" vs knowledge-base
  entry #428 ("15–20s"). Pick one figure.

---

## 8. Prioritised action items

### Quick wins (≤ half a day each)

| # | Item | Ref |
|---|---|---|
| Q1 | Hard-fail spacetime test skips under CI | T2 |
| Q2 | Delete dead fns; gate test-only fns | S3 |
| Q3 | Sweep stale SQLite comments; fix broken intra-doc links and `module-map.md` row | A3 |
| Q4 | Move `run_git`/`git_error` to `src/git.rs`, route the 19 sites | S1 |
| Q5 | `UpdateEpicParams::for_epic`; `EpicService::new` takes one handle | S2, DI |
| Q6 | `TaskId` instead of `i64` in hooks/CLI; `is_review` uses `TaskStatus` | S4 |
| Q7 | `CreateTaskRequest` + `Epic` test builders; tidy `tui/tests/dispatch.rs` | Fixtures |
| Q8 | Flatten `handle_get_epic`/`handle_list_epics` and the nest-6 fns | §4 |
| Q9 | CLAUDE.md additions | §7 |

### Larger efforts

| # | Item | Ref |
|---|---|---|
| L1 | Collapse store traits + ports; non-optional ports | A1, A2 |
| L2 | Live-store coverage job (`cargo llvm-cov`) merged with tarpaulin | T1 |
| L3 | Fix upward/sideways deps; break `service↔dispatch` cycle | A4 |
| L4 | Rename/split the "store" modules; move wiring out of `runtime/mod.rs` | A5, §4 |
| L5 | Build services once; share between MCP and TUI | DI |
| L6 | Consolidate agent-tree into one module | Placement |
| L7 | Split `App` by sub-state | §4 |
| L8 | Split `tui/types.rs`, `keybindings.rs` table, `models/tasks.rs` agent events | §4 |
