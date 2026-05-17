import { computed, type Ref } from 'vue'

import type { RewindCheckpointResource, SessionExecutionResource, SessionResource, SessionTreeResource, WorkspaceResource } from '../lib/agenaApi'
import { chatUsageBreakdownFacts, chatUsageFacts, formatUsageCount, formatUsageUsd, summarizeChatUsage } from './chatUsageModel'

export type ChatDerivedStateInput = {
  formatEventTime: (timestampMs: number) => string
  messages: Ref<import('../lib/agenaApi').MessageResource[]>
  rewindCheckpoints: Ref<RewindCheckpointResource[]>
  selectedSessionId: Ref<number | null>
  selectedWorkspaceId: Ref<number | null>
  sessionState: Ref<SessionExecutionResource | null>
  sessionTree: Ref<SessionTreeResource[]>
  workspaces: Ref<WorkspaceResource[]>
  sessions: Ref<SessionResource[]>
}

export function useChatDerivedState(input: ChatDerivedStateInput) {
  function flattenSessionTreeRows(items: SessionTreeResource[], depth = 0): Array<{ session: SessionTreeResource; depth: number }> {
    return items.flatMap((session) => [
      { session, depth },
      ...flattenSessionTreeRows(input.sessionTree.value.filter((item) => item.parent_id === session.id), depth + 1),
    ])
  }

  const selectedWorkspace = computed(
    () => input.workspaces.value.find((workspace) => workspace.id === input.selectedWorkspaceId.value) || null,
  )

  const selectedSession = computed(
    () => input.sessions.value.find((session) => session.id === input.selectedSessionId.value) || null,
  )

  const sessionUsageSummary = computed(() => summarizeChatUsage(input.messages.value))
  const sessionUsageSummaryFacts = computed(() => chatUsageFacts(sessionUsageSummary.value))
  const sessionTreeRows = computed(() => {
    const rootSessions = input.sessionTree.value.filter((session) => !session.parent_id)
    return flattenSessionTreeRows(rootSessions)
  })
  const rewindCheckpointFacts = computed(() =>
    input.rewindCheckpoints.value.map((checkpoint) => ({
      key: `${checkpoint.target_message_id}-${checkpoint.at_ms}`,
      label: `message #${checkpoint.target_message_id}`,
      messageId: checkpoint.target_message_id,
      summary: `${input.formatEventTime(checkpoint.at_ms)} · dropped ${formatUsageCount(checkpoint.dropped.length)} message(s)`,
    })),
  )
  const sessionUsageHeadline = computed(() => {
    if (!sessionUsageSummary.value.turns) return 'No assistant usage yet.'
    return `${formatUsageCount(sessionUsageSummary.value.turns)} turns · ${formatUsageCount(sessionUsageSummary.value.inputTokens + sessionUsageSummary.value.outputTokens)} visible tokens · ${formatUsageUsd(sessionUsageSummary.value.totalCostUsd)}`
  })
  const sessionUsageModelLines = computed(() =>
    sessionUsageSummary.value.byModel.map((item) => ({
      key: `${item.providerId}/${item.modelId}`,
      label: `${item.providerId}/${item.modelId}`,
      facts: chatUsageBreakdownFacts(item),
    })),
  )

  const sessionsById = computed(() => {
    const map = new Map<number, SessionResource>()
    for (const session of input.sessions.value) {
      map.set(session.id, session)
    }
    return map
  })

  const parentSession = computed(() => {
    const parentId = input.sessionState.value?.session.parent_id ?? selectedSession.value?.parent_id ?? null
    return parentId ? sessionsById.value.get(parentId) || null : null
  })

  const childSessions = computed(() => {
    const sessionId = input.sessionState.value?.session.id ?? selectedSession.value?.id ?? null
    if (!sessionId) return [] as SessionResource[]
    return input.sessions.value.filter((session) => session.parent_id === sessionId)
  })

  const ancestorSessions = computed(() => {
    const items: SessionResource[] = []
    let current = parentSession.value
    while (current) {
      items.unshift(current)
      current = current.parent_id ? sessionsById.value.get(current.parent_id) || null : null
    }
    return items
  })

  const siblingSessions = computed(() => {
    const current = selectedSession.value
    if (!current?.parent_id) return [] as SessionResource[]
    return input.sessions.value.filter((session) => session.parent_id === current.parent_id && session.id !== current.id)
  })

  const sessionLineageLabel = computed(() => {
    const session = input.sessionState.value?.session || selectedSession.value
    if (!session) return ''
    const rootLabel = ancestorSessions.value.length ? `root=#${ancestorSessions.value[0]?.id}` : 'root'
    const parent = session.parent_id ? `parent=#${session.parent_id}` : 'parent=none'
    const siblings = `siblings=${siblingSessions.value.length}`
    const children = `children=${childSessions.value.length}`
    return `${rootLabel} · ${parent} · ${siblings} · ${children}`
  })

  const executionFacts = computed(() => {
    const execution = input.sessionState.value?.execution
    if (!execution) return [] as string[]

    const facts: string[] = []
    if (execution.agent_profile) facts.push(`agent=${execution.agent_profile}`)
    if (execution.active_skill_name) facts.push(`skill=${execution.active_skill_name}`)
    if (execution.task_id) facts.push(`task=${execution.task_id}`)
    if (execution.model_provider_id || execution.model_id) {
      facts.push(`model=${[execution.model_provider_id, execution.model_id].filter(Boolean).join('/')}`)
    }
    if (execution.model_thinking_mode) facts.push(`thinking=${execution.model_thinking_mode}`)
    if (execution.model_speed_mode) facts.push(`speed=${execution.model_speed_mode}`)
    if (execution.model_verbosity) facts.push(`verbosity=${execution.model_verbosity}`)
    if (execution.effective_workspace_root) facts.push(`workspace=${execution.effective_workspace_root}`)
    if (execution.allowed_tools.length) facts.push(`allowed_tools=${execution.allowed_tools.length}`)
    return facts
  })

  return {
    ancestorSessions,
    childSessions,
    executionFacts,
    parentSession,
    rewindCheckpointFacts,
    selectedSession,
    selectedWorkspace,
    sessionLineageLabel,
    sessionTreeRows,
    sessionUsageHeadline,
    sessionUsageModelLines,
    sessionUsageSummary,
    sessionUsageSummaryFacts,
    siblingSessions,
  }
}
