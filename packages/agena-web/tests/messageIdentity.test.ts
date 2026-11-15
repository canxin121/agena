import { describe, expect, test } from 'bun:test'
import { entriesFromParts } from '../src/stores/chat/api'
import { messageSenderDetails, messageSenderLabel, normalizeMessageIdentity } from '../src/lib/messageIdentity'
import type { MessageAuthor, MessageOrigin } from '../src/types/chat'

const origin: MessageOrigin = {
  platform: { transport: 'chat', platform: { id: 'telegram', account: 'agena_bot' } },
  conversation: '-100',
  kind: 'group',
  thread: '42',
}
const author = (id: string, display_name?: string): MessageAuthor => ({
  id,
  ...(display_name !== undefined ? { display_name } : {}),
  kind: { kind: 'channel_member', role: 'member' },
})
const part = (id: number, sender?: MessageAuthor) => ({
  part_id: id,
  run_id: id,
  message_id: id,
  message_boundary: true,
  role: 'user',
  kind: 'text',
  state: 'completed',
  created_at_ms: id,
  content: { text: 'hello' },
  ...(sender ? { author: sender, origin } : {}),
  sections: [],
})

describe('message sender identity', () => {
  test('shared group messages preserve each sender and the exact source on history loads', () => {
    const messages = entriesFromParts('3', [part(1, author('77', 'Ann Lee')), part(2, author('88', '陈星'))])
    expect(messages.map((message) => messageSenderLabel(message.info))).toEqual(['Ann Lee', '陈星'])
    expect(messages.map((message) => message.info.author?.id)).toEqual(['77', '88'])
    expect(messages[0]?.info.origin).toEqual(origin)
    expect(messageSenderDetails(messages[0]!.info)).toBe('77 · telegram/agena_bot · -100 · 42')
    expect(messages[0]?.info.runContent).toEqual({ text: 'hello' })
  })

  test('missing names fall back to stable IDs, while old and local messages keep their role', () => {
    const messages = entriesFromParts('3', [part(1, author('77')), part(2)])
    expect(messages.map((message) => messageSenderLabel(message.info))).toEqual(['77', 'user'])
    expect(messageSenderLabel({ role: 'user', author: author('88', '   ') })).toBe('88')
    expect(
      messageSenderLabel({ role: 'user', author: { id: 'instance_operator', kind: { kind: 'instance_operator' } } }),
    ).toBe('user')
    expect(messageSenderLabel({ role: 'assistant', author: author('77', 'Ann') })).toBe('assistant')
  })

  test('malformed optional identity is ignored without losing the message', () => {
    expect(normalizeMessageIdentity({ author: { id: '77', kind: 'user' }, origin: { platform: {} } })).toEqual({})
    expect(normalizeMessageIdentity({ author: author('77', '<admin>') }).author?.kind).toEqual({
      kind: 'channel_member',
      role: 'member',
    })
  })
})
