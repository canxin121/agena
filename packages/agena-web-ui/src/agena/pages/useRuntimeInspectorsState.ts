import type { ComputedRef, Ref } from 'vue'

import type { RuntimeStatus } from '../lib/agenaApi'

export type RuntimeMcpServer = RuntimeStatus['operator']['mcp']['servers'][number]
export type RuntimeLspServer = RuntimeStatus['operator']['lsp']['servers'][number]

export type RuntimeInspectorsStateInput = {
  filteredLspServers: ComputedRef<RuntimeLspServer[]>
  filteredMcpServers: ComputedRef<RuntimeMcpServer[]>
  lspQuery: Ref<string>
  mcpQuery: Ref<string>
  openRuntimeConfigRoot: () => void
  openWorkspacePath: (relativePath?: string | null) => void
  openWorkspaceShortcut: (shortcutId: string) => void
  runtime: Ref<RuntimeStatus | null>
}

export function useRuntimeInspectorsState(input: RuntimeInspectorsStateInput) {
  return {
    filteredLspServers: input.filteredLspServers,
    filteredMcpServers: input.filteredMcpServers,
    lspQuery: input.lspQuery,
    mcpQuery: input.mcpQuery,
    openRuntimeConfigRoot: input.openRuntimeConfigRoot,
    openWorkspacePath: input.openWorkspacePath,
    openWorkspaceShortcut: input.openWorkspaceShortcut,
    runtime: input.runtime,
  }
}
