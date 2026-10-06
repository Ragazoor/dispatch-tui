# Recording detail

What not to record, how to pick a kind and scope, and how to write the summary. Read this before calling `record_learning`.

### Do NOT record:

- Code patterns readable from source code — the code is self-documenting
- Things already in CLAUDE.md, README, or existing docs
- Git history — visible via `git log` / `git blame`
- Debugging solutions where the fix is in the commit
- Things too specific to generalise — if it won't apply to other tasks, skip it
- How you fixed a specific problem — that's in the code and commit message
- **The name of the code that currently implements it.** An entry describes durable behaviour, a convention, or a domain fact. It does not name the **function**, **type**, **macro**, **fixture**, test, or **file** behind it. That includes a bare one: `TuiRuntime`, `handle_tick`, `make_task` and `in_memory_db()` are all out, not only `path.rs::symbol` and `Type::method`. <!-- allow-phantom-symbol: describes the citation shape itself, not a real reference -->

  **Why:** a refactor can invalidate a name at any moment, and nothing re-checks the knowledge base the way `check-doc-symbols.sh` re-checks docs on every push. A correct citation today goes stale forever with nobody noticing. A high upvote count does not exempt an entry — the rule is about rot, not about present usefulness.

  **What to do instead:** if the fact is worth stating precisely, put it in the Allium spec or a Rust doc comment. Both are gated and re-checked on every push. The knowledge base keeps the prose.

  **`record_learning` rejects only the shapes prose never produces** — a `::` citation, a call with empty parentheses, a macro invocation, a long snake_case name, a path into the tree, a source filename. A bare `TuiRuntime` or `handle_tick` passes the validator and still breaks the rule. Do not read a successful call as approval. A stable MCP tool name (`query_learnings`, `wrap_up`, ...) is fine — that is a public interface, not internal detail. So is a root manifest or a spec file (`Cargo.toml`, `package.json`, `feeds.allium`).

  Bad: "A step that must behave identically on both feed paths goes in `src/feed/cycle.rs::run_feed_cycle`." <!-- allow-phantom-symbol: the actual stale citation learning #401 carried -->
  Good: "Feed-cycle logic shared by the auto-poll and manual-refresh paths must live in one place, not be duplicated per caller — see feeds.allium."
- Generic language/library idioms that would apply to any codebase (e.g. "use an enum instead of a string sentinel," "clone the Arc once, not per branch") — if it's not tied to a specific type, module, or convention in *this* repo, it's not repo-scoped knowledge

### Picking a kind

| Kind | Use for |
|------|---------|
| `pitfall` | Silent failures, API traps, behaviour surprises — warn future agents |
| `convention` | Preferred patterns or style for this codebase |
| `preference` | Explicit user preference expressed during the task |
| `tool_recommendation` | Specific tool or library for a problem type |
| `procedural` | Step-by-step instructions that steer other agents (epic-level) |
| `landscape` | Codebase/system overviews — service maps, module responsibilities |

**A `procedural` entry must say where it stops.** It steers other agents, and an instruction that says what to do and never when to stop is not a guardrail. Its `detail` is required, and it must name the case where the agent should stop following the entry and ask a human. `record_learning` rejects a `procedural` entry with no `detail` at all; that the detail actually names a boundary is on you. No other kind needs this — a `pitfall` either bites or it doesn't.

Good: summary "Sync the repo before starting work." — detail "…Stop and ask a human when the sync reports a conflict you did not cause."

### Picking a scope

| Scope | Use when | `scope_ref` |
|-------|----------|-------------|
| `user` | Personal workflow preference, applies to all work | omit |
| `repo` | Codebase-wide convention or landscape entry | omit (auto-derived) |
| `epic` | Shared design decision for this epic only | omit (auto-derived) |
| `task` | One-off note; not auto-injected into future prompts | omit (auto-derived) |

**Default to `repo` for code conventions and `user` for workflow preferences.**

### Writing a good summary

- **One sentence only.** If you need two, the entry is too broad — split or drop it.
- **Be specific about the behaviour, not about the code.** Not "be careful with DB queries" but "a task update that clears a field and one that leaves it untouched are different requests, and the update API distinguishes them — passing an empty value is not the same as passing nothing."
- **Lead with the actionable insight.** What should a future agent do differently?
- **Name no function, type, macro, fixture, test, or file** — see "Do NOT record" above. The validator will not catch most of them.
