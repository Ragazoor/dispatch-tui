// The agent tree pane (docs/specs/agent-tree.allium, "Agent Tree Mod Pane").
// Run with `claude plugin test plugin`.
import { describe, expect, mock, test } from 'claude-code/testing'
import type { On } from 'claude-code'

import type { TreeRow } from '../types'

const SURFACES = ['terminal', 'desktop'] as const
const PANE = 'dispatch-tree'
const WORKTREE = '/repo/.worktrees/28719-tree'

type Task = { worktree: string | null; base_branch: string } | null
type Commit = { id: string; subject: string }
type Half<K extends string, V> = { [key in K]: V } | { error: string }
type Report = { tree: Half<'rows', TreeRow[]>; commits: Half<'commits', Commit[]> }
type Run = { exitCode: number; stdout: string; stderr?: string }

const file = (path: string, depth: number, badge: TreeRow['badge'], counts: TreeRow['counts'] = null): TreeRow => ({
  depth,
  name: path.split('/').pop() ?? path,
  path,
  kind: 'file',
  badge,
  counts,
})
const dir = (path: string, depth: number, name = path.split('/').pop() ?? path, counts: TreeRow['counts'] = null): TreeRow => ({
  depth,
  name,
  path,
  kind: 'directory',
  badge: null,
  counts,
})

const ROWS = [
  file('new.txt', 0, 'added'),
  dir('src', 0),
  file('src/lib.rs', 1, 'modified', { added: 1, removed: 1 }),
  dir('src/tui/ui', 1, 'tui/ui', { added: 2, removed: 1 }),
  file('src/tui/ui/a.rs', 2, 'deleted', { added: 0, removed: 3 }),
]
const FIRST: Commit = { id: 'abc1234def', subject: 'first' }
const SECOND: Commit = { id: 'fed9876abc', subject: 'second' }

const report = (rows: TreeRow[] = ROWS, commits: Commit[] = [SECOND, FIRST]): Report => ({
  tree: { rows },
  commits: { commits },
})
const printed = (r: Report): Run => ({ exitCode: 0, stdout: JSON.stringify(r) })

/** The world beneath the mod: a branch, a board, the dispatch binary. */
function world(
  on: On,
  options: {
    branch?: string | (() => string)
    task?: () => { status: number; task?: Task }
    dispatch: (argv: string[]) => Run
    /** Awaited before dispatch answers: how a test holds a run open. */
    gate?: () => Promise<void>
  },
) {
  const asked: string[] = []
  const runs: string[][] = []
  mock.env(on, { DISPATCH_PORT: '4555' })
  const clock = mock.clock(on)
  const task = options.task ?? (() => ({ status: 200, task: { worktree: WORKTREE, base_branch: 'main' } }))
  on('http.fetch', async (_$, e) => {
    asked.push(e.url)
    const { status, task: t } = task()
    const text = JSON.stringify({ task: t ?? null, live_agents: [] })
    return { value: { status, ok: status >= 200 && status < 300, headers: {}, text } }
  })
  on('process.run', async (_$, e) => {
    const result = (exitCode: number, stdout: string, stderr = '') => ({
      value: { exitCode, stdout, stderr, isStdoutTruncated: false, isStderrTruncated: false },
    })
    if (e.argv[0] === 'git') {
      const branch = typeof options.branch === 'function' ? options.branch() : options.branch
      return result(0, `${branch ?? '28719-mod-agent-tree'}\n`)
    }
    if (e.argv[0] === 'dispatch') {
      runs.push([...e.argv])
      await options.gate?.()
      const { exitCode, stdout, stderr } = options.dispatch([...e.argv])
      return result(exitCode, stdout, stderr)
    }
    return result(1, '', `unexpected ${e.argv.join(' ')}`)
  })
  on('command.register', async (_$, e) => ({ value: { command: e.name } }))
  on('ui.open', async () => ({ value: { isPlaced: true } }))
  on('ui.close', async () => ({ value: undefined }))
  return { asked, runs, clock }
}

