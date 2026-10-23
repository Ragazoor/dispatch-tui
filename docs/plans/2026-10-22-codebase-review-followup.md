# Codebase review — 2026-10-22 follow-up

Task #98442. Second pass on the same day as `docs/plans/2026-10-22-codebase-review.md` (task #98311). This pass checks every morning finding against today's tree, reviews the 25 commits since the morning report (03aa1860..90a392f6), and runs a fresh defect hunt in areas the last three reviews barely touched: `src/cli/`, `src/setup/`, `src/keybindings/`, `src/agent_tree/`, the learnings and work-package tools, and feed ingest. Two read-only reviewers plus inline checks. Paths are relative to the worktree root.

**What was run.** `cargo test --no-fail-fast` and `cargo clippy --all-targets -- -D warnings` on this host (rustc 1.99.0, now equal to the pin). Results are in "Test and lint run" below.

**Confidence key.** *Verified*: re-read in the code. *Confirmed*: traced through more than one file. *Plausible*: the path exists but depends on environment or timing.

## 1. Executive summary

- **The morning's work packages landed and hold up.** Of the morning's 17 actionable items, 13 are Fixed in code with tests, and 4 stay Open: the `PaneId` newtype (S2), the relink-ordering move (C2), the CLAUDE.md unwrap sentence and the trust model.
- **No defects in the 25 new commits.** The split-pane settle fix, toolchain guard, pin bump to 1.99.0, flat lens in the package view, CVE runbook branch and `BootstrapInputs` all read clean.
- **One new Medium: `prune-repo-paths` judges a shared table by one machine's disk.** It deletes other hosts' repo paths and their verify commands. It is another face of the open trust-model decision.
- **Four Lows in the fresh areas:** repeated `rate_learning` inflates scores, work-package create/update is not one transaction, a `store import` port retry that can never fire, and a third atomic-write copy with a fixed temp name.
- **The trust model is now the single biggest open decision.** It blocks `subscribe_to_epic`, `seed_*`, the MCP `identity` question and now `prune-repo-paths`.

## 2. Findings

### 2.1 Architecture and patterns

Layering is now test-enforced for the board too: `tests/module_layering.rs` (`RULES`) has a `tui` row that bans `runtime`, `tmux`, `store`, `sync`, `dispatch` and more. The only `crate::dispatch` left in non-test `src/tui` is a doc link (`src/tui/messages/split.rs:30`). The composition root takes one `BootstrapInputs` (`src/runtime/mod.rs:237`). The module and its in-memory twin match for learnings and work packages (create, patch, cascade, rescope, link/unlink idempotence), checked by reading both sides.

**A1. Medium — `prune-repo-paths` deletes every host's saved repo paths, and their verify commands, based on one machine's disk.** `src/cli/commands.rs:174` (`prune_repo_paths`) deletes each row whose path does not exist locally; `spacetime/module/src/tables.rs:269-279` (`RepoPath`) is a public table with no host column, and `verify_command` lives in the same row. *Confirmed.* The spec rule `docs/specs/cli.allium` `PruneRepoPathsViaCli` says "Existence is judged on THIS machine's filesystem" but the delete is store-wide. Scenario: operator A runs `dispatch prune-repo-paths`; host B saved `/home/b/code/x` with a verify command; the path does not exist on A, so the row and B's verify command are gone for everyone. The same happens on one host with an unmounted drive. **Decision item** (tied to the trust model, §4): (a) remove only rows this host saved (needs a host column, a module change and a `RULES_VERSION` bump); (b) refuse to remove a row with a non-empty `verify_command` without `--force`; (c) print the list and ask before deleting.

**A2. Low — work-package create and update are not one transaction.** `src/service/work_packages.rs` (`create_work_package`: one reducer call for the package, then one per link; `update_work_package`: patch, unlinks, links as separate calls). *Verified.* Scenario: a linked task is deleted between the item check and its `link_work_package_item` call; the package exists with partial links, the MCP call returns an error, the agent retries and a duplicate package appears. Fix: a module reducer `create_work_package_with_links` (and an update twin) that does it all in one transaction; mirror it in `src/sync/memory_caller/work_packages.rs` and cover the pair in `tests/memory_caller_conformance.rs`. The service's own title/package/item checks then go, since the module repeats them (the "data rules in service" pattern CLAUDE.md says to stop).

