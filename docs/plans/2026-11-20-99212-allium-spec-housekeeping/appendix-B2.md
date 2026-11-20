# Appendix B2 — split-pane, agent-health, keep-awake

Condensed report of one read-only review agent for task #99212, at commit b03f5ba0 (main, 2026-11-18). Part of docs/plans/2026-11-20-99212-allium-spec-housekeeping.md. Line numbers drift as packages land: search for the cited symbol, not the line. Acceptance checks run from the worktree root.

# B2 — split-pane, agent-health, keep-awake (review at b03f5ba0)

Read in full: split-pane.allium 1-1876, agent-health.allium 1-1441, keep-awake.allium 1-274; full diffs `git diff 10b68974 HEAD` for all three (198, 186 and 49 changed lines). Allium references: assessing-specs.md and actioning-findings.md read in full; language-reference.md searched by topic, not read in full; patterns.md not opened. Plan rules (98956 33-132, 98983 76-167) and Appendices B and E read in full.
Whole-set check at HEAD: 0 diagnostics and 0 analyse findings in these three files; gate passes (43 specs, 16 baselined warnings).
Scratch-tested (scratch-B2/): `SwapInFlight.created(task: task)`, `respawned_pane(...)`, `dispatch/DispatchTask(task)` and ExpireBackgroundWait's `and (host = null or host.owner = caller())` add no diagnostic (counts unchanged: 11 definition.unused, 9 field.unused, 4 missingSourceHint).
Cited `path::symbol`s in both specs all exist (23 checked), and so do all cross-spec rule/guarantee names (40 checked).

## Carried from Appendix B / E (98983)

All resolved: agent-health H-n1–H-n9; split-pane S-n1–S-n12; X-1 (now `agent_health/StopApplied` -> FlipTaskToReview, a single owner); keep-awake K1–K3.
Still by-design: H-n10 ("predates the field" compatibility notes, 487-490, 589-591, 842-845).
Nothing is still open. New findings below come from the rewrite (X-1, S-n4, host-owner guard) and from reading the whole file.

## split-pane.allium

| Code | Class | Line | Problem | Remedy |
|---|---|---|---|---|
| B2-S-1 | drift | 1212 | PinTaskInSplitPane creates `SwapInFlight.created()` without `task`; the code sets it (`src/tui/update/split_pane.rs::handle_swap_split_pane`: `InFlight::swap().moving(Some(task_id))`, shared by pin and swap), and agent-health WindowLiveness/PinnedWindowIsNotGone relies on it ("a swap's incoming task") | `SwapInFlight.created(task: task)` |
| B2-S-2 | drift (code comment) | — | `src/tui/types/state.rs` cites "`core.allium`: `Rearrangement`" (enum doc) and "See `Rearrangement` in `docs/specs/core.allium`" (`InFlight` doc); Rearrangement is in split-pane.allium (core.allium 12 says so) | point both at split-pane.allium |
| B2-S-3 | bp | 891 | `RespawnedPane(split_pane.pane)` is a PascalCase black-box call, so it reads as a trigger or constructor | `respawned_pane(split_pane.pane)` |
| B2-S-4 | bp (N1) | 308, 312, 433, 435, 519, 975/1004/1010, 1213, 1335, 1706 | SplitPaneEntryStarted, SplitPaneSwapStarted, TaskWindowRestored, SplitPaneKilled, UserInformed, PaneWorkingDirectorySet and ConfirmRetry are observable outputs that no rule listens to; only TmuxFocusMoved has the N1 comment (965-966) | extend that comment to list them all |
| B2-S-5 | bp (order) | 18-27 | `config` (no section header) comes before Entities and Variants | move it after the entities under a "Config" header |
| B2-S-6 | mech | 541 | "(see SwapSplitPane above)": SwapSplitPane is below (1321) | "below" |
| B2-S-7 | mech | 233-238 | "Those are the only two writes the store receives" contradicts 245-255: the subagent clear and the Stop are a third write (`spacetime/module/src/agent_state.rs::subagent_clear`) | "the board's task-row write carries only two fields: …" |
| B2-S-8 | mech | 451 | "Emitted from four places" sits in ExitSplitMode, which now listens on SplitExitRequested | "SplitExitRequested is raised from four places" |
| B2-S-9 | mech | 1777-1779 | "the two confirmations": the surface provides three (ConfirmDetachTmux, ConfirmBatchDetachTmux, ConfirmQuit) | "the confirmations are the [y] answers to the [T] (one task or a selection) and quit prompts" |
| B2-S-10 | mech | 1286 | `dispatch::kill_task_window` is not in `path::symbol` form | `src/dispatch/split_panes.rs::kill_task_window` (and `::kill_task_window_if_present`) |
| B2-S-11 | bp | 990 | ActivateTask raises dispatch's `DispatchTask(task)` unqualified, yet the same rule qualifies `dispatch/ResumeTask` (1008). ConfirmRetry (999) is split-pane's own output (no other spec declares it), so it stays | `dispatch/DispatchTask(task)` (scratch-tested clean) |
| B2-S-12 | bp (present tense) | 1040-1047 | "does not yet resolve…", "there are no remote hosts to mark yet": roadmap wording, and the second is stale now that multi-host rows exist (core/Host, is_locally_owned) | say what the board does now (`src/tui/mod.rs::foreign_worktree_refusal` says "another machine") and drop "no remote hosts yet" |
| B2-S-13 | by-design | 975, 1004, 1010, 519 | `about:` literals of UserInformed have no declared enum. Declaring one adds a baselined `definition.unused` (the literal-only enum gap) | leave |
| B2-S-14 | by-design | 1714, 1759 | "measured on tmux 3.7b", "Measured, both ways": evidence for external tmux behaviour, not history | leave |

