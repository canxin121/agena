import { computed, type Ref } from 'vue'

import { patchSettings, type RuntimeStatus } from '../lib/agenaApi'

export type SettingsAgentCard = {
  name: string
  description: string
  isDefault: boolean
  scope: 'project' | 'user' | 'bundled'
  sourcePath: string
  permissionSummary: string
  defaultSummary: string
  detailFacts: string[]
}

export type SettingsAgentsStateInput = {
  actionError: Ref<string>
  actionMessage: Ref<string>
  load: () => Promise<void>
  runtime: Ref<RuntimeStatus | null>
}

export type SettingsAgentsStateDeps = {
  patchSettings: typeof patchSettings
}

const defaultDeps: SettingsAgentsStateDeps = {
  patchSettings,
}

function readPermissionSummary(
  permission: RuntimeStatus['operator']['agents']['agents'][number]['permission'],
): string {
  const config = permission || {}
  const tools = config.entries || config.tools || {}
  const parts: string[] = []

  const toolRuleCount = Object.keys(tools.rules || {}).length
  const toolNameCount = Object.keys(tools.names || {}).length
  const toolTagCount = Object.keys(tools.tags || {}).length
  const pathRuleCount = Object.keys(config.path?.rules || {}).length
  const networkRuleCount = Object.keys(config.network?.rules || {}).length

  if (toolRuleCount || toolNameCount || toolTagCount || pathRuleCount || networkRuleCount) {
    parts.push(
      [
        toolNameCount ? `tool-names=${toolNameCount}` : null,
        toolTagCount ? `tool-tags=${toolTagCount}` : null,
        toolRuleCount ? `tool-rules=${toolRuleCount}` : null,
        pathRuleCount ? `path-rules=${pathRuleCount}` : null,
        networkRuleCount ? `network-rules=${networkRuleCount}` : null,
      ]
        .filter(Boolean)
        .join(' · '),
    )
  }

  return parts.length ? parts.join(' · ') : 'inherits runtime defaults'
}

function readDefaultSummary(agent: RuntimeStatus['operator']['agents']['agents'][number]): string {
  const defaults = agent.defaults || {}
  const parts = [
    defaults.provider ? `provider=${defaults.provider}` : null,
    defaults.adapter ? `adapter=${defaults.adapter}` : null,
    defaults.model ? `model=${defaults.model}` : null,
    defaults.thinking_mode ? `thinking=${defaults.thinking_mode}` : null,
    defaults.speed_mode ? `speed=${defaults.speed_mode}` : null,
    defaults.verbosity ? `verbosity=${defaults.verbosity}` : null,
    defaults.parallel_tool_calls != null
      ? `parallel_tools=${defaults.parallel_tool_calls ? 'on' : 'off'}`
      : null,
  ].filter(Boolean)
  return parts.length ? parts.join(' · ') : 'inherits runtime defaults'
}

function buildAgentCard(
  agent: RuntimeStatus['operator']['agents']['agents'][number],
  defaultAgent: string,
): SettingsAgentCard {
  return {
    name: agent.name,
    description: agent.description || 'No description provided.',
    isDefault: agent.name === defaultAgent,
    scope: agent.scope,
    sourcePath: agent.source_path || '',
    permissionSummary: readPermissionSummary(agent.permission),
    defaultSummary: readDefaultSummary(agent),
    detailFacts: [
      `scope=${agent.scope}`,
    ].filter(Boolean) as string[],
  }
}

export function useSettingsAgentsState(input: SettingsAgentsStateInput, deps: SettingsAgentsStateDeps = defaultDeps) {
  const summaryFacts = computed(() => {
    const agents = input.runtime.value?.operator.agents
    if (!agents) return []
    return [
      { label: 'Default Agent', value: agents.default_agent || 'n/a' },
      { label: 'Total Agents', value: String(agents.total_count) },
    ]
  })

  const agentCards = computed<SettingsAgentCard[]>(() => {
    const agents = input.runtime.value?.operator.agents
    if (!agents) return []
    return [...agents.agents]
      .sort((left, right) => left.name.localeCompare(right.name))
      .map((agent) => buildAgentCard(agent, agents.default_agent))
  })

  async function setDefaultAgent(agentName: string) {
    const trimmed = agentName.trim()
    if (!trimmed) return
    input.actionError.value = ''
    input.actionMessage.value = ''
    try {
      await deps.patchSettings({
        path: 'agents',
        changes: { default: trimmed },
        validate: true,
        reload: true,
      })
      input.actionMessage.value = `Default agent set to ${trimmed}.`
      await input.load()
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  return {
    actionError: input.actionError,
    actionMessage: input.actionMessage,
    agentCards,
    load: input.load,
    setDefaultAgent,
    summaryFacts,
  }
}
