# Cleanups: skill drift and the keep-awake fallback

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** the preference-review skill and its spec agree, the retro skill's frontmatter is valid YAML, and keep-awake falls back when the GNOME helper refuses.

## Context

Findings N1, N2 and R7 from `docs/plans/2026-11-15-codebase-review.md` §3.6 and §3.8. Edit skills in `plugin/skills/*/SKILL.md`, never the installed copy.

## Findings

### 🟢 Low — N1: preference-review skill and spec disagree after 7c71ae8a (`plugin/skills/preference-review/SKILL.md`; `docs/specs/preference-review.allium`)

**Issue:** Step 4 says "split them by file across agents" while the spec has `single_verifier_max_findings = 8`; Step 6 dropped the voice rules (short and direct, a question when the point is a question, no hedging, no praise) the spec's Present guidance requires; the skill no longer says where `owner`/`repo` come from for a bare PR number, which Steps 1, 2 and 7 need.

**Fix:** restore the three points in the skill in plain wording (no checklists), or change the spec to match; keep them agreeing.

### 🟢 Low — N2: the retro skill's description is not strict YAML (`plugin/skills/retro/SKILL.md` frontmatter; `tests/repo_skills.rs::frontmatter_value`) — plausible

**Issue:** an unquoted plain scalar containing `task: to finish`; the gate splits by line prefix, so it passes.

**Fix:** quote the description; make the gate parse frontmatter with a YAML parser (the crate likely has `serde_yaml` or similar already; check before adding a dependency).

### 🟢 Low — R7: with no desktop variable, the GNOME helper is chosen with no fallback (`src/keep_awake.rs::hold_command`) — plausible

**Issue:** an ssh-started tmux server with `gnome-session-inhibit` installed but no session: the helper is refused, `systemd-inhibit` is never tried, and `OnceLock` keeps the choice.

**Fix:** fall back to `systemd-inhibit` on refusal; do not cache a failed choice. State it in `docs/specs/keep-awake.allium`.

## Changes

| File | Change |
|------|--------|
| `plugin/skills/preference-review/SKILL.md` or `docs/specs/preference-review.allium` | agree |
| `plugin/skills/retro/SKILL.md`, `tests/repo_skills.rs` | quote; YAML parse |
| `src/keep_awake.rs`, `docs/specs/keep-awake.allium` | fallback |

## Verification

- [ ] `./scripts/verify.sh` passes
- [ ] The frontmatter gate fails on an unquoted `: ` in a description
