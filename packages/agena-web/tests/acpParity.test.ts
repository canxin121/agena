import { expect, test } from 'bun:test'
import { isAcpSessionId, sessionApiPath } from '../src/lib/acpSession'
import { isExternalAgentCommandInvocation } from '../src/pages/chat/useChatCommands'
import { buildCreateSessionRequest } from '../src/stores/chat/api'

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
