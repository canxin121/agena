import { describe, expect, test } from 'bun:test'

import type { MessageLike, MessagePartLike } from '../src/components/chat/messageList.types'
import { durablePartKind, projectTranscriptBlocks } from '../src/pages/chat/transcriptProjection'

function part(id: string, kind: string, content: Record<string, unknown>, state = 'completed'): MessagePartLike {
  return {
    id,
    type: kind === 'think' ? 'reasoning' : kind === 'tool_call' ? 'tool' : 'text',
    partState: state,
    agenaKind: kind,
    agenaRole: 'assistant',
    agenaContent: content,
    ...(kind === 'tool_call' ? { agenaPresentation: { title: 'Tool operation', summary: '', blocks: [] } } : {}),
  }
}

function message(id: string, role: string, parts: MessagePartLike[], runState = 'completed'): MessageLike {
  return {
    info: { id, role, runState, finish: runState, time: { created: Number(id) || 1 } },
    parts,
  }
}

describe('TUI-parity transcript projection', () => {
  test('does not reinterpret removed type-only tool rows', () => {
    expect(durablePartKind({ type: 'tool' })).toBe('unknown')
    expect(durablePartKind({ type: 'reasoning' })).toBe('unknown')
    expect(durablePartKind({ agenaKind: 'tool_call', type: 'tool' })).toBe('tool_call')
  })

  test('keeps parts inside their run and promotes only the final assistant text to Answer', () => {
    const blocks = projectTranscriptBlocks(
      [
        message('1', 'user', [{ ...part('2', 'text', { text: 'hello' }), agenaRole: 'user', text: 'hello' }]),
        message('3', 'assistant', [
          part('4', 'think', { summary: ['full', ' reasoning'] }),
          part('5', 'text', { text: 'working note' }),
          part('6', 'tool_call', {
            name: 'fs.read',
            state: 'completed',
          }),
          part('7', 'text', { text: 'final answer' }),
        ]),
      ],
      { showReasoning: true },
    )

    expect(blocks).toHaveLength(2)
    expect(blocks[0]?.kind).toBe('message')
    expect(blocks[0]?.kind === 'message' ? blocks[0].displayParts.map((item) => item.kind) : []).toEqual(['text'])
    expect(blocks[1]?.kind === 'message' ? blocks[1].displayParts.map((item) => item.kind) : []).toEqual([
      'reasoning',
      'text_segment',
      'operation',
      'answer',
    ])
    expect(blocks[1]?.kind === 'message' ? blocks[1].displayParts[0]?.copyText : '').toBe('full reasoning')
    expect(blocks[1]?.kind === 'message' ? blocks[1].displayParts[0]?.defaultExpanded : true).toBe(false)
    expect(blocks[1]?.kind === 'message' ? blocks[1].displayParts[3]?.defaultExpanded : false).toBe(true)
  })

  test('folds consecutive assistant rounds but preserves runtime and user boundaries', () => {
    const blocks = projectTranscriptBlocks(
      [
        message('10', 'assistant', [part('11', 'tool_call', { name: 'tools_search' })], 'in_progress'),
        message('12', 'assistant', [part('13', 'text', { text: 'done' })]),
        message('14', 'system', [part('15', 'notice', { summary: 'boundary' })]),
        message('16', 'assistant', [part('17', 'text', { text: 'after boundary' })]),
      ],
      { showReasoning: true },
    )

    expect(blocks).toHaveLength(3)
    expect(blocks[0]?.kind === 'message' ? blocks[0].runIds : []).toEqual(['10', '12'])
    expect(blocks[0]?.kind === 'message' ? blocks[0].displayParts.map((item) => item.kind) : []).toEqual([
      'operation',
      'answer',
    ])
    expect(blocks[1]?.kind === 'message' ? blocks[1].message.info.role : '').toBe('system')
  })

  test('retains unknown open-set parts as readable JSON instead of dropping them', () => {
    const blocks = projectTranscriptBlocks(
      [message('1', 'assistant', [part('2', 'future_kind', { summary: 'future payload', nested: { ok: true } })])],
      { showReasoning: true },
    )
    const projected = blocks[0]?.kind === 'message' ? blocks[0].displayParts[0] : null
    expect(projected?.kind).toBe('unknown')
    expect(projected?.copyText).toContain('future payload')
  })

  test('orders durable numeric part ids numerically inside a message', () => {
    const blocks = projectTranscriptBlocks(
      [
        message('2', 'assistant', [
          part('10', 'text', { text: 'later' }),
          part('3', 'text', { text: 'earlier' }),
        ]),
      ],
      { showReasoning: true },
    )
    const projected = blocks[0]
    expect(projected?.displayParts.map((item) => item.id)).toEqual(['3', '10'])
  })

  test('projects empty and terminal assistant reply lifecycle rows like the TUI', () => {
    const blocks = projectTranscriptBlocks(
      [
        message('20', 'assistant', [], 'in_progress'),
        message('21', 'system', [part('22', 'notice', { summary: 'boundary' })]),
        message('23', 'assistant', [part('24', 'text', { text: 'partial response' })], 'failed'),
      ],
      { showReasoning: true },
    )
    const running = blocks[0]?.kind === 'message' ? blocks[0].displayParts : []
    const failed = blocks[2]?.kind === 'message' ? blocks[2].displayParts : []
    expect(running.map((item) => [item.kind, item.title, item.status])).toEqual([
      ['lifecycle', 'Response running', 'in_progress'],
    ])
    expect(failed.map((item) => item.kind)).toEqual(['answer', 'lifecycle'])
    expect(failed.at(-1)?.title).toBe('Response failed')
  })

  test('uses the latest backend run state when adjacent assistant rounds are folded', () => {
    const failed = message('40', 'assistant', [part('41', 'text', { text: 'partial response' })], 'failed')
    failed.info.error = { message: 'older failed attempt' }
    const running = message('42', 'assistant', [], 'in_progress')
    delete running.info.finish

    const blocks = projectTranscriptBlocks([failed, running], {
      showReasoning: true,
    })
    const folded = blocks[0]?.kind === 'message' ? blocks[0] : null

    expect(blocks).toHaveLength(1)
    expect(folded?.runIds).toEqual(['40', '42'])
    expect(folded?.message.info.runState).toBe('in_progress')
    expect(folded?.message.info.finish).toBeUndefined()
    expect(folded?.message.info.error).toBeUndefined()
    expect(folded?.displayParts.some((item) => item.title === 'Response failed')).toBe(false)
  })

  test('does not turn an unknown backend run state into a synthetic failure', () => {
    const blocks = projectTranscriptBlocks([message('50', 'assistant', [], 'future_state')], {
      showReasoning: true,
    })
    const projected = blocks[0]?.kind === 'message' ? blocks[0].displayParts : []

    expect(projected).toEqual([])
  })

  test('keeps pending operation interactions visible and expanded', () => {
    const blocks = projectTranscriptBlocks(
      [
        message(
          '30',
          'assistant',
          [
            part(
              '31',
              'tool_call',
              {
                user_input: {
                  requests: [{ request: { request_id: 'request-31', questions: [] }, reply: null }],
                },
              },
              'in_progress',
            ),
          ],
          'in_progress',
        ),
      ],
      { showReasoning: true },
    )
    const projected = blocks[0]?.kind === 'message' ? blocks[0].displayParts[0] : null
    expect(projected?.kind).toBe('operation')
    expect(projected?.defaultExpanded).toBe(true)
  })
})
