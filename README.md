# Dispatch

A terminal kanban board that runs Claude Code agents. Each agent gets its own git worktree and tmux window.

Dispatch is a hobby project. I built it to learn how I want to work with agents, and it is still changing as I learn. Take it as one person's experiment, not a finished product.

<!-- The images below come from a made-up demo project. Regenerate them with scripts/readme-demo/run.sh. -->
![A tour of the board: into an epic, into a nested epic, back out, and into a work package](docs/images/tour.gif)

**[Try it in your browser →](https://ragazoor.github.io/dispatch-tui/guide/)** A playable tour of the board, epics and work packages. Nothing to install.

## Why

Agents let me take on more work than before. Dispatch is my way of organising that work so I do not get overwhelmed. These are the questions I have been exploring, in the order they came up.

**How do I keep a lot of work in my head?** I namespace it. Epics work like folders: they hold tasks and other epics, nested as deep as I need. Work packages gather the epics and tasks behind one goal, wherever they sit on the board. Each agent has a task card, and moves that card itself as it starts, finishes or gets stuck.

**How do I work on a project where the code is a black box?** In this project agents write the code, and I mostly do not read it. Instead, all work starts from the specs in [`docs/specs/`](docs/specs/), written in [Allium](https://github.com/juxt/allium). They are the source of truth the agents work from: spec first, then tests, then code. So far that has worked well.

**How do I stay close to code I do care about?** At work the code is not a black box, and I do not want to drift away from it because an agent writes it. The agent-tree pane (`Prefix+e` in an agent's window) shows the files the agent changed. From there one key opens a file's diff and another opens the file in your editor.

**How far can agents orchestrate themselves?** I do some of the orchestration myself from the board. Agents now do a lot more of it. Claude is a first-class user of the board: through MCP tools it can do most of what you can do there. Agents can also send each other messages natively. Auto-dispatch helps too: it starts the next task in an epic when the one before it finishes. With only these tools, a lot of patterns are easy to try, for example:

- fan out work to several agents, then fan their results back in
- let one agent create, dispatch and follow other tasks
- let agents talk to each other with messages to coordinate

## A look around

**The board.** Each column shows what needs you first. Running cards are grouped by what the agent is doing: *needs input* at the top, then *active*. Review cards are grouped by where the work is: *changes requested*, *approved*, then *awaiting review*.

![The board, with an agent waiting for input at the top of Running](docs/images/board.png)

**Epics nest.** An epic card shows how its work is spread across the columns. Press `Space` to step inside; the breadcrumb shows how deep you are. Inside, a card can be another epic or a task. Here, an agent on #1 runs two subagents of its own while #2 waits in Review, approved.

![Inside the nested epic Checkout v2 › Payment methods](docs/images/epic-nested.png)

**Work packages cut across epics.** A package gathers the epics and tasks behind one goal, such as a launch, wherever they live on the board. `Space` opens it as its own board.

![The Black Friday launch work package, holding two epics and a task](docs/images/package.png)

## Requirements

| Tool | | Install |
|---|---|---|
| Rust | only to build from source | [rustup.rs](https://rustup.rs) |
| `tmux` | required | `dnf install tmux` / `brew install tmux` |
| `git` | required | usually already there |
| `claude` | required | [Claude Code CLI](https://claude.ai/code) |
| `gh` | optional | [GitHub CLI](https://cli.github.com), for pull requests |

## Install

Download a prebuilt binary from [Releases](https://github.com/Ragazoor/dispatch-tui/releases). Builds exist for Linux (x86_64, aarch64) and macOS (Apple Silicon). Pick your target:

```bash
target=x86_64-unknown-linux-gnu   # or aarch64-unknown-linux-gnu, aarch64-apple-darwin
curl -fsSL "https://github.com/Ragazoor/dispatch-tui/releases/latest/download/dispatch-$target.tar.gz" | tar -xz
mkdir -p ~/.local/bin && install -m 755 "dispatch-$target/dispatch" ~/.local/bin/dispatch
dispatch tui
```

The Linux binary needs glibc 2.35 or newer and OpenSSL 3. Each archive has a `.sha256` file beside it.

Or build from source:

```bash
git clone https://github.com/Ragazoor/dispatch-tui
cd dispatch-tui
cargo install --path .
dispatch tui
```

The board lives in a SpacetimeDB store.

- **Working alone:** install the [`spacetime` CLI](https://spacetimedb.com/install). That is all. `dispatch tui` starts a local store on `127.0.0.1:3000`, publishes its module, and stops it when the board exits.
- **In a team:** point every board at a shared store: `export DISPATCH_SPACETIME_SERVER=http://team-store:3000`

That is the whole install. If you are not already in tmux, `dispatch tui` starts a session named `dispatch` for itself. Before the board draws, it offers to register the MCP server, plugin and tmux settings. It asks only when something is out of date, so re-run it after an upgrade to stay current.

Run it again later and it restarts the board in the same session. The old board's window closes and a fresh one takes its place. The agent windows beside it keep running.

## Your first task

1. Press `n` and fill in a title, a description, and a repo path.
2. Press `Space`. Dispatch cuts a worktree, opens a tmux window, and starts an agent there.
3. Watch it work: `s` splits the screen, board on the left and agent on the right.
4. The agent moves itself to Review when it is done. Type `/wrap-up` in its session to commit and either rebase onto the base branch or open a draft pull request.

In a hurry? Press `D` instead. It skips the form, dispatches at once, and the agent names the task itself.

## How it works

**Columns** — Backlog → Running → Review → Done. `Space` is the one action key: it dispatches a Backlog task, and jumps to or resumes a Running one.

**Worktrees** — each agent works in `<repo>/.worktrees/<id>-<slug>`. Closing the tmux window keeps the worktree, so `Space` resumes where the agent left off.

**Agent tree** — `Prefix+e` in an agent's tmux window shows or hides a pane with the files the agent changed. `Space` opens a file's diff and `v` opens the file in your editor.

**Plans** — a markdown file describing what to build. Attach one and the agent implements it directly instead of designing first.

**Tags** — labels such as `bug`, `feature`, `chore`, `research`, `pr-review`. Most are just labels. Two change behaviour: `research` launches a research agent, and `pr-review` skips the design step and, given a PR link, cuts the worktree from that PR's branch.

**Epics** — a group of tasks that run in order. Epics can hold other epics. `Space` on an epic card steps inside; `q` steps back out one level and `Q` returns to the board. Inside an epic, turn on auto-dispatch (`U`) and finishing one subtask starts the next, so each worktree is cut from a branch that already holds its predecessor's work.

**Work packages** — a set of epics and tasks that serve one goal. `w` links the selected card to a package; `Space` on a package card opens it.

**Your name** — `u` sets the display name other people see beside what you create; agents can set it with the `set_display_name` tool. Until you set one, people see the first eight characters of your identity.

**Workspaces and people** — `A` opens the admin screen: the workspaces you administer and their members (a super-admin sees every workspace, the super-admins and the private items nobody can see any more). Add people by display name, flip a member between admin and member, create, rename and delete workspaces, and claim the first super-admin. Agents have the same through `list_workspaces`, `create_workspace`, `update_workspace`, `set_workspace_member`, `set_super_admin`, `claim_super_admin`, `list_unseen_items` and `delete_unseen_item`. The rules are in `docs/specs/admin-surfaces.allium`.

**Knowledge base** — agents record what they learn. Dispatch injects the relevant entries into the next agent's prompt.

**Feeds** — a shell command that pulls work onto the board on a schedule, such as open pull requests or CVE alerts.

## Docs

- [Guided tour](docs/guide/index.html) — try the board in your browser
- [Reference](docs/reference.md) — key bindings, configuration, CLI, feeds, troubleshooting
- [MCP](docs/mcp.md) — the tools agents call
- [Specs](docs/specs/) — what the system is meant to do
- [CLAUDE.md](CLAUDE.md) — building, testing, and contributing

## License

[MIT](LICENSE)