const CLOSER = {
  name: 'closer',
  register(on: On) {
    on('session.start', async ($, e, next) => {
      await $.command.register({ name: 'close-tree', description: 'close' })
      return next(e)
    })
    on('command.run', { command: 'close-tree' }, async $ => {
      await $.ui.close({ id: 'dispatch-tree' })
      return { text: 'closed' }
    })
  },
}

const RUN = { origin: { kind: 'composer' }, presentation: { isFullscreen: true, columns: 160 } } as const

const PROPS = {
  title: 'Changes',
  isFocused: true,
  bodyColumns: 60,
  placement: 'dock',
  scroll: { offset: 0, bodyRows: 40 },
  view: {},
} as const

async function open($: any, surface: (typeof SURFACES)[number]) {
  await $.command.run({ ...RUN, command: 'dispatch-tree', args: '' })
  return $.ui.mount({ plugin: 'dispatch', surface, component: 'Pane', requestId: PANE, props: PROPS })
}

const texts = async (ui: any): Promise<string[]> =>
  (await ui.findAll({ type: 'Text' })).map((t: any) => t.text)
/** The notice lines: red, and wrapped rather than cut. */
const notices = async (ui: any): Promise<string[]> =>
  (await ui.findAll({ type: 'Text' }))
    .filter((t: any) => t.props.color === 'red' && t.props.wrap === 'wrap')
    .map((t: any) => t.text)
/** The drawn tree rows, each as one line of text: the lines cut at the edge. */
const rows = async (ui: any): Promise<string[]> =>
  (await ui.findAll({ type: 'Text' })).filter((t: any) => t.props.wrap === 'truncate').map((t: any) => t.text)

const ARGS = ['dispatch', 'agent-changes', '--root', WORKTREE, '--base', 'main']

