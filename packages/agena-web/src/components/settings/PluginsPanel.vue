<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { RiCommandLine, RiExternalLinkLine, RiPlayLine, RiRefreshLine, RiShieldCheckLine } from '@remixicon/vue'

import MarkdownRenderer from '@/components/markdown/MarkdownRenderer.vue'
import PluginConfigEditor from '@/components/settings/plugins/PluginConfigEditor.vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import { apiJson } from '@/lib/api'
import { defaultValueForSchema, isJsonRecord } from '@/components/settings/plugins/pluginConfigSchema'
import { useChatStore } from '@/stores/chat'
import { useToastsStore } from '@/stores/toasts'
import type { JsonValue } from '@/types/json'

type PluginStatus = {
  plugin_id: string
  kind: string
  state: 'running' | 'restarting' | 'failed' | 'stopped' | string
  pid?: number | null
  restart_count: number
  last_exit_code?: number | null
  last_restart_at_ms?: number | null
  last_failure?: {
    message?: string
    rendered?: string
    user?: { fallback?: string }
  } | null
}

type PluginUiAction = {
  kind: 'none' | 'invoke_tool' | 'open_plugin_workbench' | 'open_url' | 'submit_prompt' | 'invoke_command' | string
  tool?: string
  command?: string
  input?: JsonValue
  tab?: string | null
  url?: string
  prompt?: string
  submit_output_as_prompt?: boolean
}

type PluginControlOption = { label: string; value: string; description?: string }

type PluginControl = {
  plugin_id: string
  id: string
  title: string
  description?: string
  location: string
  kind: string
  options?: PluginControlOption[]
  value?: JsonValue
  action: PluginUiAction
}

type PluginCommand = {
  plugin_id: string
  id: string
  title: string
  description?: string
  category: string
  slash?: string | null
  aliases?: string[]
  usage?: string | null
  location: string
  input_schema?: JsonValue
  handler?: string | null
  action: PluginUiAction
}

type PluginView = {
  plugin_id: string
  id: string
  title: string
  description?: string
  location: string
  kind: string
  content?: string | null
  url?: string | null
  controls?: Array<Omit<PluginControl, 'plugin_id'>>
}

type PluginTool = {
  name?: string
  summary?: string | null
  description?: string | null
  contract?: {
    input_schema?: JsonValue
    output_schema?: JsonValue
    strict?: boolean
  }
  tags?: string[]
  permissions?: JsonValue
}

type PluginUiCatalogResponse = {
  catalog?: {
    studio?: {
      commands?: PluginCommand[]
      controls?: PluginControl[]
      views?: PluginView[]
    }
  }
  tool_registry_generation?: number
}

type PluginStatusListResponse = { items?: PluginStatus[] }

type ConfiguredPlugin = {
  enabled?: boolean
  package?: JsonValue
  config?: JsonValue
  timeouts?: JsonValue
  [key: string]: JsonValue
}

type PluginManifest = {
  namespace?: string
  name?: string
  version?: string
  summary?: string | null
  help?: string | null
  authors?: string[]
  transports?: string[]
  hooks?: JsonValue
  tags?: string[]
  tools?: PluginTool[]
  commands?: Array<{ id?: string; title?: string }>
  skills?: Array<{ name?: string; description?: string }>
  config_schema?: JsonValue
  config_schema_i18n?: Record<string, JsonValue>
}

type PluginInspectResponse = {
  plugin?: {
    status?: PluginStatus
    manifest?: PluginManifest | null
    authority?: {
      trust_level?: string
      provenance?: string[]
      plugin_capabilities?: string[]
      tool_capabilities?: Record<string, string[]>
    } | null
    hooks?: JsonValue[]
    configured_plugin?: ConfiguredPlugin | null
  }
}

type PluginLog = {
  seq: number
  timestamp_ms: number
  level: string
  source: string
  message: string
  fields?: JsonValue
}

type PluginLogsResponse = { plugin_id: string; logs?: PluginLog[] }
type PanelTab =
  | 'overview'
  | 'config'
  | 'tools'
  | 'commands'
  | 'views'
  | 'controls'
  | 'capabilities'
  | 'logs'
  | 'diagnostics'

const router = useRouter()
const route = useRoute()
const chat = useChatStore()
const toasts = useToastsStore()

const loading = ref(false)
const error = ref('')
const statuses = ref<PluginStatus[]>([])
const catalog = ref<PluginUiCatalogResponse | null>(null)
const selectedPluginId = ref('')
const selectedInspect = ref<PluginInspectResponse | null>(null)
const logs = ref<PluginLog[]>([])
const detailLoading = ref(false)
const detailError = ref('')
const activeTab = ref<PanelTab>('overview')
const busyActionKey = ref('')
const controlValues = ref<Record<string, JsonValue>>({})
const commandInputs = ref<Record<string, string>>({})
const toolInputs = ref<Record<string, string>>({})
const selectedToolName = ref('')
const pluginQuery = ref('')
const transportFilter = ref('')
const stateFilter = ref('')
const lastResult = ref<JsonValue>(null)

const panelTabs: Array<{ id: PanelTab; label: string }> = [
  { id: 'overview', label: 'Overview' },
  { id: 'config', label: 'Config' },
  { id: 'tools', label: 'Tools' },
  { id: 'commands', label: 'Commands' },
  { id: 'views', label: 'Views' },
  { id: 'controls', label: 'Controls' },
  { id: 'capabilities', label: 'Capabilities' },
  { id: 'logs', label: 'Logs' },
  { id: 'diagnostics', label: 'Diagnostics' },
]

function panelTabFromRequest(value: unknown): PanelTab | null {
  const requested = String(value || '')
    .trim()
    .toLowerCase()
  const tabMap: Record<string, PanelTab> = {
    config: 'config',
    tools: 'tools',
    commands: 'commands',
    capabilities: 'capabilities',
    logs: 'logs',
    diagnostics: 'diagnostics',
    overview: 'overview',
    views: 'views',
    controls: 'controls',
  }
  return tabMap[requested] || null
}

