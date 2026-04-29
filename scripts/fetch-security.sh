#!/usr/bin/env bash
# Fetch open Dependabot vulnerability alerts for a list of GitHub repositories
# and output them as FeedItem JSON for use as a dispatch feed epic command.
#
# Requirements: gh (GitHub CLI, authenticated), jq
#
# Usage:
#   dispatch verify-feed scripts/fetch-security.sh   # validate output
#   # Or configure as feed_command on a feed epic with feed_interval_secs = 300
#
# To add/remove repos, edit the REPOS array below.

set -euo pipefail

REPOS=(
  "owner/repo1"
  "owner/repo2"
  # add more repos here
)

for repo in "${REPOS[@]}"; do
  gh api "/repos/$repo/dependabot/alerts?state=open&per_page=100" \
    | jq --arg repo "$repo" '[.[] | {
        external_id: ("dependabot:\($repo)#" + (.number | tostring)),
        title: ("[" + (.security_advisory.severity | ascii_upcase) + "] " + .security_advisory.summary),
        description: (.security_advisory.description // ""),
        url: .html_url,
        status: "backlog"
      }]'
done | jq -s '[.[][]]'
