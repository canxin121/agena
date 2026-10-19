import { encodeModelSelectionKey, parseModelSlug } from './modelSelectionDefaults'
import type { SessionRunConfig } from '@/types/chat'

type ChatMessageLike = {
  info?: {
    providerID?: unknown
    adapterID?: unknown
    modelID?: unknown
  }
}

export type SessionSelection = {
  provider: string
  adapter: string
  model: string
  thinkingMode: string
  speedMode: string
  verbosity: string
}

function text(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

export function readSessionRunConfigSelection(runConfig: SessionRunConfig | null | undefined): SessionSelection {
  return {
    provider: text(runConfig?.providerID),
    adapter: text(runConfig?.adapterID),
    model: text(runConfig?.modelID),
    thinkingMode: text(runConfig?.thinkingMode),
    speedMode: text(runConfig?.speedMode),
    verbosity: text(runConfig?.verbosity),
  }
}

export function deriveSessionSelectionFromMessages(messages: ChatMessageLike[]): SessionSelection {
  const list = Array.isArray(messages) ? messages : []
  for (let index = list.length - 1; index >= 0; index -= 1) {
    const info = list[index]?.info
    const provider = text(info?.providerID)
    const model = text(info?.modelID)
    if (!provider || !model) continue
    return {
      provider,
      adapter: text(info?.adapterID),
      model,
      thinkingMode: '',
      speedMode: '',
      verbosity: '',
    }
  }
  return {
    provider: '',
    adapter: '',
    model: '',
    thinkingMode: '',
    speedMode: '',
    verbosity: '',
  }
}

/** A session-scoped manual model choice together with the moment it was made. */
export type SessionManualModelEntry = { at: number; value: string }

/**
 * A session id alone cannot prove which session a cached choice belongs to: a
 * recreated database starts numbering again, so an entry from an older install
 * can collide with a brand-new session. An entry written a full day before the
 * session existed therefore cannot describe it, while ordinary client/server
 * clock skew stays far below that slack.
 */
const SESSION_CACHE_SLACK_MS = 24 * 60 * 60 * 1000

/** Whether a cached session choice was written too early to belong to the session. */
export function sessionCachePredatesSession(writtenAt: number, sessionCreatedMs: number | null | undefined): boolean {
  const at = Number(writtenAt)
  if (!Number.isFinite(at) || at <= 0) return true
  const created = Number(sessionCreatedMs)
  if (!Number.isFinite(created) || created <= 0) return false
  return at + SESSION_CACHE_SLACK_MS < created
}

function readManualModelEntry(raw: string): SessionManualModelEntry | null {
  const serialized = text(raw)
  if (!serialized) return null
  let parsed: unknown
  try {
    parsed = JSON.parse(serialized)
  } catch {
    // Legacy entries were bare slugs without a timestamp. They cannot be
    // attributed to a session, so they are dropped instead of trusted.
    return null
  }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return null
  const record = parsed as { at?: unknown; value?: unknown }
  const at = Number(record.at)
  const value = text(record.value)
  if (!Number.isFinite(at) || at <= 0 || !value) return null
  return { at, value }
}

export function normalizeSessionManualModelStorageEntry(
  sessionId: string,
  value: unknown,
): { key: string; value: string } | null {
  const key = text(sessionId)
  if (!key || typeof value !== 'string') return null
  const entry = readManualModelEntry(value)
  if (!entry) return null
  const normalized = encodeModelSelectionKey(parseModelSlug(entry.value))
  if (!normalized) return null
  return { key, value: JSON.stringify({ at: Math.floor(entry.at), value: normalized }) }
}

export function readSessionManualModelEntry(
  map: Record<string, string>,
  sessionId: string,
): SessionManualModelEntry | null {
  const key = text(sessionId)
  return key ? readManualModelEntry(map[key] || '') : null
}

export function readSessionManualModelPair(
  map: Record<string, string>,
  sessionId: string,
): { provider: string; adapter: string; model: string } {
  const entry = readSessionManualModelEntry(map, sessionId)
  return entry ? parseModelSlug(entry.value) : { provider: '', adapter: '', model: '' }
}

export function writeSessionManualModelPair(
  map: Record<string, string>,
  sessionId: string,
  provider: string,
  adapter: string,
  model: string,
  writtenAt: number = Date.now(),
): Record<string, string> {
  const key = text(sessionId)
  const value = encodeModelSelectionKey({ provider, adapter, model })
  const at = Math.floor(Number(writtenAt))
  if (!key || !value || !Number.isFinite(at) || at <= 0) return map
  const entry = JSON.stringify({ at, value })
  if (map[key] === entry) return map
  return { ...map, [key]: entry }
}

export function removeSessionManualModelPair(map: Record<string, string>, sessionId: string): Record<string, string> {
  const key = text(sessionId)
  if (!key || !Object.prototype.hasOwnProperty.call(map, key)) return map
  const next = { ...map }
  delete next[key]
  return next
}
