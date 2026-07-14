#!/usr/bin/env bash
# fetch-reviews.sh — outputs every open PR you are involved with as a single,
# deduped FeedItem JSON array, for use as a dispatch feed_command.
#
# Prerequisites: gh CLI (https://cli.github.com/) and jq must be in PATH.
#
# Setup:
#   1. Copy this file to scripts/local/fetch-reviews.sh
#   2. Edit repos.conf in the same directory (the REPOS array) to list the
#      "owner/repo" slugs you want review-related PR activity for. This is
#      the same SSOT fetch-cve.sh reads, so reviews and CVEs stay scoped to
#      one repo list. Feeds My/Team/Bots exactly as before.
#   3. Optionally edit org.conf in the same directory (the ORGS array) to
#      list GitHub org slugs you want review activity for. This is a
#      SEPARATE scope: it re-runs three of the four review-related queries
#      (excluding the team-inclusive review-requested:@me — see below)
#      against whole orgs instead of the repo list, and every match routes
#      to My Reviews only (it never widens Team Reviews or Bots).
#   4. Point the parent "Reviews" epic's feed_command at the local copy.
#      There is NO scope argument — the dispatch role router (feed_role =
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
#   These four are scoped by repos.conf's REPOS list. THREE of the four are
#   run again, scoped by org.conf's ORGS list instead, and every match from
#   that pass carries a single shared signal so it always lands in My
#   Reviews:
#     - user-review-requested:@me | reviewed-by:@me |
#       commenter:@me -author:@me   (per org)  -> signal "org-review"
#   review-requested:@me is deliberately excluded from the org-scoped pass:
#   it also matches PRs requested from a team you belong to (not just you
#   personally), which org-wide would sweep in team-request noise.
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
# Repositories to search for the review-related queries: edit repos.conf in
# the same directory (the REPOS array — SSOT shared with fetch-cve.sh). Falls
# back to skipping those queries when repos.conf is absent or lists no repos.
REPOS=()

# Organisations to search for the org-scoped review queries ONLY: edit
# org.conf in the same directory (the ORGS array). Falls back to skipping
# those queries when org.conf is absent or lists no orgs.
ORGS=()

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [[ -f "$SCRIPT_DIR/repos.conf" ]]; then
  # shellcheck source=repos.conf
  source "$SCRIPT_DIR/repos.conf"
fi
if [[ -f "$SCRIPT_DIR/org.conf" ]]; then
  # shellcheck source=org.conf
  source "$SCRIPT_DIR/org.conf"
fi
# ---------------------------------------------------------------------------

repo_flags=()
for repo in "${REPOS[@]}"; do
  repo_flags+=(--repo "$repo")
done

owner_flags=()
for org in "${ORGS[@]}"; do
  owner_flags+=(--owner "$org")
done

# The gh user's login, for the author-me signal. Soft-fails to empty so a
# transient `gh api` error degrades author-me detection rather than the feed.
ME="$(gh api user -q .login 2>/dev/null || true)"

# Run one `gh search prs` query for the given qualifier, scoped by the given
# scope flags (repo_flags or owner_flags), and print a FeedItem JSON array on
# stdout, tagging every PR with the supplied signal plus any per-PR author
# signals. Usage: search_prs <qualifier> <signal> <scope_flags_name>
search_prs() {
  local qualifier="$1"
  local signal="$2"
  local -n scope_flags="$3"
  local raw

  if [[ ${#scope_flags[@]} -eq 0 ]]; then
    echo "[]"
    return 0
  fi

  # `$qualifier` is one or more bare GitHub search terms (e.g.
  # "review-requested:@me" or "commenter:@me -author:@me"). They go AFTER `--`
  # so a leading-dash term like `-author:@me` isn't parsed as a gh flag, and
  # are deliberately left unquoted so a multi-term qualifier word-splits into
  # separate search terms instead of one mangled `commenter:"@me -author:@me"`.
  # Capture stdout only; let gh's stderr flow to the feed log so a warning
  # on a successful exit can't corrupt the JSON we hand to jq.
  # shellcheck disable=SC2086  # intentional word-splitting of $qualifier
  if ! raw=$(gh search prs \
    --state=open \
    "${scope_flags[@]}" \
    --json number,title,body,url,repository,isDraft,author \
    --limit 100 \
    -- $qualifier); then
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
  search_prs "review-requested:@me" "team-request" repo_flags
  search_prs "user-review-requested:@me" "direct-request" repo_flags
  search_prs "reviewed-by:@me" "reviewed" repo_flags
  search_prs "commenter:@me -author:@me" "commented" repo_flags
  # Three of the four qualifiers again, org-scoped — every match here is
  # tagged with one shared signal so it always lands in My Reviews (never
  # Team/Bots), regardless of which qualifier matched. review-requested:@me
  # is deliberately EXCLUDED from this org-scoped pass: unlike the other
  # three (which are always about ME personally), it also matches PRs
  # requested from a TEAM I belong to, and org-wide that would sweep in
  # far more team-request noise than repo-scoped ever did.
  search_prs "user-review-requested:@me" "org-review" owner_flags
  search_prs "reviewed-by:@me" "org-review" owner_flags
  search_prs "commenter:@me -author:@me" "org-review" owner_flags
} | jq -s 'add
  | group_by(.url)
  | map(.[0] + {signals: (map(.signals[]) | unique)})'