const sortedStatuses = computed(() => [...statuses.value].sort((a, b) => a.plugin_id.localeCompare(b.plugin_id)))
const transportOptions = computed(() =>
  [...new Set(sortedStatuses.value.map((status) => status.kind).filter(Boolean))].map((value) => ({
    value,
    label: value,
  })),
)
const stateOptions = computed(() =>
  [...new Set(sortedStatuses.value.map((status) => status.state).filter(Boolean))].map((value) => ({
    value,
    label: value,
  })),
)
const filteredStatuses = computed(() => {
  const query = pluginQuery.value.trim().toLowerCase()
  return sortedStatuses.value.filter((status) => {
    if (transportFilter.value && status.kind !== transportFilter.value) return false
    if (stateFilter.value && status.state !== stateFilter.value) return false
    if (!query) return true
    return `${status.plugin_id}\n${status.kind}\n${status.state}`.toLowerCase().includes(query)
  })
})

const selectedStatus = computed(
  () => statuses.value.find((status) => status.plugin_id === selectedPluginId.value) || null,
)
const selectedManifest = computed(() => selectedInspect.value?.plugin?.manifest || null)
const selectedAuthority = computed(() => selectedInspect.value?.plugin?.authority || null)
const selectedConfiguredPlugin = computed(() => selectedInspect.value?.plugin?.configured_plugin || null)
const selectedTools = computed(() =>
  (Array.isArray(selectedManifest.value?.tools) ? selectedManifest.value?.tools : [])
    .filter((tool) => String(tool?.name || '').trim())
    .sort((left, right) => String(left.name || '').localeCompare(String(right.name || ''))),
)
const selectedTool = computed(
  () =>
    selectedTools.value.find((tool) => String(tool.name || '').trim() === selectedToolName.value) ||
    selectedTools.value[0] ||
    null,
)

const studioCatalog = computed(() => catalog.value?.catalog?.studio || {})
const selectedViews = computed(() =>
  (Array.isArray(studioCatalog.value.views) ? studioCatalog.value.views : []).filter(
    (item) => item.plugin_id === selectedPluginId.value,
  ),
)
const topLevelControls = computed(() =>
  (Array.isArray(studioCatalog.value.controls) ? studioCatalog.value.controls : []).filter(
    (item) => item.plugin_id === selectedPluginId.value,
  ),
)
const selectedCommands = computed(() =>
  (Array.isArray(studioCatalog.value.commands) ? studioCatalog.value.commands : []).filter(
    (item) => item.plugin_id === selectedPluginId.value,
  ),
)
const nestedControls = computed<PluginControl[]>(() =>
  selectedViews.value.flatMap((view) =>
    (view.controls || []).map((control) => ({ ...control, plugin_id: view.plugin_id })),
  ),
)
const selectedControls = computed(() => [...topLevelControls.value, ...nestedControls.value])

function preferredPluginId(): string {
  const contributedIds = new Set(
    [
      ...(Array.isArray(studioCatalog.value.commands) ? studioCatalog.value.commands : []),
      ...(Array.isArray(studioCatalog.value.controls) ? studioCatalog.value.controls : []),
      ...(Array.isArray(studioCatalog.value.views) ? studioCatalog.value.views : []),
    ]
      .map((item) => String(item.plugin_id || '').trim())
      .filter(Boolean),
  )
  return (
    sortedStatuses.value.find((status) => contributedIds.has(status.plugin_id))?.plugin_id ||
    sortedStatuses.value[0]?.plugin_id ||
    ''
  )
}

const activeSessionId = computed(() => {
  const id = Number(chat.selectedSessionId)
  return Number.isSafeInteger(id) && id > 0 ? id : null
})
const contributionCount = computed(
  () => selectedViews.value.length + selectedControls.value.length + selectedCommands.value.length,
)
const diagnosticLogs = computed(() =>
  logs.value.filter((entry) => ['warn', 'warning', 'error', 'fatal'].includes(String(entry.level || '').toLowerCase())),
)

function statusFailure(status: PluginStatus | null): string {
  return String(
    status?.last_failure?.user?.fallback || status?.last_failure?.rendered || status?.last_failure?.message || '',
  ).trim()
}

function statusTone(state: string): string {
  if (state === 'running') return 'text-success'
  if (state === 'restarting') return 'text-warning'
  return 'text-destructive'
}

function controlKey(control: PluginControl): string {
  return `${control.plugin_id}:${control.id}`
}
function controlValue(control: PluginControl): JsonValue {
  const key = controlKey(control)
  return Object.prototype.hasOwnProperty.call(controlValues.value, key)
    ? controlValues.value[key]
    : (control.value ?? null)
}
function setControlValue(control: PluginControl, value: JsonValue) {
  controlValues.value = { ...controlValues.value, [controlKey(control)]: value }
}
function stringControlValue(control: PluginControl): string {
  const value = controlValue(control)
  if (typeof value === 'string' || typeof value === 'number') return String(value)
  return ''
}
function boolControlValue(control: PluginControl): boolean {
  return controlValue(control) === true
}

