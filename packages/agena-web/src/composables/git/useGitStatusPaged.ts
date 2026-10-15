import { computed, ref } from 'vue'

import type { GitStatusFile, GitStatusResponse, GitStatusScopePage } from '@/types/git'
import { limitBackgroundReads } from '../../lib/backgroundReads'

export function useGitStatusPaged(opts: {
  gitReady: { value: boolean }
  status: { value: GitStatusResponse | null }
  pageSize: number
  loadStatusPage: (args: {
    directory: string
    scope: 'staged' | 'unstaged' | 'untracked' | 'merge'
    offset: number
    limit: number
    snapshotId?: string
    signal?: AbortSignal
  }) => Promise<GitStatusResponse>
}) {
  const mergeList = ref<GitStatusFile[]>([])
  const stagedList = ref<GitStatusFile[]>([])
  const changesList = ref<GitStatusFile[]>([])
  const untrackedList = ref<GitStatusFile[]>([])

  const mergeListLoading = ref(false)
  const stagedListLoading = ref(false)
  const changesListLoading = ref(false)
  const untrackedListLoading = ref(false)

  const mergeCount = computed(() => opts.status.value?.mergeCount ?? mergeList.value.length)
  const stagedCount = computed(() => opts.status.value?.stagedCount ?? stagedList.value.length)
  const changesCount = computed(() => opts.status.value?.unstagedCount ?? changesList.value.length)
  const untrackedCount = computed(() => opts.status.value?.untrackedCount ?? untrackedList.value.length)

  type Scope = 'staged' | 'unstaged' | 'untracked' | 'merge'
  const pageByScope = ref<Partial<Record<Scope, { offset: number; hasMore: boolean }>>>({})
  const hasMoreMerge = computed(() => pageByScope.value.merge?.hasMore ?? mergeList.value.length < mergeCount.value)
  const hasMoreStaged = computed(() => pageByScope.value.staged?.hasMore ?? stagedList.value.length < stagedCount.value)
  const hasMoreUnstaged = computed(
    () => pageByScope.value.unstaged?.hasMore ?? changesList.value.length < changesCount.value,
  )
  const hasMoreUntracked = computed(
    () => pageByScope.value.untracked?.hasMore ?? untrackedList.value.length < untrackedCount.value,
  )

  let generation = 0
  const snapshotByScope = new Map<Scope, string>()
  type PageRead = {
    directory: string
    controller: AbortController
    promise: Promise<void>
    reload: boolean
  }
  const flights = new Map<Scope, PageRead>()
  const firstPageReads = new Map<Scope, symbol>()
  const allowedAt = new Map<Scope, number>()
  const failures = new Map<Scope, number>()

  function listForScope(scope: Scope) {
    if (scope === 'merge') return mergeList
    if (scope === 'staged') return stagedList
    if (scope === 'unstaged') return changesList
    return untrackedList
  }

  function loadingForScope(scope: Scope) {
    if (scope === 'merge') return mergeListLoading
    if (scope === 'staged') return stagedListLoading
    if (scope === 'unstaged') return changesListLoading
    return untrackedListLoading
  }

  function updateLoading(scope: Scope) {
    loadingForScope(scope).value = flights.has(scope) || firstPageReads.has(scope)
  }

  function beginFirstPages(scopes: Scope[]) {
    const owner = Symbol('status first pages')
    for (const scope of scopes) {
      firstPageReads.set(scope, owner)
      updateLoading(scope)
    }
    return () => {
      for (const scope of scopes) {
        if (firstPageReads.get(scope) !== owner) continue
        firstPageReads.delete(scope)
        updateLoading(scope)
      }
    }
  }

  function mapFiles(resp: GitStatusResponse): GitStatusFile[] {
    const diffStats = resp?.diffStats || {}
    if (!Array.isArray(resp?.files)) return []
    return resp.files.map((file) => {
      const stat = diffStats[file.path]
      if (!stat) return file
      return {
        ...file,
        insertions: stat.insertions,
        deletions: stat.deletions,
      }
    })
  }

  async function waitForCooldown(scope: Scope, signal: AbortSignal) {
    signal.throwIfAborted()
    const delay = Math.max(0, (allowedAt.get(scope) ?? 0) - Date.now())
    if (!delay) return
    await new Promise<void>((resolve, reject) => {
      const abort = () => {
        clearTimeout(timer)
        reject(signal.reason)
      }
      const timer = setTimeout(() => {
        signal.removeEventListener('abort', abort)
        resolve()
      }, delay)
      signal.addEventListener('abort', abort, { once: true })
    })
  }

  function readPage(directory: string, scope: Scope, firstPage: boolean): Promise<void> {
    const trimmedDirectory = directory.trim()
    if (!opts.gitReady.value || !trimmedDirectory) return Promise.resolve()
    const existing = flights.get(scope)
    if (existing && existing.directory === trimmedDirectory && !existing.controller.signal.aborted) {
      if (firstPage) existing.reload = true
      // The returned promise includes the one trailing reload, including its
      // failure. Callers cannot consume an invalidation before it was read.
      return existing.promise
    }
    existing?.controller.abort()
    const requestGeneration = generation
    const controller = new AbortController()
    const { signal } = controller
    const list = listForScope(scope)
    const current: PageRead = { directory: trimmedDirectory, controller, promise: undefined!, reload: false }
    current.promise = Promise.resolve()
      .then(async () => {
        let reload = firstPage
        let snapshotForReload: string | undefined
        do {
          await waitForCooldown(scope, signal)
          let offset = 0
          let requestedSnapshotId: string | undefined
          const resp = await limitBackgroundReads(() => {
            // A first-page invalidation received while a load-more request
            // waits for cooldown / a permit changes that queued read itself.
            // Consume it only after choosing the actual dispatch offset.
            reload ||= current.reload
            current.reload = false
            offset = reload ? 0 : (pageByScope.value[scope]?.offset ?? list.value.length)
            requestedSnapshotId = reload ? snapshotForReload : snapshotByScope.get(scope)
            return opts.loadStatusPage({
              directory: trimmedDirectory,
              scope,
              offset,
              limit: opts.pageSize,
              snapshotId: requestedSnapshotId,
              signal: AbortSignal.any([signal, AbortSignal.timeout(30_000)]),
            })
          }, signal)
          signal.throwIfAborted()
          if (requestGeneration !== generation || flights.get(scope) !== current) return
          if (requestedSnapshotId && resp.snapshotId !== requestedSnapshotId) {
            // The bounded server cache evicted this old cursor. Restart from
            // page one against the fresh snapshot instead of mixing revisions.
            // Keep this per scope so other lists can detect their stale cursor
            // and refresh themselves before appending another page.
            if (resp.snapshotId) snapshotByScope.set(scope, resp.snapshotId)
            else snapshotByScope.delete(scope)
            snapshotForReload = resp.snapshotId
            current.reload = true
            reload = true
            continue
          }
          if (resp.snapshotId) snapshotByScope.set(scope, resp.snapshotId)
          else snapshotByScope.delete(scope)
          failures.delete(scope)
          allowedAt.set(scope, Date.now() + 750)
          if (!current.reload) {
            const next = mapFiles(resp)
            pageByScope.value[scope] = { offset: offset + next.length, hasMore: resp.hasMore && next.length > 0 }
            if (reload) list.value = next
            else {
              const existingPaths = new Set(list.value.map((file) => file.path))
              list.value = [...list.value, ...next.filter((file) => !existingPaths.has(file.path))]
            }
          }
          reload = true
        } while (current.reload)
      })
      .catch((error) => {
        if (!signal.aborted && requestGeneration === generation && flights.get(scope) === current) {
          const count = Math.min(5, (failures.get(scope) ?? 0) + 1)
          failures.set(scope, count)
          allowedAt.set(scope, Date.now() + Math.min(60_000, 5000 * 2 ** (count - 1)))
        }
        throw error
      })
      .finally(() => {
        if (flights.get(scope) === current) {
          flights.delete(scope)
          updateLoading(scope)
        }
      })
    flights.set(scope, current)
    updateLoading(scope)
    return current.promise
  }

  function reloadScopeFirstPage(directory: string, scope: Scope) {
    return readPage(directory, scope, true)
  }

  function applyFirstPages(pages: Partial<Record<Scope, GitStatusScopePage>> | undefined, snapshotId?: string) {
    // Watch refreshes can update just one visible scope. Keep the other scopes'
    // snapshot IDs paired with their existing first pages until they refresh.
    if (!pages) return
    for (const scope of Object.keys(pages) as Scope[]) {
      const page = pages[scope]
      if (!page) continue
      if (snapshotId) snapshotByScope.set(scope, snapshotId)
      else snapshotByScope.delete(scope)
      flights.get(scope)?.controller.abort()
      flights.delete(scope)
      listForScope(scope).value = mapFiles({ files: page.files, diffStats: page.diffStats } as GitStatusResponse)
      pageByScope.value[scope] = { offset: page.files.length, hasMore: page.hasMore }
      updateLoading(scope)
      failures.delete(scope)
    }
  }

  async function applySummaryPages(directory: string, response: GitStatusResponse, scopes: Scope[]) {
    applyFirstPages(response.scopePages, response.snapshotId)
    // Older servers can return the summary without batched first pages. Fetch
    // those scopes explicitly so nonzero counts cannot leave empty file lists.
    await Promise.all(
      scopes.map((scope) => {
        if (response.scopePages?.[scope]) return Promise.resolve()
        const count =
          scope === 'staged'
            ? response.stagedCount
            : scope === 'unstaged'
              ? response.unstagedCount
              : scope === 'untracked'
                ? response.untrackedCount
                : response.mergeCount
        if (count === 0) {
          clearScope(scope)
          return Promise.resolve()
        }
        return reloadScopeFirstPage(directory, scope)
      }),
    )
  }

  function loadMore(directory: string, scope: Scope) {
    if (pageByScope.value[scope]?.hasMore === false) return Promise.resolve()
    return readPage(directory, scope, false)
  }

  async function reloadFirstPages(directory: string) {
    const scopes: Scope[] = ['merge', 'staged', 'unstaged', 'untracked']
    await Promise.all(scopes.map((scope) => reloadScopeFirstPage(directory, scope)))
  }

  function cancelRequests() {
    generation++
    firstPageReads.clear()
    for (const read of flights.values()) {
      read.controller.abort()
    }
    flights.clear()
    for (const scope of ['merge', 'staged', 'unstaged', 'untracked'] as const) updateLoading(scope)
  }

  function clearScope(scope: Scope) {
    flights.get(scope)?.controller.abort()
    flights.delete(scope)
    snapshotByScope.delete(scope)
    updateLoading(scope)
    listForScope(scope).value = []
    pageByScope.value[scope] = { offset: 0, hasMore: false }
  }

  function resetAll() {
    cancelRequests()
    snapshotByScope.clear()
    pageByScope.value = {}
    mergeList.value = []
    stagedList.value = []
    changesList.value = []
    untrackedList.value = []
    allowedAt.clear()
    failures.clear()
  }

  return {
    mergeList,
    stagedList,
    changesList,
    untrackedList,
    mergeListLoading,
    stagedListLoading,
    changesListLoading,
    untrackedListLoading,
    mergeCount,
    stagedCount,
    changesCount,
    untrackedCount,
    hasMoreMerge,
    hasMoreStaged,
    hasMoreUnstaged,
    hasMoreUntracked,
    loadMore,
    reloadFirstPages,
    reloadScopeFirstPage,
    applyFirstPages,
    applySummaryPages,
    beginFirstPages,
    cancelRequests,
    clearScope,
    resetAll,
  }
}
