import { describe, expect, test } from 'bun:test'
import { computed, ref } from 'vue'

import { createRuntimeInspectorPanelState, useRuntimeInspectorPageState } from './useRuntimeInspectorPageState'

describe('useRuntimeInspectorPageState', () => {
  test('assembles inspector panel state from provided runtime source', () => {
    const panel = createRuntimeInspectorPanelState({
      filteredLspServers: computed(() => [
        { name: 'tsserver', command: 'typescript-language-server --stdio', file_extensions: ['ts'], root_markers: ['package.json'] },
      ]),
      filteredMcpServers: computed(() => [{ name: 'filesystem', tool_count: 3 }]),
      lspQuery: ref('ts'),
      mcpQuery: ref('file'),
      openRuntimeConfigRoot: () => {},
      openWorkspacePath: () => {},
      openWorkspaceShortcut: () => {},
      runtime: ref(null),
    })

    expect(panel.lspQuery.value).toBe('ts')
    expect(panel.mcpQuery.value).toBe('file')
    expect(panel.filteredLspServers.value[0]?.name).toBe('tsserver')
    expect(panel.filteredMcpServers.value[0]?.name).toBe('filesystem')
  })

  test('exposes shared shell fields via injected section state', () => {
    const route = { path: '/runtime/mcp' }
    const router = { push: async () => {}, replace: async () => {} }
    const shared = {
      actionError: ref(''),
      actionMessage: ref('ok'),
      load: async () => {},
      loading: ref(false),
      pageDescription: computed(() => 'desc'),
      pageTitle: computed(() => 'title'),
    }

    const result = useRuntimeInspectorPageState(
      { route: route as never, router: router as never },
      {
        useRuntimeSectionState: (value) => {
          expect(value).toEqual({ route, router, section: 'runtime' })
          return {
            shared,
            state: {
              filteredLspServers: computed(() => []),
              filteredMcpServers: computed(() => []),
              lspQuery: ref(''),
              mcpQuery: ref(''),
              openRuntimeConfigRoot: () => {},
              openWorkspacePath: () => {},
              openWorkspaceShortcut: () => {},
              runtime: ref(null),
            },
          }
        },
      },
    )

    expect(result.actionMessage).toBe(shared.actionMessage)
    expect(result.pageTitle).toBe(shared.pageTitle)
    expect(result.pageDescription).toBe(shared.pageDescription)
    expect(result.load).toBe(shared.load)
    expect(result.inspectors.filteredLspServers.value).toEqual([])
    expect(result.inspectors.filteredMcpServers.value).toEqual([])
  })
})
