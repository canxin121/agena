import { computed, reactive, ref, type Ref } from 'vue'

import { getSettings, patchSettings, setSettings, type RuntimeStatus } from '../lib/agenaApi'

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
  getSettings: typeof getSettings
  patchSettings: typeof patchSettings
  setSettings: typeof setSettings
}

const defaultDeps: SettingsAgentsStateDeps = {
  getSettings,
  patchSettings,
  setSettings,
}

function readRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {}
}

function quotedSettingsSegment(value: string): string {
  return `"${value.replaceAll('\\', '\\\\').replaceAll('"', '\\"')}"`
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
    defaults.parallel_tool_calls != null ? `parallel_tools=${defaults.parallel_tool_calls ? 'on' : 'off'}` : null,
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
    detailFacts: [`scope=${agent.scope}`].filter(Boolean) as string[],
  }
}

export function useSettingsAgentsState(input: SettingsAgentsStateInput, deps: SettingsAgentsStateDeps = defaultDeps) {
  const configAgents = ref<Record<string, unknown>>({})
  const newAgentName = ref('')
  const agentSaving = ref(false)
  const editor = reactive({
    open: false,
    name: '',
    description: '',
    prompt: '',
    provider: '',
    adapter: '',
    model: '',
    permissionJson: '',
    raw: {} as Record<string, unknown>,
  })
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

  async function loadConfigAgents() {
    try {
      const response = await deps.getSettings({ path: 'agents', source: 'file' })
      const root = readRecord(response.value)
      configAgents.value = Object.fromEntries(Object.entries(root).filter(([name]) => name !== 'default'))
    } catch (error) {
      input.actionError.value = error instanceof Error ? error.message : String(error)
    }
  }

  function isConfigAgent(name: string): boolean {
    return Object.prototype.hasOwnProperty.call(configAgents.value, name)
  }

  async function openAgentEditor(name: string) {
    input.actionError.value = ''
    input.actionMessage.value = ''
    try {
      const response = await deps.getSettings({ path: `agents.${quotedSettingsSegment(name)}`, source: 'file' })
      if (!response.value || typeof response.value !== 'object' || Array.isArray(response.value)) {
        throw new Error(`Agent ${name} is not stored in the editable config file.`)
      }
      const raw = readRecord(response.value)
      const defaults = readRecord(raw.defaults)
      editor.open = true
      editor.name = name
      editor.description = typeof raw.description === 'string' ? raw.description : ''
      editor.prompt = typeof raw.prompt === 'string' ? raw.prompt : ''
      editor.provider = typeof defaults.provider === 'string' ? defaults.provider : ''
      editor.adapter = typeof defaults.adapter === 'string' ? defaults.adapter : ''
      editor.model = typeof defaults.model === 'string' ? defaults.model : ''
      editor.permissionJson = raw.permission === undefined ? '' : JSON.stringify(raw.permission, null, 2)
      editor.raw = raw
    } catch (error) {
      input.actionError.value = error instanceof Error ? error.message : String(error)
    }
  }

  function closeAgentEditor() {
    editor.open = false
    editor.name = ''
    editor.raw = {}
  }

  async function createConfigAgent() {
    const name = newAgentName.value.trim()
    if (!name) return
    if (agentCards.value.some((agent) => agent.name === name)) {
      input.actionError.value = `Agent ${name} already exists.`
      return
    }
    agentSaving.value = true
    input.actionError.value = ''
    input.actionMessage.value = ''
    try {
      await deps.setSettings({
        path: `agents.${quotedSettingsSegment(name)}`,
        value: {},
        validate: true,
        reload: true,
      })
      newAgentName.value = ''
      input.actionMessage.value = `Created configurable agent ${name}.`
      await Promise.all([input.load(), loadConfigAgents()])
      await openAgentEditor(name)
    } catch (error) {
      input.actionError.value = error instanceof Error ? error.message : String(error)
    } finally {
      agentSaving.value = false
    }
  }

  async function saveAgentEditor() {
    if (!editor.open || !editor.name) return
    agentSaving.value = true
    input.actionError.value = ''
    input.actionMessage.value = ''
    try {
      const value = { ...editor.raw }
      if (editor.description.trim()) value.description = editor.description
      else delete value.description
      if (editor.prompt.trim()) value.prompt = editor.prompt
      else delete value.prompt

      const previousDefaults = readRecord(value.defaults)
      const defaults: Record<string, unknown> = { ...previousDefaults }
      for (const [key, fieldValue] of [
        ['provider', editor.provider],
        ['adapter', editor.adapter],
        ['model', editor.model],
      ] as const) {
        if (fieldValue.trim()) defaults[key] = fieldValue.trim()
        else delete defaults[key]
      }
      if (Object.keys(defaults).length) value.defaults = defaults
      else delete value.defaults

      if (editor.permissionJson.trim()) value.permission = JSON.parse(editor.permissionJson)
      else delete value.permission

      await deps.setSettings({
        path: `agents.${quotedSettingsSegment(editor.name)}`,
        value,
        validate: true,
        reload: true,
      })
      input.actionMessage.value = `Saved agent ${editor.name}.`
      editor.raw = value
      await Promise.all([input.load(), loadConfigAgents()])
    } catch (error) {
      input.actionError.value = error instanceof Error ? error.message : String(error)
    } finally {
      agentSaving.value = false
    }
  }

  return {
    actionError: input.actionError,
    actionMessage: input.actionMessage,
    agentSaving,
    agentCards,
    closeAgentEditor,
    configAgents,
    createConfigAgent,
    editor,
    isConfigAgent,
    load: input.load,
    loadConfigAgents,
    newAgentName,
    openAgentEditor,
    saveAgentEditor,
    setDefaultAgent,
    summaryFacts,
  }
}
