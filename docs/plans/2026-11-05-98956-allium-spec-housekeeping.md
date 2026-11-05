# Allium spec housekeeping — investigation and work plan

Task #98956. Investigation only: no spec was edited. The fixes are the subtasks of
the epic "Allium spec housekeeping" (see "Work packages" below).

Line numbers are as of commit `8376c7e9` (main, 2026-11-05). They drift as
earlier packages land: search for the cited symbol, not the line.

## Baseline

CLI `allium` 3.6.1 (upgraded from 3.5.0 for this review), 34 specs, all
`-- allium: 3`, 44,655 lines. Run from the repo root:

```bash
allium check docs/specs/*.allium     # always check the whole set
allium analyse docs/specs/*.allium
```

| Severity | Code | Count |
|---|---|---|
| error | `reference.undefinedImportedAlias` | 3 (all `core.allium` naming itself `core/…`) |
| warning | `definition.unused` | 20 |
| warning | `reference.unknownName` | 14 (qualified collections such as `core/Hosts`) |
| warning | `entity.unused` | 7 |
| warning | `externalEntity.missingSourceHint` | 6 |
| warning | `surface.unusedBinding` | 5 |
| info | `rule.unreachableTrigger` | 300 (115 named in another spec, 185 named nowhere else) |
| info | `field.unused` | 99 |
| analyse | findings | 30 (agent-tree 16, sync 10, cli 2, keep-awake 2) |

No gate runs `allium check` today.

## Decisions taken with the user (apply them in every package)

1. **Key presses** enter through a small per-spec TUI surface that `provides`
   the spec's own key triggers (the dispatch.allium picker precedent). Rules do
   not listen to `keybindings/KeyActionRun`.
