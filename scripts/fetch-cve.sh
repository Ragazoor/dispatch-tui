#!/usr/bin/env bash
# Fetch open Dependabot vulnerability alerts (CVEs) for a list of GitHub
# repositories and output them as FeedItem JSON for use as a dispatch feed
# epic command.
#
# Requirements: gh (GitHub CLI, authenticated), jq
#
# Usage:
#   dispatch verify-feed scripts/fetch-cve.sh   # validate output
#   # Or configure as feed_command on the CVE epic with feed_interval_secs = 300
#
# Note: when used as a dispatch feed_command, use the absolute path to this
# script. Relative paths only work if dispatch is launched from the project root.
#
# To add/remove repos, edit the REPOS array below.

set -euo pipefail

REPOS=(
  # Add "owner/repo" slugs here, one per line. Example:
  #   "myorg/frontend"
  #   "myorg/backend"
)

if [[ ${#REPOS[@]} -eq 0 ]]; then
  echo '[]'
  exit 0
fi

# NOTE: per_page=100 is the GitHub REST API maximum per request. Repos with
# more than 100 open Dependabot alerts will be silently truncated. For full
# pagination use:
#   gh api --paginate "/repos/$repo/dependabot/alerts?state=open&per_page=100"

for repo in "${REPOS[@]}"; do
  gh api "/repos/$repo/dependabot/alerts?state=open&per_page=100" 2>/dev/null \
    | jq --arg repo "$repo" '[.[] | {
        external_id: ("cve:\($repo)#" + (.number | tostring)),
        title: (
          "[" + (.security_advisory.severity | ascii_upcase) + "] " +
          (if .security_advisory.cve_id != null
           then .security_advisory.cve_id + ": "
           else ""
           end) +
          .security_advisory.summary
        ),
        description: (.security_advisory.description // ""),
        url: .html_url,
        status: "backlog",
        tag: "fix"
      }]' \
    || echo '[]'
done | jq -s '[.[][]]'
