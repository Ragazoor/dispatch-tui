#!/usr/bin/env bash
# Verify every QUOTED citation in our agent-facing docs still names text that
# occurs in the file it points at. Third sibling to check-doc-paths.sh (which
# confirms a path exists but never looks inside it) and check-doc-symbols.sh
# (which resolves identifiers, not quoted section names). A quoted heading fell
# between the two and was checked by nothing.
#
# The spec audit in #4738 found two citations that had ALREADY been broken
# before it touched anything: feeds.allium cited a "Log-warning triage" heading
# that existed in no file, and a RoleRoutedFeedSync note pointed "above" at a
# section that had drifted out of sight. The same task moved three sections
# between specs, leaving 32 doc comments across 17 files to be repointed by
# hand with no gate watching. #4760 added this script.
#
# Three citation shapes are checked:
#
# crossfile — `core.allium: "Interval literals"`, backticked or not. Resolved
#   against docs/specs/<name>.allium, whatever directory prefix the citation
#   carries.
# inpath — `see "Column Sections" in docs/specs/core.allium`. The largest
#   population in this corpus, and where one of the two findings on the first
#   run lived (core.allium cited "CI status label" in feeds.allium, text that
#   has never appeared in that file). A bare name resolves under docs/specs/
#   for `.allium` and docs/ for `.md`, then falls back to the repo root.
# samefile — a spec's own `see "Derived review sections" below`. Resolved
#   against the containing file. This is the shape a cross-file citation
#   cannot rot into silently: it names no target at all, so nothing but this
#   check can tell it from prose.
#
# --- Why occurrence and not heading equality --------------------------------
#
# This is a PHANTOM check, not a heading check, for the same reason
# check-doc-symbols.sh is a phantom check: the strict version measured as
# noise. Requiring the quoted text to EQUAL a heading was measured against the
# real corpus at #4760 and flagged 4 citations, of which 1 was real rot. The
# other 3 were legitimate, and they were legitimate in four different ways —
# this corpus cites a heading PREFIX ("What this replaced" for "What this
# replaced, part 1: the start command"), a heading SUBSTRING ("Feed-Synced
# Auto-Completion" inside "Known Limitation: … Bypasses Notification"), a
# bolded list item that is not a heading at all (`- **Bulk reads skip and
# warn.**`), and plain quoted prose. Every one of those would have to be
# reworded to make a heading-equality gate green, and a gate that demands
# busywork gets bypassed.
#
# Requiring the quoted text to OCCUR in the target file flagged 2 across the
# same 67 citations, both real, none false. It still catches every phantom the
# audit found, because a heading that was renamed or deleted leaves no trace of
# its old text anywhere in the file. What it gives up is a heading DEMOTED to
# prose within the same file: the citation keeps resolving even though the
# section it names is gone. That trade is deliberate and is what the
# calibration cases in scripts/test-check-doc-headings.sh pin.
#
# Matching is case-insensitive: dispatch.allium cites its own guarantee's
# "this surface is a UI backstop" against prose that opens a sentence.
#
# --- Line wrapping is load-bearing ------------------------------------------
#
# Citations wrap across comment lines, and so does ordinary quoted prose —
# agent-tree.allium alone splits a quoted phrase across lines a dozen times.
# 22 of the 75 citations in this corpus span more than one line. Without a join
# a wrapped citation is invisible, because each half reads as an unbalanced
# quote.
#
# The scan unit is therefore a BLOCK — a maximal run of consecutive scannable
# lines, joined into one string with an offset map back to the physical line
# each character came from. A citation is reported at the line its match opens
# on, and the line it closes on is carried alongside, because a same-file
# citation must exclude the FULL span it occupies: excluding only its first
# line let a wrapped citation resolve against its own tail and report success.
#
# A blank comment line ends a block. A quoted phrase does not wrap across a
# paragraph break, while joining across one would let a quote ending one
# paragraph pair with an `above` opening the next and manufacture a citation
# out of ordinary prose.
#
# Scanned surfaces (or the explicit paths given as arguments):
#   - README.md, CLAUDE.md and the topic files under docs/
#   - docs/specs/*.allium — comment lines only; a quoted Allium string literal
#     is a value, not a citation
#   - plugin/skills/*/SKILL.md — agent-facing skill copy
#   - src/**/*.rs doc comments (`///`, `//!`)
#
# Deliberately NOT scanned:
#   - docs/plans/, docs/superpowers/, docs/research/ — dated working artifacts
#     that legitimately describe the docs as they stood then.
#   - the samefile shape outside docs/specs/. In CLAUDE.md the same words point
#     OUT of the file: "Validated knowledge for this task" above names a block
#     of the dispatch prompt, which is prepended at runtime and is nowhere in
#     the repo. Specs are self-contained prose; the other surfaces are not.
#   - a directional word with no quoted target ("see the section above"). There
#     is nothing to resolve, so there is nothing to check.
#   - markdown fenced code blocks are NOT exempt, matching check-doc-symbols.sh.
#
# Escape hatch: an `allow-phantom-heading: <why>` comment on the offending line,
# on its wrapped continuation, or on the line directly above — mirroring
# `allow-phantom-symbol:` in check-doc-symbols.sh.
#
# Behaviour is pinned by scripts/test-check-doc-headings.sh.
# Run from the repo root. Exits non-zero if anything is stale.
set -euo pipefail

