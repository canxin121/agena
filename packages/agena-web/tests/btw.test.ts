import { describe, expect, test } from 'bun:test'
import { effectScope, ref } from 'vue'
import { parseBtwFrame, askBtw, type BtwAnswer } from '../src/pages/chat/btwRequest'
import { useBtw } from '../src/pages/chat/useBtw'

function fixture() {
  const open = ref(true)
  const sessionId = ref<string | null>('7')
  const calls: Array<{
    sessionId: string
    question: string
    signal: AbortSignal
    update: (answer: BtwAnswer) => void
    finish: () => void
  }> = []
  const scope = effectScope()
  const state = scope.run(() =>
    useBtw(
      () => [open.value, sessionId.value],
      (sessionId, question, signal, update) =>
        new Promise<void>((finish) => {
          calls.push({ sessionId, question, signal, update, finish })
        }),
    ),
  )!
  return { open, sessionId, calls, scope, state }
}

describe('BTW request ownership', () => {
  test('sending twice starts one independent request; closing discards late answers', async () => {
    const f = fixture()
    const done = f.state.submit('  a question  ')
    await f.state.submit('duplicate')
    expect(f.calls.length).toBe(1)
    expect(f.calls[0]!.question).toBe('a question')
    f.calls[0]!.update({ text: 'partial', done: false })
    expect(f.state.markdown.value).toBe('partial')
    f.open.value = false
    expect(f.calls[0]!.signal.aborted).toBe(true)
    f.calls[0]!.update({ text: 'too late', done: true })
    f.calls[0]!.finish()
    await done
    expect(f.state.markdown.value).toBe('')
    expect(f.state.loading.value).toBe(false)
    f.scope.stop()
  })

  test('stop preserves the visible partial answer and stale completion cannot replace a newer question', async () => {
    const f = fixture()
    const first = f.state.submit('first')
    f.calls[0]!.update({ text: 'partial first', done: false })
    f.state.stop()
    expect(f.state.markdown.value).toBe('partial first')
    const second = f.state.submit('second')
    f.calls[1]!.update({ text: 'second answer', done: false })
    f.calls[0]!.update({ text: 'stale', done: true, error: 'stale error' })
    f.calls[0]!.finish()
    await first
    expect(f.state.markdown.value).toBe('second answer')
    expect(f.state.loading.value).toBe(true)
    expect(f.state.error.value).toBe('')
    f.calls[1]!.finish()
    await second
    f.scope.stop()
  })

  test('navigation and component disposal abort only their own request', async () => {
    const f = fixture()
    const first = f.state.submit('first')
    f.sessionId.value = '9'
    expect(f.calls[0]!.signal.aborted).toBe(true)
    const second = f.state.submit('second')
    expect(f.calls[1]!.sessionId).toBe('9')
    f.scope.stop()
    expect(f.calls[1]!.signal.aborted).toBe(true)
    for (const call of f.calls) call.finish()
    await Promise.all([first, second])
  })
})

describe('BTW stream protocol', () => {
  test('accepts keepalives, CRLF and Chinese Markdown; rejects malformed snapshots', () => {
    expect(parseBtwFrame(': keep-alive')).toBeNull()
    expect(parseBtwFrame('event: btw\r\ndata: {"text":"**中文**","done":true}')).toEqual({
      text: '**中文**',
      done: true,
    })
    for (const data of [
      '[]',
      '{"text":1,"done":true}',
      '{"text":"x","done":"yes"}',
      '{"text":"x","done":true,"error":{}}',
    ]) {
      expect(() => parseBtwFrame(`event: btw\ndata: ${data}`)).toThrow()
    }
    expect(() => parseBtwFrame('event: error\ndata: {}')).toThrow()
  })

  test('reassembles split UTF-8 frames without replaying a POST', async () => {
    const original = globalThis.fetch
    let calls = 0
    globalThis.fetch = (async (_input, init) => {
      calls += 1
      expect(init?.method).toBe('POST')
      const bytes = new TextEncoder().encode(
        ': keep-alive\n\nevent: btw\ndata: {"text":"中文","done":false}\n\nevent: btw\ndata: {"text":"中文完成","done":true}\n\n',
      )
      return new Response(
        new ReadableStream({
          start(controller) {
            for (const byte of bytes) controller.enqueue(new Uint8Array([byte]))
            controller.close()
          },
        }),
        { headers: { 'content-type': 'text/event-stream' } },
      )
    }) as typeof fetch
    try {
      const updates: BtwAnswer[] = []
      await askBtw('7', 'question', new AbortController().signal, (answer) => updates.push(answer))
      expect(updates.map((answer) => answer.text)).toEqual(['中文', '中文完成'])
      expect(calls).toBe(1)
    } finally {
      globalThis.fetch = original
    }
  })

  test('a truncated stream reports failure instead of silently accepting partial output', async () => {
    const original = globalThis.fetch
    globalThis.fetch = (async () =>
      new Response('event: btw\ndata: {"text":"partial","done":false}\n\n')) as typeof fetch
    try {
      await expect(askBtw('7', 'question', new AbortController().signal, () => {})).rejects.toThrow('before completion')
    } finally {
      globalThis.fetch = original
    }
  })
})
