import { describe, expect, test } from 'bun:test'

import { entriesFromParts } from '../src/stores/chat/api'

function runMarker(partId: number, role: string, content: Record<string, unknown>) {
  return {
    part_id: partId,
    message_boundary: true, kind: 'run',
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

  test('compaction is owned by its run without fabricating content from the marker', () => {
    const marker = runMarker(9, 'assistant', { run_kind: 'compaction' })
    expect(entriesFromParts('1', [marker] as never)[0]?.parts).toHaveLength(0)
    const entries = entriesFromParts('1', [
      marker,
      {
        part_id: 10,
        run_id: 9,
        kind: 'compaction',
        role: 'assistant',
        state: 'completed',
        content: { checkpoint: { content: { type: 'text_summary', summary: 'Checkpoint summary' } } },
        created_at_ms: 100,
      },
    ] as never)
    expect(entries[0]?.parts).toHaveLength(1)
    expect(entries[0]?.parts?.[0]).toMatchObject({ id: '10', type: 'compaction', text: 'Checkpoint summary' })
  })
})


test('the native first response entry is both its stable boundary and content exactly once', () => {
  const first={part_id:31,run_id:31,message_boundary:true,kind:'text',role:'assistant',state:'running',revision:1,created_at_ms:31,content:{text:'first'}}
  const tail={part_id:32,run_id:31,message_boundary:false,kind:'text',role:'assistant',state:'running',revision:1,created_at_ms:32,content:{text:'tail'}}
  const full=entriesFromParts('1',[first,tail] as never)
  expect(full).toHaveLength(1)
  expect(full[0]!.info.id).toBe('31')
  expect(full[0]!.parts.map(part=>part.id)).toEqual(['31','32'])
  const partial=entriesFromParts('1',[tail] as never,[],[31])
  expect(partial[0]!.info.id).toBe('31')
  expect(partial[0]!.parts.map(part=>part.id)).toEqual(['32'])
})