function commandKey(command: PluginCommand): string {
  return `${command.plugin_id}:${command.id}`
}
function hasCommandInput(command: PluginCommand): boolean {
  return command.input_schema !== undefined && command.input_schema !== null
}
function parseJsonObject(raw: string, label: string): JsonValue {
  if (!raw.trim()) return {}
  const parsed = JSON.parse(raw) as JsonValue
  if (!isJsonRecord(parsed)) throw new Error(`${label} must be a JSON object.`)
  return parsed
}
function parseCommandInput(command: PluginCommand): JsonValue {
  return parseJsonObject(String(commandInputs.value[commandKey(command)] || ''), 'Command input')
}
function toolName(tool: PluginTool | null | undefined): string {
  return String(tool?.name || '').trim()
}
function toolInputKey(tool: PluginTool): string {
  return `${selectedPluginId.value}:${toolName(tool)}`
}
function defaultToolInput(tool: PluginTool): string {
  const schema = tool.contract?.input_schema
  const value = schema && isJsonRecord(schema) ? defaultValueForSchema(schema, schema) : {}
  return JSON.stringify(isJsonRecord(value) ? value : {}, null, 2)
}
function toolInput(tool: PluginTool): string {
  const key = toolInputKey(tool)
  if (!Object.prototype.hasOwnProperty.call(toolInputs.value, key)) {
    toolInputs.value = { ...toolInputs.value, [key]: defaultToolInput(tool) }
  }
  return toolInputs.value[key] || '{}'
}
function setToolInput(tool: PluginTool, value: string) {
  toolInputs.value = { ...toolInputs.value, [toolInputKey(tool)]: value }
}

function resultText(value: JsonValue): string {
  if (value === null || value === undefined) return ''
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}
function asRecord(value: JsonValue): Record<string, JsonValue> | null {
  return isJsonRecord(value) ? (value as Record<string, JsonValue>) : null
}
function openExternalUrl(raw: string) {
  const url = new URL(raw, window.location.href)
  if (url.protocol !== 'http:' && url.protocol !== 'https:') throw new Error('Plugin URLs must use HTTP or HTTPS.')
  window.open(url.toString(), '_blank', 'noopener,noreferrer')
}
async function submitPrompt(prompt: string) {
  const sessionId = String(chat.selectedSessionId || '').trim()
  if (!sessionId) throw new Error('Open a session before submitting a plugin prompt.')
  const previous = chat.getComposerDraft(sessionId).trim()
  chat.setComposerDraft(sessionId, previous ? `${previous}\n${prompt}` : prompt)
  await router.push('/chat')
}
async function handleClientAction(action: PluginUiAction | null | undefined): Promise<boolean> {
  if (!action || action.kind === 'none') return false
  if (action.kind === 'open_url') {
    openExternalUrl(String(action.url || ''))
    return true
  }
  if (action.kind === 'submit_prompt') {
    await submitPrompt(String(action.prompt || ''))
    return true
  }
  if (action.kind === 'open_plugin_workbench') {
    const tab = panelTabFromRequest(action.tab)
    if (tab) activeTab.value = tab
    return true
  }
  return false
}

async function invokePluginTool(pluginId: string, tool: string, input: JsonValue): Promise<JsonValue> {
  if (!activeSessionId.value) throw new Error('Open a session before running plugin tools.')
  return await apiJson<JsonValue>('/api/v1/plugins/ui/invoke-tool', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      plugin_id: pluginId,
      tool,
      input: asRecord(input) || {},
      session_id: activeSessionId.value,
    }),
  })
}
async function handlePluginToolResult(value: JsonValue, submitOutputAsPrompt = false) {
  const result = asRecord(value)
  if (!result) return
  const outputText = typeof result.output_text === 'string' ? result.output_text.trim() : ''
  if (submitOutputAsPrompt && outputText) {
    await submitPrompt(outputText)
    return
  }
  if (outputText) toasts.push('success', outputText)
}
async function runManifestTool(tool: PluginTool) {
  const name = toolName(tool)
  if (!name || busyActionKey.value) return
  const key = `tool:${selectedPluginId.value}:${name}`
  busyActionKey.value = key
  lastResult.value = null
  try {
    const input = parseJsonObject(toolInput(tool), 'Tool input')
    const response = await invokePluginTool(selectedPluginId.value, name, input)
    lastResult.value = response
    await handlePluginToolResult(response)
    toasts.push('success', `${name} completed`)
  } catch (reason) {
    toasts.push('error', reason instanceof Error ? reason.message : String(reason))
  } finally {
    busyActionKey.value = ''
  }
}

