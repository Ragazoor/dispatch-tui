// The dispatch mod's $.state contract (docs/specs/agent-tree.allium:
// AgentListModPane).

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

declare module 'claude-code' {
  interface PluginState {
    dispatch: { agentList: AgentList }
  }
}
