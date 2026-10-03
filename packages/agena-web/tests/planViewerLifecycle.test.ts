import { describe, expect, test } from 'bun:test'
import { effectScope, ref } from 'vue'
import { usePlanViewer } from '../src/pages/chat/usePlanViewer'

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: Error) => void
  const promise = new Promise<T>((yes, no) => {
    resolve = yes
    reject = no
  })
  return { promise, resolve, reject }
}
const plan = (text: string, autorun = false) => ({ output_text: text, payload: { plan: { autorun } } })
const settle = async () => {
  for (let i = 0; i < 6; i++) await Promise.resolve()
}

function fixture() {
  const open = ref(true)
  const session = ref<string | null>('1')
  const calls: Array<{
    sessionId: string | null
    tool: string
    signal: AbortSignal
    response: ReturnType<typeof deferred<ReturnType<typeof plan>>>
  }> = []
  const scope = effectScope()
  const state = scope.run(() =>
    usePlanViewer(
      () => [open.value, session.value],
      async (sessionId, tool, _input, signal) => {
        const response = deferred<ReturnType<typeof plan>>()
        calls.push({ sessionId, tool, signal, response })
        return await response.promise
      },
    ),
  )!
  return { open, session, calls, scope, state }
}

describe('plan viewer request ownership', () => {
  test('old reads cannot populate a new session even if transport ignores abort', async () => {
    const f = fixture()
    f.session.value = '2'
    expect(f.calls[0]!.signal.aborted).toBe(true)
    expect(f.state.markdown.value).toBe('')
    f.calls[1]!.response.resolve(plan('second'))
    await settle()
    f.calls[0]!.response.resolve(plan('first', true))
    await settle()
    expect(f.state.markdown.value).toBe('second')
    expect(f.state.autorun.value).toBe(false)
    f.scope.stop()
  })

  test('late toggle completion cannot refresh or clear the new session', async () => {
    const f = fixture()
    f.calls[0]!.response.resolve(plan('first'))
    await settle()
    void f.state.toggleAutorun()
    void f.state.toggleAutorun()
    expect(f.calls.length).toBe(2)
    f.session.value = '2'
    expect(f.calls[1]!.signal.aborted).toBe(true)
    f.calls[2]!.response.resolve(plan('second'))
    await settle()
    f.calls[1]!.response.reject(new Error('late old error'))
    await settle()
    expect(f.calls.length).toBe(3)
    expect(f.state.markdown.value).toBe('second')
    expect(f.state.error.value).toBe('')
    f.scope.stop()
  })

  test('refresh is coalesced and disposal aborts outstanding work', async () => {
    const f = fixture()
    void f.state.refresh()
    void f.state.refresh()
    expect(f.calls.length).toBe(1)
    f.scope.stop()
    expect(f.calls[0]!.signal.aborted).toBe(true)
    f.calls[0]!.response.resolve(plan('closed'))
    await settle()
    expect(f.state.markdown.value).toBe('')
  })

  test('successful autorun mutation refreshes the same session exactly once', async () => {
    const f = fixture()
    f.calls[0]!.response.resolve(plan('first'))
    await settle()
    void f.state.toggleAutorun()
    void f.state.refresh()
    f.calls[1]!.response.resolve(plan('toggled', true))
    await settle()
    expect(f.calls.map((c) => [c.sessionId, c.tool])).toEqual([
      ['1', 'get'],
      ['1', 'phase'],
      ['1', 'get'],
    ])
    f.calls[2]!.response.resolve(plan('updated', true))
    await settle()
    expect(f.state.autorun.value).toBe(true)
    expect(f.state.loading.value).toBe(false)
    f.scope.stop()
  })
})
