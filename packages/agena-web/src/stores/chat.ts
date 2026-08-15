import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import * as chatApi from './chat/api'
import { normalizeAgenaPart } from './chat/api'
import {
  upsertMessageEntryIn,
  upsertPart,
  binarySearchById,
} from './chat/messageIndex'
import { createSessionRunConfigPersister, loadSessionRunConfigMap } from './chat/runConfig'
import { STORAGE_RUN_CONFIG } from './chat/storeKeys'
import { ApiError } from '../lib/api'
import { setLocalJson, getLocalJson } from '../lib/persist'
import { useToastsStore } from './toasts'
import { useSettingsStore } from './settings'
import type { SseEvent } from '../lib/sse'
import type {
  AttentionEvent,
  MessageEntry,
  MessageInfo,
  MessagePart,
  Session,
  SessionError,
  SessionErrorClassification,
  SessionErrorEvent,
  SessionRunConfig,
  SessionStatus,
  SessionStatusEvent,
} from '../types/chat'
import type { JsonObject, JsonValue } from '../types/json'

// ─── constants ──────────────────────────────────────────────────────────────

const SESSION_PAGE_SIZE = 30
const MESSAGE_PAGE_SIZE = 200
const STORAGE_SELECTED_SESSION = 'agena.chat.selected-session-id.v1'

function clampText(input: JsonValue, max = 260): string {
  const raw = typeof input === 'string' ? input : ''
  const t = raw.trim()
  if (!t) return ''
  return t.length > max ? `${t.slice(0, max - 1)}…` : t
}

function safeJson(value: JsonValue): string {
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}

