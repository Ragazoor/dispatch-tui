# Release-please bot PRs as their own review tag (task 53281)

Decided with the user. Release PRs from the Kognic release bot
(`app/kognic-gha-release-bot`, login `kognic-gha-release-bot[bot]`, titles
"chore(master): release X.Y.Z") should appear on the board as tasks that an
agent can approve and merge.

1. `scripts/bots.conf` gets `RELEASE_BOT_AUTHORS`, validated exactly like
   `BOT_AUTHORS` (app form, fatal on a malformed entry, every offender named).
   Documents `app/kognic-gha-release-bot` as the example; shipped value empty.
   An entry on both `BOT_AUTHORS` and `RELEASE_BOT_AUTHORS` is fatal in both
   readers (`fetch-reviews.sh` and `fetch-dependabot.sh`): every overlapping
   entry named on stderr, non-zero exit, nothing emitted, like a malformed
   entry (feed-scripts.allium: "A bot on both lists is fatal").
2. `scripts/fetch-reviews.sh`'s bot-author pass queries `BOT_AUTHORS` plus
   `RELEASE_BOT_AUTHORS`. A PR authored by a `RELEASE_BOT_AUTHORS` bot gets
   tag `release-please`; it routes to the Bots sub-epic via `author_bot`. The
   tag is never derived from the `[bot]` suffix (learning #657). The
   `dependabot` tag logic is unchanged; `fetch-dependabot.sh` changes only to
   read `RELEASE_BOT_AUTHORS` for the overlap check in step 1.
3. New `TaskTag::ReleasePlease`: wire string `release-please`, short label
   `rel`, a review tag (`is_review` true). Update every site that lists
   `pr_review`/`dependabot`: core, board-layout, mcp-task-tools, store-split,
   dispatch-prompt and dispatch specs; `spacetime/module/src/session_request.rs`
   `REVIEW_TAGS` and its tests; MCP tool schema text; TUI pickers.
4. New runbook prompt for the tag: read the changelog in the PR and check CI.
   A major version bump or a breaking-change entry in the changelog stops the
   agent: it reports to the user and does not merge, whatever CI says. If
   CI is green (`ci:pass`), approve and merge. "Green" is the PR's live CI
   state read by the agent at decision time; the card's `ci:pass` label is not
   required. CI is the only merge gate: no `kognic-github-app` APPROVED verdict
   is needed, and the agent approves the PR itself. If CI failed, is pending
   or is absent, stop and report; do not merge. The agent never authors the
   PR. See dispatch-prompt.allium: `AReleasePrMergesOnlyOnGreenCi`,
   `ABreakingReleaseIsNeverMerged`.
5. Rebuild `module.wasm` (`./scripts/build-managed-module.sh`) and regenerate
   bindings only if needed.
