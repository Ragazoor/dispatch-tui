# Remove workspaces and the multi-user access model

Task 99362. Elicited with the operator on 2026-11-23.

## Goal

Dispatch assumes one person uses the store. Drop the machinery that makes the
store a backend shared between people who must be kept apart.

## Decisions

R1. Removed: Workspace, Membership, WorkspaceRole, the super-admin flag and its
    claim/grant/revoke, the admin screen (`A`), its MCP tools (workspace_*,
    workspace-member_set, super-admin_*, unseen-item_*), the placement picker's
    workspace step, and SuperAdminDeletesUnseenItem.
R2. Removed: public/private visibility of epics and work packages
    (Visibility, Epic.private_holder, is_effectively_private, set-visibility
    writes) and every sight check (visible_to, WritesRequireSight,
    UnseenIsAsMissing, ReParentingKeepsTheHolder, SightChangeNeverStrandsAHost,
    ClaimsFollowSight, LearningVisibility, setting visibility, archive-record
    visibility). No row is withheld from anyone for access reasons.
R3. Kept: UserIdentity, Host and Host.owner, host-scoped write checks
    (store-split D11), user boards (Task.owner), epic subscriptions, created_by
    stamps, UserProfile display names. One person may still use several
    machines against one store.
R4. The per-caller views stay as the DELIVERY path for follows and own rows
    (sync.allium's covered_epics, own_creations, own_subscriptions). The sight
    predicate is removed from them; they narrow by subscription and ownership
    only. That narrowing is delivery (what a board receives), not access: the
    tables stay private only because changing that is a schema change.
R5. A signalled new task (task_create signal, reviews feed) marks every known
    person: one UnseenTask per UserProfile.
R6. Managed feed roots stay one per creator (the workspace drops out of the
    uniqueness key). UniqueFeedRolePerParent and UniqueRepoGroupPerParent lose
    their workspace term.
R7. Person-to-person trust lines go: anyone may change an epic's feed_command
    (the setter record and FeedCommandRunsOnSettersHost claim drop stay);
    anyone may detach a task; all SavedRepoPath rows with one github_path share
    one verify command whatever their host owner.
R8. Old store data: the workspace, membership and super-admin columns and
    tables stay in the SpacetimeDB module as dead, never-written, never-read
    schema (automigration cannot drop them). No data move.
R9. AdoptOwnerlessUserBoardTasks goes with ClaimSuperAdmin. Checked on
    2026-11-23: the live store has 49 epic-less tasks and none lacks an owner.
R10. Survivors of workspaces.allium move elsewhere, not out: UserProfile and
    the display-name rules (RegisterUserProfile, SetDisplayName and its board
    and MCP twins), caller stamping of owner and created_by,
    CreatorIsShownByName, and the origin-patch check of RepoGroupsStayPublic.
R11. Snapshot and seed: workspace, membership and super-admin data in an old
    snapshot is ignored on restore (dead schema, as R8). Learning.workspace,
    Epic.workspace/visibility, WorkPackage.workspace/visibility and
    UserProfile.is_super_admin stay as dead columns; new rows write their
    defaults and no reader looks at them.
