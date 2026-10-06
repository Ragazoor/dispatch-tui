# Handling an iteration result

Detail for step 3 of "Each Iteration": what to do when the subagent's result arrives.

3. When that call's result arrives (a task-notification, per the Agent tool's async dispatch
   model — this may land in a different turn than the one that dispatched it):

   - **The subagent errored, was skipped, or returned an unparseable report** — treat a report as
     unparseable if it is missing EITHER required label (`CONVERGED:` or `SUMMARY:`), not just
     `CONVERGED:`; a partial or malformed report is an error, not a result. Read `retry_count` from <!-- allow-phantom-symbol: allium-loop's own state-file field, schema declared only in this skill's prose -->
     the state file.
     - If `retry_count == 0`: set it to `1`, keep `{{ITERATION_NUMBER}}` unchanged, and
       re-dispatch the same iteration (repeat step 2 above).
     - If `retry_count` was already `1`: delete the state file and tell the user this iteration <!-- allow-phantom-symbol: allium-loop's own state-file field, schema declared only in this skill's prose -->
       failed twice and the loop has stopped — do not retry indefinitely or silently treat this
       as "no progress."
   - **A real report was returned** (both labels present): increment `runs_completed` in the state <!-- allow-phantom-symbol: allium-loop's own state-file field, schema declared only in this skill's prose -->
     file by exactly 1 and reset `retry_count` to `0`. Then update `consecutive_no_change_runs`: if <!-- allow-phantom-symbol: allium-loop's own state-file field, schema declared only in this skill's prose -->
     this run's `SUMMARY` reports no changes, increment it by 1; otherwise reset it to `0`.
     - **`CONVERGED: yes`**: delete `.claude/allium-loop-state.local.md` and report success to
       the user, including the final `SUMMARY`.
     - **`CONVERGED: no`** and either `runs_completed >= max_iterations` or
       `consecutive_no_change_runs >= 2`: delete the state file, summarize to the user exactly
       what's unresolved and why, and stop. Never emit a false convergence claim to exit early.
     - **`CONVERGED: no`** and budget remains (`runs_completed < max_iterations` **and**
       `consecutive_no_change_runs < 2`): dispatch the next iteration (repeat step 1 above).

   When dispatching the next iteration the number advances on its own: `runs_completed` was just <!-- allow-phantom-symbol: allium-loop's own state-file field, schema declared only in this skill's prose -->
   incremented, and step 1 derives the number as `runs_completed + 1`. (The retry re-dispatch above
   is the one case that deliberately reuses the same number.)
