import { describe, expect, test } from 'bun:test'

import type { MessagePart, MessageResource } from '@/agena/lib/agenaApi'

import { partBlocks, rewindMessageComposerText } from './chatRenderModel'

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

describe('Skill reference rendering', () => {
  test('renders a compact Skill chip from summary-only message projection', () => {
    const blocks = partBlocks({
      id: 9,
      message_id: 42,
      part_index: 0,
      status: 'completed',
      kind: 'skill_reference',
      name: 'skill_reference',
      summary: 'Skill: review',
      created_at: '2026-07-13T00:00:00Z',
      content: null,
    })

    expect(blocks).toEqual([
      {
        title: 'review',
        body: 'User-selected Skill instructions were attached to this message.',
        kind: 'input_activity',
        activityLabel: 'Skill',
      },
    ])
  })
})

describe('non-execution outcome rendering', () => {
  test('renders policy denial as a warning with rule provenance rather than an error', () => {
    const blocks = partBlocks({
      id: 10,
      message_id: 42,
      part_index: 0,
      status: 'policy_denied',
      kind: 'activity',
      created_at: '2026-07-13T00:00:00Z',
      content: {
        type: 'operation',
        model_output: { text: 'Blocked by the saved workspace rule.' },
        details: {
          payload: {
            denial: {
              source: 'permission_studio',
              scope: 'workspace',
              rule_id: 42,
            },
          },
        },
      },
    })

    expect(blocks).toEqual([
      {
        body: 'Blocked by the saved workspace rule.',
        kind: 'operation_outcome',
        outcome: 'policy_denied',
        title: 'Blocked by permission policy',
        summary: 'permission_studio · workspace · rule #42',
      },
    ])
  })
})
