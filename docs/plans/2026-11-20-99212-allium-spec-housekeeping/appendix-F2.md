# Appendix F2 — mcp-task-tools, work-packages, learnings

Condensed report of one read-only review agent for task #99212, at commit b03f5ba0 (main, 2026-11-18). Part of docs/plans/2026-11-20-99212-allium-spec-housekeeping.md. Line numbers drift as packages land: search for the cited symbol, not the line. Acceptance checks run from the worktree root.

# F2 — mcp-task-tools, work-packages, learnings (review #99212, HEAD b03f5ba0)

Read in full: learnings.allium 1-543; work-packages.allium 1-792; mcp-task-tools.allium 1-2630. Diffs 10b68974..HEAD: the work-packages/learnings diff read (its middle by a removed/added-construct filter: only the WorkPackageItemKind move); the mcp-task-tools diff by the same filter (2 open questions removed, 5 rules added, `trim(` added). The full current text of all three specs was read. Language reference: validation rules 14b-68 read, rest scanned; patterns.md headings scanned. Whole-set `allium check` on a scratch copy at HEAD: 0 diagnostics, 0 findings for all three (only startup.allium changed since the slice baseline e0e4b62a).
Verified against code: tool access classes (src/mcp/handlers/dispatch.rs `mcp_tools!`) match the read-only and destructive lists at 83-97; the redirect table's names match `OLD_TOOL_NAMES` (hint wording does not, F2-M-17); trigger params match the `*Args` structs; every "Asserted by" test exists; every citation of the `<spec>.allium: Name` form resolves except F2-W-3 (other citation forms not swept).

## Carried from Appendix F (98983): all 17 resolved, 0 open
W1 133/159/384/398 `trimmed()` · W2 512-515 `.any` · W3 49-53 Enumerations · W4 note gone · M1 1320 `limit?` · M2 200-206 restated, open question gone · M3 441-445 "the repo decides", open question gone · M4 no /queue-plan · M5 1553 `exists task` · M6 1564-1575 pointers, cli.allium 19/76/835 has uninstall, hook wording 1574 · M7 756 · A-n9 2467 A6 · L1 230-233 · L2 288-289 · L3 62-66 (N2 form; the code still runs the sweep: spacetime/module/src/learning.rs::retire_task_scoped_learnings, TASK_SCOPE_REMOVED_RULES_VERSION = 48) · L4 431 · L5 174-175.

## Findings
Codes are F2-<M|W|L>-n; in prose, M-n etc. abbreviate them. L-3 checked: no other spec states that retrievals and verdicts go with a deleted learning.

