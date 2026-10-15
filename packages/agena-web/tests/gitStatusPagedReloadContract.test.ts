import assert from 'node:assert/strict'
import test from 'node:test'
import { ref } from 'vue'

import { useGitStatusPaged } from '../src/composables/git/useGitStatusPaged'

function makeStatus(files: Array<{ path: string; index: string; workingDir: string }>, scope: string) {
  return {
    current: 'main',
    tracking: null,
    ahead: 0,
    behind: 0,
    files,
    totalFiles: files.length,
    stagedCount: 0,
    unstagedCount: 0,
    untrackedCount: 0,
    mergeCount: 0,
    offset: 0,
    limit: 200,
    hasMore: false,
    scope,
  }
}

test('useGitStatusPaged queues first-page reload while a scope is loading', async () => {
  const gitReady = ref(true)
  const status = ref(makeStatus([], 'summary'))
  const calls: string[] = []

  let releaseStagedLoad: (() => void) | null = null
  let stagedLoadMorePending = true
  let started!: () => void
  const firstReadStarted = new Promise<void>((resolve) => {
    started = resolve
  })

  const paged = useGitStatusPaged({
    gitReady,
    status,
    pageSize: 50,
    loadStatusPage: async ({ directory, scope, offset }) => {
      calls.push(`${directory}:${scope}:${offset}`)

      if (scope === 'staged' && offset === 0 && stagedLoadMorePending) {
        await new Promise<void>((resolve) => {
          releaseStagedLoad = resolve
          started()
        })
        stagedLoadMorePending = false
        return makeStatus([{ path: 'staged-old.txt', index: 'M', workingDir: '' }], scope)
      }

      if (scope === 'staged' && offset === 0) {
        return makeStatus([{ path: 'staged-new.txt', index: 'M', workingDir: '' }], scope)
      }

      return makeStatus([], scope)
    },
  })

  const loadMorePromise = paged.loadMore('/repo', 'staged')
  await firstReadStarted

  const reloadPromise = paged.reloadFirstPages('/repo')
  releaseStagedLoad?.()

  await loadMorePromise
  await reloadPromise
  await new Promise((resolve) => setTimeout(resolve, 0))

  assert.deepEqual(
    paged.stagedList.value.map((item) => item.path),
    ['staged-new.txt'],
  )

  const stagedFirstPageCalls = calls.filter((item) => item.includes(':staged:0'))
  assert.equal(stagedFirstPageCalls.length, 2)
})

test('resetting or switching repositories cannot publish an obsolete Git page', async () => {
  let release!: (value: ReturnType<typeof makeStatus>) => void
  let started!: () => void
  const firstReadStarted = new Promise<void>((resolve) => {
    started = resolve
  })
  const paged = useGitStatusPaged({
    gitReady: ref(true),
    status: ref(makeStatus([], 'summary')),
    pageSize: 10,
    loadStatusPage: async () =>
      new Promise((resolve) => {
        release = resolve
        started()
      }),
  })
  const old = paged.loadMore('/old', 'staged')
  const aborted = assert.rejects(old, { name: 'AbortError' })
  await firstReadStarted
  paged.resetAll()
  release(makeStatus([{ path: 'old.txt', index: 'M', workingDir: '' }], 'staged'))
  await aborted
  assert.equal(paged.stagedList.value.length, 0)
  assert.equal(paged.stagedListLoading.value, false)
})

test('overlapping Git pages advance by server rows and an empty page ends loading', async () => {
  const offsets: number[] = []
  const paged = useGitStatusPaged({
    gitReady: ref(true),
    status: ref({ ...makeStatus([], 'summary'), stagedCount: 100 }),
    pageSize: 2,
    loadStatusPage: async ({ offset, scope }) => {
      offsets.push(offset)
      const paths = offset === 0 ? ['a', 'b'] : offset === 2 ? ['b', 'c'] : []
      return {
        ...makeStatus(
          paths.map((path) => ({ path, index: 'M', workingDir: '' })),
          scope,
        ),
        hasMore: true,
      }
    },
  })
  await paged.loadMore('/repo', 'staged')
  await paged.loadMore('/repo', 'staged')
  await paged.loadMore('/repo', 'staged')
  assert.deepEqual(offsets, [0, 2, 4])
  assert.deepEqual(
    paged.stagedList.value.map((file) => file.path),
    ['a', 'b', 'c'],
  )
  assert.equal(paged.hasMoreStaged.value, false)
})

