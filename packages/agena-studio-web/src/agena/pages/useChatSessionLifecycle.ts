import { onBeforeUnmount, onMounted, watch, type Ref } from 'vue'
import type { RouteLocationNormalizedLoaded } from 'vue-router'

import {
  fetchRuntimeStatus,
  getSessionState,
  getSessionTree,
  listMessages,
  listProviderModels,
  listProviders,
  listRewindCheckpoints,
  listSessionTimeline,
  listSessions,
  listWorkspaces,
  streamSessionEvents,
  type MessageResource,
  type ProviderModel,
  type ProviderSummary,
  type RewindCheckpointResource,
  type RuntimeStatus,
  type SessionExecutionResource,
  type SessionResource,
  type SessionTreeResource,
  type TimelineEventRecord,
  type WorkspaceResource,
} from '../lib/agenaApi'
import { readChatRouteSessionId, readChatRouteSlash, readChatRouteWorkspaceId } from './chatRouteState'
import { applySessionEvent } from './chatPageModel'
import { useChatConversationRuntime } from './useChatConversationRuntime'

export type ChatSessionLifecycleInput = {
  composer: Ref<string>
  errorMessage: Ref<string>
  loading: Ref<boolean>
  localCommandNotice: Ref<string>
  messages: Ref<MessageResource[]>
  providerModels: Record<string, ProviderModel[]>
  providers: Ref<ProviderSummary[]>
  rewindCheckpoints: Ref<RewindCheckpointResource[]>
  route: RouteLocationNormalizedLoaded
  runtime: Ref<RuntimeStatus | null>
  selectedAdapterId: Ref<string>
  selectedModelId: Ref<string>
  selectedProviderId: Ref<string>
  selectedThinkingMode: Ref<string>
  selectedSpeedMode: Ref<string>
  selectedVerbosity: Ref<string>
  selectedParallelToolCalls: Ref<string>
  selectedSessionId: Ref<number | null>
  selectedWorkspaceId: Ref<number | null>
  sessionSearch: Ref<string>
  sessionState: Ref<SessionExecutionResource | null>
  sessions: Ref<SessionResource[]>
  sessionTree: Ref<SessionTreeResource[]>
  timelineEvents: Ref<TimelineEventRecord[]>
  workspaces: Ref<WorkspaceResource[]>
}

export type ChatSessionLifecycleDeps = {
  applySessionEvent: typeof applySessionEvent
  fetchRuntimeStatus: typeof fetchRuntimeStatus
  getSessionState: typeof getSessionState
  getSessionTree: typeof getSessionTree
  listMessages: typeof listMessages
  listProviderModels: typeof listProviderModels
  listProviders: typeof listProviders
  listRewindCheckpoints: typeof listRewindCheckpoints
  listSessionTimeline: typeof listSessionTimeline
  listSessions: typeof listSessions
  listWorkspaces: typeof listWorkspaces
  streamSessionEvents: typeof streamSessionEvents
}

const defaultDeps: ChatSessionLifecycleDeps = {
  applySessionEvent,
  fetchRuntimeStatus,
  getSessionState,
  getSessionTree,
  listMessages,
  listProviderModels,
  listProviders,
  listRewindCheckpoints,
  listSessionTimeline,
  listSessions,
  listWorkspaces,
  streamSessionEvents,
}

export type ChatSessionLifecycleOptions = {
  registerComponentLifecycle?: boolean
}

