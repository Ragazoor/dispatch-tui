#!/usr/bin/env bash
# test-check-doc-headings.sh — behavioural test for scripts/check-doc-headings.sh.
#
# The checker resolves QUOTED citations — the shape `core.allium: "Interval
# literals"`, `see "Column Sections" in docs/specs/core.allium`, and a spec's
# own `see "Derived review sections" below`. Nothing checked these before
# #4760: check-doc-paths.sh confirms the file exists without looking inside it,
# and check-doc-symbols.sh resolves identifiers, not quoted section names. The
# spec audit in #4738 found two citations that had been broken the whole time.
#
# The assertions below pin the decisions that make the check trustworthy rather
# than noisy:
#   - resolution is a VERBATIM OCCURRENCE test, not a heading-equality test.
#     Measured on the real corpus at #4760: heading-equality flagged 4
#     citations of which 1 was real rot, because the corpus legitimately cites
#     heading PREFIXES, heading SUBSTRINGS, bolded list anchors, and plain
#     prose. Verbatim occurrence flagged 2, both real, none false. Cases for
#     each of those four legitimate shapes are below, and they are what fails
#     if anyone tightens the rule to heading-equality.
#   - matching is case-insensitive: dispatch.allium cites its own guarantee's
#     "this surface is a UI backstop" against prose that opens the sentence.
#   - citations WRAP across comment lines, so a line and its continuation are
#     scanned as one. Un-wrapping is not cosmetic: quoted prose in these specs
#     spans lines constantly, and a single-line scan reads the fragments as
#     unbalanced quotes.
#   - `above`/`below` is scanned in SPECS ONLY. In CLAUDE.md the same shape
#     points at the dispatch prompt ("Validated knowledge for this task"
#     above), which is not in the file at all.
#
# Hermetic: every assertion runs against a temp fixture repo. Validating this
# repo's own docs is check-doc-headings.sh's own job, run as its own hook step.
#
# Run from the repo root:  bash scripts/test-check-doc-headings.sh
# Exits 0 on success, non-zero with a diagnostic on the first failed assertion.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CHECKER="$SCRIPT_DIR/check-doc-headings.sh"

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

# --- Fixture repo -----------------------------------------------------------
mkdir -p "$WORKDIR/src" "$WORKDIR/docs/specs" "$WORKDIR/docs/plans" "$WORKDIR/plugin/skills/some-skill"

printf 'Fixture root doc.\n' >"$WORKDIR/CLAUDE.md"
printf '# Fixture\n' >"$WORKDIR/README.md"

# A markdown topic doc. Its heading carries a trailing clause, so a citation of
# the bare "DB access" is a PREFIX — the shape heading-equality would reject.
# The bold list item is the fourth legitimate anchor shape: the real corpus
# cites `- **Bulk reads skip and warn.**`, which is not a heading at all.
cat >"$WORKDIR/docs/conventions.md" <<'MD'
## DB access — `db_call` / `db_call_read`

- **Bulk reads skip and warn.** One corrupt row degrades the board.
MD

# Two heading forms, both real in this corpus: the banner (rule line, `-- Text`,
# rule line) and the inline `-- == Text ==`.
cat >"$WORKDIR/docs/specs/other.allium" <<'ALLIUM'
-- allium: 3

------------------------------------------------------------
-- Polling
------------------------------------------------------------

-- == Upsert semantics, part one ==
--
-- The runtime never overlaps two cycles.
--
-- A pane is "a pane dispatch put in this window", whatever else it
-- holds.

rule SidecarStarts { when: Whatever }
ALLIUM

# A second spec, so a citation can name a heading that is real but lives in the
# WRONG file. "Polling" exists only in other.allium.
cat >"$WORKDIR/docs/specs/real.allium" <<'ALLIUM'
-- allium: 3

------------------------------------------------------------
-- Board Columns
------------------------------------------------------------

-- == Card stripe ==

rule DoThing { when: Whatever }
ALLIUM

failures=0

