// The dispatch mod's $.state contract (docs/specs/agent-tree.allium:
// AgentListModPane, AgentTreeModPane).

/** One live agent, as BoardPaneView's `live_agents` lists it. */
export type ListedAgent = { id: number; title: string; tmux_window: string }

export type AgentListNotice = { text: string; source: 'agent_list' | 'agent_jump' }

export type AgentList = {
  /** The session's task id; null when the branch has none, undefined until read. */
  taskId?: number | null
  /** The last successful read, sorted by id; null until one succeeds. */
  agents: ListedAgent[] | null
  notice: AgentListNotice | null
}

/** One drawn tree row, as `dispatch agent-changes` prints it (AgentTreeModRow). */
export type TreeRow = {
  depth: number
  name: string
  path: string
  kind: 'file' | 'directory'
  badge: 'added' | 'modified' | 'deleted' | null
  /** The counts this row draws; null when it draws none. */
  counts: { added: number; removed: number } | null
}

export type AgentCommit = { id: string; subject: string }

export type AgentTree = {
  /** The session's task id; null when the branch has none, undefined until read. */
  taskId?: number | null
  /** waiting until the board names the worktree; unresolved when it names none. */
  resolution: 'waiting' | 'resolved' | 'unresolved'
  root: string | null
  baseBranch: string | null
  /** The last successful tree read for the selected source; null until one succeeds. */
  rows: TreeRow[] | null
  /** The last successful commits read; null until one succeeds. */
  commits: AgentCommit[] | null
  /** null shows unstaged work, a commit id shows that commit. */
  selectedCommit: string | null
  boardNotice: string | null
  treeNotice: string | null
  commitsNotice: string | null
  startupFailure: string | null
}

declare module 'claude-code' {
  interface PluginState {
    dispatch: { agentList: AgentList; agentTree: AgentTree }
  }
}
