import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import * as chatApi from './chat/api'
import { messageErrorFromAgenaPart, normalizeAgenaPart } from './chat/api'
import { binarySearchById, compareChatIds, upsertMessageEntryIn, upsertPart } from './chat/messageIndex'
import { createSessionRunConfigPersister, loadSessionRunConfigMap } from './chat/runConfig'
import { STORAGE_RUN_CONFIG } from './chat/storeKeys'
import { ApiError } from '../lib/api'
import { isRunTerminal } from '../lib/chatRunState'
import { setLocalJson, getLocalJson } from '../lib/persist'
import { localStorageKeys } from '../lib/persistence/storageKeys'
import { useToastsStore } from './toasts'
import { useDirectoryStore } from './directory'
import { i18n } from '../i18n'
import { sessionStateExecution, sessionStateRequests } from '../types/chat'
import type { SseEvent } from '../lib/sse'
import type {
  AttentionEvent,
  MessageEntry,
  MessageFold,
  MessageInfo,
  MessagePart,
  Session,
  SessionErrorEvent,
  SessionRunConfig,
  SessionState,
  SessionUsage,
} from '../types/chat'
import type { JsonObject, JsonValue } from '../types/json'
import { readSessionIdFromQuery } from '@/app/navigation/sessionQuery'
import { useWorkspacePaneContext, type WorkspacePaneContext } from '@/app/workspace/workspacePaneContext'
import { DEFAULT_TRANSCRIPT_PART_PAGE_SIZE, normalizeTranscriptPartPageSize } from '@/pages/chat/transcriptPartPaging'

// ─── constants ──────────────────────────────────────────────────────────────

const SESSION_PAGE_SIZE = 30
// Keep session entry cheap. The transcript endpoint interprets this as a
// two same-role pagination groups: one user-side group and one assistant-side
// group. The server keeps consecutive runs of one role together so a burst of
// user sends or assistant continuations is never split at a page boundary.
const MESSAGE_PAGE_SIZE = 2
const STORAGE_SELECTED_SESSION = 'agena.chat.selected-session-id.v1'

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

function loadStoredTranscriptPartPageSize(): number {
  return normalizeTranscriptPartPageSize(
    getLocalJson<unknown>(localStorageKeys.chat.transcriptPartPageSize, DEFAULT_TRANSCRIPT_PART_PAGE_SIZE),
  )
}

