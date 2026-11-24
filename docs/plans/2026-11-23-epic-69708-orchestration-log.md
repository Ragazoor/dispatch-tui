# Epic #69708 orchestration log (task #99396, agent zlata2)

Operator left 2026-11-23 ~21:00 UTC, returns ~07:00 UTC 2026-11-24.

## DONE 06:30 UTC: step 8 approved by the operator and landed as 524965c3 (inventory kept below for the record)

Step 8's task text says to show **you** the inventory before deleting. I did not approve it for you. alina2 builds the removal on its branch (worktree `.worktrees/99392-orchestrator-8-10-remove-auto-dispatch`), runs verify, and does **not** wrap up. Say yes (or name what to keep) to alina2 or to me, and it lands with rebase. Steps 9 and 10 do not depend on step 8 and go on meanwhile.

```
STORE MODULE
1. Epic.auto_dispatch column: stays as dead schema (an automigration cannot drop a column), like workspace_id. Nothing reads it; a new row writes false. The update_epic patch loses its auto_dispatch field.
2. SessionRequest.chained_task column: stays dead, always 0.
3. chain_next_subtask + chain_candidates (AutoDispatchNextSubtask) and the call in request_exit_session: removed.
4. Origin "epic_chain": removed from DISPATCH_ORIGINS, so a request with it is refused as an unknown origin. Its phoenix and schedule guards in validate_dispatch_request go with it.
5. RULES_VERSION / BOARD_RULES_VERSION bump, module.wasm, bindings.
(recalculate_epic_chain is the status rollup up the parent chain; it stays.)

BOARD
6. Epic.auto_dispatch field, EpicPatch/EpicBuilder/UpdateEpicParams auto_dispatch.
7. ToggleAutoDispatch command, the U key (toggle_auto_dispatch), the "auto dispatch [U]" header indicator.
8. DispatchOrigin::EpicChain; handle_auto_dispatch_failed, AgentTracking.auto_dispatch_failed, CardIndicator::AutoDispatchFailed ("auto-dispatch failed") and its desktop notification.
9. SessionRequest.chained_task in the board model; session_exit's reply "Dispatching next epic subtask ..." (always "Session closed." now); the task_dispatch description's mention of the chain.
10. All tests of the above.

SPECS / DOCS
11. epics.allium: AutoDispatchNextSubtask, PhoenixIsNeverChained, ReportChainedDispatchFailure, SurfaceAutoDispatchFailure, ChainStallMarked, AutoDispatchFailureIndicator, ToggleAutoDispatch, the header indicator. core.allium Epic.auto_dispatch becomes a dead-column note. store-split: epic_chain origin, chained_task, W6, the two requires, its open question about chained failures. pr-workflow: SessionClosed's only consumer was the chain, so SessionClosed goes too (and mcp-task-tools / store-split notes about "no SessionClosed, so no chain"). schedules S8, mcp-task-tools, tasks, sync, retro-skill, onboarding-guide mentions. orchestrators.allium STATUS trimmed.
12. README, docs/guide/index.html, docs/mcp.md. Old docs/plans and docs/superpowers files stay as dated snapshots.

ADDENDUM (alina2, 02:03 UTC)
A. "epic_chain" stays as a history-only request origin: the module refuses a new request with it, the board still reads old rows and serves an open one. Otherwise every old chain row becomes a decode error. (Approved by zlata2: it keeps more.)
B. More files of the same removal: the notification kind auto_dispatch_failed; src/sync/encode.rs, decode.rs and memory_caller/convert.rs (the epic flag and chained_task fields); the memory_caller mirror of the chain (memory_caller/session_requests.rs); the U branch in src/tui/input/normal.rs; src/tui/messages/epic.rs; src/runtime/epics.rs::exec_toggle_epic_auto_dispatch; the keybinding guide in docs/guide/index.html; mentions in the keybindings, board-visuals, dispatch, agents and agent-tree specs.

UPDATE 06:15 UTC: the operator decided with alina2 that the orchestrator's start follows the same rules as a person's (SameRulesAsAPerson). The phoenix and scheduled-time refusals and the prompt markers are dropped; my choice (a) below is overruled. New head 3f3cb5ca, verify green.
READY TO LAND: main (steps 9 and 10) is merged in, head 92f42924, RULES_VERSION 71, verify green, inventory unchanged by the merge. After your yes alina2 wraps up with rebase; then run scripts/rotate-previous-store.sh once on main.
BUILT AND GREEN (02:40 UTC): branch 99392-orchestrator-8-10-remove-auto-dispatch, verify exit 0, head now e71faab9 (two extra commits beyond the removal: e490e3ab fix(test) for a real flaky conformance test, e71faab9 the orchestrator prompt never lists a phoenix or not-yet-due task as ready). `git diff main` in that worktree is the exact inventory. Not wrapped up.
- SessionClosed: no code consumed it (one test comment); removed from pr-workflow.allium only.
- Guards (a) built: the orchestrator's own start refuses a phoenix task and a task waiting for its scheduled time; a person's start still takes both.
- RULES_VERSION 70 on the branch is a placeholder; it takes 71 after merging main (step 9 has 70).
- After it lands: run scripts/rotate-previous-store.sh once (nobody is left to do it in a branch unless step 10 is still open).

Question: the phoenix and scheduled-time guards live only on origin epic_chain today. alina2 recommends (a): move both onto the orchestrator's own start in the module, as the orchestrators spec already says. zlata2 agrees with (a).
```

