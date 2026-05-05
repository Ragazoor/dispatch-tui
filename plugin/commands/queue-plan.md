---
description: Queue a plan file as a Ready task in the Dispatch kanban board
allowed-tools: Bash, Glob, Read, mcp__dispatch__create_task
---

Queue a plan as a task in Dispatch.

## Instructions

1. **Find the plan file:**
   - If an argument was provided ("$ARGUMENTS"), use that as the plan file path
   - Otherwise, use Glob to find the most recently modified `.md` file in `docs/superpowers/plans/` or `docs/plans/`
   - If no plan file is found, ask the user for the path

2. **Resolve to an absolute path** for the plan file (use Bash `realpath` or equivalent if the path is relative).

3. **Determine the repo path:** use the current working directory (`pwd` via Bash).

4. **Create the task via the Dispatch MCP server:** call `mcp__dispatch__create_task` with:
   - `repo_path`: the absolute repo path from step 3
   - `plan_path`: the absolute plan path from step 2

   The MCP handler reads the plan file, extracts the title and description, and creates a backlog task.

5. **Report the result to the user.** Show the task ID and title from the MCP response.
