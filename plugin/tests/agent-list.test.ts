// The agent list pane (docs/specs/agent-tree.allium, "Agent List Mod Pane").
// Run with `claude plugin test plugin`.
import { describe, expect, mock, test } from 'claude-code/testing'
import type { On } from 'claude-code'

import type { ListedAgent as Agent } from '../types'

const SURFACES = ['terminal', 'desktop'] as const
const PANE = 'dispatch-agents'

type Board = { answer: () => { status: number; text: string }; asked: { url: string; body?: string }[] }

const agent = (id: number, title = `task ${id}`): Agent => ({
  id,
  title,
  tmux_window: `task-${id}`,
})

const view = (agents: Agent[]) => ({
  status: 200,
  text: JSON.stringify({ task: null, live_agents: agents }),
})

/** The world beneath the mod: a branch, a board, tmux. */
function world(
  on: On,
  options: {
    branch?: string
    board: Board['answer']
    panes?: string
    /** Awaited before the board answers: how a test holds a read open. */
    gate?: () => Promise<void>
  },
) {
  const board: Board = { answer: options.board, asked: [] }
  const ran: string[][] = []
  mock.env(on, { DISPATCH_PORT: '4555' })
  const clock = mock.clock(on)
  on('http.fetch', async (_$, e) => {
    board.asked.push({ url: e.url, body: e.init?.body })
    await options.gate?.()
    const { status, text } = board.answer()
    return { value: { status, ok: status >= 200 && status < 300, headers: {}, text } }
  })
  on('process.run', async (_$, e) => {
    ran.push([...e.argv])
    const [cmd, sub] = e.argv
    const result = (exitCode: number, stdout: string, stderr: string) => ({
      value: { exitCode, stdout, stderr, isStdoutTruncated: false, isStderrTruncated: false },
    })
    const ok = (stdout: string) => result(0, stdout, '')
    if (cmd === 'git') return ok(`${options.branch ?? '28718-mod-agent-list'}\n`)
    if (cmd === 'tmux' && sub === 'list-panes') return ok(options.panes ?? '')
    if (cmd === 'tmux' && sub === 'select-window') return ok('')
    return result(1, '', `unexpected ${e.argv.join(' ')}`)
  })
  on('command.register', async (_$, e) => ({ value: { command: e.name } }))
  on('ui.open', async () => ({ value: { isPlaced: true } }))
  on('ui.close', async () => ({ value: undefined }))
  return { board, ran, clock }
}

/** Closes the pane the way any closer does: a ui.close the mod's hook sees. */
const CLOSER = {
  name: 'closer',
  register(on: On) {
    on('session.start', async ($, e, next) => {
      await $.command.register({ name: 'close-agents', description: 'close' })
      return next(e)
    })
    on('command.run', { command: 'close-agents' }, async $ => {
      await $.ui.close({ id: 'dispatch-agents' })
      return { text: 'closed' }
    })
  },
}

const RUN = { origin: { kind: 'composer' }, presentation: { isFullscreen: true, columns: 160 } } as const

const PROPS = {
  title: 'Agents',
  isFocused: true,
  bodyColumns: 60,
  placement: 'dock',
  scroll: { offset: 0, bodyRows: 20 },
  view: {},
} as const

async function open($: any, surface: (typeof SURFACES)[number]) {
  await $.command.run({ ...RUN, command: 'dispatch-agents', args: '' })
  return $.ui.mount({
    plugin: 'dispatch',
    surface,
    component: 'Pane',
    requestId: PANE,
    props: PROPS,
  })
}

/** The notice line: the one Text drawn in red. */
const notice = async (ui: any): Promise<string | undefined> =>
  (await ui.findAll({ type: 'Text' })).find((t: any) => t.props.color === 'red')?.text

const texts = async (ui: any) => (await ui.findAll({ type: 'Text' })).map((t: any) => t.text)

