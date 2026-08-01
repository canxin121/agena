import { userErrorMessage } from '@/lib/api'
import { nextTick, onBeforeUnmount, onMounted, watch, type Ref } from 'vue'
import type { RouteLocationNormalizedLoaded } from 'vue-router'

import {
  type DomainEventRecord,
  fetchRuntimeStatus,
  getSessionState,
  getSessionTree,
  listPluginToolRegistryChanges,
  listProviders,
  listRewindCheckpoints,
  listSessionTimeline,
  listSessions,
  listWorkspaces,
  streamPluginToolRegistryChanges,
  streamSessionEvents,
  type MessageResource,
  type ProviderModel,
  type ProviderSummary,
  type RewindCheckpointResource,
  type RuntimeStatus,
  type SessionExecutionResource,
  type SessionResource,
  type SessionTreeResource,
  type WorkspaceResource,
} from '../lib/agenaApi'
import { usePluginToolRegistryRuntimeSync } from '../lib/usePluginToolRegistryRuntimeSync'
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
  selectedTemperature: Ref<string>
  selectedMaxOutput: Ref<string>
  selectedSystemPrompt: Ref<string>
  selectedSessionId: Ref<number | null>
  selectedWorkspaceId: Ref<number | null>
  sessionSearch: Ref<string>
  sessionViewMode: Ref<'all' | 'roots' | 'subtree'>
  sessionState: Ref<SessionExecutionResource | null>
  sessions: Ref<SessionResource[]>
  sessionTree: Ref<SessionTreeResource[]>
  timelineEvents: Ref<DomainEventRecord[]>
  workspaces: Ref<WorkspaceResource[]>
}

export type ChatSessionLifecycleDeps = {
  applySessionEvent: typeof applySessionEvent
  fetchRuntimeStatus: typeof fetchRuntimeStatus
  getSessionState: typeof getSessionState
  getSessionTree: typeof getSessionTree
  listPluginToolRegistryChanges: typeof listPluginToolRegistryChanges
  listProviders: typeof listProviders
  listRewindCheckpoints: typeof listRewindCheckpoints
  listSessionTimeline: typeof listSessionTimeline
  listSessions: typeof listSessions
  listWorkspaces: typeof listWorkspaces
  streamPluginToolRegistryChanges: typeof streamPluginToolRegistryChanges
  streamSessionEvents: typeof streamSessionEvents
}

