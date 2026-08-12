import { describe, expect, test } from 'bun:test'

import type { MessagePart, MessageResource, SessionPart, UserInputRequest } from '@/agena/lib/agenaApi'

import { partBlocks, partsToMessages, pendingInteractionParts, rewindMessageComposerText } from './chatRenderModel'

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

type SessionPartFixture = Partial<Omit<SessionPart, 'part_id' | 'kind' | 'role' | 'content'>> & {
  part_id: number
  kind: SessionPart['kind']
  role: SessionPart['role']
  content?: Record<string, unknown>
}

function sessionPart(partial: SessionPartFixture): SessionPart {
  return {
    state: 'completed',
    content: {},
    created_at_ms: 1_800_000_000_000,
    ...partial,
  }
}

function runPart(partId: number, runKind = 'user_send', overrides: Partial<SessionPart> = {}): SessionPart {
  return sessionPart({
    part_id: partId,
    kind: 'run',
    role: 'user',
    content: { run_kind: runKind },
    ...overrides,
  })
}

/** The single-activity ask shape: a tool_call part whose flattened content
 * carries `operation.user_input` with one unanswered request. */
function operationAskContent(requestOverrides: Partial<UserInputRequest> = {}): Record<string, unknown> {
  return {
    name: 'host.ask_user',
    input: { question: '…' },
    operation: {
      user_input: {
        requests: [
          {
            request: {
              request_id: 'host-input:1:2:0',
              session_id: 7,
              title: 'Approve New Plan',
              body_markdown: '## Proposed Plan',
              kind: 'review',
              source: 'host',
              questions: [],
              created_at: '2026-07-13T00:00:00Z',
              ...requestOverrides,
            },
            reply: null,
            replied_at_ms: null,
          },
        ],
      },
    },
  }
}

describe('v2 parts projection (partsToMessages)', () => {
  test('groups content parts under their run marker, ordered by created_at_ms then part_id', () => {
    const parts: SessionPart[] = [
      sessionPart({ part_id: 103, kind: 'tool_call', role: 'assistant', run_id: 100, content: { name: 'fs.read' }, created_at_ms: 1_800_000_000_003 }),
      sessionPart({ part_id: 101, kind: 'text', role: 'user', run_id: 100, content: { text: 'Fix it.' }, created_at_ms: 1_800_000_000_001 }),
      runPart(100),
      sessionPart({ part_id: 102, kind: 'think', role: 'assistant', run_id: 100, content: { summary: ['Let me look.'] }, created_at_ms: 1_800_000_000_002 }),
      sessionPart({ part_id: 104, kind: 'tool_result', role: 'tool', run_id: 100, parent_part_id: 103, content: { output: 'ok', ok: true }, created_at_ms: 1_800_000_000_004 }),
    ]

    const messages = partsToMessages(parts, 7)

    expect(messages.length).toBe(1)
    expect(messages[0]?.id).toBe('run:100')
    expect(messages[0]?.role).toBe('user')
    expect(messages[0]?.metadata.run_part_id).toBe(100)
    expect(messages[0]?.parts?.map((part) => part.kind)).toEqual([
      'run',
      'text',
      'think',
      'tool_call',
      'tool_result',
    ])
  })

  test('produces one message per run marker and sorts run groups canonically', () => {
    const parts: SessionPart[] = [
      runPart(200, 'continue', { created_at_ms: 1_800_000_000_010 }),
      sessionPart({ part_id: 201, kind: 'text', role: 'assistant', run_id: 200, content: { text: 'Follow-up.' }, created_at_ms: 1_800_000_000_011 }),
      sessionPart({ part_id: 101, kind: 'text', role: 'user', run_id: 100, content: { text: 'First.' }, created_at_ms: 1_800_000_000_001 }),
      runPart(100, 'user_send', { created_at_ms: 1_800_000_000_000 }),
    ]

    const messages = partsToMessages(parts, 7)

    expect(messages.map((message) => message.id)).toEqual(['run:100', 'run:200'])
    expect(messages[1]?.metadata.run_kind).toBe('continue')
  })

  test('still renders content parts whose run marker is missing locally', () => {
    const parts: SessionPart[] = [
      sessionPart({ part_id: 301, kind: 'text', role: 'assistant', run_id: 300, content: { text: 'Streamed before marker.' }, created_at_ms: 1_800_000_000_001 }),
      sessionPart({ part_id: 401, kind: 'notice', role: 'runtime', content: { kind: 'hook', summary: 'done' }, created_at_ms: 1_800_000_000_002 }),
    ]

    const messages = partsToMessages(parts, 7)

    expect(messages.length).toBe(2)
    expect(messages[0]?.parts?.map((part) => part.kind)).toEqual(['text'])
    expect(messages[1]?.parts?.map((part) => part.kind)).toEqual(['notice'])
  })

  test('maps a terminal run state onto the message and keeps usage from run content', () => {
    const parts: SessionPart[] = [
      runPart(500, 'user_send', {
        state: 'failed',
        content: { run_kind: 'user_send', abort_reason: 'provider_error', usage: { requests: 1, input_tokens: 10 } },
      }),
    ]

    const messages = partsToMessages(parts, 7)

    expect(messages[0]?.state).toBe('failed')
    expect(messages[0]?.usage).toEqual({ requests: 1, input_tokens: 10 })
    expect(rewindMessageComposerText(messages[0]!)).toBe('')
  })

  test('projects userInput from a tool_call awaiting an operation ask', () => {
    const parts: SessionPart[] = [
      runPart(600),
      sessionPart({
        part_id: 601,
        kind: 'tool_call',
        role: 'assistant',
        run_id: 600,
        state: 'in_progress',
        content: operationAskContent(),
        created_at_ms: 1_800_000_000_001,
      }),
    ]

    const messages = partsToMessages(parts, 7)
    const toolPart = messages[0]?.parts?.find((part) => part.kind === 'tool_call')
    expect(toolPart?.userInput?.request_id).toBe('host-input:1:2:0')
    expect(toolPart?.userInput?.kind).toBe('review')
    expect(toolPart?.userInput?.body_markdown).toBe('## Proposed Plan')
  })
})