# Run the checker over fixture file $2 (contents $3), asserting exit status $1.
# $4 is the assertion label.
#
# The scratch target is removed afterwards. A scratch spec left under
# docs/specs/ would become a resolution source for the next assertion.
expect() {
    local want="$1" target="$2" body="$3" label="$4"
    printf '%s\n' "$body" >"$WORKDIR/$target"

    local got=0
    local out
    out="$(cd "$WORKDIR" && bash "$CHECKER" "$target" 2>&1)" || got=$?
    rm -f "$WORKDIR/$target"

    if [[ "$got" != "$want" ]]; then
        echo "FAIL: $label" >&2
        echo "  target: $target" >&2
        echo "  body: $body" >&2
        echo "  expected exit $want, got $got" >&2
        echo "  output: $out" >&2
        failures=$((failures + 1))
    fi
}

# --- Shape 1: `<spec>.allium: "Heading"` -----------------------------------
expect 0 docs/specs/scratch.allium '-- Cadence is fixed (other.allium: "Polling").' \
    'cross-file citation of an exact banner heading passes'
expect 0 docs/specs/scratch.allium '-- Cadence is fixed (`other.allium`: "Polling").' \
    'backticking the spec name does not change resolution'
expect 0 docs/scratch.md 'See docs/specs/other.allium: "Polling" for the cadence.' \
    'a path-qualified spec name resolves to the same file'
expect 0 src/scratch.rs '/// Cadence is fixed — other.allium: "Polling".' \
    'cross-file citation passes in a Rust doc comment'
expect 1 docs/specs/scratch.allium '-- Cadence is fixed (other.allium: "Ghost Heading").' \
    'cross-file citation of text absent from the cited spec fails'
expect 1 docs/specs/scratch.allium '-- Cadence is fixed (real.allium: "Polling").' \
    'cross-file citation naming the WRONG spec fails'
expect 1 docs/specs/scratch.allium '-- Cadence is fixed (nowhere.allium: "Polling").' \
    'cross-file citation of a spec file that does not exist fails'
expect 1 src/scratch.rs '/// Cadence is fixed — other.allium: "Ghost Heading".' \
    'a stale cross-file citation fails in a Rust doc comment'
expect 0 src/scratch.rs 'fn f() { let s = "other.allium: \"Ghost Heading\""; }' \
    'a Rust CODE line is not scanned'

# --- Shape 3: `"Heading" in <path>` ----------------------------------------
# Not in #4760's brief, but it is the largest population in the corpus and it
# held one of the two real findings (core.allium cited "CI status label" in
# feeds.allium, where that text has never appeared).
expect 0 docs/specs/scratch.allium '-- Deferred — see "Polling" in other.allium.' \
    'in-path citation of an exact heading passes'
expect 0 src/scratch.rs '/// See "Polling" in `docs/specs/other.allium`.' \
    'a backticked path in an in-path citation resolves'
expect 0 docs/scratch.md 'See "Polling" in other.allium for the cadence.' \
    'a bare `.allium` name resolves under docs/specs/'
expect 0 docs/scratch.md 'See "DB access" in conventions.md for the rule.' \
    'a bare `.md` name resolves under docs/'
expect 1 docs/specs/scratch.allium '-- Deferred — see "Ghost Heading" in other.allium.' \
    'in-path citation of text absent from the target fails (the #4738 rot)'
expect 1 docs/specs/scratch.allium '-- Deferred — see "Polling" in nowhere.allium.' \
    'in-path citation whose target file does not exist fails'
expect 1 docs/scratch.md 'See "Polling" in `docs/conventions.md` for the cadence.' \
    'in-path citation naming the WRONG file fails'

# --- The four legitimate anchor shapes heading-equality would reject --------
# These are the calibration. Tightening the rule to heading-equality fails all
# four, which is what happened when the strict rule was measured at #4760.
expect 0 docs/specs/scratch.allium '-- Deferred — see "Upsert semantics" in other.allium.' \
    'a PREFIX of a real heading resolves'
