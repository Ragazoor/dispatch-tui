#!/usr/bin/env bash
# fetch-reviews.sh — outputs open PRs awaiting review as a FeedItem JSON array,
# for use as a dispatch feed_command.
#
# Prerequisites: gh CLI (https://cli.github.com/) and jq must be in PATH.
#
# Usage:
#   fetch-reviews.sh [my|team]
#     my     — PRs that request you specifically (user-review-requested:@me)
#     team   — PRs requested via a team you belong to, excluding your direct
#              requests (review-requested:@me minus user-review-requested:@me)
#     (none) — PRs requesting you or any of your teams (review-requested:@me)
#
# Setup:
#   1. Copy this file to scripts/local/fetch-reviews.sh
#   2. Set ORGS below to your list of GitHub organisation slugs.
#   3. Point each review epic's feed_command at the local copy plus the scope
#      arg, e.g. ".../scripts/local/fetch-reviews.sh my".
#
# Output format (FeedItem):
#   [{"external_id":"review:org/repo#42","title":"#42 PR title","description":"...","url":"...","status":"backlog","tag":"pr-review","labels":["@author","repo"]}]
#
# Note: review-requested:@me folds in BOTH direct and team review requests.
# user-review-requested:@me is direct-only. "team" is the set difference.

set -euo pipefail

# ---------------------------------------------------------------------------
# Organisations to search. Fill in your org slugs, e.g.:
#   ORGS=("myorg" "another-org")
ORGS=()
# ---------------------------------------------------------------------------

SCOPE="${1:-all}"

if [[ ${#ORGS[@]} -eq 0 ]]; then
  echo "[]"
  exit 0
fi

owner_flags=()
for org in "${ORGS[@]}"; do
  owner_flags+=(--owner "$org")
done

# Run one `gh search prs` query for the given review qualifier and print a
# FeedItem JSON array on stdout. Usage: search_reviews <qualifier>
search_reviews() {
  local qualifier="$1"
  local raw
  raw=$(gh search prs \
    --state=open \
    "$qualifier" \
    "${owner_flags[@]}" \
    --json number,title,body,url,repository,isDraft,updatedAt,author \
    --limit 100 2>&1) || {
    echo "fetch-reviews: gh search prs ($qualifier) failed: $raw" >&2
    echo "[]"
    return 0
  }

  printf '%s' "$raw" | jq '[
    .[] |
    select(.isDraft == false) |
    select(.author.login != "dependabot[bot]") |
    {
      external_id: ("review:" + .repository.nameWithOwner + "#" + (.number | tostring)),
      title: ("#" + (.number | tostring) + " " + .title),
      description: ((.body // "") | .[0:500]),
      url: .url,
      status: "backlog",
      tag: "pr-review",
      labels: ((if .author.login then ["@\(.author.login)"] else [] end) + [.repository.name])
    }
  ]'
}

case "$SCOPE" in
  my)
    search_reviews "user-review-requested:@me"
    ;;
  all)
    search_reviews "review-requested:@me"
    ;;
  team)
    all=$(search_reviews "review-requested:@me")
    mine=$(search_reviews "user-review-requested:@me")
    # Team = all minus mine, matched by PR url, so a PR that requests me
    # directly never also appears under the team epic.
    jq -n --argjson all "$all" --argjson mine "$mine" \
      '($mine | map(.url)) as $mine_urls
       | $all | map(select(.url as $u | ($mine_urls | index($u)) | not))'
    ;;
  *)
    echo "fetch-reviews: unknown scope '$SCOPE' (expected: my, team, or no argument)" >&2
    exit 2
    ;;
esac