### mcp-task-tools.allium (F2-M)
| code | class | line | problem | remedy |
|---|---|---|---|---|
| F2-M-1 | mech | 1143-1144 | `McpUpdateTasks(task_id)` binds the first positional parameter, which is caller_identity (681, 2321); the tool takes task_ids | split the `or` into per-trigger rules; the update one binds task_ids and requires `task_id in task_ids` per id |
| F2-M-2 | mech | 825, 902 | CreateTaskViaMcp has two `@guidance` blocks with rule-level comments (853-900) between them; lang-ref rules 66/68 (not flagged by the checker) | move 853-900 above the first; merge into one `@guidance`, last |
| F2-M-3 | mech | 1798 | `core/SavedRepoPath.created_or_updated(` is not built-in (checker facts) | `let saved = core/SavedRepoPath{path: repo_path}`; `if exists saved` update, else `.created(` |
| F2-M-4 | mech | 967, 1084, 1197 | GetTaskViaMcp and the two list rules all listen on McpTaskSearch; the list `when:` omits task_id (positionally epic_id sits in the surface's task_id slot, 2329) and nothing requires task_id set or unset | write the full surface signature (with `task_id?`) in all three `when:`; GetTaskViaMcp `requires: task_id != null`, the list rules `requires: task_id = null` |
| F2-M-5 | mech | 307 vs 797 | `route_target` takes an Epic at 307 and an id at 797 | one signature |
| F2-M-6 | bp | 358, 1490 | unqualified cross-spec triggers: `BookmarkSet` (tasks.allium:1655), `ResumeTask` (dispatch.allium:1020; no `use dispatch`) | `tasks/BookmarkSet`; add `use "./dispatch.allium" as dispatch`, `dispatch/ResumeTask` |
| F2-M-7 | mech | 98-101 | the inserted workspace-member_set paragraph leaves "it is false on a read-only tool" orphaned (meaning destructiveHint) | move the sentence back after the destructive list |
| F2-M-8 | mech | 1851 | `trim(command)`, where the convention (work-packages 123) is `trimmed()` | `trimmed(command)` |
| F2-M-9 | bp | 104,123,167,652,705,973,1058,1104,1151,1219,1332,1830,1943,1960,1972,2091-2104,2132 | N1: 17 events no rule listens to (ToolsAnnotated … ContextReadRejected) | one "observable output; no rule listens" comment each, or one block naming them |
| F2-M-10 | bp | 120,140-141,255,721,855-862,1250-1251,1282-1283,1377,1441-1446,1501,1833,1899-1901,1921,1946,2066,2537-2539 | history and dated notes: "legacy allow-list is gone", "the older update_task", "now binds", "the old single-task reply", the master-repo anecdote, "258 KB", "done before the field was stamped", "only remaining", the salvage anecdote, "the older route", "the old set_review_command", "checked 2026-10-07 … Revisit", "today" ×3, "cost too much compute" | present tense; move the skills-extension "revisit" into a `deferred` entry; rewrap overlong lines 132 and 1635 |
| F2-M-11 | bp | 1226, 1351, 1136/1261 | literal limits: task_search default 50 (code `DEFAULT_TASK_LIMIT`), usage_read 50/500, goal cut 80 chars | add to the `config` block (task_search_default_limit, usage_read_default_limit, usage_read_max_limit) and read them |
| F2-M-12 | bp | 107,569,1025,1585,1806-1810,2613 | code named outside `path::symbol`: `mcp_tools!`, `ServiceError::Validation/NotFound`, `TaskTag::is_review`, "DB layer / trait contract", `build_prompt` | behavioural wording, or a `path::symbol` where a reader needs it |
| F2-M-13 | bp | 2541-2558 | AgentsRunTheVerifyCommand describes this repo's ./scripts/verify.sh levels inside a product guarantee (duplicates CLAUDE.md, docs/testing.md) | keep "the command may scale to the change"; move the level detail to docs/testing.md (line 301 has only part of it) and point there |
| F2-M-14 | mech | 1994-1996 | the listing says "ALL" approved learnings; the code lists only what the board's person sees (src/mcp/handlers/context.rs::list_entries → src/store/queries/learnings.rs::list_learnings, which filters `reader.sees`), and learnings.allium 85-88 says every read path applies visibility | "every approved learning the board's person sees; not narrowed by the caller's repo or epic"; the same for the read (get_learning filters too). Also fix the stale code comment above `create_learning` there ("visibility is governed entirely by its own scope/scope_ref"). Pairs with L-2 |
| F2-M-17 | mech | 137-166 | redirect hints quoted in the spec differ from src/mcp/handlers/dispatch.rs::OLD_TOOL_NAMES: update_tasks gets "call task_update" (spec groups it with update_task's "with task_ids: [id]"); rate_learning, set_review_command, managed_feed_config, update_workspace, list_keybindings are worded differently | state each hint's content without quoting, or quote exactly |
| F2-M-15 | by-design | 193,314,320,367,478 | `omitted` sentinel for three-state wrap_up_mode/model | no language form separates absent from null; explained in place; keep |
| F2-M-16 | by-design | 965-966 | GetTaskViaMcp/ListTasksViaMcp keep pre-rename rule names | present tense, and tests, specs and src cite them; keep |

### work-packages.allium (F2-W)
| code | class | line | problem | remedy |
|---|---|---|---|---|
| F2-W-1 | bp | 130,156,209,245,298,317 | six rules share their trigger's name (CreateWorkPackage, UpdateWorkPackage, ReorderPackage, LinkItem, UnlinkItem, DeleteWorkPackage) | rename the TRIGGERS, avoiding the existing WorkPackageUpdateRequested (175): e.g. CreatePackageRequested, PatchPackageRequested, ReorderPackagePressed, LinkItemRequested, UnlinkItemRequested, DeletePackageRequested. Keep the rule names: src/ and spacetime/module/src cite them. Update the emitters/providers: 147,186,192,194,279,387,400,410,448,547,592,735-736 and workspaces.allium:962 `wp/DeleteWorkPackage(item)` |
| F2-W-2 | mech | 501,543,545,781,786 | MCP triggers bind `package`, an entity at 518 and an id at 545; the code argument is `work_package_id` (src/mcp/handlers/work_packages.rs::UpdateWorkPackageArgs, ::DeleteWorkPackageArgs), and 789 says so | bind work_package_id; `let package = WorkPackage{id: work_package_id}` |
| F2-W-3 | mech | 69-70 | phantom citation `workspaces.allium: WorkPackageVisibility` (no such name) | cite SetWorkPackageVisibility or the workspaces read-predicate name |
| F2-W-4 | bp | 535-537, 555-556 | history: "no longer has a delete form", "replaces the `delete: true` form …, which is gone" | present tense: "has no delete form; a delete flag is an unknown field" |
| F2-W-5 | bp | 473 (336,355,366) | N1: WorkPackagesListed has no listener. ViewShowing/PackageView/EpicView have none anywhere either (epics.allium uses the same form) | comment WorkPackagesListed; leave ViewShowing to an epics package (cross-spec), note it |
| F2-W-6 | mech | 19 | Dependencies header omits core/Workspace and core/Visibility | add them |
| F2-W-7 | bp | 645 | "board_search_filter's own-match clause" is a code name | cite the board-layout rule, or a `path::symbol` |
| F2-W-8 | by-design | 21-43 | "Decisions taken with the user" block | present-tense design decisions; keep |

### learnings.allium (F2-L) — overlaps #99317 (running; its worktree has no spec edits yet)
| code | class | line | problem | remedy |
|---|---|---|---|---|
| F2-L-1 | mech | 131-133 | says source "learning_search" for that tool; the enum value is `query_learnings` (core.allium:364; rule 349) | "source query_learnings for learning_search" |
| F2-L-2 | mech | 85-87 | the read-path list names `get_learning`, a service method (src/service/learnings.rs::get_learning) and no tool | name the dispatch://learnings resource and listing (mcp-task-tools.allium: ResolveContextUri, ListContextResourcesViaMcp). Pairs with M-14 |
| F2-L-3 | drift | 424-438 | DeleteLearningViaMcp ensures only `not exists learning`; the module also deletes its Retrieval and LearningVerdict rows (spacetime/module/src/learning.rs::delete_learning_rows) | add both `for … where learning = learning: not exists …` loops |
| F2-L-4 | bp | 353-354, 360, 368-369 | dated statistics and history: "90th percentile of 16.5 KB", "the stored name predates the rename", "most-called tool (2083 calls…)" | present-tense rationale, no figures |
| F2-L-5 | bp | 158, 206, 350 | N1: DispatchPromptAugmented, RateLearningNudged, LearningsQueried have no listener | "observable output; no rule listens" |
| F2-L-6 | bp | 363-374, 521 | learning_rate takes a `ratings` list and is all-or-nothing, but the trigger and surface model one pair, and the batch rule is a comment (#99317 territory) | `McpRateLearnings(task, ratings)` rule: `ratings.count > 0`, unique learnings, every pair valid; per-pair ensures |
| F2-L-7 | bp | 263 | `validate_learning_scope` names a module function | "the store refuses it too" |
| F2-L-8 | by-design | 98-112 | ranking constants and caps 5/50 are literals | 109-112 states why they are not config; keep |
| F2-L-9 | by-design | 62-66 | rules-version-48 sweep sentence | N2-compliant, and the code still runs it; keep |
| F2-L-10 | by-design | 468-493 | ArchiveStaleLearning temporal trigger | guarded: requires approved and sets archived, so it fires once |

Counts (35): mech 14 (M-1…M-5, M-7, M-8, M-14, M-17, W-2, W-3, W-6, L-1, L-2), drift 1 (L-3), bp 14 (M-6, M-9…M-13, W-1, W-4, W-5, W-7, L-4…L-7), by-design 6 (M-15, M-16, W-8, L-8…L-10), entry 0, QUESTION 0.

## Questions
None. Each finding is behaviour-neutral or a spec-side catch-up to code (L-3).

## Work packages
| # | title | files | findings | order | acceptance (fails at HEAD, confirmed) |
|---|---|---|---|---|---|
| P1 | mcp-task-tools: bindings, built-ins, N1, present tense | mcp-task-tools.allium, src/store/queries/learnings.rs (one stale comment) | F2-M-1…F2-M-14, F2-M-17 (M-14 with L-2's wording) | none | see P1 below |
| P2 | work-packages: triggers renamed off rule names, id binding, tidy | work-packages.allium, workspaces.allium (962 emission only) | F2-W-1…F2-W-7 | none; grep src/ for trigger names in comments | see P2 |
| P3 | learnings: mechanical tidy | learnings.allium | F2-L-1…F2-L-7 | coordinate with #99317 (agent raoul3) before editing; run after its current run lands, or hand L-6 to it | see P3 |

Every package's footer: `./scripts/check-allium.sh` must show no new warning and no new field.unused, then run allium:weed on the package's specs.

P1:
```
f=docs/specs/mcp-task-tools.allium && ! grep -nE 'created_or_updated|McpUpdateTasks\(task_id\)|checked 2026-|legacy allow-list is gone|reply is the old|only remaining MCP dispatch tool|read half of the old|258 KB|measured going wrong|had no human in front|^\s+BookmarkSet\(|ensures: ResumeTask\(' $f && [ "$(awk '/^rule CreateTaskViaMcp/,/^}/' $f | grep -c '@guidance')" -eq 1 ] && grep -q 'no rule listens' $f && ./scripts/check-allium.sh
```
P2:
```
f=docs/specs/work-packages.allium && ! grep -nE 'when: (CreateWorkPackage|UpdateWorkPackage|ReorderPackage|LinkItem|UnlinkItem|DeleteWorkPackage)\(|no longer has a delete form|which is gone|^\s*-- WorkPackageVisibility\)|McpWorkPackage(Update|Delete)\(package' $f && grep -q 'no rule listens' $f && ./scripts/check-allium.sh
```
P3:
```
f=docs/specs/learnings.allium && ! grep -nE 'get_learning|predates the rename|2083 calls|90th percentile|prompt_injection for the dispatch prompt and learning_search for' $f && grep -q 'no rule listens' $f && awk '/^rule DeleteLearningViaMcp/,/^}/' $f | grep -q 'Retrieval' && ./scripts/check-allium.sh
```
