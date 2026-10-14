# Codebase Review — dispatch

**Date:** 2026-10-14
**Reviewed at:** `main` @ `4991b153`
**Scope:** whole repository, with emphasis on the 128 commits since the
2026-10-08 follow-up review (`docs/plans/review-2026-10-08-followup/report.md`)
— chiefly the store-split milestones M2–M7 (`docs/specs/store-split.allium`,
`spacetime/module/src/`, `src/sync/memory_caller/`, the request-row seams in
`src/service/tasks/`), and whether the 14 work packages of that review landed.
**Method:** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test --no-fail-fast` (with `tmux` and `spacetime` on `PATH`), every
gate script in `.githooks/pre-push`, `cargo tarpaulin --engine llvm` locally;
a brace-depth parser over 283 production files (2,832 functions; `bindings/`,
test files, `#[cfg(test)]` modules and `mock_sequence.rs` excluded) and over
the module crate; a cross-module `crate::X` reference count; an
`allium:weed` pass over `store-split.allium` against the module, the memory
double and the host code; `gh run list` on `origin/main`.

---

## 1. Executive summary

- **Every enforcement gate is inert right now.** `main` has not been pushed
  since 2026-10-08 (78 commits), so neither the pre-push hook nor CI has seen
  six days of work. Two hook scripts fail on today's `main`
  (`check-no-test-sleep.sh` on `src/runtime/event_loop.rs:56`,
  `check-rustdoc-links.sh` on `src/models/tasks.rs:394`, both from the last
  two commits; the second is fixed in this review's commit). On `origin`, CI
  has been red for seven consecutive runs since 2026-09-10; the latest
  coverage job died on a tarpaulin timeout inside `tests/spacetime_module.rs`,
  and when the same command finishes locally it scores 85.74% against a floor
  of 86 — the module crate's wasm-only lines are now counted (board code
  alone: 92.15%).
- **The code itself is healthy.** Clippy, fmt and the doc checkers are green;
  5,917 lib tests + 372 integration tests + 172 module tests pass; 12
  production `#[allow]`s, all justified; no uncalled production function.
- **Most of the 10-08 follow-up landed.** `Store` has no `Option` ports and no
  forwarding layer; `src/git.rs` owns `run_git`; services are built once in
  `src/service/services.rs`; agent-tree is one module; `keybindings/table.rs`
  and `tui/types/` are split; raw `i64` ids are gone from the hook/CLI edge.
- **The memory double is the new structural debt.** `src/sync/memory_caller/`
  (4,723 lines) re-implements the SpacetimeDB module (4,885 code lines) and
  reuses only 8 of its helpers; the weed pass found it accepts statuses the
  module refuses, and two 100-line ingest functions exist in both. Native
  copies of rules that moved to the module (`SubStatus::is_valid_for`,
  `Task::is_wrappable`) survive with zero production callers.
- **Test placement and fixtures stalled.** 20 production files still carry
  inline test modules over 200 lines (`src/store/mod.rs` 1,472,
  `src/dispatch/worktree.rs` 1,305); `CreateTaskRequest {` is written out 180
  times in tests although `CreateTaskRequest::fixture` exists; `App` is still
  318 methods over 25 `impl` blocks.

---

## 2. Architecture & patterns

### Pattern

Layered, with an Elm-style core for the TUI (`App::update` → `Command` →
`runtime` executes effects), and — new since M2–M7 — a **thin-client split**:
every rule about shared data lives in the SpacetimeDB module
(`spacetime/module/src/`), the board process keeps only what needs the
operator's machine (git, tmux, claude, gh, files, embeddings), and multi-step
work is driven by request rows (`dispatch_request`, `session_request`) that
the owning host serves and reports on. `src/sync/rules_gate.rs` refuses to
start a board whose `BOARD_RULES_VERSION` disagrees with the module's
`RULES_VERSION` (both 6 today).

### Is it consistently applied?

Yes, and the weed pass agrees: the module, the memory double and the host
enforce the same rules, with one medium gap (H1 below). Production
cross-module references:

| Edge | Count | Verdict |
|---|---:|---|
| `tui→store`, `tui→sync`, `tui→tmux`, `mcp→tui`, `service→tui`, `feed→runtime`, `feed→mcp`, `mcp→cli`, `sync→service`, `dispatch→service`, `cli→runtime` | 0 | Correct — all guarded by `tests/module_layering.rs` |
| `runtime→tui` | 130 | Expected (runtime owns `App`) |
| `tui→models` / `service→models` / `sync→models` | 124 / 99 / 44 | Correct direction |
| `store→sync` / `sync→store` | 18 / 20 | **Cycle**, unguarded (H3) |
| `service→dispatch` | 5 | One-way now (the `dispatch→service` leg is gone) |

