import { afterAll, expect, test } from 'bun:test'
import { createServer } from 'vite'
import { fileURLToPath } from 'node:url'
import { createRenderer, defineComponent, nextTick, ref } from 'vue'
import { createI18n } from 'vue-i18n'
import en from '../src/i18n/messages/en-US'
import zh from '../src/i18n/messages/zh-CN'
import type { SessionFileChange, SessionFileChanges } from '../src/types/sessionFileChanges'
import type { apiJson } from '../src/lib/api'

const vite = await createServer({
  root: fileURLToPath(new URL('..', import.meta.url)),
  server: { middlewareMode: true, watch: null, hmr: false },
})
afterAll(() => vite.close())
const { useWorkspaceChanges } = (await vite.ssrLoadModule(
  '/src/pages/chat/useWorkspaceChanges.ts',
)) as typeof import('../src/pages/chat/useWorkspaceChanges')

test('workspace and task labels resolve in the chat namespace with interpolated values', () => {
  for (const [locale, messages, title, count, retry] of [
    ['zh-CN', zh, '会话编辑', '3 个文件', '第 2 次重试'],
    ['en-US', en, 'Session edits', '3 files', 'Retry attempt 2'],
  ] as const) {
    const { t, te } = createI18n({ legacy: false, locale, messages: { [locale]: messages } }).global
    expect(t('chat.sessionWork.changes')).toBe(title)
    expect(t('chat.sessionWork.files', { count: 3 })).toBe(count)
    expect(t('chat.sessionWork.retry', { attempt: 2 })).toBe(retry)
    expect(t('chat.sessionWork.pageRange', { start: 41, end: 80, total: 83 })).toBe('41–80 / 83')
    for (const key of [
      'refresh',
      'clean',
      'notRepository',
      'loadFailed',
      'loading',
      'moreDiff',
      'tasks',
      'permission',
      'review',
      'question',
      'resolve',
      'stop',
      'pause',
      'resume',
      'delete',
      'dismiss',
      ...['pending', 'running', 'waiting', 'paused', 'succeeded', 'failed', 'cancelled', 'stopped'].map(
        (s) => `status.${s}`,
      ),
    ]) {
      expect(te(`chat.sessionWork.${key}`)).toBe(true)
    }
  }
})

type Call = {
  path: string
  query: Record<string, string>
  signal: AbortSignal
  resolve: (value: unknown) => void
  reject: (error: Error) => void
}
const file = (path: string): SessionFileChange => ({
  path,
  operation_count: 1,
  operation_history: false,
  operations: [],
})
const snapshot = (files: SessionFileChange[], total_files = files.length, offset = 0): SessionFileChanges => ({
  files,
  total_files,
  offset,
  has_more: offset + files.length < total_files,
  recording_incomplete: false,
})

async function withPanel(
  run: (ctx: {
    state: ReturnType<typeof useWorkspaceChanges>
    sessionId: ReturnType<typeof ref<string>>
    calls: Call[]
    advance: (ms: number) => Promise<void>
    settle: () => Promise<void>
  }) => Promise<void>,
) {
  const original = {
    document: globalThis.document,
    setTimeout: globalThis.setTimeout,
    clearTimeout: globalThis.clearTimeout,
    now: Date.now,
  }
  let now = 100_000,
    serial = 0
  const timers = new Map<number, { at: number; callback: () => void }>()
  Object.assign(globalThis, {
    document: Object.assign(new EventTarget(), { hidden: false }),
    setTimeout: (callback: () => void, delay: number) => {
      const id = ++serial
      timers.set(id, { at: now + delay, callback })
      return id
    },
    clearTimeout: (id: number) => timers.delete(id),
  })
  Date.now = () => now
  const settle = async () => {
    for (let i = 0; i < 8; i++) await nextTick()
  }
  const advance = async (ms: number) => {
    const end = now + ms
    while (true) {
      const next = [...timers].filter(([, t]) => t.at <= end).sort((a, b) => a[1].at - b[1].at)[0]
      if (!next) break
      const [id, timer] = next
      now = timer.at
      timers.delete(id)
      timer.callback()
      await settle()
    }
    now = end
    await settle()
  }
  const renderer = createRenderer<Record<string, never>, Record<string, never>>({
    createElement: () => ({}),
    createText: () => ({}),
    createComment: () => ({}),
    insert() {},
    remove() {},
    setText() {},
    setElementText() {},
    patchProp() {},
    parentNode: () => null,
    nextSibling: () => null,
  })
  const sessionId = ref('7'),
    calls: Call[] = []
  let state!: ReturnType<typeof useWorkspaceChanges>
  const app = renderer.createApp(
    defineComponent({
      setup() {
        state = useWorkspaceChanges({
          sessionId,
          busy: ref(false),
          request: ((url, init) => {
            const parsed = new URL(url, 'http://test')
            return new Promise((resolve, reject) =>
              calls.push({
                path: parsed.pathname,
                query: Object.fromEntries(parsed.searchParams),
                signal: init!.signal as AbortSignal,
                resolve,
                reject,
              }),
            )
          }) as typeof apiJson,
        })
        return () => null
      },
    }),
  )
  try {
    app.mount({})
    await run({ state, sessionId, calls, advance, settle })
  } finally {
    app.unmount()
    Date.now = original.now
    Object.assign(globalThis, {
      document: original.document,
      setTimeout: original.setTimeout,
      clearTimeout: original.clearTimeout,
    })
  }
}

