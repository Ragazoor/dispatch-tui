# Appendix B1 — agent-tree, agents

Condensed report of one read-only review agent for task #99212, at commit b03f5ba0 (main, 2026-11-18). Part of docs/plans/2026-11-20-99212-allium-spec-housekeeping.md. Line numbers drift as packages land: search for the cited symbol, not the line. Acceptance checks run from the worktree root.

# B1 — agent-tree.allium, agents.allium (review at b03f5ba0)

Coverage: agents.allium 1-570 read in full; agent-tree.allium 1-3995 read in full (incl. 1420-1590, 2052-2160 and all surfaces). Full diff 10b68974..HEAD read for agents; for agent-tree, every hunk and every removed line read (added text read as part of the full read). Whole-set `allium check` (3.6.1): agents 0 diagnostics; agent-tree 4 baselined definition.unused. `./scripts/check-allium.sh` exits 0 today. Code spot-checks (no drift found): every `path::symbol` and bare symbol either spec cites exists in src/ or spacetime/module/src/; `src/agent_tree/state.rs::RESUME_JUMP_MAX_READS` = 30 = config; commits/agents titles, "N agents hidden by board filter", "waiting for the task", "did not start after a resume" strings match; MCP names in agents 547-550 exist; `src/hooks/wire.rs::PaneAgent`.agent_name is the current agent's name, and every listed task has one, so it equals display_name.

