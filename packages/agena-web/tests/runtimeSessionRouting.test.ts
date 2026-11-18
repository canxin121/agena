import { describe, expect, test } from 'bun:test'
import { readConversationSessionId } from '../src/app/navigation/sessionQuery'

describe('runtime conversation route ownership', () => {
  test.each([
    '/settings',
    '/settings/permissions',
    '/settings/runtime-session',
    '/files',
    '/terminal',
    '/git',
    '/preview',
    '/plugins',
    '/bot',
    '/chatbot',
  ])('%s cannot replace the last conversation through an old session query', (path) => {
    expect(readConversationSessionId({ path, query: { sessionId: '999' }, fullPath: `${path}?sessionId=999` })).toBe('')
  })

  test.each(['/', '/chat', '/assistant', '/integrations'])('%s preserves explicit conversation deep links', (path) => {
    expect(readConversationSessionId({ path, query: { sessionId: ['', '7'] }, fullPath: `${path}?sessionId=7` })).toBe(
      '7',
    )
    expect(readConversationSessionId({ path, query: {}, fullPath: `${path}?sessionId=8#context` })).toBe('8')
  })

  test('query values take precedence only inside a conversation route', () => {
    expect(
      readConversationSessionId({ path: '/chat', query: { sessionId: '7' }, fullPath: '/chat?sessionId=999' }),
    ).toBe('7')
    expect(
      readConversationSessionId({ path: '/settings/permissions', query: {}, fullPath: '/chat?sessionId=999' }),
    ).toBe('')
  })
})
