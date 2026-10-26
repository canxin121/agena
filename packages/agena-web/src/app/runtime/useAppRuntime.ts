import { onBeforeUnmount, onMounted, watch } from 'vue'
import { useRoute } from 'vue-router'

import { useHealthStore } from '@/stores/health'
import { useSettingsStore } from '@/stores/settings'
import { useSessionActivityStore } from '@/stores/sessionActivity'
import { useChatStore } from '@/stores/chat'
import { useUiStore } from '@/stores/ui'
import { useDirectoryStore } from '@/stores/directory'
import { useDirectorySessionStore } from '@/stores/directorySessionStore'
import { useBrowserNotificationsStore } from '@/stores/browserNotifications'

import { withInterfaceLocale } from '@/i18n/interfaceLocale'
import { connectSse } from '@/lib/sse'
import { createRevalidator } from '@/lib/revalidation'
import { isDocumentVisible } from '@/lib/backgroundReads'
import { applyResourceEvent, invalidateResources, startResourceSync } from '@/lib/resourceSync'
import type { SseClient, SseClientStats } from '@/lib/sse'
import { subscribeAppBroadcast } from '@/lib/appBroadcast'
import { installKeyboardInsets } from '@/lib/keyboardInsets'
import { installKeyboardTapFix } from '@/lib/keyboardTapFix'
import { installKeyboardShortcuts } from '@/app/runtime/installKeyboardShortcuts'
import { readSessionIdFromFullPath, readSessionIdFromQuery } from '@/app/navigation/sessionQuery'
import { useWorkspaceNavigation } from '@/app/navigation/useWorkspaceNavigation'
import { createBrowserNotifier } from '@/lib/browserNotifications'
import { ApiError, apiJson, apiUrl } from '@/lib/api'
import { limitBackgroundReads } from '@/lib/backgroundReads'
import { i18n } from '@/i18n'
import { normalizeSessionState, sessionStateRequests } from '@/types/chat'