## Named items: FileDiff (133), AgentTreeNode (161)
Not dead. The invariants read their fields through the derived `rendered_diffs` / `rendered_nodes` (DiffBodyExcludesRefusal; OnlyFilesOpenDiffs … OnlyFilesCarryBadges), and the key triggers pass a node. A real checker gap: a derived value cannot carry a type, and a trigger parameter cannot be typed. Each has its comment beside the declaration (130-132, 159-160), and the baseline header describes the gap. No upstream issue number is recorded for this case (only #106/#118 are listed).
A cleaner form exists, verified alone on a scratch copy: `contract AgentTreeRendering { rendered_tree: (root: String, changes: List<GitFileChange>, open: List<String>) -> List<AgentTreeNode>; rendered_file_diffs: (root: String, commit: String?, open: List<String>) -> List<FileDiff> }` clears both warnings with no new diagnostic. Caveats: nothing demands or fulfils it unless a surface is made to, and the checker does not tie the operations to the black-box calls. See Q-B1-1.

## Carried items (Appendix B, previous review)
agents: A-n1 resolved (74-75) · A-n2 resolved (42-48) · A-n3 resolved (517) · A-n4 by-design, still (537-538) · A-n5 resolved (A10 dropped; no spec cites it) · A-n6 resolved · A-n7 resolved (core Task.default_dispatch_mode, read by agents 169 and store-split 814) · A-n8 resolved (Invariants 386 before Surfaces 472) · A-n9 resolved (mcp-task-tools 2467 cites A6).
agent-tree: T-n1 resolved (242-247) · T-n2 resolved by deletion (DiscardAgentTreePaneOnWindowGone gone; DetectCrashedAgent clears the window, agent-health 158-160, so DiscardAgentTreePaneOnWindowCleared 1165 covers crashes) · T-n3 resolved (1064) · T-n4 resolved (1661) · T-n5 resolved · T-n6/7/8 resolved (no task numbers left) · T-n9 resolved (render events dropped) · T-n10 resolved (1969-1972; EditorPaneOutlivesItsTree is now guidance) · T-n11 resolved (2399-2401) · T-n12 resolved (1899-1901) · T-n13 resolved (every key trigger ends …KeyPressed or differs from its rule) · T-n14 resolved · T-n15 by-design, still (231-236, 1157, 3339-3345) · T-n16 by-design, still (697-699; juxt/allium-tools#104).
Still open: none (only the three by-design items).

## Findings — agents.allium
| code | class | line | problem | remedy |
|---|---|---|---|---|
| B1-A-1 | bp | 363, 381, 383 | LaunchedWithSessionName, ResolvedTo, NotFound are emitted, no rule listens, no N1 comment | add "observable output; no rule listens" beside each (optionally rename the generic ResolvedTo/NotFound to TaskRefResolved/TaskRefNotFound) |
| B1-A-2 | mech | 183 | `waiting.tmux_window = null` restates AssignedAgentHasNoSession (an assigned agent has no window) | delete the line |
| B1-A-3 | by-design | 299 | `where agent = agent` (a field and a binding share a name): the idiom is used repo-wide (agent-health 241/615/635/740, store-split 823 and others) | leave |
| B1-A-4 | mech | 550 | "epic_search'" (bad possessive on a tool name) | "epic_search's" or "in epic_search, `(parent:<id>)`" |
| B1-A-5 | mech | 237, 368-371, 500 | an overlong line (237, 500) and a broken wrap ("A\n-- name") | reflow |
| B1-A-6 | mech | 129-130, 195-196, 333-334 | `live_subagents: 0, stop_pending: false` restate core Agent defaults (core 853-854); BackfillCurrentAgents' current create (319-326) omits them | drop them everywhere, or pass them everywhere |
| B1-A-7 | by-design | 63 (A9→A11) | no A10: the id is retired, and the others stay stable because other specs cite them | leave |
| B1-A-8 | by-design | 46-48 | name-ledger table note: present tense, about a table the store still holds | leave |
| B1-A-9 | bp (cross-cluster: for store-split's reviewer) | store-split 819, 825-826 vs agents 171-197 | Two specs write the same agent facts on the same claim (last_pre_tool_use_at, live_subagents, stop_pending). Which agent store-split's `current_agent(task)` names in an ensures (pre-claim or new) is unclear: its guidance at 846-850 says the claim starts the agent in the same transaction | one owner: the store-split reviewer settles it (agents 149-153 already claims it) |

## Findings — agent-tree.allium
| code | class | line | problem | remedy |
|---|---|---|---|---|
| B1-T-1 | bp | 1348, 1632, 2039, 2209, 2285, 2370, 2497 (stamps 893-908; comment 1350-1352) | A temporal trigger fires once, when its condition becomes true. Every refresh stamp starts due (`now - interval`) while `task_resolution = waiting` (or `diff_pane = null`) fails the requires, and nothing re-arms it: the resolve rules (1217, 1295) and the splits (1782, 1842) do not restamp. Read strictly, no refresh ever fires. 1350-1352 assumes a trigger that "stays satisfied" | Move the guards into derived due conditions, e.g. `tree_refresh_due: status = shown and task_resolution = resolved and last_rendered_at + config.agent_tree_refresh_interval <= now` and `when: pane: AgentTreePane.tree_refresh_due`. One each for tree, diff, commits and agents; the ok/failed rule pairs share the trigger. Verified on scratch: parses, no new diagnostic. Rewrite 1350-1352. A clock tick from core/BoardClock also checks clean but contradicts 727-728 (the pane's timer is independent of the board) |
| B1-T-2 | bp | 830, 866, 2395, 2634, 2677 | AgentTreeToggleBound/Unbound, AgentWindowSelected, AgentResumeRequested: emitted, no listener, no N1 comment | add the one-line comment (for AgentResumeRequested, name BoardPaneView.resume_agent as what it calls) |
| B1-T-3 | mech | 3476-3478 | cites "AgentTreeDiffPane's Order section" and says the diff pane "is GIVEN" the order. No such section exists, and RefreshAgentTreeDiff's Order (2082-2085) says there is nothing to order | replace with "Nothing outside the tree uses this order: the diff pane shows one file" |
| B1-T-4 | mech | 847-848 | "See ToggleAgentTreePane's own guidance": that is a trigger, not a rule | "See HideAgentTreePane's guidance" |
| B1-T-5 | mech | 3268-3270 | "not the task's alias": tasks have no alias (agents A6) | drop the clause |
| B1-T-6 | mech | 535, 1333, 1516, 3990 | "today" / "now READS" (dated wording) | present tense ("Always true: …", "everything that closes it", "reads") |
| B1-T-7 | mech | 232, 3343 | "as it did before" / "as before" | drop; keep the compatibility sentence |
| B1-T-8 | mech | 3882 | "Space, Enter and `a`": the `a` (all-files) key no longer exists anywhere | "Space and Enter among them" |
| B1-T-9 | mech | 650-668 | the task_arrived/diff_task_arrived comment (650-658) is cut off from its declarations by the shown_agents block (659-665) | move shown_agents above the comment |
| B1-T-10 | mech | 1223, 1300 | the requires repeats `worktree != null`, which task_arrived / diff_task_arrived already hold | delete |
| B1-T-11 | bp | 835, 952, 1068, 1075-1076, 1083, 1148 (also 1073) | bare symbols, or "fn (path)", instead of `path::symbol`: bind_key, split_window_horizontal, exec_swap_split_pane, toggle_agent_tree_pane, cmd_toggle_agent_tree_pane, spawn_agent_tree_pane, TmuxWindow::for_task | `src/tmux.rs::bind_key`, `src/tmux.rs::split_window_horizontal`, `src/runtime/split.rs::exec_swap_split_pane`, `src/dispatch/agents.rs::toggle_agent_tree_pane`, `src/main.rs::cmd_toggle_agent_tree_pane`, `src/dispatch/agents.rs::spawn_agent_tree_pane`, `src/models/tmux_window.rs::for_task` (all exist) |
| B1-T-12 | bp | 2582-2584 vs 665 | CycleAgentListFilter re-derives the shown_agents formula | parameterised derived `shown_under(f)` on AgentTreePane; `shown_agents: shown_under(agent_filter)`; the rule uses `pane.shown_under(next)` |
| B1-T-21 | bp (optional; core edit) | 665, 2382, 2394, 2413-2419, 2628 | is_live_agent is a black box restating core facts (Running/Review and a recorded window), while its twin is_detached is derived in core | derive `Task.is_live_agent` in core beside is_detached (coordinate with core's package), or leave it |
| B1-T-13 | bp | 3951-3966 (and 1888) | AgentTreeTmuxOutcomes faces core/Tmux but also carries non-tmux outcomes: `v` on a missing file, and a failed write to the git admin dir | rename it to a renderer-outcome surface whose prose lists both sources, or add a rule `RefuseEditorOnMissingFile` (when OpenSelectedAgentTreeFileInEditor and `not file_exists(node)`, emit AgentTreeDiffPaneOpenFailed) |
| B1-T-14 | mech | 805, 1459, 1503, 1663, 2703, 3134, 3166-3168, 3174, 3733, 3909, 3945 | double blank line, overlong lines, an awkward parenthesis at 3909, a blank line before `}` | reflow |
| B1-T-15 | by-design | 130-132, 159-160, 242-247, 223-225 | the four definition.unused gap comments | leave (or Q-B1-1) |
| B1-T-16 | by-design | 535-539, 1792, 1902, 1949 | diff_pane_is_ours is always true: kept so a reader sees the ownership rule | leave (wording in B1-T-6) |
| B1-T-17 | by-design | 541-545 | lower_pane_has_focus is never cleared by a rule: tmux navigation clears it | leave |
| B1-T-18 | by-design | 983, 1123, 1136, 1800 | null assigned to when-qualified fields as a state leaves: reads as clearing; the checker accepts it | leave |
| B1-T-19 | by-design | 1615-1621 | test advice ("verify by RENDERING"), but inside @guidance, which is for non-normative advice | leave |
| B1-T-20 | by-design | 1605-1613, 2123-2154 | performance short-circuits, kept for their one behavioural consequence (expansion and scroll survive) | leave |

## Questions
- **Q-B1-1 (FileDiff/AgentTreeNode; structure, not behaviour).** (a) Keep both baselined with their comments, which already complies with Decision 5. (b) Add `contract AgentTreeRendering` typing rendered_tree / rendered_file_diffs, with AgentTreeCompanionPane and AgentTreeDiffPane each `fulfils AgentTreeRendering` (tested in exactly this form on a fresh scratch copy: both warnings clear, no new diagnostic). The Codec precedent: pure-function contracts are idiomatic). Drop the two baseline lines, the comments at 130-132 and 159-160, and the baseline header's derived-value sentence. **Recommend (b)**: Decision 5 prefers a restructure where a clean form exists, and the derived values really call these functions. The cost: the checker does not link the operations to the calls, so part of the gain is cosmetic. If (a), file the derived-value gap upstream, since the header records no issue for it.

## Work packages
1. **B1-P1 agents: N1 comments and tidy.** Files: docs/specs/agents.allium. Findings: B1-A-1, A-2, A-4, A-5, A-6. No question. No ordering constraint. (B1-A-9 belongs in store-split's package, not here.)
   Acceptance (exit 1 today):
   `cd <wt> && test "$(grep -c 'observable output; no rule listens' docs/specs/agents.allium)" -ge 3 && ! grep -qE "waiting\.tmux_window = null|epic_search'" docs/specs/agents.allium`
2. **B1-P2 agent-tree: present tense, stale references, N1, citations.** Files: docs/specs/agent-tree.allium. Findings: B1-T-2…T-14 (T-21 optional). No question. Each alternative of the grep below hits exactly once today.
   Acceptance (exit 1 today):
   `cd <wt> && test "$(grep -c 'observable output; no rule listens' docs/specs/agent-tree.allium)" -ge 4 && ! grep -qE "Order section|Always true today|closes it today|as before|as it did before|now READS|ToggleAgentTreePane's own|row, not the task's|Enter and .a. among" docs/specs/agent-tree.allium && grep -q 'src/dispatch/agents.rs::toggle_agent_tree_pane' docs/specs/agent-tree.allium`
3. **B1-P3 agent-tree: refresh timers that re-arm, and the typing gap.** Files: docs/specs/agent-tree.allium, scripts/allium-check.baseline (only if Q-B1-1 = b). Findings: B1-T-1, plus Q-B1-1. Run after B1-P2 (same file), or fold into it. Run allium:weed on the timer rules against src/agent_tree/ (the renderer polls; check the first query still runs at once on resolve and on show).
   Acceptance (exit 1 today):
   `cd <wt> && ! grep -qE "when: pane: AgentTreePane\.last_[a-z_]+ \+ config" docs/specs/agent-tree.allium && ./scripts/check-allium.sh >/dev/null`
   If (b), also: `! grep -qE "'(FileDiff|AgentTreeNode)'" scripts/allium-check.baseline`

Counts: mech 13 (A-2, A-4, A-5, A-6, T-3…T-10, T-14) · bp 8 (A-1, A-9 cross-cluster, T-1, T-2, T-11, T-12, T-13, T-21 optional) · by-design 9 new (A-3, A-7, A-8, T-15…T-20) plus 3 carried (A-n4, T-n15, T-n16) · drift 0 · entry 0 · QUESTION 1.