expect 0 docs/specs/scratch.allium '-- Deferred — see "semantics, part one" in other.allium.' \
    'a SUBSTRING of a real heading resolves'
expect 0 docs/specs/scratch.allium '-- Deferred — see "never overlaps two cycles" in other.allium.' \
    'plain PROSE quoted from the target resolves'
expect 0 docs/scratch.md 'See "Bulk reads skip and warn" in `docs/conventions.md`.' \
    'a bolded list-item anchor resolves — it is not a heading at all'

# --- Case-insensitive resolution -------------------------------------------
# dispatch.allium cites its own guarantee's "this surface is a UI backstop",
# and the prose it points at opens a sentence with a capital.
expect 0 docs/specs/scratch.allium '-- Deferred — see "THE RUNTIME NEVER OVERLAPS" in other.allium.' \
    'resolution is case-insensitive'

# --- Shape 2: same-file `"Heading" above` / `below`, SPECS ONLY ------------
expect 0 docs/specs/scratch.allium '-- == Local Heading ==
--
-- Nothing here yet.
--
-- Handled where it is raised — see "Local Heading" above.' \
    'same-file citation of a heading in the same spec passes'
expect 0 docs/specs/scratch.allium '-- Quoted below is the phrase "cards never overlap" itself.
--
-- Which is what "cards never overlap" above means.' \
    'same-file citation of prose that occurs elsewhere in the file passes'
expect 1 docs/specs/scratch.allium '-- Which is what "no two cards share a border" above means.' \
    'same-file citation of text that occurs nowhere else fails (the #4760 core.allium rot)'
# The same rot, but in a spec with other content. A single-line fixture cannot
# tell a real finding from a parser bug that empties the quoted text: `grep -F
# ""` matches every line of a non-empty file but nothing at all in an empty
# one, so an empty-quote bug reads as a correct finding on a one-line file and
# as silent success on every real one. That is exactly how the tab-IFS field
# collapse survived the first draft of this suite.
expect 1 docs/specs/scratch.allium '-- == Something Else ==
--
-- Cards are drawn with a complete border of their own.
--
-- Which is what "no two cards share a border" above means.' \
    'same-file rot is still caught in a spec with other content'
expect 1 docs/specs/scratch.allium '-- Handled where it is raised — see "Ghost Heading" below.' \
    'same-file citation of a heading no section declares fails'
# In CLAUDE.md the same shape names the dispatch prompt, not the file: the
# "Validated knowledge for this task" block is prepended at runtime and is
# nowhere in the repo. Asserted on a scratch doc rather than on CLAUDE.md
# itself, because expect() removes its target afterwards.
expect 0 docs/scratch.md 'Rate each entry — see "Validated knowledge for this task" above.' \
    'above/below is not scanned in markdown'
expect 0 src/scratch.rs '/// Ordering matters — see "Ghost Heading" above.' \
    'above/below is not scanned in Rust doc comments'

# --- Directional words with no quoted target are not citations -------------
expect 0 docs/specs/scratch.allium '-- Handled where it is raised — see the section above.' \
    'a directional word with no quoted target is not a citation'
expect 0 docs/specs/scratch.allium '-- See scripts/fetch-reviews.sh above for the emission shape.' \
    'an unquoted path plus a directional word is not a citation'

# --- Citations wrap across comment lines -----------------------------------
# Un-wrapping is load-bearing, not cosmetic: without it the fragments read as
# unbalanced quotes and the citation is never seen at all.
expect 0 docs/specs/scratch.allium '-- Deferred — see "Polling" in
-- other.allium, which owns the cadence.' \
    'a citation wrapped between the quote and its target resolves'
expect 1 docs/specs/scratch.allium '-- Deferred — see "Ghost Heading" in
-- other.allium, which owns the cadence.' \
    'a wrapped citation is really parsed — the stale form still fails'
expect 0 docs/specs/scratch.allium '-- == Polling Local ==
--
-- Handled where it is raised — see "Polling
-- Local" above.' \
    'a citation whose quoted text itself wraps resolves'
