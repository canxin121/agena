import { expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { isAcpSessionId, sessionApiPath } from '../src/lib/acpSession'
import { isExternalAgentCommandInvocation } from '../src/pages/chat/useChatCommands'
import { buildCreateSessionRequest } from '../src/stores/chat/api'
import { useUiStore } from '../src/stores/ui'

test('isAcpSessionId and sessionApiPath isolate ephemeral ACP views from durable sessions', () => {
  const acpId = String(2 ** 52)
  const acpUpper = String(2 ** 53 - 1)
  expect(isAcpSessionId('42')).toBe(false)
  expect(isAcpSessionId(acpId)).toBe(true)
  expect(isAcpSessionId(acpUpper)).toBe(true)
  expect(sessionApiPath('42')).toBe('/api/v1/sessions/42')
  expect(sessionApiPath(acpId)).toBe(`/api/v1/acp/sessions/${acpId}`)
})

test('isExternalAgentCommandInvocation matches qualified /agent: commands only', () => {
  expect(isExternalAgentCommandInvocation('/agent:compact')).toBe(true)
  expect(isExternalAgentCommandInvocation('/AGENT:review --all')).toBe(true)
  expect(isExternalAgentCommandInvocation('//agent:compact')).toBe(false)
  expect(isExternalAgentCommandInvocation('/compact')).toBe(false)
  expect(isExternalAgentCommandInvocation('/agent:/nested')).toBe(false)
})

test('buildCreateSessionRequest preserves autoTitle and externalAgent for ACP sessions', () => {
  const payload = buildCreateSessionRequest({
    workspaceId: 7,
    title: 'Claude Code',
    externalAgent: 'claude-code',
    autoTitle: true,
  })
  expect(payload).toEqual({
    workspace_id: 7,
    title: 'Claude Code',
    auto_title: true,
    external_agent: 'claude-code',
  })
})

test('ACP workspace windows open distinct tabs per sessionId and reuse matching tabs', () => {
  const storageDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'localStorage')
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: undefined })
  const pinia = createPinia()
  setActivePinia(pinia)
  try {
    const ui = useUiStore(pinia)
    const sidA = String(2 ** 52 + 1)
    const sidB = String(2 ** 52 + 2)

    const tabA = ui.openWorkspaceWindow('acp', {
      path: '/acp',
      query: { sessionId: sidA },
      title: 'Session A',
      matchKeys: ['sessionId'],
      reuseExisting: true,
    })
    const tabB = ui.openWorkspaceWindow('acp', {
      path: '/acp',
      query: { sessionId: sidB },
      title: 'Session B',
      matchKeys: ['sessionId'],
      reuseExisting: true,
    })
    const tabAAgain = ui.openWorkspaceWindow('acp', {
      path: '/acp',
      query: { sessionId: sidA },
      title: 'Session A Updated',
      matchKeys: ['sessionId'],
      reuseExisting: true,
    })

    expect(tabA).not.toBe(tabB)
    expect(tabAAgain).toBe(tabA)
    const acpTabs = ui.workspaceWindows.filter((item) => item.mainTab === 'acp')
    expect(acpTabs).toHaveLength(2)
    expect(acpTabs.find((item) => item.id === tabA)?.title).toBe('Session A Updated')
    expect(acpTabs.find((item) => item.id === tabB)?.title).toBe('Session B')
    expect(acpTabs.find((item) => item.id === tabA)?.routeQuery.sessionId).toBe(sidA)
    expect(acpTabs.find((item) => item.id === tabB)?.routeQuery.sessionId).toBe(sidB)
  } finally {
    disposePinia(pinia)
    setActivePinia(undefined)
    if (storageDescriptor) Object.defineProperty(globalThis, 'localStorage', storageDescriptor)
    else Reflect.deleteProperty(globalThis, 'localStorage')
  }
})