async function handleCommandOutput(pluginId: string, value: JsonValue, depth = 0) {
  if (depth > 5) throw new Error('Plugin command output recursion limit reached.')
  const output = asRecord(value)
  if (!output) return
  const kind = String(output.kind || '')
  if (kind === 'message' && typeof output.text === 'string') {
    toasts.push('success', output.text)
    return
  }
  if (await handleClientAction(output as PluginUiAction)) return
  if (kind === 'invoke_tool') {
    const tool = String(output.tool || '').trim()
    if (!tool) throw new Error('Plugin command did not provide a tool name.')
    const response = await invokePluginTool(pluginId, tool, output.input ?? {})
    lastResult.value = response
    await handlePluginToolResult(response, output.submit_output_as_prompt === true)
    return
  }
  if (kind === 'invoke_command') {
    const command = String(output.command || '').trim()
    if (!command) throw new Error('Plugin command did not provide a command id.')
    const response = await postPluginAction(
      `/api/v1/plugins/${encodeURIComponent(pluginId)}/commands/${encodeURIComponent(command)}`,
      output.input ?? {},
    )
    lastResult.value = response
    const record = asRecord(response)
    await handleCommandOutput(pluginId, record?.result, depth + 1)
  }
}
async function handleActionResponse(pluginId: string, response: JsonValue, fallbackAction: PluginUiAction) {
  const record = asRecord(response)
  if (!record) return
  const action = (asRecord(record.action) || fallbackAction) as PluginUiAction
  if (action.kind === 'invoke_command') {
    await handleCommandOutput(pluginId, record.result)
  } else if (action.kind === 'invoke_tool') {
    await handlePluginToolResult(record.result, action.submit_output_as_prompt === true)
  }
}
async function postPluginAction(path: string, input: JsonValue): Promise<JsonValue> {
  if (!activeSessionId.value) throw new Error('Open a session before running plugin actions.')
  return await apiJson<JsonValue>(path, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ input, session_id: activeSessionId.value }),
  })
}
async function runControl(control: PluginControl, input: JsonValue = controlValue(control)) {
  const key = controlKey(control)
  if (busyActionKey.value) return
  busyActionKey.value = key
  lastResult.value = null
  try {
    if (await handleClientAction(control.action)) return
    const response = await postPluginAction(
      `/api/v1/plugins/${encodeURIComponent(control.plugin_id)}/ui/actions/${encodeURIComponent(control.id)}`,
      { value: input },
    )
    lastResult.value = response
    await handleActionResponse(control.plugin_id, response, control.action)
    toasts.push('success', `${control.title} completed`)
  } catch (reason) {
    toasts.push('error', reason instanceof Error ? reason.message : String(reason))
  } finally {
    busyActionKey.value = ''
  }
}
async function updateToggle(control: PluginControl, event: Event) {
  const checked = (event.target as HTMLInputElement | null)?.checked === true
  setControlValue(control, checked)
  await runControl(control, checked)
}
async function updateSelect(control: PluginControl, value: string) {
  setControlValue(control, value)
  await runControl(control, value)
}
async function runCommand(command: PluginCommand) {
  const key = commandKey(command)
  if (busyActionKey.value) return
  busyActionKey.value = key
  lastResult.value = null
  try {
    if (await handleClientAction(command.action)) return
    const input = parseCommandInput(command)
    const endpoint = command.handler
      ? `/api/v1/plugins/${encodeURIComponent(command.plugin_id)}/commands/${encodeURIComponent(command.id)}`
      : `/api/v1/plugins/${encodeURIComponent(command.plugin_id)}/ui/actions/${encodeURIComponent(command.id)}`
    const response = await postPluginAction(endpoint, input)
    lastResult.value = response
    if (command.handler) {
      const record = asRecord(response)
      await handleCommandOutput(command.plugin_id, record?.result)
    } else {
      await handleActionResponse(command.plugin_id, response, command.action)
    }
    toasts.push('success', `${command.title} completed`)
  } catch (reason) {
    toasts.push('error', reason instanceof Error ? reason.message : String(reason))
  } finally {
    busyActionKey.value = ''
  }
}

async function loadSelectedPlugin() {
  const id = selectedPluginId.value
  selectedInspect.value = null
  logs.value = []
  detailError.value = ''
  lastResult.value = null
  if (!id) return
  detailLoading.value = true
  try {
    const [inspect, logResponse] = await Promise.all([
      apiJson<PluginInspectResponse>(`/api/v1/plugins/${encodeURIComponent(id)}`),
      apiJson<PluginLogsResponse>(`/api/v1/plugins/${encodeURIComponent(id)}/logs?limit=100`),
    ])
    selectedInspect.value = inspect
    logs.value = Array.isArray(logResponse?.logs) ? logResponse.logs : []
    const availableTools = Array.isArray(inspect?.plugin?.manifest?.tools) ? inspect.plugin.manifest.tools : []
    if (!availableTools.some((tool) => toolName(tool) === selectedToolName.value)) {
      selectedToolName.value = toolName(availableTools[0])
    }
  } catch (reason) {
    detailError.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    detailLoading.value = false
  }
}

async function refresh() {
  loading.value = true
  error.value = ''
  try {
    const [statusData, catalogData] = await Promise.all([
      apiJson<PluginStatusListResponse | PluginStatus[]>('/api/v1/plugins'),
      apiJson<PluginUiCatalogResponse>('/api/v1/plugins/ui'),
    ])
    statuses.value = Array.isArray(statusData) ? statusData : Array.isArray(statusData?.items) ? statusData.items : []
    catalog.value = catalogData && typeof catalogData === 'object' ? catalogData : null
    const ids = new Set(statuses.value.map((status) => status.plugin_id))
    const requestedPlugin = String(route.query.plugin || '').trim()
    const targetPluginId =
      requestedPlugin && ids.has(requestedPlugin)
        ? requestedPlugin
        : selectedPluginId.value && ids.has(selectedPluginId.value)
          ? selectedPluginId.value
          : preferredPluginId()
    if (selectedPluginId.value === targetPluginId) {
      await loadSelectedPlugin()
    } else {
      selectedPluginId.value = targetPluginId
    }
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
    statuses.value = []
    catalog.value = null
  } finally {
    loading.value = false
  }
}

async function selectPlugin(id: string) {
  if (!id || id === selectedPluginId.value) return
  selectedPluginId.value = id
  await router.replace({ path: route.path, query: { ...route.query, plugin: id }, hash: route.hash })
}

async function syncDeepLink() {
  if (!selectedPluginId.value) return
  const plugin = String(route.query.plugin || '')
  const tab = String(route.query.pluginTab || '')
  if (plugin === selectedPluginId.value && tab === activeTab.value) return
  await router.replace({
    path: route.path,
    query: { ...route.query, plugin: selectedPluginId.value, pluginTab: activeTab.value },
    hash: route.hash,
  })
}

watch(selectedPluginId, () => {
  activeTab.value = panelTabFromRequest(route.query.pluginTab) || 'overview'
  void loadSelectedPlugin()
})
watch(activeTab, () => void syncDeepLink())
watch(
  () => [route.query.plugin, route.query.pluginTab] as const,
  ([plugin, tab]) => {
    const requestedPlugin = String(plugin || '').trim()
    if (requestedPlugin && statuses.value.some((status) => status.plugin_id === requestedPlugin)) {
      selectedPluginId.value = requestedPlugin
    }
    const requestedTab = panelTabFromRequest(tab)
    if (requestedTab) activeTab.value = requestedTab
  },
)

onMounted(() => void refresh())
</script>

