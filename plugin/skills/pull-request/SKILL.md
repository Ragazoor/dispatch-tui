---
name: pull-request
description: >-
  A guide to preparing, opening and updating a GitHub pull request so a human
  reviewer can take it in quickly: what belongs in it, its title and body, and
  keeping them true as the branch changes. Use before you open a PR, when
  dispatch's PR gate stops your first PR-creation command, on wrap-up's pr
  path, and when you push more commits to a PR that is already open.
---

# Pull request

A PR is read by a person between other work. Aim for one they can understand, and decide on, in about a minute. Everything below serves that aim, and none of it is a form to fill in: where a point does not fit the change in front of you, use your judgement.

If the repo has its own PR template, or the task or the knowledge base gives PR rules for this repo, those come first. This guide covers what they leave open.

## What goes in

A PR is one change. Before you write anything, read the commit list and the whole diff against the base branch, so you know what actually shipped and what a user will notice. If something unrelated rode along (a drive-by fix, a formatting sweep, a stray file), take it out or give it its own PR. Generated files, snapshots and lock files can stay; when they are most of the diff, one line saying so lets the reviewer skip them.

## Title

Name the change in the imperative, in at most 72 characters, in the repo's commit style. It should still make sense in `git log --oneline` a year from now, so it names the change, not the task.

## Body

There is no template; the body takes the shape of the change. What makes a body quick to take in:

The opening carries the PR. Say the situation and why it matters in a sentence or two of plain words. The dispatch review feed shows only the first 500 characters of a body, so a reader who stops there should still know what this is and why it exists.

Show the change where it has a shape. A table of inputs and outputs, a before/after of an interface or a config, a few lines of example, or a small diagram often says in seconds what a paragraph says slowly. A bug fix may need nothing beyond its two opening sentences. Then say, once and plainly, what a reviewer must not miss: a breaking change, something left out on purpose, a deploy order, a change another team has to make. A heading earns its place when a part is for a different reader, such as that other team.

Leave out what the reviewer already has. The diff shows the files and functions, CI shows the tests, and the files say what they contain. A body that walks through the deliverable section by section is a table of contents, and one that ends with a summary of itself makes the reader read everything twice. The exception is a PR whose point is an argument, such as an ADR or an RFC: reviewers approve the reasoning, so it may belong in the body; ask before you cut it.

A check that works: for each line, would the reviewer miss it if it were gone? If not, cut it.

The operator's standing preferences: short prose and tables rather than bullet lists, and a Test plan section only for a dangerous or breaking change (a migration, auth, billing).

Do not mention the dispatch task, by id or as "Implements #N": GitHub links `#N` to an unrelated issue or PR in the target repo. Notes meant for a reviewing agent rather than a human go in an HTML comment at the end of the body, out of the rendered preview; the attribution footer your harness requires stays last.

As an illustration, not a template: a PR adding a token-check endpoint that the operator liked opened with two sentences on why a gateway needed it, gave a request → response table, a before/after table for the config another team had to change, and one line each for the deploy order and how to check it on staging. It had no summary and no file list.

## Opening it

Push the branch with `git push -u origin <branch>`. If the push is rejected, stop and show the user the error; never force-push without their say.

Create the PR with `gh pr create`, as a draft unless you were told otherwise. Pass `--repo <owner>/<repo>` and `--head <owner>:<branch>` (the owner and repo come from `git remote get-url origin`), so `gh` looks for the branch in that repo and not in your own namespace, and pass the body through a quoted heredoc so its Markdown survives the shell. The command prints the PR's URL. If `gh` says a PR already exists for the branch, that PR is the one: use its URL and edit it rather than open another.

Dispatch's PR gate stops a task's first `gh pr create` once and points here. If you already wrote the PR with this guide, run the command again.

## Keeping it true

When you push more commits (review fixes, a changed scope), reread the title and body against the new diff. Fix what is no longer true and add anything new a reviewer must not miss, with `gh pr edit`. The body describes the PR as it is now, not a log of review rounds.
