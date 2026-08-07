import { computed, type Ref } from 'vue'

import type {
  RewindCheckpointResource,
  SessionExecutionResource,
  SessionResource,
  SessionTreeResource,
  WorkspaceResource,
} from '../lib/agenaApi'
import { formatSessionExecutionModelLabel } from '../lib/agenaApi'
import {
  chatUsageBreakdownFacts,
  chatUsageFacts,
  formatUsageCount,
  formatUsageUsd,
  summarizeChatUsage,
} from './chatUsageModel'

const CONTEXT_USAGE_BASELINE_TOKENS = 12_000
const EFFECTIVE_CONTEXT_WINDOW_PERCENT = 95

export type ChatDerivedStateInput = {
  formatEventTime: (timestampMs: number) => string
  messages: Ref<import('../lib/agenaApi').MessageResource[]>
  rewindCheckpoints: Ref<RewindCheckpointResource[]>
  selectedSessionId: Ref<number | null>
  selectedWorkspaceId: Ref<number | null>
  selectedThinkingMode: Ref<string>
  selectedSpeedMode: Ref<string>
  selectedProviderId: Ref<string>
  selectedAdapterId: Ref<string>
  selectedModelId: Ref<string>
  modelDefaultModes: () => { thinking: string; speed: string }
  sessionState: Ref<SessionExecutionResource | null>
  sessionTree: Ref<SessionTreeResource[]>
  workspaces: Ref<WorkspaceResource[]>
  sessions: Ref<SessionResource[]>
}

export function useChatDerivedState(input: ChatDerivedStateInput) {
  function formatTokenK(value: number): string {
    const normalized = Number.isFinite(value) && value > 0 ? value : 0
    if (!normalized) return '0k'
    const kValue = normalized / 1000
    if (kValue < 10) return `${kValue.toFixed(1)}k`
    return `${Math.round(kValue).toLocaleString('en-US')}k`
  }

  function contextUsagePercentUsed(currentTokens: number, contextWindowTokens: number): number {
    const effectiveWindow = Math.floor((contextWindowTokens * EFFECTIVE_CONTEXT_WINDOW_PERCENT) / 100)
    if (effectiveWindow <= CONTEXT_USAGE_BASELINE_TOKENS) return 100
    const usableWindow = effectiveWindow - CONTEXT_USAGE_BASELINE_TOKENS
    const used = Math.max(0, currentTokens - CONTEXT_USAGE_BASELINE_TOKENS)
    return Math.round(Math.max(0, Math.min(100, (used / usableWindow) * 100)))
  }

  function flattenSessionTreeRows(
    items: SessionTreeResource[],
    depth = 0,
  ): Array<{ session: SessionTreeResource; depth: number }> {
    return items.flatMap((session) => [
      { session, depth },
      ...flattenSessionTreeRows(
        input.sessionTree.value.filter((item) => item.parent_id === session.id),
        depth + 1,
      ),
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
    if (!sessionUsageSummary.value.requests) return 'No provider usage yet.'
    return `${formatUsageCount(sessionUsageSummary.value.requests)} requests · ${formatUsageCount(sessionUsageSummary.value.inputTokens + sessionUsageSummary.value.outputTokens)} visible tokens · ${formatUsageUsd(sessionUsageSummary.value.totalCostUsd)}`
  })
  const contextUsageLabel = computed(() => {
    const usage = input.sessionState.value?.usage
    if (!usage) return ''
    const currentTokens = usage.projected_tokens ?? usage.current_tokens
    const contextWindowTokens = usage.model_context_window_tokens
    if (contextWindowTokens && contextWindowTokens > 0) {
      return `context ${contextUsagePercentUsed(currentTokens, contextWindowTokens)}% used`
    }
    return `context ${formatTokenK(currentTokens)} used`
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
    return input.sessions.value.filter(
      (session) => session.parent_id === current.parent_id && session.id !== current.id,
    )
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
    const facts: string[] = []
    if (execution) {
      facts.push(`agent=${execution.agent_id}`)
      if (execution.execution_access !== 'inherit') facts.push(`access=${execution.execution_access}`)
      if (execution.task_id) facts.push(`task=${execution.task_id}`)
    }
    // Model label: prefer the active session's execution context, falling
    // back to the run-options model stack so the status stays populated
    // even before a session exists.
    const modelLabel = execution
      ? formatSessionExecutionModelLabel(execution)
      : formatSessionExecutionModelLabel({
          model_provider_id: input.selectedProviderId.value,
          model_adapter_id: input.selectedAdapterId.value,
          model_id: input.selectedModelId.value,
        })
    if (modelLabel) {
      facts.push(`model=${modelLabel}`)
    }
    // Think/speed: prefer the modes a run actually used, then run-options
    // overrides, then the resolved model defaults so they stay visible
    // before the first message of a new session.
    const defaults = input.modelDefaultModes()
    const thinkingMode = firstNonEmpty(
      execution?.model_thinking_mode || '',
      input.selectedThinkingMode.value,
      defaults.thinking,
    )
    if (thinkingMode) facts.push(`thinking=${thinkingMode}`)
    const speedMode = firstNonEmpty(execution?.model_speed_mode || '', input.selectedSpeedMode.value, defaults.speed)
    if (speedMode) facts.push(`speed=${speedMode}`)
    if (execution) {
      if (execution.model_verbosity) facts.push(`verbosity=${execution.model_verbosity}`)
      if (execution.model_parallel_tool_calls != null) {
        facts.push(`parallel_tools=${execution.model_parallel_tool_calls ? 'on' : 'off'}`)
      }
      if (execution.effective_workspace_root) facts.push(`workspace=${execution.effective_workspace_root}`)
    }
    return facts
  })

  return {
    ancestorSessions,
    childSessions,
    contextUsageLabel,
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

function firstNonEmpty(...values: string[]): string {
  return values.find((value) => value.trim().length > 0)?.trim() || ''
}