if [[ $# -gt 0 ]]; then
    TARGETS=("$@")
else
    TARGETS=(README.md CLAUDE.md)
    # Globs that match nothing are dropped rather than passed through.
    shopt -s nullglob
    TARGETS+=(docs/*.md docs/specs/*.allium)
    # Both skill directories: plugin/skills/ is the embedded source of truth and
    # .claude/skills/ is auto-discovered for sessions in this repo. CLAUDE.md
    # holds them to the same contract, so a gate that scans one must scan both.
    TARGETS+=(plugin/skills/*/SKILL.md .claude/skills/*/SKILL.md)
    shopt -u nullglob
    # `tests/` as well as `src/`: the integration targets carry doc comments
    # that cite specs, and a sibling gate was found indexing tests/ without
    # scanning it, which hid a stale citation there indefinitely.
    mapfile -t -O "${#TARGETS[@]}" TARGETS < <(find src tests -name '*.rs' 2>/dev/null)
fi

MARKER='allow-phantom-heading:'

# `"[^"][^"]+"` is two-or-more characters, spelled out rather than written
# `{2,}`: interval expressions are not portable across awk implementations.
CROSSFILE_RE='`?[A-Za-z0-9_./-]+[.]allium`?[[:space:]]*: *"[^"][^"]+"'
INPATH_RE='"[^"][^"]+"[[:space:]]+in[[:space:]]+`?[A-Za-z0-9_./-]+[.](allium|md)`?'
# A sentinel space is appended to every scanned line so the trailing
# `[^A-Za-z]` can always match, which is how `above` is kept from matching
# inside a longer word.
SAMEFILE_RE='"[^"][^"]+"[[:space:]]+(above|below)[^A-Za-z]'

# --- Scan -------------------------------------------------------------------
# One awk pass per file emits
# "startline<TAB>endline<TAB>flag<TAB>kind<TAB>target<TAB>quote".
#
# Scanning is per BLOCK, not per line: a block is a maximal run of consecutive
# scannable lines, and the whole run is joined into one string with an offset
# map back to the physical line each character came from. A citation is then
# reported at the line its match OPENS on, and `endline` is the line it closes
# on — which is what lets a same-file citation exclude the FULL span it
# occupies rather than only its first line. Excluding only the first line let a
# wrapped citation resolve against its own tail and report success.
#
# A blank comment line (a bare `--` or `///`) ENDS a block. That is deliberate:
# a quoted phrase never wraps across a paragraph break, while joining across
# one would let a quote at the end of one paragraph pair with an `above` at the
# start of the next and manufacture a citation out of ordinary prose.
#
# Each shape is masked out of the joined text once consumed, and masked with
# spaces of EQUAL LENGTH rather than removed, because the offset of a match is
# what maps it back to a physical line.
extract_candidates() {
    local file="$1" mode=md
    [[ "$file" == *.rs ]] && mode=rs
    [[ "$file" == *.allium ]] && mode=allium

    awk -v mode="$mode" -v marker="$MARKER" -v crossre="$CROSSFILE_RE" \
        -v inre="$INPATH_RE" -v samere="$SAMEFILE_RE" -v self="$file" '
        # The scannable prose of one physical line, comment markers stripped.
        # A line that carries no prose in this file type yields "", which both
        # skips it as a citation site and ends the block it would have joined.
        function bodyof(s) {
            if (mode == "allium") {
                if (s !~ /^[[:space:]]*--/) return ""
                sub(/^[[:space:]]*--[[:space:]]?/, "", s)
                return s
            }
            if (mode == "rs") {
                if (s !~ /^[[:space:]]*(\/\/\/|\/\/!)/) return ""
                sub(/^[[:space:]]*(\/\/\/|\/\/!)[[:space:]]?/, "", s)
                return s
            }
            return s
        }
        # Which physical line a character offset in the joined block came from.
        function lineof(off,   k) {
            for (k = blo; k <= bhi; k++)
                if (off >= bstart[k] && off <= bend[k]) return k
            return bhi
        }
        # Emit every match of re, then blank it out in place so a later shape
        # cannot claim the same text and report it a second time.
        function harvest(kind, re,   st, len, tok, q, t, sl, el, k, f) {
            while (match(text, re)) {
                st = RSTART
                len = RLENGTH
                tok = substr(text, st, len)
                sl = lineof(st)
                el = lineof(st + len - 1)
                q = tok
                t = ""
                if (kind == "crossfile") {
                    sub(/^[^"]*"/, "", q)
                    sub(/"[^"]*$/, "", q)
                    t = tok
                    sub(/[[:space:]]*:.*$/, "", t)
                    gsub(/`/, "", t)
                } else if (kind == "inpath") {
                    sub(/^"/, "", q)
                    sub(/"[[:space:]]+in[[:space:]]+.*$/, "", q)
                    t = tok
                    sub(/^.*"[[:space:]]+in[[:space:]]+/, "", t)
                    gsub(/`/, "", t)
                } else {
                    sub(/^"/, "", q)
                    sub(/"[[:space:]]+(above|below)[^A-Za-z]$/, "", q)
                    # The citing file resolves its own samefile citations.
                    # Emitted rather than left blank because `read` with a
                    # tab IFS COLLAPSES an empty field, which shifted the
                    # quote into the target slot and left an empty quote
                    # that then matched every file. Silently green.
                    t = self
                }
                # Collapse the whitespace an unwrap leaves behind so the
                # quoted text is compared as one normalised phrase.
                gsub(/[[:space:]]+/, " ", q)
                sub(/^ /, "", q)
                sub(/ $/, "", q)
                # The marker may sit anywhere in the lines the citation spans,
                # or on the line directly above it.
                f = 0
                for (k = sl - 1; k <= el; k++)
                    if (k >= 1 && index(raw[k], marker)) f = 1
                if (q != "") print sl "\t" el "\t" f "\t" kind "\t" t "\t" q
                # `sprintf("%*s", len, "")` is a run of `len` spaces. Equal
                # length, not removal: the offset of every later match is what
                # maps it back to a physical line.
                text = substr(text, 1, st - 1) sprintf("%*s", len, "") substr(text, st + len)
            }
        }
        { raw[NR] = $0 }
        END {
            for (i = 1; i <= NR; i++) body[i] = bodyof(raw[i])
            i = 1
            while (i <= NR) {
                if (body[i] == "") { i++; continue }
                j = i
                while (j + 1 <= NR && body[j + 1] != "") j++
                blo = i
                bhi = j
                text = ""
                for (k = i; k <= j; k++) {
                    bstart[k] = length(text) + 1
                    text = text body[k] " "
                    bend[k] = length(text)
                }
                # A trailing sentinel space, so the `[^A-Za-z]` that keeps
                # `above` from matching inside a longer word always has a
                # character to land on. It belongs to the last line.
                text = text " "
                bend[j] = length(text)
                harvest("crossfile", crossre)
                harvest("inpath", inre)
                if (mode == "allium") harvest("samefile", samere)
                i = j + 1
            }
        }
    ' "$file"
}

# Where an `inpath` citation points. A path with a directory is taken as
# written; a bare name is looked for where that kind of file lives.
resolve_target() {
    local raw="$1"
    if [[ "$raw" == */* ]]; then
        printf '%s' "$raw"
        return
    fi
    local candidate home=docs
    [[ "$raw" == *.allium ]] && home=docs/specs
    for candidate in "docs/specs/$raw" "docs/$raw" "$raw"; do
        if [[ -f "$candidate" ]]; then
            printf '%s' "$candidate"
            return
        fi
    done
    # Nothing exists. Name the conventional home for the kind — the most useful
    # thing to put in front of whoever has to fix the citation.
    printf '%s/%s' "$home" "$raw"
}

