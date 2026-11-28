import { openNativeAgentSession } from '@/features/sessions/api/nativeAgentSessions'
import type { ExternalAgent } from '@/types/chat'
import { isAcpSessionId } from './acpSession'
import { getLocalJson, setLocalJson } from './persist'
import { localStorageKeys } from './persistence/storageKeys'

export type AcpViewBinding = {
  sessionId: string
  workspaceId: number
  agent: ExternalAgent
  nativeId: string
  title?: string
  updatedAt: number
}

export type AcpViewRecoveryResult = {
  sessionId: string
  title?: string
}

const STORAGE_KEY = localStorageKeys.chat.acpViewBindings
const MAX_BINDINGS = 64
const DEFAULT_RETRY_DELAY_MS = 1200

const memoryBindings = new Map<string, AcpViewBinding>()
const recoveredIdByExpiredId = new Map<string, string>()
const inFlightRecoveries = new Map<string, Promise<AcpViewRecoveryResult | null>>()
const changeListeners = new Set<() => void>()
let hydratedFromStorage = false

function notifyChange() {
  for (const listener of changeListeners) {
    try {
      listener()
    } catch {
      // Ignore subscriber errors.
    }
  }
}

function normalizeBinding(raw: unknown): AcpViewBinding | null {
  if (!raw || typeof raw !== 'object') return null
  const rec = raw as Record<string, unknown>
  const sessionId = String(rec.sessionId || '').trim()
  const workspaceId = Number(rec.workspaceId)
  const agent = String(rec.agent || '').trim() as ExternalAgent
  const nativeId = String(rec.nativeId || '').trim()
  const title = typeof rec.title === 'string' && rec.title.trim() ? rec.title.trim() : undefined
  const updatedAtRaw = Number(rec.updatedAt)
  const updatedAt = Number.isFinite(updatedAtRaw) && updatedAtRaw > 0 ? Math.floor(updatedAtRaw) : Date.now()
  if (!isAcpSessionId(sessionId) || !Number.isSafeInteger(workspaceId) || workspaceId <= 0 || !agent || !nativeId) {
    return null
  }
  return { sessionId, workspaceId, agent, nativeId, ...(title ? { title } : {}), updatedAt }
}

function ensureHydrated() {
  if (hydratedFromStorage) return
  hydratedFromStorage = true
  const raw = getLocalJson<unknown>(STORAGE_KEY, [])
  if (!Array.isArray(raw)) return
  for (const item of raw) {
    const binding = normalizeBinding(item)
    if (binding) memoryBindings.set(binding.sessionId, binding)
  }
}

function persistBindings() {
  const ordered = Array.from(memoryBindings.values())
    .sort((a, b) => b.updatedAt - a.updatedAt)
    .slice(0, MAX_BINDINGS)
  if (memoryBindings.size > ordered.length) {
    memoryBindings.clear()
    for (const item of ordered) memoryBindings.set(item.sessionId, item)
  }
  setLocalJson(STORAGE_KEY, ordered)
}

export function subscribeAcpViewBindings(listener: () => void): () => void {
  changeListeners.add(listener)
  return () => {
    changeListeners.delete(listener)
  }
}

export function rememberAcpViewBinding(input: {
  sessionId: string | number | null | undefined
  workspaceId: number | null | undefined
  agent: ExternalAgent | null | undefined
  nativeId: string | null | undefined
  title?: string | null
}): AcpViewBinding | null {
  ensureHydrated()
  const sessionId = String(input.sessionId ?? '').trim()
  const workspaceId = Number(input.workspaceId)
  const agent = String(input.agent ?? '').trim() as ExternalAgent
  const nativeId = String(input.nativeId ?? '').trim()
  if (!isAcpSessionId(sessionId) || !Number.isSafeInteger(workspaceId) || workspaceId <= 0 || !agent || !nativeId) {
    return null
  }
  const existing = memoryBindings.get(sessionId)
  const title = typeof input.title === 'string' && input.title.trim() ? input.title.trim() : existing?.title
  const unchanged =
    existing &&
    existing.workspaceId === workspaceId &&
    existing.agent === agent &&
    existing.nativeId === nativeId &&
    existing.title === title
  const record: AcpViewBinding = {
    sessionId,
    workspaceId,
    agent,
    nativeId,
    ...(title ? { title } : {}),
    updatedAt: Date.now(),
  }
  memoryBindings.set(sessionId, record)
  persistBindings()
  if (!unchanged) notifyChange()
  return record
}

export function updateAcpViewBindingTitle(sessionId: string | null | undefined, title: string | null | undefined) {
  ensureHydrated()
  const sid = String(sessionId ?? '').trim()
  const trimmed = String(title ?? '').trim()
  if (!sid || !trimmed) return
  const existing = memoryBindings.get(sid)
  if (!existing || existing.title === trimmed) return
  memoryBindings.set(sid, { ...existing, title: trimmed, updatedAt: Date.now() })
  persistBindings()
  notifyChange()
}

export function getAcpViewBinding(sessionId: string | null | undefined): AcpViewBinding | null {
  ensureHydrated()
  const sid = String(sessionId ?? '').trim()
  if (!isAcpSessionId(sid)) return null
  const redirected = recoveredIdByExpiredId.get(sid)
  if (redirected) {
    const target = memoryBindings.get(redirected)
    if (target) return target
  }
  return memoryBindings.get(sid) ?? null
}

export function hasAcpViewBinding(sessionId: string | null | undefined): boolean {
  return getAcpViewBinding(sessionId) !== null
}