<template>
  <div class="grid min-w-0 gap-5">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h2 class="text-base font-semibold">Plugin Workbench</h2>
        <p class="mt-1 max-w-3xl text-sm text-muted-foreground">
          Configure loaded plugins, run their tools and commands, and inspect every host-declared UI and runtime signal.
        </p>
      </div>
      <IconButton
        variant="outline"
        size="md"
        :tooltip="loading ? 'Refreshing plugins' : 'Refresh plugins'"
        :aria-label="loading ? 'Refreshing plugins' : 'Refresh plugins'"
        :disabled="loading"
        @click="refresh"
      >
        <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
      </IconButton>
    </div>

    <div
      v-if="error"
      class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
    >
      {{ error }}
    </div>

    <section
      class="grid gap-3 rounded-lg border border-border/60 bg-muted/10 p-3 md:grid-cols-[minmax(0,1fr)_12rem_12rem]"
    >
      <SearchInput
        v-model="pluginQuery"
        placeholder="Search plugins"
        :show-search-button="false"
        input-aria-label="Search plugins"
      />
      <OptionPicker
        v-model="transportFilter"
        :options="transportOptions"
        title="Transport filter"
        empty-label="All transports"
      />
      <OptionPicker v-model="stateFilter" :options="stateOptions" title="State filter" empty-label="All states" />
    </section>

    <div v-if="loading && statuses.length === 0" class="text-sm text-muted-foreground">Loading plugins…</div>
    <div v-else-if="statuses.length === 0" class="text-sm text-muted-foreground">No plugins are loaded.</div>

    <div
      v-else
      class="grid min-h-[42rem] min-w-0 overflow-hidden rounded-lg border border-border/60 lg:grid-cols-[17rem_minmax(0,1fr)]"
    >
      <nav
        class="min-w-0 border-b border-border/60 bg-muted/10 p-2 lg:border-b-0 lg:border-r"
        aria-label="Loaded plugins"
      >
        <button
          v-for="status in filteredStatuses"
          :key="status.plugin_id"
          type="button"
          class="flex w-full min-w-0 items-start justify-between gap-2 rounded-md px-3 py-2.5 text-left transition-colors"
          :class="selectedPluginId === status.plugin_id ? 'bg-primary/10 ring-1 ring-primary/20' : 'hover:bg-muted/60'"
          @click="selectPlugin(status.plugin_id)"
        >
          <span class="min-w-0">
            <span class="block truncate font-mono text-xs font-semibold">{{ status.plugin_id }}</span>
            <span class="mt-0.5 block truncate text-[11px] text-muted-foreground"
              >{{ status.kind }} · {{ status.state }}</span
            >
          </span>
          <span class="mt-1 h-2 w-2 shrink-0 rounded-full bg-current" :class="statusTone(status.state)" />
        </button>
        <div v-if="filteredStatuses.length === 0" class="px-3 py-8 text-center text-xs text-muted-foreground">
          No matching plugins.
        </div>
      </nav>

      <div class="min-w-0 p-4 lg:p-5">
        <div v-if="detailLoading" class="text-sm text-muted-foreground">Loading plugin details…</div>
        <div v-else-if="detailError" class="break-words text-sm text-destructive">{{ detailError }}</div>
        <template v-else-if="selectedStatus">
          <header class="flex flex-wrap items-start justify-between gap-3 border-b border-border/60 pb-4">
            <div class="min-w-0">
              <h3 class="break-all font-mono text-base font-semibold">{{ selectedPluginId }}</h3>
              <p v-if="selectedManifest?.summary" class="mt-1 max-w-3xl text-sm text-muted-foreground">
                {{ selectedManifest.summary }}
              </p>
              <div class="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
                <span :class="statusTone(selectedStatus.state)">{{ selectedStatus.state }}</span>
                <span>transport: {{ selectedStatus.kind }}</span>
                <span v-if="selectedManifest?.version">version: {{ selectedManifest.version }}</span>
                <span>{{ selectedTools.length }} tools</span>
                <span>{{ contributionCount }} Studio contributions</span>
                <span v-if="catalog?.tool_registry_generation !== undefined"
                  >registry: {{ catalog.tool_registry_generation }}</span
                >
              </div>
              <div v-if="statusFailure(selectedStatus)" class="mt-2 text-xs text-destructive">
                {{ statusFailure(selectedStatus) }}
              </div>
            </div>
          </header>

          <div class="flex gap-1 overflow-x-auto border-b border-border/60 py-2" role="tablist">
            <button
              v-for="tab in panelTabs"
              :key="tab.id"
              type="button"
              role="tab"
              :aria-selected="activeTab === tab.id"
              class="shrink-0 rounded-md px-3 py-2 text-xs font-medium transition-colors"
              :class="
                activeTab === tab.id ? 'bg-primary/10 text-foreground' : 'text-muted-foreground hover:bg-muted/50'
              "
              @click="activeTab = tab.id"
            >
              {{ tab.label }}
            </button>
          </div>

          <div class="mt-5 min-w-0">
            <div v-if="activeTab === 'overview'" class="grid gap-5">
              <MarkdownRenderer v-if="selectedManifest?.help" :content="selectedManifest.help" source-path="" />
              <dl class="grid gap-x-6 gap-y-4 sm:grid-cols-2 xl:grid-cols-3">
                <div>
                  <dt class="text-xs text-muted-foreground">Trust level</dt>
                  <dd class="mt-1 font-mono text-sm">{{ selectedAuthority?.trust_level || 'Not reported' }}</dd>
                </div>
                <div>
                  <dt class="text-xs text-muted-foreground">Authors</dt>
                  <dd class="mt-1 text-sm">{{ selectedManifest?.authors?.join(', ') || 'Not reported' }}</dd>
                </div>
                <div>
                  <dt class="text-xs text-muted-foreground">Tools</dt>
                  <dd class="mt-1 font-mono text-sm">{{ selectedTools.length }}</dd>
                </div>
                <div>
                  <dt class="text-xs text-muted-foreground">Commands</dt>
                  <dd class="mt-1 font-mono text-sm">{{ selectedCommands.length }}</dd>
                </div>
                <div>
                  <dt class="text-xs text-muted-foreground">Skills</dt>
                  <dd class="mt-1 font-mono text-sm">{{ selectedManifest?.skills?.length || 0 }}</dd>
                </div>
                <div>
                  <dt class="text-xs text-muted-foreground">Configured</dt>
                  <dd class="mt-1 text-sm">{{ selectedConfiguredPlugin ? 'Yes' : 'No explicit record' }}</dd>
                </div>
              </dl>
              <div class="grid gap-2 sm:grid-cols-2 xl:grid-cols-4">
                <Button variant="outline" @click="activeTab = 'config'">Configure plugin</Button>
                <Button variant="outline" @click="activeTab = 'tools'">Run tools</Button>
                <Button variant="outline" @click="activeTab = 'capabilities'">Inspect capabilities</Button>
                <Button variant="outline" @click="activeTab = 'diagnostics'">Open diagnostics</Button>
              </div>
            </div>

            <PluginConfigEditor
              v-else-if="activeTab === 'config'"
              :plugin-id="selectedPluginId"
              :manifest="selectedManifest"
              :configured-plugin="selectedConfiguredPlugin"
              :disabled="detailLoading"
              @saved="refresh"
            />

            <div v-else-if="activeTab === 'tools'" class="grid min-w-0 gap-4 lg:grid-cols-[15rem_minmax(0,1fr)]">
              <nav class="grid content-start gap-1 rounded-lg border border-border/60 bg-muted/10 p-2">
                <button
                  v-for="tool in selectedTools"
                  :key="toolName(tool)"
                  type="button"
                  class="grid min-w-0 gap-0.5 rounded-md px-3 py-2 text-left"
                  :class="
                    selectedTool?.name === tool.name ? 'bg-primary/10 ring-1 ring-primary/20' : 'hover:bg-muted/60'
                  "
                  @click="selectedToolName = toolName(tool)"
                >
                  <span class="truncate font-mono text-xs font-semibold">{{ toolName(tool) }}</span>
                  <span class="line-clamp-2 text-[11px] text-muted-foreground">{{
                    tool.summary || tool.description || 'No summary'
                  }}</span>
                </button>
                <div v-if="selectedTools.length === 0" class="px-3 py-8 text-center text-xs text-muted-foreground">
                  This plugin does not expose tools.
                </div>
              </nav>
              <section v-if="selectedTool" class="grid min-w-0 gap-4">
                <div>
                  <h4 class="font-mono text-sm font-semibold">{{ toolName(selectedTool) }}</h4>
                  <p class="mt-1 text-sm text-muted-foreground">
                    {{ selectedTool.summary || selectedTool.description || 'No summary.' }}
                  </p>
                  <div v-if="selectedTool.tags?.length" class="mt-2 flex flex-wrap gap-1.5">
                    <span
                      v-for="tag in selectedTool.tags"
                      :key="tag"
                      class="rounded bg-muted px-2 py-1 font-mono text-[10px]"
                      >{{ tag }}</span
                    >
                  </div>
                </div>
                <label class="grid gap-1.5">
                  <span class="text-xs text-muted-foreground">JSON input</span>
                  <textarea
                    :value="toolInput(selectedTool)"
                    rows="12"
                    spellcheck="false"
                    :disabled="Boolean(busyActionKey)"
                    class="w-full rounded-md border border-input bg-transparent p-3 font-mono text-xs leading-5 outline-none focus:border-ring"
                    @input="setToolInput(selectedTool, ($event.target as HTMLTextAreaElement).value)"
                  />
                </label>
                <div class="flex flex-wrap items-center justify-between gap-3">
                  <span v-if="!activeSessionId" class="text-xs text-amber-700 dark:text-amber-300"
                    >Open a session before running plugin tools.</span
                  >
                  <span v-else class="text-xs text-muted-foreground"
                    >The invocation is scoped to session {{ activeSessionId }}.</span
                  >
                  <Button :disabled="Boolean(busyActionKey) || !activeSessionId" @click="runManifestTool(selectedTool)">
                    <RiPlayLine class="mr-2 h-4 w-4" /> Run tool
                  </Button>
                </div>
                <details class="rounded-md border border-border/60">
                  <summary class="cursor-pointer px-3 py-2 text-sm font-medium">Input schema</summary>
                  <pre class="max-h-72 overflow-auto border-t border-border/60 p-3 font-mono text-[11px] leading-5">{{
                    resultText(selectedTool.contract?.input_schema) || '{}'
                  }}</pre>
                </details>
                <details class="rounded-md border border-border/60">
                  <summary class="cursor-pointer px-3 py-2 text-sm font-medium">
                    Output schema & permission contract
                  </summary>
                  <pre class="max-h-72 overflow-auto border-t border-border/60 p-3 font-mono text-[11px] leading-5">{{
                    resultText({
                      output_schema: selectedTool.contract?.output_schema,
                      permissions: selectedTool.permissions,
                    })
                  }}</pre>
                </details>
              </section>
            </div>

            <div v-else-if="activeTab === 'commands'" class="grid gap-4">
              <div v-if="selectedCommands.length === 0" class="text-sm text-muted-foreground">
                This plugin does not declare Studio commands.
              </div>
              <article
                v-for="command in selectedCommands"
                :key="commandKey(command)"
                class="grid gap-3 rounded-lg border border-border/60 p-4"
              >
                <div class="flex flex-wrap items-start justify-between gap-3">
                  <div>
                    <h4 class="text-sm font-semibold">{{ command.title }}</h4>
                    <p v-if="command.description" class="mt-1 text-xs text-muted-foreground">
                      {{ command.description }}
                    </p>
                    <div class="mt-1 flex flex-wrap gap-2 font-mono text-[10px] text-muted-foreground">
                      <span v-if="command.slash">{{ command.slash }}</span
                      ><span>{{ command.category }}</span
                      ><span>{{ command.location }}</span>
                    </div>
                  </div>
                  <Button size="sm" :disabled="Boolean(busyActionKey)" @click="runCommand(command)">
                    <RiCommandLine class="mr-2 h-4 w-4" /> Run
                  </Button>
                </div>
                <textarea
                  v-if="hasCommandInput(command)"
                  v-model="commandInputs[commandKey(command)]"
                  rows="7"
                  spellcheck="false"
                  placeholder="{}"
                  class="w-full rounded-md border border-input bg-transparent p-3 font-mono text-xs"
                />
                <details v-if="hasCommandInput(command)" class="rounded-md border border-border/60">
                  <summary class="cursor-pointer px-3 py-2 text-xs font-medium">Input schema</summary>
                  <pre class="max-h-56 overflow-auto border-t border-border/60 p-3 font-mono text-[11px]">{{
                    resultText(command.input_schema)
                  }}</pre>
                </details>
              </article>
            </div>

            <div v-else-if="activeTab === 'views'" class="grid gap-5">
              <div v-if="selectedViews.length === 0" class="text-sm text-muted-foreground">
                This plugin does not declare Studio views.
              </div>
              <section v-for="view in selectedViews" :key="view.id" class="rounded-lg border border-border/60 p-4">
                <div class="flex flex-wrap items-start justify-between gap-2">
                  <div>
                    <h4 class="text-sm font-semibold">{{ view.title }}</h4>
                    <p v-if="view.description" class="mt-1 text-xs text-muted-foreground">{{ view.description }}</p>
                    <div class="mt-1 font-mono text-[10px] text-muted-foreground">
                      {{ view.kind }} · {{ view.location }}
                    </div>
                  </div>
                  <Button v-if="view.url" variant="outline" size="sm" @click="openExternalUrl(view.url)">
                    <RiExternalLinkLine class="mr-2 h-4 w-4" /> Open
                  </Button>
                </div>
                <MarkdownRenderer
                  v-if="view.content && view.kind.toLowerCase() === 'markdown'"
                  class="mt-3"
                  :content="view.content"
                  source-path=""
                />
                <pre
                  v-else-if="view.content"
                  class="mt-3 max-h-80 overflow-auto whitespace-pre-wrap break-words rounded bg-muted/40 p-3 text-xs"
                  >{{ view.content }}</pre
                >
              </section>
            </div>

            <div v-else-if="activeTab === 'controls'" class="grid gap-4">
              <div v-if="selectedControls.length === 0" class="text-sm text-muted-foreground">
                This plugin does not declare Studio controls.
              </div>
              <article
                v-for="control in selectedControls"
                :key="controlKey(control)"
                class="grid gap-3 rounded-lg border border-border/60 p-4 sm:grid-cols-[minmax(0,1fr)_minmax(12rem,20rem)] sm:items-center"
              >
                <div>
                  <h4 class="text-sm font-semibold">{{ control.title }}</h4>
                  <p v-if="control.description" class="mt-1 text-xs text-muted-foreground">{{ control.description }}</p>
                  <div class="mt-1 font-mono text-[10px] text-muted-foreground">
                    {{ control.kind }} · {{ control.location }}
                  </div>
                </div>
                <div class="flex min-w-0 items-center justify-end gap-2">
                  <label
                    v-if="['toggle', 'checkbox', 'switch'].includes(control.kind.toLowerCase())"
                    class="inline-flex items-center gap-2 text-sm"
                  >
                    <input
                      type="checkbox"
                      :checked="boolControlValue(control)"
                      :disabled="Boolean(busyActionKey)"
                      @change="updateToggle(control, $event)"
                    />
                    {{ boolControlValue(control) ? 'On' : 'Off' }}
                  </label>
                  <OptionPicker
                    v-else-if="control.options?.length"
                    :model-value="stringControlValue(control)"
                    :options="control.options"
                    :title="control.title"
                    :include-empty="false"
                    :disabled="Boolean(busyActionKey)"
                    @update:model-value="updateSelect(control, $event)"
                  />
                  <template v-else-if="['input', 'text', 'number'].includes(control.kind.toLowerCase())">
                    <Input
                      :model-value="stringControlValue(control)"
                      :type="control.kind.toLowerCase() === 'number' ? 'number' : 'text'"
                      :disabled="Boolean(busyActionKey)"
                      @update:model-value="setControlValue(control, $event)"
                    />
                    <Button size="sm" :disabled="Boolean(busyActionKey)" @click="runControl(control)">Apply</Button>
                  </template>
                  <Button v-else size="sm" :disabled="Boolean(busyActionKey)" @click="runControl(control)">
                    <RiPlayLine class="mr-2 h-4 w-4" /> Run
                  </Button>
                </div>
              </article>
            </div>

            <div v-else-if="activeTab === 'capabilities'" class="grid gap-5">
              <section class="grid gap-3 rounded-lg border border-border/60 p-4">
                <h4 class="text-sm font-semibold">Plugin authority</h4>
                <dl class="grid gap-4 sm:grid-cols-2">
                  <div>
                    <dt class="text-xs text-muted-foreground">Trust level</dt>
                    <dd class="mt-1 font-mono text-sm">{{ selectedAuthority?.trust_level || 'Not reported' }}</dd>
                  </div>
                  <div>
                    <dt class="text-xs text-muted-foreground">Provenance</dt>
                    <dd class="mt-1 text-sm">{{ selectedAuthority?.provenance?.join(' · ') || 'Not reported' }}</dd>
                  </div>
                </dl>
                <div v-if="selectedAuthority?.plugin_capabilities?.length" class="flex flex-wrap gap-1.5">
                  <span
                    v-for="capability in selectedAuthority.plugin_capabilities"
                    :key="capability"
                    class="rounded bg-muted px-2 py-1 font-mono text-[11px]"
                    >{{ capability }}</span
                  >
                </div>
              </section>
              <section class="grid gap-3 rounded-lg border border-border/60 p-4">
                <h4 class="text-sm font-semibold">Tool capability grants</h4>
                <div
                  v-if="!Object.keys(selectedAuthority?.tool_capabilities || {}).length"
                  class="text-sm text-muted-foreground"
                >
                  No tool capability grants reported.
                </div>
                <div
                  v-for="(capabilities, tool) in selectedAuthority?.tool_capabilities || {}"
                  :key="tool"
                  class="grid gap-1 border-b border-border/50 pb-3 last:border-b-0"
                >
                  <code class="font-mono text-xs font-semibold">{{ tool }}</code>
                  <div class="flex flex-wrap gap-1.5">
                    <span
                      v-for="capability in capabilities"
                      :key="capability"
                      class="rounded bg-muted px-2 py-1 font-mono text-[10px]"
                      >{{ capability }}</span
                    >
                  </div>
                </div>
              </section>
              <details class="rounded-lg border border-border/60">
                <summary class="cursor-pointer px-4 py-3 text-sm font-medium">Raw hooks and manifest tags</summary>
                <pre class="max-h-80 overflow-auto border-t border-border/60 p-3 font-mono text-[11px] leading-5">{{
                  resultText({
                    hooks: selectedInspect?.plugin?.hooks,
                    tags: selectedManifest?.tags,
                    transports: selectedManifest?.transports,
                  })
                }}</pre>
              </details>
            </div>

            <div v-else-if="activeTab === 'logs'" class="grid gap-2">
              <div v-if="logs.length === 0" class="text-sm text-muted-foreground">No plugin logs recorded.</div>
              <article
                v-for="entry in logs"
                :key="entry.seq"
                class="grid gap-1 border-b border-border/50 py-2 last:border-b-0"
              >
                <div class="flex flex-wrap items-center gap-2 text-[11px] text-muted-foreground">
                  <span class="font-mono">#{{ entry.seq }}</span
                  ><span>{{ entry.level }}</span
                  ><span>{{ entry.source }}</span
                  ><span>{{ new Date(entry.timestamp_ms).toLocaleString() }}</span>
                </div>
                <div class="whitespace-pre-wrap break-words text-sm">{{ entry.message }}</div>
                <pre v-if="entry.fields" class="overflow-auto rounded bg-muted/30 p-2 font-mono text-[10px]">{{
                  resultText(entry.fields)
                }}</pre>
              </article>
            </div>

            <div v-else class="grid gap-4">
              <section class="grid gap-3 rounded-lg border border-border/60 p-4">
                <div class="flex items-center gap-2">
                  <RiShieldCheckLine class="h-4 w-4" />
                  <h4 class="text-sm font-semibold">Runtime diagnostics</h4>
                </div>
                <dl class="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
                  <div>
                    <dt class="text-xs text-muted-foreground">State</dt>
                    <dd class="mt-1 text-sm" :class="statusTone(selectedStatus.state)">{{ selectedStatus.state }}</dd>
                  </div>
                  <div>
                    <dt class="text-xs text-muted-foreground">PID</dt>
                    <dd class="mt-1 font-mono text-sm">{{ selectedStatus.pid ?? '—' }}</dd>
                  </div>
                  <div>
                    <dt class="text-xs text-muted-foreground">Restart count</dt>
                    <dd class="mt-1 font-mono text-sm">{{ selectedStatus.restart_count ?? 0 }}</dd>
                  </div>
                  <div>
                    <dt class="text-xs text-muted-foreground">Last exit code</dt>
                    <dd class="mt-1 font-mono text-sm">{{ selectedStatus.last_exit_code ?? '—' }}</dd>
                  </div>
                </dl>
                <div
                  v-if="statusFailure(selectedStatus)"
                  class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive"
                >
                  {{ statusFailure(selectedStatus) }}
                </div>
                <div v-else class="text-xs text-muted-foreground">No runtime failure is currently reported.</div>
              </section>
              <section class="grid gap-2 rounded-lg border border-border/60 p-4">
                <h4 class="text-sm font-semibold">Warnings and errors from recent logs</h4>
                <div v-if="diagnosticLogs.length === 0" class="text-sm text-muted-foreground">
                  No warning or error log entries in the latest 100 records.
                </div>
                <article
                  v-for="entry in diagnosticLogs"
                  :key="entry.seq"
                  class="grid gap-1 border-b border-border/50 py-2 last:border-b-0"
                >
                  <div class="font-mono text-[10px] text-muted-foreground">
                    {{ entry.level }} · {{ entry.source }} · #{{ entry.seq }}
                  </div>
                  <div class="whitespace-pre-wrap break-words text-sm">{{ entry.message }}</div>
                </article>
              </section>
              <details class="rounded-lg border border-border/60">
                <summary class="cursor-pointer px-4 py-3 text-sm font-medium">Raw plugin inspection</summary>
                <pre
                  class="max-h-[32rem] overflow-auto border-t border-border/60 p-3 font-mono text-[11px] leading-5"
                  >{{ resultText(selectedInspect) }}</pre
                >
              </details>
            </div>

            <section v-if="resultText(lastResult)" class="mt-6 grid gap-2 border-t border-border/60 pt-4">
              <h4 class="text-sm font-semibold">Last action result</h4>
              <pre
                class="max-h-[28rem] overflow-auto rounded-md border border-border/60 p-3 font-mono text-xs leading-5"
                >{{ resultText(lastResult) }}</pre
              >
            </section>
          </div>
        </template>
      </div>
    </div>
  </div>
</template>