# Quoted prose spans lines constantly in these specs. Joining must not let the
# tail of one prose quote pair up with the head of the next.
expect 0 docs/specs/scratch.allium '-- The user is scanning for "what did this
-- agent do to my repo", and the answer is above.' \
    'wrapped prose does not manufacture a same-file citation'

# --- Wrapping: the whole comment block is one scan unit --------------------
# Joining only the NEXT line left three blind spots, all found in review of the
# first draft and all reproduced before being fixed. They share one cause: a
# citation can span more than two lines, and a same-file citation that wraps
# carries most of its quoted text on a line the self-exclusion did not remove.
expect 1 docs/specs/scratch.allium '-- Which is what "
-- Ghost Text" above means.' \
    'a wrapped same-file citation cannot resolve against its own tail'
expect 1 docs/specs/scratch.allium '-- Deferred — see "Polling
-- across
-- lines" in other.allium.' \
    'a citation wrapped across three lines is still parsed'
expect 1 docs/specs/scratch.allium '-- Cadence is fixed (other.allium
-- : "Ghost Heading").' \
    'a citation wrapped right at the colon is still parsed'
# The positive half of the same three: a wrap must not turn a good citation red.
expect 0 docs/specs/scratch.allium '-- == Ghost Text ==
--
-- Which is what "
-- Ghost Text" above means.' \
    'a wrapped same-file citation still resolves against a real heading'
expect 0 docs/specs/scratch.allium '-- Deferred — see "Upsert
-- semantics,
-- part one" in other.allium.' \
    'a three-line wrap resolves when the phrase is real'
expect 0 docs/specs/scratch.allium '-- Cadence is fixed (other.allium
-- : "Polling").' \
    'a wrap at the colon resolves when the heading is real'

# A blank comment line ENDS the block, so a citation split across a paragraph
# break is deliberately not assembled. Joining across one would let a quote
# ending paragraph A pair with an `above` opening paragraph B and manufacture a
# citation out of ordinary prose. A quoted phrase does not wrap across a
# paragraph break, so the cost is nil and the false-positive risk is real.
expect 0 docs/specs/scratch.allium '-- Deferred — see "Polling
--
-- Ghost" in other.allium.' \
    'a paragraph break ends the block and is not joined across'
expect 0 docs/specs/scratch.allium '-- The rejected alternative was to paint it on the column "ground"
--
-- above all else, the frame must read as an edge.' \
    'a quote ending one paragraph cannot pair with `above` opening the next'

# The marker may sit anywhere in the span a wrapped citation occupies.
expect 0 docs/specs/scratch.allium '-- Was other.allium: "Ghost
-- Heading". allow-phantom-heading: section retired' \
    'marker on the continuation line of a wrapped citation suppresses it'

# --- Escape hatch: allow-phantom-heading marker ----------------------------
expect 0 docs/specs/scratch.allium '-- Was other.allium: "Ghost Heading". allow-phantom-heading: section retired' \
    'marker on the offending line suppresses the finding'
expect 0 docs/specs/scratch.allium '-- allow-phantom-heading: section retired
-- Was other.allium: "Ghost Heading".' \
    'marker on the line directly above suppresses the finding'
expect 1 docs/specs/scratch.allium '-- allow-phantom-heading: too far away
--
-- Was other.allium: "Ghost Heading".' \
    'marker two lines above does not suppress the finding'
expect 0 docs/scratch.md 'Was "Ghost Heading" in other.allium. <!-- allow-phantom-heading: retired -->' \
    'marker suppresses a stale in-path citation in markdown'

# --- Default scan surfaces -------------------------------------------------
# docs/plans/ is a dated working artifact and must stay out of the default scan.
printf 'Stale by design: see "Ghost Heading" in other.allium.\n' >"$WORKDIR/docs/plans/old.md"
out="$(cd "$WORKDIR" && bash "$CHECKER" 2>&1)" || true
if grep -q 'docs/plans/' <<<"$out"; then
    echo "FAIL: default scan must not read docs/plans/" >&2
    echo "  output: $out" >&2
    failures=$((failures + 1))
