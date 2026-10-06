import { computed, ref } from 'vue'

import type { GitStatusFile, GitStatusResponse } from '@/types/git'
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
  type PageRead = {
    directory: string
    controller: AbortController
    promise: Promise<void>
    reload: boolean
  }
  const flights = new Map<Scope, PageRead>()
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
    const loading = loadingForScope(scope)
    const list = listForScope(scope)
    const current: PageRead = { directory: trimmedDirectory, controller, promise: undefined!, reload: false }
    loading.value = true
    current.promise = Promise.resolve()
      .then(async () => {
        let reload = firstPage
        do {
          await waitForCooldown(scope, signal)
          let offset = 0
          const resp = await limitBackgroundReads(() => {
            // A first-page invalidation received while a load-more request
            // waits for cooldown / a permit changes that queued read itself.
            // Consume it only after choosing the actual dispatch offset.
            reload ||= current.reload
            current.reload = false
            offset = reload ? 0 : (pageByScope.value[scope]?.offset ?? list.value.length)
            return opts.loadStatusPage({
              directory: trimmedDirectory,
              scope,
              offset,
              limit: opts.pageSize,
              signal: AbortSignal.any([signal, AbortSignal.timeout(30_000)]),
            })
          }, signal)
          signal.throwIfAborted()
          if (requestGeneration !== generation || flights.get(scope) !== current) return
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
          loading.value = false
        }
      })
    flights.set(scope, current)
    return current.promise
  }

  function reloadScopeFirstPage(directory: string, scope: Scope) {
    return readPage(directory, scope, true)
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
    for (const [scope, read] of flights) {
      read.controller.abort()
      loadingForScope(scope).value = false
    }
    flights.clear()
  }

  function clearScope(scope: Scope) {
    flights.get(scope)?.controller.abort()
    flights.delete(scope)
    loadingForScope(scope).value = false
    listForScope(scope).value = []
    pageByScope.value[scope] = { offset: 0, hasMore: false }
  }

  function resetAll() {
    cancelRequests()
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
    cancelRequests,
    clearScope,
    resetAll,
  }
}