const defaultDeps: ChatSessionLifecycleDeps = {
  applySessionEvent,
  fetchRuntimeStatus,
  getSessionState,
  getSessionTree,
  listPluginToolRegistryChanges,
  listProviders,
  listRewindCheckpoints,
  listSessionTimeline,
  listSessions,
  listWorkspaces,
  streamPluginToolRegistryChanges,
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
  usePluginToolRegistryRuntimeSync(
    {
      runtime: input.runtime,
    },
    {
      fetchRuntimeStatus: deps.fetchRuntimeStatus,
      listPluginToolRegistryChanges: deps.listPluginToolRegistryChanges,
      streamPluginToolRegistryChanges: deps.streamPluginToolRegistryChanges,
    },
    {
      registerComponentLifecycle,
      onError: (error) => {
        console.warn('chat plugin tool registry sync failed', error)
      },
    },
  )

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
      listSessionTimeline: deps.listSessionTimeline,
      streamSessionEvents: deps.streamSessionEvents,
    },
    {
      loadRewindCheckpoints,
      loadSessionTree,
    },
  )

  async function syncRunOptionsFromSelectedSession() {
    const execution = input.sessionState.value?.execution
    if (!execution) return
    input.selectedProviderId.value = execution.model_provider_id?.trim() || ''
    input.selectedAdapterId.value = execution.model_adapter_id?.trim() || ''
    input.selectedModelId.value = execution.model_id?.trim() || ''
    input.selectedTemperature.value = ''
    input.selectedMaxOutput.value = ''
    // Agena's fixed identity and project instructions are composed by the runtime.
    // This field is only a one-run caller addition; copying runtime-owned system
    // instructions into it would submit the same content twice.
    input.selectedSystemPrompt.value = ''

    // Provider/model watchers intentionally clear dependent choices. Apply
    // persisted choices on the next tick so selecting a session wins.
    await nextTick()
    input.selectedThinkingMode.value = execution.model_thinking_mode?.trim() || ''
    input.selectedSpeedMode.value = execution.model_speed_mode?.trim() || ''
    input.selectedVerbosity.value = execution.model_verbosity?.trim() || ''
    input.selectedParallelToolCalls.value =
      execution.model_parallel_tool_calls == null ? '' : String(execution.model_parallel_tool_calls)
  }

  async function refreshSelectedConversation() {
    await refreshConversation(true)
    await syncRunOptionsFromSelectedSession()
  }

  async function trySelectRouteSession(workspaceItems: WorkspaceResource[], routeSessionId: number): Promise<boolean> {
    for (const workspace of workspaceItems) {
      const workspaceSessions = await deps.listSessions(workspace.id, { search: input.sessionSearch.value })
      const match = workspaceSessions.find((session) => session.id === routeSessionId)
      if (!match) continue
      input.sessions.value = workspaceSessions
      input.selectedWorkspaceId.value = workspace.id
      input.selectedSessionId.value = match.id
      await refreshSelectedConversation()
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

    if (!input.selectedProviderId.value && providerData.length === 1) {
      input.selectedProviderId.value = providerData[0]?.provider_id || ''
      input.selectedAdapterId.value = providerData[0]?.defaults.adapter || ''
      input.selectedModelId.value = providerData[0]?.defaults.model || ''
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
    if (input.sessionViewMode.value === 'subtree' && input.selectedSessionId.value) {
      const tree = await deps.getSessionTree(input.selectedSessionId.value)
      const query = input.sessionSearch.value.trim().toLowerCase()
      input.sessions.value = query
        ? tree.filter((session) => [session.title, String(session.id)].join(' ').toLowerCase().includes(query))
        : tree
    } else {
      input.sessions.value = await deps.listSessions(workspaceId, {
        search: input.sessionSearch.value,
        roots: input.sessionViewMode.value === 'roots',
      })
    }
    input.selectedWorkspaceId.value = workspaceId

    const currentSelectionStillExists =
      preserveSelection &&
      input.selectedSessionId.value !== null &&
      input.sessions.value.some((session) => session.id === input.selectedSessionId.value)

    if (currentSelectionStillExists && input.selectedSessionId.value !== null) {
      await refreshSelectedConversation()
      return
    }

    const routeSessionId = readChatRouteSessionId(input.route.query.session)
    const routeSession =
      routeSessionId !== null ? input.sessions.value.find((session) => session.id === routeSessionId) : null
    if (routeSession) {
      input.selectedSessionId.value = routeSession.id
      await refreshSelectedConversation()
      return
    }

    clearConversationSelection()
  }

  async function selectWorkspace(workspaceId: number) {
    await loadSessionsForWorkspace(workspaceId, false)
  }

  async function setSessionViewMode(mode: 'all' | 'roots' | 'subtree', query = '') {
    if (mode === 'subtree' && !input.selectedSessionId.value) {
      input.localCommandNotice.value = 'Select a session before using subtree view.'
      return
    }
    input.sessionViewMode.value = mode
    input.sessionSearch.value = query
    const workspaceId = input.selectedWorkspaceId.value
    if (workspaceId) await loadSessionsForWorkspace(workspaceId, true)
  }

  async function loadSessionTimeline(limit = 100) {
    const sessionId = input.selectedSessionId.value
    if (!sessionId) return
    input.loading.value = true
    input.errorMessage.value = ''
    try {
      input.timelineEvents.value = await deps.listSessionTimeline(sessionId, { limit })
    } catch (error) {
      input.errorMessage.value = userErrorMessage(error)
    } finally {
      input.loading.value = false
    }
  }

  async function selectSession(sessionId: number) {
    stopEventStream()
    clearScheduledConversationRefresh()
    input.selectedSessionId.value = sessionId
    await refreshSelectedConversation()
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

  function clearConversationSelection() {
    input.selectedSessionId.value = null
    input.messages.value = []
    input.timelineEvents.value = []
    input.sessionState.value = null
    input.sessionTree.value = []
    input.rewindCheckpoints.value = []
    stopEventStream()
    clearScheduledConversationRefresh()
    stopPolling()
  }

  watch(
    () => input.route.query.session,
    (value) => {
      const routeSessionId = readChatRouteSessionId(value)
      if (routeSessionId === null) {
        if (input.selectedSessionId.value !== null) clearConversationSelection()
        return
      }
      if (routeSessionId === input.selectedSessionId.value) return
      input.selectedSessionId.value = routeSessionId
      void loadSidebar().catch((err) => {
        input.errorMessage.value = userErrorMessage(err)
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
        input.errorMessage.value = userErrorMessage(err)
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
        input.errorMessage.value = userErrorMessage(err)
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
    loadSessionTimeline,
    loadSidebar,
    openSessionById,
    refreshConversation,
    selectSession,
    selectWorkspace,
    setSessionViewMode,
    syncEventStream,
  }
}