fi

# The fixture corpus is otherwise clean, so the default scan must be green.
rm -f "$WORKDIR/docs/plans/old.md"
if ! (cd "$WORKDIR" && bash "$CHECKER" >/dev/null 2>&1); then
    echo "FAIL: default scan over the clean fixture must pass" >&2
    echo "  output: $(cd "$WORKDIR" && bash "$CHECKER" 2>&1 || true)" >&2
    failures=$((failures + 1))
fi

# Every surface the corpus keeps citations on must be in the default scan —
# asserted BEHAVIOURALLY, by planting a stale citation on each surface and
# requiring the default scan to name the file.
#
# The obvious cheap version of this check greps the checker's own source for
# each surface string. That version is worthless: four of the five strings
# also appear in the checker's header prose, so it passes against a checker
# whose TARGETS array is empty. Planting a rot is the only form that can tell
# a scanned surface from a mentioned one.
scan_surfaces=(
    'README.md'
    'CLAUDE.md'
    'docs/topic.md'
    'docs/specs/scratch.allium'
    'src/scratch.rs'
    'tests/scratch.rs'
    'plugin/skills/some-skill/SKILL.md'
    '.claude/skills/other-skill/SKILL.md'
)
for surface in "${scan_surfaces[@]}"; do
    mkdir -p "$WORKDIR/$(dirname "$surface")"
    saved=""
    [[ -f "$WORKDIR/$surface" ]] && saved="$(cat "$WORKDIR/$surface")"
    # A Rust surface needs the citation inside a doc comment to be in scope.
    if [[ "$surface" == *.rs ]]; then
        printf '/// See "Ghost Heading" in other.allium.\n' >"$WORKDIR/$surface"
    else
        printf -- '-- See "Ghost Heading" in other.allium.\n' >"$WORKDIR/$surface"
    fi
    out="$(cd "$WORKDIR" && bash "$CHECKER" 2>&1)" || true
    if [[ -n "$saved" ]]; then printf '%s\n' "$saved" >"$WORKDIR/$surface"; else rm -f "$WORKDIR/$surface"; fi
    if ! grep -qF "$surface" <<<"$out"; then
        echo "FAIL: the default scan does not cover $surface" >&2
        echo "  output: $out" >&2
        failures=$((failures + 1))
    fi
done
rm -rf "$WORKDIR/plugin/skills/some-skill" "$WORKDIR/.claude" "$WORKDIR/tests"

# --- A citation is reported exactly once -----------------------------------
printf '%s\n' '-- Deferred — see "Ghost Heading" in other.allium.' >"$WORKDIR/docs/specs/scratch.allium"
out="$(cd "$WORKDIR" && bash "$CHECKER" docs/specs/scratch.allium 2>&1)" || true
rm -f "$WORKDIR/docs/specs/scratch.allium"
if [[ "$(grep -c 'Ghost Heading' <<<"$out")" != 1 ]]; then
    echo "FAIL: a citation must be reported exactly once" >&2
    echo "  output: $out" >&2
    failures=$((failures + 1))
fi

# A finding must name the line the citation opens on, not the start of the
# comment block it sits in.
printf '%s\n' '-- Preamble line one.
-- Preamble line two.
-- Deferred — see "Ghost Heading" in other.allium.' >"$WORKDIR/docs/specs/scratch.allium"
out="$(cd "$WORKDIR" && bash "$CHECKER" docs/specs/scratch.allium 2>&1)" || true
rm -f "$WORKDIR/docs/specs/scratch.allium"
if ! grep -q 'scratch.allium:3' <<<"$out"; then
    echo "FAIL: a finding must report the line the citation opens on" >&2
    echo "  output: $out" >&2
    failures=$((failures + 1))
fi

if ((failures > 0)); then
    echo "test-check-doc-headings: $failures assertion(s) failed" >&2
    exit 1
fi

echo "test-check-doc-headings: all assertions passed"
