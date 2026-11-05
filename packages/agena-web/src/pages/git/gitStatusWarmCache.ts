import type { GitStatusPagesSnapshot } from '@/composables/git/useGitStatusPaged'
import { readActiveBackendBaseUrl } from '@/lib/backend'
import { readServerAuthTokenVersion } from '@/lib/serverAuthToken'
import type { GitStatusResponse, GitWatchStatusPayload } from '@/types/git'

/** Only used when opening another diff tab for the same repository. */
export type GitStatusWarmSnapshot = {
  status: GitStatusResponse
  pages: GitStatusPagesSnapshot
  watchBaseline: GitWatchStatusPayload | null
}

const MAX_AGE_MS = 15_000
const MAX_REPOS = 8
const snapshots = new Map<string, { at: number; snapshot: GitStatusWarmSnapshot }>()

function cacheKey(repo: string) {
  return JSON.stringify([readActiveBackendBaseUrl(), readServerAuthTokenVersion(), repo.trim()])
}

export function rememberGitStatusForDiff(repo: string, snapshot: GitStatusWarmSnapshot, now = Date.now()) {
  if (!repo.trim()) return
  const key = cacheKey(repo)
  snapshots.delete(key)
  snapshots.set(key, { at: now, snapshot })
  if (snapshots.size > MAX_REPOS) snapshots.delete(snapshots.keys().next().value!)
}

export function recentGitStatusForDiff(repo: string, now = Date.now()): GitStatusWarmSnapshot | null {
  if (!repo.trim()) return null
  const key = cacheKey(repo)
  const entry = snapshots.get(key)
  if (!entry) return null
  if (now < entry.at || now - entry.at > MAX_AGE_MS) {
    snapshots.delete(key)
    return null
  }
  return entry.snapshot
}
