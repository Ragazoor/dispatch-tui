import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register } from 'claude-code'

import type { AgentCommit, AgentList, AgentListNotice, AgentTree, ListedAgent, TreeRow } from '../types'

// The agent list pane: docs/specs/agent-tree.allium, "Agent List Mod Pane".

const PANE = 'dispatch-agents'
/** config.agent_tree_agents_refresh_interval */
const REFRESH_MS = 1000
/** tmux::WINDOW_PANE_FORMAT */
const WINDOW_PANE_FORMAT = '#{pane_active} #{pane_id} #{window_name}'

const EMPTY: AgentList = { agents: null, notice: null }
const list = atom({ plugin: 'dispatch', key: 'agentList' } as const, EMPTY)

type $ = EngineInterface

// == Shared: the session's task id and the board's pane view (BoardPaneView) ==

const DEFAULT_PORT = '3142'
const BOARD_TIMEOUT_MS = 2000

/** The leading digits of the git branch, as the command hooks read it. */
async function branchTaskId($: $): Promise<number | null> {
  const branch = await $.process.run(['git', 'branch', '--show-current'])
  const digits = /^\d+/.exec(branch.stdout.trim())?.[0]
  return digits === undefined ? null : Number(digits)
}

/**
 * Races `work` against the mod's own wait. The losing wait is cancelled, so
 * no stray timer outlives the read.
 */
async function within<T>($: $, ms: number, work: Promise<T>): Promise<T> {
  const giveUp = new AbortController()
  const timeout = $.clock.sleep(ms, { signal: giveUp.signal }).then(
    () => Promise.reject(new Error(`no answer within ${ms} ms`)),
    () => new Promise<never>(() => {}),
  )
  return Promise.race([work, timeout]).finally(() => giveUp.abort())
}

/**
 * BoardPaneView's pane_view(taskId), as a parsed object; throws on any failed
 * read, including an answer that is not a pane view.
 */
async function paneView($: $, taskId: number): Promise<{ task: unknown; live_agents: unknown[] }> {
  const port = (await $.env.get('DISPATCH_PORT')) || DEFAULT_PORT
  const url = `http://127.0.0.1:${port}/pane-view`
  const ask = $.http.fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ task_id: taskId }),
  })
  const answer = await within($, BOARD_TIMEOUT_MS, ask)
  if (!answer.ok) {
    throw new Error(`the board on ${url} answered ${answer.status}`)
  }
  let body: unknown
  try {
    body = JSON.parse(answer.text)
  } catch {
    throw new Error(`the board on ${url} answered something that is not JSON`)
  }
  const view = body as { task?: unknown; live_agents?: unknown }
  if (typeof view !== 'object' || view === null || !Array.isArray(view.live_agents)) {
    throw new Error(`the board on ${url} answered something that is not a pane view`)
  }
  return { task: view.task ?? null, live_agents: view.live_agents }
}

// == The agent list pane ==

function isListedAgent(value: unknown): value is ListedAgent {
  const a = value as ListedAgent
  return (
    typeof a === 'object' &&
    a !== null &&
    typeof a.id === 'number' &&
    typeof a.title === 'string' &&
    typeof a.tmux_window === 'string'
  )
}

/** BoardPaneView's pane_view(taskId).live_agents; throws on any failed read. */
async function liveAgents($: $, taskId: number): Promise<ListedAgent[]> {
  const { live_agents } = await paneView($, taskId)
  // A row this mod cannot read is skipped, as the companion pane skips one.
  return live_agents.filter(isListedAgent).sort((a, b) => a.id - b.id)
}

/** AgentListModRowShowsIdAndTitle: `#<id> <title>`, the own task dotted. */
const rowLabel = (agent: ListedAgent, taskId: number | null | undefined) =>
  `#${agent.id} ${agent.title}${agent.id === taskId ? ' ●' : ''}`

/**
 * Sets `source`'s notice to `text`, or with null clears it only if `source`
 * set it. Writes nothing when nothing changes, so an idle pane is not redrawn.
 */
async function setNotice($: $, source: AgentListNotice['source'], text: string | null) {
  const { notice } = await read($, list)
  const next = text !== null ? { text, source } : notice?.source === source ? null : notice
  if (next?.text === notice?.text && next?.source === notice?.source) return
  await update($, list, l => ({ ...l, notice: next }))
}

