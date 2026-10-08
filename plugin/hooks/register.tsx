import type { Register } from 'claude-code'

const PANE = 'dispatch-spike'

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    await $.command.register({
      name: 'dispatch-spike',
      description: 'Show the dispatch task id and board port in a pane',
    })
    return next(e)
  })

  on('command.run', { command: 'dispatch-spike' }, async $ => {
    await $.ui.open({ id: PANE, title: 'Dispatch spike' })
    return { text: 'Spike pane opened.' }
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const { Box, Text } = $.ui.resolve(e)
    const port = (await $.env.get('DISPATCH_PORT')) ?? 'unset'
    const cwd = await $.session.cwd()
    const branch = await $.process.run(['git', 'branch', '--show-current'])
    const taskId = /^\d+/.exec(branch.stdout.trim())?.[0] ?? 'none'
    return (
      <Box flexDirection="column">
        <Text>task id: {taskId}</Text>
        <Text>board port: {port}</Text>
        <Text dimColor>cwd: {cwd}</Text>
      </Box>
    )
  })
}
