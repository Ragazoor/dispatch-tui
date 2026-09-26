# Is there a better language for the dispatch TUI? (task #4961, epic #314)

Decision note. Answers the question in task #4961: would a different
implementation language fit the dispatch TUI's purpose better than Rust, in
particular given the complaint that the board relies on polling loops instead
of state reacting to updates? Follows the same "decision, not a spike" shape
as [`4931-claude-memory-vs-knowledge-base.md`](4931-claude-memory-vs-knowledge-base.md).

## Decision

**Keep Rust + ratatui. Do not migrate the TUI to another language.** No code
change follows from this task. The polling complaint that motivated it is
real, but it is a data-transport choice already being replaced —
project-tracked in epic #325, the SpacetimeDB migration — not something a
different language would fix, and every alternative language loses more than
it gains once the actual cost is priced in.

## Evaluation criteria

Set before surveying any candidate, so the survey answers a fixed question
rather than backing into whichever language felt appealing:

1. **Reactive state-update model.** Does the language/framework let a data
   change push a UI update, without a hand-rolled tick loop re-reading state
   on a timer? This is the property the task's description names directly.
2. **Native access to the shared store.** Dispatch's board is migrating to
   SpacetimeDB as the authoritative shared store (epic #325), reached via
   subscriptions rather than polling. A candidate needs a client SDK for that
   store, or it re-introduces exactly the poll-and-glue problem it was meant
   to escape, just moved into an IPC bridge to a Rust sidecar.
3. **Single-binary distribution.** Dispatch installs on a developer's machine
   and re-execs itself inside tmux (`docs/reference.md`, "Running & Debugging
   Locally"). A runtime dependency (Node, Python, a VM) that must be present
   and version-matched on every machine is a real cost this doesn't pay today.
4. **Concurrency for background I/O.** The TUI shells out to git/tmux/gh,
   runs an MCP HTTP server, and drains async command queues concurrently with
   rendering (`docs/architecture.md`, "Command queue draining"). The
   language needs real concurrency primitives, not just an event loop that a
   blocking subprocess call can stall.
5. **Type-safety for illegal-states-unrepresentable modeling.** The codebase
   leans on this deliberately — `TaskPatch`'s double-`Option`, exhaustive
   enum matches, the `patch_struct!`/`service_api!` macro families (see
   `docs/conventions.md`). A weaker type system regresses a pattern the repo
   has already invested in and validated (learning #790).
6. **Ecosystem fit for a data-dense, full-screen app.** Kanban columns,
   popups, scrollable panes, snapshot-style widget testing — not a
   line-oriented CLI with an occasional prompt.
7. **Migration cost against sunk investment.** The current TUI is a large,
   working Rust codebase with ~40 Allium specs, a macro-generated MCP tool
   registry, and a SpacetimeDB Rust SDK integration that is most of the way
   landed (epic #325, phases 0–7 done as of this task). A rewrite pays this
   cost once, up front, for whatever the new language buys.

## Candidates surveyed

| Language / framework | #1 Reactive | #2 Store SDK | #3 Single binary | #7 Rewrite cost |
|---|---|---|---|---|
| **Rust + ratatui** (current) | Elm-style Message→Command, hand-built (already true) | Yes — native SDK, phases 0–7 landed | Yes | none — already paid |
| Go + Bubble Tea | Yes, natively (The Elm Architecture) | **No SDK** | Yes | Full rewrite |
| TypeScript + Ink / OpenTUI | Yes, React reconciliation | Yes — TS SDK exists | No — needs Node/Bun runtime | Full rewrite |
| Python + Textual | Yes, natively (`reactive` attributes) | **No** (SDK unmaintained) | No — heavy to freeze into one binary | Full rewrite |
| C# + Terminal.Gui | Partial — no built-in reactive layer | Yes — C# SDK exists | Yes, with native AOT | Full rewrite |
| Elixir + Ratatouille | Yes, natively (BEAM actor model) | **No** | No — needs BEAM runtime | Full rewrite; immature TUI lib |

Sources for the framework comparison: SpacetimeDB's own
[language support page](https://spacetimedb.com/docs/intro/language-support/)
(client SDKs: Rust, TypeScript, C#, C++, Unity, Unreal; the Python SDK is
unmaintained; no Go SDK exists at all), and a September 2026 cross-framework
comparison covering [Bubble Tea, ratatui and
Textual](https://www.glukhov.org/developer-tools/comparisons/tui-frameworks-bubbletea-go-vs-ratatui-rust/)
plus a broader [Ink / OpenTUI / Bubble Tea / ratatui / Textual
guide](https://labhub.hopto.org/blog/2026-07-31-terminal-ui-development-guide?lang=en).

Every alternative fails criterion #7 outright — none of them, including the
ones that clear every other bar, offer enough upside to justify discarding a
working, spec-covered Rust codebase and an already-landed SpacetimeDB
integration. Beyond that:

- **Go + Bubble Tea** is the framework most often recommended for "simpler
  TUI," and its Elm Architecture is a cleaner starting point than dispatch's
  hand-rolled Message→Command layer. But there is no SpacetimeDB SDK for Go.
  Moving to Go would mean re-introducing a polling bridge to a Rust sidecar
  process to reach the shared store — the opposite of what this task is
  asking for.
- **TypeScript + Ink/OpenTUI** is the only alternative that keeps native
  access to the shared store (a TypeScript SDK exists) and gets a reactive
  model for free from React's reconciliation. It fails on distribution (a
  Node/Bun runtime becomes a hard dependency where today there is none) and
  on ecosystem fit — the September 2026 comparison places Ink's sweet spot as
  "adding UI to an existing Node CLI," not a full-screen, data-dense board;
  OpenTUI is newer and less proven for that shape of app.
- **Python + Textual** has the most explicitly reactive framework of the
  group (its state primitive is literally called `reactive`), and the
  ecosystem fit for data tools is good. But its SpacetimeDB SDK is
  unmaintained, and freezing a Python app into one distributable binary is
  friction this project doesn't have today.
- **C# + Terminal.Gui** and **Elixir + Ratatouille** were considered and
  dropped faster: C# has SDK access and can produce a native AOT binary, but
  its TUI ecosystem and reactive story are both weaker than the above, with
  no matching upside. Elixir's BEAM actor model is the most naturally
  "reactive" fit of anything surveyed, but it has no SpacetimeDB SDK, no
  single-binary story, and its TUI library (Ratatouille) is far less mature
  than any of the above.

## Why the polling complaint is not a language problem

The task's description names the actual pain precisely: "a lot of polling
states, instead of states reacting to updates across the state." Two
different things sit under that description, and only one of them is real:

- **The TUI's own render/update loop already reacts to updates.** Input
  handling and message routing follow the Elm-style Message → Command
  pattern (`docs/architecture.md`); redraws are driven by a dirty flag set on
  state-changing events, not a timer (see "Render dirty flag (fail-open)" in
  the same file). This part of the architecture is what Bubble Tea and
  Textual are praised for, and dispatch already has it, in Rust.
- **The data layer polls.** `exec_refresh_from_db` fires a speculative
  refresh every 5 ticks, and today every board reads through SQLite via
  `board_reads` rather than a subscription (`docs/module-map.md`, "The
  board's read source is a seam"). This is the actual polling the task
  reacts to, and it is a data-transport decision, not a UI-framework one.
  It already has a name, a design, and most of its implementation: epic
  #325's SpacetimeDB migration replaces exactly this with subscriptions that
  push rows to the board unasked (`docs/specs/sync.allium`, rules
  `BoardReadsFromTheSubscription` and `SubscribedRowsArriveUnasked`). Phases
  0 through 7 are landed; `docs/specs/sync.allium` itself carries the open
  question of whether a periodic refresh is still worth keeping as a
  backstop once subscriptions are the default — the same open question a
  language migration would not touch.

Rewriting the TUI in a different language would not shorten the path to
fixing this; the fix is already mid-flight in the same language the rewrite
would throw away.

## What was considered and rejected

- **Full migration to Go/Bubble Tea, TypeScript/Ink, Python/Textual,
  C#/Terminal.Gui, or Elixir/Ratatouille** — covered above; every one fails
  the sunk-cost criterion, and the two most commonly recommended
  alternatives (Go, Python) additionally lose native access to the shared
  store dispatch is migrating onto.
- **A hybrid split (Rust backend/store, alternative-language TUI talking to
  it over IPC)** — considered briefly. This reintroduces a polling or
  message-bridge layer between the two processes, which is the exact
  architecture epic #325 is removing on the Rust-only path. Not worth the
  operational complexity of running two processes for one desktop TUI.

## References

- [`docs/architecture.md`](../architecture.md) — Message→Command pattern,
  render dirty flag, command queue draining
- [`docs/module-map.md`](../module-map.md) — "The board's read source is a
  seam, not the database handle"
- [`docs/specs/sync.allium`](../specs/sync.allium) — `BoardReadsFromTheSubscription`,
  `SubscribedRowsArriveUnasked`, and the open question on periodic-refresh
  backstops
- [`docs/plans/2026-09-17-spacetimedb-migration-plan.md`](../plans/2026-09-17-spacetimedb-migration-plan.md) —
  epic #325's phased migration off polling
- [`4931-claude-memory-vs-knowledge-base.md`](4931-claude-memory-vs-knowledge-base.md) —
  prior decision note this one follows the shape of
- [SpacetimeDB language support](https://spacetimedb.com/docs/intro/language-support/)
- [Bubble Tea vs. ratatui comparison, Rost Glukhov](https://www.glukhov.org/developer-tools/comparisons/tui-frameworks-bubbletea-go-vs-ratatui-rust/)
- [Terminal UI development guide — Ink, OpenTUI, Bubble Tea, ratatui, Textual](https://labhub.hopto.org/blog/2026-07-31-terminal-ui-development-guide?lang=en)
