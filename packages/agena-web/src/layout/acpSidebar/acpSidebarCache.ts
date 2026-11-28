import type { NativeIndexCapabilities, NativeIndexedSession } from '@/features/sessions/api/nativeAgentSessions'
import { sessionStorageKeys } from '@/lib/persistence/storageKeys'

export type CachedAcpProject = {
  sessions: NativeIndexedSession[]
  cursor: string | null
  capabilities: NativeIndexCapabilities | null
  showArchived: boolean
  cachedAt: number
}

const MAX_CACHED_PROJECTS = 20
const MAX_CACHE_AGE_MS = 30 * 60 * 1000

type CacheStore = Record<string, CachedAcpProject>

function cacheKey(agent: string, workspaceId: string): string {
  return `${agent}::${workspaceId}`
}

function readStore(): CacheStore {
  try {
    if (typeof sessionStorage === 'undefined') return {}
    const raw = sessionStorage.getItem(sessionStorageKeys.acp.sidebarListCache)
    if (!raw) return {}
    const parsed = JSON.parse(raw)
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return {}
    return parsed as CacheStore
  } catch {
    return {}
  }
}

function writeStore(store: CacheStore): void {
  try {
    if (typeof sessionStorage === 'undefined') return
    sessionStorage.setItem(sessionStorageKeys.acp.sidebarListCache, JSON.stringify(store))
  } catch {
    // Ignore quota or storage access errors.
  }
}

export function readCachedAcpProject(
  agent: string,
  workspaceId: string,
  now = Date.now(),
): CachedAcpProject | null {
  if (!agent || !workspaceId) return null
  const store = readStore()
  const entry = store[cacheKey(agent, workspaceId)]
  if (!entry || !Array.isArray(entry.sessions) || typeof entry.cachedAt !== 'number') {
    return null
  }
  if (now - entry.cachedAt > MAX_CACHE_AGE_MS) {
    delete store[cacheKey(agent, workspaceId)]
    writeStore(store)
    return null
  }
  return entry
}

export function writeCachedAcpProject(
  agent: string,
  workspaceId: string,
  entry: Omit<CachedAcpProject, 'cachedAt'>,
  now = Date.now(),
): void {
  if (!agent || !workspaceId) return
  const store = readStore()
  const key = cacheKey(agent, workspaceId)

  // Drop expired entries first.
  for (const [existingKey, existing] of Object.entries(store)) {
    if (!existing || typeof existing.cachedAt !== 'number' || now - existing.cachedAt > MAX_CACHE_AGE_MS) {
      delete store[existingKey]
    }
  }

  store[key] = {
    sessions: entry.sessions,
    cursor: entry.cursor,
    capabilities: entry.capabilities,
    showArchived: Boolean(entry.showArchived),
    cachedAt: now,
  }

  const keys = Object.keys(store)
  if (keys.length > MAX_CACHED_PROJECTS) {
    keys
      .sort((a, b) => (store[a]?.cachedAt ?? 0) - (store[b]?.cachedAt ?? 0))
      .slice(0, keys.length - MAX_CACHED_PROJECTS)
      .forEach((oldestKey) => {
        delete store[oldestKey]
      })
  }

  writeStore(store)
}

export function invalidateCachedAcpProject(agent?: string, workspaceId?: string): void {
  const store = readStore()
  if (!agent) {
    writeStore({})
    return
  }
  if (workspaceId) {
    delete store[cacheKey(agent, workspaceId)]
  } else {
    const prefix = `${agent}::`
    for (const key of Object.keys(store)) {
      if (key.startsWith(prefix)) delete store[key]
    }
  }
  writeStore(store)
}
