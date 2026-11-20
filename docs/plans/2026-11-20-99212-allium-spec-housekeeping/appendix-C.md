# Appendix C — feeds, feed-scripts, schedules, pr-workflow

Condensed report of one read-only review agent for task #99212, at commit b03f5ba0 (main, 2026-11-18). Part of docs/plans/2026-11-20-99212-allium-spec-housekeeping.md. Line numbers drift as packages land: search for the cited symbol, not the line. Acceptance checks run from the worktree root.

# Appendix C — feeds, feed-scripts, schedules, pr-workflow (review #99212, HEAD b03f5ba0)

## Coverage
- feeds.allium (4106 lines): read in full, 1–4106 (2127–2234 `KeepWorkedOnFeedTask` first through the diff, then 2190–2234 numbered). Full diff 10b68974..HEAD read.
- feed-scripts.allium (605): read in full. Diff read (17 lines).
- schedules.allium (392): read in full. Diff read (26 lines, tool renames + headers).
- pr-workflow.allium (1283): read in full. Diff read as removed lines (-U0); the end state was read in full.
- References: assessing-specs.md read; language-reference.md grepped (lambdas, `this` 1005–1012, ensures guard semantics ~860–890, overlapping triggers 745, `if not exists`); patterns.md grepped only for `if (not) exists`; actioning-findings.md not read.
- Checker: slices are at e0e4b62a. No later commit touches these four specs. A scratch whole-set `allium check` at b03f5ba0 gives the same result: one diagnostic, feeds:188 `FeedCommandRun` (baselined, comment present, upstream #106), and none in the other three. `analyse` reports 0 findings (from the slices, not re-run).
- Code checked: spacetime/module/src/feed.rs (refreshed_feed_task, left_feed_is_deleted, left_feed_moves_to_done, keep_left_feed_task), spacetime/module/src/feed_ingest.rs (route_and_move, ingest_role_routed), spacetime/module/src/schedule.rs::schedule_after_status_write, cited pr-workflow symbols (all exist), and the scripts' 500-char and limit literals.

## Carried from the last appendix (98983 Appendix C)
Every item was re-checked against the current text.
- feeds: 12 of 13 bullets resolved: old() → `let changed`; TaskTeardown emissions → comments; the dot-methods; PollOwnerClaimed instead of a direct create (FeedTick, ManualFeedTrigger); route(item.signals)/m.user; the 2787 history; ingest_role_routed citation; the Upsert heading; PollOwnerReducers in Surfaces; inverted if; the history passages at 22, 2603, 3346, 3384, 3550 and 2169; the N2 sentence; ScriptFile* comment. **Still open: B7 history remnant.** 126–129 still has "preserving wire compatibility…" (fixed at 72, 97 and 133 but missed here), and new history arrived with the keep feature (see C-F-12).
- feed-scripts: 2/2 bullets resolved (pr_description_chars, Config header).
- schedules: 1/1 bullet resolved (headers).
- pr-workflow: 10/10 bullets resolved. B16 sub_status forms are now all `core/SubStatus.…`. The PR-state seam drift is fixed: PollPrStatus posts raw strings, and PrMerged/PrClosed keep host effects only. The ClosePrOnDone QUESTION was settled as (a): it fires on the claim of a close row. The WrapUpRequested binding, upper(), @guidance in the entity, Surfaces placement, `core/local_host()`, the dead "ClosePrOnDone's owner" citations and the history passages are all fixed.

## Findings
Classes: mech, entry, by-design, QUESTION, drift, bp. No entry findings: every trigger the four specs listen to has an emitter or a providing surface.

### feeds.allium
- **C-F-1 drift 2647–2666.** RoleRoutedFeedSync's match arm refreshes 5 fields inline and sends only absent items to UpsertFeedItems. So on a reviews parent the coming-back reopen (2011–2015), the url backfill and UnchangedFeedItemIsNotRewritten never apply, although NewReviewRequestIsSignalled (2896–2898) relies on the reopen. Code: `spacetime/module/src/feed_ingest.rs::ingest_role_routed` sends every routed group through `upsert_feed_tasks` (`spacetime/module/src/feed.rs::refreshed_feed_task`). The spec's `UpsertFeedItems(target, {item}, true, host)` at 2665–2666 ("Insert only", always additive) drifts too: the code passes the cycle's own mode (`upsert_feed_tasks(sub, group, created_by, !additive)`), so on a trusted cycle each target also runs its per-epic stale pass. → After the moves, `UpsertFeedItems(target, the items routed there, additive, host)` once per target; drop the inline refresh, "Field refresh (same as…)" and "Insert only".
- **C-F-2 drift.** Prose still says a feed task absent from a trusted emission is deleted and torn down, or comes back by reinsert: 110–114, 257–258, 1530–1533, 1573–1575, 1867–1872, 1914–1919, 2248–2252, 2269–2273, 2353–2357, 2405–2410, 2570, 2848–2854 (coverage text), 3231–3232, 3356–3357, 3512–3514, 3934–3940. Code: `spacetime/module/src/feed.rs::left_feed_is_deleted` (only backlog is deleted). → "An untouched (backlog) task is removed; a worked-on one is kept (KeepWorkedOnFeedTask)". A kept task whose item returns keeps its wrap_up_mode, repo_path and model; only done→backlog.
- **C-F-3 drift 2774–2782, 3342–3352.** The rationale contradicts the keep rule. "No clause here consults status … force-torn-down even with a live agent" and "WHY NOT GATE THE TEARDOWN ON TASK STATE … rejected … the exact case feed-driven teardown exists for" conflict with 2680–2682: a merged PR's running agent is kept, not torn down. → The stale pass decides removal by status (backlog only); teardown of a removed row stays unconditional; the rejected alternative is "delete the row but skip its teardown".
- **C-F-4 drift 2964–2965.** ReviewTaskModelIsSetByRole names only "removed as stale and later emitted again". → Add: a kept task whose item comes back is a refresh and keeps its model (`spacetime/module/src/feed_ingest.rs::route_and_move` sets a model only when `found.is_none()`).
- **C-F-5 bp 2185–2193.** KeepWorkedOnFeedTask: `if not task.left_feed:` repeats the requires, and an ensures guard reads the resulting state. `if task.status in {running, review}:` guards on the status it writes. → `task.left_feed = true` unguarded; `let finishing = task.status in {running, review}` before ensures; reword "IDEMPOTENT" (a later absence does not fire the rule: requires).
- **C-F-6 bp.** Ensures guards read fields the same block writes: 1990 (`existing.url = null`) and 2011–2015 (`existing.left_feed`, `existing.status = done`). → Bind the pre-values with `let` before the assignments (the `if old(f)` convention). 2425 relies on the post-state on purpose; leave it.
- **C-F-7 bp 1287, 2292, 2304, 2644–2646.** `this` used in rule `where` clauses; it is valid only in entity declarations and identified_by. → `for request in … : if not served_by_this_process(request)`; `for item in items where not retired_without_task(epic, item)`; a black box `subtree_task(epic, item.external_id)` for the existing-task lookups.
- **C-F-8 bp 3372–3375, 3578–3582.** `for task …: exists task` in ensures is not a postcondition form. → Drop both loops; `additive: true` already carries the meaning, and the comment can stay.
- **C-F-9 bp 3046, 3153.** `log_warn(...)` is a black-box call used as an ensures statement. → Emit an observable trigger (e.g. `FeedRunWarned(epic, text)`) with the N1 comment.
- **C-F-10 bp 1366–1367.** Rule OverrideFeedOwner shares its trigger's name. → Rename the trigger `FeedOwnerTakeoverAccepted(epic)` (FeedBoardKeys 4037; comment epics.allium:177).
- **C-F-11 bp (N1).** Unlistened outcomes have no "observable output; no rule listens" comment: FeedVerified 728, SetupSectionLine 1156, FeedIntervalBelowMinimumWarned 1226, FeedCycleDropped 1588, FeedTaskReleased 3899. → Add the one-line comment.
- **C-F-12 bp history.** 126–129 "preserving wire compatibility…"; 2136 "what every mirroring feed did before"; 268–271 "did reach my_reviews in practice"; 2588 and 2707 "legacy duplicate"; 3244–3246 "where the incident was observed"; 3297–3299 "the reported wipe"; 3472 "today it is reached only by accident"; 3975 "a command that predates the column"; 1323 "Today's…"; 2546 "today's documented…"; 1619 "not reachable today". → Present tense.
- **C-F-13 by-design 1477, 2903.** `workspace.is_admin(…)` and `epic.visible_to(…)` look like dot-methods but are parametrised derived values declared in core (core.allium:691, 943, 998). Leave them.
- **C-F-14 by-design 188.** `FeedCommandRun` warning: a real checker gap, commented and baselined. Keep.

### feed-scripts.allium
- **C-FS-1 drift 216–218, 283–284.** "the feed's reconcile already removes the task once the merged PR closes the alert" and "a missing one deletes its task under reconcile": a worked-on CVE task is kept in done (feeds: KeepWorkedOnFeedTask). → Reword: "an untouched one is removed; a worked-on one moves to done and its agent window closes".
- **C-FS-2 bp 538, 560.** Literal defaults in prose: LEVELS "WARN|ERROR" and SAMPLES 3 (scripts/fetch-log-warnings.sh:67–68). → `config.log_warning_levels` and `config.log_warning_samples`.
- **C-FS-3 bp history 58–59, 164, 527.** "became 'dependabot[bot][bot]'", "now surfaces", "The runtime learned". → Present tense.

### schedules.allium
- **C-S-1 bp 224–231 vs 269–277.** A hand write moving a scheduled phoenix task from running to backlog fires both LeavingRunningCancelsThePendingRun (`next_due = null`) and EnteringBacklogRearmsAPhoenixTask (`next_due = next_match`): their preconditions overlap and their ensures conflict. Code applies the re-arm last (`spacetime/module/src/schedule.rs::schedule_after_status_write`), and the guidance at 285 says "then re-armed". → LeavingRunning gets `requires: not (to = backlog and task.phoenix and task.schedule != null)`.
- **C-S-2 mech 18.** "local_host() from core.allium". → `core/local_host()`.
- **C-S-3 bp history 256–259, 234, 280.** A past-tense narrative ("kept its due time … would fire it at once") and "Decided with the user:" prefixes. → Present tense; cite the S-id or drop the prefix.

- **C-S-4 by-design 165.** FireDueTask's temporal trigger is guarded against re-firing by the open-request check and `board_last_asked_to_fire` (config.schedule_refire_after, matching `src/tui/mod.rs::SCHEDULE_REFIRE_AFTER`).

### pr-workflow.allium
- **C-P-1 bp.** Ensures guards read fields the block writes: 96–97 (`pr_learnings_gate_shown_at = null`); 504 and 553 (an outer `task.status = review` while the merged branch writes `done`); 514, 522 and 554 (`sub_status != X: sub_status = X`). → Bind the pre-values with `let` before ensures.
- **C-P-2 bp 1236–1240.** FinishTaskConflict emits UserInformed, but its comment says no host sends a status line or notification and the card section is the signal. → Drop the emission, or make the comment the definition.- **C-P-3 bp (N1).** Unlistened outcomes have no comment: PrCreateBlocked and PrCreateAllowed 98/100, UserInformed 381, TmuxWindowKilled 425/1193, DesktopNotification 428/463. → Add the one-line comment.
- **C-P-4 bp 702–704, 880–881.** "Do not re-add an updated_at assertion" is an editing instruction, not behaviour. 1094 has "today". → Drop them; the "performs no task write" sentence already says it.
- **C-P-5 by-design 793–904.** WrapUpPr and WrapUpDone have no ensures, deliberately: the module writes the row (store-split RequestWrapUp), and the guidance says so.
- **C-P-6 by-design 229.** `pr_report_reply(task)` read in the same ensures as the PrStatusReported emission. It models the synchronous reducer reply.

## Counts
drift 5 (C-F-1..4, C-FS-1; C-F-2 groups 16 passages) · bp 16 · mech 1 · by-design 5 · entry 0 · QUESTION 0.

Cross-cluster note, not filed as a finding: find-or-create guards (`if not exists X: X.created(…)`, feeds 1412 and 2296; `if exists poll … else PrPollState.created`, pr-workflow 242, 256 and 285) read the resulting state by the ensures-guard rule. Neither language-reference.md nor patterns.md shows the idiom, and the 98983 appendix prescribed it for 1412. Changing it would reverse that remedy, so decide it once for every cluster, not here.

## Work packages
**C-WP1. feeds: one story for the kept feed task.** Files: feeds.allium, feed-scripts.allium. Findings: C-F-1..5, C-FS-1. No QUESTION: `left_feed_is_deleted` and `refreshed_feed_task` already decide the behaviour. Do it first, because C-WP2 edits the same rules (KeepWorkedOnFeedTask, RoleRoutedFeedSync). Run `allium:weed` on RoleRoutedFeedSync and KeepWorkedOnFeedTask afterwards.
Acceptance (fails today, exit 1):
```
! grep -nE "is deleted and reinserted|deleted and reinserted, taking|no clause here consults status|feed-driven teardown exists for|already removes the task once|a missing one deletes its task|is deleted from the epic\.|Field refresh \(same as UpsertFeedTasks' match path\)" docs/specs/feeds.allium docs/specs/feed-scripts.allium
```
**C-WP2. feeds and feed-scripts: forms, names, N1, history.** Files: feeds.allium, feed-scripts.allium, plus the one comment at epics.allium:177 (cluster boundary; coordinate with the epics package). Findings: C-F-6..12, C-FS-2, C-FS-3. No QUESTION. Run after C-WP1. Also grep src/ for "OverrideFeedOwner" when renaming the trigger (only keybinding names match; the rule keeps its name).
Acceptance (fails today, exit 1):
```
! grep -nE "^[^-]*\bthis\b[.)]|^\s+exists task\s*$|^\s+log_warn\(|when: OverrideFeedOwner\(|preserving wire|predates the column|if not task\.left_feed:|if existing\.left_feed:|what every mirroring feed did before" docs/specs/feeds.allium
```
**C-WP3. pr-workflow and schedules.** Files: pr-workflow.allium, schedules.allium. Findings: C-P-1..4, C-S-1..3. No QUESTION. C-P-2 removes an emission: confirm with `allium:weed` that no test asserts it. Independent of WP1 and WP2.
Acceptance (fails today, exit 1):
```
! grep -nE "if task\.pr_learnings_gate_shown_at = null:|if task\.sub_status != core/SubStatus\.pr_closed:|ensures: UserInformed\(about: rebase_conflict|^\s+if task\.status = review and task\.url" docs/specs/pr-workflow.allium && sed -n '/^rule LeavingRunningCancelsThePendingRun/,/^}/p' docs/specs/schedules.allium | grep -qE '^\s+requires:.*phoenix'
```
Every package's footer: run the whole-set `./scripts/check-allium.sh`; no new warning and no new field.unused.
