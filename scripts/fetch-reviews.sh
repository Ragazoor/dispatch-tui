#!/usr/bin/env bash
# fetch-reviews.sh — outputs every open PR you are involved with as a single,
# deduped FeedItem JSON array, for use as a dispatch feed_command.
#
# Prerequisites: gh CLI (https://cli.github.com/) and jq must be in PATH.
#
# Setup:
#   1. Copy this file to scripts/local/fetch-reviews.sh
#   2. Set ORGS below to your list of GitHub organisation slugs.
#   3. Point the parent "Reviews" epic's feed_command at the local copy. There
#      is NO scope argument — the dispatch role router (feed_role =
#      reviews_parent) splits the single emission into My / Team / Bots
#      sub-epics using the per-PR `signals` this script attaches.
#
# What it emits:
#   ONE FeedItem array covering the union of these open-PR searches, each PR
#   tagged with the signal(s) that matched it:
#     - review-requested:@me        -> signal "team-request" (direct + team)
#     - user-review-requested:@me   -> signal "direct-request" (direct only)
#     - reviewed-by:@me             -> signal "reviewed"
#     - commenter:@me -author:@me   -> signal "commented" (excludes your own PRs)
#   Plus per-PR author signals: "author-bot" when the author login ends in
#   "[bot]" (Renovate/Dependabot), "author-me" when the author is the gh user.
#
#   A PR matched by several searches appears ONCE, with its signals merged
#   (unioned) — the dedup groups by URL and unions the signal arrays.
#
#   Bot-authored PRs are INCLUDED (Renovate/Dependabot are no longer excluded);
#   they get tag "dependabot". Human-review PRs get tag "pr-review". Drafts are
#   excluded.
#
# Output format (FeedItem):
#   [{"external_id":"review:org/repo#42","title":"#42 PR title","description":"...","url":"...","status":"backlog","tag":"pr-review","labels":["@author","repo"],"signals":["team-request","reviewed"]}]
#
# Routing is handled by dispatch, not here. The signal vocabulary is the wire
# contract with the role router (see docs/specs/feeds.allium, enum Signal).

set -euo pipefail

# ---------------------------------------------------------------------------
# Organisations to search. Fill in your org slugs, e.g.:
#   ORGS=("myorg" "another-org")
ORGS=()
# ---------------------------------------------------------------------------

if [[ ${#ORGS[@]} -eq 0 ]]; then
  echo "[]"
  exit 0
fi

owner_flags=()
for org in "${ORGS[@]}"; do
  owner_flags+=(--owner "$org")
done

# The gh user's login, for the author-me signal. Soft-fails to empty so a
# transient `gh api` error degrades author-me detection rather than the feed.
ME="$(gh api user -q .login 2>/dev/null || true)"

# Run one `gh search prs` query for the given review qualifier and print a
# FeedItem JSON array on stdout, tagging every PR with the supplied signal plus
# any per-PR author signals. Usage: search_reviews <qualifier> <signal>
search_reviews() {
  local qualifier="$1"
  local signal="$2"
  local raw
  # `$qualifier` is a bare GitHub search term (e.g. review-requested:@me),
  # not a named flag — gh search prs accepts qualifiers positionally.
  # Capture stdout only; let gh's stderr flow to the feed log so a warning
  # on a successful exit can't corrupt the JSON we hand to jq.
  if ! raw=$(gh search prs \
    --state=open \
    "$qualifier" \
    "${owner_flags[@]}" \
    --json number,title,body,url,repository,isDraft,author \
    --limit 100); then
    echo "fetch-reviews: gh search prs ($qualifier) failed" >&2
    echo "[]"
    return 0
  fi

  printf '%s' "$raw" | jq --arg signal "$signal" --arg me "$ME" '[
    .[] |
    select(.isDraft == false) |
    (.author.login // "") as $login |
    ($login | test("\\[bot\\]$")) as $is_bot |
    {
      external_id: ("review:" + .repository.nameWithOwner + "#" + (.number | tostring)),
      title: ("#" + (.number | tostring) + " " + .title),
      description: ((.body // "") | .[0:500]),
      url: .url,
      status: "backlog",
      tag: (if $is_bot then "dependabot" else "pr-review" end),
      labels: ((if $login != "" then ["@\($login)"] else [] end) + [.repository.name]),
      signals: (
        [$signal]
        + (if $is_bot then ["author-bot"] else [] end)
        + (if ($me != "" and $login == $me) then ["author-me"] else [] end)
      )
    }
  ]'
}

# Run every search, then dedup by URL MERGING the signal arrays (a PR matched
# by several queries keeps all its signals). NOT unique_by, which would drop
# all but one object and lose the other queries' signals.
{
  search_reviews "review-requested:@me" "team-request"
  search_reviews "user-review-requested:@me" "direct-request"
  search_reviews "reviewed-by:@me" "reviewed"
  search_reviews "commenter:@me -author:@me" "commented"
} | jq -s 'add
  | group_by(.url)
  | map(.[0] + {signals: (map(.signals[]) | unique)})'
