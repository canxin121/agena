import { describe, expect, test } from 'bun:test'
import { matchesConversationRoute } from '../src/app/navigation/conversationOwnership'
import { appConversationLink } from '../src/app/navigation/appConversationLink'

describe('conversation routing authority', () => {
  test('an outgoing Bot composer cannot send against an explicit project URL', () => {
    expect(matchesConversationRoute({ path: '/chat', query: { sessionId: '7' } }, '/bot', '42', 'bot')).toBe(false)
    expect(matchesConversationRoute({ path: '/chat', query: { sessionId: '7' } }, '/chat', '42', 'bot')).toBe(false)
    expect(
      matchesConversationRoute({ path: '/chat', query: { sessionId: '7', windowId: 'project' } }, '/chat', '7', 'bot'),
    ).toBe(false)
    expect(matchesConversationRoute({ path: '/chat', query: { sessionId: ['7'] } }, '/chat', '7', 'project')).toBe(true)
    expect(
      matchesConversationRoute(
        { path: '/bot', query: { sessionId: '42', botId: 'atlas', windowId: 'bot' } },
        '/bot',
        '42',
        'bot',
        'atlas',
      ),
    ).toBe(true)
  })

  test('same session with a different Bot identity never authorizes a send', () => {
    expect(
      matchesConversationRoute(
        { path: '/bot', query: { sessionId: '42', botId: 'other' } },
        '/bot',
        '42',
        undefined,
        'atlas',
      ),
    ).toBe(false)
    expect(
      matchesConversationRoute({ path: '/bot', query: { sessionId: '42' } }, '/bot', '42', undefined, 'atlas'),
    ).toBe(false)
  })

  test('only explicit same-origin app conversations bypass Markdown file handling', () => {
    const origin = 'http://127.0.0.1:5173'
    expect(appConversationLink('/chat?sessionId=7#message', origin)).toBe('/chat?sessionId=7#message')
    expect(appConversationLink(`${origin}/chat?sessionId=7`, origin)).toBe('/chat?sessionId=7')
    for (const href of [
      '/chat',
      '/chat?sessionId=',
      '/chat.md?sessionId=7',
      '/project/readme.md',
      'file:///chat?sessionId=7',
      'http://example.com/chat?sessionId=7',
      '//example.com/chat?sessionId=7',
    ]) {
      expect(appConversationLink(href, origin)).toBeNull()
    }
  })
})