The mutation boundary holds: writes go through `TaskServiceApi` /
`EpicServiceApi`, and since M2 the guards themselves (cycle detection,
status validation, wrap-up preconditions) sit in reducers, so a client cannot
skip them.

### Findings

**A1. `store` and `sync` depend on each other (high).** `sync` imports
`crate::store::{bump_decode_fallback, parse_datetime, drop_undecodable}`
(`src/sync/decode.rs`, `src/sync/rows.rs`), the patch/filter types
`EpicPatch`, `LearningPatch`, `LearningFilter`, `UsageQuery`
(`src/sync/encode.rs`, `src/sync/sdk_connector/queries.rs`) and the traits
`HostStore`, `IdentityCredentialStore`, `SubscriptionStore`
(`src/sync/session.rs:38`). `store` imports `crate::sync::{encode,
ReducerOutcome, SharedRows, ReducerCaller, WriterIdentity}`. The RULES table
in `tests/module_layering.rs:21` has no entry for either direction, so the
cycle can grow unnoticed. *Fix:* move the decode helpers into `sync` (or a
leaf `decode` module), move the patch/filter types next to `models`, and add
`("sync", &["store"])` to the layering rules.

**A2. The memory double is a second implementation of the module
(high).** `src/sync/memory_caller/` is 4,723 production lines; the module is
4,885 code lines; the double reuses exactly 8 pure helpers
(`validate_sub_status`, `route_signals`, `can_auto_group`, `repo_name_from_*`,
`extract_github_repo`, `excluded_from_reviews`, `RULES_VERSION`). The rest
is re-derived natively, including the two largest functions in the crate
(`ingest_role_routed`: 128 lines at `src/sync/memory_caller/feed.rs:493` vs
105 at `spacetime/module/src/feed_ingest.rs:231`; `ingest_grouped`: 93 lines
at `feed.rs:344` vs 69 at `feed_ingest.rs:97`). Seven conformance tests
(`tests/memory_caller_conformance.rs`) are the only thing holding them
together. The weed pass found the first real drift: the module refuses an
unknown task/epic status via `validate_status`
(`spacetime/module/src/tasks_epics.rs:21`, `:551`, `:582`), the double's
`write_task`/`write_epic` never check
(`src/sync/memory_caller/tasks_epics.rs:20-37`, `:58-64`), and
`validate_status` is `pub(crate)` (`spacetime/module/src/support.rs:65`) so
the double *cannot* reuse it. Two smaller mismatches: `expire_*` on a
finished row returns `Ok` in the module
(`spacetime/module/src/dispatch_request.rs:701`,
`session_request.rs:1033-1038`) and `Refused` in the double
(`src/sync/memory_caller/dispatch_requests.rs:288-293`,
`session_requests.rs:424-429`). *Fix:* make the module's pure validators and
ingest bodies `pub` and call them from the double; add a conformance test per
reused rule.

**A3. Native copies of rules that moved to the module (high).**
`SubStatus::is_valid_for` (`src/models/tasks.rs:272`) has 31 test references
and no production caller — the module's `validate_sub_status` is the live
rule. `Task::is_wrappable` (`src/models/tasks.rs:549`) is documented as the
answer "every gate reads", and no production gate reads it; the module's
`validate_wrap_up_request` (`spacetime/module/src/session_request.rs:216`)
decides. Both are untested promises that the TUI and the module agree.
*Fix:* delete them, or make them the one implementation the double calls.

