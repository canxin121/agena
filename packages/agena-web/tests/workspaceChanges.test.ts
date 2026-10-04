import { afterAll, expect, test } from 'bun:test'
import { createServer } from 'vite'
import { fileURLToPath } from 'node:url'
import { createRenderer, defineComponent, nextTick, ref } from 'vue'
import { createI18n } from 'vue-i18n'
import en from '../src/i18n/messages/en-US'
import zh from '../src/i18n/messages/zh-CN'
import type { GitStatusFile, GitStatusResponse } from '../src/types/git'
import type { gitJson } from '../src/lib/gitApi'

const vite = await createServer({
  root: fileURLToPath(new URL('..', import.meta.url)),
  server: { middlewareMode: true, watch: null, hmr: false },
})
afterAll(() => vite.close())
const { useWorkspaceChanges } = (await vite.ssrLoadModule(
  '/src/pages/chat/useWorkspaceChanges.ts',
)) as typeof import('../src/pages/chat/useWorkspaceChanges')
const { ApiError } = (await vite.ssrLoadModule('/src/lib/api.ts')) as typeof import('../src/lib/api')

test('workspace and task labels resolve in the chat namespace with interpolated values', () => {
  for (const [locale, messages, title, count, retry] of [
    ['zh-CN', zh, '工作区变更', '3 个文件', '第 2 次重试'],
    ['en-US', en, 'Workspace changes', '3 files', 'Retry attempt 2'],
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
  directory: string
  query: Parameters<typeof gitJson>[2]
  signal: AbortSignal
  resolve: (value: unknown) => void
  reject: (error: Error) => void
}
const file = (path: string): GitStatusFile => ({ path, index: '', workingDir: 'M' })
const snapshot = (files: GitStatusFile[], totalFiles = files.length, offset = 0): GitStatusResponse => ({
  current: 'master',
  tracking: null,
  ahead: 0,
  behind: 0,
  files,
  totalFiles,
  offset,
  limit: 40,
  stagedCount: 0,
  unstagedCount: totalFiles,
  untrackedCount: 0,
  mergeCount: 0,
  hasMore: offset + files.length < totalFiles,
  scope: 'all',
})

async function withPanel(
  run: (ctx: {
    state: ReturnType<typeof useWorkspaceChanges>
    directory: ReturnType<typeof ref<string>>
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
  const directory = ref('/repo/a'),
    calls: Call[] = []
  let state!: ReturnType<typeof useWorkspaceChanges>
  const app = renderer.createApp(
    defineComponent({
      setup() {
        state = useWorkspaceChanges({
          directory,
          busy: ref(false),
          request: ((path, directory, query, init) =>
            new Promise((resolve, reject) =>
              calls.push({ path, directory, query, signal: init!.signal as AbortSignal, resolve, reject }),
            )) as typeof gitJson,
        })
        return () => null
      },
    }),
  )
  try {
    app.mount({})
    await run({ state, directory, calls, advance, settle })
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

test('workspace reads stay lazy and discard old pages and counts when changing directories', async () =>
  withPanel(async ({ state, directory, calls, advance, settle }) => {
    expect(calls[0]!.query).toMatchObject({ summary: true, limit: 40, includeDiffStats: false })
    calls[0]!.resolve(snapshot([], 83))
    await settle()
    expect(state.total.value).toBe(83)
    expect(calls.filter((c) => c.path === 'diff')).toHaveLength(0)
    state.expanded.value = true
    expect(state.statusPending.value).toBe(true)
    await advance(750)
    calls.at(-1)!.resolve(snapshot([file('a.ts')], 83))
    await settle()
    state.select(file('a.ts'))
    const oldDiff = calls.at(-1)!
    state.page.value = 2
    expect(state.selected.value).toBe(null)
    expect(oldDiff.signal.aborted).toBe(true)
    expect(state.files.value).toEqual([])
    await advance(750)
    const oldPage = calls.at(-1)!
    directory.value = '/repo/b'
    expect(oldPage.signal.aborted).toBe(true)
    expect(state.total.value).toBe(null)
    expect(state.page.value).toBe(0)
    oldPage.resolve(snapshot([file('wrong.ts')], 999, 80))
    oldDiff.resolve({ diff: 'old workspace' })
    await settle()
    expect(state.total.value).toBe(null)
    expect(state.diff.data.value).toBe(null)
    await advance(750)
    const error = new ApiError('not a repository', 400)
    error.code = 'not_git_repo'
    calls.at(-1)!.reject(error)
    await settle()
    expect(state.notRepository.value).toBe(true)
    expect(state.status.error.value).toBe('')
    expect(state.total.value).toBe(null)
  }))

test('diff expansion cancels an older read and uses the correct rename side and new budget', async () =>
  withPanel(async ({ state, calls, advance, settle }) => {
    const renamed = {
      path: 'new.ts',
      index: 'R',
      workingDir: 'R',
      indexOldPath: 'index-old.ts',
      workingOldPath: 'work-old.ts',
    }
    calls[0]!.resolve(snapshot([], 1))
    await settle()
    state.expanded.value = true
    await advance(750)
    calls.at(-1)!.resolve(snapshot([renamed]))
    await settle()
    state.select(renamed)
    expect(calls.at(-1)!.query).toMatchObject({ staged: false, oldPath: 'work-old.ts', maxBytes: 256 * 1024 })
    calls.at(-1)!.resolve({ diff: 'preview', truncated: true })
    await settle()
    await advance(750)
    void state.diff.refresh()
    const oldRead = calls.at(-1)!
    state.moreDiff()
    expect(oldRead.signal.aborted).toBe(true)
    oldRead.resolve({ diff: 'stale preview', truncated: true })
    await settle()
    expect(state.diff.data.value).toBe(null)
    expect(state.diffPending.value).toBe(true)
    expect(state.visibleDiff.value?.diff).toBe('preview')
    await advance(750)
    expect(calls.at(-1)!.query?.maxBytes).toBe(512 * 1024)
    calls.at(-1)!.resolve({ diff: 'expanded diff', truncated: false })
    await settle()
    expect(state.diff.data.value?.diff).toBe('expanded diff')
    await advance(750)
    void state.status.refresh()
    calls.at(-1)!.resolve(snapshot([{ ...renamed }]))
    await settle()
    expect(state.visibleDiff.value?.diff).toBe('expanded diff')
    void state.diff.refresh()
    expect(calls.at(-1)!.query?.maxBytes).toBe(512 * 1024)
    state.staged.value = true
    expect(state.visibleDiff.value).toBe(null)
    await advance(750)
    expect(calls.at(-1)!.query).toMatchObject({ staged: true, oldPath: 'index-old.ts', maxBytes: 256 * 1024 })
  }))

test('refresh failures hide stale rows and a shrinking last page recovers to the new last page', async () =>
  withPanel(async ({ state, calls, advance, settle }) => {
    calls[0]!.resolve(snapshot([], 83))
    await settle()
    state.expanded.value = true
    await advance(750)
    calls.at(-1)!.resolve(snapshot([file('a.ts')], 83))
    await settle()
    state.select(file('a.ts'))
    calls.at(-1)!.resolve({ diff: 'diff' })
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
    expect(state.selected.value).toBe(null)
    await advance(750)
    expect(calls.at(-1)!.query?.offset).toBe(40)
    calls.at(-1)!.resolve(snapshot([file('last.ts')], 41, 40))
    await settle()
    expect(state.current.value?.offset).toBe(40)
    expect(state.files.value.map((f) => f.path)).toEqual(['last.ts'])
  }))
