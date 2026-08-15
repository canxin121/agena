import { userErrorMessage } from '@/lib/api'
import type { Ref } from 'vue'

import {
  fetchSessionExecution,
  streamSessionChanges,
  type SessionChange,
  type SessionChangeStreamHandle,
  type SessionExecutionResource,
  type SessionPart,
} from '../lib/agenaApi'
import { applySessionChange } from './chatPageModel'
import type { NotificationsHandle } from '../lib/notifications/types'

export type ChatConversationRuntimeInput = {
  notify: NotificationsHandle
  loading: Ref<boolean>
  parts: Ref<SessionPart[]>
  selectedSessionId: Ref<number | null>
  sessionState: Ref<SessionExecutionResource | null>
}

export type ChatConversationRuntimeDeps = {
  applySessionChange: typeof applySessionChange
  fetchSessionExecution: typeof fetchSessionExecution
  streamSessionChanges: typeof streamSessionChanges
}

export type ChatConversationRuntimeOptions = {
  loadRewindCheckpoints: (sessionId: number) => Promise<void>
  loadSessionTree: (rootId: number) => Promise<void>
}

function isRunTerminalTransition(change: SessionChange): boolean {
  if (change.type !== 'PartAdded' && change.type !== 'PartUpdated') return false
  const state = change.part.state
  return change.part.kind === 'run' && state !== 'pending' && state !== 'in_progress'
}

export function useChatConversationRuntime(
  input: ChatConversationRuntimeInput,
  deps: ChatConversationRuntimeDeps,
  options: ChatConversationRuntimeOptions,
) {
  let pollTimer: ReturnType<typeof setInterval> | null = null
  let refreshTimer: ReturnType<typeof setTimeout> | null = null
  let refreshInFlight = false
  let refreshQueued = false
  let changeStream: SessionChangeStreamHandle | null = null

  function stopChangeStream() {
    changeStream?.close()
    changeStream = null
  }

  function clearScheduledConversationRefresh() {
    refreshQueued = false
    if (!refreshTimer) return
    clearTimeout(refreshTimer)
    refreshTimer = null
  }

  function stopPolling() {
    if (!pollTimer) return
    clearInterval(pollTimer)
    pollTimer = null
  }

  function ensurePolling() {
    if (pollTimer || !input.selectedSessionId.value) return
    pollTimer = setInterval(() => {
      void refreshConversation(false)
    }, 1800)
  }

  function syncPolling() {
    if (changeStream) {
      stopPolling()
      return
    }

    if (!input.sessionState.value) {
      stopPolling()
      return
    }

    if (input.sessionState.value.workflow_state === 'blocked' || input.sessionState.value.active_execution) {
      ensurePolling()
      return
    }

    stopPolling()
  }

  function scheduleConversationRefresh(delayMs = 120) {
    if (!input.selectedSessionId.value || refreshTimer) return
    refreshTimer = setTimeout(() => {
      refreshTimer = null
      void refreshConversation(false)
    }, delayMs)
  }

  function applyChatSessionChange(change: SessionChange): boolean {
    const result = deps.applySessionChange(
      {
        parts: input.parts.value,
        sessionState: input.sessionState.value,
        selectedSessionId: input.selectedSessionId.value,
      },
      change,
    )
    input.parts.value = result.state.parts
    input.sessionState.value = result.state.sessionState
    return result.shouldRefresh
  }

  function syncChangeStream() {
    const sessionId = input.selectedSessionId.value
    if (!sessionId) {
      stopChangeStream()
      stopPolling()
      return
    }

    if (typeof ReadableStream === 'undefined' || typeof TextDecoder === 'undefined') {
      stopChangeStream()
      syncPolling()
      return
    }

    if (changeStream) {
      return
    }

    changeStream = deps.streamSessionChanges(sessionId, {
      onOpen: () => {
        stopPolling()
        // Re-read once after every connection. This closes the small window
        // between the state snapshot and the server-side subscription, so
        // patches issued in that interval are reconciled by the next read.
        scheduleConversationRefresh(0)
      },
      onChange: (change) => {
        if (input.selectedSessionId.value !== sessionId) return
        const shouldRefresh = applyChatSessionChange(change) || isRunTerminalTransition(change)
        if (shouldRefresh) {
          scheduleConversationRefresh()
        }
      },
      onInvalidate: () => {
        if (input.selectedSessionId.value !== sessionId) return
        scheduleConversationRefresh(0)
      },
      onError: (error) => {
        if (input.selectedSessionId.value !== sessionId) return
        console.warn('session change stream failed', error)
      },
    })
  }

  async function refreshConversation(foreground: boolean) {
    const sessionId = input.selectedSessionId.value
    if (!sessionId) return

    if (refreshInFlight) {
      refreshQueued = true
      return
    }

    if (foreground) {
      input.loading.value = true
    }
    refreshInFlight = true

    try {
      const state = await deps.fetchSessionExecution(sessionId)
      if (input.selectedSessionId.value !== sessionId) return

      input.sessionState.value = state
      input.parts.value = state.parts ?? []
      const rootId = state.session.parent_id ? state.session.parent_id : state.session.id
      await Promise.all([options.loadSessionTree(rootId), options.loadRewindCheckpoints(sessionId)])
      syncChangeStream()
      syncPolling()
    } catch (err) {
      if (input.selectedSessionId.value !== sessionId) return
      input.notify.error(userErrorMessage(err))
      stopPolling()
    } finally {
      refreshInFlight = false
      if (refreshQueued && input.selectedSessionId.value === sessionId) {
        refreshQueued = false
        scheduleConversationRefresh(0)
      }
      if (foreground) {
        input.loading.value = false
      }
    }
  }

  function dispose() {
    stopChangeStream()
    stopPolling()
    clearScheduledConversationRefresh()
  }

  return {
    clearScheduledConversationRefresh,
    dispose,
    refreshConversation,
    stopChangeStream,
    stopPolling,
    syncChangeStream,
    syncPolling,
  }
}