test('session reads stay lazy and discard stale membership when switching sessions', async () =>
  withPanel(async ({ state, sessionId, calls, advance, settle }) => {
    expect(calls[0]!.path).toBe('/api/v1/sessions/7/file-changes')
    expect(calls[0]!.query).toMatchObject({ summary: 'true', limit: '40' })
    calls[0]!.resolve(snapshot([], 83))
    await settle()
    expect(state.total.value).toBe(83)
    state.expanded.value = true
    await advance(750)
    calls.at(-1)!.resolve(snapshot([file('a.ts')], 83))
    await settle()
    state.select(file('a.ts'))
    const oldDiff = calls.at(-1)!
    expect(oldDiff.query.path).toBe('a.ts')
    state.page.value = 2
    expect(oldDiff.signal.aborted).toBe(true)
    await advance(750)
    const oldPage = calls.at(-1)!
    sessionId.value = '8'
    expect(oldPage.signal.aborted).toBe(true)
    expect(state.total.value).toBe(null)
    expect(state.page.value).toBe(0)
    oldPage.resolve(snapshot([file('wrong.ts')], 999, 80))
    oldDiff.resolve(snapshot([file('wrong.ts')]))
    await settle()
    expect(state.total.value).toBe(null)
    expect(state.diff.data.value).toBe(null)
    await advance(750)
    expect(calls.at(-1)!.path).toBe('/api/v1/sessions/8/file-changes')
  }))

test('empty recorded edits close the dock, but incomplete shell recording remains visible', async () =>
  withPanel(async ({ state, calls, advance, settle }) => {
    calls[0]!.resolve(snapshot([], 1))
    await settle()
    state.expanded.value = true
    await advance(750)
    calls.at(-1)!.resolve(snapshot([file('modified.ts')]))
    await settle()
    state.select(file('modified.ts'))
    const preview = calls.at(-1)!
    await advance(750)
    void state.status.refresh()
    calls.at(-1)!.resolve(snapshot([], 0))
    await settle()
    expect(state.hasChanges.value).toBe(false)
    expect(state.expanded.value).toBe(false)
    expect(preview.signal.aborted).toBe(true)
    await advance(30_000)
    calls.at(-1)!.resolve({ ...snapshot([], 0), recording_incomplete: true })
    await settle()
    expect(state.hasChanges.value).toBe(true)
    expect(state.recordingIncomplete.value).toBe(true)
  }))

test('operation history and rename facts stay raw; expanding diff cancels older reads', async () =>
  withPanel(async ({ state, calls, advance, settle }) => {
    const row = { ...file('new name.ts'), operation_count: 2, operation_history: true }
    calls[0]!.resolve(snapshot([], 1))
    await settle()
    state.expanded.value = true
    await advance(750)
    calls.at(-1)!.resolve(snapshot([row]))
    await settle()
    state.select(row)
    expect(calls.at(-1)!.query).toMatchObject({ path: 'new name.ts', max_bytes: String(256 * 1024) })
    const recorded: SessionFileChange = {
      ...row,
      operations: [
        {
          part_id: 1,
          tool: 'fs.apply_patch',
          kind: 'moved',
          from_path: 'old.ts',
          before_sha256: null,
          after_sha256: null,
          diff: 'preview',
          diff_truncated: true,
          diff_scope: 'operation',
          diff_unavailable_reason: null,
        },
      ],
    }
    calls.at(-1)!.resolve(snapshot([recorded]))
    await settle()
    await advance(750)
    void state.diff.refresh()
    const oldRead = calls.at(-1)!
    state.moreDiff()
    expect(oldRead.signal.aborted).toBe(true)
    oldRead.resolve(snapshot([row]))
    await settle()
    expect(state.visibleDiff.value?.files[0]?.operations[0]?.diff).toBe('preview')
    await advance(750)
    expect(calls.at(-1)!.query.max_bytes).toBe(String(512 * 1024))
    calls.at(-1)!.resolve(snapshot([recorded]))
    await settle()
    expect(state.visibleDiff.value?.files[0]?.operation_history).toBe(true)
    expect(state.visibleDiff.value?.files[0]?.operations[0]?.from_path).toBe('old.ts')
  }))

test('refresh failures hide stale rows and shrinking pages recover', async () =>
  withPanel(async ({ state, calls, advance, settle }) => {
    calls[0]!.resolve(snapshot([], 83))
    await settle()
    state.expanded.value = true
    await advance(750)
    calls.at(-1)!.resolve(snapshot([file('a.ts')], 83))
    await settle()
    state.select(file('a.ts'))
    calls.at(-1)!.resolve(snapshot([file('a.ts')]))
    await settle()
    await advance(750)
    void state.status.refresh()
    calls.at(-1)!.reject(new Error('connection unavailable'))
    await settle()
    expect(state.status.error.value).toBe('connection unavailable')
    expect(state.files.value).toEqual([])
    expect(state.diff.data.value).toBe(null)
    state.page.value = 2
    await advance(750)
    calls.at(-1)!.resolve(snapshot([], 41, 80))
    await settle()
    expect(state.page.value).toBe(1)
    await advance(750)
    expect(calls.at(-1)!.query.offset).toBe('40')
    calls.at(-1)!.resolve(snapshot([file('last.ts')], 41, 40))
    await settle()
    expect(state.files.value.map((f) => f.path)).toEqual(['last.ts'])
  }))