export function forgetAcpViewBinding(sessionId: string | null | undefined) {
  ensureHydrated()
  const sid = String(sessionId ?? '').trim()
  if (!sid) return
  let changed = memoryBindings.delete(sid)
  if (recoveredIdByExpiredId.delete(sid)) changed = true
  for (const [oldId, newId] of recoveredIdByExpiredId.entries()) {
    if (newId === sid) {
      recoveredIdByExpiredId.delete(oldId)
      changed = true
    }
  }
  if (changed) {
    persistBindings()
    notifyChange()
  }
}

export function forgetAcpViewBindingsByNative(
  workspaceId: number | null | undefined,
  agent: ExternalAgent | null | undefined,
  nativeId: string | null | undefined,
) {
  ensureHydrated()
  const ws = Number(workspaceId)
  const trimmedNative = String(nativeId ?? '').trim()
  if (!Number.isSafeInteger(ws) || ws <= 0 || !agent || !trimmedNative) return
  const removedIds: string[] = []
  for (const [sid, binding] of memoryBindings.entries()) {
    if (binding.workspaceId === ws && binding.agent === agent && binding.nativeId === trimmedNative) {
      memoryBindings.delete(sid)
      removedIds.push(sid)
    }
  }
  if (!removedIds.length) return
  for (const sid of removedIds) {
    recoveredIdByExpiredId.delete(sid)
    for (const [oldId, newId] of recoveredIdByExpiredId.entries()) {
      if (newId === sid) recoveredIdByExpiredId.delete(oldId)
    }
  }
  persistBindings()
  notifyChange()
}

function isAbortError(error: unknown): boolean {
  return error instanceof DOMException
    ? error.name === 'AbortError'
    : Boolean(error && typeof error === 'object' && (error as { name?: string }).name === 'AbortError')
}

function isPermanentMissingError(error: unknown): boolean {
  if (!error || typeof error !== 'object') return false
  const status = (error as { status?: unknown }).status
  return status === 404 || status === 400 || status === 410
}

function waitMs(ms: number, signal?: AbortSignal): Promise<boolean> {
  if (ms <= 0) return Promise.resolve(!signal?.aborted)
  return new Promise((resolve) => {
    if (signal?.aborted) {
      resolve(false)
      return
    }
    const timer = setTimeout(() => {
      signal?.removeEventListener('abort', onAbort)
      resolve(true)
    }, ms)
    const onAbort = () => {
      clearTimeout(timer)
      resolve(false)
    }
    signal?.addEventListener('abort', onAbort, { once: true })
  })
}

/**
 * Automatically re-opens an expired ephemeral ACP view using its remembered
 * native `(workspaceId, agent, nativeId)` binding (for example after an
 * `agena-server` restart rotates the in-memory ACP session ID epoch).
 */
export async function recoverAcpView(
  expiredSessionId: string,
  opts?: {
    openSession?: typeof openNativeAgentSession
    signal?: AbortSignal
    retryDelayMs?: number
  },
): Promise<AcpViewRecoveryResult | null> {
  ensureHydrated()
  const sid = String(expiredSessionId || '').trim()
  if (!isAcpSessionId(sid)) return null

  const alreadyRecovered = recoveredIdByExpiredId.get(sid)
  if (alreadyRecovered) {
    const target = memoryBindings.get(alreadyRecovered)
    return { sessionId: alreadyRecovered, ...(target?.title ? { title: target.title } : {}) }
  }

  const binding = memoryBindings.get(sid)
  if (!binding) return null

  const existing = inFlightRecoveries.get(sid)
  if (existing) return existing

  const openFn = opts?.openSession ?? openNativeAgentSession
  const retryDelayMs = opts?.retryDelayMs ?? DEFAULT_RETRY_DELAY_MS

  const run = (async (): Promise<AcpViewRecoveryResult | null> => {
    for (let attempt = 0; attempt < 2; attempt++) {
      if (opts?.signal?.aborted) return null
      try {
        const openedId = await openFn(binding.workspaceId, binding.agent, binding.nativeId, opts?.signal)
        const nextSid = String(openedId).trim()
        if (!isAcpSessionId(nextSid)) return null
        if (nextSid !== sid) {
          memoryBindings.delete(sid)
          recoveredIdByExpiredId.set(sid, nextSid)
        }
        rememberAcpViewBinding({
          sessionId: nextSid,
          workspaceId: binding.workspaceId,
          agent: binding.agent,
          nativeId: binding.nativeId,
          title: binding.title,
        })
        return { sessionId: nextSid, ...(binding.title ? { title: binding.title } : {}) }
      } catch (error) {
        if (isAbortError(error) || opts?.signal?.aborted) return null
        if (isPermanentMissingError(error)) {
          forgetAcpViewBinding(sid)
          return null
        }
        if (attempt === 0) {
          const ok = await waitMs(retryDelayMs, opts?.signal)
          if (!ok) return null
          continue
        }
        return null
      }
    }
    return null
  })().finally(() => {
    if (inFlightRecoveries.get(sid) === run) inFlightRecoveries.delete(sid)
  })

  inFlightRecoveries.set(sid, run)
  return run
}

export function resetAcpViewRecoveryForTests() {
  memoryBindings.clear()
  recoveredIdByExpiredId.clear()
  inFlightRecoveries.clear()
  changeListeners.clear()
  hydratedFromStorage = false
  try {
    if (typeof window !== 'undefined' && window.localStorage) {
      window.localStorage.removeItem(STORAGE_KEY)
    }
  } catch {
    // Ignore storage errors in non-browser environments.
  }
}