describe('v2 part rendering (4.1.1 kinds)', () => {
  test('renders think parts as a Reasoning markdown block', () => {
    const blocks = partBlocks({
      id: 1,
      message_id: 'run:1',
      part_index: 0,
      status: 'completed',
      kind: 'think',
      created_at: '2026-07-13T00:00:00Z',
      content: { summary: ['First thought.', 'Second thought.'] },
    })

    expect(blocks).toEqual([{ body: 'First thought.\nSecond thought.', kind: 'markdown', title: 'Reasoning' }])
  })

  test('renders tool_call and tool_result parts', () => {
    const callBlocks = partBlocks({
      id: 2,
      message_id: 'run:1',
      part_index: 1,
      status: 'in_progress',
      kind: 'tool_call',
      created_at: '2026-07-13T00:00:00Z',
      content: { name: 'fs.read', input: { path: 'README.md' } },
    })
    expect(callBlocks[0]?.title).toBe('fs.read')
    expect(callBlocks[0]?.kind).toBe('markdown')

    const resultBlocks = partBlocks({
      id: 3,
      message_id: 'run:1',
      part_index: 2,
      status: 'completed',
      kind: 'tool_result',
      created_at: '2026-07-13T00:00:00Z',
      content: { output: '# README\n', ok: true },
    })
    expect(resultBlocks[0]?.title).toBe('Result')
    expect(resultBlocks[0]?.body).toBe('# README\n')
  })

  test('renders no tool block for a tool_call awaiting a host ask (it is the form)', () => {
    const blocks = partBlocks({
      id: 5,
      message_id: 'run:1',
      part_index: 1,
      status: 'in_progress',
      kind: 'tool_call',
      created_at: '2026-07-13T00:00:00Z',
      content: operationAskContent(),
      userInput: {
        request_id: 'host-input:1:2:0',
        session_id: 7,
        title: 'Approve New Plan',
        body_markdown: '## Proposed Plan',
        kind: 'review',
        source: 'host',
        questions: [],
        created_at: '2026-07-13T00:00:00Z',
      },
    })

    expect(blocks).toEqual([])
  })

  test('renders notice, compaction, and error parts as labelled blocks', () => {
    const notice = partBlocks({
      id: 4,
      message_id: 'run:1',
      part_index: 0,
      status: 'completed',
      kind: 'notice',
      created_at: '2026-07-13T00:00:00Z',
      content: { kind: 'hook_started', summary: 'Pre-commit hook', detail: 'Running lints…' },
    })
    expect(notice[0]?.activityLabel).toBe('Notice')
    expect(notice[0]?.title).toBe('hook_started')

    const hook = partBlocks({
      id: 40,
      message_id: 'run:1',
      part_index: 0,
      status: 'completed',
      kind: 'hook',
      created_at: '2026-07-13T00:00:00Z',
      content: {
        hook: 'agent.stop',
        summary: 'agent.stop hook blocked stop: workflow plan autorun',
        detail: '<plan_context>continue with the next plan step</plan_context>',
        message: '<plan_context>continue with the next plan step</plan_context>',
      },
    })
    expect(hook[0]?.activityLabel).toBe('Hook')
    expect(hook[0]?.body).toContain('continue with the next plan step')

    const compaction = partBlocks({
      id: 5,
      message_id: 'run:1',
      part_index: 0,
      status: 'completed',
      kind: 'compaction',
      created_at: '2026-07-13T00:00:00Z',
      content: { summary: 'Summarized 120 messages.', window: [1, 120] },
    })
    expect(compaction[0]?.activityLabel).toBe('Compaction')

    const error = partBlocks({
      id: 6,
      message_id: 'run:1',
      part_index: 0,
      status: 'failed',
      kind: 'error',
      created_at: '2026-07-13T00:00:00Z',
      content: { category: 'provider', message: 'Request timed out.', detail: {} },
    })
    expect(error[0]?.activityLabel).toBe('Error')
    expect(error[0]?.title).toBe('provider')
    expect(error[0]?.body).toBe('Request timed out.')
  })

  test('renders a pending interaction as a waiting block', () => {
    const blocks = partBlocks({
      id: 7,
      message_id: 'run:1',
      part_index: 0,
      status: 'in_progress',
      kind: 'interaction',
      created_at: '2026-07-13T00:00:00Z',
      content: { type: 'ask_user', prompt: 'Proceed with the write?', options: ['yes', 'no'], response: null },
    })

    expect(blocks[0]?.activityLabel).toBe('Waiting')
    expect(blocks[0]?.title).toBe('Proceed with the write?')
  })

  test('renders the run marker as a group header block', () => {
    const blocks = partBlocks({
      id: 8,
      message_id: 'run:1',
      part_index: 0,
      status: 'in_progress',
      kind: 'run',
      created_at: '2026-07-13T00:00:00Z',
      content: { run_kind: 'user_send' },
    })

    expect(blocks[0]?.activityLabel).toBe('Run')
    expect(blocks[0]?.title).toBe('User run')
  })
})

