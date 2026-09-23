# Claude Code memory vs. the dispatch knowledge base (task #4931, epic #314)

Decision note. Answers the question in task #4931: should dispatch lean into Claude
Code's built-in memory feature instead of maintaining its own knowledge base
(`docs/specs/learnings.allium`)? Follows on from the WP1 spike
([`341-unified-learning-design.md`](341-unified-learning-design.md)), which
deferred exactly this question to "coexistence" and never built the read-only
import it proposed.

## Decision

**Keep the dispatch-native knowledge base as the system of record. Do not
replace it with Claude Code's memory feature.** No code change follows from
this task; it closes the open question rather than opening new work.

## What Claude Code's memory feature actually does here

Verified empirically in this session rather than assumed from docs:

- It is a folder of markdown files (`MEMORY.md` plus per-topic files) that the
  running Claude Code session reads and writes directly, gated by a system
  prompt describing four memory types (user/feedback/project/reference).
- The storage location keys off the **git checkout**, not the literal working
  directory. Every dispatch task gets its own worktree
  (`.worktrees/<id>-<slug>/`), each a distinct directory — but this session,
  running from a worktree, was pointed at the **parent repo's** memory folder
  (`~/.claude/projects/-home-ragge-Code-work-experiments-dispatch/memory/`),
  not a worktree-local one. Checking `~/.claude/projects/` confirmed no
  dispatch worktree directory has ever had its own `memory/` subfolder — only
  the main checkout does. So Claude's memory naturally collapses across every
  worktree of one repo onto a single shared store. That is closer to
  dispatch's `repo` scope than the "one silo per session" assumption in the
  341 spike.
- It has no equivalent of dispatch's `epic` scope (all of one repo's memory is
  one flat pool) or `task` scope, no ranked/embedding retrieval (the whole
  index is read every time, not top-N by similarity), no upvote/downvote
  signal, and no automatic staleness cleanup — pruning is "notice it's stale,
  edit the file," done by whichever agent happens to look.
- It is plain files with no locking or transaction boundary. Dispatch's own
  store lives in SQLite behind a single serialized writer connection
  (`src/db/mod.rs`).

## Why the custom knowledge base stays

1. **Concurrency.** Dispatch's premise is several agents running at once, in
   separate worktrees, potentially against the same repo. The knowledge base
   already handles that safely (SQLite, single writer). Claude's memory is
   unlocked markdown files that just happened to collapse onto one shared
   folder — two dispatched agents writing to it at the same time is a real
   race, not a hypothetical one.
2. **Scope granularity.** `epic` scope exists specifically so a large repo
   with many unrelated concurrent epics doesn't pollute every dispatch prompt
   with irrelevant entries (see `learnings.allium` §Scope Model). Claude's
   memory has no equivalent — everything in a repo's memory folder is visible
   to every task in that repo, epic-relevant or not.
3. **Ranking and pruning are load-bearing.** `query_learnings`' cosine
   threshold, scope multiplier, and upvote boost (see `learnings.allium`
   §MCP Tools) plus `ArchiveStaleLearning` keep the injected block small and
   relevant as the store grows. Claude's memory has no retrieval ranking and
   no automated pruning; it degrades as the file grows, curated only by
   whichever agent happens to notice.
4. **Agent portability.** Dispatch's knowledge base is reached over MCP tools
   any agent can call. Claude's memory tool is a Claude Code-specific feature
   with its own system-prompt contract; an agent running under a different
   harness has no access to it at all.
5. **Host coupling.** Memory files live under the operating user's `$HOME` on
   whichever machine ran the session. Dispatch's recent work
   (`docs/specs/sync.allium`, the SpacetimeDB phases) is moving toward a
   shared, multi-host store; a per-machine filesystem store pulls the other
   way.

## What was considered and rejected

- **Full replacement** — covered above; rejected on concurrency, scope
  granularity, ranking/pruning, portability and host-coupling grounds.
- **Read-only import of Claude's memory as extra `user`-scope entries**
  (the 341 spike's proposal, §5) — considered again here and still not worth
  building. The user-scope preferences it would surface
  (`~/.claude/rules/*.md`, this repo's `CLAUDE.md`) are already read directly
  by every agent as ordinary context files; a second import path through the
  knowledge base would duplicate them without adding retrieval value the
  agent doesn't already have.

## References

- [`341-unified-learning-design.md`](341-unified-learning-design.md) — original
  coexistence spike; §5 is superseded by this note's finding that memory
  collapses to the repo checkout, and its read-only import proposal is
  declined here rather than merely deferred.
- [`329-self-learning-frameworks.md`](329-self-learning-frameworks.md) — survey
  that this note narrows to the one question task #4931 asked.
- `docs/specs/learnings.allium` — current knowledge base spec (scope model,
  MCP tools, ranking, stale cleanup).
