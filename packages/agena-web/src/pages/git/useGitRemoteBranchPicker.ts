import { computed, getCurrentScope, onScopeDispose, ref, watch, type Ref } from 'vue'
import { createLatestRequestGuard } from '@/lib/latestRequest'
import { captureResourceObservation } from '@/lib/resourceSync'
import { subscribeGitMutations } from '@/lib/gitApi'
import type { JsonValue } from '@/types/json'

type QueryValue = string | number | boolean | null | undefined

type GitJson = <T = JsonValue>(
  endpoint: string,
  directory: string,
  query?: Record<string, QueryValue> | undefined,
  init?: RequestInit | undefined,
) => Promise<T>

type GitRemoteBranchListResponse = {
  branches?: string[]
}

export function useGitRemoteBranchPicker(opts: {
  gitJson: GitJson
  repoRoot: Ref<string | null>
  pushToOpen: Ref<boolean>
  pullFromOpen: Ref<boolean>
  fetchFromOpen: Ref<boolean>
  targetRemote: Ref<string>
  targetBranch: Ref<string>
}) {
  const { gitJson, repoRoot, pushToOpen, pullFromOpen, fetchFromOpen, targetRemote, targetBranch } = opts

  const remoteBranchOptions = ref<string[]>([])
  const remoteBranchLoading = ref(false)

  type RemoteBranchCacheEntry = { fetchedAt: number; branches: string[] }
  const REMOTE_BRANCH_CACHE_TTL_MS = 60_000
  const remoteBranchCache = ref<Record<string, RemoteBranchCacheEntry>>({})
  const remoteBranchInflight = ref<Record<string, Promise<string[]>>>({})
  let remoteBranchFetchTimer: number | null = null
  const generations = new Map<string, number>()
  const beginRead = createLatestRequestGuard(
    () => `${captureResourceObservation('sessions').scope}:${repoRoot.value}:${targetRemote.value}`,
    remoteBranchOptions,
  )

  const filteredRemoteBranchOptions = computed(() => {
    const q = (targetBranch.value || '').trim().toLowerCase()
    const list = remoteBranchOptions.value
    if (!q) return list
    return list.filter((b) => b.toLowerCase().includes(q))
  })

  const branchPickVisible = ref(false)
  const branchPickIndex = ref(0)

  function clampBranchPickIndex() {
    const max = Math.min(20, filteredRemoteBranchOptions.value.length) - 1
    if (max < 0) {
      branchPickIndex.value = 0
      return
    }
    if (branchPickIndex.value < 0) branchPickIndex.value = 0
    if (branchPickIndex.value > max) branchPickIndex.value = max
  }

  function onBranchPickKeydown(ev: KeyboardEvent) {
    if (!branchPickVisible.value) return
    const list = filteredRemoteBranchOptions.value.slice(0, 20)
    if (!list.length) return

    if (ev.key === 'ArrowDown') {
      ev.preventDefault()
      branchPickIndex.value = Math.min(list.length - 1, branchPickIndex.value + 1)
      return
    }
    if (ev.key === 'ArrowUp') {
      ev.preventDefault()
      branchPickIndex.value = Math.max(0, branchPickIndex.value - 1)
      return
    }
    if (ev.key === 'Enter') {
      ev.preventDefault()
      const b = list[branchPickIndex.value]
      if (b) targetBranch.value = b
      branchPickVisible.value = false
      return
    }
    if (ev.key === 'Escape') {
      ev.preventDefault()
      branchPickVisible.value = false
    }
  }

  function hideBranchPickSoon() {
    window.setTimeout(() => {
      branchPickVisible.value = false
    }, 100)
  }

  async function loadRemoteBranches(directory: string, remote: string) {
    const r = (remote || '').trim()
    if (!r) return

    const dir = (directory || '').trim()
    if (!dir) return

    const prefix = `${captureResourceObservation('sessions').scope}:${dir}::`
    const key = `${prefix}${r}`
    const generation = generations.get(prefix) ?? 0
    const latest = beginRead()
    const isCurrent = () => latest() && generation === (generations.get(prefix) ?? 0)
    const now = Date.now()
    const cached = remoteBranchCache.value[key]
    if (cached && now - cached.fetchedAt < REMOTE_BRANCH_CACHE_TTL_MS) {
      remoteBranchOptions.value = cached.branches
      remoteBranchLoading.value = false
      return
    }

    const inflight = remoteBranchInflight.value[key]
    if (inflight) {
      remoteBranchLoading.value = true
      try {
        const branches = await inflight
        if (isCurrent()) remoteBranchOptions.value = branches
      } catch (error) {
        if (isCurrent()) remoteBranchOptions.value = []
      } finally {
        if (isCurrent()) remoteBranchLoading.value = false
      }
      return
    }

    remoteBranchLoading.value = true
    const p = (async () => {
      const resp = await gitJson<GitRemoteBranchListResponse>('remote-branches', dir, { remote: r })
      const branches = Array.isArray(resp?.branches) ? resp.branches : []
      if (generation === (generations.get(prefix) ?? 0)) {
        const next = { ...remoteBranchCache.value, [key]: { fetchedAt: Date.now(), branches } }
        if (Object.keys(next).length > 64) delete next[Object.keys(next)[0]!]
        remoteBranchCache.value = next
      }
      return branches
    })()

    remoteBranchInflight.value = { ...remoteBranchInflight.value, [key]: p }
    try {
      const branches = await p
      if (isCurrent()) remoteBranchOptions.value = branches
    } catch (error) {
      if (isCurrent()) remoteBranchOptions.value = []
    } finally {
      const next = { ...remoteBranchInflight.value }
      if (next[key] === p) delete next[key]
      remoteBranchInflight.value = next
      if (isCurrent()) remoteBranchLoading.value = false
    }
  }

  watch(
    () => [pushToOpen.value, pullFromOpen.value, fetchFromOpen.value, targetRemote.value, repoRoot.value] as const,
    ([pushOpen, pullOpen, fetchOpen, remote, dir]) => {
      const open = pushOpen || pullOpen || fetchOpen
      if (!open) return
      const d = (dir || '').trim()
      const r = (remote || '').trim()
      if (!d || !r) return

      // Throttle while users switch remotes or open dialogs.
      if (remoteBranchFetchTimer) window.clearTimeout(remoteBranchFetchTimer)
      remoteBranchFetchTimer = window.setTimeout(() => {
        remoteBranchFetchTimer = null
        void loadRemoteBranches(d, r)
      }, 200)
    },
  )

  function clearRemoteBranchOptions() {
    beginRead()
    remoteBranchOptions.value = []
    remoteBranchLoading.value = false
  }

  function prefetchRemoteBranches(remote: string) {
    const dir = (repoRoot.value || '').trim()
    const r = (remote || '').trim()
    if (!dir || !r) return
    void loadRemoteBranches(dir, r)
  }

  if (getCurrentScope()) {
    const release = subscribeGitMutations((directory, path) => {
      if (
        ![
          'fetch',
          'pull',
          'push',
          'remotes',
          'remotes/rename',
          'remotes/set-url',
          'branches/delete-remote',
          'create-github-repo-and-push',
        ].includes(path)
      )
        return
      const prefix = `${captureResourceObservation('sessions').scope}:${directory}::`
      generations.set(prefix, (generations.get(prefix) ?? 0) + 1)
      const keep = <T>(rows: Record<string, T>) =>
        Object.fromEntries(Object.entries(rows).filter(([key]) => !key.startsWith(prefix)))
      remoteBranchCache.value = keep(remoteBranchCache.value)
      remoteBranchInflight.value = keep(remoteBranchInflight.value)
      if (generations.size > 64) generations.delete(generations.keys().next().value!)
      if (repoRoot.value === directory) {
        beginRead()
        remoteBranchLoading.value = false
        if (pushToOpen.value || pullFromOpen.value || fetchFromOpen.value) prefetchRemoteBranches(targetRemote.value)
      }
    })
    onScopeDispose(() => {
      release()
      beginRead()
      if (remoteBranchFetchTimer !== null) window.clearTimeout(remoteBranchFetchTimer)
    })
  }

  watch(
    () => [filteredRemoteBranchOptions.value.length, branchPickVisible.value] as const,
    () => {
      if (!branchPickVisible.value) return
      clampBranchPickIndex()
    },
  )

  return {
    remoteBranchLoading,
    filteredRemoteBranchOptions,
    branchPickVisible,
    branchPickIndex,
    onBranchPickKeydown,
    hideBranchPickSoon,
    clearRemoteBranchOptions,
    prefetchRemoteBranches,
  }
}
