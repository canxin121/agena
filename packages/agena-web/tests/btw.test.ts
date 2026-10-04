import { describe, expect, test } from 'bun:test'
import { effectScope } from 'vue'
import { parseBtwFrame, askBtw, type BtwAnswer } from '../src/pages/chat/btwRequest'
import { useBtw, BTW_HISTORY_LIMIT } from '../src/pages/chat/btwState'

function fixture() {
  const calls: Array<{
    sessionId: string
    question: string
    signal: AbortSignal
    update: (answer: BtwAnswer) => void
    finish: () => void
    fail: (error: Error) => void
  }> = []
  const scope = effectScope()
  const state = scope.run(() =>
    useBtw(
      (sessionId, question, signal, update) =>
        new Promise<void>((finish, fail) => calls.push({ sessionId, question, signal, update, finish, fail })),
    ),
  )!
  async function submit(id: string, question: string) {
    state.open(id)
    state.sessions.get(id)!.draft = question
    return state.submit(id)
  }
  return { calls, scope, state, submit }
}

describe('BTW session ownership', () => {
  test('switching sessions and hiding the answer preserve the running request and history', async () => {
    const f = fixture()
    const first = f.submit('7', '  first question  ')
    f.state.sessions.get('7')!.expanded = false
    const second = f.submit('9', 'second question')
    expect(f.calls[0]!.signal.aborted).toBe(false)
    expect(f.calls[0]!.question).toBe('first question')
    f.calls[0]!.update({ text: '**first answer**', done: true })
    f.calls[0]!.finish()
    f.calls[1]!.update({ text: 'second answer', done: true })
    f.calls[1]!.finish()
    await Promise.all([first, second])
    f.state.open('7')
    expect(f.state.sessions.get('7')!.exchanges[0]!.markdown).toBe('**first answer**')
    expect(f.state.sessions.get('9')!.exchanges[0]!.markdown).toBe('second answer')
    expect(f.calls.length).toBe(2)
    f.scope.stop()
  })

  test('duplicate submission keeps the next draft and starts only one request per session', async () => {
    const f = fixture()
    const first = f.submit('7', 'first')
    await f.submit('7', 'next draft')
    expect(f.calls.length).toBe(1)
    expect(f.state.sessions.get('7')!.draft).toBe('next draft')
    f.calls[0]!.finish()
    await first
    f.scope.stop()
  })

  test('stopping preserves partial Markdown; a late answer cannot replace a newer question', async () => {
    const f = fixture()
    const first = f.submit('7', 'first')
    f.calls[0]!.update({ text: 'partial', done: false })
    f.state.stop('7')
    expect(f.calls[0]!.signal.aborted).toBe(true)
    const second = f.submit('7', 'second')
    f.calls[0]!.update({ text: 'late', done: true, error: 'stale error' })
    f.calls[0]!.finish()
    await first
    const entries = f.state.sessions.get('7')!.exchanges
    expect(entries[0]!.markdown).toBe('partial')
    expect(entries[0]!.status).toBe('stopped')
    expect(entries[1]!.status).toBe('running')
    expect(entries[1]!.error).toBe('')
    f.calls[1]!.update({ text: 'new', done: true })
    f.calls[1]!.finish()
    await second
    expect(entries[1]!.markdown).toBe('new')
    f.scope.stop()
  })

  test('clearing one session cancels only its request and discards late responses after reopening', async () => {
    const f = fixture()
    const first = f.submit('7', 'first')
    const second = f.submit('9', 'second')
    f.state.clear('7')
    f.state.open('7')
    f.calls[0]!.update({ text: 'late', done: true })
    expect(f.state.sessions.get('7')!.exchanges.length).toBe(0)
    expect(f.calls[0]!.signal.aborted).toBe(true)
    expect(f.calls[1]!.signal.aborted).toBe(false)
    f.scope.stop()
    expect(f.calls[1]!.signal.aborted).toBe(true)
    for (const call of f.calls) call.finish()
    await Promise.all([first, second])
  })

  test('failed requests preserve received content and can be followed by a new question', async () => {
    const f = fixture()
    const first = f.submit('7', 'first')
    f.calls[0]!.update({ text: 'partial', done: false })
    f.calls[0]!.fail(new Error('network failed'))
    await first
    expect(f.state.sessions.get('7')!.exchanges[0]!.status).toBe('failed')
    expect(f.state.sessions.get('7')!.exchanges[0]!.markdown).toBe('partial')
    const second = f.submit('7', 'retry')
    f.calls[1]!.finish()
    await second
    f.scope.stop()
  })

  test('history is bounded and old answers are collapsed without being re-requested', async () => {
    const f = fixture()
    for (let n = 0; n < BTW_HISTORY_LIMIT + 2; n++) {
      const done = f.submit('7', `question ${n}`)
      f.calls[n]!.update({ text: `answer ${n}`, done: true })
      f.calls[n]!.finish()
      await done
    }
    const entries = f.state.sessions.get('7')!.exchanges
    expect(entries.length).toBe(BTW_HISTORY_LIMIT)
    expect(entries[0]!.question).toBe('question 2')
    expect(entries[0]!.expanded).toBe(false)
    expect(entries.at(-1)!.expanded).toBe(true)
    f.state.open('7')
    expect(f.calls.length).toBe(BTW_HISTORY_LIMIT + 2)
    f.scope.stop()
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
