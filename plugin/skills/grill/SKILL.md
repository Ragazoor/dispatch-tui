---
name: grill
description: >-
  Interview the user relentlessly about a plan or design, then capture decisions
  in Allium specs and ADRs. Use when the user wants to stress-test an idea before
  building — "grill me on this", "poke holes in this", "interrogate this design",
  "challenge my assumptions", "what am I missing".
---

# Grill

**Announce at start:** "I'm using the grill skill to interview you about this."

The goal: reach a shared understanding of the plan, with the decisions written
down where the next agent will find them.

## Interview

Walk the design tree, resolving each decision before the ones that depend on
it. Ask one question at a time and wait for the answer. Give your recommended
answer with each question. If the codebase can answer a question, read it
instead of asking.

Use `docs/specs/*.allium` as the shared vocabulary. Name a term that is fuzzy
or used two ways, and say when a decision contradicts or extends a spec. After
a decision, probe its edges: nulls, concurrent agents, the failure path.

## Capture

- **New or changed domain concepts** (entity, enum, rule, canonical term) go
  into the relevant spec through the `allium:tend` skill. Record only what was
  decided.
- **An ADR** in `docs/adr/NNNN-slug.md` (next free number; create the directory
  if needed) when a decision is hard to reverse, surprising without context,
  and had real alternatives. Skip obvious choices, anything the spec already
  captures, and implementation details.

Keep an ADR to a few short paragraphs under `Status`, `Context`, `Decision`
and `Consequences`.