# --- Citation-free view of a target ------------------------------------------
# A citation must not resolve against ANOTHER CITATION of the same phrase. That
# is the same self-validation hazard check-doc-symbols.sh calls load-bearing in
# its header ("indexing raw file text makes every phantom self-validate through
# its own doc comment"), and it is not hypothetical: feed-scripts.allium cited
# "Card label badges" in core.allium long after that section moved to
# board-visuals.allium, and stayed green because core.allium still held the
# phrase — inside a citation of its own.
#
# So every line span that is itself a citation is deleted from the target
# before the phrase is looked up. The citing line is one such span, which is
# why this also subsumes the same-file self-exclusion: a citation cannot
# validate itself for exactly the same reason it cannot validate a sibling.
#
# Spans come from running the same extractor over the target, cached per file.
declare -A CITE_SPANS=()
citation_free() {
    local path="$1"
    if [[ -z "${CITE_SPANS[$path]+set}" ]]; then
        local spans="" sl el rest
        while IFS=$'\t' read -r sl el rest; do
            spans+="${sl},${el}d;"
        done < <(extract_candidates "$path")
        CITE_SPANS["$path"]="$spans"
    fi
    sed -e "${CITE_SPANS[$path]}" "$path"
}

problems=0
report() {
    echo "check-doc-headings: $1" >&2
    problems=$((problems + 1))
}