## Standing orders (from the operator)

- I am the backup. If tasks manage alone, do nothing.
- Every 15 minutes: check epic #69708 for stuck agents (needs_input, stale, crashed, conflict, review, or no progress) and help them continue.
- Each task agent must know to ask zlata2, not the operator. Message each agent once when it starts. Re-read the agent name from task_search each time (names change at dispatch).
- All step tasks wrap up with rebase (wrap_up_mode is set on #99385-#99394). Agents wrap up when verify is green, without waiting for a go-ahead.
- Design questions: answer from docs/specs/orchestrators.allium. If the spec is silent, take a decision, have the agent write it in its spec change, and record it under "Decisions for approval" below. This overrides the task text "Ask the user".
- Dispatch or hold tasks myself only if needed. Auto dispatch is on and chains by sort order (steps 1..10 in order is a valid order).
- Leave research task #99395 in the backlog. If auto dispatch starts it, tell its agent to stop and wait for the operator; note it here.
- Help only through SendMessage and the dispatch MCP tools. Other worktrees and main are read-only for me. Never drive tmux.
- Store rotation (scripts/rotate-previous-store.sh) does not block: check-previous-store.sh passes with a note when the fixture lags. One rotation after the run is enough; a non-module step may also do it in its own branch after merging main.

## Dependencies

1: none. 2: 1. 3: 1. 4: 1,3. 5: 1,3, after 4. 6: 3,5. 7: 4,5,6. 8: 4,7, after 5. 9: 2,3,5. 10: 9.
Module steps 1, 4, 5, 8 never run at the same time.
Task ids: step N = #(99384+N).

## Standard help message (replace the task id)

```
Help contact for task #99385: ask zlata2 (task #99396), not the operator, who is away for 10 hours.
- Any question or blocker: send it to zlata2 with SendMessage. Do not wait for the user.
- Design questions: zlata2 answers from docs/specs/orchestrators.allium, and decides when the spec is silent. This replaces "Ask the user" in your task text. Write any such decision into your spec change so the operator can review it.
- Wrap-up: your task is set to rebase. When ./scripts/verify.sh is green, run dispatch:wrap-up with rebase. Do not wait for a go-ahead.
- If verify fails for a reason you did not cause, tell zlata2.
No reply needed if all is well.
```

## Agents told to ask zlata2

- #99395 nika2 (asked to pause), 04:05 UTC
- #99394 natalia2, 03:10 UTC
- #99393 luca2, 02:04 UTC
- #99392 alina2, 02:00 UTC
- #99391 ayla3, 01:20 UTC
- #99390 elsa2, 00:51 UTC
- #99389 linnea2, 23:51 UTC
- #99388 yasmin2, 23:16 UTC
- #99387 sita2 (wrote to me first), 22:15 UTC
- #99386 sanja2 (description carries the rule; it wrote to me first), 21:43 UTC
- #99385 folke3, 21:02 UTC

## Decisions for approval

1. (#99385, folke3, 21:02 UTC) A snapshot restore does not carry the new `epic_orchestrations` and `ready_marks` tables. Same treatment as bookmarks. Reason: orchestrators.allium is silent; a restore must not switch an orchestrator on by itself; ready marks are cheap to set again; both tables can be added to the restore later without other changes. Recorded by the agent in spacetime-seed.allium.
2. (#99385, folke3, 21:25 UTC, decided by the agent, recorded in tasks.allium MarkTaskReady guidance and the store code)
   - A ready mark by an agent must name the agent; one by the operator names none.
   - Marking or clearing a missing task is refused ("task <id> not found"); clearing a task that is not ready is no error.
   - Change settings or switch off on an epic with no switch: "epic <id> has no orchestrator". A limit below 1: "the limit must be at least 1". An unknown start rule or marker is refused.
   - The O4 refusals read "epic <id> (<title>) above/below this one has an orchestrator".
   - DeletingAnEpicRemovesItsOrchestration is split: the switch part moved to epics.allium DeleteEpic; the log/notes/items part stays in orchestrators.allium as DeletingAnEpicRemovesItsOrchestratorMemory (step 5).
3. (#99386, sanja2, 21:43 UTC, agent's proposals, approved by zlata2; spec silent on each)
   - task_update/task_create `ready`: two store writes (field patch or create first, then mark or clear), because step 2 may not change the module. ready=true is refused up front when the task's resulting status is not backlog. If the second write fails, the reply says so.
   - epic_update gets `orchestrator` (bool), `start_rule`, `max_parallel` (>= 1). true switches on; false switches off and refuses settings next to it; settings alone change settings. true on an epic that is already on is refused, not turned into a settings change. No way yet to reset the limit to the default.
   - The orchestrator arguments are refused next to any other epic_update field ("the orchestrator arguments go in a call of their own").
   - Who marked: a task agent marks as agent with its name; a non-dispatched session marks as operator; a task caller with no agent name is refused.
   - Reads: task_search shows "Ready: ..." lines for backlog tasks and "Orchestrated by: epic <id>"; list rows get a " ready" word, no ready filter yet; epic_search shows "Orchestrator: on, start rule ..., limit N (default)" or "covered by epic <id>".
   - epic_update's reply to a re-parent names the orchestrators it switched off (board reads before and after). The board's own move reply waits for step 9.
   - Deviation (sanja2, 22:10 UTC, accepted): the re-parent reply reads before the move only. It names the switched-on epics in the moved subtree when an epic covers the new parent, which is what the store rule switches off. Reason: the subscription may lag the reducer reply, so a read after the move is not reliable. Recorded in epics.allium guidance.

4. (#99387, sita2, 22:15 UTC, agent's proposals, approved by zlata2)
   - **Step 3 changes the store module** (new Orchestrator table and reducers, RULES_VERSION bump), though the epic lists only 1, 4, 5, 8. Reason: names must be unique across agents and all hosts' orchestrators, and names are picked in the module. Step 1's text already pointed the rows at step 3.
   - A stored `session_died` flag and `last_turn_at` on the Orchestrator row (spec entity had neither; session_died was a derived fact).
   - **An orchestrator window survives a board quit or restart** and the next board start adopts it, like an agent window. "A board that does not run, runs none" is read as "starts none". Alternative not taken: kill the windows on board exit.
   - An orchestrator name given where a task id or agent name is expected answers "<name> is the orchestrator of epic <id> (<title>)"; epic_search and epic_update accept the name for the epic.
   - Mechanics: directory <data_dir>/orchestrators/epic-<id>; window "orch-<epic_id>"; `--name`, env DISPATCH_ORCHESTRATOR, MCP header X-Caller-Orchestrator; resume with `claude --continue` when the directory held a session.
   - Deferred from step 3: closing summary and EpicFinishedNotified (5/9); keeps-dying item and session log entry, logged to app.log until then (5); answer line in the prompt (6); notes/items/log/learnings in the prompt (5); the restart key (9).
   - Two more from sita2's own review (22:55 UTC, written in the spec): a board closes only the window of a row it saw on its own host and then saw go (a second board on the same tmux server closes nothing); a session that follows one that died starts fresh instead of `--continue`.
   - **Note for the operator:** from the next board start on the new build, every switched-on epic with open work gets a live orchestrator session, before steps 4-6 give it its tools and answer line. Switch off epics you do not want running yet. Until step 5, a twice-dead orchestrator is only logged to app.log; switch off and on restarts it.

5. (#99388, yasmin2, 23:16 UTC, agent's proposal, approved by zlata2)
   - **A start that is still being provisioned takes a slot.** running_here counts a running task whose current agent is on this host and either has a window or holds an open dispatch request (requested or provisioning). Reason: read literally, a burst of starts would each pass the limit, because no window exists yet; O7 means "at most N at once". A detached task, a task in review and a failed request take no slot.
   - Two hosts cannot both start one task: the claim is one transaction. Stated in the spec; no behaviour change.

6. (#99389, linnea2, 23:55 UTC; proposals approved or decided by zlata2)
   - New tools: orchestrator-answer_record, waiting-item_create, waiting-item_resolve, epic-note_create/update/delete, orchestrator-log_search. Epic and host come from the caller identity, never from arguments.
   - task_dispatch gets `reason`: required from an orchestrator (written as the started_task log entry in the same write), refused from anyone else.
   - Who may call: answers, items and notes are written by the orchestrator only; any caller may resolve an item. **The operator's own session cannot write epic notes for now.**
   - **Closing summary is split (my decision, differs from the agent's proposal to defer it to step 9):** step 5 builds orchestrator-summary_record (orchestrator only, only while the epic is done). The board asking for the summary before it closes the window moves to step 7 (it needs wake events).
   - Session log lines: "session started", "session resumed", "session ended", "session died".
   - **Learnings: only the prompt part is built** (ranked by the epic's title and description, no retrieval recorded). **Open for the operator:** learning_search/record/rate all need a task id and an orchestrator has none. Left undecided; belongs with the spec's knowledge-base open question and task #99395.
   - Deleting a task (soft delete too) resolves its open waiting items.
   - The prompt shows the newest 10 log entries; the three new tables stay out of the snapshot restore; epic and task reads of items, notes and log are built in step 5.
   - **Changes the spec's words (00:40 UTC, linnea2's proposal, approved):** the store now refuses an orchestrator's start of a task outside its epic's subtree. The spec said all three requires of OrchestratorStartsATask were prompt discipline and no store check. Reason: the start writes a log entry in the same transaction, and it would land under the wrong epic. Only the orchestrator's own path is checked; the ready mark is still not checked.
   - Tool id arguments are `epic_note_id` and `waiting_item_id` (a registry test requires <noun>_id).
   - **Known gap (linnea2):** the "orchestrator died twice" item is not raised while any open epic-wide item of that machine exists, as the spec says. An epic-wide question the orchestrator raised itself therefore hides the down alert. An item kind would fix it; not built.
   - **Process note (linnea2, honest report):** in step 5 the store module code was written before its tests, so those tests were never seen failing. The operator may want a second look at spacetime/module-tests for step 5.

7. (#99390, elsa2, 00:55 UTC, agent's plan, approved; all within the spec's surfaces)
   - A covered task's prompt names the orchestrator row of the covering epic on this host, read at dispatch; with no row, no orchestrator section.
   - Fallback to the operator is the agent's own check at question time (orchestrator not in the agent list, or message undelivered). No live check at dispatch.
   - One general line in the prompt redirects "ask the user" in skills to the orchestrator. Only the wrap-up skill is edited (path question and the short outcome report).
   - The orchestrator's prompt gains the answer line and names orchestrator-answer_record and waiting-item_create.
   - **The "try it on a small real epic" part of step 6 is not done.** It needs all steps on main and a board restart by the operator. elsa2 files a follow-up trial task in this epic (not ready, sort order 200).
   - **Gaps elsa2 found, for the trial and the operator:** (1) the orchestrator section is fixed at dispatch, so a task started before the orchestrator row existed, or a session resumed from before step 6, never learns of the orchestrator; (2) a covered agent that waits for the orchestrator raises no needs_input, so it may look idle or stale and be pinged — handed to step 7; (3) tasks created by decompose-review, fix and retro are not ready by default, so an orchestrated epic does not start them until marked; (4) on the pr path the outcome report goes out before the PR exists, so it carries no PR URL; (5) learning tools need a task.

8. (#99391, ayla3, 01:20 UTC; proposals approved or decided by zlata2)
   - **Step 7 changes the store module too** (a reducer and tool that write acted_on_trouble entries and refuse a second retry; RULES_VERSION 69). The log had no write path for trouble entries.
   - **Wake events are sent by the board, not stored.** The board compares the subtree between ticks and types one batched message into the orchestrator's window. Events while no board runs are not replayed; a "board restarted, check the board" wake and the first-turn check cover them. Alternative not taken: wake rows in the store.
   - **already_retried is narrower than the spec (flag):** a retry is spent until an item of the task is resolved or the task is started again. The spec says the retry also comes back when it worked ("the trouble ended by itself"), but the store has no record of that. Effect: crash, resume, a day of work, crash again goes to the operator. Fails toward the operator, never loops. Kept in the spec as intent, listed as open.
   - started_by_orchestrator = a started_task log entry written in the request's transaction.
   - Ping expiry is computed from the log entry time; no second item when the task already has an open one.
   - **Review reminder clock lives in the store** (my decision; ayla3 proposed board-local time). A board-local clock restarts with every board restart, so the 24h reminder might never fire. Use an existing "entered status at" time or add one in the same module change.
   - Closing summary: on a finished epic the board asks for it and keeps the window until the next completed turn or orchestrator_ping_wait, then ends the session.
   - Stale reaction covers elsa2's gap 2: the orchestrator does not ping an agent that waits for its own answer or for an open item.
   - Review reminder clock, detail (01:25 UTC): the store has no "entered status at" time, and a column or side table is a large change. Instead the store writes a log entry (kind "review", "entered review") when a covered task moves into review; the reminder counts from the newest such entry. A task already in review when the switch went on gets no reminder.
   - ayla3's own decisions, approved 01:45 UTC: (a) for start_failed only a resolved item returns the retry; (b) **a task with no host (a review of someone else's PR) counts as every machine's** for wakes and the review reminder, and only the open-item guard stops doubles — open with several machines; (c) **the board does not check waits_on_someone_else**; the orchestrator judges it from the wake text; (d) the orchestrator learns of an undelivered message from its own SendMessage error; (e) a failed wake send is logged and not retried; (f) the summary wait ends on a new summary entry or orchestrator_ping_wait; (g) wake lines carry ids only, no titles or item text (titles can come from outside feeds), and a failed start's reason is cleaned and cut to 120 characters.

9. (#99393, luca2, 02:10 UTC; proposals approved by zlata2)
   - **Step 9 changes the store module too:** a restart_orchestrator reducer that clears restarted_once, so the operator's restart of a down orchestrator can work. RULES_VERSION 70. I asked for an MCP twin of the restart.
   - **Keys:** 'Y' marks a backlog task ready (any view), 'O' opens the orchestrator (epic view and covered task cards), 'I' resolves the oldest open item of the epic after a y/n confirm.
   - Orchestrator key with several orchestrators below the viewed epic: on a task card it opens that task's; with exactly one below it opens it; with several it names them in the status line and opens nothing.
   - Raised-item notification: only the item's host board notifies, once per item; items already open at board start do not notify.
   - The ready marker shows a real mark only, not "counts as ready" under start rule everything; the header shows the start rule.
   - More from luca2 (02:20 UTC, approved): the epic edit form gets ORCHESTRATOR, START_RULE and MAX_PARALLEL sections, written as their own store writes after the other fields; new MCP tool orchestrator_restart(epic_id) for this machine's down orchestrator; 'Y' on a non-backlog card only says "Only a backlog task can be marked ready"; 'O' on a down orchestrator restarts it, starts the session at once and switches to its window; the epic view gets a band (name, state, start rule, running/limit, item count, open items, newest log lines; at most a third of the height); a board re-parent that switches an orchestrator off says so in the status line. I asked luca2 to add a way to reset the limit to the default (step 2 had none), so the form's empty MAX_PARALLEL can mean "default".
   - More from luca2 (03:00 UTC, approved): the agent list shows only this machine's live orchestrators, and the epic hide-list hides them; a settings change with limit 0 resets the limit to the default (switch-on still refuses 0); 'O' with no session restarts this machine's orchestrator when the epic has open work, down or resting; the header and the 'I' key cover every machine's open items (notifications stay per machine); the guide documents 'Y' and orchestrators; the marker reads "✓ ready" in cyan, with "· <agent>" when an agent set it. I asked that task labels use the shared name-or-id helper.

10. (#99394, natalia2, 03:10 UTC)
   - **The spec's first open question, decided by zlata2 (the task text says to answer it with the user; I told the operator before they left that I would decide it unless told otherwise): every epic stays reachable without the epic-grouped lens, and no new path is built.** Reasoning (natalia2's, from code and spec): an epic with no visible task shows as a card in Backlog, which never flattens; an epic with started work is entered with Enter on any of its flattened task cards, Done included; a sub-epic the same way inside its parent's view; a filter hid the same epics under the old lens. The one narrowing is flat_active, and flat is one BackTab away. Each case gets a test. Checked: the Done column never drops or ages out tasks; a done task leaves only by the operator's delete, and an epic left with no task falls back to a Backlog card.
   - The orchestrator view exists on the main board only. Inside an epic or package view the orchestrators lens reads as flat.
   - Badge [orch]; four columns, no sections; cards sorted by epic sort order; the "everything" lens is removed with its tests moved to flat.
   - **Keys on an orchestrator card: Enter, O, movement, and also the edit key (my addition to the spec's KeysOnTheCard)**, because the switch, start rule and limit live in the epic edit form. Other epic-card keys do nothing there.
   - More from natalia2 (03:20 UTC, approved): search in the orchestrator view matches the epic's own title or id only; the [orch] badge stays inside an epic view (which reads as flat, with no [flat] badge); Space on an orchestrator card enters the epic like Enter.
   - **Gap natalia2 found (03:30 UTC), decided by zlata2:** with the epic-grouped lens gone, a finished epic had no card anywhere, so the card keys x (delete), H/L (move status), m (re-parent) and w (link to a package) had no board path for it. **Decision: a finished epic draws one card in Done**, at its completed_at, beside its flattened done tasks. Options not taken: epic-view keys fall back to the viewed epic when the cursor is on no card; or accept the loss (MCP only).
   - **Still open for the operator:** an unfinished epic whose tasks are all in Running or Review has no card either (confirmed by natalia2), so x, H/L, m and w have no board path for it until a task returns to backlog or the epic finishes. Not built; recorded as an open question in board-layout.allium.
   - Card look and counts (natalia2's choices, approved 03:45 UTC): four lines (title; state · N waiting · 2/3; task counts; newest log line); down draws a red stripe; waiting is yellow bold; the card's "ready" count is real marks only, also under start rule everything; a finished epic's card goes just ahead of its own task group in Done on a time tie.
   - Fixed on my say: orchestrator_state skips detached tasks like the column does (the spec's "runs here"), so a card cannot read "working" with 0/3.
   - **Fixed on my say:** in the orchestrator view the batch keys do nothing even with a selection carried over from another lens, and the select-all row is not drawn; the selection is kept for the flat lens. **Open for the operator (today's behaviour, not changed):** a selection made in flat survives a Tab to flat_active, so x/H/L there act on hidden selected cards.

## Action log

- 21:05 UTC: rewrote the "Ask the user" rule in the descriptions of #99386-#99394 to point at zlata2.
- 21:00 UTC: set wrap_up_mode=rebase on #99385-#99394.

- 21:25 UTC: folke3 reports step 1 done, verify green, wrapping up with rebase. RULES_VERSION is now 65. Rotation of the previous-store fixture is pending (not blocking); I will ask the next non-module step agent to run it in its branch after it merges main.
- 21:43 UTC: step 1 is on main; step 2 (#99386, sanja2) started by auto dispatch. Answered its six questions (decision 3). Asked sanja2 to rotate the previous-store fixture in its branch.
- 22:10 UTC: sanja2 reports step 2 done, verify green, wrapping up with rebase. The previous-store fixture is rotated to main's module after step 1 (its own chore(fixtures) commit in step 2's branch).
- 22:15 UTC: step 3 (#99387, sita2) running. Answered six questions (decision 4). Step 3 is now a module step: do not let step 4 run beside it (auto dispatch chains one at a time, so no action yet). Fixture rotation after step 3 lands: ask step 6 or 7 (non-module), or leave to the operator.
- 22:55 UTC: sita2 reports step 3 done, verify green, wrapping up with rebase. RULES_VERSION is now 66. Fixture rotation due again; plan: ask step 4's agent to run it as its first commit, before it touches the module.
- 23:16 UTC: step 3 is on main (RULES_VERSION 66). Step 4 (#99388, yasmin2) running. Answered its limit question (decision 5). Asked it to rotate the fixture before its first module edit.
- 23:40 UTC: yasmin2 reports step 4 done, verify green, wrapping up with rebase. RULES_VERSION goes to 67. Refusal text: "epic <id> already runs <n> of <limit> tasks". Checked its note that the orchestrator's covering/backlog/ready requires are prompt discipline, not store checks: the spec on main already said so. StartAsked was removed from OrchestratorMcpTools (the limit lives in RequestDispatch). Fixture rotation due again; ask step 5's agent to do it before its first module edit.
- 23:51 UTC: step 4 is on main. Step 5 (#99389, linnea2) running. Sent help rules, the deferred pieces it owns, and the fixture rotation request.
- 23:55 UTC: answered linnea2's seven points (decision 6). TODO for me: when step 7 (#99391) starts, tell its agent it owns "ask the orchestrator for the closing summary before the window closes".
- 00:40 UTC: linnea2 reports step 5 verify green; approved the covering check; it wraps up with rebase next.
- 00:45 UTC: linnea2 wraps up step 5 with rebase, verify green. RULES_VERSION 68. Step 7 owns OrchestratorAskedForItsSummary; step 9 owns the resolve key and the notification on a raised item. Fixture rotation due again: ask step 6's agent (not a module step).
- 00:51 UTC: step 5 is on main (RULES_VERSION 68). Step 6 (#99390, elsa2) running. Sent help rules and the fixture rotation request.
- 00:55 UTC: elsa2 rotated the fixture to rules 68 (own commit). Approved its four plans for step 6 (decision 7).
- 01:05 UTC: elsa2 reports step 6 built, verify pending. Told it to file the trial as a follow-up task. TODO for step 7's agent: closing summary ask (from step 5) and gap 2 (waiting-for-orchestrator must not read as stale).
- 01:10 UTC: elsa2 reports step 6 done, verify green, wrapping up with rebase. Trial task #99398 filed (not ready, sort 200).
- 01:20 UTC: step 6 on main; step 7 (#99391, ayla3) running. Answered six questions (decision 8), handed over the closing summary ask and gap 2. Step 7 is a module step: step 8 must not run beside it (chain is sequential, so no action). Step 8 should rotate the fixture before its first module edit.
- 01:45 UTC: ayla3 reports step 7 built, RULES_VERSION 69, verify and wrap-up next. Step 8 must rotate the fixture before its first module edit.
- 02:00 UTC: step 7 is on main (RULES_VERSION 69). Step 8 (#99392, alina2) sent its removal inventory. Held its landing for the operator (see top), bookmarked #99392, told it to build and verify without wrap-up. Dispatched step 9 (#99393) myself, because the chain only fires when a session closes and alina2's will not: dispatched Task 99393 (luca2, "Orchestrator 9/10: Board: ready key, epic header, orchestrator key, notifications") — created worktree: /home/ragge/Code/work/experiments/dispatch/.worktrees/99393-orche
- 02:04 UTC: briefed luca2 (step 9): step 8 is not on main, deferred pieces it owns, fixture rotation to rules 69. Answered alina2's addendum (keep epic_chain as history-only origin). TODO after step 8 lands: fixture rotation (operator or a follow-up).
- 02:10 UTC: luca2 rotated the fixture to rules 69. Answered its six questions (decision 9). Told alina2 that step 9 takes RULES_VERSION 70 and step 8 takes 71 after merging main.
- 02:40 UTC: alina2 reports step 8 green and unwrapped. Asked it for two small extra commits while it waits: the orchestrator's prompt skips phoenix and not-yet-due tasks; and a look at a flaky conformance test (learning_scope_refs, epic id 2 vs 4096, seen once).
- 02:55 UTC: alina2 finished both extra commits, verify green at e71faab9, still unwrapped. The flake was real: two conformance tests compared a sub-epic id across the real store and the twin.
- 03:00 UTC: luca2 reports step 9 built, verify next. Step 10 should rotate the fixture to rules 70.
- 03:10 UTC: step 9 is on main (RULES_VERSION 70). Step 10 (#99394, natalia2) started by the chain. Answered its questions (decision 10), asked for the fixture rotation to 70. Told alina2 to merge main only after step 10 lands.
- 03:15 UTC: natalia2 rotated the fixture to rules 70 (b75573b9) and confirmed the Done check. Building step 10.
- 03:55 UTC: natalia2 reports step 10 done, verify green, wrapping up with rebase. Next: when it is on main, tell alina2 to merge main, take RULES_VERSION 71 and re-verify (still no wrap-up).
- 04:05 UTC: step 10 landed and the chain started the research task #99395 (nika2), which the operator said to leave. Asked nika2 to pause, not wrap up and not close its session (closing would chain #99398). Gave it two short answers for later.
- 04:06 UTC: step 10 is on main. Nine steps landed (1-7, 9, 10). Told alina2 to merge main into the step 8 branch, take RULES_VERSION 71 and re-verify, without wrap-up.
- 04:25 UTC: alina2 merged main into the step 8 branch: head 92f42924, RULES_VERSION 71, verify green, inventory did not grow. Waits for the operator's yes. Nothing else runs; research #99395 is paused; trial #99398 is in the backlog.
- 06:15 UTC: alina2 reports the operator's decision SameRulesAsAPerson: the orchestrator may start a ready phoenix task and a task before its scheduled time. Head 3f3cb5ca, verify green, not wrapped up. Raised with the operator: under start rule "everything" a phoenix task's fresh copy counts as ready at once, so an orchestrator could restart it without end.
- 06:30 UTC: step 8 landed on main as one squashed commit 524965c3 after the operator's approval. All ten steps are on main. Left: fixture rotation to rules 71 on main; trial #99398; research #99395 paused; the running board is still the old build, so its chain may start another backlog task when alina2's session closes.
- 07:00 UTC: the operator is back; nika2 (#99395) landed. Stopped the 15-minute check (cron c8f585a7).


## Fix tasks proposed after the trial (not created; the user has not said go)

From hoa3's trial report (task #99398), my reading of the orchestrator prompt, and hoa3's review of the list on 2026-11-24.

1. **Keep unanswered questions open.** A waiting item must not close when its task finishes unless the operator answered it. Store rule `FinishedTaskResolvesItsItems` in `spacetime/module/src/orchestrator_memory.rs`: a module change, a `RULES_VERSION` bump and the memory_caller mirror. hoa3 recommends: an item about work beyond its task is raised with no task, so the task's end cannot close it.
2. **Stop false stale alarms.** An agent inside one long Bash call (a 10+ minute verify run) fires no hook, and stale reads the time since the last tool call. Most likely host side (`src/runtime/`), not a store rule. Check that first.
3. **A resumed review task goes to running, then stale.** The orchestrator did not resume #99403: no transcript holds a `task_resume` call for it. Both resumes (07:55:39 and 08:08:12 UTC) came through the board's resume path, most likely from a message the operator sent. Resume moves a review task to running by design (`dispatch.allium`). The orchestrator then pings it and the agent wraps up again. Ask the operator about 07:55 first.
4. **Epic status stays backlog while tasks run.** By design (`spacetime/module/src/support.rs::derive_epic_status`, `epics.allium` RecalculateEpicStatus). A question for the user, not a fix task.
5. **Orchestrator prompt and launch** (`src/dispatch/orchestrator.rs::build_orchestrator_prompt`):
   - guidelines, not bans: no raw tmux, no git or gh writes, stay out of the checkouts. The tmux rule is only in the repo `CLAUDE.md`, which the orchestrator never loads;
   - no dispatch tool shows what an agent is doing (`ListAgents` gives busy or idle, `task_search` the sub-status). Add a read tool for an agent's last screen lines, or tell the orchestrator to ask the agent by message;
   - give it the repo path. It went into the main checkout to run `git fetch` and `gh pr view` while it judged a PR;
   - say that a task that waits for a person is not trouble, and that a long check run is normal;
   - do not load the task agent skills (wrap-up, retro, pull-request);
   - notes must not spread credential recipes (an orchestrator note led an agent to print a token into its transcript).
6. **Wrap-up skill.** An argument counts as the chosen path only when a person gave it, not when the agent passes one itself; that is why the path question never reached the orchestrator. On the rebase path, the outcome report goes out after `session_wrap-up`, not before.
7. **Repo-local skills that create tasks** (`.claude/skills`, for example brainstorm-features) do not know about orchestrators. Separate from 6: other files.

Order: only 1 (and 4, if kept) change store rules and must run one at a time. The rest can run beside them.

Still open from the trial and not in this list: the covered prompt is fixed at dispatch, and the learning tools need a task id.

Also dispatched on 2026-11-24, standalone: #99414 (guido), errors from the renamed pr-learnings hook.

Note: `main` already moved on parts of 6 and 7 while this list was written (6c2c68ea and ed8b4136 on the outcome report, 15c34c2f and 5b25263a on task-creating skills and the ready mark). Check them before creating those two tasks.