describe('pendingInteractionParts', () => {
  function interactionPart(overrides: Partial<MessagePart> = {}): MessagePart {
    return {
      id: 90,
      message_id: 42,
      part_index: 0,
      status: 'in_progress',
      kind: 'interaction',
      created_at: '2026-07-13T00:00:00Z',
      content: {
        type: 'review',
        prompt: 'Approve New Plan',
        request: {
          request_id: 'host-input:1:2:0',
          session_id: 7,
          title: 'Approve New Plan',
          body_markdown: '## Proposed Plan',
          kind: 'review',
          source: 'host',
        },
      },
      ...overrides,
    }
  }

  test('returns only pending or in-progress interaction parts', () => {
    const message = userMessage([
      interactionPart({ id: 1, part_index: 0, status: 'in_progress' }),
      interactionPart({ id: 2, part_index: 1, status: 'pending' }),
      interactionPart({ id: 3, part_index: 2, status: 'completed' }),
      { ...interactionPart({ id: 4, part_index: 3, status: 'in_progress' }), kind: 'text' },
    ])

    const pending = pendingInteractionParts(message)
    expect(pending.map((part) => part.id)).toEqual([1, 2])
  })

  test('preserves per-part identity and carries the typed request body', () => {
    const part = interactionPart({ status: 'in_progress' })
    const message = userMessage([part])

    const pending = pendingInteractionParts(message)
    expect(pending.length).toBe(1)
    expect(pending[0]?.id).toBe(part.id)
    const request = (pending[0]?.content as Record<string, unknown>)?.request as Record<string, unknown>
    expect(request.body_markdown).toBe('## Proposed Plan')
    expect(request.kind).toBe('review')
  })

  test('recognizes interaction content even when the kind field is not set', () => {
    const part = interactionPart({ kind: 'activity', content: { type: 'interaction', prompt: 'Pick' } })
    const message = userMessage([part])

    expect(pendingInteractionParts(message).length).toBe(1)
  })

  test('treats a tool_call awaiting an operation user_input as the interaction part', () => {
    const part: MessagePart = {
      id: 91,
      message_id: 42,
      part_index: 0,
      status: 'in_progress',
      kind: 'tool_call',
      created_at: '2026-07-13T00:00:00Z',
      content: operationAskContent(),
      userInput: {
        request_id: 'host-input:1:2:0',
        session_id: 7,
        title: 'Approve New Plan',
        body_markdown: '## Proposed Plan',
        kind: 'review',
        source: 'host',
        questions: [],
        created_at: '2026-07-13T00:00:00Z',
      },
    }
    const answered: MessagePart = {
      ...part,
      id: 92,
      userInput: null,
      content: {
        ...operationAskContent(),
        operation: {
          user_input: {
            requests: [
              {
                request: {
                  request_id: 'host-input:1:2:0',
                  title: 'Approve New Plan',
                  kind: 'review',
                  source: 'host',
                  questions: [],
                  created_at: '2026-07-13T00:00:00Z',
                },
                reply: { request_id: 'host-input:1:2:0', kind: 'selection', answers: { '0': ['Approve'] } },
                replied_at_ms: 1_800_000_000_500,
              },
            ],
          },
        },
      },
    }

    const pending = pendingInteractionParts(userMessage([answered, part]))
    expect(pending.map((item) => item.id)).toEqual([91])
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

describe('pasted text artifact rendering', () => {
  test('renders pasted text as an input activity block with a truncated preview', () => {
    const blocks = partBlocks({
      id: 11,
      message_id: 42,
      part_index: 0,
      status: 'completed',
      kind: 'text_artifact',
      name: 'text_artifact',
      created_at: '2026-07-13T00:00:00Z',
      content: {
        type: 'text_artifact',
        text: 'x'.repeat(500),
        label: 'my paste',
      },
    })

    expect(blocks.length).toBe(1)
    expect(blocks[0]?.kind).toBe('input_activity')
    expect(blocks[0]?.activityLabel).toBe('Pasted text')
    expect(blocks[0]?.title).toBe('my paste')
    expect(blocks[0]?.body?.length).toBe(240)
    expect(blocks[0]?.body?.endsWith('…')).toBe(true)
  })

  test('falls back to a generic label and keeps short text as-is', () => {
    const blocks = partBlocks({
      id: 12,
      message_id: 42,
      part_index: 0,
      status: 'completed',
      kind: 'text_artifact',
      name: 'text_artifact',
      created_at: '2026-07-13T00:00:00Z',
      content: { type: 'text_artifact', text: 'short paste' },
    })

    expect(blocks).toEqual([
      {
        title: 'Pasted text',
        body: 'short paste',
        kind: 'input_activity',
        activityLabel: 'Pasted text',
      },
    ])
  })
})