### 2.2 Test coverage

The morning's T1 gap is closed: `src/runtime/tests/split_mode.rs:962-987` covers rename, rejoin and kill failures after `swap-pane`, each with the reverse swap working and failing, and `src/tui/tests/split_pane.rs:765` covers the board side. The `RULES_VERSION` floor test (`spacetime/module/src/tests/agents.rs`, `the_rules_version_is_at_least_nine_since_agents_a5`) is weak on its own, but the real pin is the equality test in `src/sync/rules_gate.rs:71`; not a gap.

**T1. Low — no test pins `prune-repo-paths` against a row with a verify command.** `src/cli/commands.rs:174`. *Verified.* Whatever A1's decision is, a test that a pruned row's `verify_command` is (or is not) kept should land with it.

### 2.3 Complexity hotspots

Unchanged from the morning list except that `bootstrap_inner`/`build_runtime` now take `BootstrapInputs` and `list_epic_subtree` became `SubtreeWalk` (`src/tui/update/board_filter.rs:320`). New in this pass:

**C1. Low — `ingest_role_routed` is a 120-line, five-phase function.** `spacetime/module/src/feed_ingest.rs:362-481` (ensure roles, index existing, route and move, upsert, delete stale and recalculate). *Verified.* The comments insist on a phase order that the body makes hard to see. Scenario: a change to the routing phase reorders a step the comments say must come first, and the review misses it inside 120 lines. Fix: extract `index_existing` and `route_items` as named helpers; keep the order in the caller.

### 2.4 Code smells

**S1. Low — a third atomic-write copy, with a fixed temp name.** `src/agent_tree/open_set.rs:75` (`write_atomic` writes `path.with_extension("json.tmp")` then renames). *Verified.* `src/atomic_write.rs` (5 callers) and `src/cli/statusline.rs` (`write_atomically`, unique name on purpose) are the other two. Scenario: two agent-tree panes save the open set at once; both write the same `.json.tmp`, and one rename moves the other's half-written file into place. Fix: call `crate::atomic_write::write_bytes` (or its equivalent) from `open_set.rs` and delete the local helper.

**S2. Low — the `store import` port retry can never fire.** `src/cli/store_import.rs:107` (`PORT_ATTEMPTS`) and `:174` retry on `ErrorKind::AddrInUse` from `spawner.spawn`, but `SpacetimeStartSpawner::spawn` (`src/spacetime/managed_store.rs:506`) only runs `Command::spawn`; the child binds the port later, so spawn never returns `AddrInUse`. *Verified.* Scenario: another process takes the picked port; the import waits the full start timeout and fails with "did not start within 30 seconds", never retrying. Fix: either detect the bind failure after spawn (child exited, log says address in use) inside `wait_until_answering` and retry there, or delete `PORT_ATTEMPTS`, the dead match arm and the doc sentence that promises the retry.

**S3. Low — repeated `rate_learning` calls inflate a learning's score.** `src/service/learnings.rs:300` (`apply_verdicts`) only checks that a retrieval exists; `spacetime/module/src/learning.rs` (`apply_learning_verdicts`) adds ±1 on every call. *Verified.* The spec `docs/specs/learnings.allium` `RateLearningViaMcp` is silent on repeats. Scenario: the task prompt says to rate a learning "when you act on it" and the wrap-up `/learnings` step says to rate again; one use gives +2, and the `↑N` ranking and the stale-archive rule (`upvote_count <= 0`) both read that count. **Decision item:** (a) one verdict per (task, learning), the latest counting — a module rule, spec first, `RULES_VERSION` bump; (b) document that repeats are intended. The retrieval check itself is also a data rule in `src/service/`; option (a) would move it into the reducer.

