import { userErrorMessage } from '@/lib/api'
import type { Ref } from 'vue'

import {
  getSessionState,
  listSessionTimeline,
  streamSessionEvents,
  type DomainEventRecord,
  type MessageResource,
  type SessionEventStreamHandle,
  type SessionExecutionResource,
} from '../lib/agenaApi'
import { applySessionEvent, type ChatEventState } from './chatPageModel'
import { transcriptMessages } from './chatRenderModel'

export type ChatConversationRuntimeInput = {
  errorMessage: Ref<string>
  loading: Ref<boolean>
  messages: Ref<MessageResource[]>
  selectedSessionId: Ref<number | null>
  sessionState: Ref<SessionExecutionResource | null>
  timelineEvents: Ref<DomainEventRecord[]>
}

export type ChatConversationRuntimeDeps = {
  applySessionEvent: typeof applySessionEvent
  getSessionState: typeof getSessionState
  listSessionTimeline: typeof listSessionTimeline
  streamSessionEvents: typeof streamSessionEvents
}

export type ChatConversationRuntimeOptions = {
  loadRewindCheckpoints: (sessionId: number) => Promise<void>
  loadSessionTree: (rootId: number) => Promise<void>
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
  let eventStream: SessionEventStreamHandle | null = null

  function stopEventStream() {
    eventStream?.close()
    eventStream = null
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
    if (eventStream) {
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

  function applyChatSessionEvent(event: DomainEventRecord): boolean {
    const result = deps.applySessionEvent(
      {
        messages: input.messages.value,
        timelineEvents: input.timelineEvents.value,
        sessionState: input.sessionState.value,
        selectedSessionId: input.selectedSessionId.value,
      } satisfies ChatEventState,
      event,
    )
    input.messages.value = result.state.messages
    input.timelineEvents.value = result.state.timelineEvents
    input.sessionState.value = result.state.sessionState
    return result.shouldRefresh
  }

  function syncEventStream() {
    const sessionId = input.selectedSessionId.value
    if (!sessionId) {
      stopEventStream()
      stopPolling()
      return
    }

    if (typeof ReadableStream === 'undefined' || typeof TextDecoder === 'undefined') {
      stopEventStream()
      syncPolling()
      return
    }

    if (eventStream) {
      return
    }

    eventStream = deps.streamSessionEvents(sessionId, {
      afterSeq: input.sessionState.value?.latest_event_seq ?? 0,
      pollIntervalMs: 250,
      onOpen: () => {
        stopPolling()
        // Re-read once after every connection. This closes the small window
        // between the state snapshot and the server-side event subscription,
        // including descendant requests raised in that interval.
        scheduleConversationRefresh(0)
      },
      onDescendantEvent: () => scheduleConversationRefresh(0),
      onLagged: () => scheduleConversationRefresh(0),
      onEvent: (event) => {
        if (input.selectedSessionId.value !== sessionId) return
        if (input.sessionState.value) {
          input.sessionState.value = {
            ...input.sessionState.value,
            latest_event_seq: Math.max(input.sessionState.value.latest_event_seq ?? 0, event.seq_global),
          }
        }
        if (applyChatSessionEvent(event)) {
          scheduleConversationRefresh()
        }
      },
      onError: (error) => {
        if (input.selectedSessionId.value !== sessionId) return
        console.warn('session event stream failed', error)
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
      // Read the registry-backed execution state first. Once it reports no
      // active execution, the server has already persisted ExecutionFinished
      // and synchronously advanced the transcript projection, so the
      // subsequent message read cannot return an older open assistant.
      const state = await deps.getSessionState(sessionId)
      const eventItems = await deps.listSessionTimeline(sessionId, { limit: 100 })
      if (input.selectedSessionId.value !== sessionId) return

      // Never overwrite an event-reduced state with a response whose event
      // fence is older. Queueing another read also refreshes messages that may
      // have raced the same terminal event.
      const localEventSeq = input.sessionState.value?.latest_event_seq ?? 0
      const fetchedEventSeq = state.latest_event_seq ?? 0
      if (localEventSeq > fetchedEventSeq) {
        refreshQueued = true
        return
      }
      const locallyTerminalAtSameFence =
        localEventSeq === fetchedEventSeq &&
        input.sessionState.value !== null &&
        input.sessionState.value.active_execution === null &&
        state.active_execution !== null
      input.sessionState.value = locallyTerminalAtSameFence ? { ...state, active_execution: null } : state
      input.messages.value = transcriptMessages(state.transcript)
      input.timelineEvents.value = eventItems
      const rootId = state.session.parent_id ? state.session.parent_id : state.session.id
      await Promise.all([options.loadSessionTree(rootId), options.loadRewindCheckpoints(sessionId)])
      syncEventStream()
      syncPolling()
    } catch (err) {
      if (input.selectedSessionId.value !== sessionId) return
      input.errorMessage.value = userErrorMessage(err)
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
    stopEventStream()
    stopPolling()
    clearScheduledConversationRefresh()
  }

  return {
    clearScheduledConversationRefresh,
    dispose,
    refreshConversation,
    stopEventStream,
    stopPolling,
    syncEventStream,
    syncPolling,
  }
}
