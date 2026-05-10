import { describe, expect, test } from 'bun:test'
import { computed, ref } from 'vue'

import type { ChatUsageSummary } from '../pages/chatUsageModel'
import { createChatCommandCatalog } from './chatCommandCatalog'
import { createCommandPalette } from './commandPalette'

function createState() {
  const selectedWorkspaceId = ref<number | null>(1)
  const selectedSessionId = ref<number | null>(7)
  const sessions = ref([
    {
      id: 7,
      workspace_id: 1,
      title: 'Session 7',
      version: 1,
      created_at: '2026-05-10T00:00:00Z',
      updated_at: '2026-05-10T00:00:00Z',
      message_count: 3,
      child_session_count: 0,
    },
  ])
  const workspaces = ref([
    {
      id: 1,
      path: '/workspace',
      created_at: '2026-05-10T00:00:00Z',
      updated_at: '2026-05-10T00:00:00Z',
      session_count: 1,
    },
  ])
  const sessionImportJsonl = ref('')
  const sessionTreeRows = ref([{ session: sessions.value[0]!, depth: 0 }])
  const rewindCheckpoints = ref([{ id: 'cp1' }])
  const ancestorSessions = ref<typeof sessions.value>([])
  const sessionUsageSummary = ref<ChatUsageSummary>({
    turns: 2,
    inputTokens: 100,
    outputTokens: 50,
    reasoningTokens: 0,
    cacheWriteTokens: 0,
    cacheReadTokens: 0,
    totalCostUsd: 0.0123,
    byModel: [
      {
        providerId: 'anthropic',
        modelId: 'claude-sonnet',
        turns: 2,
        inputTokens: 100,
        outputTokens: 50,
        reasoningTokens: 0,
        cacheWriteTokens: 0,
        cacheReadTokens: 0,
        totalCostUsd: 0.0123,
      },
    ],
  })

  return {
    selectedWorkspaceId: computed(() => selectedWorkspaceId.value),
    selectedSessionId: computed(() => selectedSessionId.value),
    sessions: computed(() => sessions.value),
    workspaces: computed(() => workspaces.value),
    sessionImportJsonl: computed(() => sessionImportJsonl.value),
    sessionTreeRows: computed(() => sessionTreeRows.value),
    rewindCheckpoints: computed(() => rewindCheckpoints.value),
    ancestorSessions: computed(() => ancestorSessions.value),
    sessionUsageSummary: computed(() => sessionUsageSummary.value),
    refs: {
      sessionImportJsonl,
    },
  }
}

function createRouterStub() {
  return {
    push: async () => {},
  }
}

function createOpenRuntimeSectionSpy() {
  const calls: Array<{ section: string; tab: string }> = []
  return {
    calls,
    openRuntimeSection: (section: string, tab: string) => {
      calls.push({ section, tab })
    },
  }
}

describe('chatCommandCatalog', () => {
  test('shows usage notice for invalid open-session input', async () => {
    const state = createState()
    const notices: string[] = []
    const commands = createChatCommandCatalog(state, {
      openWorkspaceBrowser: () => {},
      openRuntimeSection: () => {},
      openSessionById: async () => false,
      setNewSessionTitle: () => {},
      createSessionAction: async () => {},
      continueCurrentSession: async () => {},
      forkCurrentSession: async () => {},
      exportCurrentSession: async () => {},
      importSessionFromJsonl: async () => {},
      selectWorkspace: async () => {},
      resolveWorkspaceAction: async () => {},
      setWorkspacePath: () => {},
      loadSessionTree: async () => {},
      loadRewindCheckpoints: async () => {},
      setLocalCommandNotice: (value) => {
        notices.push(value)
      },
    })

    const command = commands.find((item) => item.id === 'chat.open-session')
    await command?.run({ input: '/open-session nope', args: ['nope'] })

    expect(notices.at(-1)).toBe('Usage: /open-session <session-id>')
  })

  test('blocks import without jsonl and opens workspace shortcuts', async () => {
    const state = createState()
    const notices: string[] = []
    const openedPaths: string[] = []
    const commands = createChatCommandCatalog(state, {
      openWorkspaceBrowser: (relativePath) => {
        openedPaths.push(relativePath || '')
      },
      openRuntimeSection: () => {},
      openSessionById: async () => true,
      setNewSessionTitle: () => {},
      createSessionAction: async () => {},
      continueCurrentSession: async () => {},
      forkCurrentSession: async () => {},
      exportCurrentSession: async () => {},
      importSessionFromJsonl: async () => {},
      selectWorkspace: async () => {},
      resolveWorkspaceAction: async () => {},
      setWorkspacePath: () => {},
      loadSessionTree: async () => {},
      loadRewindCheckpoints: async () => {},
      setLocalCommandNotice: (value) => {
        notices.push(value)
      },
    })

    await commands.find((item) => item.id === 'chat.import-session')?.run()
    expect(notices.at(-1)).toBe('Paste session JSONL before running /import-session.')

    await commands.find((item) => item.id === 'workspace-shortcut.commands')?.run()
    expect(openedPaths.length > 0).toBe(true)
    expect((notices.at(-1) || '').includes('Opened workspace path')).toBe(true)
  })

  test('palette query passes args into parameterized catalog command', async () => {
    const state = createState()
    const opened: number[] = []
    const runtimeNavigation = createOpenRuntimeSectionSpy()
    const commands = createChatCommandCatalog(state, {
      openWorkspaceBrowser: () => {},
      openRuntimeSection: runtimeNavigation.openRuntimeSection as never,
      openSessionById: async (sessionId) => {
        opened.push(sessionId)
        return true
      },
      setNewSessionTitle: () => {},
      createSessionAction: async () => {},
      continueCurrentSession: async () => {},
      forkCurrentSession: async () => {},
      exportCurrentSession: async () => {},
      importSessionFromJsonl: async () => {},
      selectWorkspace: async () => {},
      resolveWorkspaceAction: async () => {},
      setWorkspacePath: () => {},
      loadSessionTree: async () => {},
      loadRewindCheckpoints: async () => {},
      setLocalCommandNotice: () => {},
    })
    const palette = createCommandPalette({
      router: createRouterStub() as never,
      runtimeSkills: computed(() => []),
      runtimeCommands: computed(() => []),
      localCommands: computed(() => commands),
    })

    palette.openPalette()
    palette.query.value = '/open-session 8'
    await palette.runHighlighted()

    expect(opened).toEqual([8])
  })

  test('reuses shared section navigation metadata for runtime destinations', async () => {
    const state = createState()
    const runtimeNavigation = createOpenRuntimeSectionSpy()
    const commands = createChatCommandCatalog(state, {
      openWorkspaceBrowser: () => {},
      openRuntimeSection: runtimeNavigation.openRuntimeSection as never,
      openSessionById: async () => true,
      setNewSessionTitle: () => {},
      createSessionAction: async () => {},
      continueCurrentSession: async () => {},
      forkCurrentSession: async () => {},
      exportCurrentSession: async () => {},
      importSessionFromJsonl: async () => {},
      selectWorkspace: async () => {},
      resolveWorkspaceAction: async () => {},
      setWorkspacePath: () => {},
      loadSessionTree: async () => {},
      loadRewindCheckpoints: async () => {},
      setLocalCommandNotice: () => {},
    })

    await commands.find((item) => item.id === 'chat.runtime.workflow')?.run()
    await commands.find((item) => item.id === 'chat.settings.desktop')?.run()

    expect(runtimeNavigation.calls).toEqual([
      { section: 'runtime', tab: 'workflow' },
      { section: 'settings', tab: 'desktop' },
    ])
  })
})
