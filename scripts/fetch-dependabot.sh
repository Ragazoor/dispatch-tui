#!/usr/bin/env bash
# fetch-dependabot.sh — outputs open dependency-update PRs as a FeedItem JSON
# array for use as a dispatch feed_command.
#
# Prerequisites: gh CLI (https://cli.github.com/) and jq must be in PATH.
#
# Usage:
#   1. Set REPOS in repos.conf to your list of "owner/repo" slugs.
#   2. Set BOT_AUTHORS in bots.conf to your bots' logins, in gh's app form
#      ("app/<slug>"). This is the SAME file fetch-reviews.sh's bot-author pass
#      reads: a bot login is deployment-specific (a self-hosted Renovate app is
#      named after the org that installed it), and one repo can carry PRs from
#      several bots at once — a Renovate rollout that has not finished still
#      has open Dependabot PRs. One pass runs per login.
#   3. Set feed_command on your Dependabot epic to the path of this script.
#      Example: /home/you/scripts/fetch-dependabot.sh
#
# Output format (FeedItem):
#   [{"external_id":"dep:owner/repo#42","title":"#42 Bump foo","description":"...","url":"...","status":"backlog","tag":"dependabot"}]

# ---------------------------------------------------------------------------
# Repositories: edit repos.conf in the same directory (SSOT), or set REPOS
# directly below as a fallback when repos.conf is not present.
REPOS=()

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [[ -f "$SCRIPT_DIR/repos.conf" ]]; then
  # shellcheck source=repos.conf
  source "$SCRIPT_DIR/repos.conf"
fi

# Bot logins: edit bots.conf in the same directory (SSOT, shared with
# fetch-reviews.sh). An absent or empty BOT_AUTHORS falls back to
# app/kognic-renovate rather than emitting nothing — an existing copy of this
# script predates bots.conf and must keep emitting what it emitted before.
BOT_AUTHORS=()
if [[ -f "$SCRIPT_DIR/bots.conf" ]]; then
  # shellcheck source=bots.conf
  source "$SCRIPT_DIR/bots.conf"
fi
if [[ ${#BOT_AUTHORS[@]} -eq 0 ]]; then
  BOT_AUTHORS=("app/kognic-renovate")
fi
# ---------------------------------------------------------------------------

if [[ ${#REPOS[@]} -eq 0 ]]; then
  echo "[]"
  exit 0
fi

result="[]"

for repo in "${REPOS[@]}"; do
  # Probe repo existence/auth first — `gh pr list --author <bot>` silently
  # returns [] on 404/SSO failures, so we'd never see auth issues.
  probe=$(gh api "/repos/$repo" --jq '.full_name' 2>&1)
  status=$?
  if [ $status -ne 0 ]; then
    echo "fetch-dependabot: $repo — repo unreachable (exit $status): $probe" >&2
    continue
  fi

  # One pass per configured bot. A PR has exactly one author, so the passes
  # cannot emit the same external_id twice and no dedup is needed. A failing
  # pass skips that bot only — one bot missing from an org must not cost the
  # cards the other bot raised in the same repo.
  for author in "${BOT_AUTHORS[@]}"; do
    raw=$(gh pr list \
      --repo "$repo" \
      --author "$author" \
      --state open \
      --json number,title,body,url 2>&1)
    status=$?
    if [ $status -ne 0 ]; then
      echo "fetch-dependabot: $repo ($author) — gh pr list failed (exit $status): $raw" >&2
      continue
    fi

    items=$(printf '%s' "$raw" | jq --arg repo "$repo" '[.[] | {
        external_id: ("dep:" + $repo + "#" + (.number | tostring)),
        title: ("#" + (.number | tostring) + " " + .title),
        description: ((.body // "") | .[0:500]),
        url: .url,
        status: "backlog",
        tag: "dependabot",
        labels: [($repo | split("/") | last)]
      }]') || {
      echo "fetch-dependabot: $repo ($author) — jq failed on output: $raw" >&2
      continue
    }

    result=$(printf '%s\n%s' "$result" "$items" | jq -s 'add // []')
  done
done

echo "$result"
