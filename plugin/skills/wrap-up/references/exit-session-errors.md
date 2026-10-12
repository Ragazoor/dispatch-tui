# If `exit_session` errors

- `"has no active session"` — something else (a merge, a manual close) already tore the session down. Treat this as already wrapped up, not a failure. Do not retry, and on the PR path do not re-create the PR.
- An error naming a mismatched action — the token doesn't match the action you're closing with. Show the user the exact error rather than guessing which action was intended.
- Missing/empty `pr_url` (pr path) — pass the URL you captured.
- `"no exit token — call wrap_up first"` or `"invalid exit token"` — the token was already used, or a newer `wrap_up` or a new dispatch of the task replaced it. Call `wrap_up` again for a fresh token, then `exit_session` with that one.

### If `exit_session` errors saying the close did not take effect

A refused or unanswered `exit_session` is an **error**, and it changes nothing: the task is still in its previous status, your tmux session is still alive, and the exit token is still valid. (The close is one all-or-nothing step, so a failed close never spends the token.) This happens, for example, when the shared store gave no answer.

When you get it:
- **Call `exit_session` again** with the same token and arguments — the token is still valid.
- If it keeps failing, tell the user plainly: the close did not take effect, the task is still in its previous status, and the tmux window is still alive, so it can be closed by hand from the TUI.
- **Do not** call `wrap_up` again for a fresh token unless the error says the token is missing or invalid.
- On the PR path, the PR itself still exists — don't re-create it. Only the task's move to Review failed.