export function useAppRuntime() {
  const route = useRoute()

  const ui = useUiStore()
  const health = useHealthStore()
  const settings = useSettingsStore()
  const activity = useSessionActivityStore()
  const chat = useChatStore()
  const directory = useDirectoryStore()
  const directorySessions = useDirectorySessionStore()
  const notifications = useBrowserNotificationsStore()
  const navigation = useWorkspaceNavigation()
  const notifier = createBrowserNotifier({
    enabled: () => notifications.enabled,
    scope: () => apiUrl('/'),
    sessionTitle: (sid) => chat.getSessionById(sid)?.title || '',
    message: (kind) => String(i18n.global.t(`browserNotifications.events.${kind}`)),
    openSession: (sid) =>
      navigation.openWorkspaceLocation('chat', {
        query: { sessionId: sid },
        title: chat.getSessionById(sid)?.title,
        matchKeys: ['sessionId'],
        reuseExisting: true,
      }),
    readSession: (sid, replyIds) =>
      limitBackgroundReads(async () => {
        const path = `/api/v1/sessions/${encodeURIComponent(sid)}`
        const init = { signal: AbortSignal.timeout(15_000) }
        try {
          // Status shells carry canonical pending requests without loading history.
          const status = await apiJson<{
            session?: { state?: unknown }
            parts?: unknown[]
            queued_messages?: unknown[]
          }>(`${path}/state?include_parts=false`, init)
          const requests = sessionStateRequests(normalizeSessionState(status.session?.state))
          if (!replyIds) return { parts: [], requests }
          // Queued inputs follow the active reply in newest-first message windows.
          const limit = Math.min(128, (status.queued_messages?.length || 0) + 1)
          const query = replyIds.length ? `/parts?ids=${replyIds.join(',')}` : `/runs?limit=${limit}&part_limit=1`
          const reply = await apiJson<{ parts?: unknown[] }>(`${path}${query}`, init)
          return { parts: reply.parts || [], requests }
        } catch (error) {
          if (error instanceof ApiError && error.status === 404) return null
          throw error
        }
      }),
  })

  let disposed = false
  let sse: SseClient | null = null
  let visibilityHandler: (() => void) | null = null
  let onlineHandler: (() => void) | null = null
  let focusHandler: (() => void) | null = null
  let blurHandler: (() => void) | null = null
  let pageShowHandler: ((evt: PageTransitionEvent) => void) | null = null
  let pageHideHandler: ((evt: PageTransitionEvent) => void) | null = null
  let freezeHandler: (() => void) | null = null
  let resumeHandler: (() => void) | null = null
  let cleanupKeyboard: (() => void) | null = null
  let cleanupShortcuts: (() => void) | null = null
  let cleanupKeyboardTapFix: (() => void) | null = null
  let cleanupBroadcast: (() => void) | null = null
  let cleanupResourceSync: (() => void) | null = null
  let sseDebugTimer: number | null = null
  let lastSseDebugAt = 0
  let lastSseDebugErrorSum = 0
  const recovery = createRevalidator(
    async () => {
      chat.reconcileLiveState()
      invalidateResources()
      directorySessions.scheduleSidebarRecoverySync('stream-connected', 0, { force: true })
      activity.invalidate()
      await settings.refresh()
    },
    { intervalMs: 10_000, retryMs: 10_000, enabled: () => !disposed && isDocumentVisible() },
  )
  const notificationRecovery = createRevalidator(() => notifier.reconcile(), {
    intervalMs: 1000,
    retryMs: 5000,
    enabled: () => !disposed && notifications.enabled,
  })

  let globalSseCursor = ''

  let lastResumeSyncAt = 0
  let lastVisibilityState: DocumentVisibilityState | null = null

  // Watchdog for system sleep/background timer throttling.
  let clockWatchdogTimer: number | null = null
  let lastClockWatchdogAt = Date.now()
  let pendingResumeReason: string | null = null
  let pendingResumeGapMs = 0

  function sseMaxChunkAgeMs(): number {
    const now = Date.now()
    const stats = [readSseStats(sse)]
    let maxAge = 0
    for (const st of stats) {
      const lastChunkAtRaw = (st as Partial<SseClientStats> | null)?.lastChunkAt
      const lastChunkAt = typeof lastChunkAtRaw === 'number' && Number.isFinite(lastChunkAtRaw) ? lastChunkAtRaw : 0
      if (!Number.isFinite(lastChunkAt) || lastChunkAt <= 0) continue
      maxAge = Math.max(maxAge, now - lastChunkAt)
    }
    return maxAge
  }

  function shouldForceReconnectAfterResume(opts: { reason: string; gapMs?: number }) {
    const gapMs = Math.max(0, Math.floor(opts.gapMs || 0))

    // If the JS thread was paused for a while, fetch-stream SSE can silently stall.
    if (gapMs >= 20000) return true

    // If we haven't seen any SSE bytes in a long time while visible, treat it as stalled.
    const maxAge = sseMaxChunkAgeMs()
    if (document.visibilityState === 'visible' && maxAge >= 45000) return true

    // BFCache restores a fully-rendered page but network connections are dead.
    if (opts.reason === 'pageshow') return true
    if (opts.reason === 'resume') return true

    return false
  }

  function resyncAfterResume(reason: string, opts?: { gapMs?: number }) {
    if (disposed || !isDocumentVisible()) return
    const now = Date.now()
    if (now - lastResumeSyncAt < 1500) return
    lastResumeSyncAt = now

    const forceReconnect = shouldForceReconnectAfterResume({ reason, gapMs: opts?.gapMs })
    if (forceReconnect || !sse) {
      connectActivity()
    }

    recovery.invalidate(0)

    try {
      console.debug('[sse] resync after resume:', reason)
    } catch {
      // ignore
    }

    logSseStats({ force: true, reason: `resume:${reason}` })
  }

  function readSseStats(client: { getStats?: () => unknown } | null): unknown {
    if (!client) return null
    if (typeof client.getStats === 'function') {
      try {
        return client.getStats()
      } catch {
        return null
      }
    }
    return null
  }

  function logSseStats(opts?: { force?: boolean; reason?: string }) {
    if (!import.meta.env.DEV) return
    const now = Date.now()
    if (!opts?.force && now - lastSseDebugAt < 15000) return

    const globalStats = readSseStats(sse)

    const errorSum = Number((globalStats as { errorCount?: number } | null)?.errorCount || 0)

    const maxAgeMs = Math.max(
      globalStats && typeof (globalStats as { lastChunkAt?: number }).lastChunkAt === 'number'
        ? now - Number((globalStats as { lastChunkAt?: number }).lastChunkAt || 0)
        : 0,
    )

    if (!opts?.force && errorSum === lastSseDebugErrorSum && maxAgeMs < 45000) return

    lastSseDebugAt = now
    lastSseDebugErrorSum = errorSum
    try {
      console.debug('[sse] stats', {
        reason: opts?.reason || '',
        visibility: document.visibilityState,
        global: globalStats,
      })
    } catch {
      // ignore
    }
  }

  function handleVisibilityEvent() {
    const state = document.visibilityState
    const visible = state === 'visible'

    if (!visible && !notifications.enabled && sse) {
      // Notifications need the live feed even while no page is visible.
      // Without permission, retain the existing hidden-tab pause behavior.
      sse.close()
      sse = null
    }

    // Only trigger a resync on hidden -> visible transitions.
    if (visible && lastVisibilityState && lastVisibilityState !== 'visible') {
      if (pendingResumeReason) {
        const reason = pendingResumeReason
        const gapMs = pendingResumeGapMs
        pendingResumeReason = null
        pendingResumeGapMs = 0
        resyncAfterResume(reason, { gapMs })
      } else {
        resyncAfterResume('visibility')
      }
    }
    lastVisibilityState = state
  }

  function connectActivity() {
    if (disposed || (!isDocumentVisible() && !notifications.enabled)) return
    sse?.close()
    sse = null

    try {
      sse = connectSse({
        endpoint: withInterfaceLocale('/api/v1/changes/stream?scope_kind=global'),
        initialLastEventId: globalSseCursor,
        debugLabel: 'sse:global',
        onCursor: (lastEventId) => {
          globalSseCursor = lastEventId
        },
        onOpen: () => {
          recovery.invalidate(0)
          notificationRecovery.invalidate(0)
        },
        onSequenceGap: () => {
          recovery.invalidate(0)
          notificationRecovery.invalidate(0)
        },
        onEvent: (evt) => {
          if (evt.type === 'lagged' || evt.type === 'subscription_closed') {
            recovery.invalidate(0)
            notificationRecovery.invalidate(0)
            return
          }
          void notifier.applyEvent(evt).catch(() => {})
          activity.applyEvent(evt)
          applyResourceEvent(evt)
          chat.applyEvent(evt)
          directory.applyGlobalEvent(evt)
          directorySessions.applyGlobalEvent(evt)
        },
        onError: (err) => {
          // Recovery needs a live connection. Retrying snapshots while the
          // stream/server is failing multiplies load and cannot close the gap.
          console.warn('[sse] connection error', err)
        },
      })
    } catch (err) {
      sse = null
      try {
        console.error('[sse] failed to start', err)
      } catch {
        // ignore
      }
    }
  }

  async function ensureSelectedSessionFromQuery() {
    const sid = readSessionIdFromQuery(route.query) || readSessionIdFromFullPath(route.fullPath)

    // If the URL includes a session id, treat it as an explicit deep link.
    if (sid && !ui.sessionQueryEnabled) {
      ui.enableSessionQuery()
    }

    if (sid) {
      if (sid !== chat.selectedSessionId) {
        await chat.selectSession(sid)
      }
      return
    }

    // In the desktop workspace, the shell route follows whichever pane has
    // focus.  A files/terminal/etc. pane has no session id, but that must not
    // clear the session selected by a still-live chat pane.  The scoped chat
    // store reads its own pane route; the global runtime only syncs a route
    // that actually represents chat.
    const isChatRoute = String(route.path || '')
      .toLowerCase()
      .startsWith('/chat')
    if (!isChatRoute) return

    ui.disableSessionQuery()

    const selected = chat.selectedSessionId
    const selectedExists = selected ? !!chat.selectedSession : false
    if (!selectedExists) {
      await chat.selectSession(null)
    }
  }

  onMounted(async () => {
    cleanupResourceSync = startResourceSync()
    cleanupKeyboard = installKeyboardInsets({ enabled: true })
    cleanupShortcuts = installKeyboardShortcuts()
    cleanupKeyboardTapFix = installKeyboardTapFix({ enabled: true })

    await Promise.all([
      health.refresh().catch(() => {}),
      settings.refresh().catch(() => {}),
      activity.refresh().catch(() => {}),
    ])

    if (disposed) return
    notifications.offerPermission()
    // Keep sessions available for all views.
    await chat.refreshSessions().catch(() => {})
    if (disposed) return
    notifier.trackSessions(
      chat.sessions
        .filter((session) => session.state?.kind === 'running' || session.state?.kind === 'awaiting_interaction')
        .map((session) => session.id),
    )
    await ensureSelectedSessionFromQuery().catch(() => {})
    if (disposed) return

    connectActivity()

    cleanupBroadcast?.()
    cleanupBroadcast = subscribeAppBroadcast((msg) => {
      if (!msg?.type) return
      if (msg.type === 'settings.updated') {
        void settings.refresh().catch(() => {})
        return
      }
    })

    if (import.meta.env.DEV && sseDebugTimer === null) {
      sseDebugTimer = window.setInterval(() => {
        if (document.visibilityState !== 'visible') return
        logSseStats({ force: false, reason: 'interval' })
      }, 20000)
    }

    lastVisibilityState = document.visibilityState
    handleVisibilityEvent()
    visibilityHandler = () => handleVisibilityEvent()
    document.addEventListener('visibilitychange', visibilityHandler)

    // BFCache: pageshow can resume without a visibilitychange transition.
    pageShowHandler = (evt) => {
      if (evt && (evt as { persisted?: boolean }).persisted) {
        resyncAfterResume('pageshow', { gapMs: 60000 })
      } else if (document.visibilityState === 'visible') {
        resyncAfterResume('pageshow')
      }
    }
    pageHideHandler = (evt) => {
      if (evt && (evt as { persisted?: boolean }).persisted) {
        sse?.close()
        sse = null
      }
    }
    window.addEventListener('pageshow', pageShowHandler)
    window.addEventListener('pagehide', pageHideHandler)

    // Page Lifecycle API (Chrome, some Chromium-based browsers).
    freezeHandler = () => {
      try {
        console.debug('[lifecycle] freeze')
      } catch {
        // ignore
      }
      sse?.close()
      sse = null
    }
    resumeHandler = () => {
      if (isDocumentVisible()) resyncAfterResume('resume', { gapMs: 60000 })
      else if (notifications.enabled) connectActivity()
    }
    document.addEventListener('freeze', freezeHandler as EventListener)
    document.addEventListener('resume', resumeHandler as EventListener)

    // Clock-jump watchdog: detects system sleep or heavy background throttling.
    lastClockWatchdogAt = Date.now()
    if (clockWatchdogTimer === null) {
      const intervalMs = 2000
      const jumpThresholdMs = 8000
      clockWatchdogTimer = window.setInterval(() => {
        const now = Date.now()
        const delta = now - lastClockWatchdogAt
        lastClockWatchdogAt = now
        if (delta < jumpThresholdMs) return
        const reason = 'clock-jump'
        if (document.visibilityState === 'visible') {
          resyncAfterResume(reason, { gapMs: delta })
        } else {
          pendingResumeReason = reason
          pendingResumeGapMs = Math.max(pendingResumeGapMs, delta)
        }
      }, intervalMs)
    }

    // Some mobile browsers don't reliably emit visibilitychange on resume; focus/online are cheap extra signals.
    focusHandler = () => {
      handleVisibilityEvent()
      if (document.visibilityState === 'visible') resyncAfterResume('focus')
    }
    blurHandler = () => {
      handleVisibilityEvent()
    }
    window.addEventListener('focus', focusHandler)
    window.addEventListener('blur', blurHandler)
    onlineHandler = () => {
      if (document.visibilityState === 'visible') resyncAfterResume('online')
      else if (notifications.enabled) connectActivity()
    }
    window.addEventListener('online', onlineHandler)
  })

  watch(
    () => notifications.enabled,
    () => {
      if (disposed || isDocumentVisible()) return
      if (notifications.enabled) {
        if (!sse) connectActivity()
      } else {
        sse?.close()
        sse = null
      }
    },
  )

  watch(
    () => ({ path: route.path, query: route.query }),
    () => {
      void ensureSelectedSessionFromQuery().catch(() => {})
    },
    { immediate: true, deep: true },
  )

  onBeforeUnmount(() => {
    disposed = true
    notifier.dispose()
    notificationRecovery.dispose()
    recovery.dispose()
    cleanupResourceSync?.()
    sse?.close()
    sse = null
    cleanupKeyboard?.()
    cleanupKeyboard = null
    cleanupShortcuts?.()
    cleanupShortcuts = null
    cleanupKeyboardTapFix?.()
    cleanupKeyboardTapFix = null
    if (sseDebugTimer !== null) {
      window.clearInterval(sseDebugTimer)
      sseDebugTimer = null
    }
    if (visibilityHandler) {
      document.removeEventListener('visibilitychange', visibilityHandler)
      visibilityHandler = null
    }
    if (focusHandler) {
      window.removeEventListener('focus', focusHandler)
      focusHandler = null
    }
    if (blurHandler) {
      window.removeEventListener('blur', blurHandler)
      blurHandler = null
    }
    if (onlineHandler) {
      window.removeEventListener('online', onlineHandler)
      onlineHandler = null
    }
    if (pageShowHandler) {
      window.removeEventListener('pageshow', pageShowHandler)
      pageShowHandler = null
    }
    if (pageHideHandler) {
      window.removeEventListener('pagehide', pageHideHandler)
      pageHideHandler = null
    }
    if (freezeHandler) {
      document.removeEventListener('freeze', freezeHandler as EventListener)
      freezeHandler = null
    }
    if (resumeHandler) {
      document.removeEventListener('resume', resumeHandler as EventListener)
      resumeHandler = null
    }
    if (clockWatchdogTimer !== null) {
      window.clearInterval(clockWatchdogTimer)
      clockWatchdogTimer = null
    }
    cleanupBroadcast?.()
    cleanupBroadcast = null
  })
}
