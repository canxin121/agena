/**
 * The conversation this client used last, independent of which pane is open.
 *
 * Settings pages are not conversations: a session-scoped page must name a
 * conversation even while the focused pane shows Files or Terminal. Keeping
 * this in one small module means every surface reads the same value, and every
 * surface that opens a conversation (a sidebar click, a deep link, a bot
 * binding, a pane of a split window) writes it.
 *
 * The value is client state, so it lives in localStorage next to the other
 * remembered UI choices. It is validated as a positive integer id: an unknown
 * or deleted conversation is simply not a target, and the next open replaces it.
 *
 * Subscribers read `lastSessionId` rather than storage, because a conversation
 * opened from a background pane must update the settings pages already on
 * screen. The ref is the single in-memory copy; writes go through it.
 */
import { readonly, ref } from 'vue'

import { getLocalJson, setLocalJson } from './persist'
import { localStorageKeys } from './persistence/storageKeys'

// Pre-namespaced spelling written by earlier builds; read once so an existing
// user keeps their conversation instead of starting over.
const LEGACY_KEY = 'agena.chat.selected-session-id'

function normalizeSessionId(value: unknown): string {
  const raw = Array.isArray(value) ? value.find((item) => typeof item === 'string') : value
  const id = String(raw ?? '').trim()
  if (!/^\d+$/.test(id)) return ''
  const numeric = Number(id)
  return Number.isSafeInteger(numeric) && numeric > 0 ? id : ''
}

export function normalizeLastSessionId(value: unknown): string {
  return normalizeSessionId(value)
}

function readStored(): string {
  const stored = normalizeSessionId(getLocalJson<unknown>(localStorageKeys.chat.lastSessionId, ''))
  if (stored) return stored
  const legacy = normalizeSessionId(getLocalJson<unknown>(LEGACY_KEY, ''))
  if (!legacy) return ''
  // Adopt the legacy value under the current key so the fallback is read once.
  persist(legacy)
  return legacy
}

function persist(id: string): void {
  try {
    if (id) setLocalJson(localStorageKeys.chat.lastSessionId, id)
    else localStorage.removeItem(localStorageKeys.chat.lastSessionId)
  } catch {
    // ignore storage failures
  }
}

const current = ref(readStored())

/** The conversation this client used last, reactive to every open. */
export const lastSessionId = readonly(current)

export function readLastSessionId(): string {
  return current.value
}

export function writeLastSessionId(sessionId: unknown): void {
  const raw = sessionId == null ? '' : String(sessionId).trim()
  // Clearing is explicit; an unrecognized value must not erase a good record.
  if (raw && !normalizeSessionId(raw)) return
  const id = normalizeSessionId(raw)
  if (id === current.value) return
  current.value = id
  persist(id)
}

export function clearLastSessionId(): void {
  writeLastSessionId('')
}

/** Re-read storage into the shared copy. Used when the page resumes after the
 *  storage may have been rewritten elsewhere (another tab, a test fixture). */
export function reloadLastSessionId(): void {
  current.value = readStored()
}