export function useChatSessionLifecycle(
  input: ChatSessionLifecycleInput,
  deps: ChatSessionLifecycleDeps = defaultDeps,
  options: ChatSessionLifecycleOptions = {},
) {
  const registerComponentLifecycle = options.registerComponentLifecycle !== false

  const conversationRuntime = useChatConversationRuntime(
    {
      errorMessage: input.errorMessage,
      loading: input.loading,
      messages: input.messages,
      selectedSessionId: input.selectedSessionId,
      sessionState: input.sessionState,
      timelineEvents: input.timelineEvents,
    },
    {
      applySessionEvent: deps.applySessionEvent,
      getSessionState: deps.getSessionState,
      listMessages: deps.listMessages,
      listSessionTimeline: deps.listSessionTimeline,
      streamSessionEvents: deps.streamSessionEvents,
    },
    {
      loadRewindCheckpoints,
      loadSessionTree,
    },
  )

  async function trySelectRouteSession(workspaceItems: WorkspaceResource[], routeSessionId: number): Promise<boolean> {
    for (const workspace of workspaceItems) {
      const workspaceSessions = await deps.listSessions(workspace.id, { search: input.sessionSearch.value })
      const match = workspaceSessions.find((session) => session.id === routeSessionId)
      if (!match) continue
      input.sessions.value = workspaceSessions
      input.selectedWorkspaceId.value = workspace.id
      input.selectedSessionId.value = match.id
      await refreshConversation(true)
      return true
    }
    return false
  }

  async function loadSidebar() {
    const [runtimeData, providerData, workspaceData] = await Promise.all([
      deps.fetchRuntimeStatus(),
      deps.listProviders(),
      deps.listWorkspaces(),
    ])

    input.runtime.value = runtimeData
    input.providers.value = providerData
    input.workspaces.value = workspaceData

    await Promise.all(
      providerData.map(async (provider) => {
        input.providerModels[provider.provider_id] = await deps.listProviderModels(provider.provider_id)
      }),
    )

    if (!input.selectedProviderId.value && providerData.length === 1) {
      input.selectedProviderId.value = providerData[0]?.provider_id || ''
      input.selectedAdapterId.value = providerData[0]?.default_adapter || ''
      input.selectedModelId.value = providerData[0]?.default_model || ''
    }

    const routeSlash = readChatRouteSlash(input.route.query.slash)
    if (routeSlash) {
      input.composer.value = routeSlash
      input.localCommandNotice.value = `Prepared ${routeSlash} from runtime inspector.`
    }

    const routeWorkspaceId = readChatRouteWorkspaceId(input.route.query.workspace)
    if (routeWorkspaceId !== null && workspaceData.some((workspace) => workspace.id === routeWorkspaceId)) {
      input.selectedWorkspaceId.value = routeWorkspaceId
    }

    const routeSessionId = readChatRouteSessionId(input.route.query.session)
    if (routeSessionId !== null) {
      input.selectedSessionId.value = routeSessionId
      const matched = await trySelectRouteSession(workspaceData, routeSessionId)
      if (matched) return
    }

    if (
      input.selectedWorkspaceId.value &&
      workspaceData.some((workspace) => workspace.id === input.selectedWorkspaceId.value)
    ) {
      await loadSessionsForWorkspace(input.selectedWorkspaceId.value, false)
      return
    }

    const firstWorkspace = workspaceData[0]
    if (firstWorkspace) {
      await selectWorkspace(firstWorkspace.id)
    }
  }

  async function loadSessionTree(rootId: number) {
    input.sessionTree.value = await deps.getSessionTree(rootId)
  }

  async function loadRewindCheckpoints(sessionId: number) {
    input.rewindCheckpoints.value = await deps.listRewindCheckpoints(sessionId)
  }

  async function loadSessionsForWorkspace(workspaceId: number, preserveSelection = true) {
    input.sessions.value = await deps.listSessions(workspaceId, { search: input.sessionSearch.value })
    input.selectedWorkspaceId.value = workspaceId

    const currentSelectionStillExists =
      preserveSelection &&
      input.selectedSessionId.value !== null &&
      input.sessions.value.some((session) => session.id === input.selectedSessionId.value)

    if (currentSelectionStillExists && input.selectedSessionId.value !== null) {
      await refreshConversation(true)
      return
    }

    const routeSessionId = readChatRouteSessionId(input.route.query.session)
    const routeSession =
      routeSessionId !== null ? input.sessions.value.find((session) => session.id === routeSessionId) : null
    if (routeSession) {
      input.selectedSessionId.value = routeSession.id
      await refreshConversation(true)
      return
    }

    const firstSession = input.sessions.value[0]
    if (firstSession) {
      input.selectedSessionId.value = firstSession.id
      await refreshConversation(true)
      return
    }

    input.selectedSessionId.value = null
    input.messages.value = []
    input.timelineEvents.value = []
    input.sessionState.value = null
    stopEventStream()
    clearScheduledConversationRefresh()
    stopPolling()
  }

  async function selectWorkspace(workspaceId: number) {
    await loadSessionsForWorkspace(workspaceId, false)
  }

  async function selectSession(sessionId: number) {
    stopEventStream()
    clearScheduledConversationRefresh()
    input.selectedSessionId.value = sessionId
    await refreshConversation(true)
  }

  async function openSessionById(sessionId: number): Promise<boolean> {
    if (!Number.isFinite(sessionId)) return false
    if (input.sessions.value.some((session) => session.id === sessionId)) {
      await selectSession(sessionId)
      return true
    }
    return await trySelectRouteSession(input.workspaces.value, sessionId)
  }

  const {
    clearScheduledConversationRefresh,
    dispose,
    refreshConversation,
    stopEventStream,
    stopPolling,
    syncEventStream,
  } = conversationRuntime

  watch(
    () => input.route.query.session,
    (value) => {
      const routeSessionId = readChatRouteSessionId(value)
      if (routeSessionId === null || routeSessionId === input.selectedSessionId.value) return
      input.selectedSessionId.value = routeSessionId
      void loadSidebar().catch((err) => {
        input.errorMessage.value = err instanceof Error ? err.message : String(err)
      })
    },
  )

  watch(
    () => input.route.query.workspace,
    (value) => {
      const routeWorkspaceId = readChatRouteWorkspaceId(value)
      if (routeWorkspaceId === null || routeWorkspaceId === input.selectedWorkspaceId.value) return
      input.selectedWorkspaceId.value = routeWorkspaceId
      void loadSidebar().catch((err) => {
        input.errorMessage.value = err instanceof Error ? err.message : String(err)
      })
    },
  )

  watch(
    () => input.route.query.slash,
    (value) => {
      const routeSlash = readChatRouteSlash(value)
      if (!routeSlash) return
      input.composer.value = routeSlash
      input.localCommandNotice.value = `Prepared ${routeSlash} from runtime inspector.`
    },
  )

  if (registerComponentLifecycle) {
    onMounted(() => {
      void loadSidebar().catch((err) => {
        input.errorMessage.value = err instanceof Error ? err.message : String(err)
      })
    })

    onBeforeUnmount(() => {
      dispose()
    })
  }

  return {
    loadRewindCheckpoints,
    loadSessionsForWorkspace,
    loadSessionTree,
    loadSidebar,
    openSessionById,
    refreshConversation,
    selectSession,
    selectWorkspace,
    syncEventStream,
  }
}
