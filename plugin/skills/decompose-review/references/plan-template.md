# {{Work Package Name}}

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** {{One sentence describing what this work package accomplishes}}

## Context

This work package addresses findings from a code review.

## Findings

{{For each finding, include:}}

### {{Severity Emoji}} {{Finding Title}} (`{{file:line}}`)

**Issue:** {{description from review}}

**Fix:** {{actionable suggestion from review}}

## Decisions

{{Include this section only when a finding needs a user decision; otherwise delete it. For each decision item:}}

### {{Finding Title}} (`{{file:line}}`)

**Question:** {{what the user must decide}}

**Options:** {{each option and what it costs}}

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `{{file}}` | {{what to change}} |

## Verification

- [ ] Run existing tests — all pass
- [ ] {{Any additional verification specific to the findings}}
