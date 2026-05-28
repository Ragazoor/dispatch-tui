---
name: find
description: >-
  Search the repository for context relevant to a query. Uses semantic search
  (search_docs MCP) when the repo is indexed; falls back to the Explore agent
  when it is not. Call this before implementing to understand what code is relevant.
---

# Find

Search the repository for context relevant to a query.

**Announce at start:** "I'm using the find skill to locate relevant context for: <query>"

## Step 1: Check for a query argument

If no argument was provided, stop and tell the caller:
> "Usage: /find \<query\> — e.g. /find 'task dispatch flow'"

## Step 2: Semantic search

Call the `search_docs` MCP tool:

```
search_docs(query=<query>, limit=5)
```

Do **not** pass `repo_path`. In a dispatched task the MCP infers it automatically
from the task context. In an interactive session without a task context,
`search_docs` will return an error — treat that as empty results and go to Step 3.

## Step 3: Branch on results

**If `search_docs` returned one or more results:**

1. Collect unique file paths from the results (multiple chunks from the same file → read once).
2. Use the `Read` tool to load each file.
3. Go to Step 4.

**If `search_docs` returned empty results or an error:**

Announce: "Repo not indexed or no semantic matches — searching with Explore agent."

Spawn the `Explore` agent:

```
Agent({
  subagent_type: "Explore",
  description: "Find files relevant to: <query>",
  prompt: "Search the repository for files and code relevant to: '<query>'. Return a list of file paths (relative to the repo root) that contain the most relevant content. Focus on source files (.rs, .md, .allium). Return up to 5 paths."
})
```

Parse the file paths from Explore's response and use the `Read` tool to load each one.
Go to Step 4.

## Step 4: Report

After reading the files, produce a brief report:

```
Found relevant context in:
- <file1>
- <file2>
...

Summary: [2–4 sentences describing what's relevant to the query and where the key logic lives]
```
