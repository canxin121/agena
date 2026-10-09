import { describe, expect, test } from 'bun:test'
import { computed, effect, reactive, ref } from 'vue'
import { createTranscriptProjector, projectTranscriptBlocks } from '../src/pages/chat/transcriptProjection'
import type { MessageLike } from '../src/components/chat/messageList.types'
import { upsertPart } from '../src/stores/chat/messageIndex'
import type { MessageEntry, MessagePart } from '../src/types/chat'
import { syncKeySet } from '../src/lib/reactiveKeySet'
import { SseFrames } from '../src/lib/sseFrames'
import { highlightCodeToHtml } from '../src/lib/highlight'

function message(id: number, role: string, body: string): MessageLike {
  return { info: { id: String(id), role }, parts: [{ id: String(id + 1), agenaKind: 'text', text: body }] }
}

describe('transcript work isolation', () => {
  test('long blank replies keep their full text and find the first meaningful line without regex backtracking', () => {
    const blank = ' '.repeat(64 * 1024)
    const empty = projectTranscriptBlocks([message(1, 'user', blank)])[0]!.displayParts[0]!
    expect(empty.summary).toBe('')
    expect(empty.copyText).toBe(blank)
    const body = `${blank}\r\n\t  中文 first line  \r\nnext line`
    const meaningful = projectTranscriptBlocks([message(1, 'assistant', body)])[0]!.displayParts[0]!
    expect(meaningful.summary).toBe('中文 first line')
    expect(meaningful.copyText).toBe(body)
  })
  test('in-place stream edits preserve every unrelated block and part identity', () => {
    const messages = reactive([
      message(1, 'user', 'question'),
      message(3, 'assistant', 'first'),
      message(5, 'user', 'next'),
      message(7, 'assistant', 'tail'),
    ])
    const project = createTranscriptProjector(() => ({ showReasoning: true }))
    const blocks = computed(() => project(messages))
    const before = blocks.value
    messages[3]!.parts[0]!.text += ' token'
    const after = blocks.value
    expect(after[0]).toBe(before[0])
    expect(after[1]).toBe(before[1])
    expect(after[2]).toBe(before[2])
    expect(after[3]).not.toBe(before[3])
    expect(after[3]!.displayParts[0]!.copyText).toBe('tail token')
  })

  test('a live upsert re-renders only the edited part and keeps its identity', () => {
    const messages = reactive([
      message(1, 'user', 'question'),
      message(3, 'assistant', 'first'),
      message(5, 'user', 'next'),
      message(7, 'assistant', 'tail'),
    ])
    const project = createTranscriptProjector(() => ({ showReasoning: true }))
    const blocks = computed(() => project(messages))
    const initial = blocks.value
    const target = messages[3]! as unknown as MessageEntry
    const cachedPart = target.parts[0]! as MessagePart

    upsertPart(target, { ...cachedPart, revision: 2, updatedAt: 20, text: 'tail token' }, '')
    const after = blocks.value
    expect(target.parts[0]).toBe(cachedPart)
    expect(after[0]).toBe(initial[0])
    expect(after[1]).toBe(initial[1])
    expect(after[2]).toBe(initial[2])
    expect(after[3]).not.toBe(initial[3])
    expect(after[3]!.displayParts[0]!.copyText).toBe('tail token')

    // A repeated snapshot is not an edit: no derived block changes.
    upsertPart(target, { ...cachedPart, revision: 2, updatedAt: 20 }, '')
    expect(blocks.value).toBe(after)
  })

  test('folded replies retain unchanged part identity and reclassify the old answer', () => {
    const messages = reactive([message(1, 'assistant', 'first'), message(3, 'assistant', 'second')])
    const showReasoning = ref(true)
    const project = createTranscriptProjector(() => ({ showReasoning: showReasoning.value }))
    const blocks = computed(() => project(messages))
    const before = blocks.value[0]!
    messages[1]!.parts[0]!.text += ' token'
    expect(blocks.value[0]!.displayParts[0]).toBe(before.displayParts[0])
    messages.push(message(5, 'assistant', 'final'))
    expect(blocks.value[0]!.runIds).toEqual(['1', '3', '5'])
    expect(blocks.value[0]!.displayParts.map((part) => part.kind)).toEqual(['text_segment', 'text_segment', 'answer'])
    showReasoning.value = false
    expect(blocks.value).toEqual(projectTranscriptBlocks(messages, { showReasoning: false }))
  })

  test('membership updates rerun only the keys that changed', () => {
    const keys = reactive(new Set<string>())
    const counts = Array(100).fill(0)
    for (let i = 0; i < 100; i++)
      effect(() => {
        keys.has(String(i))
        counts[i]++
      })
    syncKeySet(keys, ['4', '8'])
    expect(counts.filter((count) => count === 2)).toHaveLength(2)
    syncKeySet(keys, ['8', '10'])
    expect(counts[4]).toBe(3)
    expect(counts[8]).toBe(2)
    expect(counts[10]).toBe(2)
    expect(counts[90]).toBe(1)
  })
})

test('unsupported code languages obey the automatic-detection budget while ordinary explicit code keeps its colors', () => {
  const code = 'const item = value + 1\n'.repeat(100)
  expect(highlightCodeToHtml(code, 'unknown-language')).toBe(code)
  expect(highlightCodeToHtml(code)).toBe(code)
  expect(highlightCodeToHtml(code, 'javascript')).toContain('hljs-keyword')
  const longLine = 'const item = value + 1; '.repeat(220)
  expect(highlightCodeToHtml(longLine, 'javascript')).toBe(longLine)
  expect(highlightCodeToHtml('<img> & "text"', 'text')).toBe('&lt;img&gt; &amp; &quot;text&quot;')
})

describe('incremental SSE framing', () => {
  test('matches frame boundaries across every split position and CRLF pairs', () => {
    const source = 'id: 1\r\ndata: first\r\ndata: second\r\n\r\nid: 2\ndata: third\n\n'
    const expected = ['id: 1\ndata: first\ndata: second', 'id: 2\ndata: third']
    for (let split = 0; split <= source.length; split++) {
      const frames = new SseFrames()
      expect([...frames.push(source.slice(0, split)), ...frames.push(source.slice(split))]).toEqual(expected)
    }
    const frames = new SseFrames()
    expect([...source].flatMap((character) => frames.push(character))).toEqual(expected)
  })

  test('retains an incomplete large frame and emits it exactly once', () => {
    const frames = new SseFrames()
    const source = `data: ${'x'.repeat(2 * 1024 * 1024)}`
    for (let i = 0; i < source.length; i += 4096) expect(frames.push(source.slice(i, i + 4096))).toEqual([])
    expect(frames.push('\n')).toEqual([])
    expect(frames.push('\n')).toEqual([source])
    expect(frames.push('data: next\n\n')).toEqual(['data: next'])
  })
})
