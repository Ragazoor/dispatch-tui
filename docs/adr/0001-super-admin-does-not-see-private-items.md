# 0001. A super-admin does not see private items

## Status

Superseded by `docs/plans/2026-11-23-remove-workspaces.md` (task 99362,
2026-11-23): one person uses the store, so there are no super-admins and no
private items left to withhold. Accepted 2026-11-13; the text below is kept as
it was.

## Context

`docs/specs/workspaces.allium` decision D3 says a private epic or work
package is seen by its holder and nobody else, super-admins included. A task
with no epic sits on its owner's user board, which is always private (D2).
An agent sees what its host's owner sees (D10).

It was suggested that a super-admin should see every task and epic in the
store. The options weighed were: keep D3; let a super-admin see that a
private item exists but not its title or contents; let a super-admin read
everything but write only their own; let a super-admin read and write
everything.

## Decision

Keep D3. A super-admin reads no private item that is not theirs: not its
title and not its contents.

## Consequences

- People can keep personal work (user boards, private epics and packages) in
  the shared store without their admins or their admins' agents reading it.
  Any of the other options would put every user-board task in front of every
  super-admin and their agents.
- A person who leaves takes their private items out of everyone's sight. The
  escape hatch stays: `SuperAdminDeletesUnseenItem` and the unseen-item
  listing (`docs/specs/admin-surfaces.allium`), which shows no titles.
- The per-caller views (`docs/architecture.md`, "Read enforcement: per-caller
  views") and the write rules (`WritesRequireSight`, `SetEpicVisibility`,
  `MoveEpicToWorkspace`) are unchanged. A super-admin's extra powers stay
  limited to the admin actions that D1 and the hatch name.