**A4. Store trait surface (medium, down from high).** `src/store/mod.rs`
still declares 23 domain traits (`TaskRead`, `TaskCrud`, `DispatchRequestRead`,
`DispatchRequestStore`, `SessionRequestRead`, …) with one implementor,
`Store`, now a direct `rows + caller + identity` struct
(`src/store/mod.rs:1368`). The forwarding layer is gone and the reads are
infallible (tasks #40967, #40985, #40994 landed). What is left is naming
granularity: 23 traits for one type is still a three-line edit per new
method (trait, impl, narrowing alias). Fine to leave; not worth a work
package on its own.

**A5. Stale SQLite prose (low).** 23 production comment lines still mention
SQLite (was 62). Twelve are in `src/spacetime/snapshot.rs` and describe the
snapshot's inherited column order — legitimately historical. Three describe
live behaviour wrongly: `src/models/epics.rs:14` ("the two decoders —
SQLite's and the shared store's"), `src/service/tasks/crud.rs:483` ("left to
the SQLite foreign key"), `src/sync/rows.rs:21` ("the retired SQLite store's
… order").

### Dependency injection

Constructor-injected `Arc<dyn Trait>`, now built once:
`src/service/services.rs:34-37` constructs `TaskService`, `EpicService` and
`LearningService` for both MCP and the TUI. `EpicService::new` takes one
handle. The remaining shape issue is positional: `build_runtime`
(`src/runtime/mod.rs:984`) and `run_loop` (`src/runtime/event_loop.rs:142`)
take 8 parameters each, `render_epic_item` (`src/tui/ui/kanban/cards.rs:709`)
takes 9; `clippy.toml` does not raise `too-many-arguments`, so these pass only
because the default limit is 7 and these functions carry an `#[allow]` or are
below it after `self` — worth a parameter struct each.

### Specification alignment (store-split)

The weed pass over `docs/specs/store-split.allium` found 9 divergences, none
high: the header (`store-split.allium:7-9`, `:497`) still says M7 "is landing"
although `RULES_VERSION = 6` is the post-M7 value; D11's prose
(`store-split.allium:281-283`) says the poll-ownership and settings reducers
"do not check the host yet" while they do (`spacetime/module/src/agent_state.rs:158`,
`config.rs:119`); `HostReportsDispatched` (`store-split.allium:955-997`) does
not state the non-empty-worktree refusal the module enforces
(`spacetime/module/src/dispatch_request.rs:636-640`); W2's check order
(`store-split.allium:348-352`: wrappable → branch → committed → verify) is
not what `handle_wrap_up` runs (`src/mcp/handlers/tasks/wrap_up.rs:194-211`:
commit + a verify run of up to 900 s first, branch check last); and the four
M7 leftovers `create_repo_group_sub_epic` (`spacetime/module/src/feed.rs:415`),
`create_managed_role_epic` (`feed.rs:431`), `recalculate_epic_status`
(`tasks_epics.rs:838`) and `upsert_feed_tasks` (`feed.rs:272`) exist with only
test callers and are not in X5's delete list. Six open questions in the spec
remain undecided in code (close with an open dispatch request; retention of
finished request rows; `update_tasks` done path on an unanswered `close_task`).

---

## 3. Test coverage

### Numbers

- **Local, 2026-10-14: 85.74%** (21,698 / 25,307 lines; `cargo tarpaulin
  --engine llvm --exclude-files 'src/spacetime/bindings/*'`, the CI command,
  live-store tests running and finishing). **That is below the floor of 86**
  (`.github/workflows/ci.yml:254`). The denominator grew from 22,509 lines on
  2026-10-08 to 25,307 because the module crate is now a `test-support` path
  dependency, so tarpaulin instruments `spacetime/module/src/*` natively —
  2,449 lines at 25.93%, seven files at 0% (`feed.rs`, `feed_ingest.rs`,
  `agent_state.rs`, `seed.rs`, `grouping.rs`, `sequence.rs`, `lib.rs`), since
  the reducers run inside the live store as wasm, not in-process. The
  hand-written board code on its own is **92.15%** (21,063 / 22,858). The
  module's own 172 tests run under `cargo test` in `spacetime/module/`, which
  tarpaulin never sees.
- 5,907 `#[test]`/`#[tokio::test]` in `src/`, 372 in `tests/`, 172 in
  `spacetime/module/src/tests.rs`; 66 insta snapshots; one `#[ignore]`.
- Wall time, this machine (busy): lib target 29.9 s for 5,917 tests;
  `tests/spacetime_module.rs` 58 s for 145 tests; the full `--no-fail-fast`
  run about 3.5 minutes.

### Untested critical paths

**T1. The gates have not run (blocker).** See §1. Concretely:

- `scripts/check-no-test-sleep.sh` rejects `src/runtime/event_loop.rs:56`
  (`tokio::time::sleep(wait)` inside `next_loop_event`, commit `6e48cfed`,
  2026-10-13). The script's check 1 bans the call "anywhere under `src/`"
  because "production has no legitimate use" (`scripts/check-no-test-sleep.sh:10`,
  `docs/conventions.md:647`). That premise is now false: the frame-wait is a
  real timer. Swapping to `sleep_until` would only evade the grep.
- `scripts/check-rustdoc-links.sh` rejected `src/models/tasks.rs:394` (an
  intra-doc link to a `Task::display_name` that no commit ever added —
  `cf22f5e0`, 2026-10-14). Fixed in this review.
- On `origin`, run 37892931546 (2026-10-09): Clippy failed on two
  `nonminimal_bool` lints that local `main` has since fixed; Coverage died
  with `cargo_tarpaulin: Failed to run tests: Error: Timed out waiting for
  test response` inside `tests/spacetime_module.rs` — the consequence of
  task #40968 making the live tests run under tarpaulin. `--timeout` is not
  passed (`.github/workflows/ci.yml:226-250`).

**T2. The SDK glue is still only exercised live.**
`src/sync/sdk_connector/callers.rs` (1,149 lines, the largest non-table
production file) and `wiring.rs::wire_tables` (105 lines) run only in
`tests/spacetime_module.rs` and `tests/memory_caller_conformance.rs`. With
T1, those are also the tests the coverage job cannot finish.

**T3. Test-only production functions, ungated.** 26 functions are called
only from tests and carry no `#[cfg(test)]`/`test-support` gate. Beyond A3:
`contexts_overlap` (`src/keybindings.rs:357`), `Notice::agent_list`/
`commit_list` and `RenderState::is_diff_open` (`src/agent_tree/state.rs:55`,
`:63`, `:222`), `TaskService::with_verify_wait`
(`src/service/tasks/wrap_up.rs:225`), `EditorSession::occupied_for_test`
(`src/runtime/editor.rs:87`), `TuiRuntime::bootstrap_with`
(`src/runtime/mod.rs:806`), `MemoryReducerCaller::task_owner`
(`src/sync/memory_caller/mod.rs:276`), `SharedTable::assembled_columns`/
`module_only_columns` (`src/spacetime/snapshot.rs:366`, `:447`),
`ManagedStore::with_timeouts` (`src/spacetime/managed_store.rs:585`), the
`MockProcessRunner` builders in `src/process.rs:634-743`, and the
`ConfirmSpy` counters in `src/setup/confirm.rs:113-121`.

### Behaviour vs implementation

Mostly behavioural, as before: TUI tests assert on returned `Command`s, MCP
tests go through JSON-RPC, service tests use the in-memory store. The
module's 172 tests (`spacetime/module/src/tests.rs`, 2,514 lines in one file)
test reducers through their public entry points. Residual brittleness:
`recorded_calls()` argv assertions grew to 344 (was 329).

### Placement

`docs/testing.md:117-129` names which tests belong inline. The inline modules
have outgrown it: 20 production files carry a `#[cfg(test)] mod` over 200
lines — `src/store/mod.rs` 1,472, `src/dispatch/worktree.rs` 1,305,
`src/dispatch/bump.rs` 1,157, `src/setup/hooks.rs` 751,
`src/sync/memory_caller/mod.rs` 733, `src/runtime/mod.rs` 720,
`src/tui/ui/kanban/mod.rs` 715, `src/dispatch/mock_sequence.rs` 656,
`src/models/epics.rs` 551, `src/service/learnings.rs` 521. The 10-08 WP
"move inline tests" (#40975's predecessor) did not reach these. The two
integration files are also outliers: `tests/spacetime_module.rs` 6,273 lines
and `tests/memory_caller_conformance.rs` 3,723.

### Fixture duplication

`src/store/fixtures.rs:14` provides `CreateTaskRequest::fixture(title,
repo)`, and `TaskBuilder`/`EpicBuilder` exist (22 and 27 uses). Yet
`CreateTaskRequest {` is still written out 180 times, `Task {` 228 times,
`App::new(` 842 times (101 in `src/tui/tests/dispatch.rs` alone, next to five
`make_app*` helpers). Task #40972 (fixture builders) is marked done; the
call sites were not migrated.

---

## 4. Complexity hotspots

### Largest production files (code lines; comments, blanks and inline tests excluded)

| File | Code | Note |
|---|---:|---|
| `src/keybindings/table.rs` | 1,296 | The declarative table; fine |
| `src/sync/sdk_connector/callers.rs` | 941 | One method per reducer; live-only coverage (T2) |
| `src/mcp/handlers/dispatch.rs` | 749 | Tool routing + 80-line `handle_tools_call` |
| `src/runtime/tasks.rs` | 741 | `exec_cleanup` 7 params, `spawn_shared_store_connection` nest 5 |
| `src/sync/memory_caller/feed.rs` | 734 | Duplicate of the module's feed ingest (A2) |
| `src/runtime/mod.rs` | 708 | Down from 1,113 |
| `src/tmux.rs` | 706 | |
| `src/tui/columns.rs` | 680 | Two `impl App` blocks |
| `src/sync/rows.rs` | 668 | |
| `src/spacetime/managed_store.rs` | 653 | |

Total production code: 50,156 lines over 283 files; module crate 4,885 over 19.

### Longest functions

| Function | Lines | Nest |
|---|---:|---:|
| `src/sync/memory_caller/feed.rs:493::ingest_role_routed` | 128 | 4 |
| `src/sync/sdk_connector/wiring.rs:39::wire_tables` | 105 | 3 |
| `src/runtime/commands.rs:138::dispatch_task` | 103 | 4 |
| `src/tui/ui/kanban/columns.rs:95::build_task_col_data` | 96 | 3 |
| `src/sync/memory_caller/feed.rs:344::ingest_grouped` | 93 | 4 |
| `src/tui/ui/kanban/status_bar.rs:148::status_line` | 93 | 2 |
| `src/startup/launch.rs:240::message` | 92 | 2 |
| `src/service/tasks/wrap_up.rs:241::check_wrap_up_ready` | 86 | 5 |
| `src/mcp/handlers/tasks/crud.rs:99::handle_update_tasks` | 84 | 3 |
| `spacetime/module/src/feed_ingest.rs:231::ingest_role_routed` | 105 | 4 |

Eleven functions over 80 lines (was 8). `dispatch_task` at nest 4 was named
last time and is unchanged.

### Deepest nesting (nest ≥ 5, 24 functions)

The MCP handlers are the cluster: `handle_record_learning`
(`src/mcp/handlers/learnings.rs:129`), `handle_rate_learning` (`:259`),
`handle_managed_feed_config` (`src/mcp/handlers/managed_feeds.rs:73`),
`handle_create_epic`/`handle_update_epic` (`src/mcp/handlers/epics.rs:78`,
`:243`), `handle_update_task`/`handle_create_task`
(`src/mcp/handlers/tasks/crud.rs:17`, `:343`). Elsewhere:
`check_wrap_up_ready` (`src/service/tasks/wrap_up.rs:241`), `import_old_store`
(`src/spacetime/import.rs:117`), `read_data_dir_with`
(`src/cli/store_import.rs:114`), the three `exec_*split*` functions in
`src/runtime/split.rs:46-159`, `report` (`src/runtime/pr.rs:124`),
`spawn_shared_store_connection` (`src/runtime/tasks.rs:686`),
`cleanup_removed_feed_tasks` (`src/feed/mod.rs:70`). The 10-08 WP flattened
`handle_get_epic`/`handle_list_epics`; the pattern (parse → look up → match →
match) recurs in every handler that was not touched.

### Parameters / fields

`render_epic_item` 9 (`src/tui/ui/kanban/cards.rs:709`); `run_loop` and
`build_runtime` 8; seven functions at 7 (`reconcile_diff_pane`,
`agent_tree::run_loop`, `launch_agent`, `resolve_start_point`, `add_worktree`,
`exec_cleanup`, `apply_create_managed_role_epic`). In the module:
`create_managed_role_epic` 8 (`spacetime/module/src/feed.rs:431`, an M7
leftover). Structs: `Task` 30 fields (was 29), `UpdateTaskParams` 21,
`Epic` 17.

### God type

`App`: 318 non-test methods over 25 `impl App` blocks in 24 files (was
346 / 23). Task #40975 ("split App by sub-state") is marked done; the
method count fell by 28 and the block count rose by two.

---

## 5. Code smells

**S1. Rules implemented twice (A2, A3).** The dominant smell this round.

**S2. Test-only code in production modules (T3).** 26 ungated functions;
the 10-08 WP #40970 gated some and missed these.

**S3. Fixture literals (§3).** 180 `CreateTaskRequest {`, 228 `Task {`,
842 `App::new(`.

**S4. Stale prose.** The three live-behaviour SQLite comments (A5); the
store-split header and D11 (§2); module doc comments that predate X1
(`spacetime/module/src/dispatch_request.rs:37-38` says M5 "adds wrap-up and
exit-session requests as further values"; `session_request.rs:25`, `:64-74`
omit the `close` kind).

**S5. Primitive ids — mostly resolved.** 99 `i64` id parameters remain
(was 116), now almost all in `src/sync/rows.rs` (13) and
`src/sync/memory_caller/*` (46), where `i64` is the wire type. The
hook/CLI/main edge is on `TaskId`. Leave it.

**S6. Dead code: none.** Every production function has a caller outside
tests except the 26 in T3. `#[allow]` audit: 12 sites, all `unwrap_used`/
`expect_used` behind commented invariants plus one `unused_mut` and one
`needless_range_loop`. Two `TODO`s in `src/`.

---

## 6. Magic wand: top 3 changes

1. **Make the gates fire again.** Push `main`; scope `check-no-test-sleep.sh`
   check 1 to test code and say so in `docs/conventions.md`; give the CI
   coverage job a tarpaulin `--timeout` that survives the live store (or run
   the two live targets outside tarpaulin and merge). Six days of work have
   landed with no hook, no CI and no coverage number — every other finding
   here is smaller than that.
2. **One implementation per rule.** Expose the module's pure validators and
   ingest bodies, call them from `src/sync/memory_caller/`, delete the native
   `is_valid_for`/`is_wrappable`. That turns the conformance suite from "do
   two implementations agree" into "does the double wire the one
   implementation correctly", and removes the only medium spec-code gap the
   weed pass found.
3. **Finish the placement work the last review started.** Move the ten
   largest inline test modules out, migrate the 180 `CreateTaskRequest`
   literals to `fixture`, and split `tests/spacetime_module.rs` by table.
   None of it changes behaviour; all of it is what makes the next 128 commits
   cheaper.

---

## 7. CLAUDE.md improvements

`CLAUDE.md` is 142 lines and mostly accurate. Specific drift and gaps:

- **Timing is stale twice.** "The lib target runs in ~4s once built (measured
  3.5s, 5582 tests)" — measured today 29.9 s for 5,917 tests on a loaded
  machine, and `tests/spacetime_module.rs` alone is 58 s. Knowledge-base entry
  #428 says 15–20 s. State one measured figure with its conditions, or drop
  the number and keep the advice (run it in the foreground, pass a timeout
  on the first run).
- **The thin-TUI split is not in the file.** Nothing tells an agent that data
  rules live in `spacetime/module/src/`, that a rule change must be mirrored
  in `src/sync/memory_caller/` and covered in
  `tests/memory_caller_conformance.rs`, that `RULES_VERSION`/
  `BOARD_RULES_VERSION` must move together, or that `store-split.allium`
  exists. The "Allium Specification" list names eight specs and omits the
  largest one. Add one paragraph under "Store Layering".
- **Say that the gate scripts can fail on committed `main`.** The hook runs at
  push time; the file tells agents to run `cargo fmt` before committing but
  not the five `check-*.sh` scripts. Two of them fail today.
- **Say what the coverage job runs.** It now includes the live-store tests,
  which both lengthens the job and is what timed it out on 2026-10-09.
- **Fix the inline-test guidance pointer.** `docs/testing.md:117-129` is the
  placement table; CLAUDE.md should say that an inline module over ~200
  lines belongs in a `tests/` submodule, since ten files are past that.

---

## 8. Findings by severity

### 🚨 Blockers (Must Fix)

- **B1. Pre-push hook fails on `main`** (`src/runtime/event_loop.rs:56`,
  `scripts/check-no-test-sleep.sh:10`, `scripts/test-check-no-test-sleep.sh`,
  `docs/conventions.md:647`, `docs/testing.md:161`). `tokio::time::sleep` in
  production is now legitimate (the frame-wait timer). Fix: scope check 1 to
  test code (same region rule as check 2) and update the convention text and
  the self-test; do **not** rewrite the timer to dodge the grep.
- **B2. CI coverage job cannot pass: it times out, and when it finishes it
  is under the floor** (`.github/workflows/ci.yml:226-254`,
  `tests/common/spacetime_instance.rs`, `docs/testing.md:180-184`). Run
  37892931546 died with "Timed out waiting for test response" in
  `tests/spacetime_module.rs`. Locally the same command finishes at 85.74%
  against a floor of 86, because the module crate's 2,449 native lines
  (25.93% covered — the reducers run as wasm in the live store) are now in
  the denominator; the board code alone is 92.15%. Fix: add
  `--exclude-files 'spacetime/module/*'` beside the bindings exclusion and
  say why in the floor comment and `docs/testing.md`; pass `--timeout 600`
  (or higher) so the live targets survive; re-record the figure. If the
  timeout persists, run the two live targets outside tarpaulin and merge.
- **B3. `main` unpushed for six days; CI red on `origin` since 2026-09-10.**
  Not a code change — push `main` once B1 is green, then confirm one green
  run. (`.github/workflows/ci.yml`)
- ~~B4. Broken rustdoc link `Task::display_name` (`src/models/tasks.rs:394`).~~
  Fixed in this review's commit.

### ⚠️ High Priority (Strongly Recommend Fixing)

- **H1. Memory double does not validate status** —
  `src/sync/memory_caller/tasks_epics.rs:20-37`, `:58-64` vs
  `spacetime/module/src/tasks_epics.rs:21`, `:551`, `:582`;
  `spacetime/module/src/support.rs:65` (`validate_status` is `pub(crate)`).
  Fix: make `validate_status` `pub`, call it from `write_task`/`write_epic`,
  add a conformance test that patches to an unknown status.
- **H2. Duplicate ingest bodies** — `src/sync/memory_caller/feed.rs:493`
  (`ingest_role_routed`, 128 lines) and `:344` (`ingest_grouped`, 93 lines)
  vs `spacetime/module/src/feed_ingest.rs:231` and `:97`. Fix: lift the
  decision logic in the module into ctx-free functions over plain rows, make
  them `pub`, and have both the reducer and the double call them.
- **H3. `expire_*` outcome differs between double and module** —
  `src/sync/memory_caller/dispatch_requests.rs:288-293`,
  `src/sync/memory_caller/session_requests.rs:424-429` vs
  `spacetime/module/src/dispatch_request.rs:701`,
  `spacetime/module/src/session_request.rs:1033-1038`. Fix: return `Ok` on
  an already-finished row in the double; pin with a conformance test.
- **H4. Native rule copies with no production caller** —
  `src/models/tasks.rs:272` (`SubStatus::is_valid_for`),
  `src/models/tasks.rs:549` (`Task::is_wrappable`). Fix: delete both and
  point their 39 tests at the module's `validate_sub_status` /
  `validate_wrap_up_request` through the double.
- **H5. `store`↔`sync` cycle, unguarded** — `src/sync/decode.rs`,
  `src/sync/rows.rs`, `src/sync/encode.rs`, `src/sync/sdk_connector/queries.rs`,
  `src/sync/session.rs:38`; `tests/module_layering.rs:21`. Fix: move
  `bump_decode_fallback`/`parse_datetime`/`drop_undecodable` into `sync`,
  move `EpicPatch`/`LearningPatch`/`LearningFilter`/`UsageQuery` to `models`
  (or `sync`), move the three session traits to `sync`, then add
  `("sync", &["store"])` to RULES.
- **H6. Inline test modules over 200 lines in 20 production files** —
  `src/store/mod.rs` (1,472), `src/dispatch/worktree.rs` (1,305),
  `src/dispatch/bump.rs` (1,157), `src/setup/hooks.rs` (751),
  `src/sync/memory_caller/mod.rs` (733), `src/runtime/mod.rs` (720),
  `src/tui/ui/kanban/mod.rs` (715), `src/models/epics.rs` (551),
  `src/service/learnings.rs` (521), `src/runtime/editor.rs` (503). Fix: move
  each to `<module>/tests/<topic>.rs` per `docs/testing.md:117`; the
  `check-no-test-sleep.sh` region tracker then also sees them as files.
- **H7. Store-split spec header and D11 are stale** —
  `docs/specs/store-split.allium:7-9`, `:281-283`, `:497`; the
  non-empty-worktree rule missing from `HostReportsDispatched` (`:955-997`,
  enforced at `spacetime/module/src/dispatch_request.rs:636-640`); the M7
  leftovers `spacetime/module/src/feed.rs:272`, `:415`, `:431`,
  `spacetime/module/src/tasks_epics.rs:838` (test-only callers, not in X5).
  Fix: record M7 as landed, rewrite D11, add the worktree requirement, delete
  the four leftovers or list them in X5 with a reason.

### 💡 Medium Priority (Consider for Follow-up)

- **M1. W2 check order** — `src/mcp/handlers/tasks/wrap_up.rs:194-211` runs
  the commit check and a verify run of up to 900 s before the branch check
  that `docs/specs/store-split.allium:348-352` puts second. Fix: check the
  branch before `check_wrap_up_ready`.
- **M2. Nest-5 MCP handlers** — `src/mcp/handlers/learnings.rs:129`, `:259`;
  `src/mcp/handlers/managed_feeds.rs:73`; `src/mcp/handlers/epics.rs:78`,
  `:243`; `src/mcp/handlers/tasks/crud.rs:17`, `:343`. Fix: the parse →
  look-up → early-return shape `handle_get_epic` got in the last review.
- **M3. Other nest-5 / long functions** —
  `src/service/tasks/wrap_up.rs:241` (`check_wrap_up_ready`, 86 lines, nest 5),
  `src/runtime/commands.rs:138` (`dispatch_task`, 103, nest 4),
  `src/sync/sdk_connector/wiring.rs:39` (`wire_tables`, 105),
  `src/spacetime/import.rs:117`, `src/cli/store_import.rs:114`,
  `src/runtime/split.rs:46`, `:77`, `:159`, `src/runtime/pr.rs:124`,
  `src/runtime/tasks.rs:686`, `src/feed/mod.rs:70`.
- **M4. Parameter lists** — `src/tui/ui/kanban/cards.rs:709` (9),
  `src/runtime/mod.rs:984` (8), `src/runtime/event_loop.rs:142` (8),
  `src/runtime/tasks.rs:1012` (7), `src/dispatch/agents.rs:373` (7),
  `src/dispatch/worktree.rs:385`, `:566` (7). Fix: a context struct each.
- **M5. Fixture literals** — 180 `CreateTaskRequest {` (`src/store/tests/`,
  `src/mcp/handlers/tests/tasks/wrap_up.rs`, `src/service/tasks/tests/crud/`),
  101 `App::new(` in `src/tui/tests/dispatch.rs`, 228 `Task {`. Fix: migrate
  to `CreateTaskRequest::fixture` (`src/store/fixtures.rs:14`), `TaskBuilder`,
  and the existing `make_app*` helpers.
- **M6. Test-only functions ungated** — `src/keybindings.rs:357`,
  `src/agent_tree/state.rs:55`, `:63`, `:222`, `src/service/tasks/wrap_up.rs:225`,
  `src/runtime/editor.rs:87`, `src/runtime/mod.rs:806`,
  `src/sync/memory_caller/mod.rs:276`, `src/spacetime/snapshot.rs:366`, `:447`,
  `src/spacetime/managed_store.rs:585`, `src/process.rs:634-743`,
  `src/setup/confirm.rs:113-121`. Fix: `#[cfg(any(test, feature = "test-support"))]`
  or delete.
- **M7. Oversized test files** — `tests/spacetime_module.rs` (6,273 lines,
  145 tests), `tests/memory_caller_conformance.rs` (3,723),
  `spacetime/module/src/tests.rs` (2,514), `src/tui/tests/navigation.rs`
  (2,634), `src/dispatch/prompts/tests.rs` (2,624),
  `src/mcp/handlers/tests/tasks/wrap_up.rs` (2,616). Fix: split by table /
  reducer / feature.
- **M8. Stale comments** — `src/models/epics.rs:14`,
  `src/service/tasks/crud.rs:483`, `src/sync/rows.rs:21` (SQLite as live);
  `spacetime/module/src/dispatch_request.rs:37-38`,
  `spacetime/module/src/session_request.rs:25`, `:64-74` (pre-X1 request
  kinds).
- **M9. CLAUDE.md** — `CLAUDE.md:35` (timing), `CLAUDE.md:75` (store
  layering section: add the thin-TUI rule and `store-split.allium`),
  `CLAUDE.md:49` (gate scripts: run before committing; coverage job runs
  the live store), `CLAUDE.md:144` (spec list omits `store-split`).
- **M10. `App` god type** — `src/tui/mod.rs` and 23 other files, 318 methods.
  Still the largest structural item in the TUI; no cheap step, so it stays a
  larger effort.

---

## 9. Prioritised action items

### Quick wins (≤ half a day each)

| # | Item | Ref |
|---|---|---|
| Q1 | Scope `check-no-test-sleep.sh` check 1 to test code; update conventions, testing.md, self-test | B1 |
| Q2 | Exclude `spacetime/module/*` from tarpaulin; `--timeout`; record today's figure | B2 |
| Q3 | Push `main`; confirm one green CI run | B3 |
| Q4 | `validate_status` reused by the double; `expire_*` outcome aligned; two conformance tests | H1, H3 |
| Q5 | Delete `is_valid_for` / `is_wrappable`; re-point their tests | H4 |
| Q6 | Store-split header, D11, `HostReportsDispatched`, M7 leftovers | H7 |
| Q7 | Branch check before the verify run in `handle_wrap_up` | M1 |
| Q8 | Gate the 26 test-only functions | M6 |
| Q9 | Three SQLite comments; module doc comments | M8 |
| Q10 | CLAUDE.md edits | M9 |

### Larger efforts

| # | Item | Ref |
|---|---|---|
| L1 | Share the module's ingest bodies with the double | H2 |
| L2 | Break the `store`↔`sync` cycle; add the layering rule | H5 |
| L3 | Move the ten largest inline test modules out | H6 |
| L4 | Flatten the nest-5 MCP handlers and the other long functions; parameter structs | M2, M3, M4 |
| L5 | Migrate fixture literals to the builders | M5 |
| L6 | Split the six oversized test files | M7 |
| L7 | Split `App` by sub-state | M10 |
