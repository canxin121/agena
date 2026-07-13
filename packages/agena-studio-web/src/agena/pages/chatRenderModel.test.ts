import { describe, expect, test } from 'bun:test'

import type { MessagePart, MessageResource } from '@/agena/lib/agenaApi'

import { rewindMessageComposerText } from './chatRenderModel'

function userMessage(parts: MessagePart[]): MessageResource {
  return {
    id: 42,
    session_id: 7,
    role: 'user',
    state: 'completed',
    created_at: '2026-07-13T00:00:00Z',
    updated_at: '2026-07-13T00:00:00Z',
    metadata: {},
    part_count: parts.length,
    parts,
  }
}

function textPart(partIndex: number, text: string, flags: Record<string, unknown> = {}): MessagePart {
  return {
    id: partIndex + 1,
    message_id: 42,
    part_index: partIndex,
    status: 'completed',
    kind: 'text',
    created_at: '2026-07-13T00:00:00Z',
    content: { type: 'text', text, ...flags },
  }
}

describe('rewindMessageComposerText', () => {
  test('restores visible user text in part order', () => {
    const message = userMessage([textPart(2, 'third'), textPart(0, 'first'), textPart(1, 'second')])

    expect(rewindMessageComposerText(message)).toBe('first\n\nsecond\n\nthird')
  })

  test('omits synthetic, ignored, empty, and non-text parts', () => {
    const message = userMessage([
      textPart(0, 'visible'),
      textPart(1, 'generated', { synthetic: true }),
      textPart(2, 'hidden', { ignored: true }),
      textPart(3, '   '),
      {
        id: 5,
        message_id: 42,
        part_index: 4,
        status: 'completed',
        kind: 'attachment',
        created_at: '2026-07-13T00:00:00Z',
        content: { type: 'attachment', attachments: [] },
      },
    ])

    expect(rewindMessageComposerText(message)).toBe('visible')
  })
})