for (const surface of SURFACES) {
  describe(surface, () => {
    test('draws the tree with badges, counts and indentation, for unstaged work', async ($, on) => {
      const { runs, asked } = world(on, { dispatch: () => printed(report()) })
      const ui = await open($, surface)
      expect(await rows(ui)).toEqual([
        'new.txt [Added]',
        'src',
        '  lib.rs [Modified] +1 -1',
        '  tui/ui +2 -1',
        '    a.rs [Deleted] +0 -3',
      ])
      expect(await texts(ui)).toContain('Unstaged work')
      expect(runs[0]).toEqual(ARGS)
      expect(asked[0]).toBe('http://127.0.0.1:4555/pane-view')
    })

    test('without a task id it says so and reads nothing', async ($, on) => {
      const { runs, asked } = world(on, { branch: 'main', dispatch: () => printed(report()) })
      const ui = await open($, surface)
      expect(await texts(ui)).toContain('Not in a dispatch task worktree.')
      expect(asked).toHaveLength(0)
      expect(runs).toHaveLength(0)
    })

    test('waits for a task the board does not hold yet, then reads', async ($, on) => {
      let arrived = false
      const { runs, clock } = world(on, {
        task: () => ({ status: 200, task: arrived ? { worktree: WORKTREE, base_branch: 'main' } : null }),
        dispatch: () => printed(report()),
      })
      const ui = await open($, surface)
      expect(await texts(ui)).toContain('Waiting for task 28719…')
      expect(runs).toHaveLength(0)
      arrived = true
      await clock.advance(1000)
      expect(await rows(ui)).toHaveLength(ROWS.length)
    })

    test('a board that is down while waiting says why', async ($, on) => {
      const { runs } = world(on, { task: () => ({ status: 502 }), dispatch: () => printed(report()) })
      const ui = await open($, surface)
      expect(await texts(ui)).toContain('Waiting for task 28719…')
      expect((await notices(ui)).join(' ')).toMatch(/502/)
      expect(runs).toHaveLength(0)
    })

    test('a task with no worktree says so and runs nothing', async ($, on) => {
      const { runs, clock } = world(on, {
        task: () => ({ status: 200, task: { worktree: null, base_branch: 'main' } }),
        dispatch: () => printed(report()),
      })
      const ui = await open($, surface)
      await clock.advance(3000)
      expect(await texts(ui)).toContain('Task has no worktree.')
      expect(runs).toHaveLength(0)
    })

    test('the board is asked once; the tree keeps working without it', async ($, on) => {
      let up = true
      const { asked, runs, clock } = world(on, {
        task: () => (up ? { status: 200, task: { worktree: WORKTREE, base_branch: 'main' } } : { status: 500 }),
        dispatch: () => printed(report()),
      })
      const ui = await open($, surface)
      up = false
      await clock.advance(3000)
      expect(asked).toHaveLength(1)
      expect(runs.length).toBeGreaterThan(1)
      expect(await notices(ui)).toEqual([])
    })

    test('a failed tree read keeps the last tree and says why, until it recovers', async ($, on) => {
      let broken = false
      const { clock } = world(on, {
        dispatch: () =>
          printed(broken ? { tree: { error: 'index.lock exists' }, commits: { commits: [] } } : report()),
      })
      const ui = await open($, surface)
      broken = true
      await clock.advance(1000)
      expect(await rows(ui)).toHaveLength(ROWS.length)
      expect((await notices(ui)).join(' ')).toMatch(/index\.lock/)
      broken = false
      await clock.advance(1000)
      expect(await notices(ui)).toEqual([])
    })

    test('a failed commits read leaves the tree drawn and says why', async ($, on) => {
      world(on, {
        dispatch: () => printed({ tree: { rows: ROWS }, commits: { error: 'unknown revision main' } }),
      })
      const ui = await open($, surface)
      expect(await rows(ui)).toHaveLength(ROWS.length)
      expect((await notices(ui)).join(' ')).toMatch(/unknown revision/)
    })

    test('no changes reads differently from not read yet', async ($, on) => {
      let ok = false
      const { clock } = world(on, {
        dispatch: () => (ok ? printed(report([])) : { exitCode: 2, stdout: '', stderr: "unrecognized subcommand 'agent-changes'" }),
      })
      const ui = await open($, surface)
      expect(await texts(ui)).toContain('Reading changes…')
      expect((await notices(ui)).join(' ')).toMatch(/agent-changes/)
      ok = true
      await clock.advance(1000)
      expect(await texts(ui)).toContain('No unstaged changes.')
      expect(await texts(ui)).not.toContain('Reading changes…')
    })

    test('output that is not a report is a failure, not an empty tree', async ($, on) => {
      world(on, { dispatch: () => ({ exitCode: 0, stdout: 'hello' }) })
      const ui = await open($, surface)
      expect(await notices(ui)).not.toEqual([])
      expect(await texts(ui)).toContain('Reading changes…')
    })

    test('a report with one malformed half fails both halves', async ($, on) => {
      world(on, {
        dispatch: () => ({
          exitCode: 0,
          stdout: JSON.stringify({ tree: { rows: [{ depth: 0, name: 'a', path: 'a', kind: 'file', badge: 'added' }] }, commits: { commits: [] } }),
        }),
      })
      const ui = await open($, surface)
      expect(await notices(ui)).toHaveLength(2)
      expect(await texts(ui)).toContain('Reading changes…')
      expect(await ui.find({ type: 'Select', key: 'source' })).toBeUndefined()
    })

    test('a task with no base branch is a failed board read, not main', async ($, on) => {
      const { runs } = world(on, {
        task: () => ({ status: 200, task: { worktree: WORKTREE } as any }),
        dispatch: () => printed(report()),
      })
      const ui = await open($, surface)
      expect(await texts(ui)).toContain('Waiting for task 28719…')
      expect((await notices(ui)).join(' ')).toMatch(/base branch/)
      expect(runs).toHaveLength(0)
    })

    test('a reopen on another task starts over', async ($, on) => {
      let branch = '28719-mod-agent-tree'
      const { asked } = world(on, { branch: () => branch, dispatch: () => printed(report()) })
      await $.command.run({ ...RUN, command: 'dispatch-tree', args: '' })
      branch = '30000-other'
      const ui = await open($, surface)
      expect(asked).toHaveLength(2)
      expect(await rows(ui)).toHaveLength(ROWS.length)
    })

    test('the picker lists unstaged work then commits, and picking one shows it', async ($, on) => {
      const { runs } = world(on, {
        dispatch: argv =>
          printed(argv.includes('--commit') ? report([file('a.txt', 0, 'added', { added: 1, removed: 0 })]) : report()),
      })
      const ui = await open($, surface)
      const picker = await ui.find({ type: 'Select', key: 'source' })
      expect(picker?.props.options).toEqual([
        { value: '', label: 'Unstaged work ●' },
        { value: SECOND.id, label: 'fed9876 second' },
        { value: FIRST.id, label: 'abc1234 first' },
      ])
      await ui.select({ key: 'source', value: FIRST.id })
      expect(runs.at(-1)).toEqual([...ARGS, '--commit', FIRST.id])
      expect(await rows(ui)).toEqual(['a.txt [Added] +1 -0'])
      expect(await texts(ui)).toContain('Commit abc1234 first')
      const after = await ui.find({ type: 'Select', key: 'source' })
      expect(after?.props.options[2]).toEqual({ value: FIRST.id, label: 'abc1234 first ●' })
    })

    test('a commit with no changes says so', async ($, on) => {
      world(on, { dispatch: argv => printed(argv.includes('--commit') ? report([]) : report()) })
      const ui = await open($, surface)
      await ui.select({ key: 'source', value: FIRST.id })
      expect(await texts(ui)).toContain('No changes in this commit.')
    })

    test('a selected commit that leaves the list falls back to unstaged work', async ($, on) => {
      let rewritten = false
      const { runs, clock } = world(on, {
        dispatch: () => printed(report(ROWS, rewritten ? [SECOND] : [SECOND, FIRST])),
      })
      const ui = await open($, surface)
      await ui.select({ key: 'source', value: FIRST.id })
      rewritten = true
      await clock.advance(1000)
      await clock.advance(1000)
      expect(runs.at(-1)).toEqual(ARGS)
      expect(await texts(ui)).toContain('Unstaged work')
      expect(await notices(ui)).toEqual([])
    })

    test('an answer for the old source is never drawn under the new one', async ($, on) => {
      let hang = false
      let release = () => {}
      world(on, {
        dispatch: argv =>
          printed(argv.includes('--commit') ? report([file('a.txt', 0, 'added')]) : report()),
        gate: () => (hang ? new Promise<void>(resolve => (release = resolve)) : Promise.resolve()),
      })
      const ui = await open($, surface)
      hang = true
      const picked = ui.select({ key: 'source', value: FIRST.id })
      expect(await rows(ui)).toEqual([])
      hang = false
      release()
      await picked
      expect(await rows(ui)).toEqual(['a.txt [Added]'])
    })

    test('a slow run is never overlapped by the next tick', async ($, on) => {
      let hang = false
      let release = () => {}
      const { runs, clock } = world(on, {
        dispatch: () => printed(report()),
        gate: () => (hang ? new Promise<void>(resolve => (release = resolve)) : Promise.resolve()),
      })
      await open($, surface)
      hang = true
      await clock.advance(1000)
      const held = runs.length
      await clock.advance(1000)
      expect(runs.length).toBe(held)
      hang = false
      release()
      await clock.advance(1000)
      expect(runs.length).toBeGreaterThan(held)
    })

    test('a closed pane stops reading', { plugins: [CLOSER] }, async ($, on) => {
      const { runs, clock } = world(on, { dispatch: () => printed(report()) })
      await open($, surface)
      await $.command.run({ ...RUN, command: 'close-tree', args: '' })
      const ran = runs.length
      await clock.advance(5000)
      expect(runs.length).toBe(ran)
    })
  })
}