/** RefreshAgentListModPane / AgentListModPaneFailureKeepsLastList. */
async function refresh($: $) {
  // One read at a time: a slow answer must never land after a newer one.
  if (reading) return
  const { taskId, agents: shown } = await read($, list)
  if (taskId == null) return
  reading = true
  try {
    const agents = await liveAgents($, taskId)
    if (JSON.stringify(agents) !== JSON.stringify(shown)) {
      await update($, list, l => ({ ...l, agents }))
    }
    await setNotice($, 'agent_list', null)
  } catch (error) {
    await setNotice($, 'agent_list', `Could not read the agent list: ${(error as Error).message}`)
  } finally {
    reading = false
  }
}

/**
 * The pane id of the window named exactly `window`, across every tmux session.
 * `select-window -t <name>` would match by prefix (task-12 picks task-123).
 */
async function windowTarget($: $, window: string): Promise<string> {
  const listed = await $.process.run(['tmux', 'list-panes', '-a', '-F', WINDOW_PANE_FORMAT])
  if (listed.exitCode !== 0) throw new Error(`tmux could not list windows: ${listed.stderr.trim()}`)
  const [pane, ...more] = listed.stdout.split('\n').flatMap(line => {
    const m = /^1 (\S+) (.*)$/.exec(line)
    return m?.[1] !== undefined && m[2] === window ? [m[1]] : []
  })
  if (pane === undefined) throw new Error(`no tmux window named '${window}'`)
  if (more.length > 0) throw new Error(`more than one tmux window named '${window}'`)
  return pane
}

/** JumpFromAgentListModPane / AgentListModPaneJumpFailureIsVisible. */
async function jump($: $, id: number) {
  const { agents, taskId } = await read($, list)
  const target = agents?.find(a => a.id === id)
  if (target === undefined || target.id === taskId) return
  try {
    const pane = await windowTarget($, target.tmux_window)
    const selected = await $.process.run(['tmux', 'select-window', '-t', pane])
    if (selected.exitCode !== 0) {
      throw new Error(`tmux could not select '${target.tmux_window}': ${selected.stderr.trim()}`)
    }
    await setNotice($, 'agent_jump', null)
  } catch (error) {
    await setNotice($, 'agent_jump', (error as Error).message)
  }
}

// The refresh timer runs while the pane is drawn. A reload of the module
// starts over with no timer, and the next draw starts one.
let timer: { cancel: () => void } | null = null
let reading = false

function startReading($: $) {
  if (timer !== null) return
  timer = $.clock.every(REFRESH_MS, () => void refresh($))
}

function stopReading() {
  timer?.cancel()
  timer = null
}

// == The agent tree pane: docs/specs/agent-tree.allium, "Agent Tree Mod Pane".
// The git work is the dispatch binary's (`dispatch agent-changes`); this
// module asks for it and draws the answer.

const TREE_PANE = 'dispatch-tree'
/** config.agent_tree_refresh_interval */
const TREE_REFRESH_MS = 1000
/** Above the binary's own git steps (config.agent_tree_git_timeout each). */
const RUN_TIMEOUT_MS = 15000
/** CommitRowShowsShortIdAndSubject */
const SHORT_ID = 7
const UNSTAGED = ''

const EMPTY_TREE: AgentTree = {
  resolution: 'waiting',
  root: null,
  baseBranch: null,
  rows: null,
  commits: null,
  selectedCommit: null,
  boardNotice: null,
  treeNotice: null,
  commitsNotice: null,
  startupFailure: null,
}
const tree = atom({ plugin: 'dispatch', key: 'agentTree' } as const, EMPTY_TREE)

type Half<T> = { ok: true; value: T } | { ok: false; error: string }

const BADGE = {
  added: { text: '[Added]', color: 'green' },
  modified: { text: '[Modified]', color: 'yellow' },
  deleted: { text: '[Deleted]', color: 'red' },
} as const

const isCounts = (c: unknown) =>
  c === null ||
  (typeof c === 'object' &&
    typeof (c as { added?: unknown }).added === 'number' &&
    typeof (c as { removed?: unknown }).removed === 'number')

