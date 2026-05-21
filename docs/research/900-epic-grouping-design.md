# Epic Grouping Redesign

**Date:** 2026-05-21
**Task:** #900

## Problem

Epic cards drift across kanban columns as their child tasks progress (backlog → running → review → done). Users expect epics to stay in a predictable place but cannot reliably find them because the derived status mirrors child task states in real time.

## Design

### Core rule change

Replace the multi-state `recalculate_epic_status` derivation with a binary rule:

| Condition | Derived status |
|-----------|---------------|
| No active children | backlog |
| All active children done | done |
| Otherwise (any child in backlog, running, or review) | backlog |

The `running` and `review` intermediate states are no longer auto-reachable via recalculation. An epic with tasks in any mix of backlog/running/review always stays in the backlog column.

### Automatic transitions

Two automatic transitions remain:

- **→ done**: fires when all active children become done
- **→ backlog**: fires when a task is added to a done epic, or when a done task is reopened

Both are driven by the existing `recalculate_epic_status` call sites (task creation, task status change, sub-epic status propagation). No new trigger points are needed.

### Manual moves

`[` and `]` continue to traverse backlog → running → review → done manually. Manual moves can be overridden by the next recalc trigger (a task state change). This is intentional: recalc always wins.

### Card display

`EpicSubstatus` (active, N blocked, in review, wrapping up, etc.) is derived independently of column placement and continues to appear on the card's second line alongside the `●N` task-count indicators. The card still communicates the internal state of the epic's tasks; it just no longer moves columns to express it.

### What is not affected

- `EpicSubstatus` derivation logic
- `MoveEpicForward` / `MoveEpicBackward` implementation (manual moves still traverse all four statuses)
- Flat view, feed epics, sub-epics, epic detail view
- MCP `update_epic` tool — explicit `status` values including running/review remain valid

## Implementation scope

1. **`src/service/epics.rs`** — replace 5-branch derivation in `recalculate_epic_status` with 3-branch binary rule
2. **`docs/specs/epics.allium`** — update `EpicStatusRecalculation` invariant to reflect new rule
3. **Tests** — update service and TUI tests asserting auto-move to running/review; add tests for "stays in backlog while tasks run" and "returns to backlog from done when task added/reopened"
4. **Snapshot tests** — update any snapshot showing an epic card in running/review via auto-recalc
