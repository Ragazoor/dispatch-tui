import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register } from 'claude-code'

import type { AgentList, AgentListNotice, ListedAgent } from '../types'

// The agent list pane: docs/specs/agent-tree.allium, "Agent List Mod Pane".

const PANE = 'dispatch-agents'
const DEFAULT_PORT = '3142'
/** config.agent_tree_agents_refresh_interval */
const REFRESH_MS = 1000
const BOARD_TIMEOUT_MS = 2000
/** tmux::WINDOW_PANE_FORMAT */
const WINDOW_PANE_FORMAT = '#{pane_active} #{pane_id} #{window_name}'

const EMPTY: AgentList = { agents: null, notice: null }
const list = atom({ plugin: 'dispatch', key: 'agentList' } as const, EMPTY)

type $ = EngineInterface

/** The leading digits of the git branch, as the command hooks read it. */
async function branchTaskId($: $): Promise<number | null> {
  const branch = await $.process.run(['git', 'branch', '--show-current'])
  const digits = /^\d+/.exec(branch.stdout.trim())?.[0]
  return digits === undefined ? null : Number(digits)
}

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
  const port = (await $.env.get('DISPATCH_PORT')) || DEFAULT_PORT
  const url = `http://127.0.0.1:${port}/pane-view`
  const ask = $.http.fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ task_id: taskId }),
  })
  // The losing wait is cancelled, so no stray timer outlives the read.
  const giveUp = new AbortController()
  const timeout = $.clock.sleep(BOARD_TIMEOUT_MS, { signal: giveUp.signal }).then(
    () => Promise.reject(new Error(`no answer within ${BOARD_TIMEOUT_MS} ms`)),
    () => new Promise<never>(() => {}),
  )
  const answer = await Promise.race([ask, timeout]).finally(() => giveUp.abort())
  if (!answer.ok) {
    throw new Error(`the board on ${url} answered ${answer.status}`)
  }
  let body: unknown
  try {
    body = JSON.parse(answer.text)
  } catch {
    throw new Error(`the board on ${url} answered something that is not JSON`)
  }
  const agents = (body as { live_agents?: unknown })?.live_agents
  if (!Array.isArray(agents)) {
    throw new Error(`the board on ${url} answered something that is not a pane view`)
  }
  // A row this mod cannot read is skipped, as the companion pane skips one.
  return agents.filter(isListedAgent).sort((a, b) => a.id - b.id)
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

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    await $.command.register({
      name: 'dispatch-agents',
      description: 'List the live dispatch agents and jump to one',
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
}