const isRow = (r: unknown): r is TreeRow => {
  const row = r as TreeRow
  return (
    typeof row === 'object' &&
    row !== null &&
    typeof row.depth === 'number' &&
    typeof row.name === 'string' &&
    typeof row.path === 'string' &&
    (row.kind === 'file' || row.kind === 'directory') &&
    (row.badge === null || Object.hasOwn(BADGE, row.badge)) &&
    isCounts(row.counts)
  )
}

const isCommit = (c: unknown): c is AgentCommit => {
  const commit = c as AgentCommit
  return typeof commit === 'object' && commit !== null && typeof commit.id === 'string' && typeof commit.subject === 'string'
}

const NOT_A_REPORT = 'dispatch agent-changes printed something that is not a report'

/**
 * One half of the printed report: its list, or the reason it failed; null
 * when the half is not in the report's shape, which fails the whole report.
 */
function half<T>(value: unknown, key: string, isItem: (v: unknown) => v is T): Half<T[]> | null {
  const obj = value as Record<string, unknown> | null
  if (typeof obj?.error === 'string') return { ok: false, error: obj.error }
  const list = obj?.[key]
  if (!Array.isArray(list) || !list.every(isItem)) return null
  return { ok: true, value: list }
}

/** AgentChangesCommand: both halves, each failing on its own. */
async function agentChanges(
  $: $,
  root: string,
  baseBranch: string,
  commit: string | null,
): Promise<{ tree: Half<TreeRow[]>; commits: Half<AgentCommit[]> }> {
  const argv = ['dispatch', 'agent-changes', '--root', root, '--base', baseBranch]
  if (commit !== null) argv.push('--commit', commit)
  try {
    // The run rejects once it overruns timeoutMs.
    const run = await $.process.run(argv, { timeoutMs: RUN_TIMEOUT_MS })
    if (run.exitCode !== 0) {
      const why = run.stderr.trim().split('\n')[0] || `exit ${run.exitCode}`
      throw new Error(`dispatch agent-changes failed: ${why}`)
    }
    let printed: { tree?: unknown; commits?: unknown } | null
    try {
      printed = JSON.parse(run.stdout)
    } catch {
      throw new Error('dispatch agent-changes printed something that is not JSON')
    }
    const tree = half(printed?.tree, 'rows', isRow)
    const commits = half(printed?.commits, 'commits', isCommit)
    // A run that printed something that is not this report fails both halves.
    if (tree === null || commits === null) throw new Error(NOT_A_REPORT)
    return { tree, commits }
  } catch (error) {
    const failed = { ok: false, error: (error as Error).message } as const
    return { tree: failed, commits: failed }
  }
}

/**
 * Merges `patch` into the tree state, writing nothing when nothing changes,
 * so an idle pane is not redrawn every tick.
 */
async function patchTree($: $, patch: Partial<AgentTree>) {
  const current = await read($, tree)
  const next = { ...current, ...patch }
  if (JSON.stringify(next) === JSON.stringify(current)) return
  await update($, tree, t => ({ ...t, ...patch }))
}

/** AgentTreeModPaneResolvesItsTask and the two rules beside it. */
async function resolveTask($: $, taskId: number) {
  let task: { worktree?: unknown; base_branch?: unknown } | null
  try {
    task = (await paneView($, taskId)).task as typeof task
  } catch (error) {
    return patchTree($, { boardNotice: (error as Error).message })
  }
  if (task === null || typeof task !== 'object') return patchTree($, { boardNotice: null })
  const { worktree, base_branch } = task
  // PaneTask.base_branch is never null: an answer without one is not a pane view.
  if (typeof base_branch !== 'string') {
    return patchTree($, { boardNotice: 'the board answered a task with no base branch' })
  }
  if (typeof worktree !== 'string') {
    return patchTree($, { resolution: 'unresolved', startupFailure: 'Task has no worktree.', boardNotice: null })
  }
  return patchTree($, { resolution: 'resolved', root: worktree, baseBranch: base_branch, boardNotice: null })
}

