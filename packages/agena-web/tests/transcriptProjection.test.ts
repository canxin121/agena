import { describe, expect, test } from 'bun:test'

import type { MessageLike, MessagePartLike } from '../src/components/chat/messageList.types'
import { durablePartKind, projectTranscriptBlocks, userInputOrder } from '../src/pages/chat/transcriptProjection'

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
  test('native composer documents preserve activity identity, resource metadata and exact text', () => {
    const resourceId='4e1af70c-3b18-4a6e-a03a-c45c83e5c5c0'
    const artifactId='b6354fc1-7f1e-4561-b91c-169868bcc976'
    const native=part('41','text',{text:'  exact\n',document:[
      {type:'text',text:'  exact\n'},
      {type:'activity',activity:{id:resourceId,payload:{activity_type:'resource',kind:'pdf',reference:{reference_type:'workspace_path',path:'uploads/report.pdf'},name:'report.pdf',media_type:'application/pdf',size_bytes:123}}},
      {type:'activity',activity:{id:artifactId,payload:{activity_type:'text_artifact',text:' artifact \n',label:'snippet',language:'text'}}},
    ]})
    const blocks=projectTranscriptBlocks([message('41','user',[native])])
    expect(blocks[0]?.kind).toBe('message')
    const parts=blocks[0]?.kind==='message'?blocks[0].displayParts:[]
    const resource=parts.find(entry=>entry.id===resourceId)
    expect(resource?.kind).toBe('resource')
    expect(resource?.source.agenaContent).toMatchObject({path:'uploads/report.pdf',name:'report.pdf',mime:'application/pdf',size_bytes:123})
    expect(parts.find(entry=>entry.id===artifactId)?.copyText).toContain(' artifact \n')
    expect(parts.filter(entry=>entry.id==='41')).toHaveLength(1)
  })

  test('does not reinterpret removed type-only tool rows', () => {
    expect(durablePartKind({ type: 'tool' })).toBe('unknown')
    expect(durablePartKind({ type: 'reasoning' })).toBe('unknown')
    expect(durablePartKind({ agenaKind: 'tool_call', type: 'tool' })).toBe('tool_call')
  })

  test('an attachment-only user message still projects its resource part', () => {
    // Regression: a user message without body text must render its
    // attachment. The send path stores the payload under `text`, so the wire
    // publishes the canonical `file_ref` kind that clients render by.
    const filePart: MessagePartLike = {
      id: '2',
      type: 'file',
      partState: 'completed',
      agenaKind: 'file_ref',
      agenaRole: 'user',
      filename: 'report.pdf',
      serverPath: 'uploads/report.pdf',
      agenaContent: { path: 'uploads/report.pdf', name: 'report.pdf', mime: 'application/pdf' },
    }
    const blocks = projectTranscriptBlocks([message('1', 'user', [filePart])])
    expect(blocks).toHaveLength(1)
    const displayParts = blocks[0]?.displayParts ?? []
    const resource = displayParts.find((entry) => entry.kind === 'resource')
    expect(resource).toBeDefined()
    expect(`${resource?.title} ${resource?.summary}`).toContain('report.pdf')
  })

  test('a user attachment row is collapsed by default', () => {
    const filePart: MessagePartLike = {
      id: '2',
      type: 'file',
      partState: 'completed',
      agenaKind: 'file_ref',
      agenaRole: 'user',
      filename: 'photo.png',
      serverPath: 'uploads/photo.png',
      agenaContent: { path: 'uploads/photo.png', name: 'photo.png', mime: 'image/png' },
    }
    const blocks = projectTranscriptBlocks([message('1', 'user', [filePart])], { showReasoning: true })
    const resource = (blocks[0]?.kind === 'message' ? blocks[0].displayParts : []).find(
      (entry) => entry.kind === 'resource',
    )
    expect(resource?.defaultExpanded).toBe(false)
    expect(resource?.toggleable).toBe(true)
  })

  test('an assistant resource part keeps the collapsed default', () => {
    const filePart: MessagePartLike = {
      id: '3',
      type: 'file',
      partState: 'completed',
      agenaKind: 'file_ref',
      agenaRole: 'assistant',
      filename: 'report.pdf',
      agenaContent: { path: 'out/report.pdf', name: 'report.pdf', mime: 'application/pdf' },
    }
    const blocks = projectTranscriptBlocks([message('2', 'assistant', [filePart])], { showReasoning: true })
    const resource = (blocks[0]?.kind === 'message' ? blocks[0].displayParts : []).find(
      (entry) => entry.kind === 'resource',
    )
    expect(resource?.defaultExpanded).toBe(false)
  })

  test('attachment rows use the injected UI vocabulary while runtime titles win', () => {
    const filePart: MessagePartLike = {
      id: '4',
      type: 'file',
      partState: 'completed',
      agenaKind: 'file_ref',
      agenaRole: 'user',
      filename: 'photo.png',
      agenaContent: { path: 'uploads/photo.png', name: 'photo.png', mime: 'image/png' },
    }
    const localized = projectTranscriptBlocks([message('3', 'user', [filePart])], {
      showReasoning: true,
      labels: { attachment: '附件' },
    })
    const switched = (localized[0]?.kind === 'message' ? localized[0].displayParts : []).find(
      (entry) => entry.kind === 'resource',
    )
    expect(switched?.title).toBe('附件')

    const runtime = projectTranscriptBlocks(
      [message('4', 'user', [{ ...filePart, agenaPresentation: { title: 'Shared file' } }])],
      {
        showReasoning: true,
        labels: { attachment: '附件' },
      },
    )
    const runtimeTitled = (runtime[0]?.kind === 'message' ? runtime[0].displayParts : []).find(
      (entry) => entry.kind === 'resource',
    )
    expect(runtimeTitled?.title).toBe('Shared file')
  })

  test('a committed attachment is localized and collapsed before the assistant replies', () => {
    const durable = projectTranscriptBlocks(
      [
        message('9', 'user', [
          {
            id: '10',
            type: 'file',
            partState: 'completed',
            agenaKind: 'file_ref',
            agenaRole: 'user',
            filename: 'photo.png',
            serverPath: 'uploads/photo.png',
            agenaContent: { path: 'uploads/photo.png', name: 'photo.png', mime: 'image/png' },
          },
        ]),
      ],
      { showReasoning: true, labels: { attachment: '附件' } },
    )
    const durableFile = (durable[0]?.kind === 'message' ? durable[0].displayParts : []).find(
      (entry) => entry.kind === 'resource',
    )
    expect([durableFile?.title, durableFile?.defaultExpanded]).toEqual(['附件', false])
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
        message('10', 'assistant', [part('11', 'tool_call', { name: 'tools_list' })], 'in_progress'),
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
      [message('2', 'assistant', [part('10', 'text', { text: 'later' }), part('3', 'text', { text: 'earlier' })])],
      { showReasoning: true },
    )
    const projected = blocks[0]
    expect(projected?.displayParts.map((item) => item.id)).toEqual(['3', '10'])
  })

  test('shows active envelopes and a lifecycle fallback for failed runs without a durable error Part', () => {
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
    expect(failed[1]?.title).toBe('Response failed')
    expect(blocks[2]?.message.info.runState).toBe('failed')
  })

  test('a durable error Part replaces the failed lifecycle fallback', () => {
    const error = part('61', 'error', { problem: { user: { fallback: 'Provider failed' } } })
    const blocks = projectTranscriptBlocks(
      [message('60', 'assistant', [part('62', 'text', { text: 'partial response' }), error], 'failed')],
      { showReasoning: true },
    )
    const failed = blocks[0]?.kind === 'message' ? blocks[0].displayParts : []
    expect(failed.map((item) => item.kind)).toEqual(['error', 'answer'])
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

  test('recovery retains error Part identity while the reply status advances', () => {
    const error = part('41', 'error', { problem: { user: { fallback: 'Temporary failure' } } })
    const blocks = projectTranscriptBlocks(
      [
        message('40', 'assistant', [error], 'failed'),
        message('42', 'assistant', [part('43', 'text', { text: 'Recovered answer' })], 'completed'),
      ],
      { showReasoning: true },
    )
    expect(blocks).toHaveLength(1)
    expect(blocks[0]?.message.info.runState).toBe('completed')
    expect(blocks[0]?.displayParts.map((part) => [part.id, part.kind])).toEqual([
      ['41', 'error'],
      ['43', 'answer'],
    ])
    expect(blocks[0]?.displayParts.some((part) => part.kind === 'lifecycle')).toBe(false)
  })

  test('does not turn an unknown backend run state into a synthetic failure', () => {
    const blocks = projectTranscriptBlocks([message('50', 'assistant', [], 'future_state')], {
      showReasoning: true,
    })
    const projected = blocks[0]?.kind === 'message' ? blocks[0].displayParts : []

    expect(projected).toEqual([])
  })

  test('expands pending operation interactions by default', () => {
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

  test('answered interactions, permissions and terminal parts also default to expanded', () => {
    const sources = [
      { user_input: { requests: [{ request: { request_id: 'answered' }, reply: { answers: {} } }] } },
      { authorization: { permissions: [{ request: { request_id: 'approve' }, reply: null }] } },
      { resources: [{ resource_id: 'terminal', kind: 'terminal' }] },
      { resources: [{ resource_id: 'log', kind: 'log' }] },
    ]
    const blocks = projectTranscriptBlocks(
      [
        message(
          '30',
          'assistant',
          sources.map((content, i) => part(String(31 + i), 'tool_call', content)),
        ),
      ],
      { showReasoning: true },
    )
    expect(
      blocks[0]?.displayParts.filter((part) => part.kind === 'operation').map((part) => part.defaultExpanded),
    ).toEqual([true, true, true, false])
  })
})

describe('user input ordering', () => {
  test('attachments are shown above the text that already carries their placeholders', () => {
    const textPart = part('1', 'text', { text: '1. [image.png] 有一段额外的 card 包裹需要移除掉' })
    const filePart: MessagePartLike = {
      id: '2',
      type: 'file',
      partState: 'completed',
      agenaKind: 'file_ref',
      agenaRole: 'user',
      agenaContent: { name: 'image.png', mime: 'image/png' },
    }
    const blocks = projectTranscriptBlocks([message('1', 'user', [textPart, filePart])])
    const displayParts = blocks[0]?.displayParts ?? []
    expect(displayParts.map((entry) => entry.kind)).toEqual(['text', 'resource'])
    expect(userInputOrder(displayParts).map((entry) => entry.kind)).toEqual(['resource', 'text'])
  })

  test('a message without attachments keeps its part order', () => {
    const first = part('1', 'text', { text: 'first line' })
    const second = part('2', 'text', { text: 'second line' })
    const blocks = projectTranscriptBlocks([message('1', 'user', [first, second])])
    const displayParts = blocks[0]?.displayParts ?? []
    expect(userInputOrder(displayParts).map((entry) => entry.id)).toEqual(displayParts.map((entry) => entry.id))
  })
})
