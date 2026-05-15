import { describe, expect, test } from 'bun:test'

import type { MessagePart, MessageResource } from '@/agena/lib/agenaApi'

import { messageBlocks, messageTags, messageUsageFacts, readPayloadMessageId, readPayloadPartId } from './chatRenderModel'

function messagePart(id: number, overrides?: Partial<MessagePart>): MessagePart {
  return {
    id,
    message_id: 1,
    part_index: 0,
    status: 'complete',
    kind: 'output',
    created_at: '2026-05-10T00:00:00Z',
    ...overrides,
  }
}

function message(id: number, overrides?: Partial<MessageResource>): MessageResource {
  return {
    id,
    session_id: 1,
    role: 'assistant',
    state: 'complete',
    created_at: '2026-05-10T00:00:00Z',
    updated_at: '2026-05-10T00:00:00Z',
    metadata: {},
    usage: null,
    finish: null,
    part_count: 1,
    parts: [messagePart(1, { content: { type: 'text', text: 'hello world' } })],
    ...overrides,
  }
}

describe('chatRenderModel', () => {
  test('renders text and apply_patch blocks', () => {
    const textBlocks = messageBlocks(message(1))
    expect(textBlocks).toEqual([{ body: 'hello world', kind: 'text' }])

    const patchMessage = message(2, {
      parts: [
        messagePart(2, {
          content: {
            type: 'tool_execution',
            output_text: 'applied patch',
            details: {
              source: 'custom',
              output: {
                name: 'apply_patch',
                payload: {
                  diff: '--- a/file\n+++ b/file',
                  changes: [{ path: 'file' }],
                },
              },
            },
          },
        }),
      ],
    })

    expect(messageBlocks(patchMessage)).toEqual([
      { body: 'applied patch', kind: 'text' },
      { body: '--- a/file\n+++ b/file', kind: 'diff', summary: 'Patch diff (1 file)' },
    ])
  })

  test('renders summary-only activity parts without fetching full content', () => {
    const summaryOnly = message(4, {
      parts: [
        messagePart(4, {
          kind: 'permission_request',
          summary: 'Awaiting permission: Need to inspect git status',
          has_detail: true,
          content: undefined,
        }),
      ],
    })

    expect(messageBlocks(summaryOnly)).toEqual([{ body: 'Awaiting permission: Need to inspect git status', kind: 'text' }])
  })

  test('extracts tags, usage facts, and timeline message-part ids', () => {
    const tagged = message(3, {
      metadata: { tags: ['tool', '', 'review'] },
      usage: {
        input_tokens: 12,
        output_tokens: 8,
        reasoning_tokens: 4,
        cache_read_tokens: 2,
        cache_write_tokens: 1,
        total_cost: 0.0042,
      },
    })

    expect(messageTags(tagged)).toEqual(['tool', 'review'])
    expect(messageUsageFacts(tagged)).toEqual([
      'in 12',
      'out 8',
      'reasoning 4',
      'cache read 2',
      'cache write 1',
      'cost $0.0042',
    ])
    expect(readPayloadMessageId({ message_id: 42 })).toBe(42)
    expect(readPayloadMessageId({ message_id: '42' })).toBe(null)
    expect(readPayloadPartId({ part_id: 7 })).toBe(7)
    expect(readPayloadPartId({ part_id: '7' })).toBe(null)
  })
})
