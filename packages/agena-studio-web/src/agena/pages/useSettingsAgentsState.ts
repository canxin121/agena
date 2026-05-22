import { computed, type Ref } from 'vue'

import { patchSettings, type RuntimeStatus } from '../lib/agenaApi'

export type SettingsAgentCard = {
  name: string
  description: string
  mode: string
  hidden: boolean
  canToggleHidden: boolean
  isDefault: boolean
  scope: 'project' | 'user' | 'bundled'
  sourcePath: string
  allowedTools: string[]
  aliases: string[]
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
  const parts: string[] = []
  if (typeof config.inherit === 'boolean') {
    parts.push(`inherit=${config.inherit ? 'on' : 'off'}`)
  } else if (config.inherit && typeof config.inherit === 'object') {
    const inheritModes = Object.entries(config.inherit)
      .filter(([, value]) => Boolean(value))
      .map(([key]) => key)
    if (inheritModes.length) parts.push(`inherit=${inheritModes.join(',')}`)
  }

  const toolRuleCount = Object.keys(config.tools?.rules || {}).length
  const toolNameCount = Object.keys(config.tools?.names || {}).length
  const toolTagCount = Object.keys(config.tools?.tags || {}).length
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
  const defaults = agent.default || {}
  const parts = [
    defaults.provider ? `provider=${defaults.provider}` : null,
    defaults.adapter ? `adapter=${defaults.adapter}` : null,
    defaults.model ? `model=${defaults.model}` : null,
  ].filter(Boolean)
  return parts.length ? parts.join(' · ') : 'inherits runtime model defaults'
}

function buildAgentCard(
  agent: RuntimeStatus['operator']['agents']['agents'][number],
  defaultAgent: string,
): SettingsAgentCard {
  return {
    name: agent.name,
    description: agent.description || 'No description provided.',
    mode: agent.mode,
    hidden: agent.hidden,
    canToggleHidden: agent.scope === 'project',
    isDefault: agent.name === defaultAgent,
    scope: agent.scope,
    sourcePath: agent.source_path || '',
    allowedTools: agent.allowed_tools || [],
    aliases: agent.aliases || [],
    permissionSummary: readPermissionSummary(agent.permission),
    defaultSummary: readDefaultSummary(agent),
    detailFacts: [
      `scope=${agent.scope}`,
      `visibility=${agent.hidden ? 'hidden' : 'visible'}`,
      `mode=${agent.mode}`,
      agent.color ? `color=${agent.color}` : null,
      agent.temperature != null ? `temperature=${agent.temperature}` : null,
      agent.max_output_tokens != null ? `max_output_tokens=${agent.max_output_tokens}` : null,
      agent.steps != null ? `steps=${agent.steps}` : null,
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
      { label: 'Primary', value: String(agents.primary_count) },
      { label: 'Subagents', value: String(agents.subagent_count) },
      { label: 'Hidden', value: String(agents.hidden_count) },
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
        path: 'default',
        changes: { agent: trimmed },
        validate: true,
        reload: true,
      })
      input.actionMessage.value = `Default agent set to ${trimmed}.`
      await input.load()
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  async function toggleAgentHidden(agent: SettingsAgentCard) {
    if (!agent.canToggleHidden) {
      input.actionError.value = `Agent ${agent.name} is managed outside this config file.`
      return
    }
    input.actionError.value = ''
    input.actionMessage.value = ''
    try {
      await deps.patchSettings({
        path: 'agents',
        changes: {
          [agent.name]: { hidden: !agent.hidden },
        },
        validate: true,
        reload: true,
      })
      input.actionMessage.value = `${agent.hidden ? 'Unhid' : 'Hid'} agent ${agent.name}.`
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
    toggleAgentHidden,
  }
}
