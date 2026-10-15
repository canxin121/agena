import assert from 'node:assert/strict'
import test from 'node:test'
import { ref } from 'vue'
import { useGitStatusPaged } from '../src/composables/git/useGitStatusPaged'
import { useGitStatusSummary } from '../src/composables/git/useGitStatusSummary'
import { createRevalidator } from '../src/lib/revalidation'
import type { GitStatusResponse } from '../src/types/git'

type Scope = 'staged' | 'unstaged' | 'untracked' | 'merge'
const deferred = <T>() => {
  let resolve!: (value: T) => void
  let reject!: (error: Error) => void
  const promise = new Promise<T>((done, fail) => {
    resolve = done
    reject = fail
  })
  return { promise, resolve, reject }
}

function snapshot(current = 'master'): GitStatusResponse {
  const files = (scope: string, count: number) =>
    Array.from({ length: count }, (_, i) => ({
      path: `${scope}/${i}.ts`,
      index: '',
      workingDir: scope === 'untracked' ? '?' : 'M',
    }))
  const page = (scope: string, count: number) => ({
    files: files(scope, count),
    totalFiles: count,
    offset: 0,
    limit: 200,
    hasMore: false,
  })
  return {
    current,
    tracking: null,
    ahead: 0,
    behind: 0,
    files: [],
    totalFiles: 36,
    stagedCount: 0,
    unstagedCount: 31,
    untrackedCount: 5,
    mergeCount: 0,
    offset: 0,
    limit: 200,
    hasMore: false,
    scope: 'all',
    scopePages: { unstaged: page('unstaged', 31), untracked: page('untracked', 5) },
  }
}

for (const obsoleteFailure of [false, true]) {
  test(`an SSE summary arriving before the first HTTP ${obsoleteFailure ? 'failure' : 'response'} cannot strand nonzero file counts`, async () => {
    const directory = ref('/repo')
    const status = ref<GitStatusResponse | null>(null)
    let generation = 0
    const signal = new AbortController().signal
    const first = deferred<GitStatusResponse>()
    const dirty = new Set<Scope>()
    const paged = useGitStatusPaged({
      gitReady: ref(true),
      status,
      pageSize: 200,
      loadStatusPage: async () => {
        throw new Error('batched first pages need no individual reads')
      },
    })
    let reads = 0
    const queue = createRevalidator(
      async () => {
        const scopes = [...dirty]
        dirty.clear()
        await summary.loadSummary('/repo', scopes)
      },
      { intervalMs: 0 },
    )
    const summary = useGitStatusSummary({
      directory,
      status,
      generation: () => generation,
      readSignal: () => signal,
      pages: paged,
      read: async () => (++reads === 1 ? first.promise : snapshot()),
      invalidate: (scopes) => {
        scopes.forEach((scope) => dirty.add(scope))
        queue.invalidate(0)
      },
    })
    try {
      const initial = summary.loadSummary('/repo', ['unstaged', 'untracked'])
      assert.equal(paged.changesListLoading.value, true)
      // Exactly the screenshot state: SSE publishes counts while the first
      // HTTP file read is still pending and no previous rows exist.
      generation++
      status.value = { ...snapshot(), scopePages: undefined }
      assert.equal(paged.changesList.value.length, 0)
      if (obsoleteFailure) first.reject(new Error('obsolete initial failure'))
      else first.resolve(snapshot('obsolete-branch'))
      await initial
      await queue.refresh()
      assert.equal(reads, 2)
      assert.equal(status.value.current, 'master')
      assert.equal(paged.changesCount.value, 31)
      assert.equal(paged.changesList.value.length, 31)
      assert.equal(paged.untrackedCount.value, 5)
      assert.equal(paged.untrackedList.value.length, 5)
      assert.equal(paged.changesListLoading.value, false)
    } finally {
      queue.dispose()
    }
  })
}

test('a superseded summary preserves the page obligations omitted by the newer read', async () => {
  const status = ref<GitStatusResponse | null>(null)
  const requests = [deferred<GitStatusResponse>(), deferred<GitStatusResponse>()]
  const invalidated: Scope[][] = []
  let calls = 0
  const paged = useGitStatusPaged({
    gitReady: ref(true),
    status,
    pageSize: 200,
    loadStatusPage: async () => snapshot(),
  })
  const signal = new AbortController().signal
  const summary = useGitStatusSummary({
    directory: ref('/repo'),
    status,
    generation: () => 0,
    readSignal: () => signal,
    pages: paged,
    read: () => requests[calls++]!.promise,
    invalidate: (scopes) => invalidated.push(scopes),
  })
  const old = summary.loadSummary('/repo', ['unstaged'])
  const newer = summary.loadSummary('/repo', ['untracked'])
  requests[1]!.resolve({ ...snapshot(), scopePages: { untracked: snapshot().scopePages!.untracked } })
  await newer
  requests[0]!.resolve(snapshot('old'))
  await old
  assert.equal(status.value?.current, 'master')
  assert.equal(paged.untrackedList.value.length, 5)
  assert.deepEqual(invalidated, [['unstaged']])
})

test('navigation and hidden-pane cancellation cannot refill old rows or schedule stale retries', async () => {
  const directory = ref('/old')
  const status = ref<GitStatusResponse | null>(null)
  let controller = new AbortController()
  const read = deferred<GitStatusResponse>()
  const invalidated: Scope[][] = []
  const paged = useGitStatusPaged({
    gitReady: ref(true),
    status,
    pageSize: 200,
    loadStatusPage: async () => snapshot(),
  })
  const summary = useGitStatusSummary({
    directory,
    status,
    generation: () => 0,
    readSignal: () => controller.signal,
    pages: paged,
    read: () => read.promise,
    invalidate: (scopes) => invalidated.push(scopes),
  })
  const old = summary.loadSummary('/old', ['unstaged'])
  controller.abort()
  controller = new AbortController()
  directory.value = '/new'
  paged.resetAll()
  read.resolve(snapshot())
  await old
  assert.equal(status.value, null)
  assert.equal(paged.changesList.value.length, 0)
  assert.deepEqual(invalidated, [])
})
