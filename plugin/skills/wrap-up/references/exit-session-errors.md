# If `exit_session` errors

- `"has no active session"` — something else (a merge, a manual close) already tore the session down. Treat this as already wrapped up, not a failure. Do not retry, and on the PR path do not re-create the PR.
- An error naming a mismatched action — the token doesn't match the action you're closing with. Show the user the exact error rather than guessing which action was intended.
- Missing/empty `pr_url` (pr path) — pass the URL you captured.

### If `exit_session` succeeds but says the close did not take effect

`exit_session` can return a **successful** response that nevertheless reports the close did **not** happen: text saying the task could not be moved to its terminal status, that your tmux session is still alive, and that it needs closing by hand. This is deliberate rather than an error — the exit token is consumed before the terminal write is attempted, so an error response would strand you with no retry path. Read the response text; don't infer success from the absence of an error.

When you get it:
- **Do not retry** `exit_session` — the token is gone, so a retry only produces "call wrap_up first".
- **Do not** call `wrap_up` again for a fresh token.
- Tell the user plainly: the close failed, the task is still in its previous status, the tmux window is still alive, and it needs closing by hand from the TUI.
- Your session stays open. Nothing was torn down, so the user can attach to the window.
- On the PR path, the PR itself still exists — don't re-create it. Only the task's move to Review failed.