for TARGET in "${TARGETS[@]}"; do
    if [[ ! -f "$TARGET" ]]; then
        echo "check-doc-headings: $TARGET not found" >&2
        exit 2
    fi

    # `endline` is read to keep the fields aligned; the span it describes is
    # applied by citation_free(), which recomputes it from the target itself.
    while IFS=$'\t' read -r lineno endline flag kind rawtarget quote; do
        case "$kind" in
        crossfile)
            # `docs/specs/other.allium` and `other.allium` name the same file,
            # so any directory prefix is dropped.
            cited="docs/specs/${rawtarget##*/}"
            ;;
        inpath)
            cited="$(resolve_target "$rawtarget")"
            ;;
        samefile)
            cited="$rawtarget"
            ;;
        esac

        if [[ -z "$quote" ]]; then
            echo "check-doc-headings: internal error — empty quote parsed from $TARGET:$lineno" >&2
            exit 2
        fi

        if [[ ! -f "$cited" ]]; then
            [[ "$flag" == 1 ]] && continue
            report "$TARGET:$lineno cites \"$quote\" in $cited, but that file does not exist"
            continue
        fi

        # A citation can never resolve against itself: without this every
        # samefile citation is vacuously green, since the quoted text is right
        # there on the citing line.
        # Counted rather than `grep -q`: under `set -o pipefail` a short-
        # circuiting `grep -q` leaves sed writing to a closed pipe and the
        # pipeline exits 141. The count drains it.
        hits="$(citation_free "$cited" | grep -icF -- "$quote" || true)"
        [[ "$hits" -gt 0 ]] && continue
        [[ "$flag" == 1 ]] && continue
        report "$TARGET:$lineno cites \"$quote\", but that text occurs nowhere in $cited"
    done < <(extract_candidates "$TARGET")
done

if ((problems > 0)); then
    echo "check-doc-headings: $problems unresolvable citation(s)" >&2
    echo "Name text the target file actually contains, or annotate a deliberate" >&2
    echo "historical reference with '$MARKER <why>' on or directly above the line." >&2
    exit 1
fi

echo "check-doc-headings: all quoted citations resolve"
