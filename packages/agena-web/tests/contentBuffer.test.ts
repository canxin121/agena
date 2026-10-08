import { describe, expect, test } from 'bun:test'
import { ContentBuffer, type ContentPage, type ContentPayload } from '../src/lib/content'
import { updateDocument, contentDocumentText, type ContentDocument } from '../src/lib/contentDocument'

function page(first: number, values: string[], options: { epoch?: string; state?: 'active' | 'complete'; gap?: boolean } = {}): ContentPage {
  const epoch = options.epoch || 'epoch-1'
  const last = first + values.length - 1
  const cursor = { epoch, sequence: last }
  return {
    resource: {
      resource_id: 'resource-1', kind: 'log', owner_session_id: 1, part_id: 2,
      state: options.state || 'active', cursor, committed_cursor: cursor,
      total_bytes: values.join('').length, dropped_bytes: 0, retained_ranges: [{ first, last }],
    },
    chunks: values.map((text, index) => ({ cursor: { epoch, sequence: first + index }, captured_at_ms: 1000, payload: { type: 'log', stream: 'stdout', text } })),
    next_cursor: cursor, has_more: false, gap: options.gap || false,
  }
}

describe('content resource reduction', () => {
  test('bootstrap tails and subsequent appends preserve exact whitespace', () => {
    const buffer = new ContentBuffer()
    buffer.apply(page(12, ['  中\n\n', 'partial']))
    buffer.apply(page(14, ['\rupdate']))
    expect(buffer.text()).toBe('  中\n\npartial\rupdate')
    expect(buffer.cursor?.sequence).toBe(14)
  })

  test('replayed and stale records never duplicate or roll back bytes', () => {
    const buffer = new ContentBuffer()
    buffer.apply(page(1, ['one', 'two']))
    buffer.apply(page(2, ['two', 'three']))
    buffer.apply(page(1, ['one']))
    expect(buffer.text()).toBe('onetwothree')
    expect(buffer.cursor?.sequence).toBe(3)
  })

  test('an unexplained gap requests recovery without modifying displayed output', () => {
    const buffer = new ContentBuffer()
    buffer.apply(page(1, ['one']))
    expect(buffer.apply(page(3, ['three'])).missing).toBe(true)
    expect(buffer.text()).toBe('one')
    buffer.apply(page(2, ['two', 'three']))
    expect(buffer.text()).toBe('onetwothree')
  })

  test('retention gaps advance explicitly and remain visible', () => {
    const buffer = new ContentBuffer()
    buffer.apply(page(1, ['one']))
    expect(buffer.apply(page(7, ['seven'], { gap: true })).missing).toBe(false)
    expect(buffer.gap).toBe(true)
    expect(buffer.cursor?.sequence).toBe(7)
  })

  test('terminal state survives an equal-position delayed live page', () => {
    const buffer = new ContentBuffer()
    buffer.apply(page(1, ['one'], { state: 'complete' }))
    buffer.apply(page(1, ['one']))
    expect(buffer.resource?.state).toBe('complete')
  })

  test('a new epoch resets the old data and state together', () => {
    const buffer = new ContentBuffer()
    buffer.apply(page(10, ['old'], { state: 'complete', gap: true }))
    const update = buffer.apply(page(1, ['new'], { epoch: 'epoch-2' }))
    expect(update.reset).toBe(true)
    expect(buffer.text()).toBe('new')
    expect(buffer.resource?.state).toBe('active')
    expect(buffer.cursor?.epoch).toBe('epoch-2')
    expect(buffer.gap).toBe(false)
  })

  test('tiny records are bounded by memory overhead as well as payload bytes', () => {
    const buffer = new ContentBuffer(1024)
    buffer.apply(page(1, Array.from({ length: 100 }, () => 'x')))
    expect(buffer.chunks.length).toBeLessThan(20)
    expect(buffer.cursor?.sequence).toBe(100)
    expect(buffer.windowed).toBe(true)
    expect(buffer.gap).toBe(false)
  })

  function documentPage(first: number, payloads: ContentPayload[], gap = false): ContentPage {
    const value = page(first, payloads.map(() => ''), { gap })
    value.resource.kind = 'document'
    value.chunks = payloads.map((payload, index) => ({ cursor: { epoch: 'epoch-1', sequence: first + index }, captured_at_ms: 1000, payload }))
    return value
  }

  const initial: ContentDocument = { blocks: [{ type: 'text', id: 'body', text: '  中\n\n' }, { type: 'table', id: 'rows', columns: ['path'], rows: [] }] }

  test('typed documents preserve block identity, ordering and copy whitespace', () => {
    const next = updateDocument(initial, { type: 'append_rows', block_id: 'rows', rows: [['a']] })
    expect(next.blocks[0]).toBe(initial.blocks[0])
    expect(initial.blocks[1]!.rows).toEqual([])
    expect(contentDocumentText(next)).toBe('  中\n\n\n\npath\na')
    expect(() => updateDocument(next, { type: 'append_rows', block_id: 'rows', rows: [['a', 'b']] })).toThrow()
    expect(next.blocks[1]!.rows).toEqual([['a']])
  })

  test('document bases cross missing logs without confusing them with missing mutations', () => {
    const buffer = new ContentBuffer()
    buffer.apply(documentPage(1, [{ type: 'structured_snapshot', document: initial }]))
    const update = buffer.apply(documentPage(9, [{ type: 'structured', base_cursor: { epoch: 'epoch-1', sequence: 1 }, event: { type: 'append_text', block_id: 'body', text: 'tail' } }], true))
    expect(update.missing).toBe(false)
    expect(buffer.document!.blocks[0]!.text).toBe('  中\n\ntail')
    expect(buffer.gap).toBe(true)
  })

  test('a missing document base preserves the last display until a valid replacement arrives', () => {
    const buffer = new ContentBuffer()
    buffer.apply(documentPage(1, [{ type: 'structured_snapshot', document: initial }]))
    const update = buffer.apply(documentPage(4, [{ type: 'structured', base_cursor: { epoch: 'epoch-1', sequence: 3 }, event: { type: 'append_text', block_id: 'body', text: 'bad' } }], true))
    expect(update.missing).toBe(true)
    expect(buffer.cursor).toBeNull()
    expect(buffer.document).toBe(initial)
    const recovered = buffer.apply(documentPage(5, [{ type: 'structured_snapshot', document: { blocks: [{ type: 'text', id: 'body', text: 'new' }] } }]))
    expect(recovered.reset).toBe(true)
    expect(buffer.document!.blocks[0]!.text).toBe('new')
    expect(buffer.chunks.map((chunk) => chunk.cursor.sequence)).toEqual([5])
  })

  test('an invalid later update rejects a whole page without partially appending logs', () => {
    const buffer = new ContentBuffer()
    buffer.apply(documentPage(1, [{ type: 'structured_snapshot', document: initial }]))
    expect(buffer.apply(documentPage(2, [{ type: 'log', stream: 'stdout', text: 'uncommitted' }, { type: 'structured', base_cursor: { epoch: 'epoch-1', sequence: 1 }, event: { type: 'append_rows', block_id: 'rows', rows: [['a', 'b']] } }])).missing).toBe(true)
    expect(buffer.text()).toBe('')
    expect(buffer.document).toBe(initial)
  })
})
