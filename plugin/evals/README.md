# Plugin evals

Local `claude plugin eval` cases for the dispatch plugin's skills. Run them with
`scripts/run-evals.sh`; the rules are in "Agent-behaviour evals" in
`docs/testing.md`.

Layout:

- `<case>/prompt.md` — the user message, with frontmatter (`allowed_tools`, `runs`, `max_turns`).
- `<case>/graders/*.md` — one grader per file. LLM graders need `focus: trace`.
- `<case>/mocks/dispatch/<tool>.md` — per-case mock reply for a dispatch MCP tool.
- `mocks/dispatch/<tool>.md` — shared mock replies; a case's own mock wins.
- `results/` — run output, gitignored.

This directory is embedded in the binary with the rest of `plugin/`, but
`src/setup/plugins.rs::is_dev_only` keeps it out of the installed plugin.

Cases (2 runs each; the whole suite costs about $2):

- `wrap-up-done`, `wrap-up-rebase` — a preset path runs to `exit_session`.
- `wrap-up-asks-path` — no path preset: asks first, no `wrap_up`, no PR work before the choice.
- `wrap-up-retro-order` — the retro runs before `wrap_up`.
- `retro-done-files-task` — on `done` the retro files a task and edits nothing.
- `learnings-rate-on-use` — rates a used entry `helped`.
- `learnings-no-lintable` — does not record a rule a lint could check.
- `grill-one-question` — one question with a recommendation, then waits.

Notes: Bash and git are unavailable in a run, so commit steps are not tested.
`AskUserQuestion` is not available either; the agent asks in plain text, so
graders check the text, not the tool. The default Haiku judge misreads the
tool order in `wrap-up-retro-order`; `--judge-model sonnet` was stable there.