/** RefreshAgentTreeModPane: one run, two answers, for the selected source. */
async function readChanges($: $) {
  const { root, baseBranch, selectedCommit } = await read($, tree)
  if (root === null || baseBranch === null) return
  const answer = await agentChanges($, root, baseBranch, selectedCommit)
  const now = await read($, tree)
  // An answer for a source that is no longer selected is dropped.
  if (now.selectedCommit !== selectedCommit) return
  const commits = answer.commits.ok ? answer.commits.value : now.commits
  const commitsNotice = answer.commits.ok ? null : answer.commits.error
  // A selected commit that left a working list falls back to unstaged work;
  // what this run drew was that commit's tree, so it is not shown.
  if (answer.commits.ok && selectedCommit !== null && !answer.commits.value.some(c => c.id === selectedCommit)) {
    return patchTree($, { commits, commitsNotice, selectedCommit: null, rows: null, treeNotice: null })
  }
  return patchTree($, {
    rows: answer.tree.ok ? answer.tree.value : now.rows,
    treeNotice: answer.tree.ok ? null : answer.tree.error,
    commits,
    commitsNotice,
  })
}

async function refreshTree($: $) {
  // One read at a time: a slow answer must never land after a newer one.
  if (treeReading) return
  treeReading = true
  try {
    for (;;) {
      const { taskId, resolution, selectedCommit } = await read($, tree)
      if (taskId == null) return
      if (resolution === 'waiting') await resolveTask($, taskId)
      if ((await read($, tree)).resolution !== 'resolved') return
      await readChanges($)
      // The source changed while the run was out: read the new one now.
      if ((await read($, tree)).selectedCommit === selectedCommit) return
    }
  } finally {
    treeReading = false
  }
}

/** SelectAgentTreeModSource */
async function selectSource($: $, value: string) {
  const commit = value === UNSTAGED ? null : value
  const { selectedCommit, commits } = await read($, tree)
  if (commit === selectedCommit) return
  if (commit !== null && !commits?.some(c => c.id === commit)) return
  await update($, tree, t => ({ ...t, selectedCommit: commit, rows: null, treeNotice: null }))
  await refreshTree($)
}

// The refresh timer runs while the pane is drawn. A reload of the module
// starts over with no timer, and the next draw starts one.
let treeTimer: { cancel: () => void } | null = null
let treeReading = false

function startTreeReading($: $) {
  if (treeTimer !== null) return
  treeTimer = $.clock.every(TREE_REFRESH_MS, () => void refreshTree($))
}

function stopTreeReading() {
  treeTimer?.cancel()
  treeTimer = null
}

const shortId = (id: string) => id.slice(0, SHORT_ID)
const commitLabel = (c: AgentCommit) => `${shortId(c.id)} ${c.subject}`


