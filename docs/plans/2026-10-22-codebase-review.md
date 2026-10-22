# Codebase review — 2026-10-22

Task #98311. Follow-up to `docs/plans/2026-10-18-codebase-review.md` (22 findings). This pass re-checks every one of those against today's tree, reviews the 13 commits since 2026-10-18 (split-pane rejoin, CVE fix-PR linking, package-view task link, MCP server instructions, agent-name card lead, work-package reorder) and surveys architecture, tests, complexity and smells. One read-only reviewer. Paths are relative to the worktree root.

**What was run.** `cargo test --no-fail-fast`: 32 targets, 6,403 passed, 0 failed, 1 ignored, no `skipping:` lines (`tmux` and `spacetime` both on `PATH`). The four doc/sleep checkers, `test-fetch-cve.sh`, `test-fetch-reviews.sh`, `test-bots-conf.sh`, `test-check-commit-msg.sh` and `build-managed-module.sh --check` all pass. `cargo clippy --all-targets -- -D warnings` could **not** be evaluated on the pinned toolchain from this host (see finding N3).

**Confidence key.** *Verified*: re-read in the code. *Confirmed*: traced through more than one file. *Plausible*: the path exists but depends on environment or timing.

## 1. Executive summary

- **The 2026-10-18 fixes landed.** Of the 22 old findings, 17 are Fixed in code, 3 are Changed (narrowed or documented), 1 is unchanged by design (#22) and 1 stays Open as a decision (#17, `RULES_VERSION`). The separate open question, the trust model, is still open. All five old test gaps are filled.
- **One new Medium in the split-pane work.** A failure after `swap_pane` has already succeeded (rejoin or rename) reports `SwapFailed`, so the board keeps the outgoing task pinned while tmux already holds the incoming pane. The rename arm had this before; the rejoin commit added a second post-swap failure point and no test covers either.
- **The pre-push gate is dead on a distro-rust host.** `rust-toolchain.toml` pins 1.96.0 but `/usr/bin/cargo` is Fedora's 1.99.0 with no rustup, so the hook's clippy step fails with 240 `double_must_use` errors from `async_trait` before a push is even considered. CLAUDE.md offers the dnf path without saying the pin is then ignored.
- **The task's two numeric premises are non-findings.** 149 of the ~150 `allow(dead_code|unused)` hits are in generated `src/spacetime/bindings/`; the last is an `allow(unused_mut)` inside a macro. Every one of the 314 non-test-file `unwrap`/`expect` calls sits in `#[cfg(test)]`-gated code. Clippy's `unwrap_used` lint is doing its job.
- **Architecture holds.** Layering is test-enforced and the read-side convention greps at zero on every axis except two `tui → dispatch` references. The CVE script, package link and MCP instructions are clean apart from Lows.

## 2. Findings

Sections follow the task brief: 2.1 architecture, 2.2 tests, 2.3 complexity, 2.4 smells, 2.5 new defects in the 13 commits, 2.6 CLAUDE.md. The "magic wand" (brief section 5) is the first block of §4, so it sits beside the action items it ranks.

### 2.1 Architecture and patterns

Layering `tui → runtime → service → store → sync → module` is intact. `tests/module_layering.rs:21` (`RULES`) holds the enforced list; the read-side convention from `docs/invariants.md` greps at zero for `tui → tmux`, `tui → store`, `tui → runtime`, `mcp → tui`, `service → tui`. The in-memory twin (`src/sync/memory_caller/`) mirrors the module reducer-for-reducer and is diffed live in `tests/memory_caller_conformance.rs`. `ProcessRunner` (`src/process.rs:278`) has 2 production implementers, 1 decorator (`DeadlineRunner`, `src/dispatch/request_host.rs:127`) and 7 test runners, and the launcher binaries are injected through it (`agent_binaries`, `src/process.rs:304`) — a deliberate, documented exception to trait narrowing.

**A1. Low — `tui` has no row in the layering tripwire, and reaches into `dispatch` twice.** `tests/module_layering.rs:21`; `src/tui/mod.rs:425` (`crate::dispatch::DISPATCH_REQUEST_TIMEOUT`), `src/tui/update/forms.rs:144` (`crate::dispatch::validate_repo_path`). *Verified.* The invariants doc calls `tui → tmux`/`tui → db` a convention "not compiler-enforced"; the test could enforce it today at zero cost, and the two `dispatch` references are a timing constant and a filesystem validator that belong in `models`/`clock` by the doc's own leaf rule. Scenario: a future handler imports `crate::tmux` from `src/tui/update/` and nothing fails. Fix: add `("tui", &["runtime", "tmux", "store", "sync", "spacetime", "mcp", "dispatch"])` after moving the constant and validator to a leaf.

**A2. Low — `serverInfo.version` is hard-coded.** `src/mcp/handlers/dispatch.rs:736` (`"version": "0.1.0"`). *Verified.* No other `CARGO_PKG_VERSION` use in `src/`. Scenario: a client logs the server version for a bug report; every release reports 0.1.0. Fix: `env!("CARGO_PKG_VERSION")`.

**A3. Low — `docs/module-map.md:101` names `join_task_window_into_pane` and `swap_task_window_into_pane` but not `restore_pinned_pane`,** the third public entry point `src/runtime/split.rs:84` calls. *Verified.* The symbol checker only checks that cited symbols exist, so the omission passes. Scenario: an agent adding a fourth split sequence copies the two named functions' pattern and never learns that a leftover window is tagged and must be checked. Fix: add it to the row with the rejoin rule.

### 2.2 Test coverage

Numbers: 6,114 `#[test]`/`#[tokio::test]` in `src/`, 262 in `tests/`, 364 in `spacetime/`; 132k lines of test code against 86k of production (bindings excluded). 336 assertions in `src/` inspect recorded argv (`.calls()` and kin) — the mock-argv style `docs/testing.md` prescribes for "which command we sent", paired with real-tmux targets for "what tmux did". The balance is right for this codebase; the risk is not ratio but the seams below.

**T1. Medium — no test for a failure after `swap_pane` succeeds.** `src/dispatch/split_panes.rs:203-242`; `src/runtime/tests/split_mode.rs` has swap tests at `:365` and `:408` only for the success paths. *Confirmed.* See N1 for the defect; the gap is why it shipped twice. Fix: a `MockProcessRunner` case where `rename-window` (and one where `join-pane`) fails after `swap-pane` succeeded, asserting what the board is told.

**T2. Low — the old test gaps are closed** (listed so nobody reopens them): origin-guard edge cases `src/mcp/middleware.rs:273` (`origin_edge_cases_are_rejected`), non-JSON hook body `src/mcp/handlers/hooks.rs:56`, phoenix refusal `spacetime/module-tests/tests/tasks_and_epics.rs:141` and `tests/memory_caller_conformance/data_rules.rs:315`, over-deep subtree refusal `tests/memory_caller_conformance/data_rules.rs:318-327` (against `MODULE_MAX_EPIC_DEPTH`), zero/one-row scroll indicators `src/tui/ui/kanban/tests.rs:517`. *Verified.*

**T3. Low — the live-only surface is still live-only.** `src/sync/sdk_connector/callers.rs` (46 lines at ≥28-space indent, the deepest nesting in `src/`) and `wiring.rs` run only under a published module; `src/runtime/mod.rs`'s signal handler, health watcher and input thread need a process. *Verified via docs/testing.md "Live-only wiring".* Documented and accepted; noted because the deepest code in the crate has the thinnest instrumentation.

### 2.3 Complexity hotspots

Largest non-test files: `src/tmux.rs` 1,571 (56 `pub fn`), `src/dispatch/worktree.rs` 1,517, `src/runtime/mod.rs` 1,261, `src/dispatch/prompts.rs` 1,212, `src/store/mod.rs` 1,187 (trait declarations; benign), `src/runtime/tasks.rs` 1,182, `src/tui/columns.rs` 1,158, `src/models/tasks.rs` 1,092, `src/main.rs` 1,061.

Longest functions (non-test, body lines): `bootstrap_inner` `src/runtime/mod.rs:814` (128), `status_line` `src/tui/ui/kanban/status_bar.rs:148` (115), `run_tui` `src/runtime/mod.rs:351` (112), `dispatch_task` `src/runtime/commands.rs:138` (109, a flat match router — fine), `update_task` `src/service/tasks/crud.rs:166` (108), `build_task_col_data` `src/tui/ui/kanban/columns.rs:95` (105), `run_feed_command_bounded` `src/feed/exec.rs:91` (105), `handle_tools_call` `src/mcp/handlers/dispatch.rs:745` (104), `serve_dispatch_request` `src/service/tasks/dispatch.rs:95` (102), `classify_card_indicator` `src/tui/ui/kanban/cards.rs:91` (101).

Long parameter lists: `list_epic_subtree` `src/tui/update/board_filter.rs:328` (10), `render_epic_item` `src/tui/ui/kanban/cards.rs:724` (9), `card_title_line` `src/tui/ui/kanban/cards.rs:579` (8), `create_epic_row_with` `src/sync/encode.rs:306` (8), `bootstrap_with`/`bootstrap_inner`/`build_runtime` `src/runtime/mod.rs:799/:821/:978` (7 each), `run_loop` `src/runtime/event_loop.rs:150` (7), `launch_agent` `src/dispatch/agents.rs:427` (7).

**C1. Medium — the composition root is three 7-parameter functions threading the same values.** `src/runtime/mod.rs:799-830`, `:978`. *Verified.* `bootstrap_with` forwards six values to `bootstrap_inner`, which builds `StoreParts` and hands five of them plus new ones to `build_runtime`. Scenario: adding one startup input (a flag, a path) touches three signatures and every test caller. Fix: a `BootstrapInputs { data_dir, port, paths, target, build_store, accept_store_switch }` struct; `build_runtime` takes it by reference.

**C2. Low — `update_task` carries ordering logic that is a data rule in prose.** `src/service/tasks/crud.rs:220-240` (relink-before-patch when `repo_path` changes; `completed_at = None` on entering Done at `:212`). *Verified.* The comments cite the spec, so this is within the "direction of travel" rule, but it is the largest remaining place where the service decides write order instead of the module. Fix: let `patch_task` take an optional `epic_id` relink in one transaction (the module already owns `set_task_epic`'s routing), then delete the ordering branch.

**C3. Low — `list_epic_subtree` takes ten parameters.** `src/tui/update/board_filter.rs:328`. *Verified.* Six of them (`children`, `epics`, `hidden`, `folded`, `qualifies`, `out`) are invariant across the recursion. Fix: a `SubtreeWalk<'a>` struct holding the invariants with a `walk(node, depth, parent_hidden)` method.

### 2.4 Code smells

**S1. Non-finding — dead-code allows and non-test unwraps.** `grep -rE 'allow\((dead_code|unused)' src` gives 150 hits: 149 in `src/spacetime/bindings/` (generated, never hand-edited, excluded from coverage) and one `#[allow(unused_mut)]` on a macro-generated method at `src/mcp/handlers/args.rs:103`. `.unwrap()`/`.expect()` in files outside `tests/` directories: 314. By position relative to each file's first `#[cfg(test)]` (a heuristic, not a parse) every one is inside a `#[cfg(test)] mod` or a cfg(test)-gated module (`src/agent_tree/test_repo.rs` gated at `src/agent_tree/mod.rs:31`, `src/test_log.rs` at `src/lib.rs:36`). The only `allow(clippy::unwrap_used)` outside test modules are four on `MockProcessRunner` helpers (`src/process.rs:577`, `:647`, `:670`, `:677`), the test double that `docs/invariants.md` documents as deliberately ungated so `tests/` targets can see it; each is a mutex-poison unwrap on a test-only path. The real evidence is the lint: `Cargo.toml:109-111` sets `unwrap_used`/`expect_used` to warn and CI's clippy job runs `-D warnings` on the pinned toolchain, so a production unwrap cannot reach `main`. *Verified by grep and spot-reads.* Nothing to fix; CLAUDE.md:115 could say so (see §2.6).

**S2. Medium — primitive obsession around tmux panes and repo paths.** `pane_id: &str` / `pane: &str` at 26 non-test signatures (`src/tmux.rs`, `src/dispatch/split_panes.rs`, `src/runtime/split.rs`, `src/dispatch/agents.rs`), `repo_path: &str` at 43, `worktree_path: &str` at 16. `TmuxWindow` already exists as the newtype for the other half of the same API (`src/models/tmux_window.rs`), and `src/tmux.rs:169-173` documents that pane-or-name parameters stay `&str` on purpose. *Verified by grep.* Scenario: `swap_task_window_into_pane(new_window, right_pane, ..)` and `rejoin_pane(pane_id, window, ..)` take `(&str)` in different positions; swapping two string arguments compiles. Fix: a `PaneId(String)` newtype in `models/tmux_window.rs` (validated `%N` form), adopted first in `split_panes.rs` and the five tmux pane verbs it calls; `RepoPath`/`WorktreePath` later.

**S3. Low — a third copy of the sanitised-git test helper.** `tests/tmux_lifecycle.rs:155`, `tests/worktree_teardown.rs:27`, `src/dispatch/tests/worktree_safety.rs:9`. *Verified.* `docs/testing.md:138-145` names the first two as "the two copies"; the third predates or escaped that note. Scenario: a new env var is added to two of three and a CI container's `commit.gpgsign` breaks the third. Fix: one `tests/common/git.rs` helper (the `src/` copy can keep its own, gated, or move to `test-support`).

**S4. Low — `swap_task_window_into_pane` labels a `has_window` failure as "rename window failed".** `src/dispatch/split_panes.rs:179`. *Verified.* The check runs before any rename and the refusal text misdirects the operator to a rename that never ran. Fix: `.context("checking the outgoing window failed")`.

**S5. Low — CVE fix-PR matching can link a PR that only mentions the advisory.** `scripts/fetch-cve.sh` (the `$bot_prs` fallbacks: body `contains($id)`, then whole-token package name). *Plausible.* A Renovate PR whose body says "does not address GHSA-xxxx" or a monorepo bump titled "update tokio monorepo" is linked, the alert gets `wrap_up_mode: done` and the `fix-pr` label. Mitigated: `src/dispatch/prompts/cve.md:5-13` tells the agent to confirm the PR lands in the advisory's fixed set before merging. Fix if it bites: require the advisory id in title or body for the name fallback too, or emit the match reason in the `Fix PR:` line so the agent knows it was a name match. (The script's exit code was tested: a jq failure inside the per-repo loop exits non-zero under `pipefail`, and `exec_feed_command` `src/feed/exec.rs:220-228` discards non-zero output, so a truncated feed cannot reach Reconcile. Not a defect.)

### 2.5 New defects in the 13 commits

**N1. Medium — a post-swap failure leaves the board and tmux disagreeing about which task is pinned.** `src/dispatch/split_panes.rs:203` (`swap_pane` succeeds) then `:211` (`rejoin_pane` fails) or `:215` (`rename_window` fails) return `Err`; `src/runtime/split.rs:242-249` maps any `Err` to `SplitMessage::SwapFailed`; `src/tui/update/split_pane.rs:244-248` (`handle_split_pane_swap_failed`) shows the error and settles with `Settled::Swap(None)`, and `:144-146` spells the assumption out: "a swap's [pane] still holds the previous occupant". That is false once `swap_pane` has run. *Confirmed.* Scenario: the operator swaps task B into the pane pinned from A; the outgoing pane's `join-pane` into A's leftover window fails (window killed by A's `exit_session` in the same second). tmux now shows B's agent in the board pane; the board still says A is pinned, so `[s]`-exit tries to restore A's pane id into B's window. The rename arm has had this since before the review window; 1f96c697 added the rejoin arm. Fix: on a failure after `swap_pane`, attempt the reverse `swap_pane(right_pane, new_pane_id)` and report `SwapFailed` only if that also succeeds; otherwise send `SwapOpened { pane_id: new_pane_id, task_id }` plus a separate error for the leftover. Add the T1 tests. Spec: `docs/specs/split-pane.allium` `SwapSplitPane` should say what "settled" means when the exchange half succeeded.

**N2. Low — S4 above (misleading context on the pre-swap check), same commit.**

**N3. Medium — the pre-push gate cannot run on a distro-rust host.** `rust-toolchain.toml` pins `1.96.0`; this host has `/usr/bin/cargo` = rustc 1.99.0 (Fedora) and no `rustup`, so the pin is ignored. `cargo clippy --all-targets -- -D warnings` exits 101 with 240 errors, 191 of them `clippy::double_must_use` on every `#[async_trait]` trait (first at `src/agent_tree/pane.rs:158`). `.githooks/pre-push:8` runs that exact command as step 2, after a `cargo fmt` under the same wrong version. *Verified.* Scenario: an agent on this machine finishes a task, runs the hook, sees 240 errors unrelated to its change, and either bypasses the hook or spends a session "fixing" `async_trait`. CI (`.github/workflows/ci.yml:28`, dtolnay 1.96.0) is unaffected, so `main` is still gated. Fix: (a) the hook checks `rustc --version` against `rust-toolchain.toml` first and fails with "install rustup or `cargo +1.96.0`"; (b) CLAUDE.md:39 says the dnf path does not honour the pin. Separately, `double_must_use` on `#[async_trait]` methods is a known newer-clippy false positive the project will meet for real when the pin moves past 1.99; that is a toolchain-upgrade item (an `allow` with a comment, or dropping `async_trait` where the trait is object-safe without it), not a hook-repair item.

**Reviewed and clean:** package-view task link (`src/runtime/work_packages.rs:76-91` `link_new_task`, `src/tui/update/system.rs:153-163`; refused-link test at `src/runtime/tests/command_dispatch/store_writes.rs:367`), MCP server instructions (`src/mcp/handlers/dispatch.rs:707-711`, identity-free and tool-name-free by test), card lead/heading helpers (`src/models/tasks.rs:414-444`), `@dispatch_awaits_pane` tagging (`src/tmux.rs:1330-1365`; pane ids are never reused within a tmux server, so a stale tag cannot match a new pane), agent-name card lead (bfb36c45: `src/models/agents.rs:74-92` `with_session_of` lays the name over the decoded row, `src/sync/decode.rs:183` leaves it `None`, `src/tui/ui/kanban/cards.rs:532-535` widths assume lowercase-ASCII agent names, which `spacetime/module/src/agent.rs:106` `AGENT_NAMES` guarantees), work-package `sort_order` reorder (4c8a13ba: `src/tui/update/navigation.rs:159-211` refuses a swap across the package/non-package seam and persists through `WorkPackageCommand::SetSortOrder`; `spacetime/module/src/packages.rs:48-50` applies the patch; column changes do not bump `RULES_VERSION` by the constant's own rule). Skimmed by diff: 451f4b94 (crash status names the agent), 0e4c8c82 (docs).

### 2.6 CLAUDE.md improvements

1. **Toolchain.** CLAUDE.md:39 gives `dnf install rust-std-static-wasm32-unknown-unknown` as an equal alternative to `rustup target add`, but a distro rustc ignores `rust-toolchain.toml` and the hook's clippy step then fails for every push (N3). Say which is supported, or add the version check to the hook and point at it.
2. **Unwraps.** CLAUDE.md:115 reads as if production unwraps exist and are only caught at push time. Today there are none outside `cfg(test)`; say "the tree has zero and the hook keeps it there", so an agent does not audit 314 test-module calls (S1).
3. **`spacetime` on PATH.** CLAUDE.md names only `tmux` as the binary the full suite needs. `docs/testing.md:229` says the live-store targets (`tests/spacetime_module.rs`, `tests/memory_caller_conformance.rs`) skip without `spacetime` except under CI. One clause beside the tmux sentence.
4. **The trust model is still undocumented.** Zero hits for "trust model" or "single-operator" in `docs/` or `CLAUDE.md`. `report_pr_status` now checks the host (`spacetime/module/src/pr_status.rs:87`), `subscribe_to_epic` still only checks the subscriber string is non-empty (`spacetime/module/src/config.rs:143-149`), and the `seed_*` reducers have no caller check (`spacetime/module/src/seed.rs:14`). Deciding "one operator per store" or not settles all three; record it in `docs/specs/sync.allium` or `host.allium` and name it from CLAUDE.md's "Store Layering" paragraph.
5. **`RULES_VERSION` decision.** The constant's doc (`spacetime/module/src/rules.rs:21-27`) says a rule moving into the module bumps it, and eb73e7eb moved three; it is still 9 on both sides (`rules.rs:52`, `src/sync/rules_gate.rs:21`). Either bump to 10 or add a line to the doc saying why that move did not count.
6. **Module map drift** (A3).

## 3. Status of the 2026-10-18 findings

| # | Finding | Status | Evidence |
|---|---|---|---|
| 1 | Hook route lost its Content-Type requirement | **Fixed** | `src/mcp/handlers/hooks.rs:56-62` refuses non-`application/json` with 415 before reading; `is_json_content_type` `:79` |
| 2 | Origin guard ignores the port | **Fixed** | `src/mcp/middleware.rs:43` strict `parse_authority` (userinfo, path, `null` rejected); `:81-93` compares the bound port, `require_port` for `Origin`; tests `:231-310` |
| 3 | Task-detail scroll bound double-counts the separator | **Fixed** | `src/tui/ui/kanban/popups/task_detail.rs:94-97`: `height − (2 + header_height)` |
| 4 | Hook/protected-branch push rejections "retryable" | **Fixed** | `src/repo_sync.rs:348-352`: markers `[rejected]`, `non-fast-forward`, `fetch first`; `[remote rejected]` no longer matches |
| 5 | `window_target` reads any `list-panes` failure as "no windows" | **Fixed** | `src/tmux.rs:189-197` bails unless `is_no_server`; comment `:178-183` now accurate |
| 6 | Soft-fail `has_window().unwrap_or(false)`; `is_no_server` too broad | **Fixed** | `src/dispatch/agents.rs:789-791` uses `?` with context; `src/tmux.rs:414-421` requires `No such file`/`Connection refused` beside `error connecting to` |
| 7 | Atomic write replaces a symlinked config | **Fixed** | `src/atomic_write.rs:40-43` canonicalises a symlink and writes the target |
| 8 | Concurrent update loss not fixed by atomic write | **Changed** | `src/dispatch/trust.rs:37` documents read-just-before-write; accepted as the narrow window |
| 9 | `add_worktree` classifies errors by English stderr | **Fixed** | `src/dispatch/worktree.rs:676-685` asks `rev-parse --verify --quiet refs/heads/<name>` instead |
| 10 | `pull_base` aborts blindly and drops the abort result | **Fixed** | `src/dispatch/finish.rs:178-181` `abort_merge_if_in_progress`; `:188-191` reports an abort failure in the message |
| 11 | `ensure_worktrees_excluded` not atomic, skips `.git` files | **Fixed** | `src/dispatch/worktree.rs:581-591` reads `commondir` through `worktree_admin_dir`; `:619` atomic write |
| 12 | Feed exec: read error leaves the group alive; stderr cap kills nothing | **Fixed** | `src/feed/exec.rs:127-160` kills the process group on either pipe's cap or error (`kill_process_group` `:79` is still a `kill(1)` subprocess, now awaited) |
| 13 | Failed editor-window create can kill a pre-existing window | **Fixed** | `src/runtime/editor.rs:255-262` `disarm_window_cleanup` before `clear_session_slot`; `:511` |
| 14 | `exit_session` etc. say "Unknown caller task N" as raw JSON-RPC errors | **Fixed** | `src/mcp/handlers/types.rs:512-523` `fetch_caller_task` returns `tool_error("Task N not found")`; callers `crud.rs:378/:465`, `wrap_up.rs:353` |
| 15 | `push_hint_spans` byte-slices the label | **Fixed** | `src/tui/ui/shared.rs:460-472` uses `label_chars.as_str()` |
| 16 | Managed store adopts any HTTP 200 on `/v1/ping` | **Fixed** | `src/spacetime/managed_store.rs:277` requires `has_store_signature` (`:290`, the store's CORS headers); `:298-304` |
| 17 | Module rule moves without a `RULES_VERSION` bump | **Open (decision)** | `spacetime/module/src/rules.rs:52` and `src/sync/rules_gate.rs:21` both 9; the constant's own doc `:21-27` says a rule move bumps it. 4c8a13ba's new column does not count by that doc |
| 18 | Wrong comment in `check_task_patch` (epic-exists check) | **Fixed** | `spacetime/module/src/tasks_epics.rs:445-449` now checks the epic exists; comment `rules.rs:209-211` is true |
| 19 | Twin `apply_patch_task` lacks rollback | **Changed (half)** | `src/sync/memory_caller/tasks_epics.rs:275` validates before any side effect; but `:304` writes the row and `:312` `reroute_on_repo_change` can still refuse afterwards, whereas the module's `:349`/`:366` sequence is one transaction. `relink_task`'s only failure is `write_task` validation already passed, so unreachable in practice — Low |
| 20 | `flatten`/regroup deletes skip `validate_epic_deletable` | **Changed** | `spacetime/module/src/grouping.rs:94-121` (`delete_if_empty_repo_group`) argues the `has_tasks || has_children` guard makes the check vacuous. The argument holds; no test pins it |
| 21 | Uninstall prompts error out on EOF | **Fixed** | `src/setup/uninstall.rs:41-46` `answer_or_decline` maps the error to "no" and prints it |
| 22 | Task delete discards the tear-down of an open `exit_session` row | **Unchanged by design** | `spacetime/module/src/tasks_epics.rs:415-421`; spec records it |
| — | Test gaps (phoenix refusal, over-deep subtree, scroll indicators, origin edges, non-JSON hooks) | **Fixed** | See T2 |
| — | Trust model | **Open (decision)** | `report_pr_status` gained a host check (`pr_status.rs:87`); `subscribe_to_epic` and `seed_*` did not; still no doc |

## 4. Prioritised action items

### Magic wand (top 3 by impact)

1. **Make a split-pane swap settle on what tmux did, not on whether the whole sequence succeeded** — `src/dispatch/split_panes.rs`, `src/runtime/split.rs`, `src/tui/update/split_pane.rs`, `docs/specs/split-pane.allium`. Reverse the swap on a post-swap failure or report the new occupant; add the T1 tests. This is the only path today where the board's belief about tmux is set by a code path's error value instead of a tmux read.
2. **Guard the toolchain in the hook and the docs** — `.githooks/pre-push`, `CLAUDE.md:39`, a 10-line `scripts/check-toolchain.sh`. Every agent on a distro-rust box currently hits 240 false clippy errors at push time (N3). Cheapest fix with the widest blast radius.
3. **`PaneId` newtype, then `RepoPath`/`WorktreePath`** — `src/models/tmux_window.rs`, `src/tmux.rs`, `src/dispatch/split_panes.rs`, `src/runtime/split.rs`. 26 pane-id strings in the exact code the first item touches; the newtype makes the swap/rejoin argument order a compile error (S2).

### Quick wins (one file, under an hour each)

- A2 `env!("CARGO_PKG_VERSION")` — `src/mcp/handlers/dispatch.rs:736`.
- A3 module-map row — `docs/module-map.md:101`.
- S4 context string — `src/dispatch/split_panes.rs:179`.
- §2.6 items 2 and 3 — `CLAUDE.md:115` and the tmux sentence.
- F17 decision line — `spacetime/module/src/rules.rs` (bump or document).
- S3 — move the third `git` helper onto `tests/common/`.

### Small efforts (one work package each)

- **WP-split-settle:** N1 + T1 + S4 (`src/dispatch/split_panes.rs`, `src/runtime/split.rs`, `src/runtime/tests/split_mode.rs`, `docs/specs/split-pane.allium`).
- **WP-toolchain:** N3 + §2.6-1 (`.githooks/pre-push`, `scripts/`, `CLAUDE.md`, `tests/ci_gates.rs` if a script is added).
- **WP-layering:** A1 (`tests/module_layering.rs`, move `DISPATCH_REQUEST_TIMEOUT` to `src/clock.rs` or `models`, `validate_repo_path` to `models/paths.rs`).
- **WP-bootstrap:** C1 + C3 (`src/runtime/mod.rs`, `src/tui/update/board_filter.rs`).

### Larger efforts

- **WP-newtypes:** S2 (`PaneId` first; 26 sites).
- **WP-patch-relink:** C2 (`spacetime/module/src/tasks_epics.rs`, `src/sync/memory_caller/tasks_epics.rs`, `src/service/tasks/crud.rs`, conformance tests; also closes F19 for good).
- **CVE matching precision:** S5, only if a false link is observed (`scripts/fetch-cve.sh`, `scripts/test-fetch-cve.sh`).

### Decisions needed

- The store's trust model (§2.6-4): one operator per store, or not. Unblocks `subscribe_to_epic`, `seed_*` and the MCP `identity` question from the 2026-10-17 report.
- `RULES_VERSION` after eb73e7eb (F17).
