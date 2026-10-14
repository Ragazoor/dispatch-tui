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