/** AgentTreeModRowsReadAsTheCompanionPanes: one row as one line. */
function Row(Text: any, row: TreeRow) {
  const cut = row.kind === 'directory' ? row.name.lastIndexOf('/') + 1 : 0
  const badge = row.badge === null ? null : BADGE[row.badge]
  return (
    <Text key={`row:${row.path}`} wrap="truncate">
      {'  '.repeat(row.depth)}
      {cut > 0 && <Text dimColor>{row.name.slice(0, cut)}</Text>}
      {row.name.slice(cut)}
      {badge !== null && [' ', <Text color={badge.color}>{badge.text}</Text>]}
      {row.counts !== null && [
        ' ',
        <Text color="green">{`+${row.counts.added}`}</Text>,
        ' ',
        <Text color="red">{`-${row.counts.removed}`}</Text>,
      ]}
    </Text>
  )
}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    await $.command.register({
      name: 'dispatch-agents',
      description: 'List the live dispatch agents and jump to one',
    })
    await $.command.register({
      name: 'dispatch-tree',
      description: "Show the files this task's agent changed",
    })
    return next(e)
  })

  // OpenAgentListModPane
  on('command.run', { command: 'dispatch-agents' }, async $ => {
    const taskId = await branchTaskId($)
    await update($, list, l => ({ ...l, taskId }))
    await $.ui.open({ id: PANE, title: 'Agents' })
    if (taskId !== null) {
      // A fresh timer starts with the next draw.
      stopReading()
      await refresh($)
    }
    return { text: 'Agents pane opened.' }
  })

  // CloseAgentListModPane
  on('ui.close', async ($, e, next) => {
    if (e.id === PANE) stopReading()
    // CloseAgentTreeModPane
    if (e.id === TREE_PANE) stopTreeReading()
    return next(e)
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const elements = $.ui.resolve(e)
    const { Box, Text } = elements
    const { taskId, agents, notice } = await read($, list)
    if (taskId === null) {
      return (
        <Box flexDirection="column">
          <Text dimColor>Not in a dispatch task worktree.</Text>
        </Box>
      )
    }
    if (taskId !== undefined) startReading($)

    const empty = agents === null ? 'Reading the board…' : 'No agents are running.'
    return (
      <Box flexDirection="column">
        {notice !== null && (
          <Text color="red" wrap="wrap">
            {notice.text}
          </Text>
        )}
        {agents === null || agents.length === 0 ? (
          <Text dimColor>{empty}</Text>
        ) : 'Select' in elements ? (
          <elements.Select
            key="agents"
            label="Jump to"
            options={agents.map(a => ({ value: String(a.id), label: rowLabel(a, taskId) }))}
            onSelect={(value: string) => void jump($, Number(value))}
          />
        ) : (
          // A surface with no Select (mobile) lists the rows without the jump.
          agents.map(a => <Text>{rowLabel(a, taskId)}</Text>)
        )}
      </Box>
    )
  })


  // OpenAgentTreeModPane
  on('command.run', { command: 'dispatch-tree' }, async $ => {
    const taskId = await branchTaskId($)
    // A reopen on another task starts over; on the same task it keeps the
    // last tree and its resolution.
    await update($, tree, t => (t.taskId === undefined || t.taskId === taskId ? { ...t, taskId } : { ...EMPTY_TREE, taskId }))
    await $.ui.open({ id: TREE_PANE, title: 'Changes' })
    if (taskId !== null) {
      // A fresh timer starts with the next draw.
      stopTreeReading()
      await refreshTree($)
    }
    return { text: 'Changes pane opened.' }
  })

  on('ui.render', { component: 'Pane', requestId: TREE_PANE }, async ($, e) => {
    const elements = $.ui.resolve(e)
    const { Box, Text } = elements
    const state = await read($, tree)
    const { taskId, resolution, rows, commits, selectedCommit } = state
    if (taskId === null) {
      return (
        <Box flexDirection="column">
          <Text dimColor>Not in a dispatch task worktree.</Text>
        </Box>
      )
    }
    if (taskId !== undefined && resolution !== 'unresolved') startTreeReading($)

    const notices = [state.boardNotice, state.treeNotice, state.commitsNotice].filter(
      (n): n is string => n !== null,
    )
    const selected = commits?.find(c => c.id === selectedCommit)
    const source = selectedCommit === null ? 'Unstaged work' : `Commit ${selected ? commitLabel(selected) : shortId(selectedCommit)}`
    const empty = selectedCommit === null ? 'No unstaged changes.' : 'No changes in this commit.'

    let body
    if (resolution === 'waiting') {
      body = <Text dimColor>{`Waiting for task ${taskId}…`}</Text>
    } else if (resolution === 'unresolved') {
      body = <Text dimColor>{state.startupFailure ?? 'Task has no worktree.'}</Text>
    } else {
      body = (
        <Box flexDirection="column">
          <Text bold>{source}</Text>
          {rows === null ? (
            <Text dimColor>Reading changes…</Text>
          ) : rows.length === 0 ? (
            <Text dimColor>{empty}</Text>
          ) : (
            rows.map(row => Row(Text, row))
          )}
          {commits !== null && 'Select' in elements && (
            <elements.Select
              key="source"
              label="Show"
              value={selectedCommit ?? UNSTAGED}
              options={[
                { value: UNSTAGED, label: `Unstaged work${selectedCommit === null ? ' ●' : ''}` },
                ...commits.map(c => ({
                  value: c.id,
                  label: `${commitLabel(c)}${c.id === selectedCommit ? ' ●' : ''}`,
                })),
              ]}
              onSelect={(value: string) => void selectSource($, value)}
            />
          )}
        </Box>
      )
    }

    return (
      <Box flexDirection="column">
        {notices.map(text => (
          <Text color="red" wrap="wrap">
            {text}
          </Text>
        ))}
        {body}
      </Box>
    )
  })
}
