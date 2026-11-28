import { expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { isAcpSessionId, sessionApiPath } from '../src/lib/acpSession'
import {
  forgetAcpViewBindingsByNative,
  getAcpViewBinding,
  hasAcpViewBinding,
  recoverAcpView,
  rememberAcpViewBinding,
  resetAcpViewRecoveryForTests,
} from '../src/lib/acpViewRecovery'
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

test('recoverAcpView automatically re-opens expired ACP views with singleflight, transient retry, and delete cleanup', async () => {
  resetAcpViewRecoveryForTests()
  try {
    const expiredSid = String(2 ** 52 + 101)
    const recoveredNum = 2 ** 52 + 202
    const recoveredSid = String(recoveredNum)

    rememberAcpViewBinding({
      sessionId: expiredSid,
      workspaceId: 2,
      agent: 'antigravity',
      nativeId: 'native-conv-42',
      title: 'Recovered Agy Chat',
    })
    expect(hasAcpViewBinding(expiredSid)).toBe(true)

    let calls = 0
    const openSession = async (workspaceId: number, agent: string, nativeId: string) => {
      calls++
      expect(workspaceId).toBe(2)
      expect(agent).toBe('antigravity')
      expect(nativeId).toBe('native-conv-42')
      if (calls === 1) throw Object.assign(new Error('Service unavailable'), { status: 503 })
      return recoveredNum
    }

    const [first, second] = await Promise.all([
      recoverAcpView(expiredSid, { openSession, retryDelayMs: 5 }),
      recoverAcpView(expiredSid, { openSession, retryDelayMs: 5 }),
    ])

    expect(calls).toBe(2)
    expect(first).toEqual({ sessionId: recoveredSid, title: 'Recovered Agy Chat' })
    expect(second).toEqual({ sessionId: recoveredSid, title: 'Recovered Agy Chat' })
    expect(getAcpViewBinding(recoveredSid)?.nativeId).toBe('native-conv-42')
    // Follow-up recovery lookup for the same expired ID reuses the migrated ID without calling open again.
    const third = await recoverAcpView(expiredSid, {
      openSession: async () => {
        throw new Error('should not be called again')
      },
    })
    expect(third).toEqual({ sessionId: recoveredSid, title: 'Recovered Agy Chat' })

    // Permanent 404 drops stale binding cleanly.
    const goneSid = String(2 ** 52 + 303)
    rememberAcpViewBinding({
      sessionId: goneSid,
      workspaceId: 2,
      agent: 'antigravity',
      nativeId: 'deleted-native',
    })
    const missing = await recoverAcpView(goneSid, {
      openSession: async () => {
        throw Object.assign(new Error('Not found'), { status: 404 })
      },
      retryDelayMs: 5,
    })
    expect(missing).toBeNull()
    expect(hasAcpViewBinding(goneSid)).toBe(false)

    // Explicit delete forgets bindings by native identity.
    forgetAcpViewBindingsByNative(2, 'antigravity', 'native-conv-42')
    expect(hasAcpViewBinding(recoveredSid)).toBe(false)
    expect(hasAcpViewBinding(expiredSid)).toBe(false)
  } finally {
    resetAcpViewRecoveryForTests()
  }
})