for (const surface of SURFACES) {
  describe(surface, () => {
    test('lists live agents by id, marks its own task, asks the board on DISPATCH_PORT', async ($, on) => {
      const { board } = world(on, {
        board: () => view([agent(30001, 'Later'), agent(28718, 'Me'), agent(12, 'Early')]),
      })
      const ui = await open($, surface)
      const select = await ui.find({ type: 'Select', key: 'agents' })
      expect(select?.props.options).toEqual([
        { value: '12', label: '#12 Early' },
        { value: '28718', label: '#28718 Me ●' },
        { value: '30001', label: '#30001 Later' },
      ])
      expect(board.asked[0]?.url).toBe('http://127.0.0.1:4555/pane-view')
      expect(JSON.parse(board.asked[0]?.body ?? '{}')).toEqual({ task_id: 28718 })
    })

    test('a failed read keeps the last list and says why', async ($, on) => {
      let up = true
      const { clock } = world(on, {
        board: () => (up ? view([agent(5)]) : { status: 500, text: 'boom' }),
      })
      const ui = await open($, surface)
      up = false
      await clock.advance(1000)
      const select = await ui.find({ type: 'Select', key: 'agents' })
      expect(select?.props.options).toEqual([{ value: '5', label: '#5 task 5' }])
      expect(await notice(ui)).toMatch(/500/)
    })

    test('a working read clears the list notice', async ($, on) => {
      let up = false
      const { clock } = world(on, {
        board: () => (up ? view([agent(5)]) : { status: 503, text: '' }),
      })
      const ui = await open($, surface)
      expect(await notice(ui)).toBeDefined()
      up = true
      await clock.advance(1000)
      expect(await notice(ui)).toBeUndefined()
    })

    test('an unreadable answer is a failure, not an empty list', async ($, on) => {
      world(on, { board: () => ({ status: 200, text: '{"nope":1}' }) })
      const ui = await open($, surface)
      expect(await notice(ui)).toBeDefined()
      expect(await texts(ui)).not.toContain('No agents are running.')
      expect(await texts(ui)).toContain('Reading the board…')
    })

    test('no live agents reads differently from not read yet', async ($, on) => {
      world(on, { board: () => view([]) })
      const ui = await open($, surface)
      expect(await texts(ui)).toContain('No agents are running.')
    })

    test('without a task id it says so and never asks the board', async ($, on) => {
      const { board } = world(on, { branch: 'main', board: () => view([agent(1)]) })
      const ui = await open($, surface)
      expect(await texts(ui)).toContain('Not in a dispatch task worktree.')
      expect(board.asked).toHaveLength(0)
    })

    test('picking an agent selects its exact window by pane id', async ($, on) => {
      const { ran } = world(on, {
        board: () => view([agent(12), agent(123)]),
        panes: '1 %7 task-123\n0 %8 task-123\n1 %3 task-12\n1 %9 some window\n',
      })
      const ui = await open($, surface)
      await ui.select({ key: 'agents', value: '12' })
      expect(ran).toContainEqual(['tmux', 'select-window', '-t', '%3'])
    })

    test('picking its own task does nothing', async ($, on) => {
      const { ran } = world(on, { board: () => view([agent(28718)]) })
      const ui = await open($, surface)
      await ui.select({ key: 'agents', value: '28718' })
      expect(ran.filter(argv => argv[0] === 'tmux')).toHaveLength(0)
    })

    test('a jump to a window that is gone shows a notice until the next pick', async ($, on) => {
      const { ran } = world(on, {
        board: () => view([agent(12), agent(13)]),
        panes: '1 %3 task-13\n',
      })
      const ui = await open($, surface)
      await ui.select({ key: 'agents', value: '12' })
      expect(await notice(ui)).toMatch(/task-12/)
      await ui.select({ key: 'agents', value: '13' })
      expect(await notice(ui)).toBeUndefined()
      expect(ran).toContainEqual(['tmux', 'select-window', '-t', '%3'])
    })

    test('two windows with the name is a failure, not a guess', async ($, on) => {
      const { ran } = world(on, {
        board: () => view([agent(12)]),
        panes: '1 %3 task-12\n1 %4 task-12\n',
      })
      const ui = await open($, surface)
      await ui.select({ key: 'agents', value: '12' })
      expect(await notice(ui)).toMatch(/task-12/)
      expect(ran.filter(argv => argv[1] === 'select-window')).toHaveLength(0)
    })

    test('a slow read is never overlapped by the next tick', async ($, on) => {
      let hang = false
      let release = () => {}
      const { board, clock } = world(on, {
        board: () => view([agent(1)]),
        gate: () => (hang ? new Promise<void>(resolve => (release = resolve)) : Promise.resolve()),
      })
      const ui = await open($, surface)
      hang = true
      await clock.advance(1000)
      const held = board.asked.length
      // The next tick (t=2000) comes before the held read times out (t=3000).
      await clock.advance(1000)
      expect(board.asked.length).toBe(held)
      hang = false
      release()
      await clock.advance(1000)
      expect(board.asked.length).toBeGreaterThan(held)
      expect(await notice(ui)).toBeUndefined()
    })

    test('a closed pane stops reading', { plugins: [CLOSER] }, async ($, on) => {
      const { board, clock } = world(on, { board: () => view([agent(1)]) })
      await open($, surface)
      await $.command.run({ ...RUN, command: 'close-agents', args: '' })
      const asked = board.asked.length
      await clock.advance(5000)
      expect(board.asked.length).toBe(asked)
    })
  })
}

