---
name: voice
description: >-
  Draft a Slack message or team announcement in the operator's own voice, and
  learn from the edits they make to it. Use when the user says "/voice",
  "draft this in my voice", "write this the way I would", or asks for a Slack
  message or team announcement draft.
---

# Voice

**Announce at start:** "I'm using the voice skill to draft this."

Draft one message in the operator's voice, using their voice files. Then learn
from the final text they send.

Do not tidy the user's grammar or capitalisation, in a draft, a sample, an
observation or a rule. Their way of writing is the data.

## Step 1: Load

The voice folder is `$DISPATCH_VOICE_CONTEXT` when that variable is set and not
blank, else `~/.local/share/dispatch/voice-context`. Resolve it with `realpath`
and keep the absolute path: step 3 writes there. The default is often a
symlink, so list and read the files under the resolved path, never the
unresolved one (`find` does not follow a symlinked start path).

Read `rules.md` first, then every file in `samples/`. A missing file is
normal. `observations.md` holds past edits; read it too if it exists.

If the folder does not exist, draft without a voice, say so in one line, and
skip step 3. This holds when `$DISPATCH_VOICE_CONTEXT` names a missing folder
too: do not fall back to the default.

Tell the user which files you loaded, by the resolved path, as its own message
before the draft.

## Step 2: Draft

Take what the message is about from the argument or the conversation. If that
is not enough, ask one question.

Match the samples, not a generic polished tone. Measure them as
`kognic-slides:voice` does: how they open, the length and rhythm of sentences,
what they never do (closers, softeners, exclamation marks).

Show the draft in the chat only. Never post to Slack, and never send or
schedule a message, unless the user asks in this session. Then stop and wait.

## Step 3: Learn

The user pastes the final text. Compare it with your draft: what they cut,
added, reordered or reworded. Propose, in this order:

1. **A sample.** A new file in `samples/` holding the final text verbatim.
   Name it by date and topic, for example `2026-11-17-ai-core-advocates.md`.
2. **An observation.** A new section in `observations.md`: what changed and
   what it shows. Quote the lines.
3. **A rule edit.** Only for a change that shows in two or more messages,
   counting this one and the earlier sections of `observations.md`. A single
   edit goes into `observations.md` only. Match the style of `rules.md`.

Show the user each proposal: the absolute path and the exact lines to write.
Use the path resolved in step 1. Write only after the user approves. That folder is
outside your worktree; the approved write is the only write there this skill
allows. Do not commit in that folder; it belongs to the user.