const useChatStoreDefinition = defineStore('chat', () => {
  const toasts = useToastsStore()
  const directoryStore = useDirectoryStore()

  // ─── sessions ─────────────────────────────────────────────────────────────
  const sessions = ref<Session[]>([])
  const sessionsById = ref<Record<string, Session>>({})
  const sessionDirectoryBySessionId = ref<Record<string, string>>({})
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

  // Number of older parts revealed by one explicit user action. The initial
  // visible tail is intentionally fixed at five and does not change when this
  // input changes.
  const transcriptPartPageSize = ref(loadStoredTranscriptPartPageSize())

  const historyLimitBySession = ref<Record<string, number>>({})
  const historyUserMessageCountBySession = ref<Record<string, number | null>>({})
  const historyLoadingBySession = ref<Record<string, boolean>>({})
  const historyExhaustedBySession = ref<Record<string, boolean>>({})
  const historyCursorBySession = ref<Record<string, string | null>>({})
  // Do not infer this from the number of MessageEntry objects: an older raw
  // page may belong entirely to an already-loaded, folded assistant reply and
  // therefore add parts without adding another message entry.
  const historyOlderLoadedBySession = ref<Record<string, boolean>>({})

  const composerDraftBySession = ref<Record<string, string>>({})
  const pendingInputText = ref('')
  const pendingInputParts = ref<JsonValue[]>([])

  const attentionBySession = ref<Record<string, AttentionEvent>>({})
  const sessionErrorBySession = ref<Record<string, SessionErrorEvent>>({})
  const sessionRunConfigBySession = ref<Record<string, SessionRunConfig>>({})
  const sessionUsageBySession = ref<Record<string, SessionUsage>>({})
  // Keep the server execution projection separate from the compact run
  // config.  The composer status line needs agent/task/permission/activity
  // fields that are not part of the model picker configuration.
  const sessionExecutionBySession = ref<Record<string, NonNullable<chatApi.AgenaExecutionState['execution']>>>({})
  sessionRunConfigBySession.value = loadSessionRunConfigMap(STORAGE_RUN_CONFIG)
  const runConfigPersister = createSessionRunConfigPersister(STORAGE_RUN_CONFIG, () => sessionRunConfigBySession.value)

  // ─── timers / inflight guards ─────────────────────────────────────────────
  let refreshTimer: number | null = null
  let refreshSessionsInFlight: Promise<void> | null = null
  const getSessionInFlightById = new Map<string, Promise<Session>>()
  const refreshMessagesRetryTimerBySession = new Map<string, number>()
  const refreshMessagesRequestSeqBySession = new Map<string, number>()
  const refreshMessagesInFlightBySession = new Map<string, Promise<void>>()
  const refreshExecutionInFlightBySession = new Map<string, Promise<void>>()
  const refreshAttentionInFlightBySession = new Map<string, Promise<void>>()
  const presentedInteractiveRequestBySession = new Map<string, string>()
  // Incremented when an explicit transcript cache reset happens. In-flight
  // requests capture this generation so a late response cannot repopulate a
  // cache that has already been reset.
  let transcriptCacheGeneration = 0
  const statusRefreshTimerBySession = new Map<string, number>()
  let createSessionInFlight: Promise<Session | null> | null = null
  const workspaceRequestById = new Map<number, Promise<{ id: number; path: string } | null>>()
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

  async function refreshSessionsInternal() {
    sessionsLoading.value = true
    sessionsError.value = null
    try {
      const page = await chatApi.listSessions({ limit: SESSION_PAGE_SIZE, excludeSubagents: true })
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

  async function refreshSessions() {
    if (refreshSessionsInFlight) return refreshSessionsInFlight
    const request = refreshSessionsInternal()
    refreshSessionsInFlight = request
    try {
      await request
    } finally {
      if (refreshSessionsInFlight === request) refreshSessionsInFlight = null
    }
  }

  const selectedSession = computed(() => getSessionById(selectedSessionId.value))

  const selectedSessionState = computed<SessionState>(() => {
    const state = selectedSession.value?.state
    return state || { kind: 'ready', data: {} }
  })

  const selectedSessionDirectory = computed<string | null>(() => {
    const sid = selectedSessionId.value
    if (!sid) return null
    return sessionDirectoryBySessionId.value[sid] || null
  })

  async function workspaceForSession(session: Session): Promise<{ id: number; path: string } | null> {
    const sessionId = String(session.id || '').trim()
    const workspaceId = Number(session.workspace_id)
    if (!sessionId || !Number.isSafeInteger(workspaceId) || workspaceId <= 0) return null

    let request = workspaceRequestById.get(workspaceId)
    if (!request) {
      request = chatApi
        .getWorkspace(workspaceId)
        .then((workspace) => workspace)
        .catch(() => null)
        .finally(() => workspaceRequestById.delete(workspaceId))
      workspaceRequestById.set(workspaceId, request)
    }
    const workspace = await request
    if (!workspace?.path) return null
    sessionDirectoryBySessionId.value = {
      ...sessionDirectoryBySessionId.value,
      [sessionId]: workspace.path,
    }
    return workspace
  }

  async function hydrateSessionDirectory(session: Session, windowId?: string | null): Promise<void> {
    const workspace = await workspaceForSession(session)
    if (!workspace) return
    const targetWindowId = String(windowId || '').trim()
    if (targetWindowId) {
      directoryStore.setDirectoryForWindow(targetWindowId, workspace.path)
      return
    }
    if (selectedSessionId.value === session.id) directoryStore.setDirectory(workspace.path)
  }

  async function hydrateSession(id: string | null, opts?: { windowId?: string | null }) {
    const sid = String(id || '').trim()
    if (!sid) return

    let session = getSessionById(sid)
    if (!session) {
      let request = getSessionInFlightById.get(sid)
      if (!request) {
        request = chatApi.getSession(sid).finally(() => {
          if (getSessionInFlightById.get(sid) === request) getSessionInFlightById.delete(sid)
        })
        getSessionInFlightById.set(sid, request)
      }
      session = await request
      upsertSessionCache(session)
    }
    void hydrateSessionDirectory(session, opts?.windowId)

    if (!Array.isArray(messagesBySession.value[sid])) {
      messagesBySession.value = { ...messagesBySession.value, [sid]: [] }
    }

    if (!messagesHydratedBySession.value[sid]) {
      await refreshMessages(sid)
    }
  }

  // ─── selection ────────────────────────────────────────────────────────────

  async function selectSession(id: string | null) {
    const sid = (id || '').trim()
    selectedSessionId.value = sid || null
    persistSelectedSession(sid || null)
    messagesError.value = null

    if (!sid) return
    await hydrateSession(sid)
  }

  // ─── messages ─────────────────────────────────────────────────────────────

  function sessionMessageLimit(sessionId: string): number {
    void sessionId
    return MESSAGE_PAGE_SIZE
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
    return [...list].sort((a, b) => compareChatIds(String(a?.info?.id ?? ''), String(b?.info?.id ?? '')))
  }

  function mergeMessageLists(
    older: MessageEntry[],
    newer: MessageEntry[],
    opts?: { authoritativeFolds?: boolean },
  ): MessageEntry[] {
    const map = new Map<string, MessageEntry>()
    for (const m of [...older, ...newer]) {
      const id = String(m?.info?.id ?? '')
      if (!id) continue
      const existing = map.get(id)
      if (existing) {
        const merged: MessageEntry = {
          info: { ...existing.info, ...m.info },
          parts: [...existing.parts],
          ...(existing.folds || m.folds ? { folds: [...(existing.folds || [])] } : {}),
        }
        const partMap = new Map<string, MessagePart>()
        for (const p of [...merged.parts, ...(m.parts || [])]) {
          const pid = String(p?.id ?? '')
          if (pid) partMap.set(pid, p)
        }
        merged.parts = [...partMap.values()].sort((a, b) => compareChatIds(String(a.id), String(b.id)))
        // A fold-free second message can be the result of a local expansion,
        // while a second message with folds is an authoritative transcript
        // refresh. Prefer the second message's fold snapshot when present;
        // otherwise retain the currently active local fold until the explicit
        // expansion request updates it.
        const preferredFolds = opts?.authoritativeFolds
          ? m.folds || []
          : m.folds?.length
            ? m.folds
            : existing.folds || []
        const foldMap = new Map<string, MessageFold>()
        for (const fold of preferredFolds) {
          const key = String(fold.runId)
          const previous = foldMap.get(key)
          const normalized = {
            ...fold,
            hiddenCount: Math.max(0, Math.floor(Number(fold.hiddenCount))),
          }
          if (!previous || normalized.hiddenCount <= previous.hiddenCount) foldMap.set(key, normalized)
        }
        const activeFolds = [...foldMap.values()].filter((fold) => fold.hiddenCount > 0 && Boolean(fold.nextCursor))
        if (activeFolds.length) merged.folds = activeFolds
        else delete merged.folds
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

  function isLatestRefreshMessagesRequest(
    sessionId: string,
    requestSeq: number,
    generation = transcriptCacheGeneration,
  ): boolean {
    const sid = (sessionId || '').trim()
    if (!sid || requestSeq <= 0) return false
    return generation === transcriptCacheGeneration && refreshMessagesRequestSeqBySession.get(sid) === requestSeq
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
    const adapterID = readString(info.adapterID as JsonValue) || readString(info.adapter_id as JsonValue)
    if (providerID) out.providerID = providerID
    if (adapterID) out.adapterID = adapterID
    if (modelID) out.modelID = modelID
    return out
  }

  async function refreshMessagesInternal(
    sessionId: string,
    opts?: { silent?: boolean; replace?: boolean; authoritativeFolds?: boolean },
  ): Promise<void> {
    const sid = (sessionId || '').trim()
    if (!sid) return

    const requestSeq = nextRefreshMessagesRequestSeq(sid)
    const generation = transcriptCacheGeneration
    const isSelected = selectedSessionId.value === sid
    const hasCache = (messagesBySession.value[sid]?.length ?? 0) > 0
    const silent = Boolean(opts?.silent)

    if (!silent && isSelected && !hasCache) messagesLoading.value = true
    if (isSelected) messagesError.value = null

    try {
      const limit = sessionMessageLimit(sid)
      const page = await chatApi.listMessages(sid, limit, undefined, DEFAULT_TRANSCRIPT_PART_PAGE_SIZE)
      if (!isLatestRefreshMessagesRequest(sid, requestSeq, generation)) return
      const ordered = normalizeMessageList(page.entries)
      const hasLoadedOlder = historyOlderLoadedBySession.value[sid] === true
      const currentMessages = ensureSessionMessages(sid)
      const shouldReplace = opts?.replace === true
      // A refresh can race with SSE delivery. Merge a live cache into the
      // server page so a valid part received during the request is not
      // discarded just because it was not in that page's snapshot. Explicit
      // rewind/replace callers still get exact server replacement semantics.
      const nextMessages =
        shouldReplace || (!currentMessages.length && !hasLoadedOlder)
          ? ordered
          : mergeMessageLists(currentMessages, ordered, {
              authoritativeFolds: opts?.authoritativeFolds,
            })
      setSessionMessages(sid, nextMessages)
      markMessagesHydrated(sid)
      if (typeof page.userMessageCount === 'number' && Number.isFinite(page.userMessageCount)) {
        historyUserMessageCountBySession.value = {
          ...historyUserMessageCountBySession.value,
          [sid]: Math.max(0, Math.floor(page.userMessageCount)),
        }
      }
      historyLimitBySession.value = { ...historyLimitBySession.value, [sid]: nextMessages.length }
      if (!hasLoadedOlder) {
        historyCursorBySession.value = { ...historyCursorBySession.value, [sid]: page.nextCursor ?? null }
        historyExhaustedBySession.value = { ...historyExhaustedBySession.value, [sid]: page.hasMore !== true }
      }

      // Capture run config from the last message that carries provider/model.
      for (let i = nextMessages.length - 1; i >= 0; i -= 1) {
        const patch = extractRunConfigFromMessageInfo(nextMessages[i]?.info)
        if (patch.providerID || patch.modelID) {
          upsertSessionRunConfig(sid, patch)
          break
        }
      }

      // Rehydrate status + attention after load.
      void refreshExecutionStatus(sid)
      void refreshAttention(sid)
    } catch (err) {
      if (!isLatestRefreshMessagesRequest(sid, requestSeq, generation)) return
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
      // `hasCache` was captured before the request. SSE may have populated the
      // cache while the request was in flight, so inspect the current cache
      // before clearing anything. Never turn a transient refresh failure into
      // a visible empty conversation.
      const hasCurrentCache = (messagesBySession.value[sid]?.length ?? 0) > 0
      if (!authRequired && hasCurrentCache) {
        scheduleMessageRefreshRetry(sid, 240)
      }
      if (!hasCurrentCache && !silent) {
        setSessionMessages(sid, [])
      }
    } finally {
      if (generation === transcriptCacheGeneration && !silent && isSelected) messagesLoading.value = false
    }
  }

  async function refreshMessages(
    sessionId: string,
    opts?: { silent?: boolean; replace?: boolean; authoritativeFolds?: boolean },
  ) {
    const sid = (sessionId || '').trim()
    if (!sid) return

    const existing = refreshMessagesInFlightBySession.get(sid)
    if (existing) {
      await existing
      return
    }

    const request = refreshMessagesInternal(sid, opts)
    refreshMessagesInFlightBySession.set(sid, request)
    try {
      await request
    } finally {
      if (refreshMessagesInFlightBySession.get(sid) === request) {
        refreshMessagesInFlightBySession.delete(sid)
      }
    }
  }

  const selectedHistory = computed(() => {
    const sid = selectedSessionId.value
    if (!sid) return { loading: false, exhausted: false, limit: 0 }
    return {
      loading: Boolean(historyLoadingBySession.value[sid]),
      exhausted: Boolean(historyExhaustedBySession.value[sid]),
      limit: typeof historyLimitBySession.value[sid] === 'number' ? Number(historyLimitBySession.value[sid]) : 0,
      userMessageCount:
        typeof historyUserMessageCountBySession.value[sid] === 'number'
          ? Number(historyUserMessageCountBySession.value[sid])
          : null,
    }
  })

  async function loadOlderMessages(sessionId: string) {
    const sid = (sessionId || '').trim()
    if (!sid) return false

    // A scroll event can arrive while the initial recent-page request is
    // still pending.  Let that request establish the cursor first; otherwise
    // both calls would fetch the same first page with a null cursor.
    const initialRequest = refreshMessagesInFlightBySession.get(sid)
    if (initialRequest) await initialRequest

    if (historyLoadingBySession.value[sid]) return false
    if (historyExhaustedBySession.value[sid]) return false

    const generation = transcriptCacheGeneration

    historyLoadingBySession.value = { ...historyLoadingBySession.value, [sid]: true }
    try {
      const cursor = historyCursorBySession.value[sid] ?? null
      const page = await chatApi.listMessages(sid, MESSAGE_PAGE_SIZE, cursor, DEFAULT_TRANSCRIPT_PART_PAGE_SIZE)
      if (generation !== transcriptCacheGeneration) return false
      const normalized = normalizeMessageList(page.entries)
      const merged = mergeMessageLists(normalized, ensureSessionMessages(sid))
      setSessionMessages(sid, merged)
      if (typeof page.userMessageCount === 'number' && Number.isFinite(page.userMessageCount)) {
        historyUserMessageCountBySession.value = {
          ...historyUserMessageCountBySession.value,
          [sid]: Math.max(0, Math.floor(page.userMessageCount)),
        }
      }
      historyLimitBySession.value = { ...historyLimitBySession.value, [sid]: merged.length }
      historyOlderLoadedBySession.value = { ...historyOlderLoadedBySession.value, [sid]: true }
      historyCursorBySession.value = { ...historyCursorBySession.value, [sid]: page.nextCursor ?? null }
      historyExhaustedBySession.value = { ...historyExhaustedBySession.value, [sid]: page.hasMore !== true }
      return normalized.length > 0
    } finally {
      if (generation === transcriptCacheGeneration) {
        historyLoadingBySession.value = { ...historyLoadingBySession.value, [sid]: false }
      }
    }
  }

  async function loadFoldedActivity(
    sessionId: string,
    fold: MessageFold,
    all = false,
    pageSize = transcriptPartPageSize.value,
  ): Promise<boolean> {
    const sid = (sessionId || '').trim()
    if (!sid || !fold.nextCursor || !Number.isFinite(fold.runId)) return false
    if (historyLoadingBySession.value[sid]) return false
    const generation = transcriptCacheGeneration
    historyLoadingBySession.value = { ...historyLoadingBySession.value, [sid]: true }
    try {
      let cursor: string | null = fold.nextCursor
      let loadedAny = false
      let activeFold = { ...fold }
      let remaining = Math.max(0, Math.floor(Number(activeFold.hiddenCount)))
      const requestedPageSize = normalizeTranscriptPartPageSize(pageSize)
      do {
        const page = await chatApi.listTranscriptFoldParts(sid, activeFold.runIds, requestedPageSize, cursor)
        if (generation !== transcriptCacheGeneration) return false
        const normalized = normalizeMessageList(page.entries)
        if (!normalized.length) break
        const merged = mergeMessageLists(normalized, ensureSessionMessages(sid))
        setSessionMessages(sid, merged)
        loadedAny = true
        const fetchedNonText = normalized.reduce(
          (total, entry) =>
            total +
            entry.parts.filter((part) => {
              const kind = String(part.agenaKind || '').toLowerCase()
              return kind !== 'run'
            }).length,
          0,
        )
        remaining = Math.max(0, remaining - fetchedNonText)
        cursor = page.nextCursor ?? null
        const nextAnchor =
          normalized
            .flatMap((entry) => entry.parts)
            .find((part) => {
              const kind = String(part.agenaKind || '').toLowerCase()
              return kind !== 'run'
            })?.id || fold.anchorPartId
        const nextFold: MessageFold = {
          ...activeFold,
          anchorPartId: String(nextAnchor),
          hiddenCount: remaining,
          nextCursor: cursor,
        }
        replaceMessageFold(sid, activeFold, nextFold)
        activeFold = nextFold
        if (!all || !page.hasMore || !cursor || remaining <= 0) break
      } while (all)
      return loadedAny
    } finally {
      if (generation === transcriptCacheGeneration) {
        historyLoadingBySession.value = { ...historyLoadingBySession.value, [sid]: false }
      }
    }
  }

  function replaceMessageFold(sessionId: string, previous: MessageFold, next: MessageFold) {
    const sid = (sessionId || '').trim()
    if (!sid) return
    const current = ensureSessionMessages(sid)
    const target = current.find((entry) =>
      (entry.folds || []).some((fold) => fold.runId === previous.runId && fold.anchorPartId === previous.anchorPartId),
    )
    if (!target) return
    const folds = (target.folds || [])
      .map((fold) => (fold.runId === previous.runId && fold.anchorPartId === previous.anchorPartId ? next : fold))
      .filter((fold) => fold.hiddenCount > 0 && Boolean(fold.nextCursor))
    if (folds.length) target.folds = folds
    else delete target.folds
  }

  function setTranscriptPartPageSize(value: number) {
    const next = normalizeTranscriptPartPageSize(value)
    if (transcriptPartPageSize.value === next) return
    transcriptPartPageSize.value = next
    setLocalJson(localStorageKeys.chat.transcriptPartPageSize, next)
  }

  /** Drop transcript pages when the chat page is actually unmounted. */
  function clearTranscriptCache() {
    transcriptCacheGeneration += 1
    for (const timer of refreshMessagesRetryTimerBySession.values()) window.clearTimeout(timer)
    for (const timer of statusRefreshTimerBySession.values()) window.clearTimeout(timer)
    refreshMessagesRetryTimerBySession.clear()
    statusRefreshTimerBySession.clear()
    refreshMessagesRequestSeqBySession.clear()
    refreshMessagesInFlightBySession.clear()
    refreshExecutionInFlightBySession.clear()
    refreshAttentionInFlightBySession.clear()
    presentedInteractiveRequestBySession.clear()
    messagesBySession.value = {}
    messagesHydratedBySession.value = {}
    historyLimitBySession.value = {}
    historyUserMessageCountBySession.value = {}
    historyLoadingBySession.value = {}
    historyExhaustedBySession.value = {}
    historyCursorBySession.value = {}
    historyOlderLoadedBySession.value = {}
    messagesLoading.value = false
    messagesError.value = null
  }

  // ─── execution status + attention ─────────────────────────────────────────

  function scheduleSessionStatusRefresh(sessionId: string, delayMs = 150) {
    const sid = (sessionId || '').trim()
    if (!sid) return
    if (statusRefreshTimerBySession.has(sid)) return
    const timer = window.setTimeout(
      () => {
        statusRefreshTimerBySession.delete(sid)
        void refreshAttention(sid)
        void refreshExecutionStatus(sid)
      },
      Math.max(60, Math.min(1200, Math.floor(delayMs))),
    )
    statusRefreshTimerBySession.set(sid, timer)
  }

  async function refreshExecutionStatusInternal(sessionId: string): Promise<void> {
    const sid = (sessionId || '').trim()
    if (!sid) return
    const st = await chatApi.getSessionExecutionStatus(sid).catch(() => null)
    if (!st) return
    upsertSessionCache({ id: sid, state: st.state })

    const execution = st.execution
    if (execution && typeof execution === 'object') {
      sessionExecutionBySession.value = {
        ...sessionExecutionBySession.value,
        [sid]: { ...execution },
      }
      const patch: Partial<SessionRunConfig> = {}
      const providerID = readString(execution.model_provider_id as JsonValue)
      const adapterID = readString(execution.model_adapter_id as JsonValue)
      const modelID = readString(execution.model_id as JsonValue)
      const thinkingMode = readString(execution.model_thinking_mode as JsonValue)
      const speedMode = readString(execution.model_speed_mode as JsonValue)
      const verbosity = readString(execution.model_verbosity as JsonValue)
      if (providerID) patch.providerID = providerID
      if (adapterID) patch.adapterID = adapterID
      if (modelID) patch.modelID = modelID
      if (thinkingMode) patch.thinkingMode = thinkingMode
      if (speedMode) patch.speedMode = speedMode
      if (verbosity) patch.verbosity = verbosity
      if (typeof execution.model_parallel_tool_calls === 'boolean') {
        patch.parallelToolCalls = execution.model_parallel_tool_calls
      }
      if (Object.keys(patch).length) upsertSessionRunConfig(sid, patch)
    }

    if (st.usage && typeof st.usage === 'object') {
      sessionUsageBySession.value = {
        ...sessionUsageBySession.value,
        [sid]: { ...st.usage },
      }
    }
  }

  async function refreshExecutionStatus(sessionId: string) {
    const sid = (sessionId || '').trim()
    if (!sid) return

    const existing = refreshExecutionInFlightBySession.get(sid)
    if (existing) {
      await existing
      return
    }

    const request = refreshExecutionStatusInternal(sid)
    refreshExecutionInFlightBySession.set(sid, request)
    try {
      await request
    } finally {
      if (refreshExecutionInFlightBySession.get(sid) === request) {
        refreshExecutionInFlightBySession.delete(sid)
      }
    }
  }

  /** Translate canonical session.state.data.requests into the AttentionEvent shape. */
  function attentionFromPendingRequests(sessionId: string, requests: JsonValue[]): AttentionEvent | null {
    const sid = (sessionId || '').trim()
    if (!sid || !Array.isArray(requests) || requests.length === 0) return null
    for (const raw of requests) {
      const rec = asRecord(raw)
      const outerKind = readString(rec.kind as JsonValue).toLowerCase()
      const inputKind = readString(rec.input_kind as JsonValue)
      const requestId = readString(rec.request_id as JsonValue)
      if (!requestId) continue
      // PendingInteractiveRequest serializes its envelope as
      // { kind: "user_input", input_kind: "review" | "ask_user", ... }.
      // Keep both values distinct: the outer kind selects the reply family,
      // while input_kind controls the inline transcript layout.
      const isUserInput = outerKind === 'user_input' || Boolean(inputKind) || Array.isArray(rec.questions)
      if (isUserInput) {
        const title = readString(rec.title as JsonValue) || 'Question'
        const body = readString(rec.body_markdown as JsonValue)
        const rawQuestions = rec.questions
        const questions = Array.isArray(rawQuestions)
          ? rawQuestions
              .map((q, index) => {
                const qr = asRecord(q)
                const header = readString(qr.header as JsonValue) || title || `Question ${index + 1}`
                const question =
                  readString(qr.question as JsonValue) || readString(qr.title as JsonValue) || `Question ${index + 1}`
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
                const allowCustom = qr.allow_custom === true || qr.allowCustom === true || qr.custom === true
                return {
                  question_id: readString(qr.question_id as JsonValue) || String(index),
                  header,
                  question,
                  options,
                  multiple,
                  allow_custom: allowCustom,
                }
              })
              .filter((q) => Boolean(q.question))
          : []
        if (questions.length === 0) continue
        return {
          kind: 'question',
          at: Date.now(),
          payload: {
            type: 'question.asked',
            properties: {
              id: requestId,
              title,
              body_markdown: body,
              input_kind: inputKind || (outerKind === 'user_input' ? 'ask_user' : outerKind),
              questions,
              // Keep the canonical request available to any status/debug
              // consumer. The transcript itself reads the operation-owned
              // request, but attention projection must not discard fields.
              request: rec,
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
            // Keep the full state request available to the transcript
            // fallback. The compact legacy fields above remain for sidebar
            // and command consumers, while the inline UI needs reason,
            // explanation, source, scope, and the structured action.
            request: rec,
          },
        },
      }
    }
    return null
  }

  async function refreshAttentionInternal(sessionId: string): Promise<void> {
    const sid = (sessionId || '').trim()
    if (!sid) return
    const state = await chatApi.getSessionExecution(sid).catch(() => null)
    if (!state) return
    const next = attentionFromPendingRequests(sid, sessionStateRequests(state.session?.state) as JsonValue[])
    if (next) {
      attentionBySession.value = { ...attentionBySession.value, [sid]: next }
      const requestId = readString(asRecord(next.payload.properties).id as JsonValue)
      if (requestId && presentedInteractiveRequestBySession.get(sid) !== requestId) {
        presentedInteractiveRequestBySession.set(sid, requestId)
        void chatApi.presentInteractiveRequest(sid, requestId)
      }
    } else {
      if (attentionBySession.value[sid]) {
        const nextMap = { ...attentionBySession.value }
        delete nextMap[sid]
        attentionBySession.value = nextMap
      }
      presentedInteractiveRequestBySession.delete(sid)
    }
  }

  async function refreshAttention(sessionId: string) {
    const sid = (sessionId || '').trim()
    if (!sid) return

    const existing = refreshAttentionInFlightBySession.get(sid)
    if (existing) {
      await existing
      return
    }

    const request = refreshAttentionInternal(sid)
    refreshAttentionInFlightBySession.set(sid, request)
    try {
      await request
    } finally {
      if (refreshAttentionInFlightBySession.get(sid) === request) {
        refreshAttentionInFlightBySession.delete(sid)
      }
    }
  }

  function clearAttention(sessionId: string) {
    const sid = (sessionId || '').trim()
    if (!sid) return
    if (attentionBySession.value[sid]) {
      const next = { ...attentionBySession.value }
      delete next[sid]
      attentionBySession.value = next
    }
    presentedInteractiveRequestBySession.delete(sid)
  }

  const selectedAttention = computed(() => {
    const sid = selectedSessionId.value
    if (!sid) return null
    return attentionBySession.value[sid] ?? null
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

  const selectedSessionUsage = computed(() => {
    const sid = selectedSessionId.value
    if (!sid) return null
    return sessionUsageBySession.value[sid] ?? null
  })

  const selectedSessionExecution = computed(() => {
    const sid = selectedSessionId.value
    if (!sid) return null
    return sessionExecutionBySession.value[sid] ?? null
  })

  function getMessagesForSession(sessionId: string | null | undefined): MessageEntry[] {
    const sid = String(sessionId || '').trim()
    if (!sid) return []
    const list = messagesBySession.value[sid]
    return Array.isArray(list) ? list : []
  }

  function getSessionDirectory(sessionId: string | null | undefined): string | null {
    const sid = String(sessionId || '').trim()
    return sid ? sessionDirectoryBySessionId.value[sid] || null : null
  }

  function getSessionState(sessionId: string | null | undefined): SessionState {
    return getSessionById(sessionId)?.state || { kind: 'ready', data: {} }
  }

  function getSessionHistory(sessionId: string | null | undefined) {
    const sid = String(sessionId || '').trim()
    if (!sid) return { loading: false, exhausted: false, limit: 0 }
    return {
      loading: Boolean(historyLoadingBySession.value[sid]),
      exhausted: Boolean(historyExhaustedBySession.value[sid]),
      limit: typeof historyLimitBySession.value[sid] === 'number' ? Number(historyLimitBySession.value[sid]) : 0,
      userMessageCount:
        typeof historyUserMessageCountBySession.value[sid] === 'number'
          ? Number(historyUserMessageCountBySession.value[sid])
          : null,
    }
  }

  function getSessionAttention(sessionId: string | null | undefined): AttentionEvent | null {
    const sid = String(sessionId || '').trim()
    return sid ? attentionBySession.value[sid] || null : null
  }

  function getSessionError(sessionId: string | null | undefined): SessionErrorEvent | null {
    const sid = String(sessionId || '').trim()
    return sid ? sessionErrorBySession.value[sid] || null : null
  }

  function getSessionRunConfig(sessionId: string | null | undefined): SessionRunConfig | null {
    const sid = String(sessionId || '').trim()
    return sid ? sessionRunConfigBySession.value[sid] || null : null
  }

  function getSessionUsage(sessionId: string | null | undefined): SessionUsage | null {
    const sid = String(sessionId || '').trim()
    return sid ? sessionUsageBySession.value[sid] || null : null
  }

  function getSessionExecution(
    sessionId: string | null | undefined,
  ): NonNullable<chatApi.AgenaExecutionState['execution']> | null {
    const sid = String(sessionId || '').trim()
    return sid ? sessionExecutionBySession.value[sid] || null : null
  }

  function clearSessionError(sessionId: string) {
    const sid = (sessionId || '').trim()
    if (!sid) return
    if (!Object.prototype.hasOwnProperty.call(sessionErrorBySession.value, sid)) return
    const next = { ...sessionErrorBySession.value }
    delete next[sid]
    sessionErrorBySession.value = next
  }

  // ─── session CRUD ─────────────────────────────────────────────────────────

  async function createSession(opts?: {
    workspaceId?: number
    workspacePath?: string
    title?: string
    parentId?: number
  }): Promise<Session | null> {
    if (createSessionInFlight) return await createSessionInFlight
    createSessionInFlight = (async () => {
      try {
        let workspaceId = Number(opts?.workspaceId)
        if (!Number.isSafeInteger(workspaceId) || workspaceId <= 0) {
          let workspacePath = String(opts?.workspacePath || directoryStore.currentDirectory || '').trim()
          if (!workspacePath) workspacePath = await chatApi.getRuntimeWorkspaceRoot()
          const workspace = await chatApi.resolveWorkspace(workspacePath)
          workspaceId = workspace.id
        }
        const title = String(opts?.title || i18n.global.t('chat.sidebar.directoryActions.newSession.label')).trim()
        const created = await chatApi.createSession({
          workspaceId,
          title: title || 'New session',
          ...(typeof opts?.parentId === 'number' ? { parentId: opts.parentId } : {}),
        })
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
      const next = { ...sessionDirectoryBySessionId.value }
      delete next[sid]
      sessionDirectoryBySessionId.value = next
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
    {
      const next = { ...sessionUsageBySession.value }
      delete next[sid]
      sessionUsageBySession.value = next
    }
    {
      const next = { ...sessionExecutionBySession.value }
      delete next[sid]
      sessionExecutionBySession.value = next
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
      const next = { ...historyUserMessageCountBySession.value }
      delete next[sid]
      historyUserMessageCountBySession.value = next
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
    {
      const next = { ...historyCursorBySession.value }
      delete next[sid]
      historyCursorBySession.value = next
    }
    {
      const next = { ...historyOlderLoadedBySession.value }
      delete next[sid]
      historyOlderLoadedBySession.value = next
    }

    sessions.value = sessions.value.filter((s) => s?.id !== sid)
    scheduleSessionsRefresh(1200)
  }

  async function renameSession(sessionId: string, title: string) {
    const sid = (sessionId || '').trim()
    const trimmed = (title || '').trim()
    if (!sid || !trimmed) return null
    const updated = await updateSessionMetadata(sid, { title: trimmed })
    return updated
  }

  async function updateSessionMetadata(
    sessionId: string,
    patch: { title?: string; favorite?: boolean; pinned?: boolean },
  ) {
    const sid = (sessionId || '').trim()
    if (!sid || Object.keys(patch).length === 0) return null
    const updated = await chatApi.patchSessionMetadata(sid, patch)
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

  function buildRunOptions(opts: {
    providerID?: string
    adapterID?: string
    modelID?: string
    thinkingMode?: string
    speedMode?: string
    verbosity?: string
    parallelToolCalls?: boolean
  }): chatApi.RunOptionsPayload {
    const providerID = (opts.providerID || '').trim()
    const adapterID = (opts.adapterID || '').trim()
    const modelID = (opts.modelID || '').trim()
    const options: chatApi.RunOptionsPayload = {}
    if (providerID && modelID) {
      const model: chatApi.AgenaModelRef = {
        provider_id: providerID,
        ...(adapterID ? { adapter_id: adapterID } : {}),
        model_id: modelID,
      }
      options.model = model
    }
    const thinkingMode = String(opts.thinkingMode || '').trim()
    const speedMode = String(opts.speedMode || '').trim()
    const verbosity = String(opts.verbosity || '').trim()
    if (thinkingMode) options.thinking_mode = thinkingMode
    if (speedMode) options.speed_mode = speedMode
    if (verbosity) options.verbosity = verbosity
    if (typeof opts.parallelToolCalls === 'boolean') options.parallel_tool_calls = opts.parallelToolCalls
    return options
  }

  async function sendMessage(
    sessionId: string,
    opts: {
      text?: string
      parts?: JsonValue[]
      providerID?: string
      adapterID?: string
      modelID?: string
      thinkingMode?: string
      speedMode?: string
      verbosity?: string
      parallelToolCalls?: boolean
    },
  ) {
    const sid = (sessionId || '').trim()
    const document = buildDocument(opts)
    if (document.length === 0) return { queued: false }
    clearSessionError(sid)
    await chatApi.sendMessage(sid, { document, ...buildRunOptions(opts) })
    // Let SSE stream parts in; one coalesced status refresh is enough after
    // the POST.  The timer also absorbs a burst of runtime signals.
    scheduleSessionStatusRefresh(sid, 200)
    return { queued: true }
  }

  async function sendText(sessionId: string, text: string) {
    await sendMessage(sessionId, { text })
  }

  async function continueSession(sessionId: string) {
    const sid = (sessionId || '').trim()
    if (!sid) return
    await chatApi.continueSession(sid, buildRunOptions({}))
    scheduleSessionStatusRefresh(sid, 200)
  }

  async function uploadWorkspaceAttachment(
    sessionId: string,
    input: { filename: string; dataBase64: string; mime?: string },
  ) {
    const sid = String(sessionId || '').trim()
    if (!sid) throw new Error('A session is required for attachments')
    let session = getSessionById(sid)
    if (!session) {
      session = await chatApi.getSession(sid)
      upsertSessionCache(session)
    }
    const workspaceId = Number(session.workspace_id)
    if (!Number.isSafeInteger(workspaceId) || workspaceId <= 0) {
      throw new Error('The session does not have a valid workspace')
    }
    return await chatApi.uploadWorkspaceFile(workspaceId, input)
  }

  async function resolveSessionWorkspace(sessionId: string): Promise<{ id: number; path: string }> {
    const sid = String(sessionId || '').trim()
    if (!sid) throw new Error('A session is required')
    let session = getSessionById(sid)
    if (!session) {
      session = await chatApi.getSession(sid)
      upsertSessionCache(session)
    }
    const workspace = await workspaceForSession(session)
    if (!workspace) {
      throw new Error('The session does not have a valid workspace')
    }
    return workspace
  }

  function removeMessageForRun(sessionId: string, runId: number) {
    const sid = (sessionId || '').trim()
    if (!sid || !Number.isFinite(runId)) return
    const list = messagesBySession.value[sid]
    if (!Array.isArray(list)) return
    const retained = list.filter((message) => Number(message?.info?.runId) !== runId)
    if (retained.length !== list.length) setSessionMessages(sid, retained)
  }

  async function abortSession(sessionId: string): Promise<chatApi.CancellationOutcome | null> {
    const sid = (sessionId || '').trim()
    if (!sid) return null
    try {
      const st = await chatApi.getSessionExecutionStatus(sid).catch(() => null)
      const executionId = sessionStateExecution(st?.state)?.execution_id
      let outcome: chatApi.CancellationOutcome
      if (!executionId) {
        // The stop action also suppresses queued background notification wakes.
        // There may be no execution in this short window even though the next
        // delivery would otherwise start one immediately.
        outcome = await chatApi.cancelSession(sid, null)
        clearAttention(sid)
      } else {
        outcome = await chatApi.cancelSession(sid, executionId)
        // If the short-lived execution changed between the snapshot and the
        // exact cancel request, the stop action still applies to the current
        // session. The session-scoped fallback also suppresses queued delivery
        // wakes that could otherwise create the next execution immediately.
        const after = await chatApi.getSessionExecutionStatus(sid).catch(() => null)
        const afterExecutionId = sessionStateExecution(after?.state)?.execution_id
        if (outcome.result === 'execution_mismatch' || (afterExecutionId && afterExecutionId !== executionId)) {
          outcome = await chatApi.cancelSession(sid, null)
        }
      }
      if (typeof outcome.restored_user_run_id === 'number') {
        removeMessageForRun(sid, outcome.restored_user_run_id)
      }
      clearAttention(sid)
      return outcome
    } catch {
      return null
    }
  }

  // ─── interactive replies ──────────────────────────────────────────────────

  async function replyPermission(
    sessionId: string,
    requestId: string,
    reply: 'once' | 'always' | 'reject' | 'reject_always',
    message?: string,
  ) {
    const ok = await chatApi.replyPermission(sessionId, requestId, reply, message)
    if (ok) clearAttention((sessionId || '').trim())
    return ok
  }

  async function replyQuestion(sessionId: string, requestId: string, answers: string[][]) {
    const questions = ensureSessionMessages(sessionId)
    void questions
    const answersMap: Record<string, string[]> = {}
    const state = await chatApi.getSessionExecution(sessionId).catch(() => null)
    const requests = sessionStateRequests(state?.session?.state) as JsonValue[]
    const request = (Array.isArray(requests) ? requests : []).find((r) => {
      const rec = asRecord(r)
      return readString(rec.request_id as JsonValue) === requestId
    })
    const rawQuestions = request ? asRecord(request).questions : null
    if (Array.isArray(rawQuestions)) {
      rawQuestions.forEach((_, index) => {
        // The user-input reply contract is indexed by question ordinal. A
        // question_id may be present in a future/legacy presentation payload,
        // but it is not a valid wire key for this endpoint: the runtime
        // validator accepts only "0", "1", ... and rejects unknown keys.
        answersMap[String(index)] = Array.isArray(answers[index]) ? answers[index] : []
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
    await chatApi.compactSession(sid, buildRunOptions({}))
    scheduleSessionStatusRefresh(sid, 200)
    scheduleSessionsRefresh(1200)
  }

  // ─── fork ─────────────────────────────────────────────────────────────────

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

    // Stop any active run first. The tagged SessionState is the only source
    // used to decide whether cancellation is meaningful.
    const state = getSessionState(sid)
    if (state.kind === 'running' || state.kind === 'creating' || sessionStateExecution(state)) {
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
    await refreshMessages(sid, { silent: true, replace: true })
    scheduleSessionsRefresh(1200)
  }

  // ─── composer helpers ─────────────────────────────────────────────────────

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
   *   permission.asked / question.asked → session.state.data.requests (via state refresh)
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
              const messageAlreadyKnown = binarySearchById(list, String(partId), (m) => m.info.id).found
              const runState = readString(part.state as JsonValue)
              const content = asRecord(part.content)
              const info: MessageInfo = {
                id: String(partId),
                sessionID: sid,
                role: readString(part.role as JsonValue) || 'assistant',
                runId: partId,
                ...(runState ? { runState } : {}),
                runContent: content,
                ...(isRunTerminal(runState) ? { finish: runState } : {}),
                time: { created: readNumber(part.created_at_ms) ?? Date.now() },
              }
              const providerID = readString(content.provider_id as JsonValue)
              const adapterID = readString(content.adapter_id as JsonValue)
              const modelID = readString(content.model_id as JsonValue)
              const turnId = readString(content.turn_id as JsonValue)
              if (providerID) info.providerID = providerID
              if (adapterID) info.adapterID = adapterID
              if (modelID) info.modelID = modelID
              if (turnId) info.turnId = turnId
              upsertMessageEntryIn(list, info)
              if (info.role === 'user' && !messageAlreadyKnown) {
                const currentCount = historyUserMessageCountBySession.value[sid]
                if (typeof currentCount === 'number') {
                  historyUserMessageCountBySession.value = {
                    ...historyUserMessageCountBySession.value,
                    [sid]: currentCount + 1,
                  }
                }
              }
              scheduleSessionsRefresh(800)
            } else {
              const existing = binarySearchById(list, key, (m) => m.info.id)
              if (existing.found && list[existing.index]) {
                const messageError = messageErrorFromAgenaPart(part as JsonValue)
                if (messageError) list[existing.index].info.error = messageError
                const partOut = normalizeAgenaPart(String(partId), sid, key, part as JsonValue)
                if (partOut) {
                  upsertPart(list[existing.index], partOut, '')
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
                const messageError = messageErrorFromAgenaPart(part as JsonValue)
                if (messageError) info.error = messageError
                const entry = upsertMessageEntryIn(list, info)
                const partOut = normalizeAgenaPart(String(partId), sid, key, part as JsonValue)
                if (partOut) entry.parts.push(partOut)
              }
            }
            // Any part change can affect status/attention.
            scheduleSessionStatusRefresh(sid)
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
        const favorite = typeof props.favorite === 'boolean' ? props.favorite : undefined
        const pinned = typeof props.pinned === 'boolean' ? props.pinned : undefined
        const updatedAtMs = readNumber(props.updated_at_ms)
        const current = getSessionById(sid)
        if (title || current) {
          upsertSessionCache({
            id: sid,
            ...(title ? { title } : {}),
            ...(favorite !== undefined ? { favorite } : {}),
            ...(pinned !== undefined ? { pinned } : {}),
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
        scheduleSessionStatusRefresh(signalSession)
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
    selectedSessionState,
    selectedSessionDirectory,
    messages,
    messagesLoading,
    messagesError,
    transcriptPartPageSize,
    selectedAttention,
    selectedSessionError,
    selectedSessionRunConfig,
    selectedSessionUsage,
    selectedSessionExecution,
    getSessionExecution,
    sessionErrorBySession,
    sessionRunConfigBySession,
    attentionBySession,
    refreshSessions,
    selectSession,
    hydrateSession,
    refreshMessages,
    loadOlderMessages,
    loadFoldedActivity,
    setTranscriptPartPageSize,
    clearTranscriptCache,
    selectedHistory,
    createSession,
    deleteSession,
    renameSession,
    updateSessionMetadata,
    abortSession,
    sendText,
    sendMessage,
    continueSession,
    uploadWorkspaceAttachment,
    resolveSessionWorkspace,
    compactSession,
    forkSession,
    replyPermission,
    replyQuestion,
    rejectQuestion,
    getSessionById,
    getMessagesForSession,
    getSessionDirectory,
    getSessionState,
    getSessionHistory,
    getSessionAttention,
    getSessionError,
    getSessionRunConfig,
    getSessionUsage,
    cacheSessions,
    consumePendingComposer,
    getComposerDraft,
    setComposerDraft,
    clearSessionError,
    revertToMessage,
    applyEvent,
  }
})

type ChatStore = ReturnType<typeof useChatStoreDefinition>

function scopedChat(store: ChatStore, pane: WorkspacePaneContext): ChatStore {
  function selectedSessionId(): string | null {
    return readSessionIdFromQuery(pane.route.value.query) || null
  }

  async function selectSession(id: string | null) {
    const sid = String(id || '').trim()
    if (sid) await store.hydrateSession(sid, { windowId: pane.windowId.value })

    const query: Record<string, string> = {}
    for (const [rawKey, rawValue] of Object.entries(pane.route.value.query || {})) {
      const key = String(rawKey || '').trim()
      if (!key || ['session', 'sessionid', 'sessionId', 'windowid', 'windowId', 'ocEmbed'].includes(key)) continue
      const value = Array.isArray(rawValue)
        ? String(rawValue.find((item) => String(item || '').trim()) || '').trim()
        : String(rawValue || '').trim()
      if (value) query[key] = value
    }
    if (sid) query.sessionId = sid

    await pane.navigate(
      {
        path: '/chat',
        query,
        hash: pane.route.value.hash,
      },
      true,
    )
  }

  return new Proxy(store, {
    get(target, property, receiver) {
      const sid = selectedSessionId()
      if (property === 'selectedSessionId') return sid
      if (property === 'selectedSession') return target.getSessionById(sid)
      if (property === 'selectedSessionDirectory') return target.getSessionDirectory(sid)
      if (property === 'messages') return target.getMessagesForSession(sid)
      if (property === 'messagesLoading') return sid === target.selectedSessionId ? target.messagesLoading : false
      if (property === 'messagesError') return sid === target.selectedSessionId ? target.messagesError : null
      if (property === 'selectedHistory') return target.getSessionHistory(sid)
      if (property === 'selectedAttention') return target.getSessionAttention(sid)
      if (property === 'selectedSessionError') return target.getSessionError(sid)
      if (property === 'selectedSessionRunConfig') return target.getSessionRunConfig(sid)
      if (property === 'selectedSessionUsage') return target.getSessionUsage(sid)
      if (property === 'selectedSessionExecution') return target.getSessionExecution(sid)
      if (property === 'selectSession') return selectSession
      if (property === 'createSession') {
        return async (...args: Parameters<ChatStore['createSession']>) => {
          const created = await target.createSession(...args)
          if (created?.id) await selectSession(created.id)
          return created
        }
      }
      if (property === 'deleteSession') {
        return async (...args: Parameters<ChatStore['deleteSession']>) => {
          const deletedId = String(args[0] || '').trim()
          const result = await target.deleteSession(...args)
          if (deletedId && deletedId === selectedSessionId()) await selectSession(null)
          return result
        }
      }
      return Reflect.get(target, property, receiver)
    },
  })
}

export const useChatStore = Object.assign(
  (...args: Parameters<typeof useChatStoreDefinition>) => {
    const store = useChatStoreDefinition(...args)
    const pane = useWorkspacePaneContext()
    return pane ? scopedChat(store, pane) : store
  },
  { $id: useChatStoreDefinition.$id },
) as typeof useChatStoreDefinition