## agent-health.allium

| Code | Class | Line | Problem | Remedy |
|---|---|---|---|---|
| B2-H-1 | mech | 185-189 | "A deferred Stop is applied only inside the write that drains the count to zero (see HookSubagentStop) … the non-draining kind": stale since X-1. HookStop, ExpireBackgroundWait and DetachTmux apply it too (FlipTaskToReview), and "non-draining" was dropped at 227 | "A deferred Stop is applied only by a StopApplied write (FlipTaskToReview); this clear raises none" |
| B2-H-2 | bp | 1294, 1212 | Guarantee name `HookNeverOpensTheDatabase` is from the SQLite era; its body now says "store" | rename it to `HookNeverOpensTheStore`; update the two citers in `tests/hooks.rs` (doc comments at the "cheap half" test and the real-binary test) |
| B2-H-3 | bp (dated) | 1167 | "checked on 2026-11-02" | "observed:" (keep the payload evidence) |
| B2-H-4 | bp (N1) | 164, 291, 321, 1117, 1119 | DesktopNotification, SenderToldToResume and SenderToldWhyNotResumable are observable outputs with no listener and no N1 comment | add one line: "observable output; no rule listens" |
| B2-H-5 | bp | 541-551 vs 437 | HookStop states the host-owner guard structurally (`task.host = null or task.host.owner = caller()`); ExpireBackgroundWait has the same guard in the module (`spacetime/module/src/agent_state.rs::expire_background_wait` -> `require_acting_for_task_host`) but only as prose (574-577) | add `and (host = null or host.owner = caller())` to its `for … where` (scratch-tested clean) |
| B2-H-6 | QUESTION | 434-437 | See Q-H1 | — |
| B2-H-7 | mech | 565, 904-909, 1013, 1050-1056, 1069-1077 | Code citations not in `path::symbol` form: "(spacetime/module/src/agent_state.rs)" with no symbol; "`TaskService::resolve_peer_message_target` (src/service/tasks/crud.rs …)"; "`TmuxWindow::task_id` (src/models/tmux_window.rs)"; "`detect_task_transition_notifications`, src/tui/update/agent.rs"; "(src/main.rs, Commands::Hook)"; "`NotificationWrite::from_kind` (src/models/agent_events.rs)" | rewrite each as `path::symbol` (e.g. `spacetime/module/src/agent_state.rs::expire_background_wait`); thin HookPeerMessageSent's implementation walk (1050-1077) to behaviour plus one citation |
| B2-H-8 | drift (code) | — | `src/keybindings/table.rs`, `toggle_notifications` row: "Toggle the notification panel". There is no panel: N flips desktop notifications (ToggleNotifications; `src/tui/update/lifecycle.rs::handle_toggle_notifications`) | "Toggle desktop notifications" |
| B2-H-9 | drift (code comment) | — | `spacetime/module/src/agent_state.rs::subagent_clear_and_void_pending_stop` doc: "matching the SQL's `UPDATE ... WHERE id = ?`". No SQL remains | "a missing task writes nothing". Only if Q-H1 is (a): a module edit means a wasm rebuild, and `rotate-previous-store.sh` once it is on main |
| B2-H-10 | bp (present tense) | 1374-1378 | "The agent tool endpoint has always carried the same constraint … written down here because nothing else wrote it down": narrative | "The agent tool endpoint has the same limit (startup.allium: mcp_server_entry)." |
| B2-H-11 | by-design | 530-533, 560, 960 | "Verified against Claude Code 2.1.322": version evidence for external behaviour, not history | leave |
| B2-H-12 | by-design | 167, 294, 324 | DesktopNotification `about:` literals have no enum (same literal-only gap as B2-S-13) | leave |

## keep-awake.allium

No actionable finding. K1–K3 are resolved. `ReleaseKeepAwakeOnExit`'s new `requires: cause = quit` matches the code: a quit drops the `Hold` (`src/runtime/keep_awake.rs::exec_release_keep_awake`; `src/keep_awake.rs` holds the helper's stdin pipe), and every other end relies on the pipe closing (HoldDiesWithTheBoard). The error strings match `src/tui/update/keep_awake.rs`.
- B2-K-1 by-design 154, 166: `"…" + reason` string concatenation is outside the language's arithmetic table, but it is used across the set (cli, tasks, dispatch, observability, …). It is a set-wide convention, not a per-spec fix.
- Not raised: GNOME-helper fallback (`src/keep_awake.rs::hold_command`); task #99234 owns it.