**S4. Low — `update_work_package` says "updated" when an unlink touched nothing.** `src/mcp/handlers/work_packages.rs` (`handle_update_work_package`). *Plausible*, cosmetic. Scenario: an agent unlinks a task that was never linked and is told the update succeeded with no hint nothing changed. Fix: name in the reply which requested unlinks were not linked. Fold into A2.

### 2.5 The 25 new commits

Reviewed and clean: split-pane settle (64513924, bd06146a, 9fe60965; `SwapFailure::{Unmoved, Exchanged}` at `src/dispatch/split_panes.rs:184-193`, spec rule `SplitPaneSwapSettles`), toolchain guard (84171036; `.githooks/pre-push` runs `scripts/check-toolchain.sh` and its self-test first, CI runs both), pin bump to 1.99.0 and async-trait 0.1.92 (346a2c37, e4d388f3; no stale `1.96` outside the guard's history note and its test case), flat lens in a package view (b6bccfda, 90a05657, 19f093bc; `src/tui/columns.rs:133-157`, hidden epics still filtered by `src/tui/search.rs:154`), CVE runbook keyed on the `fix-pr` label (533f6302, d8d91bda, 5c1eee15; label and `Fix PR:` line are written together at `scripts/fetch-cve.sh:166/174`), CVE match reason (6266e21c), one git test helper (f92e3a30, `src/test_git.rs`), decompose-review keeps Lows (e6049095, tested in `src/setup/plugins/tests/skill_workflow.rs`), learnings metrics rescope (7186e8e4; UTC text compare is sound).

Considered and dropped: a double failure in one swap (later step fails *and* the reverse `swap-pane` fails) leaves B's window holding A's pane; the `Exchanged` error names that window, and two tmux failures in a row are needed. A hand-added `fix-pr` label with no `Fix PR:` line would pick the wrong runbook branch; it needs an operator to add the label by hand.

### 2.6 CLAUDE.md improvements

1. **Unwraps (open since the morning).** CLAUDE.md:115 still reads as if production unwraps exist. Say "the tree has zero outside `cfg(test)`, and clippy's `unwrap_used`/`expect_used` under `-D warnings` keeps it there", so an agent does not audit the ~314 test-module calls.
2. **Trust model.** Still zero hits for "trust model" or "single-operator" in `docs/` or `CLAUDE.md`. Once decided, name it in "Store Layering" — it now governs `prune-repo-paths` (A1) as well as `subscribe_to_epic` and `seed_*`.
3. **Shared vs per-host data.** Nothing tells an agent which store tables are per-host and which are global. `RepoPath` reads as host-local config but is global (A1). One line in `docs/module-map.md` or `docs/specs/sync.allium` listing the per-host tables would have flagged A1 at design time.

## 3. Status of the morning (2026-10-22) findings

| # | Finding | Status | Evidence |
|---|---|---|---|
| A1 | `tui` missing from the layering tripwire | **Fixed** | `tests/module_layering.rs` `RULES` `tui` row; 2b422718, 90a392f6 |
| A2 | `serverInfo.version` hard-coded | **Fixed** | `src/mcp/handlers/dispatch.rs:740` `env!("CARGO_PKG_VERSION")` |
| A3 | module-map omits `restore_pinned_pane` | **Fixed** | `docs/module-map.md:101` |
| T1 | No post-swap failure test | **Fixed** | `src/runtime/tests/split_mode.rs:962-987`, `src/tui/tests/split_pane.rs:765` |
| C1 | Three 7-parameter bootstrap functions | **Fixed** | `src/runtime/mod.rs:237` `BootstrapInputs`; eca3d178 |
| C2 | `update_task` relink ordering in the service | **Open** | `src/service/tasks/crud.rs:223-237` unchanged |
| C3 | `list_epic_subtree` ten parameters | **Fixed** | `src/tui/update/board_filter.rs:320` `SubtreeWalk` |
| S2 | Pane/repo paths as `&str` | **Open** | No `PaneId` in `src/`; larger effort, still worth it |
| S3 | Third sanitised-git helper | **Fixed** | `src/test_git.rs`; `docs/testing.md:144` |
| S4 | Misleading `has_window` context | **Fixed** | `src/dispatch/split_panes.rs:238` |
| S5 | CVE name-match links | **Fixed (narrowed)** | `scripts/fetch-cve.sh:146-153` states the match reason; tested in `scripts/test-fetch-cve.sh` |
| N1 | Post-swap failure leaves board and tmux disagreeing | **Fixed** | 64513924, bd06146a; spec `SplitPaneSwapSettles` |
| N3 | Pre-push gate dead on distro rust | **Fixed twice** | Guard 84171036 and pin bump 346a2c37 (host rustc now equals the pin) |
| §2.6-1 | CLAUDE.md toolchain sentence | **Fixed** | CLAUDE.md:39 |
| §2.6-2 | CLAUDE.md unwrap sentence | **Open** | CLAUDE.md:115 unchanged; see §2.6-1 above |
| §2.6-3 | `spacetime` on PATH | **Fixed** | CLAUDE.md:25 |
| F17 | `RULES_VERSION` decision | **Fixed** | Both at 10 (`spacetime/module/src/rules.rs:60`, `src/sync/rules_gate.rs:21`); 140b594e documents why eb73e7eb did not bump |
| — | Trust model | **Open (decision)** | Now also governs A1 |

## 4. Prioritised action items

### Magic wand (top 3 by impact)

1. **Decide and write down the store's trust model** — `docs/specs/sync.allium` (or `host.allium`), CLAUDE.md "Store Layering". It is the one open question four separate findings across three reviews wait on (`subscribe_to_epic`, `seed_*`, MCP `identity`, now `prune-repo-paths`). Each review re-reports it; a decision stops that.
2. **`PaneId` newtype, then `RepoPath`/`WorktreePath`** — `src/models/tmux_window.rs`, `src/tmux.rs`, `src/dispatch/split_panes.rs`, `src/runtime/split.rs`. The split-pane code just gained a second failure branch with more pane-id string arguments; the newtype makes argument order a compile error.
3. **Finish moving multi-step writes into reducers** — work-package create/update (A2) and the task relink ordering (C2). Both are the same pattern: the service calls several reducers where one transaction belongs.

### Quick wins

- S1 — `src/agent_tree/open_set.rs:75` onto `src/atomic_write.rs`.
- S2 — `src/cli/store_import.rs:107`/`:174` delete or fix the dead retry.
- §2.6-1 — CLAUDE.md:115 unwrap sentence.
- §2.6-3 — list the per-host tables (one line).

### Small efforts

- **WP-prune-repo-paths:** A1 + T1 (decision item; `src/cli/commands.rs`, `docs/specs/cli.allium`, possibly `spacetime/module/src/tables.rs`).
- **WP-work-package-atomic:** A2 + S4 (`spacetime/module/src/packages.rs`, `src/sync/memory_caller/work_packages.rs`, `src/service/work_packages.rs`, `src/mcp/handlers/work_packages.rs`, conformance tests).
- **WP-learning-verdicts:** S3 (decision item; `docs/specs/learnings.allium`, `spacetime/module/src/learning.rs`, `src/service/learnings.rs`, twin).

### Larger efforts

- **WP-newtypes:** S2 of the morning (`PaneId` first).
- **WP-patch-relink:** C2 of the morning.
- **C1** — split `ingest_role_routed`.

### Decisions needed

- The store's trust model (one operator per store, or not).
- A1: how `prune-repo-paths` treats rows other hosts saved.
- S3: one verdict per (task, learning), or repeats allowed.

## Test and lint run

`cargo test --no-fail-fast`: 32 targets, 6,440 passed, 0 failed, 1 ignored, no `skipping:` lines (`tmux` and `spacetime` both on `PATH`). `cargo clippy --all-targets -- -D warnings`: clean on this host's rustc 1.99.0, which now equals the pin — the morning's N3 symptom (240 `double_must_use` errors) is gone.
