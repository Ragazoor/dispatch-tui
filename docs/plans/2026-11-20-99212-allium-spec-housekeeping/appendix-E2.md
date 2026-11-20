# Appendix E2 — dispatch, dispatch-prompt

Condensed report of one read-only review agent for task #99212, at commit b03f5ba0 (main, 2026-11-18). Part of docs/plans/2026-11-20-99212-allium-spec-housekeeping.md. Line numbers drift as packages land: search for the cited symbol, not the line. Acceptance checks run from the worktree root.

# E2 — dispatch.allium, dispatch-prompt.allium (review #99212, HEAD b03f5ba0)

Read in full: dispatch.allium 1-2459, dispatch-prompt.allium 1-1595, both diffs since 10b68974 (the dispatch one checked for dropped content: all 56 removed lines are reworded or moved; one semantic change, teardown's prune and branch delete are now said to be bounded, which matches src/git.rs). Language-reference sections read: Contracts, Postconditions, Black box functions, Validation rules, Anti-patterns. patterns.md: only grepped, for the removal and guarded-create forms. Slices: dispatch 0 diagnostics; dispatch-prompt 2 baselined (Bump, Preceding). analyse: 0 findings, but see E2-D-23 (masked).

## Carried from Appendix E (all resolved)
- D1 resolved: no bare `local_host()`/`this_host` left (grep empty).
- D2/Q-D1 resolved: two contracts, DispatchRequestServing (39) and WorktreeProvisioner (246), are demanded by the DispatchProvisioning surface (1795). The residue from that split is E2-D-1..8.
- D3 resolved: the Invariants section (1286) now comes before Surfaces (1495).
- D4 resolved: the TmuxWindow passage (1431-1441) is in the present tense.
- D5 resolved: DefaultBaseBranchIsDetectedNotAssumed (1452-1480) is in the present tense.
- D6 resolved: "(10s)" is gone (962).
- P1 resolved: lines 88-89 and 382-385 are rewritten. P2 resolved: lines 1408-1426 are rewritten.
- P3 resolved: line 1409 says "sizing is the review command's", and line 510/519 cites `settings/review_command()` rather than the deleted ReviewCommandIsASetting.

## dispatch.allium
| code | class | line | problem | remedy |
|---|---|---|---|---|
| Q-E2-D3 | QUESTION | 41,57,…,628; 895-896 | Each @invariant opens with a `== heading ==` that differs from its name. Lines 895-896 make the headings the citation anchor on purpose, and scripts/check-doc-headings.sh gates quoted citations of them: src/dispatch/worktree.rs:850,1171 (1171 also calls the text "guidance"), src/dispatch/tests/agent_launch.rs:857, src/runtime/tasks.rs:42, src/service/tasks/tests/dispatch_requests.rs:587,642, split-pane.allium:1076, dispatch-prompt.allium:15-16, dispatch.allium:2170 | (a) Keep the headings as the gated anchor. **Recommended**: cheapest, and checked today. Only fix worktree.rs:1171's "guidance" to "invariant". (b) Drop the headings and cite invariant names everywhere. No gate checks those names in code comments |
| E2-D-1 | mech | 284, 371, 397, 518 | The body of the invariant restates its own name ("WorktreeExistenceIsOneQuestion." and others) right after the heading | Drop the restated names |
| E2-D-2 | mech | 274-280 | The fetch paragraph sits under RepoTrustPreGate | Move it to FreshVsReusedDecidesTheFetch |
| E2-D-3 | mech | 328-332 | The info/exclude and `-b`/`-B` paragraph sits under WorktreeExistenceIsOneQuestion, and line 515 cites it as "the fresh-add paragraph above" | Move it to StartPointSelection |
| E2-D-4 | mech | 587-623 | The PR-head carve-out for review worktrees sits under UnresolvableBaseIsRefusedByName | Give it an @invariant of its own (e.g. ReviewWorktreesStartFromThePrHead) |
| E2-D-5 | mech | 624-625, 644-645 | Launch-flag statements (plugin dir, no --permission-mode) sit in WorktreeProvisioner; AgentLaunchCommand already covers them (1525, 1603) | Drop them |
| E2-D-6 | mech | 1734-1743 | "The child environment (non_interactive_env)" sits inside AgentsShareOneMemoryCap | Make it a separate @guarantee |
| E2-D-7 | mech | 155, 971, 1312 | Stale pointers from the split: "bound in ProvisionDispatch", "refused-report path in ProvisionDispatch", "see DispatchTask for what each entry point reports" | Repoint to WorktreeProvisioner, ARefusedReportTearsDown and ClaimIsTheRequest |
| E2-D-8 | mech | 881-889 vs 898-904 | The "the prompt is in dispatch-prompt" text appears twice | Keep one copy |
| E2-D-9 | mech | 885 | "the dep-only path allowlist that runbook checks against" contradicts dispatch-prompt 1327 (no allowlist; the policy is stated in prose) | Say "the dep-only policy" |
| E2-D-10 | mech | 1974 | The prefill falls back to "main" / core/config.default_branch, which contradicts PrefillFromHistory (2035-2038) and DefaultBaseBranchIsDetectedNotAssumed | Say "opens on the draft value, then the detected default" |
| E2-D-11 | mech | 1931-1934 | RepoPathPicker's guidance says it renders for InputBaseBranch, but BaseBranchPicker is the surface for that step | Drop the clause |
| E2-D-12 | mech | 1641 | "(bubblewrap, socat)" are named as optional tools, but src has no reference to either (the sandbox is off) | Drop the parenthesis |
| E2-D-13 | mech | 2336, 2390 | Stray "-- prose:" markers | Delete them |
| E2-D-14 | mech | 1227 | "The re-dispatch stamps it again (DispatchTask)" is wrong: the module stamps host | Cite store-split HostReportsDispatched |
| E2-D-15 | bp | 40-54 | SubagentStateIsReset restates the module's reset, which store-split RequestDispatch (820-825) already specifies formally | Shorten it to a one-line pointer. Move the stop_pending rationale to store-split if it is missing there (coordinate with the cluster that owns store-split) |
| E2-D-16 | bp | 236-239, 265-266, 505-506, 546-553, 871, 1116-1119, 1616-1620, 1676-1680, 1705-1707, 2069-2070, 2159-2173, 2327-2328, 2383-2385 | History: "now surfaces", "always did", "today's behaviour", a past incident, "where it always has", the replaced DispatchingTimeout watchdog ("REPLACES… REVERSES"), "no longer strands", "was rejected", "froze the machine" | Rewrite in the present tense, keeping one rationale sentence each |
| E2-D-17 | bp | 2451-2452 | A dated plan path with a task number (docs/plans/2026-08-25-4373-…) | Drop it; the rationale is already stated in place |
| E2-D-18 | bp | 236, 599, 490, 1727-1732, 2131 | Literals: "(120s)" ×2, "(currently 3)", "30 seconds", "5 seconds", "5-second STATUS_MESSAGE_TTL" | Cite core/config.status_message_expiry, which exists. Add config entries: subprocess_timeout (core, since store-split:721 also cites it), fetch_max_attempts, prepare_scope_worst_case, sccache_start_timeout (values from src/process.rs, src/dispatch/worktree.rs, src/process/agent_scope.rs) |
| E2-D-19 | bp | 934-938, 1010 | Implementation and test citations in guidance (dispatch_with_prompt, the test name, build_research_prompt) | Keep one path::symbol (src/service/tasks/dispatch.rs::TaskService::serve_dispatch_request) and drop the rest |
| E2-D-20 | bp | 685, 706 | `core/SavedRepoPath.created_or_updated(…)` and `core/SavedRepoBranch.created_or_updated(…)` are not valid: creation is `.created()` only. The 98983 plan fixed the same form in settings; mcp-task-tools:1798 has it too (cluster F) | Write a guarded create plus update: a `let` join lookup, then if-exists/else `.created()` (patterns.md `requires: not exists X{…}` form) |
| E2-D-21 | bp | 1179 | `OldTmuxWindow.killed(window: …)`: the entity is undeclared and `.killed` is not a creation verb | Use the removal form, `not exists current_agent(task).tmux_window` (patterns.md), or a named emission |
| E2-D-22 | bp | 744, 979, 1019, 1167, 1210 | Five rules share their trigger's name: DispatchTask, DispatchResearchTask, ResumeTask, RetryResume, RetryFresh | Rename the triggers (…Requested/…Pressed) and keep the rule names, which code cites. Emitters and consumers to update: split-pane 990/1008/1118, mcp-task-tools 1490, AgentRelaunch provides (1830-1834) |
| E2-D-23 | entry | 745, 980 | No surface provides DispatchTask or DispatchResearchTask (the dispatch key on a backlog card, the trust prompt's accept). Quick dispatch is not a source: it requests directly with mode quick. Both triggers count as reachable only through RetryFresh's own emission (1242-1244) and split-pane:990's unqualified emission (name collision). Scratch-verified: with both removed, analyse reports two unreachable triggers. With them added to AgentRelaunch's provides, check and analyse are clean again | Add them to a dispatch TUI surface (Decision 1; extend AgentRelaunch). Qualify split-pane:990 as `dispatch/…` |
| E2-D-24 | drift | 1240-1244 vs 1252-1254 | RetryFresh chains into DispatchTask/DispatchResearchTask, which request with mode dispatch/research. The guidance and the code send no mode: src/tui/update/retry.rs:149 sends `DispatchAgent { id, mode: None }` to src/runtime/tasks.rs::TuiRuntime::exec_dispatch_agent, and the module derives the mode (store-split D3) | Change the ensures to `split/DispatchRequested(task: task, requester: core/local_host(), origin: user)` with no mode. Do it together with D-23 |
| Q-E2-D1 | QUESTION | 433-442 | "Known gap… Bounding that remove is tracked separately" (the rollback path calls teardown's unbounded `git worktree remove --force` synchronously inside the dispatch worker). src/git.rs::git_in_unbounded calls the remove deliberately unbounded | (a) Accept and state it in the present tense: a hung remove parks the worker, and the module backstop fails the request and releases the claim. **Recommended**, because it matches the code's reasoning. (b) Bound the remove on the rollback path only (code and spec change) |
| Q-E2-D2 | QUESTION | 1757-1766 | DevBuildsKeepLineTablesNotFullDebugInfo is a guarantee about this repo's Cargo.toml dev profile, with dated measurements, attached to AgentLaunchCommand. It is not launch behaviour | (a) Move it to docs/testing.md and drop it from the spec. **Recommended.** (b) Keep it, with one present-tense rationale sentence |
| — | by-design | 1290-1480 | The comment-only invariants say they cannot be expressed as state | Leave |
| — | by-design | 2459 | The open question is still open: the module's request_dispatch has no repo_path check | Leave |
| — | by-design | 338, 405, 1418-1441 | Version-probe notes and the labelled mechanism note are evidence about external tool behaviour; the cited tmux:: helpers exist | Leave |
| — | by-design | 39, 246 | Contracts that hold only @invariants are allowed (the body may be "signatures and annotations"), and a surface demands them | Leave |

## dispatch-prompt.allium
| code | class | line | problem | remedy |
|---|---|---|---|---|
| E2-P-1 | by-design | 15-16 | Cites dispatch's quoted headings, which check-doc-headings.sh gates | Leave under Q-E2-D3 (a); repoint under (b) |
| E2-P-2 | mech | 332 | "two of its four answers", but Preceding has six literals | Say "six" |
| E2-P-3 | mech | 766-767 | "DispatchTask refuses to launch at all without one" | Say ProvisioningNeedsARepoPath refuses it after the claim |
| E2-P-4 | mech | 1194-1195 | "Sharing one body meant telling every unclassified PR it was a group" is past tense, and stale now that the group has its own merging branch (1210-1228) | Drop it |
| E2-P-5 | mech | 1335-1340 | The build-file paragraph splits the "four pieces of mechanics" bullet list | Indent it into bullet 1 or move it after the list |
| E2-P-6 | mech | 251 | "DispatchTask emits one of three variants of build_prompt" | Say the AgentLaunched event ProvisionDispatch raises |
| E2-P-7 | mech | 403 | "DispatchMode routes ANY planned task…" uses a code enum name | Cite the module's mode derivation (store-split D3) |
| E2-P-8 | bp | 720-721, 863-867, 885-888, 1052-1055, 1099, 1237-1238 | History: "the always-loaded copy said", "Gating this line… left", "It was a five-step list", "before this kind existed", "it always was", "keeps the old instruction" | Rewrite in the present tense |
| E2-P-9 | bp | 1133-1150 | Dated sample evidence with a task number ("15 open bot PRs sampled from epic 275") plus measurement narrative | Replace with one present-tense rationale sentence |
| E2-P-10 | bp | 70 | No surface demands contract PromptComposer (dispatch's two contracts are demanded by DispatchProvisioning) | Mirror DispatchProvisioning: add `surface PromptComposition { facing _: core/BoardProcess; contracts: demands PromptComposer }`. Not on AgentPrompt, whose facing party is the agent. Scratch-verified clean |
| — | by-design | 40, 170 | Bump and Preceding are commented and baselined (#106) | Leave |
| — | by-design | 246, 335, 421, 470, 507, 543 | The "observable output; no rule listens" events (N1) | Leave |
| — | by-design | 703-706, 1502, 1579 | The cited tests and prompt fragments exist | Leave |

Counts: mech 20 (D 14, P 6), bp 11 (D 8, P 3), entry 1, drift 1, QUESTION 3. By-design: 8 rows (P-1 included).

## Work packages
**E2-A: dispatch.allium contract residue and text pass**
- Files: dispatch.allium; core.allium (config for D-18); src/dispatch/worktree.rs:1171 (only if Q-E2-D3 is (a)). Under (b), add the heading citations listed in Q-E2-D3.
- Findings: D-1..19, P-1 (under (b) only); Q-E2-D1, Q-E2-D2, Q-E2-D3.
- Ordering: run first.
- Acceptance (fails today):
```
! grep -nE 'Itself\.$|tracked separately|\(120s\)|currently 3|REPLACES the former|longer strands its task|bubblewrap|^ *-- prose:|2026-08-25-4373|30 seconds|by 5 seconds|5-second|"main" / core|refused-report path in ProvisionDispatch|bound in ProvisionDispatch|see DispatchTask for what|dep-only path allowlist|-- (WorktreeExistenceIsOneQuestion|ReusedBranchWithNoOwnWorkIsFastForwarded|StaleAdminRecordIsPrunedBeforeWorktreeAdd)\.|== UnresolvableBaseIsRefusedByName ==|== The prompt itself ==' docs/specs/dispatch.allium
```

**E2-B: dispatch.allium formal fixes (triggers, entry, non-built-ins, RetryFresh mode)**
- Files: dispatch.allium; split-pane.allium (990, 1008, 1118); mcp-task-tools.allium (1490). A whole-set grep confirms these are the only emitters and consumers outside dispatch.allium.
- Findings: D-20..24.
- Ordering: after E2-A, since both edit dispatch.allium. Do D-23 and D-24 together, because fixing D-24 alone leaves DispatchTask unreachable. Rename only the triggers; rule names stay, because src cites them. After the change, `allium analyse` over the whole set must show 0 findings for dispatch.allium.
- Acceptance (fails today with 10 matches):
```
! grep -nE '^\s*when: (DispatchTask|DispatchResearchTask|ResumeTask|RetryResume|RetryFresh)\(|created_or_updated|OldTmuxWindow|DispatchTask\(task: task\)|DispatchResearchTask\(task: task\)' docs/specs/dispatch.allium
```

**E2-C: dispatch-prompt.allium text pass and contract demand**
- Files: dispatch-prompt.allium.
- Findings: P-2..10.
- Ordering: independent; it does not touch lines 15-16 (E2-A owns them).
- Acceptance (fails today):
```
grep -q 'demands PromptComposer' docs/specs/dispatch-prompt.allium && ! grep -nE 'epic 275|Sharing one body|four answers|refuses to launch at all|It was a$|always-loaded copy said|before this kind existed|old instruction|Gating this line on|DispatchMode routes' docs/specs/dispatch-prompt.allium
```

Every package footer: `./scripts/check-allium.sh` must show no new warnings, then run allium:weed on the package's specs.