## Verified against code (new prose since 10b68974)

- HookSubagentStop's drain still requires `not waiting_on_background`: `spacetime/module/src/agent_state.rs::subagent_stop` checks it before `apply_pending_stop_if_drained`. DetachTmux's `subagent_clear` drains unconditionally, which matches the spec. No drift.
- ExitSplitMode 438-444 (pinned row missing): `src/runtime/split.rs::exec_refuse_exit_pinned_row_missing` sends an error and kills nothing.
- PinTaskInSplitPane 1313-1318 (home tag cleared): `src/tmux.rs::clear_pane_home`, called from both places in `src/dispatch/split_panes.rs`.
- keep-awake quit: the runtime's drop releases the hold (`src/runtime/mod.rs`, the `keep_awake` field doc). The signalled path was not traced; the spec relies only on the process ending there.
- B2-S-12: `foreign_worktree_refusal` says "this task's worktree is on another machine". Its doc comment blames the host registry being out of scope, but the `hosts` table now exists (`spacetime/module/src/agent_state.rs`, host registration).
- B2-H-8: "notification panel" appears only in `src/keybindings/table.rs` (no test, snapshot or doc pins it).

## Questions

**Q-H1 (agent-health HookStop 434-437, ExpireBackgroundWait 574-577).** Only Stop and the background-wait expiry check that the caller acts for the task's host (`require_acting_for_task_host`). That scope came from codebase-review V4 (docs/plans/2026-11-15-codebase-review.md, task #99228, commit cbbb9614), which looked only at those two reducers; neither V4 nor store-split D11/W11 decides the rest (D11 covers reducers that *name* a host). No open task covers it (task_search). The other hook reducers check visibility only: `record_user_prompt_submit`, `record_pre_tool_use`, `record_notification`, `subagent_start`/`subagent_stop` and the session-start clear. So any viewer who can see a task can still flip it Review->Running, raise needs_input or void a deferred Stop. The spec's own rationale ("An agent's hooks reach the store through its own host's board") covers every hook.
(a) Extend the guard to every hook reducer: spec `requires` on each Hook* rule, module checks, and memory_caller mirror plus conformance test. Recommended: one rule, one rationale, and it closes the review->running and void-Stop paths.
(b) Keep Stop and the expiry only, and say in HookDelivery why the other writes are open.

## Work packages

1. **split-pane tidy + in-flight task** — files: docs/specs/split-pane.allium, src/tui/types/state.rs (two doc comments). Findings: B2-S-1..S-12. No question. No ordering constraint. After the edits, run `allium:weed` on split-pane.
   Acceptance check (fails today; run from the worktree):
```bash
! grep -nE 'SwapInFlight\.created\(\)|RespawnedPane|see SwapSplitPane above|dispatch::kill_task_window|the two confirmations' docs/specs/split-pane.allium && ! grep -nF 'docs/specs/core.allium`' src/tui/types/state.rs | grep -q Rearrangement && ! grep -qF '(`core.allium`: `Rearrangement`' src/tui/types/state.rs && grep -B10 'no rule listens' docs/specs/split-pane.allium | grep -q PaneWorkingDirectorySet && [ "$(grep -n '^config {' docs/specs/split-pane.allium | cut -d: -f1)" -gt "$(grep -n '^entity SplitPane' docs/specs/split-pane.allium | cut -d: -f1)" ] && ./scripts/check-allium.sh
```
2. **agent-health tidy + host guard** — files: docs/specs/agent-health.allium, tests/hooks.rs (two comments), src/keybindings/table.rs (one description), and, only if Q-H1 is (a), spacetime/module/src/agent_state.rs (B2-H-9) plus the module hook reducers, src/sync/memory_caller/ and tests/memory_caller_conformance.rs, plus a RULES_VERSION/BOARD_RULES_VERSION bump. Findings: B2-H-1..H-10. Question: Q-H1. Runs independently of package 1; if both run at once, merge `main` first (split-pane's DetachTmux guidance cites FlipTaskToReview and VoidSubagentsAndPendingStop, and neither package renames them). Any module edit stales `module.wasm`: rebuild it with `./scripts/build-managed-module.sh`.
   Acceptance check (fails today):
```bash
! grep -nE 'HookNeverOpensTheDatabase|2026-11-02|applied only inside the write that drains' docs/specs/agent-health.allium tests/hooks.rs && ! grep -n 'notification panel' src/keybindings/table.rs && [ "$(grep -c 'caller()' docs/specs/agent-health.allium)" -ge 2 ] && grep -q 'no rule listens' docs/specs/agent-health.allium && ./scripts/check-allium.sh
```
3. keep-awake: no package.

Both checks were run at b03f5ba0: each exits 1, failing on the cited lines. `./scripts/check-allium.sh` passes today (exit 0).
