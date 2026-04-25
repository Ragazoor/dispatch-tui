# Unified Learning entity, scope model, auto-memory reconciliation (task #341, epic #27)

Design spike. Pins the v1 design for epic #27 ("Build a unified learning loop covering every aspect of development") before any code lands. Inputs: [`329-self-learning-frameworks.md`](329-self-learning-frameworks.md). Output: this doc — downstream work packages (#2–#11 in §5 of the input note) reference it.

This document pins five decisions called out in task #341:

1. SQLite schema for the unified `Learning` entity.
2. Scope-resolution rules at retrieval time.
3. Markdown-export shape and direction.
4. Coexistence with the user auto-memory at `~/.claude/projects/<project>/memory/`.
5. Privacy / secrets handling.

It deliberately leaves implementation, MCP wire shape, and TUI surface to downstream WPs; this doc only fixes the data and policy decisions those WPs build on.

## 1. Context & goals

Today every dispatched agent in this repo starts cold. The research note (#329) surveyed nine frameworks (mozilla-ai/cq, Anthropic's memory tool, Cline Memory Bank, Cursor rules, Aider, MemGPT/Letta, Mem0, LangMem, Graphiti/Zep) and recommends a dispatch-native design that:

- borrows cq's "knowledge unit" data shape,
- borrows Cline's storage simplicity (SQLite + optional markdown, no vector store, no graph),
- borrows LangMem's taxonomy (semantic / episodic / procedural) but expresses all three as one table with a `scope` field rather than three stores,
- defers vector retrieval, embedding ranking, organisational sharing, and automatic prompt mutation.

This doc fixes the v1 contract. The goal is that after these decisions land, WP #2 (Allium spec additions) and WP #3 (DB migration + service layer) can proceed without re-litigating any of the points below.

## 2. The `Learning` entity (v1 SQLite schema)

One table, `learnings`, in dispatch's existing SQLite database (same DB as `tasks`, `epics`, etc.). Columns:

| Column | Type | Notes |
|---|---|---|
| `id` | INTEGER PRIMARY KEY | autoincrement |
| `kind` | TEXT NOT NULL | enum `pitfall \| convention \| preference \| tool_recommendation \| procedural \| episodic` |
| `summary` | TEXT NOT NULL | one line, prompt-injectable, ≤ 240 chars (soft cap, validated in service layer) |
| `detail` | TEXT | optional longer body, surfaced on demand or in TUI review |
| `scope_kind` | TEXT NOT NULL | enum `user \| repo \| epic \| task` |
| `scope_id` | TEXT NULL | repo path string for `repo`; epic id (string) for `epic`; task id (string) for `task`; NULL for `user` |
| `tags` | TEXT NOT NULL DEFAULT '[]' | JSON array of free-form lowercase tag strings |
| `evidence_created_at` | TIMESTAMP NOT NULL | row insertion time |
| `evidence_last_confirmed_at` | TIMESTAMP NOT NULL | bumped by `confirm_learning` |
| `evidence_confirmed_count` | INTEGER NOT NULL DEFAULT 1 | starts at 1 (the proposing agent), bumped by `confirm_learning` |
| `source_task_id` | INTEGER NULL | FK to `tasks(id)` ON DELETE SET NULL — the task whose dispatched agent proposed this row, when applicable |
| `status` | TEXT NOT NULL DEFAULT 'proposed' | enum `proposed \| approved \| rejected \| archived` |

Constraints:

- `CHECK (kind IN (...))`, `CHECK (scope_kind IN (...))`, `CHECK (status IN (...))` — enforce enums at the DB layer.
- `CHECK ((scope_kind = 'user') = (scope_id IS NULL))` — `user` scope must have NULL `scope_id`; all other scopes must have a non-NULL `scope_id`.
- No FK on `scope_id` for `repo`/`epic`/`task` (repo is just a path string today, and decoupling lets us archive a task without losing learnings derived from it).

Indexes:

- `(scope_kind, scope_id)` — primary retrieval path.
- `(status)` — fast filter for `proposed` rows in the TUI review surface.
- `(kind)` — useful for TUI filtering and for the prompt-augmentation step that excludes `procedural` from agent-visible queries until human-approved.

### 2.1 Departures from cq's knowledge-unit schema

cq's [knowledge unit](https://github.com/mozilla-ai/cq/blob/main/docs/architecture.md) carries richer provenance: contributor DID, audit trail, organisational diversity (how many distinct orgs have confirmed), and lifecycle relationships between units. v1 collapses all of that to `source_task_id` and `evidence_confirmed_count`. Reasons:

- dispatch is single-developer; "organisational diversity" is undefined here,
- DIDs require a key infrastructure that does not exist,
- relationships between learnings can be added later as a join table without breaking the core schema.

The MCP tool surface (WP #4) should still be **shape-compatible with cq's six tools** (`query`, `propose`, `confirm`, `flag`, `reflect`, `status`) so that a future bridge to a cq remote tier is cheap if cross-developer sharing ever becomes a goal.

## 3. Scope resolution rules at retrieval time

A `query_learnings` call is always issued **on behalf of a specific dispatching task**. The query anchor is the tuple

```
(user, repo_path, epic_id?, task_id?)
```

derived from that task. The retrieval predicate unions four scope cases:

```
(scope_kind = 'user')
  OR (scope_kind = 'repo'  AND scope_id = :repo_path)
  OR (scope_kind = 'epic'  AND scope_id = :epic_id)
  OR (scope_kind = 'task'  AND scope_id IN :sibling_task_ids)
```

where `:sibling_task_ids` is the set of task ids in the same epic as the calling task (or the calling task's own id if it has no epic).

Rules:

1. **Hard scope leakage rule.** A row with `scope_kind = 'repo' AND scope_id != :repo_path` MUST NOT appear in the result set. The same applies to `epic` and `task`. The retrieval query is the only enforcement point — there is no second layer of filtering, so the SQL must be correct on its own. Tests in WP #3 must cover this case explicitly.

2. **Status filter for prompt augmentation.** When the result set will be injected into a dispatched agent's prompt (WP #6), only `status = 'approved'` rows are returned. When the call is from the TUI review surface (WP #7), `status IN ('proposed', 'approved')` is permitted. The MCP tool exposes a `statuses` parameter (default `['approved']`) rather than baking the policy in.

3. **`kind = 'procedural'` is never agent-visible.** Procedural learnings affect the dispatch prompt prefix itself; making them visible to the agent that proposed them creates a feedback loop where one agent's poisoned proposal gets re-confirmed on the next run. WP #6 reads `kind = 'procedural' AND status = 'approved'` only at prompt-construction time; `query_learnings` filters this kind out by default unless the caller is the prompt-construction code path.

4. **Ranking inside the union.** Top-N is required for prompt budgeting. v1 ranks by, in order:
   1. exact-scope match — `task` > `epic` > `repo` > `user`,
   2. tag overlap with the calling task's tags (Jaccard on the tag arrays),
   3. recency — newer `evidence_last_confirmed_at` wins,
   4. evidence count as a tiebreaker.

   Weights are deliberately fixed integers in v1; tuning is an open question for WP #4 (see §8).

5. **Default `limit`.** v1 default is `limit = 10` for prompt augmentation, no default for the TUI surface.

## 4. Markdown export shape and direction

**Direction: one-way export only.** SQLite is the source of truth. Optional markdown files at `.dispatch/learnings/` inside each repo are regenerated from the DB. Humans review and edit through the TUI, never by editing markdown.

This is decision (3) from task #341. The alternative (bidirectional sync) was considered and rejected: it requires conflict rules, a file watcher or sync command, and creates a class of race conditions where a hand-edited markdown row goes stale relative to the DB. The cost is that humans lose the ability to bulk-edit in `$EDITOR`; the upside is that the DB is unambiguously canonical and there is no merge layer to debug.

### 4.1 File layout

```
.dispatch/learnings/
  user.md                    # all rows where scope_kind = 'user'
  repo.md                    # rows where scope_kind = 'repo' AND scope_id = <this repo's path>
  epic-<epic-id>.md          # one file per epic with at least one row
  task.md                    # all task-scoped rows belonging to tasks in this repo
  README.md                  # static, explains the export and "do not edit" warning
```

Each entry in a file is rendered as:

```
---
id: 42
kind: convention
status: approved
tags: [rust, testing]
created_at: 2026-04-25T17:30:00Z
last_confirmed_at: 2026-04-26T09:12:00Z
confirmed_count: 3
source_task_id: 313
---

**Summary:** Always run `cargo fmt --check` before pushing.

Detail goes here. Multi-paragraph markdown is fine.

---
```

The first line of the file is `<!-- generated by dispatch — do not edit; changes will be overwritten -->`.

### 4.2 Regeneration triggers

- after `record_learning` (proposed rows are exported too, with `status: proposed` so they are visible in code review of `.dispatch/`),
- after `confirm_learning`,
- after a TUI status change (approve / reject / archive),
- on demand via a CLI subcommand: `dispatch learnings export [--repo <path>]`.

### 4.3 Out of scope for v1

- bidirectional sync from markdown back to DB,
- watching `.dispatch/learnings/` for human edits,
- per-tag or per-kind file splitting beyond the per-scope split above,
- export to formats other than markdown.

## 5. User auto-memory coexistence

**Direction: one-way read-only import at session start.**

The user already has a working `scope=user` store: Claude Code's auto-memory at `~/.claude/projects/<project>/memory/MEMORY.md` plus the per-topic files it indexes. This file is maintained by Claude during normal (non-dispatch) sessions. Dispatch should consume it, never write to it, and never duplicate its contents into the `learnings` table.

### 5.1 Mechanism

When `query_learnings` resolves with `scope_kind = 'user'` in the union, the service layer additionally:

1. resolves the auto-memory directory for the current user (`$XDG_CONFIG_HOME/.claude/projects/<project>/memory/` or the platform equivalent),
2. parses `MEMORY.md` as the index (one bullet per memory file with title and one-line hook),
3. for each indexed file, lazily reads the title, description, and body,
4. emits each entry as a **virtual** result row with `scope_kind = 'user'`, `status = 'approved'`, `kind` inferred from the auto-memory `type:` frontmatter (`user → preference`, `feedback → preference`, `project → episodic`, `reference → tool_recommendation`), `source_task_id = NULL`, and a sentinel marker `source = 'auto_memory'` exposed only in the response shape (not stored).

The virtual rows participate in ranking exactly like real `scope = 'user'` rows.

### 5.2 Deduplication

When a virtual auto-memory row's `summary` overlaps a real `scope = 'user'` row's `summary`, the real row wins and the virtual row is suppressed. Overlap rule for v1:

- exact match (case-insensitive) on the trimmed summary, OR
- token Jaccard ≥ 0.9 on whitespace-split, lowercased tokens after stripping punctuation.

The cheap path (case-insensitive equality) handles 95% of duplicates. The Jaccard tier catches near-duplicates phrased slightly differently. Embedding-based dedup is out of scope.

### 5.3 Failure handling

- auto-memory directory missing → no virtual rows, no error,
- auto-memory file unreadable / malformed → log a debug-level warning, skip that file, continue,
- auto-memory directory points outside the user's home (`..` traversal) → refuse to read; this is a defensive check against a hostile project name.

### 5.4 Why one-way import (rationale)

Three options were considered:

| Option | Verdict |
|---|---|
| Subsume — make dispatch's `scope=user` canonical, stop using auto-memory in dispatched sessions, migrate existing entries once. | Rejected. Auto-memory continues to work for non-dispatch Claude Code sessions; subsuming requires intercepting Claude's own memory hook (brittle). |
| Ignore — dispatch user-scope is independent of auto-memory. | Rejected. Same fact stored twice, agents see duplicates, the user has to teach two systems. |
| **One-way import** — read auto-memory at session start, never write, dedup at retrieval time. | **Selected.** Read-only on a known path, predictable, lets dispatch own ranking, preserves auto-memory's role for non-dispatch sessions. |

### 5.5 Out of scope for v1

- importing auto-memory rows into the SQLite store on first run,
- writing back to auto-memory from dispatch,
- deprecating auto-memory,
- using auto-memory's full conversational metadata (we only consume the index + body).

## 6. Privacy / secrets handling

**Direction: regex-based redaction at `record_learning` time, plus the human-review gate.**

Two layers of defence:

1. **Layer 1 — redaction at record time.** Before insertion, `record_learning` runs a redactor over `summary` and `detail`. Matches are replaced with `[REDACTED:<kind>]`. The pre-redaction text is never stored.
2. **Layer 2 — human review gate.** A row with `status = 'proposed'` does not surface in any future agent's prompt (see §3 rule 2). It is visible only in the TUI review surface, where the user approves, edits, or rejects.

### 6.1 v1 redaction patterns

Compiled-in for v1 (see §8 open question (c) about making them user-configurable). The exact regex set:

| Kind | Pattern |
|---|---|
| API key / token / secret (generic) | `(?i)(api[_-]?key\|token\|secret\|password\|passwd)["':= ]+[A-Za-z0-9_\-]{16,}` |
| AWS access key id | `\bAKIA[0-9A-Z]{16}\b` |
| JWT | `\beyJ[A-Za-z0-9_=-]+\.[A-Za-z0-9_=-]+\.[A-Za-z0-9_.+/=-]*` |
| Private key block | `-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----` |
| GitHub fine-grained PAT | `\bghp_[A-Za-z0-9]{36}\b`, `\bgithub_pat_[A-Za-z0-9_]{82}\b` |
| Email address | `\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b` |
| Absolute home path | `/home/[^/\s]+/\S*` (Linux) and the macOS equivalent `/Users/[^/\s]+/\S*` |

The redactor returns the redacted text plus a list of `(kind, count)` pairs so the service layer can emit a structured warning to the agent ("we redacted 1 token and 2 emails from your proposal"). This nudges the agent to rephrase if a redaction destroyed the meaning.

### 6.2 Out of scope for v1

- model-based PII detection,
- redaction of secret values pasted into `tags`,
- redaction of `summary`/`detail` of rows imported from auto-memory (auto-memory is treated as already trusted; if the user wants redaction there too, it's a follow-up),
- per-organisation policy / configurable patterns.

## 7. Out of scope for v1 (explicit list)

Each item below is a deliberate "not now" — flagged so reviewers see what was left out:

- cross-repo / organisational sharing (cq's "remote tier"),
- vector or embedding-based retrieval, FTS5, or semantic ranking,
- automatic prompt-prefix mutation without human review,
- two-way markdown sync (see §4),
- model-based PII / secret detection (see §6),
- importing auto-memory into the DB (see §5),
- a join table for relationships between learnings,
- contributor identity / DIDs / audit trails beyond `source_task_id`,
- multi-user concurrent edits in the TUI review surface.

## 8. Open questions for downstream WPs

Questions to revisit when the relevant WP starts; this doc does not pin them.

- **(a) Ranking weights** — §3 rule 4 fixes the order but not the relative weights. WP #4 (`query_learnings` MCP tool) should validate the ordering produces sensible top-N on real data and tune from there.
- **(b) Is `scope = 'task'` needed in v1?** §3 includes it for completeness, but the research note (#329 §3.2 item 4) recommends episodic data flow through a sibling MCP tool that reads from existing `tasks` rows directly rather than duplicating into `learnings`. WP #5 should decide whether `scope = 'task'` rows are ever produced or whether the DB column exists but stays empty in v1.
- **(c) Where do redaction patterns live?** Compiled-in is fine for v1 (§6) but a config file (`~/.config/dispatch/redaction.toml` or similar) is the obvious next step; WP #3 should leave a clean seam for it.
- **(d) Approval-flow UI shape** — WP #7 designs the TUI review surface. Open: is approval per-row or per-batch? Is rejection silent or does it notify the proposing agent?
- **(e) When does `confirm_learning` fire automatically?** A learning whose `summary` was used in a successful dispatch is implicitly confirmed; v1 may keep this manual, but WP #4 / WP #6 should decide.

## 9. References

- [`docs/research/329-self-learning-frameworks.md`](329-self-learning-frameworks.md) — input research note.
- mozilla-ai/cq architecture — [docs](https://github.com/mozilla-ai/cq/blob/main/docs/architecture.md). Schema borrowed and simplified in §2.
- Cline Memory Bank — [docs](https://docs.cline.bot/features/memory-bank). Storage simplicity (markdown-first, no DB) inverted here: dispatch puts the DB first and exports markdown.
- LangMem conceptual guide — [docs](https://langchain-ai.github.io/langmem/concepts/conceptual_guide/). Semantic / episodic / procedural taxonomy is collapsed into the `kind` column in §2.
- Anthropic memory tool — [docs](https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool). Auto-memory's on-disk shape (§5) is what dispatch reads at session start.
