import { describe, expect, test } from 'bun:test'

import { acceptedAssistantReplyId, entriesFromParts } from '../src/stores/chat/api'

function runMarker(partId: number, role: string, content: Record<string, unknown>) {
  return {
    part_id: partId,
    kind: 'run',
    role,
    state: 'in_progress',
    content,
    created_at_ms: partId * 10,
  }
}

describe('assistant reply identity', () => {
  test('run markers publish the reply id onto their message', () => {
    const parts = [
      runMarker(1, 'user', { run_kind: 'user_send' }),
      runMarker(2, 'assistant', { run_kind: 'assistant_turn', reply_id: 'r-1', turn_id: 't-1' }),
    ]
    const entries = entriesFromParts('1', parts as never)
    const assistant = entries.find((entry) => String(entry.info?.role) === 'assistant')
    expect(assistant?.info?.replyId).toBe('r-1')
    expect(assistant?.info?.turnId).toBe('t-1')
  })

  test('the accepted send reports the assistant reply it created', () => {
    const state = {
      session: { id: 1 },
      parts: [
        runMarker(1, 'user', { run_kind: 'user_send' }),
        runMarker(2, 'assistant', { run_kind: 'assistant_turn', reply_id: 'r-9' }),
      ],
    } as never
    expect(acceptedAssistantReplyId(state)).toBe('r-9')
  })

  test('a send without an assistant marker yet reports no reply id', () => {
    const state = { session: { id: 1 }, parts: [runMarker(1, 'user', {})] } as never
    expect(acceptedAssistantReplyId(state)).toBeNull()
  })
})