test('a counts-only status response loads the missing visible file pages', async () => {
  const summary = { ...makeStatus([], 'all'), unstagedCount: 31, untrackedCount: 5 }
  const calls: string[] = []
  const paged = useGitStatusPaged({
    gitReady: ref(true),
    status: ref(summary),
    pageSize: 200,
    loadStatusPage: async ({ scope, offset }) => {
      calls.push(`${scope}:${offset}`)
      const count = scope === 'unstaged' ? 31 : 5
      return makeStatus(
        Array.from({ length: count }, (_, index) => ({
          path: `${scope}/${index}.ts`,
          index: '',
          workingDir: scope === 'untracked' ? '?' : 'M',
        })),
        scope,
      )
    },
  })
  const finish = paged.beginFirstPages(['staged', 'unstaged', 'untracked'])
  assert.equal(paged.changesListLoading.value, true)
  await paged.applySummaryPages('/repo', summary, ['staged', 'unstaged', 'untracked'])
  finish()
  assert.deepEqual(calls.sort(), ['unstaged:0', 'untracked:0'])
  assert.equal(paged.changesList.value.length, 31)
  assert.equal(paged.untrackedList.value.length, 5)
  assert.equal(paged.stagedList.value.length, 0)
  assert.equal(paged.changesListLoading.value, false)
  assert.equal(paged.hasMoreStaged.value, false)
})

test('partial batched responses retain their pages and fetch only omitted scopes', async () => {
  const calls: string[] = []
  const summary = {
    ...makeStatus([], 'all'),
    stagedCount: 1,
    unstagedCount: 1,
    snapshotId: 'batch-snapshot',
    scopePages: {
      staged: {
        files: [{ path: 'staged.ts', index: 'M', workingDir: '' }],
        totalFiles: 1,
        offset: 0,
        limit: 50,
        hasMore: false,
        diffStats: { 'staged.ts': { insertions: 10, deletions: 2 } },
      },
    },
  }
  const paged = useGitStatusPaged({
    gitReady: ref(true),
    status: ref(summary),
    pageSize: 50,
    loadStatusPage: async ({ scope }) => {
      calls.push(scope)
      return makeStatus([{ path: 'working.ts', index: '', workingDir: 'M' }], scope)
    },
  })
  await paged.applySummaryPages('/repo', summary, ['staged', 'unstaged'])
  assert.deepEqual(calls, ['unstaged'])
  assert.equal(paged.stagedList.value[0]?.path, 'staged.ts')
  assert.equal(paged.stagedList.value[0]?.insertions, 10)
  assert.equal(paged.changesList.value[0]?.path, 'working.ts')
})

test('superseded batched reads cannot hide the current first-page loading state', () => {
  const paged = useGitStatusPaged({
    gitReady: ref(true),
    status: ref(makeStatus([], 'all')),
    pageSize: 50,
    loadStatusPage: async () => makeStatus([], 'all'),
  })
  const old = paged.beginFirstPages(['unstaged', 'untracked'])
  const current = paged.beginFirstPages(['unstaged'])
  old()
  assert.equal(paged.changesListLoading.value, true)
  assert.equal(paged.untrackedListLoading.value, false)
  current()
  assert.equal(paged.changesListLoading.value, false)
  const beforeReset = paged.beginFirstPages(['unstaged'])
  paged.resetAll()
  assert.equal(paged.changesListLoading.value, false)
  const afterReset = paged.beginFirstPages(['unstaged'])
  beforeReset()
  assert.equal(paged.changesListLoading.value, true)
  afterReset()
  assert.equal(paged.changesListLoading.value, false)
})