2. **Actors live once in `core.allium`**: `Operator` (replacing the three copies
   in cli, sync, spacetime-seed and the undeclared `User`), one MCP actor
   (`McpClient`, replacing keybindings' `McpAgent` and `McpCaller`), and minimal
   actors for system parties (board process, board clock, tmux, the Claude Code
   hook runner, the shared store, the operating system). Each spec keeps its own
   MCP tool surface facing `core/McpClient`, because `provides` must be local to
   the rule's spec.
3. **Board lifecycle**: `startup.allium` owns one start event and one exit event.
   The exit event covers every cause: confirmed quit, retired by a later launch
   (SIGHUP/SIGTERM) and startup abort. keep-awake, split-pane and agent-tree
   consume `startup/…`. `AppStarts`, `AppExits`, `BoardStarted`, `BoardExiting`
   and `ApplicationExits` go.
4. **History**: present tense only. Keep the decision ids (D1–D12, W1–W13,
   P1–P5, X1–X6 in store-split; A1–A16 in agents) as short present-tense
   entries, because other specs cite them. Drop narrative, task numbers,
   "was X until #NNNN", SQLite-era text and names of removed code. Keep
   `path::symbol` citations only where `scripts/check-doc-symbols.sh` or a
   reader truly needs them.
5. **Checker gaps**: restructure where a clean form exists; otherwise add a
   one-line comment saying why the warning is expected. One task files the
   upstream issue.
6. **Gate**: the last package adds `allium check` to `scripts/gates.sh`. It fails
   on any error and on warnings beyond a committed baseline file.

Per-spec behaviour questions are NOT decided. Each package lists its questions;
the agent asks the user before changing that behaviour.

## Checker facts (verified on scratch copies)

- Check the whole set. A single-file check reports `use.unresolvedPath` on every
  `use`.
- **Cross-spec triggers:** a rule that listens for a trigger raised in another
  spec must `use "./other.allium" as other` and listen on `other/Trigger`.
  Qualifying the *emission* (`split/X(...)`) does not clear the diagnostic;
  qualifying the *consumer* does. A `use` cycle is accepted without diagnostic.
  An unqualified trigger emitted by any loaded file is also counted reachable,
  but by name collision only — do not rely on that.
- **Many or no emitters** (a reducer any client calls, a key, a timer, a tmux
  outcome): the consuming spec declares a local `surface … provides:` (human or
  agent party) or a `contract` (system party). An unused `facing` binding raises
  `surface.unusedBinding`: use the binding in a guard, pass it as a trigger
  argument, or write `facing _: Actor`.
- **Qualified collections** (`core/Hosts`, `core/PollOwners`, `core/Retrievals`,
  `core/SavedRepoPaths`, `keybindings/KeyBindings`) as a method receiver raise
  `reference.unknownName`. `for x in core/Learnings` is accepted. Prefer a
  `for … where` form, a join lookup `core/Learning{id: x}` / `exists …`, a
  relationship, or a black-box function. Note: 3.6.1 does not validate
  unqualified collection names at all, so writing `PollOwners` unqualified only
  hides the warning.
- **Field reads across files are never credited.** A field of `core.allium` read
  only by another spec stays `field.unused` (≈80 such). Acceptance checks say
  "no *new* `field.unused`", not "none".
- **Entities referenced only by their plural collection** (`IdSequences`,
  `SentinelColumns`) are `entity.unused`; bind the type once (`for s: IdSequence
  in IdSequences`, or a typed trigger parameter).
- **Types used only in a contract signature** are `definition.unused`
  (`PaneView`, startup's six outcome enums, `Bump`, `Preceding`). Comment them.
- **Non-built-in collection methods are not flagged but are invalid**: only
  `.count .any() .all() .first .last .unique .add() .remove()` exist. Seen:
  `.where(…) .exists(…) .contains(…) .is_empty() .max(…) .map(…) .filter(…)
  .first(pred) .count(pred) Entity.find(…)`. Replace with `X where pred`, `in`,
  `.count = 0`, joins or black-box functions.
- `@guarantee` belongs on surfaces/contracts, never inside a rule; `@guidance`
  is always last.

## Conventions every package follows

- **Left alone:** the right-hand side of an `ensures` assignment already reads
  the pre-rule value. Delete `x = old(x)` and `x = unchanged`; write
  `if old(f) = …` as a `let` before `ensures`.
- **Render and cascade events** (`*Rendered()`, `LinkedItemDeleted`,
  `TaskFinishes`) become derived values, `becomes`/`transitions_to` triggers or
  invariants, not orphan triggers.
- **Timers:** one board-clock contract in core (or startup) provides `Tick`,
  `FeedPollTick` (renamed from `TickInterval`) and `ManagedStoreHealthDue`;
  agent-health owns the tmux liveness surface that provides `WindowGone`.
- After the edits, run `allium:weed` on the package's files and fix the drift it
  finds (spec or code; ask the user when it is a behaviour choice). Fix latent
  code bugs found in passing rather than reporting them.
- Record any new convention with `record_learning`.

## Work packages

Epic: #69688. Wave 0 runs first, alone. Wave 1 packages touch mostly their own files and can
run in parallel after wave 0; where a remedy edits another cluster's file, the
package says so — rebase on `main` before wrap-up. The gate runs last.

| # | Task | Package | Files | Findings (appendix) |
|---|---|---|---|---|
| 0a | #98958 | Foundation: shared actors, lifecycle events, board clock, core self-alias errors | core, startup (+ the duplicate `actor Operator`s in cli, sync, seed; `McpAgent` in keybindings) | Decisions 1–3; A core 1; E startup X1/X2 |
| 0b | #98959 | File upstream issue for the checker gaps on juxt/allium-tools | none | "Checker facts" |
| 1 | #98960 | core model clean-up | core (+ agent-health:621, learnings, board-layout, mcp-task-tools, store-split for shared derivations) | A core 2–16 |
| 2 | #98961 | storage, settings, host | storage, settings, host (+ store-split settings rules, startup trim guard) | A storage/settings/host |
| 3 | #98962 | agent-tree | agent-tree | B agent-tree |
| 4 | #98963 | split-pane | split-pane | B split-pane |
| 5 | #98964 | agents + agent-health | agents, agent-health (+ store-split ReleaseClaimOfFailedRequest) | B agents/agent-health, X5 |
| 6 | #98965 | feeds: mechanical, entry points, stale prose | feeds (+ cli provides, startup artefact emission, store-split patch_epic) | C feeds M, E3–E7, D1–D3, D6, B1–B7 |
| 7 | #98966 | feeds: rewire the feed-cycle chain | feeds | C feeds E1, E2, Q1–Q4, D4, D5 |
| 8 | #98967 | feed-scripts + schedules | feed-scripts, schedules (+ store-split, pr-workflow emissions) | C feed-scripts, schedules |
| 9 | #98968 | pr-workflow | pr-workflow (+ `src/dispatch/mod.rs` comment) | C pr-workflow |
| 10 | #98969 | store-split: collapse the migration log, re-home rules | store-split (+ domain specs if re-homed, CLAUDE.md if renamed) | D store-split history + question 1 |
| 11 | #98970 | store-split: module reducer boundary and prune drift | store-split (+ emitters in dispatch, pr-workflow, epics, mcp-task-tools, feeds) | D store-split E1–E5, drift, mechanical |
| 12 | #98971 | sync | sync (+ seed emission) | D sync |
| 13 | #98972 | spacetime-seed, -snapshot, -memory-store | the three files | D seed/snapshot/memory-store |
| 14 | #98973 | startup + cli | startup, cli (+ seed snapshot outcome triggers) | E startup, cli |
| 15 | #98974 | dispatch + dispatch-prompt | dispatch, dispatch-prompt | E dispatch, dispatch-prompt |
| 16 | #98975 | keep-awake, onboarding-guide, observability | the three files (+ startup receives the StatusLineDecorator path guarantees) | E keep-awake, onboarding-guide, observability |
| 17 | #98976 | tasks | tasks | F tasks |
| 18 | #98977 | epics + work-packages | epics, work-packages | F epics, work-packages |
| 19 | #98978 | mcp-task-tools + learnings | mcp-task-tools, learnings (+ possible code bug in `src/dispatch/agents.rs`) | F mcp-task-tools, learnings |
| 20 | #98979 | workspaces + admin-surfaces | workspaces, admin-surfaces (+ epics/work-packages MCP params) | G workspaces, admin-surfaces |
| 21 | #98980 | board-layout restructure + board-visuals | board-layout, board-visuals (+ tasks, work-packages consumers) | G board-layout, board-visuals |
| 22 | #98981 | keybindings | keybindings (+ observability listener) | G keybindings |
| 23 | #98982 | Gate: `allium check` in `scripts/gates.sh` with a warning baseline | scripts, docs/testing.md | Decision 6 |

Shared remedies that cross packages, so do not duplicate them:

- The repo-wide **review tag set** derivation `Task.is_review_tagged` is added
  by package 1; packages 11, 18, 19, 21 use it.
- **Settings write chain** (settings vs store-split `SaveSettingForHost`) is
  owned by package 2; package 10/11 leave those two rules alone.
- **`TaskStatusWrittenByHand` and `TaskClosed` emissions** from store-split and
  pr-workflow are owned by package 8 (schedules).
- **`records_usage` / `KeypressHandled`** seam is owned by package 22.
- **`WrapUp` → `WrapUpRequested`** question is owned by package 9; package 19
  (learnings :452) follows its answer.
- **`AgentLaunched` qualification** (`dispatch/AgentLaunched`) appears in
  packages 3, 5, 15, 19: each does its own file.
- **`PollOwners` qualified collection** appears in packages 6, 12, 20: each does
  its own file with the same form.

## Appendices: findings per cluster

Each appendix is the condensed report of one read-only review agent. Codes:
`mech` = mechanical, behaviour-neutral; `entry` = missing entry point;
`drift` = spec and code disagree (cite checked); `QUESTION` = ask the user.

### Appendix A — core, storage, settings, host

Global facts:
- analyse: no findings for this cluster.
- Checker credits cross-module ENTITY refs but not FIELD refs: core field.unused stays ~90 even when other specs read the field (e.g. Task.model). Acceptance checks must run `allium check docs/specs/*.allium` filtered per file and say "no NEW field.unused", not "none".
- Qualified triggers `startup/FirstRun()` etc. are validated (negative test rejected). Cyclic host<->sync `use` accepted.
- No spec redeclares a core type; duplication is prose/behaviour.

core (mature shared model):
1 mechanical self-alias errors: 933 core/Task->Task, 935 core/Host->Host, 1565 core/config->config; comment 1057.
2 by-design 81 field.unused used elsewhere -> header note.
3 mechanical remove Agent.is_paused(964), Epic.subtask_statuses(1140).
4 decide: code-only aliases Task.is_stale(785), is_crashed(786), pr_number(822), Epic.is_root(1074), Learning.is_active(1316), Retrieval.retrieved_at(1344): delete or keep as doc. One rule for the file.
5 by-design missingSourceHint x4 Worktree(27), TmuxWindow(38), TmuxPane(42), UserIdentity(63): add "governed by" comment; no annotation silences it.
6 drift VerdictKind(332) unused; code has LearningVerdict table (src/spacetime/bindings/learning_verdict_type.rs, apply_learning_verdicts). Add entity LearningVerdict + uniqueness invariant; delete orphan block 1347-1356.
7 NotificationKind(283) -> type agent-health:621 `kind: core/NotificationKind?` (code src/models/agent_events.rs::NotificationKind).
8 Retrieval(1340) -> learnings surfacing rules ensure core/Retrieval.created.
9 drift SQLite-era prose: 1099-1113, 1128-1131, 263-268, 695, 1881-1888, 1210 — rewrite to say who enforces (module reducer e.g. feed_ingest.rs::index_existing) and drop migration history.
10 review tag set copied: core 77-86 prose, board-layout 675/677, mcp-task-tools 1512, store-split 1647 -> derived Task.is_review_tagged.
11 hardcoded: 704 base_branch "main" -> config.default_branch; 1731 default_feed_interval -> config.min_feed_interval.
12 prose->expression invariants: SubStatus per status (252-261), reset defaults (270-274), SplitPane 1522-1523, SavedRepoPath verify_command per github_path (1184-1186).
13 section order: Agent enums 907-930 and WorkspaceRole(1035) under entities; prose sections 373-418, 1817-1902 (epics cites "Interval literals" 4x); collection uniqueness invariants inside entities -> top-level Invariants; Host @invariant(500) before invariant(569).
14 stale refs: 1704 Message flash is in board-visuals (also board-layout:21); header Excludes 7-14 incomplete.
15 duplicated prose: core 1246-1255 vs settings 36-46; host vs identity core 46-62, 467-493, host 54-62; identity persistence core 495-499, host 88-105, storage 37, settings 11-21.
16 question (low): runtime UI state entities BoardSelection, SplitPane, Rearrangement, PrPollState in core — move to consuming spec?

storage (prose only):
1 by-design DataDirectory unused; optional host.allium uses storage/DataDirectory.
2 rename StoreInUseNeverOpensSqlite(59) -> NoSqlite (grep citers settings:60, header 13).
3 storage:39 "(startup.allium: store pin)" -> StoreAddressRecord (startup:556).
4 empty Open Questions 74-76.

settings:
1 QUESTION/recommendation: SaveSetting(82)/ClearSetting(102) emitted by nobody; store-split 2572-2597 SaveSettingForHost/ClearSettingForHost write the same row on SettingSaved/SettingCleared (also unemitted, only one checks host ownership D11). Recommend: settings rules emit SettingSaved(host: local_host()), store-split listens settings/SettingSaved; feature specs (tasks ToggleSectionCollapse 882, board-layout 1139, feeds, epics) emit settings/SaveSetting. Decide whether in-memory state like collapsed_sections persists via save.
2 garbled 111-112 RepoBaseBranch/UnsubscribeFromEpic sentence.
3 duplicated rationale; settings:37 points to host but core 430-445 draws it.
4 history REMOVED block 28-34 (#4972), "since Phase 9" (65).
5 SettingsAreStoredPerHost (50-75) cited as symbol but only a comment.

host (well developed):
1 entry: use startup + sync; startup/FirstRun (141), startup/RenameHost (235), sync/UserIdentityOffered (319,369,396). Sync side: use host; host/UserIdentitySettled, UserIdentityConflict, HostRenamed.
2 core/Hosts.count = 0 (143) unknownName AND contradicts core 537-548 -> delete line (host_file_present guards).
3 trim guard duplicated startup:1257 -> drop in startup.
4 open questions in guidance 303-313 -> Open Questions (458-461).
5 analogy 42-45 inaccurate.

Proposed WPs: core self-alias+dead decls; wire host triggers; LearningVerdict+Retrieval entities; type Notification kind; settings write chain one owner (Q); core SQLite prose; core expressions/config/is_review_tagged; core section order + dedup; storage/settings/host tidy.

### Appendix B — agent-tree, split-pane, agents, agent-health

Cross-cluster patterns:
- C1 "left alone" written two ways: `old(x)` (agent-tree 1356,1640,2224-2233,2290-2291,2393,2467,2489-2493) and `= unchanged` (split-pane 590-592,601,943). Neither in language ref; RHS of ensures already reads pre-state.
- C2 three board-lifecycle vocabularies: AppStarts/AppExits (agent-tree), ApplicationQuits/ApplicationExits (split-pane), LaunchBoard/BoardWindowRetired (startup).
- C3 two key-press idioms: UserAction("W") (keep-awake:177) vs keybindings/KeyPressed(user, namespace, key) (keybindings:708).
- C4 agent-end cleanup duplicated 6x: agents 147-154, 211-216, 253-259; agent-health 135-140, 406-411; split-pane 37-50.
- C5 stale task/commit citations and `== IMPLEMENTED ==` blocks throughout.
- C6 checker doesn't count refs in contract signatures as use (PaneView, PaneResumeAnswer).

agent-tree (mature):
- A1 by-design PaneView/PaneResumeAnswer unused (C6) — comment; resolved if A3 types the trigger.
- A2 QUESTION: 6 analyse conflicts on AgentTreePane.status=shown. HideAgentTreePane(908)/ExitAgentTreeRenderer(1682) vs AgentTreeGitFailureKeepsLastGoodTree(1619), AgentTreeCommitListFailureKeepsLastList(2280), AgentTreeAgentListFailureKeepsLastList(2456) all `ensures: pane.status = shown`. Five more rules also write shown (1226,1248,2004,2560,2622); guidance 1251,2015 says it's a guarantee not a change. Code: src/agent_tree/run.rs single loop, worker-thread results discarded on exit. Option (a) promise -> make one invariant/@guarantee "a failed read never closes the pane"; (b) real write -> keypress outranks tick.
- A3 entry points: AppStarts(740)/AppExits(780) -> lifecycle (C2); AgentLaunched(801) -> `use dispatch`, dispatch/AgentLaunched; WindowGone(1125) -> agent-health-owned runtime surface; AgentTreeDiffPaneOpenFailed(2000)/AgentTreeAgentJumpFailed(2556) -> tmux actor surface; AgentTreeResumeAnswered(2596) -> Board actor surface demands BoardPaneView; AgentTreeAgentListRead(2634) -> emit from RefreshAgentTreeAgentList; AgentTreeKeyPressed(2175)/AgentTreeAgentCursorMoved(2652) -> AgentTreeCompanionPane provides.
- A4 mechanical DiscardAgentTreePaneOnWindowCleared (1145-1147) free `task` -> agent.task (same bug split-pane 665-667).
- A5 mechanical invariant DiffPaneExistsWhenSomethingIsOpen (2729-2737) missing `and`.
- A6 drift pending_jump_reads(512) no writer; code src/agent_tree/state.rs::adopt_agent_list increments/resets. Notice text differs.
- A7 ExitAgentTreeDiffRenderer provided (3656) but no rule.
- A8 SplitAgentTreePaneOnAgentLaunch (806-829) incomplete created fields; ShowAgentTreePane(1054) doesn't reset.
- A9 invariants range over value types AgentTreeNodes/FileDiffs (2753..2881, 2825).
- A10 C5 citations; IMPLEMENTED blocks 751-776, 855-905, 945-1051, 1096-1119; comment 609-611 "allium check (3.16)" wrong version; two @guidance in OpenAgentTreeFileDiff (1721,1727); AgentRowShowsAgentName duplicated (agents.allium:390).

split-pane (rules only, no surfaces):
- S1 entry: operator board surface for UserAction("s")(104,148), (" ")(737), ("prefix+space")(891), ConfirmDetachTmux(33), BatchDetachTmux(75), ConfirmQuit(385,439).
- S2 entry: tmux actor surface SplitPaneEntrySettled(1327), SplitPaneSwapSettled(1196), SplitPaneExitSettled(362,523), SplitPaneClosed(580), TmuxWindowSplit(1396).
- S3 SplitPaneFocusBorder(695) rendering rule -> @guarantee; FocusChanged rule missing (704-705).
- S4 mechanical BatchDetachTmux(74-92) emits DetachTmux (nobody listens; should be ConfirmDetachTmux); `current_window` unbound; rule shares name with trigger.
- S5 mechanical DetachTmux ensures mis-indented (41-50).
- S6 mechanical SplitPaneRespawnOnWindowCleared(665-667) free task.
- S7 stale guidance 247-249 (four emitters), 339, 829 (right_pane_id not a field).
- S8 prose invariants 1473-1503 -> invariant/@guarantee; delete empty Open questions 1505-1509.
- S9 C1/C5 lines; JumpToAgentWindow name clash with agent-tree 2526.

agents (behaviour-complete, framed as obsolete migration target):
- G1 qualify: DispatchRequested(139) -> store_split/; AgentLaunched(226,345) -> dispatch/; RetryFresh(247) unprovided (see G4); DeleteTask(282) -> tasks/ with board surface; McpToolCalled(360) unprovided, overlaps mcp-task-tools:733; AgentBackfillRequested(304) code-actor contract (stamp_rules_version_on_connect + store import).
- G2 drift FailedDispatchReleasedClaim(205) never emitted by store-split ReleaseClaimOfFailedRequest; code agent.rs AGENT_END_LAUNCH_FAILED.
- G3 QUESTION AgentsGoWithTheirTask(281-288) requires done; code delete_task_side_effects 4 callers not status gated. Remove agents with any deleted task?
- G4 drift EndAgentWhenWorktreeGoes(246) listens RetryFresh; should be Task.worktree transitions_to null.
- G5 drift BackfillCurrentAgents(317) copies t.tmux_window (no longer a field).
- G6 mechanical: `core/Task` -> `core/Tasks` (307,411,434); SubagentEntry -> core/SubagentEntry (154,259); let between ensures (155); chosen_mode unbound (162); for over nullable (147); HandedOutAgentName under Enumerations(118); delete resolved Open questions (520-526).
- G7 C4 unify agent-end; EndAgentWhenLaunchFails (211-216) misses subagent clear.
- G8 header 10-16 + migration section 462-518 finished -> present tense, move history to ADR/archive; ResumeKeepsTheAgent(225) -> invariant; AgentIdentity viewer; duplicate AgentRowShowsAgentName.

agent-health (rules, surface without provides):
- H1 HookDelivery(934) add provides for PreToolUse/PostToolUse(256), Stop(317), SubagentStart/Stop(377,399,493), SessionStart(509), UserPromptSubmit(546), Notification(621), SendMessageObserved(781), SendMessageUndelivered(849); code actor ClaudeCodeHook.
- H2 board runtime surface for Tick(27, shared pr-workflow:105) and WindowGone(129, shared agent-tree, pr-workflow).
- H3 DetectCrashedAgent guidance (176-179) non-running case has no rule.
- H4 A15 skip-when-no-agent only in prose (247-249); wrap agent writes in `if exists agent`.
- H5 stale paths: 657 db/queries/tasks.rs -> src/store/queries/tasks.rs::record_notification + spacetime/module/src/agent_state.rs::record_notification; 343-347 SQL wording -> try_record_stop; 87 pre-v50.
- H6 "10-minute"(268)/ACTIVE_THRESHOLD(276) -> config; notifications_enabled free global (142,199,229).
- H7 Q1 (1112-1114) -> open question; C4; C5 107-120,704-714,765,843; 835-838 indentation; `-- prose:` (936) -> @guidance.

Cluster WPs: X1 key vocabulary, X2 lifecycle vocabulary, X3 runtime Tick/WindowGone surface, X4 one "left alone" idiom, X5 one owner for agent-end cleanup.
Questions: A2, G3, X1, X2.

### Appendix C — feeds, feed-scripts, schedules, pr-workflow

Checker: in full-set check, an UNQUALIFIED trigger emitted by any loaded file counts reachable; QUALIFIED emission doesn't count. Local surface provides silences; unused facing binding -> pass actor as trigger arg (cli CliListRepos(operator) precedent).

feeds (behaviour rich, zero surfaces/actors):
- M1 1206 core/PollOwners.where -> for/where (verified).
- M2 FeedItem(301), ShippedScriptReport(416) unused -> contract FeedItemParse (prose 3115-3186) + shipped_script_reports signature (verified).
- M3 852-861 two Implementation lines; M4 380 "below", 406 "above" wrong.
- E1 exec seam: FeedCommandCompleted only re-emitted by GroupedFeedUpsert (1873) self-cycle; FeedCycleStarted (1360) unconsumed; exit_status/stderr free (2663, 2749, 2860, 3072). Add process-runner contract/rule.
- E2 ingest: FeedEmissionPosted(1437) unprovided; FeedSync(additive:false) never emitted; FeedIngested(1501) unconsumed; strategy rules listen board-side FeedCommandCompleted, UpsertFeedTasks fires for grouped too; `additive` unbound 1723, 1869, 2233, 2251.
- E3 TickInterval(964) -> timer contract; rename FeedPollTick (FEED_POLL_INTERVAL src/feed/mod.rs:154).
- E4 VerifyFeedCommand(494) -> cli DispatchCommandLine (cli:633).
- E5 ManualFeedRefresh(3314) key r; OverrideFeedOwner(1143) y on epics FeedOwnerTakeoverOffered (epics 113, 136-137).
- E6 ApplyFeedScriptsArtefact(579, 809) <- startup ApplyStaleConfigOnConsent(1124) emit per-artefact trigger.
- E7 EpicFeedCommandSet(1180) <- store-split patch_epic.
- Q1 FeedCycleWanted(1220) orphan; FeedTick(1014), ManualFeedTrigger(3381), ServeFeedRefreshRequest(3467) inline setter checks.
- Q2 DispatchRequiresRepoPath(2475) misplaced -> dispatch.
- Q3 3054-3070 GroupedFeedUpsert migration defect (copies tasks, loses status) untracked -> open question or task.
- Q4 3387-3409 status_bar strings in ensures.
- D1 `dispatch setup` gone but prose remains: 269, 413, 441-443, 542, 595, 683, 816, 864-867, 872-873 (run_setup_in), 3594; real entry src/setup/config_update.rs::apply_feed_scripts.
- D2 2572-2579 SQLite-era test reference.
- D3 3641-3675 resolved SQLite notes; 1749, 2984 ON CONFLICT.
- D4 empty repo_path refusal only in src/dispatch/agents.rs::dispatch_with_prompt; module dispatch_request.rs no check -> claim then fail; confirm claim released.
- D5 fields not in core: epic.last_run (2553,2712), epic.subtree (2866,3079), parent_epic_id (2194-2195, 2234-2235), can_auto_group (2180), repo_group_sub_epics (2260).
- D6 store-split open question 2704 answered (feeds 3467-3468, 3485-3487; src/feed/mod.rs:506).
- B1 section order (contracts 40 before values 289/409; rules before Rules header; prose after invariants 3629-3675).
- B2 `for epic in Epic` (992); unqualified Tasks/Task{}/Epic.find (1651,1724,1859,2192,2234,2252,2866,3079); filter/group_by (1856-1857, 2163); shadowed `epic = epic` (1724).
- B3 truncate 2000 (2666, 2752), 500 chars (3176) -> config.
- B4 now() 2553; `"…" or "…"` 3476-3477; local_host unqualified.
- B5 RouteSignals precedence twice (51-69, 87-93); signal/approved restated in feed-scripts; OverrideFeedOwner ensures (1151-1162) dup store-split OverridePollOwner.
- B6 pseudo-constructs named like symbols: FeedItemParse, FeedSync, FeedStderrNotice, IngestSkipsRetiredFeedItems, OneFeedCycleAtATimePerEpic, AReviewTaggedFeedItemNamesItsPr.
- B7 41 task refs, chronology (2635-2643, 1385, 2780-2784).

feed-scripts (prose only, 0 declarations):
- Q5 keep prose-only or constructs (contract per script 186-262, enum CiLabel 346-351, BOT_AUTHORS config 600, invariants 681, 665).
- B8 56-59 contradicts feeds 546-549 ("inert until user edits").
- B9 approved semantics 392-395, rescue rationale 405-413 dup feeds 137-168.
- B10 19 history.

schedules (well developed, no surface):
- ScheduleSet(99)/ScheduleCleared(126) emitted qualified by mcp-task-tools 354-355, 863 (not credited); flip would cycle. Recommend local surface ScheduleArguments facing McpAgent.
- TaskStatusWrittenByHand (179,198,238) <- store-split UpdateTaskIsOneTransaction (1133) emits (code schedule.rs::schedule_after_status_write).
- TaskClosed(220): NOT equivalent to status->done (apply_session_close clears next_due for every exit_session incl pr). Producers pr-workflow ExitSession + store-split RequestCloseTask emit.
- B11 config.schedule_refire_after (65) prose only (168).
- B12 cron grammar prose under Value header (68-92) -> contract CronSchedule; config order.
- B13 store_split/DispatchRequests.any (148).
- B14 ScheduledLaunchFails redundant exists (287, 291).

pr-workflow (rules only):
- M5 qualify split/: SessionRequestClaimed (493, 966), SessionCloseAccepted (735), RebaseConflict (1070); emit split/SessionRequestDone/Failed (515-524, 985-992) (verified).
- E8 Tick(105) timer contract. E9 PrLearningsGateCheck(50) -> cli pr-gate. E10 WrapUp (628, 694) never emitted; real WrapUpRequested (mcp-task-tools 1541, store-split 1635); learnings 452 same.
- D7 CreatePr (27-47) dead; src/dispatch/mod.rs:112-115 comment cites wrong file -> delete + fix comment.
- Q6 WrapUpPr(627)/WrapUpDone(693): delete and fold into store-split RequestWrapUp, or re-point at WrapUpRequested with no ensures? PrCreated unconsumed.
- B15 "30s" literal (237). B16 bare `conflict` (184, 1082) vs core/SubStatus. B17 856-858 annotation mid-sentence. B18 M5/M6 history (791-801, 293-301, 339-342, 375-380, 414-416, 97-100). B19 1039 `not exists request.tmux_window` on string.

Questions: Q1-Q6, key-press convention.

### Appendix D — store-split, sync, spacetime-seed, spacetime-snapshot, spacetime-memory-store

Checker facts:
- Must check whole set (`cd docs/specs; allium check *.allium`), else use.unresolvedPath everywhere.
- Qualifying at the EMITTER does not clear unreachableTrigger; qualifying at the CONSUMER (`use` + `x/Trigger`) does. Cyclic `use` accepted.
- Remedy rule: one emitting spec -> consumer uses+qualifies; many/no emitters (e.g. reducer any client calls) -> consumer spec declares a surface/contract providing it.
- Qualified collections (core/Subscriptions, core/Epics, core/PollOwners) never resolve: restructure (relationship, derived value, black-box fn).
- Entities referenced only via plural collection are flagged entity.unused; silence by binding by type somewhere.

store-split (behaviour rich, no actor/surface, 36 unreachable; lines 4-636 migration log):
- Present-tense rewrite of 4-13, 60-64; collapse 111-633 decision log to one present-tense paragraph per decision id (D1-D12, W1-W13, P1-P5, X1-X6 are cited from other specs 17-49 times each — keep ids defined). Remove removed-code names, RULES_VERSION history (1173-1174, 1226, 1253-1255), Resolved blocks (2678-2690, 2700), M6 marker 489-515, D9 origin list missing `schedule` (267-272 vs 674).
- QUESTION: move non-request rules (task writes 1001-1289, UpdateEpic/WorkPackageIsOneTransaction, PR state 2448-2520, poll-owner/settings host checks 2522-2601) to domain specs; store-split becomes spec of request rows (rename touches CLAUDE.md Store Layering).
- SaveSettingForHost/ClearSettingForHost (2572-2596) duplicate settings:81/101 — one owner.
- drift: RequestRowsArePruned (2027-2109) one prune incl FeedRequests; code has two: request_retention.rs::prune_terminal_requests (dispatch+session) and feed_request.rs::prune_feed_requests (feed writers). Guidance 2083 "both tables" (3).
- drift: UpdateTaskDoneWritesACloseRow (1177-1227) trigger arity 4 vs 3 in UpdateTaskIsOneTransaction(1134); guidance 1221 failing open session request missing from ensures.
- mechanical: Epic{id: scope_id} 2542, 2561 -> core/Epic. has_host_work(806) real in code (src/models/session_request.rs) -> use in SessionRequestTimesOut.
- Entry E1: actor StoreClient + surface ModuleReducers providing TaskPatchRequested, TaskUpdateRequested, EpicUpdateRequested, WorkPackageUpdateRequested, PhoenixSuccessorRequested, SubStatusBatchPatched, PrStatusReported, PrUnreadableReported, PollOwnerClaimed, PollOwnerOverridden, SettingSaved, SettingCleared, DispatchRequested, WrapUpRequested, ExitSessionRequested, CloseTaskRequested, FeedRefreshRequested, *RequestDelivered claims, DispatchProvisioned, SessionRequestDone, FeedRequestDone, FeedRequestFailed. Emitters qualify split/X (dispatch 264,1131; pr-workflow 515,519; epics 995; mcp-task-tools 533,1258,1541,1766).
- E2 use sync; sync/StoreReachedAtStartup (1535,1941).
- E3 QUESTION DeleteTask (1565,1968,1997): one module event TaskRowDeleted for all delete paths (key, batch, feed stale)?
- E4 TaskWriteRequested(1008) = seed StoreRowsWriteRequested(682) = tasks_epics.rs::store_task; declare once.
- E5 FeedTaskRemovedAsStale(2245): feeds IngestFeedEmission emits it.

sync (most complete):
- unknownName 141 core/Subscriptions, 154 core/Epics, 1573 core/PollOwners -> relationship/black-box (covered_epics_of(identity)).
- mechanical: UnsubscribeFromEpic 1571-1576 shadowed `epic`, owner vs operator compare; 442/459/479 unbound `operator`; 1512 ConnectionEstablished -> ConnectionAccepted; 614 ordinal asks; 167 "— —"; ANSWERED blocks 1905-1933 (1928-1933 contradicts 411-414).
- drift: rules_version = 9 (386) vs code 38 (rules.rs RULES_VERSION / rules_gate.rs BOARD_RULES_VERSION); history 376-385; bump policy 370-372 contradicts module doc comment. QUESTION: keep literal or cite only.
- drift: covered_epics note 158-215 cites SharedRows::retired_without_task/TaskCrud::retired_without_task (gone; now feed_ingest.rs::retired_without_task in module).
- drift: open question 1940 resolved by UnsubscribeFromEpic (#8196).
- drift: SubscribeToAgents 769-770 "unfiltered" vs visible_agents view (src/sync/sdk_connector/queries.rs).
- NothingIsSubscribedBeforeAnIdentityExists 1676-1679 vacuous.
- QUESTION: delete RefuseToStartWhereOnlySomeChangesReachTheStore (1181-1226), holds by construction; trigger BoardDrawn. Recommend delete.
- Phase 12/#4916 history 55-85, 416-424, 929-930, 958-964, 1372-1376.
- Entry S1 use host: host/UserIdentitySettled (8 rules), UserIdentityConflict (473,1482), HostRenamed (888). S2 use startup: StoreConnectionAttemptedAtStartup (396), BoardDrawn. S3 InitialSubscriptionApplied(436), ViewRowUpdated(795) -> StoreBoundary provides. S4 SharedRowMutationRequested (1081,1129,1229) -> BoardWrites surface facing Operator or @guarantees. S5 BoardNeedsRows(917) -> fold into invariant AConnectedBoardDrawsOnlyDeliveredRows(235). S6 RowReadForImport(1019) <- seed emits sync/RowReadForImport per row; rename seed-local trigger RowsWithARemovedStatusAreDropped (seed:923) which collides with rule name.

spacetime-seed (mature):
- IdSequence(298) by-design plural-only; bind by type.
- StoreRowsWriteRequested(682) = store-split E4.
- seam S6 (923-924).
- drift 69-94 exclusion list misses feed_requests, store_owner, rules_version, schema_version; 972-979 import list misses user_profiles, workspaces, memberships.
- Resolved blocks 1150-1169 stale counts; Retired/Seeding(removed) 13-16, 775-798 -> one line; 664 RefuseSchemaMismatch -> RefuseMismatchedSchema(556); 481-483 archaeology.

spacetime-snapshot:
- SentinelColumn(81), StoreSelection(92), ManagedStoreLocation(106), ManagedStoreHold(113) plural-only -> bind by type. SentinelColumn.table/.sentinel (82,84) -> invariant or accept. Entity under Enumerations (79-85). Loose comment 357-361.
- W1 WriteRows (179,202,219): seed emits TableRowsWriteRequested, snapshot listens seed/...
- W2 QUESTION ManagedStoreStopRequested(325) vs startup ManagedStoreStopped (1923,1958); ManagedStoreLocated(298) unemitted. Recommend startup emits, snapshot consumes.
- No config drift (98304, version 2, 1000000 match).

spacetime-memory-store (finished incremental build):
- drift: ReducerDomain/covered_domains/is_complete not in code (91-102,114-136,149-154,160-209,233-247); collapse to ReducerConformance contract + StandInFidelity + two rules.
- ConformanceIsCiGated (299-321) names wrong test files -> tests/memory_caller_conformance.rs + dir.
- BoardReadsRequested(274) -> MemoryStoreCallSurface provides (327-329).
- handle.shared_rows (278) undeclared.
- missingSourceHint SpacetimeModuleReducers(39) -> use store-split; TestSuite(44) accept.
- 25-27 ungrammatical.

Questions: (1) re-home store-split non-request rules + rename; (2) one TaskRowDeleted event; (3) delete dead sync guard; (4) rules_version literal vs cite; (5) managed-store stop/location ownership.

### Appendix E — startup, cli, dispatch, dispatch-prompt, keep-awake, onboarding-guide, observability

Cross:
- X1 board lifecycle names: start BoardStarted (keep-awake:59) vs BoardDrawn (startup:2123 -> sync:1182); exit BoardExited (startup:1920,2077), BoardExiting (keep-awake:159), ApplicationExits (split-pane:477); SIGHUP/SIGTERM (startup:1933, src/runtime/store_target.rs) no trigger. QUESTION: one exit event any cause, owned by startup? Recommend startup owns one start + one exit, board-process surface facing BoardProcess.
- X2 undeclared User (startup 2361,2441,2468,2485; keep-awake 178; dispatch 1449; observability 503), OperatingSystem (keep-awake 214); Operator x3 (cli 569, seed 877, sync 1689). Declare once in core.
- X3 qualified collections core/SavedRepoPaths (cli 311,341; dispatch 111,1451,2296 unflagged), keybindings/KeyBindings. Checker inconsistent.
- X4 enums used only in contract signatures/literals: startup 661-704 (6), dispatch-prompt Bump(37), Preceding(90). by-design; note + upstream report.
- X5 @guarantee inside rules: dispatch 10 (ProvisionDispatch 9 from 310, DispatchTask 173); dispatch-prompt ~25 (242-1472), no surface. Move to surfaces/contracts.
- X6 config: statusline_chain_timeout declared dispatch:46 used only observability:584; cli:131 default_store_address "http://127.0.0.1:3000" vs startup:812 "127.0.0.1:3000".

startup (mature):
- X4 enums.
- mech: StoreReachedAtStartup (1892,1974,2038,2120), StoreUnreachableAtStartup (2102) -> use sync, sync/... (sync doesn't import startup today; NOTE: sync cluster proposes sync uses startup too — cycles accepted).
- entry: BoardExited (X1); ManagedStoreHealthDue(1643) timer -> contract BoardClock or temporal with requires.
- entry: StoreImportRequested(1559) -> cli DispatchCommandLine provides, consume cli/StoreImportRequested; ConnectToTheNamedStore(1565) dead.
- stale counts StartupAbortsOnlyOnAnUnusableSubstrate 2215 "seven", 2226 "fifteen" (16, store_switched #28710), 2231 "five" lists six.
- ACCEPTED open questions 2531, 2533 -> decisions.
- "database" wording 572-575, 620-622, 2420 (record beside --data-dir, src/startup/launch.rs::recorded_store_server).
- impl detail 1047, 537-548, 1649-1650 calls contract ops in ensures, 1654 "Found on Oct 5".
- 1490-1491 blank line in @guidance.
- ConfigDrift.items exposes (2447, 2473) value not bound.

cli (detailed):
- X3 311, 341.
- mech ImportReported(438) -> use seed, seed/ImportReported.
- entry CliSnapshotCommandFinished(409): prefer seed TakeSnapshot(354)/RestoreSnapshot(408) emit SnapshotWritten/SnapshotRestored; literals dump_to_file/restore (412-414) inline enum.
- malformed ensures CliCommandsRefuseAStoreThisInstallDoesNotUse (220-222) predicate/event `or` chain -> requires + ensures CliFailed.
- 142 vs 35 owner of ImportTargetsTheNamedStore... (snapshot:260).
- X6 default_store_address.
- drift SetVerifyCommandViaCli rationale (286-292, src/cli/commands.rs) stale after 8376c7e9.
- keep open questions 665, 667; trim history 258-260.

dispatch (uneven; ProvisionDispatch ~900 lines 228-1123):
- entry A: surface AgentRelaunch providing ResumeTask(1204; raised by split-pane:775, mcp-task-tools:1374, agent-tree:344), RetryResume(1336), RetryFresh(1375; agents:247 too).
- entry B: RepoPathUsed(69), BaseBranchUsed(100) -> picker provides or tasks create emits dispatch/.
- mech DispatchProvisioningClaimed(229) -> split/ (alias imported).
- DRIFT after 8376c7e9: RecordRepoPath (68-95) path-keyed upsert vs per (path,host) rows (core 1179-1200, spacetime/module/src/config.rs::save_repo_path); RecordBaseBranch(99-146)/BaseBranchPicker/BranchHistoryCapped(2295) per repo_path vs core says shared by github_path; code config.rs::record_base_branch keys repo_path. QUESTION. RepoPathPicker (1451) may list path twice.
- X5 guarantees; split ProvisionDispatch prose into named rules/contracts.
- SandboxedAgentExecution (2030-2099) history -> one guarantee; `let sandbox_enabled = false`.
- impl detail 2026, 2094-2098.
- host accessors this_host(74) vs local_host() vs core/local_host().
- empty Open Questions 2302.

dispatch-prompt:
- X4 Bump/Preceding.
- mech AgentLaunched x6 (231,473,729,881,1290,1315) -> use dispatch.
- MISSING GUARD: ClassifyDependencyBump(880), RenderPrReviewRunbook(1289), RenderReleasePrRunbook(1314) fire on mode resume; add mode guard.
- stale header 216-221 emitters; 235-237; ~11 "previously" passages (73-89).
- X5 ~25 guarantees -> surface AgentPrompt facing DispatchedAgent.
- section order Contracts after Enumerations (swap).

keep-awake (complete):
- entry BoardStarted(59), BoardExiting(159) -> X1.
- mech unusedBinding 178, 214 -> `facing _:` (precedent sync:1706).
- blocks literals (76) -> enum IdleEffect; OperatingSystem undeclared.

onboarding-guide:
- X3 keybindings/KeyBindings(245).
- drift GuideBinding (113-132) missing description/when/label (docs/guide/index.html:262; src/keybindings/tests.rs::guide_declares_only_real_bindings); invariants 80-89, 242-249 omit description.
- 196 `.any()` used as selector.

observability (9 triggers unconnected):
- entry: McpToolCall(135) MCP surface; KeypressHandled(300) -> keybindings/KeyActionRun (matches board cluster records_usage seam); failures TrajectoryWriteFailed(198), FeatureUsageWriteFailed(265), BulkRowDecodeFailed(412), SingleRowDecodeFailed(449), BulkQueryFailed(464), TeardownTargetAlreadyGone(224), TeardownAttemptFailed(247) -> ObservedFailures surface or producers emit.
- DRIFT/QUESTION decode policy SQLite-era: SingleEntityReadFailsOnUndecodableRow (443-458) vs src/sync/rows.rs drop_undecodable; QueryFailurePropagatesFromBulkRead (460-476); "database" 140, 67-68, 549.
- X6 statusline timeout.
- wrong home: StatusLineDecorator guarantees 618, 639, 669 -> startup; 597 rename (no open database).
- impl detail 732-782, 145-158, 164-174.
- TrajectoryEntries (486,494) value not entity; 495 type mismatch.
- keep open questions 789-793.

Questions: board exit single event; base-branch history per path vs github_path; undecodable row by-id fail vs not found; checker limitation policy.

### Appendix F — tasks, epics, work-packages, mcp-task-tools, learnings

Tooling: provides must be LOCAL to rule's spec; an actor can be imported (`facing client: mt/McpClient`). `requires: exists core/Learning{id: x}` join works. Qualified collection as method receiver flagged; non-built-in methods (.exists .where .contains .is_empty Entity.find) NOT flagged but invalid. analyse 0 findings. Actors duplicated: Operator in cli:569, sync:1689, seed:877; McpClient (mcp-task-tools) vs McpAgent (keybindings); bare User in epics:2084, keybindings. No render-trigger precedent.

tasks (rules, no surface):
1 mech: 999 MoveTaskForward/1017 MoveTaskBackward emit rule names -> MoveTask(task, forward|backward); 1002 ConfirmDone(set) -> for loop.
2 mech: 1290 BookmarkSet(viewer:) vs SetBookmark(1307) binds subscriber.
3 mech: non-built-ins 1125, 1210, 1255, 1291, 1321, 1324, 1348, 1906.
4 mech: no-op ensures 600-601, 1726-1729, 1856-1858.
5 mech: 1101 @guarantee after @guidance in ReorderItem.
6 mech: Open Questions 2290-2306 only resolved -> delete.
7 state triggers: 1342 TaskFinishes -> status becomes done; 1249 TaskLeavesItsEpicOrFinishes split.
8 prose invariants 2215-2284; SubStatusConsistency+Autoreset merge into expression; DispatchMcpToolsAreTransparent(2241) -> agent-health.
9 TaskTeardown 1365-1690 (~330 lines, 36 src/ cites, 17 #NNNN) -> contract WorktreeTeardown with @invariants; CreateTask guidance 59-199, SeeTaskByLandingOnIt 1244 implementation detail.
10 QUESTION shared worktrees: ClearBuildOutputOfDoneTask(711,751-758) guard vs WorktreeIsNeverShared (1614-1635); code src/tui/update/agent.rs::tick_build_output_cleanup has guard.
11 QUESTION SetModel(203)/SetWrapUpMode(220) unemitted; EditTask (2154-2155) writes directly. Emit or delete?
12 entry: surface Board facing operator providing CreateTask, CopyTask, EditTask, QuickDispatch, TrustAndQuickDispatch, MoveTask, BatchMoveTasks, ReorderItem, MoveTaskToEpic, SelectAllColumn, ClearSelection, ToggleSectionCollapse, ToggleEpicFold, ToggleBookmarkKeyPressed, CursorRestsOnTaskCard, DeleteTask, BatchDelete; DeleteKeyRouting (1828-1843) -> when guards.
13 drift CopyTask guidance 2044-2049 default_branch fallback vs CreateTask 148-157 + src/tui/update/forms.rs::default_branch_detected.

epics:
1 mech 969 SessionClosed -> use pr-workflow, pr/SessionClosed.
2 mech: 166, 891 .exists; 897 Epic.ancestor_chain(..).contains; 1720 .is_empty/.contains; 113 bare local_host(); 243 current_window nonexistent; 903 parent_epic_id -> parent.
3 entry MCP surface EpicMcpTools facing mt/McpClient: McpCreateEpic, McpListEpics, McpUpdateEpic, McpManagedFeedConfig; GetEpicViaMcp needs requires epic_id != null.
4 entry board surface: EditEpic, CreateEpic, MoveEpicStatus, ConfirmDeleteEpic(+DeleteEpic emission 291), ReparentEpic, UserTogglesAutoDispatch, UserTogglesGroupByRepo, UserJumpsIntoEpic, UserJumpsToTaskEpic, UserExitsEpicView, UserExitsAllEpicViews.
5 QUESTION ArchivedStatusMigration (1675-1806) never fires since #16755, writes config in ensures. Recommend delete.
6 feed-identity prose invariant 1275-1289 -> expression; SQLite v72 history.
7 history: 16 M# + 16 #NNNN cites (194-207, 1351, 1499, 2182-2188 resolved OQ).
8 facing user: User (2084) undeclared.

work-packages:
1 mech picker surface (479-508) provides ToggleLink/CreatePackage/Close but rules listen LinkItem/UnlinkItem/UserCreatesPackageInPicker.
2 mech unusedBinding user (480) -> visible_to(user).
3 mech link.item nonexistent (184,188,251,286,486) -> derived item.
4 mech 516 `let package = CreateWorkPackage(...)`; CreateWorkPackage (103-114) misses workspace/visibility.
5 entry board surface: UserEntersPackage, UserRenamesPackage, UserConfirmsDeletePackage, ReorderPackage, UserExitsPackageView, UserEntersEpicFromPackage.
6 entry MCP surface WorkPackageMcpTools + missing rules for create_work_package/non-delete update/list (prose 550-594).
7 render triggers MainBoardRendered(326), PackageViewRendered(414) -> derived column + exposes.
8 LinkedItemDeleted(283) -> invariant LinksNeverDangle.
9 QUESTION TaskCreatedFromBoard(task, view)(203) no producer -> package-view surface provides CreateTaskInPackage?
10 section order: invariant PackageCardsOnlyOnMainBoard(374) mid-rules, empty; surface without header.

mcp-task-tools:
- McpContextResources(2249) works; no surface provides tool calls McpToolsList, McpUpdateTasks, McpCreateTask, McpGetTask, McpListTasks, McpQueryUsage, McpDispatchTask, McpResumeTask, McpWrapUp, McpExitSession, McpSetVerifyCommand.
1 entry surface McpTaskTools; move prose 43-125, 190-197, rule-level @guarantees 1605, 1625, 1651, 1682 (after @guidance — forbidden) onto it.
2 TaskRefResolvesToTaskId (732) phantom McpToolCalled -> @guarantee TaskRefAcceptsAgentName.
3 AgentPortRejectsNonLoopbackOrigins(162) AgentPortRequest -> contract AgentPort; 184-188.
4 mech AttachPlanViaCli(1435) -> use cli, cli/CliAttachPlan.
5 mech McpUpdateTask signature differs at 205, 475, 673, 753.
6 history/impl: 23 src/ + 22 M# cites (407-419, 572-585, 698-704, 1414-1420).
7 Scope header 4-6 incomplete; Excludes 13-14 circular with learnings:827.
8 McpClient(2240) dup keybindings McpAgent(697).

learnings:
1 mech unknownName 673 core/Retrievals.any -> exists core/Retrieval{...}; 717 core/Learnings.exists -> bind entity.
2 mech: 409 AgentLaunched -> dispatch/; 733 DeleteEpic -> epics/; 452 WrapUp -> mt/McpWrapUp.
3 mech: invariant ApprovedLearningsHaveScopeRef(362) duplicates core ScopeRefConsistency(1329) -> delete.
4 entry surface LearningMcpTools: McpRecordLearning, McpQueryLearnings, McpRateLearning, McpDeleteLearning; replace "no surfaces" 818-832; move @guarantee 517.
5 DRIFT/possible code bug: 411 modes list has epic_planning (exists nowhere), misses research; src/dispatch/agents.rs::prepare_inputs records retrievals for research agents but research_agent gets no injections; 415-431 "all relevant" vs DISPATCH_INJECTION_CAP=5.
6 drift core:1324 applies_to_dispatch exact match vs src/embeddings.rs::repo_ref_covers parent dirs.
7 duplication: scoring formula 198-233 & 638-657; BREAKING CHANGE note 224-230 & 632-636; prose restating rules 178-258, 322-349; 48 "above"; 37-38, 663, 667 cites; 359, 377 LearningService.
8 ArchiveStaleLearning(777) temporal but requires caller (785); code host tick archive_stale_learnings.
9 by-design fixed scoring constants; state once.
Sections: Invariants (351) before Rules (399).

Cluster decisions: C0 one shared Operator actor (+ one MCP actor) in core; C1 MCP actor first; C2 convention render/cascade triggers -> derived/becomes/invariants.
Questions: shared worktrees; SetModel/SetWrapUpMode; ArchivedStatusMigration; TaskCreatedFromBoard; research-mode injections/cap.

### Appendix G — workspaces, admin-surfaces, board-layout, board-visuals, keybindings

Cross-cutting:
- X1 undeclared parties: `User` declared nowhere (keybindings KeyInput 709, HelpOverlay 810; board-layout TextInputField 1408; all six board-visuals surfaces 1531-1847); McpCaller (keybindings 698), TestSuite (keybindings 899). McpAgent dup McpClient. Checker silent.
- X2 non-built-in collection ops: workspaces 359, 472, 609, 613, 742, 882; admin 95-96, 132-146, 351; board-layout 263, 280, 307-309, 622, 625; keybindings 476. -> `X where pred` + black boxes. 3.6.1 silent.
- X3 qualified collections: core/PollOwners unknownName; checker doesn't validate unqualified collections at all (Bogusthings silent). Unqualified matches file convention (Tasks, Epics...). Same in sync 1573, feeds 1206. Settle repo-wide.
- X4 key vocabulary: keybindings KeypressRunsItsRowsAction emits KeyActionRun nobody consumes; features invent triggers. Recommend each feature spec declares small TUI surface providing its key triggers (dispatch.allium pickers precedent).
- X5 MCP tool triggers: each spec declares MCP tool surface facing mcp/McpClient.

workspaces (mature):
- mech verified: unusedBinding WorkspaceAdministration(1231) use admin in guard; SuperAdministration(1251) `facing _: SuperAdmin`; 1078 `let default` keyword; 849-850 McpUpdateEpic twice; open question 1126 under Invariants -> 1282.
- PollOwners 472, 609, 613, 742.
- DRIFT RemoveMember (472-475), MoveEpicToWorkspace (742-745) release only epic-scope claims; code workspace.rs::drop_follows_in_workspace releases both (release_poll_claims + release_task_poll_claims).
- entry: (a) contract with sync/SharedStore: ClientConnected(225), StoreUpgradedToWorkspaces(1073); (b) StoreWrites surface or producers emit hooks: EpicVisibilityRequested, EpicOriginPatched, EpicMoved, TaskMoved, TaskPlacedInEpic, LinkRequested (vs work-packages LinkItem), WriteToItem (488..953); outputs EpicVisibilityAllowed/MoveAllowed/WriteAllowed unconsumed; (c) SightChanged(587) emitted by RemoveMember, MoveEpicToWorkspace, SetEpicVisibility; (d) TUI SetOwnNameKeyPressed(261), EpicOrPackageFormOpened(880); (e) MCP SetDisplayNameToolCalled(276); (f) LearningRecorded(1032) -> core/Learning.created.
- drift NameWorkspaceAndVisibilityViaMcp(846-877) union rule w/ different signatures than epics 509/640, work-packages 227/103; ensures fire all arms. Add params to owners; delete union rule.
- ShownName(288-306) rule-as-function -> derived/black box.
- 1203 long line; 886 form/ChoiceRequired unbound, mine.first on Set; TaskMovesStayInTheWorkspace(780) no ensures.
- header 82-183 implementation detail -> docs/architecture.md.
- QUESTION 1285 unseen listing vs nearest holder (visibility.rs::unseen_items_of vs epic_is_unseen): show every private epic whose own holder left? Open questions 1126, 1287, 1289 stand.

admin-surfaces (unwired):
- mech verified: AdminScreen.provides (483-492) lists RULE names -> triggers; add OpenAdminScreenKeyPressed, AdminPaneKeyPressed; clears 169..315.
- mech 82 workspaces/core/Workspace transitive -> use core.
- mech unbound: UpdateWorkspaceTool workspace (385-386), DeleteUnseenItemTool item (460), ListWorkspacesTool reader (350).
- config 64 identity_prefix_min_length dup workspaces identity_prefix_length.
- PersonResolution(91) rule-as-function -> black box resolve_person; UnseenItemListingContents(127) -> surface let.
- DirectoryEntry(72) used as collection DirectoryEntries -> UserProfiles.
- entry: AdminTools MCP surface for 8 triggers (347..458); AnAdminWriteFinished(331) contract with SharedStore.
- listing filter depends on workspaces 1285 question.

board-layout (mixed):
- RECOMMEND restructure four values (EpicCardPlacement 254, FlattenedView 558, EpicGroupTree 615, ColumnSectionLayout 654): consumed only in prose (tasks 889, 942, 1041; work-packages 415-438; keybindings 279; epics 214, 1866, 2155; agent-tree 2436, 3141); `view` entity doesn't exist; read `given board_hides` in parameterised derivations (forbidden). -> `given board_view: BoardView` entity with derived task_is_visible etc; delete FlattenedView.active (dup BoardLens.flattened); wire tasks/work-packages/board-visuals consumers. QUESTION (c): restructure vs minimal surface.
- drift BoardHides.hidden_count(1178) vs prose 1143-1150 and src/tui/mod.rs::live_hidden_counts.
- entry: CycleLensForward/Backward (1084,1093), ToggleHiddenEpic/WorkPackage (1182,1206) TUI surfaces.
- parent_chain (618) only in comment.
- stale: 1234 A-Z ignored but Z bound (1273, filter_fold_all_epics); 644 "Flattening" section missing.
- hardcoded 30 chars (637) -> config; gg_chord_timeout (152,160) -> core/config.
- prose-only behaviour: gg chord 147-170, own_match 1335-1345, selection preservation 1389-1401, column nav 109-145.
- section order: config mid-file (602).

board-visuals (mature core, no diagnostics):
- drift HuedSurface(1430)/PaletteToken(1156) closure false: signal (672,689), bookmark (714), cyan (1858), flash (622,1762), state red/yellow missing.
- stale test paths 802, 815 (rendering.rs -> rendering/palette_and_frames.rs), 805, 987 (cards.rs -> cards/tests.rs); 1051 ── prefix vs #{id}; "breadcrumb row" stale (1438,860,1806); 993 "three amber" (2); 1005 four->six; 1031 three->four.
- mech WorkPackageCard(1849) context WorkPackage unimported.
- 799-1152 test inventory essay -> docs/testing.md (check-doc-symbols.sh coupling); hex values 379/1309/334.
- section order RenderedCard(1330) before enums/values.

keybindings (mature):
- mech verified KeyReceiver(217): type receiver derivation (442).
- seam records_usage(440): KeyActionRun(487) unconsumed; observability:299 listens KeypressHandled. -> KeyActionRun carries records_usage; observability listens keybindings/KeyActionRun.
- 614 "fourteen" -> fifteen/"every".
- X1, X2 (476).

WPs: shared party decls; mechanical; AdminScreen wiring; rules-as-functions; TUI key surfaces; MCP surfaces; store lifecycle contract + StoreWrites + SightChanged + task-scope release; PollOwners convention; BoardView restructure; collection syntax sweep; records_usage seam; stale prose sweep; prose->construct promotions (optional).
Questions: workspaces 1285; X4/X5 convention; BoardView restructure.