function isRecord(value: JsonValue): value is JsonObject {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function asRecord(value: JsonValue): JsonObject {
  return isRecord(value) ? value : {}
}

function readString(value: JsonValue): string {
  return typeof value === 'string' ? value.trim() : ''
}

function readNumber(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined
}

function firstNonEmpty(values: Array<string | null | undefined>): string {
  for (const value of values) {
    const text = typeof value === 'string' ? value.trim() : ''
    if (text) return text
  }
  return ''
}

export const useChatStore = defineStore('chat', () => {
  const toasts = useToastsStore()
  const settings = useSettingsStore()

  // ─── sessions ─────────────────────────────────────────────────────────────
  const sessions = ref<Session[]>([])
  const sessionsById = ref<Record<string, Session>>({})
  const sessionsLoading = ref(false)
  const sessionsError = ref<string | null>(null)

  // ─── selected session ─────────────────────────────────────────────────────
  const selectedSessionId = ref<string | null>(loadStoredSelectedSession())
  const messagesBySession = ref<Record<string, MessageEntry[]>>({})
  const messagesHydratedBySession = ref<Record<string, boolean>>({})
  const messages = computed<MessageEntry[]>(() => {
    const sid = selectedSessionId.value
    if (!sid) return []
    const list = messagesBySession.value[sid]
    return Array.isArray(list) ? list : []
  })
  const messagesLoading = ref(false)
  const messagesError = ref<string | null>(null)

  const historyLimitBySession = ref<Record<string, number>>({})
  const historyLoadingBySession = ref<Record<string, boolean>>({})
  const historyExhaustedBySession = ref<Record<string, boolean>>({})

  const composerDraftBySession = ref<Record<string, string>>({})
  const pendingInputText = ref('')
  const pendingInputParts = ref<JsonValue[]>([])

  const attentionBySession = ref<Record<string, AttentionEvent>>({})
  const sessionStatusBySession = ref<Record<string, SessionStatusEvent>>({})
  const sessionErrorBySession = ref<Record<string, SessionErrorEvent>>({})
  const sessionRunConfigBySession = ref<Record<string, SessionRunConfig>>({})
  sessionRunConfigBySession.value = loadSessionRunConfigMap(STORAGE_RUN_CONFIG)
  const runConfigPersister = createSessionRunConfigPersister(STORAGE_RUN_CONFIG, () => sessionRunConfigBySession.value)

  // ─── timers / inflight guards ─────────────────────────────────────────────
  let refreshTimer: number | null = null
  let refreshMessagesTimer: number | null = null
  const refreshMessagesRetryTimerBySession = new Map<string, number>()
  const refreshMessagesRequestSeqBySession = new Map<string, number>()
  const attentionRefreshTimerBySession = new Map<string, number>()
  let createSessionInFlight: Promise<Session | null> | null = null
  let lastGlobalErrorToastAt = 0
  const lastSessionErrorToastByKey = new Map<string, { at: number; message: string }>()

  function loadStoredSelectedSession(): string | null {
    try {
      const raw = getLocalJson<unknown>(STORAGE_SELECTED_SESSION, null)
      if (typeof raw === 'string' && raw.trim()) return raw.trim()
    } catch {
      // ignore
    }
    return null
  }

  function persistSelectedSession(id: string | null) {
    try {
      if (id) setLocalJson(STORAGE_SELECTED_SESSION, id)
      else localStorage.removeItem(STORAGE_SELECTED_SESSION)
    } catch {
      // ignore
    }
  }

  function pushErrorToastWithDedupe(key: string, message: string, timeoutMs = 4500, dedupeWindowMs = 1200) {
    const dedupeKey = (key || '').trim() || '__global__'
    const msg = (message || '').trim()
    if (!msg) return
    const now = Date.now()
    const prev = lastSessionErrorToastByKey.get(dedupeKey)
    if (prev && prev.message === msg && now - prev.at < Math.max(0, Math.floor(dedupeWindowMs))) return
    lastSessionErrorToastByKey.set(dedupeKey, { at: now, message: msg })
    toasts.push('error', msg, timeoutMs)
  }

  // ─── session list ─────────────────────────────────────────────────────────

  function indexSessions(list: Session[]) {
    const nextById = { ...sessionsById.value }
    for (const s of list) {
      const sid = typeof s?.id === 'string' ? s.id.trim() : ''
      if (sid) nextById[sid] = s
    }
    sessionsById.value = nextById
  }

  function upsertSessionCache(updated: (Partial<Session> & { id: string }) | null | undefined) {
    if (!updated || typeof updated !== 'object') return
    const sid = typeof updated.id === 'string' ? updated.id.trim() : ''
    if (!sid) return
    const merged = { ...(sessionsById.value[sid] || {}), ...updated, id: sid } as Session
    sessionsById.value = { ...sessionsById.value, [sid]: merged }
    const hasInCurrent = sessions.value.some((s) => s.id === sid)
    if (hasInCurrent) {
      sessions.value = sessions.value.map((s) => (s.id === sid ? { ...s, ...merged } : s))
    } else {
      sessions.value = [{ ...merged }, ...sessions.value].slice(0, SESSION_PAGE_SIZE)
    }
  }

  function scheduleSessionsRefresh(delayMs = 250) {
    const delay = Math.max(0, Math.floor(delayMs))
    if (refreshTimer) window.clearTimeout(refreshTimer)
    refreshTimer = window.setTimeout(() => {
      refreshTimer = null
      void refreshSessions()
    }, delay)
  }

  function getSessionById(sessionId: string | null | undefined): Session | null {
    const sid = (sessionId || '').trim()
    if (!sid) return null
    return sessionsById.value[sid] ?? sessions.value.find((s) => s.id === sid) ?? null
  }

  async function refreshSessions() {
    sessionsLoading.value = true
    sessionsError.value = null
    try {
      const page = await chatApi.listSessions({ limit: SESSION_PAGE_SIZE })
      const list = Array.isArray(page?.sessions) ? page.sessions : []
      sessions.value = list
      indexSessions(list)
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err)
      const authRequired =
        err instanceof ApiError && err.status === 401 && (err.code || '').trim().toLowerCase() === 'auth_required'
      sessionsError.value = null
      if (!authRequired) {
        pushErrorToastWithDedupe('sessions', msg || 'Failed to load sessions', 4500, 12_000)
      }
    } finally {
      sessionsLoading.value = false
    }
  }

  const selectedSession = computed(() => getSessionById(selectedSessionId.value))

  // The browser client has no directory concept; always null.
  const selectedSessionDirectory = computed<string | null>(() => null)

  // ─── selection ────────────────────────────────────────────────────────────

  async function selectSession(id: string | null) {
    const sid = (id || '').trim()
    const previous = selectedSessionId.value
    selectedSessionId.value = sid || null
    persistSelectedSession(sid || null)
    messagesError.value = null

    if (previous && previous !== sid) {
      if (historyLimitBySession.value[previous]) {
        const next = { ...historyLimitBySession.value }
        delete next[previous]
        historyLimitBySession.value = next
      }
      if (historyExhaustedBySession.value[previous]) {
        const next = { ...historyExhaustedBySession.value }
        delete next[previous]
        historyExhaustedBySession.value = next
      }
      if (historyLoadingBySession.value[previous]) {
        const next = { ...historyLoadingBySession.value }
        delete next[previous]
        historyLoadingBySession.value = next
      }
    }

    if (!sid) return

    if (!Array.isArray(messagesBySession.value[sid])) {
      messagesBySession.value = { ...messagesBySession.value, [sid]: [] }
    }

    if (!messagesHydratedBySession.value[sid]) {
      await refreshMessages(sid)
    } else {
      void refreshAttention(sid)
      void refreshExecutionStatus(sid)
    }
  }

  // ─── messages ─────────────────────────────────────────────────────────────

  function sessionMessageLimit(sessionId: string): number {
    const sid = (sessionId || '').trim()
    // Agena doesn't expose memory-limit UI prefs; the server owns compaction.
    const fallbackHistorical = 120
    const fallbackActive = 240
    const base = selectedSessionId.value === sid ? fallbackActive : fallbackHistorical
    const window = typeof historyLimitBySession.value[sid] === 'number' ? Number(historyLimitBySession.value[sid]) : 0
    return window > base ? window : base
  }

  function pruneSessionMessages(sessionId: string) {
    const sid = (sessionId || '').trim()
    if (!sid) return
    const list = messagesBySession.value[sid]
    if (!Array.isArray(list)) return
    const limit = sessionMessageLimit(sid)
    if (list.length > limit) {
      list.splice(0, list.length - limit)
    }
  }

  function ensureSessionMessages(sessionId: string): MessageEntry[] {
    const sid = (sessionId || '').trim()
    if (!sid) return []
    const existing = messagesBySession.value[sid]
    if (Array.isArray(existing)) return existing
    messagesBySession.value = { ...messagesBySession.value, [sid]: [] }
    return messagesBySession.value[sid] as MessageEntry[]
  }

  function setSessionMessages(sessionId: string, list: MessageEntry[]) {
    const sid = (sessionId || '').trim()
    if (!sid) return
    const existing = messagesBySession.value[sid]
    if (Array.isArray(existing)) {
      existing.splice(0, existing.length, ...list)
      return
    }
    messagesBySession.value = { ...messagesBySession.value, [sid]: list }
  }

  function markMessagesHydrated(sessionId: string) {
    const sid = (sessionId || '').trim()
    if (!sid) return
    if (messagesHydratedBySession.value[sid]) return
    messagesHydratedBySession.value = { ...messagesHydratedBySession.value, [sid]: true }
  }

  function clearMessagesHydrated(sessionId: string) {
    const sid = (sessionId || '').trim()
    if (!sid) return
    if (!Object.prototype.hasOwnProperty.call(messagesHydratedBySession.value, sid)) return
    const next = { ...messagesHydratedBySession.value }
    delete next[sid]
    messagesHydratedBySession.value = next
  }

  function normalizeMessageList(list: MessageEntry[]): MessageEntry[] {
    return [...list].sort((a, b) => {
      const ida = String(a?.info?.id ?? '')
      const idb = String(b?.info?.id ?? '')
      if (ida === idb) return 0
      return ida < idb ? -1 : 1
    })
  }

  function mergeMessageLists(older: MessageEntry[], newer: MessageEntry[]): MessageEntry[] {
    const map = new Map<string, MessageEntry>()
    for (const m of [...older, ...newer]) {
      const id = String(m?.info?.id ?? '')
      if (!id) continue
      const existing = map.get(id)
      if (existing) {
        const merged: MessageEntry = { info: { ...existing.info, ...m.info }, parts: [...existing.parts] }
        const partMap = new Map<string, MessagePart>()
        for (const p of [...merged.parts, ...(m.parts || [])]) {
          const pid = String(p?.id ?? '')
          if (pid) partMap.set(pid, p)
        }
        merged.parts = [...partMap.values()].sort((a, b) => (String(a.id) < String(b.id) ? -1 : 1))
        map.set(id, merged)
      } else {
        map.set(id, m)
      }
    }
    return normalizeMessageList([...map.values()])
  }

  function nextRefreshMessagesRequestSeq(sessionId: string): number {
    const sid = (sessionId || '').trim()
    if (!sid) return 0
    const next = (refreshMessagesRequestSeqBySession.get(sid) || 0) + 1
    refreshMessagesRequestSeqBySession.set(sid, next)
    return next
  }

  function isLatestRefreshMessagesRequest(sessionId: string, requestSeq: number): boolean {
    const sid = (sessionId || '').trim()
    if (!sid || requestSeq <= 0) return false
    return refreshMessagesRequestSeqBySession.get(sid) === requestSeq
  }

  function clearMessageRefreshRetry(sessionId: string) {
    const sid = (sessionId || '').trim()
    if (!sid) return
    const timer = refreshMessagesRetryTimerBySession.get(sid)
    if (typeof timer === 'number') {
      window.clearTimeout(timer)
      refreshMessagesRetryTimerBySession.delete(sid)
    }
  }

  function scheduleMessageRefreshRetry(sessionId: string, delayMs: number) {
    const sid = (sessionId || '').trim()
    if (!sid) return
    if (refreshMessagesRetryTimerBySession.has(sid)) return
    const delay = Math.max(60, Math.min(10_000, Math.floor(delayMs || 180)))
    const timer = window.setTimeout(() => {
      refreshMessagesRetryTimerBySession.delete(sid)
      void refreshMessages(sid, { silent: true }).catch(() => {})
    }, delay)
    refreshMessagesRetryTimerBySession.set(sid, timer)
  }

  function upsertSessionRunConfig(sessionId: string, patch: Partial<SessionRunConfig>) {
    const sid = (sessionId || '').trim()
    if (!sid) return
    const now = Date.now()
    const prev = sessionRunConfigBySession.value[sid] || { at: 0 }
    const next: SessionRunConfig = {
      ...prev,
      ...patch,
      at: Math.max(prev.at || 0, now),
    }
    sessionRunConfigBySession.value = { ...sessionRunConfigBySession.value, [sid]: next }
    runConfigPersister.persistSoon()
  }

  function extractRunConfigFromMessageInfo(info: MessageInfo | null | undefined): Partial<SessionRunConfig> {
    if (!info) return {}
    const out: Partial<SessionRunConfig> = {}
    const providerID = readString(info.providerID as JsonValue) || readString(info.provider_id as JsonValue)
    const modelID = readString(info.modelID as JsonValue) || readString(info.model_id as JsonValue)
    const agent = readString(info.agent as JsonValue)
    const variant = readString(info.variant as JsonValue)
    if (providerID) out.providerID = providerID
    if (modelID) out.modelID = modelID
    if (agent) out.agent = agent
    if (variant) out.variant = variant
    return out
  }

  async function refreshMessages(sessionId: string, opts?: { silent?: boolean }) {
    const sid = (sessionId || '').trim()
    if (!sid) return

    const requestSeq = nextRefreshMessagesRequestSeq(sid)
    const isSelected = selectedSessionId.value === sid
    const hasCache = (messagesBySession.value[sid]?.length ?? 0) > 0
    const silent = Boolean(opts?.silent)

    if (!silent && isSelected && !hasCache) messagesLoading.value = true
    if (isSelected) messagesError.value = null

    try {
      const limit = sessionMessageLimit(sid)
      const page = await chatApi.listMessages(sid, limit)
      if (!isLatestRefreshMessagesRequest(sid, requestSeq)) return
      const ordered = normalizeMessageList(page.entries)
      setSessionMessages(sid, ordered)
      pruneSessionMessages(sid)
      markMessagesHydrated(sid)
      historyExhaustedBySession.value = { ...historyExhaustedBySession.value, [sid]: true }

      // Capture run config from the last message that carries provider/model.
      for (let i = ordered.length - 1; i >= 0; i -= 1) {
        const patch = extractRunConfigFromMessageInfo(ordered[i]?.info)
        if (patch.providerID || patch.modelID || patch.agent) {
          upsertSessionRunConfig(sid, patch)
          break
        }
      }

      // Rehydrate status + attention after load.
      void refreshExecutionStatus(sid)
      void refreshAttention(sid)
    } catch (err) {
      if (!isLatestRefreshMessagesRequest(sid, requestSeq)) return
      const msg = err instanceof Error ? err.message : String(err)
      const authRequired =
        err instanceof ApiError && err.status === 401 && (err.code || '').trim().toLowerCase() === 'auth_required'
      if (isSelected) {
        messagesError.value = null
        if (authRequired) {
          clearMessageRefreshRetry(sid)
        } else {
          pushErrorToastWithDedupe(`messages:${sid}`, msg || 'Failed to load messages', silent ? 3500 : 4500, 8000)
        }
      }
      if (!authRequired && hasCache) {
        scheduleMessageRefreshRetry(sid, 240)
      }
      if (!hasCache && !silent) {
        setSessionMessages(sid, [])
      }
    } finally {
      if (!silent && isSelected) messagesLoading.value = false
    }
  }

  const selectedHistory = computed(() => {
    const sid = selectedSessionId.value
    if (!sid) return { loading: false, exhausted: false, limit: 0 }
    return {
      loading: Boolean(historyLoadingBySession.value[sid]),
      exhausted: Boolean(historyExhaustedBySession.value[sid]),
      limit: typeof historyLimitBySession.value[sid] === 'number' ? Number(historyLimitBySession.value[sid]) : 0,
    }
  })

  async function loadOlderMessages(sessionId: string) {
    const sid = (sessionId || '').trim()
    if (!sid) return false
    if (historyLoadingBySession.value[sid]) return false
    if (historyExhaustedBySession.value[sid]) return false

    const current = ensureSessionMessages(sid)
    const currentLen = current.length
    const maxWindow = 2000
    const remaining = Math.max(0, maxWindow - currentLen)
    const pageSize = Math.min(MESSAGE_PAGE_SIZE, remaining)
    if (pageSize <= 0) {
      historyExhaustedBySession.value = { ...historyExhaustedBySession.value, [sid]: true }
      return false
    }

    historyLoadingBySession.value = { ...historyLoadingBySession.value, [sid]: true }
    try {
      const nextLimit = currentLen + pageSize
      const page = await chatApi.listMessages(sid, nextLimit)
      const normalized = normalizeMessageList(page.entries)
      const merged = mergeMessageLists(normalized, current)
      setSessionMessages(sid, merged)
      historyLimitBySession.value = { ...historyLimitBySession.value, [sid]: merged.length }
      historyExhaustedBySession.value = { ...historyExhaustedBySession.value, [sid]: normalized.length < nextLimit }
      return normalized.length < nextLimit ? false : true
    } finally {
      historyLoadingBySession.value = { ...historyLoadingBySession.value, [sid]: false }
    }
  }

  // ─── execution status + attention ─────────────────────────────────────────

  function scheduleAttentionRefresh(sessionId: string, delayMs = 150) {
    const sid = (sessionId || '').trim()
    if (!sid) return
    if (attentionRefreshTimerBySession.has(sid)) return
    const timer = window.setTimeout(() => {
      attentionRefreshTimerBySession.delete(sid)
      void refreshAttention(sid)
      void refreshExecutionStatus(sid)
    }, Math.max(60, Math.min(1200, Math.floor(delayMs))))
    attentionRefreshTimerBySession.set(sid, timer)
  }

  function statusFromState(state: string, workflow: string, running: boolean): SessionStatus {
    if (running || state === 'running' || state === 'creating' || workflow === 'tool_pending' || workflow === 'blocked') {
      return { type: 'busy' }
    }
    return { type: 'idle' }
  }

  async function refreshExecutionStatus(sessionId: string) {
    const sid = (sessionId || '').trim()
    if (!sid) return
    const st = await chatApi.getSessionExecutionStatus(sid).catch(() => null)
    if (!st) return
    const parsed = statusFromState(st.state, st.workflow_state, st.running)
    sessionStatusBySession.value = {
      ...sessionStatusBySession.value,
      [sid]: {
        at: Date.now(),
        payload: { type: 'runtime_signal', properties: { state: st.state, workflow_state: st.workflow_state } },
        status: parsed,
      },
    }
  }

  /** Translate agena pending_interactive_requests into the AttentionEvent shape. */
  function attentionFromPendingRequests(sessionId: string, requests: JsonValue[]): AttentionEvent | null {
    const sid = (sessionId || '').trim()
    if (!sid || !Array.isArray(requests) || requests.length === 0) return null
    for (const raw of requests) {
      const rec = asRecord(raw)
      const kind = readString(rec.kind as JsonValue)
      const requestId = readString(rec.request_id as JsonValue)
      if (!requestId) continue
      if (kind === 'user_input') {
        const title = readString(rec.title as JsonValue) || 'Question'
        const body = readString(rec.body_markdown as JsonValue)
        const rawQuestions = rec.questions
        const questions = Array.isArray(rawQuestions)
          ? rawQuestions
              .map((q) => {
                const qr = asRecord(q)
                const header = readString(qr.header as JsonValue) || title
                const question = readString(qr.question as JsonValue) || body
                const options = Array.isArray(qr.options)
                  ? qr.options
                      .map((o) => {
                        const or = asRecord(o)
                        return {
                          label: readString(or.label as JsonValue),
                          description: readString(or.description as JsonValue),
                        }
                      })
                      .filter((o) => Boolean(o.label))
                  : []
                const multiple = qr.multiple === true
                const custom = qr.allow_custom === true
                return { header, question, options, multiple, custom }
              })
              .filter((q) => Boolean(q.question && q.header))
          : []
        if (questions.length === 0) continue
        return {
          kind: 'question',
          at: Date.now(),
          payload: {
            type: 'question.asked',
            properties: {
              id: requestId,
              questions,
            },
          },
        }
      }
      // permission
      const action = asRecord(rec.action as JsonValue)
      const toolName = readString(action.tool_name as JsonValue)
      const actionKind = readString(action.kind as JsonValue)
      const targetPath = readString(action.target_path as JsonValue)
      const target = readString(action.target as JsonValue)
      const reason = readString(rec.reason as JsonValue)
      const permission = toolName || actionKind || target || reason || 'permission'
      const patterns: string[] = []
      if (targetPath) patterns.push(targetPath)
      else if (target) patterns.push(target)
      return {
        kind: 'permission',
        at: Date.now(),
        payload: {
          type: 'permission.asked',
          properties: {
            id: requestId,
            permission,
            patterns,
            always: [],
          },
        },
      }
    }
    return null
  }

  async function refreshAttention(sessionId: string) {
    const sid = (sessionId || '').trim()
    if (!sid) return
    const state = await chatApi.getSessionExecution(sid).catch(() => null)
    if (!state) return
    const requests = state.pending_interactive_requests as JsonValue[] | undefined
    const next = attentionFromPendingRequests(sid, Array.isArray(requests) ? requests : [])
    if (next) {
      attentionBySession.value = { ...attentionBySession.value, [sid]: next }
    } else if (attentionBySession.value[sid]) {
      const nextMap = { ...attentionBySession.value }
      delete nextMap[sid]
      attentionBySession.value = nextMap
    }
  }

  function clearAttention(sessionId: string) {
    const sid = (sessionId || '').trim()
    if (!sid) return
    if (!attentionBySession.value[sid]) return
    const next = { ...attentionBySession.value }
    delete next[sid]
    attentionBySession.value = next
  }

  const selectedAttention = computed(() => {
    const sid = selectedSessionId.value
    if (!sid) return null
    return attentionBySession.value[sid] ?? null
  })

  const selectedSessionStatus = computed(() => {
    const sid = selectedSessionId.value
    if (!sid) return null
    return sessionStatusBySession.value[sid] ?? null
  })

  const selectedSessionError = computed(() => {
    const sid = selectedSessionId.value
    if (!sid) return null
    return sessionErrorBySession.value[sid] ?? null
  })

  const selectedSessionRunConfig = computed(() => {
    const sid = selectedSessionId.value
    if (!sid) return null
    return sessionRunConfigBySession.value[sid] ?? null
  })

  function clearSessionError(sessionId: string) {
    const sid = (sessionId || '').trim()
    if (!sid) return
    if (!Object.prototype.hasOwnProperty.call(sessionErrorBySession.value, sid)) return
    const next = { ...sessionErrorBySession.value }
    delete next[sid]
    sessionErrorBySession.value = next
  }

  // ─── session CRUD ─────────────────────────────────────────────────────────

  async function createSession(): Promise<Session | null> {
    if (createSessionInFlight) return await createSessionInFlight
    createSessionInFlight = (async () => {
      try {
        const created = await chatApi.createSession()
        upsertSessionCache(created)
        scheduleSessionsRefresh(1200)
        if (created?.id) {
          await selectSession(created.id)
        }
        return created
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err)
        pushErrorToastWithDedupe('create-session', msg || 'Failed to create session', 4500, 2000)
        return null
      } finally {
        createSessionInFlight = null
      }
    })()
    return await createSessionInFlight
  }

  async function deleteSession(sessionId: string) {
    const sid = (sessionId || '').trim()
    if (!sid) return

    clearMessageRefreshRetry(sid)
    refreshMessagesRequestSeqBySession.delete(sid)

    await chatApi.deleteSession(sid)

    if (selectedSessionId.value === sid) {
      selectedSessionId.value = null
      persistSelectedSession(null)
    }

    clearAttention(sid)

    {
      const nextById = { ...sessionsById.value }
      delete nextById[sid]
      sessionsById.value = nextById
    }
    {
      const next = { ...sessionStatusBySession.value }
      delete next[sid]
      sessionStatusBySession.value = next
    }
    {
      const next = { ...sessionErrorBySession.value }
      delete next[sid]
      sessionErrorBySession.value = next
    }
    {
      const next = { ...sessionRunConfigBySession.value }
      delete next[sid]
      sessionRunConfigBySession.value = next
      runConfigPersister.persistSoon()
    }
    clearMessagesHydrated(sid)
    clearComposerDraft(sid)
    {
      const next = { ...messagesBySession.value }
      delete next[sid]
      messagesBySession.value = next
    }
    {
      const next = { ...historyLimitBySession.value }
      delete next[sid]
      historyLimitBySession.value = next
    }
    {
      const next = { ...historyLoadingBySession.value }
      delete next[sid]
      historyLoadingBySession.value = next
    }
    {
      const next = { ...historyExhaustedBySession.value }
      delete next[sid]
      historyExhaustedBySession.value = next
    }

    sessions.value = sessions.value.filter((s) => s?.id !== sid)
    scheduleSessionsRefresh(1200)
  }

  async function renameSession(sessionId: string, title: string) {
    const sid = (sessionId || '').trim()
    const trimmed = (title || '').trim()
    if (!sid || !trimmed) return null
    const updated = await chatApi.patchSessionTitle(sid, trimmed)
    upsertSessionCache(updated)
    scheduleSessionsRefresh(1200)
    return updated
  }

  // ─── send / abort ─────────────────────────────────────────────────────────

  function buildDocument(opts: { text?: string; parts?: JsonValue[] }): JsonValue[] {
    const trimmed = (opts.text || '').trim()
    const providedParts = Array.isArray(opts.parts) ? opts.parts : []
    const document: JsonValue[] = []
    for (const p of providedParts) {
      const rec = asRecord(p)
      const ty = readString(rec.type as JsonValue)
      if (ty === 'text' || ty === 'activity') document.push(p)
      else if (typeof p === 'string') document.push({ type: 'text', text: p })
    }
    if (trimmed) {
      const hasText = document.some((d) => {
        const rec = asRecord(d)
        return readString(rec.type as JsonValue) === 'text'
      })
      if (!hasText) document.push({ type: 'text', text: trimmed })
    }
    return document
  }

  function buildRunOptions(opts: { providerID?: string; modelID?: string }): JsonValue {
    const providerID = (opts.providerID || '').trim()
    const modelID = (opts.modelID || '').trim()
    const options: JsonObject = {}
    if (providerID && modelID) {
      const model: JsonObject = { provider_id: providerID, model_id: modelID }
      options.model = model
    }
    return options
  }

  async function sendMessage(
    sessionId: string,
    opts: { text?: string; parts?: JsonValue[]; providerID?: string; modelID?: string; agent?: string; variant?: string },
  ) {
    const sid = (sessionId || '').trim()
    const document = buildDocument(opts)
    if (document.length === 0) return { queued: false }
    clearSessionError(sid)
    await chatApi.sendMessage(sid, { document, options: buildRunOptions(opts) })
    // Let SSE stream parts in; also nudge status to busy.
    void refreshExecutionStatus(sid)
    scheduleAttentionRefresh(sid, 200)
    return { queued: true }
  }

  async function sendText(sessionId: string, text: string) {
    await sendMessage(sessionId, { text })
  }

  async function abortSession(sessionId: string) {
    const sid = (sessionId || '').trim()
    if (!sid) return false
    try {
      const st = await chatApi.getSessionExecutionStatus(sid).catch(() => null)
      const executionId = st?.active_execution?.execution_id
      if (!executionId) {
        // Nothing active; just clear UI state.
        clearAttention(sid)
        sessionStatusBySession.value = {
          ...sessionStatusBySession.value,
          [sid]: { at: Date.now(), payload: { type: 'session.status' }, status: { type: 'idle' } },
        }
        return true
      }
      await chatApi.cancelSession(sid, executionId)
      clearAttention(sid)
      sessionStatusBySession.value = {
        ...sessionStatusBySession.value,
        [sid]: { at: Date.now(), payload: { type: 'session.status' }, status: { type: 'idle' } },
      }
      return true
    } catch {
      return false
    }
  }

  // ─── interactive replies ──────────────────────────────────────────────────

  async function replyPermission(sessionId: string, requestId: string, reply: 'once' | 'always' | 'reject', message?: string) {
    const ok = await chatApi.replyPermission(sessionId, requestId, reply, message)
    if (ok) clearAttention((sessionId || '').trim())
    return ok
  }

  async function replyQuestion(sessionId: string, requestId: string, answers: string[][]) {
    const questions = ensureSessionMessages(sessionId)
    void questions
    const answersMap: Record<string, string[]> = {}
    const state = await chatApi.getSessionExecution(sessionId).catch(() => null)
    const requests = state?.pending_interactive_requests as JsonValue[] | undefined
    const request = (Array.isArray(requests) ? requests : []).find((r) => {
      const rec = asRecord(r)
      return readString(rec.request_id as JsonValue) === requestId
    })
    const rawQuestions = request ? asRecord(request).questions : null
    if (Array.isArray(rawQuestions)) {
      rawQuestions.forEach((q, index) => {
        const qr = asRecord(q)
        const qid = readString(qr.question_id as JsonValue) || String(index)
        answersMap[qid] = Array.isArray(answers[index]) ? answers[index] : []
      })
    } else {
      answers.forEach((a, index) => {
        answersMap[String(index)] = Array.isArray(a) ? a : []
      })
    }
    const ok = await chatApi.replyQuestion(sessionId, requestId, answersMap)
    if (ok) clearAttention((sessionId || '').trim())
    return ok
  }

  async function rejectQuestion(sessionId: string, requestId: string) {
    const ok = await chatApi.rejectQuestion(sessionId, requestId)
    if (ok) clearAttention((sessionId || '').trim())
    return ok
  }

  // ─── compact ──────────────────────────────────────────────────────────────

  async function compactSession(sessionId: string) {
    const sid = (sessionId || '').trim()
    if (!sid) return null
    await chatApi.compactSession(sid, { options: buildRunOptions({}) })
    scheduleAttentionRefresh(sid, 200)
    scheduleSessionsRefresh(1200)
  }

  // ─── fork (sharing equivalent) ────────────────────────────────────────────

  async function forkSession(sessionId: string, opts?: { at_message_id?: number }) {
    const sid = (sessionId || '').trim()
    if (!sid) return null
    const atMessageId =
      typeof opts?.at_message_id === 'number' && Number.isFinite(opts.at_message_id) ? opts.at_message_id : undefined
    const created = await chatApi.forkSession(sid, atMessageId != null ? { at_message_id: atMessageId } : undefined)
    upsertSessionCache(created)
    scheduleSessionsRefresh(1200)
    return created
  }

  // ─── rewind (revert) ──────────────────────────────────────────────────────

  async function revertToMessage(sessionId: string, messageId: string) {
    const sid = (sessionId || '').trim()
    const mid = (messageId || '').trim()
    if (!sid || !mid) return

    // Stop any active run first.
    const st = sessionStatusBySession.value[sid]?.status?.type
    if (st && st !== 'idle') {
      await abortSession(sid)
    }

    // The message id is the run marker id; the turn id lives on the message.
    const list = messagesBySession.value[sid] ?? []
    const target = list.find((m) => String(m?.info?.id || '') === mid) ?? null
    const turnIdRaw = target?.info?.turnId
    const turnId = typeof turnIdRaw === 'string' ? turnIdRaw.trim() : ''
    if (!turnId) return

    await chatApi.rewindSession(sid, turnId)
    clearMessagesHydrated(sid)
    // Reload the timeline (server removed later parts).
    await refreshMessages(sid, { silent: true })
    scheduleSessionsRefresh(1200)
  }

  // ─── composer helpers ─────────────────────────────────────────────────────

  function setPendingComposer(text: string, parts?: JsonValue[]) {
    pendingInputText.value = (text || '').trim()
    pendingInputParts.value = Array.isArray(parts) ? parts : []
  }

  function consumePendingComposer(): { text: string; parts: JsonValue[] } {
    const value = { text: pendingInputText.value, parts: pendingInputParts.value }
    pendingInputText.value = ''
    pendingInputParts.value = []
    return value
  }

  function getComposerDraft(sessionId: string): string {
    const sid = (sessionId || '').trim()
    if (!sid) return ''
    const v = composerDraftBySession.value[sid]
    return typeof v === 'string' ? v : ''
  }

  function setComposerDraft(sessionId: string, text: string) {
    const sid = (sessionId || '').trim()
    if (!sid) return
    composerDraftBySession.value[sid] = String(text ?? '')
  }

  function clearComposerDraft(sessionId: string) {
    const sid = (sessionId || '').trim()
    if (!sid) return
    if (Object.prototype.hasOwnProperty.call(composerDraftBySession.value, sid)) {
      const next = { ...composerDraftBySession.value }
      delete next[sid]
      composerDraftBySession.value = next
    }
  }

  /** Agena has no lazy part-detail endpoint; parts ship complete. No-op kept for UI compat. */
  async function ensureMessagePartDetail(_part: JsonValue): Promise<void> {
    return undefined
  }

  function cacheSessions(entries: Array<(Partial<Session> & { id: string }) | null | undefined>) {
    for (const entry of entries) {
      upsertSessionCache(entry)
    }
  }

  // ─── SSE applyEvent ───────────────────────────────────────────────────────

  /**
   * Consume an agena notification event (delivered by lib/sse connectSse).
   * The envelope has been normalized to `type` + `properties`:
   *
   *   session_changed  properties.kind = part_added|part_updated|part_removed|session_meta_updated
   *   runtime_signal   properties.kind / session_id / payload
   *   lagged           → full resync (handled by useAppRuntime onEvent too)
   *
   * Mapping (opencode SSE → agena):
   *   message.part.created / .updated   → part_added / part_updated
   *   message.part.removed / removed    → part_removed
   *   session.updated                   → session_meta_updated
   *   session.status / session.idle     → runtime_signal (session execution status)
   *   permission.asked / question.asked → pending_interactive_requests (via state refresh)
   */
  function applyEvent(evt: SseEvent) {
    const t = evt.type || ''
    if (!t) return
    const props = asRecord(evt.properties)
    const changeKind = readString(props.kind as JsonValue)
    const sessionIdRaw = readNumber(props.session_id)
    const sid = sessionIdRaw != null ? String(sessionIdRaw) : readString(props.sessionId as JsonValue)

    if (t === 'session_changed') {
      if (sid && (changeKind === 'part_added' || changeKind === 'part_updated')) {
        const part = props.part
        if (isRecord(part)) {
          const partId = readNumber(part.part_id)
          if (partId != null) {
            const runId = readNumber(part.run_id)
            const key = runId != null ? String(runId) : String(partId)
            const list = ensureSessionMessages(sid)
            const kind = readString(part.kind as JsonValue)

            if (kind === 'run') {
              // New message (turn marker).
              const info: MessageInfo = {
                id: String(partId),
                sessionID: sid,
                role: readString(part.role as JsonValue) || 'assistant',
                runId: partId,
                time: { created: readNumber(part.created_at_ms) ?? Date.now() },
              }
              const content = asRecord(part.content)
              const providerID = readString(content.provider_id as JsonValue)
              const modelID = readString(content.model_id as JsonValue)
              const turnId = readString(content.turn_id as JsonValue)
              if (providerID) info.providerID = providerID
              if (modelID) info.modelID = modelID
              if (turnId) info.turnId = turnId
              upsertMessageEntryIn(list, info)
              pruneSessionMessages(sid)
              scheduleSessionsRefresh(800)
            } else {
              const existing = binarySearchById(list, key, (m) => m.info.id)
              if (existing.found && list[existing.index]) {
                const partOut = normalizeAgenaPart(String(partId), sid, key, part as JsonValue)
                if (partOut) {
                  upsertPart(list[existing.index], partOut, '')
                  pruneSessionMessages(sid)
                }
              } else {
                // Orphan content part before any run marker: create a message.
                const info: MessageInfo = {
                  id: key,
                  sessionID: sid,
                  role: readString(part.role as JsonValue) || 'assistant',
                  runId,
                  time: { created: readNumber(part.created_at_ms) ?? Date.now() },
                }
                const entry = upsertMessageEntryIn(list, info)
                const partOut = normalizeAgenaPart(String(partId), sid, key, part as JsonValue)
                if (partOut) entry.parts.push(partOut)
              }
            }
            // Any part change can affect status/attention.
            scheduleAttentionRefresh(sid)
          }
        }
      } else if (sid && changeKind === 'part_removed') {
        const removedPartId = readNumber(props.part_id)
        if (removedPartId != null) {
          const list = messagesBySession.value[sid]
          if (Array.isArray(list)) {
            const idx = list.findIndex((m) => Number(m.info.runId) === removedPartId)
            if (idx >= 0) {
              list.splice(idx, 1)
              return
            }
            // Remove a part within a message.
            for (const entry of list) {
              const pidx = (entry.parts || []).findIndex((p) => String(p.id) === String(removedPartId))
              if (pidx >= 0) {
                entry.parts.splice(pidx, 1)
                return
              }
            }
          }
        }
      } else if (sid && changeKind === 'session_meta_updated') {
        const title = readString(props.title as JsonValue)
        const updatedAtMs = readNumber(props.updated_at_ms)
        const current = getSessionById(sid)
        if (title || current) {
          upsertSessionCache({
            id: sid,
            ...(title ? { title } : {}),
            ...(typeof updatedAtMs === 'number' ? { updated_at_ms: updatedAtMs } : {}),
          })
        }
        scheduleSessionsRefresh(600)
      }
      return
    }

    if (t === 'runtime_signal') {
      const signalSession = sid || (props.session_id != null ? String(props.session_id) : '')
      if (signalSession) {
        scheduleAttentionRefresh(signalSession)
        void refreshExecutionStatus(signalSession)
      }
      const signalKind = readString(props.kind as JsonValue)
      if (signalKind === 'activity' || signalKind === 'plugin') {
        scheduleSessionsRefresh(600)
      }
      return
    }

    if (t === 'lagged') {
      const sig = selectedSessionId.value
      if (sig) {
        void refreshMessages(sig, { silent: true })
      }
      void refreshSessions()
      return
    }

    // session.error normalized to an agena failure envelope (problem.user.fallback).
    if (t === 'session.error') {
      const msg = firstNonEmpty([readString(props.message as JsonValue), readString(props.fallback as JsonValue)])
      if (msg && sid) {
        const at = Date.now()
        sessionErrorBySession.value = {
          ...sessionErrorBySession.value,
          [sid]: {
            at,
            payload: evt,
            error: { message: msg, rendered: msg, raw: props },
          },
        }
        sessionStatusBySession.value = {
          ...sessionStatusBySession.value,
          [sid]: { at, payload: evt, status: { type: 'idle' } },
        }
        clearAttention(sid)
      }
    }
  }

  return {
    sessions,
    sessionsLoading,
    sessionsError,
    selectedSessionId,
    selectedSession,
    selectedSessionDirectory,
    messages,
    messagesLoading,
    messagesError,
    selectedAttention,
    selectedSessionStatus,
    selectedSessionError,
    selectedSessionRunConfig,
    sessionStatusBySession,
    sessionErrorBySession,
    sessionRunConfigBySession,
    attentionBySession,
    refreshSessions,
    selectSession,
    refreshMessages,
    loadOlderMessages,
    selectedHistory,
    createSession,
    deleteSession,
    renameSession,
    abortSession,
    sendText,
    sendMessage,
    compactSession,
    forkSession,
    replyPermission,
    replyQuestion,
    rejectQuestion,
    getSessionById,
    cacheSessions,
    ensureMessagePartDetail,
    consumePendingComposer,
    getComposerDraft,
    setComposerDraft,
    clearSessionError,
    revertToMessage,
    applyEvent,
  }
})
