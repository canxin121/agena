<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { RiCommandLine, RiExternalLinkLine, RiPlayLine, RiRefreshLine } from '@remixicon/vue'

import MarkdownRenderer from '@/components/markdown/MarkdownRenderer.vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { apiJson } from '@/lib/api'
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

type PluginInspectResponse = {
  plugin?: {
    status?: PluginStatus
    manifest?: {
      namespace?: string
      name?: string
      version?: string
      summary?: string | null
      help?: string | null
      authors?: string[]
      transports?: string[]
      tools?: Array<{ name?: string; summary?: string; description?: string }>
      commands?: Array<{ id?: string; title?: string }>
      skills?: Array<{ name?: string; description?: string }>
      config_schema?: JsonValue
    } | null
    authority?: {
      trust_level?: string
      provenance?: string[]
      plugin_capabilities?: string[]
      tool_capabilities?: Record<string, string[]>
    } | null
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
type PanelTab = 'overview' | 'views' | 'controls' | 'commands' | 'logs'

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
const lastResult = ref<JsonValue>(null)

const panelTabs: Array<{ id: PanelTab; label: string }> = [
  { id: 'overview', label: 'Overview' },
  { id: 'views', label: 'Views' },
  { id: 'controls', label: 'Controls' },
  { id: 'commands', label: 'Commands' },
  { id: 'logs', label: 'Logs' },
]

function panelTabFromRequest(value: unknown): PanelTab | null {
  const requested = String(value || '').trim()
  const tabMap: Record<string, PanelTab> = {
    config: 'overview',
    tools: 'controls',
    commands: 'commands',
    capabilities: 'overview',
    logs: 'logs',
    diagnostics: 'logs',
    overview: 'overview',
    views: 'views',
    controls: 'controls',
  }
  return tabMap[requested] || null
}

const sortedStatuses = computed(() => [...statuses.value].sort((a, b) => a.plugin_id.localeCompare(b.plugin_id)))

const selectedStatus = computed(
  () => statuses.value.find((status) => status.plugin_id === selectedPluginId.value) || null,
)

const selectedManifest = computed(() => selectedInspect.value?.plugin?.manifest || null)
const selectedAuthority = computed(() => selectedInspect.value?.plugin?.authority || null)

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

function parseCommandInput(command: PluginCommand): JsonValue {
  const raw = String(commandInputs.value[commandKey(command)] || '').trim()
  if (!raw) return {}
  const parsed = JSON.parse(raw) as JsonValue
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
    throw new Error('Command input must be a JSON object.')
  }
  return parsed
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
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  return value as Record<string, JsonValue>
}

function openExternalUrl(raw: string) {
  const url = new URL(raw, window.location.href)
  if (url.protocol !== 'http:' && url.protocol !== 'https:') {
    throw new Error('Plugin URLs must use HTTP or HTTPS.')
  }
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
  if (outputText) {
    toasts.push('success', outputText)
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
  } catch (err) {
    toasts.push('error', err instanceof Error ? err.message : String(err))
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
  } catch (err) {
    toasts.push('error', err instanceof Error ? err.message : String(err))
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
  } catch (err) {
    detailError.value = err instanceof Error ? err.message : String(err)
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
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
    statuses.value = []
    catalog.value = null
  } finally {
    loading.value = false
  }
}

watch(selectedPluginId, () => {
  activeTab.value = panelTabFromRequest(route.query.pluginTab) || 'overview'
  void loadSelectedPlugin()
})

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

onMounted(() => {
  void refresh()
})
</script>

<template>
  <div class="space-y-6">
    <div class="flex items-start justify-between gap-3">
      <div>
        <div class="text-lg font-medium">Plugin workbench</div>
        <div class="mt-1 text-sm text-muted-foreground">
          Loaded plugins and their server-declared views, controls, commands, and logs.
        </div>
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

    <div v-if="loading && statuses.length === 0" class="text-sm text-muted-foreground">Loading plugins...</div>
    <div v-else-if="statuses.length === 0" class="text-sm text-muted-foreground">No plugins are loaded.</div>

    <div v-else class="grid min-h-[34rem] grid-cols-1 border-y border-border/60 md:grid-cols-[15rem_minmax(0,1fr)]">
      <nav class="border-b border-border/60 py-3 md:border-b-0 md:border-r md:pr-3" aria-label="Loaded plugins">
        <button
          v-for="status in sortedStatuses"
          :key="status.plugin_id"
          type="button"
          class="flex w-full items-start justify-between gap-2 rounded px-2.5 py-2 text-left hover:bg-muted/40"
          :class="selectedPluginId === status.plugin_id ? 'bg-muted/60 text-foreground' : 'text-foreground/80'"
          @click="selectedPluginId = status.plugin_id"
        >
          <span class="min-w-0">
            <span class="block truncate font-mono text-xs font-semibold">{{ status.plugin_id }}</span>
            <span class="mt-0.5 block text-[11px] text-muted-foreground">{{ status.kind }}</span>
          </span>
          <span class="mt-0.5 h-2 w-2 shrink-0 rounded-full bg-current" :class="statusTone(status.state)" />
        </button>
      </nav>

      <div class="min-w-0 py-4 md:pl-5">
        <div v-if="detailLoading" class="text-sm text-muted-foreground">Loading plugin details...</div>
        <div v-else-if="detailError" class="break-words text-sm text-destructive">{{ detailError }}</div>
        <template v-else-if="selectedStatus">
          <header class="flex flex-wrap items-start justify-between gap-3">
            <div class="min-w-0">
              <h2 class="break-all font-mono text-base font-semibold">{{ selectedPluginId }}</h2>
              <p v-if="selectedManifest?.summary" class="mt-1 text-sm text-muted-foreground">
                {{ selectedManifest.summary }}
              </p>
              <div class="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
                <span :class="statusTone(selectedStatus.state)">{{ selectedStatus.state }}</span>
                <span>transport: {{ selectedStatus.kind }}</span>
                <span v-if="selectedManifest?.version">version: {{ selectedManifest.version }}</span>
                <span>{{ contributionCount }} UI contributions</span>
                <span v-if="catalog?.tool_registry_generation !== undefined">
                  tool registry: {{ catalog.tool_registry_generation }}
                </span>
              </div>
              <div v-if="statusFailure(selectedStatus)" class="mt-2 text-xs text-destructive">
                {{ statusFailure(selectedStatus) }}
              </div>
            </div>
          </header>

          <div class="mt-5 flex gap-1 overflow-x-auto border-b border-border/60" role="tablist">
            <button
              v-for="tab in panelTabs"
              :key="tab.id"
              type="button"
              role="tab"
              :aria-selected="activeTab === tab.id"
              class="shrink-0 border-b-2 px-3 py-2 text-xs font-medium"
              :class="
                activeTab === tab.id ? 'border-primary text-foreground' : 'border-transparent text-muted-foreground'
              "
              @click="activeTab = tab.id"
            >
              {{ tab.label }}
            </button>
          </div>

          <div class="mt-5">
            <div v-if="activeTab === 'overview'" class="space-y-5">
              <MarkdownRenderer v-if="selectedManifest?.help" :content="selectedManifest.help" source-path="" />
              <dl class="grid gap-x-6 gap-y-4 sm:grid-cols-2">
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
                  <dd class="mt-1 font-mono text-sm">{{ selectedManifest?.tools?.length || 0 }}</dd>
                </div>
                <div>
                  <dt class="text-xs text-muted-foreground">Skills</dt>
                  <dd class="mt-1 font-mono text-sm">{{ selectedManifest?.skills?.length || 0 }}</dd>
                </div>
              </dl>
              <div v-if="selectedAuthority?.plugin_capabilities?.length">
                <h3 class="text-xs font-medium text-muted-foreground">Capabilities</h3>
                <div class="mt-2 flex flex-wrap gap-1.5">
                  <span
                    v-for="capability in selectedAuthority.plugin_capabilities"
                    :key="capability"
                    class="rounded bg-muted px-2 py-1 font-mono text-[11px]"
                  >
                    {{ capability }}
                  </span>
                </div>
              </div>
            </div>

            <div v-else-if="activeTab === 'views'" class="space-y-5">
              <div v-if="selectedViews.length === 0" class="text-sm text-muted-foreground">
                This plugin does not declare Studio views.
              </div>
              <section
                v-for="view in selectedViews"
                :key="view.id"
                class="border-b border-border/60 pb-5 last:border-b-0"
              >
                <div class="flex flex-wrap items-start justify-between gap-2">
                  <div>
                    <h3 class="text-sm font-semibold">{{ view.title }}</h3>
                    <p v-if="view.description" class="mt-1 text-xs text-muted-foreground">{{ view.description }}</p>
                    <div class="mt-1 font-mono text-[10px] text-muted-foreground">
                      {{ view.kind }} · {{ view.location }}
                    </div>
                  </div>
                  <Button v-if="view.url" variant="outline" size="sm" @click="openExternalUrl(view.url)">
                    <RiExternalLinkLine class="mr-2 h-4 w-4" />
                    Open
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

            <div v-else-if="activeTab === 'controls'" class="space-y-4">
              <div v-if="selectedControls.length === 0" class="text-sm text-muted-foreground">
                This plugin does not declare Studio controls.
              </div>
              <div
                v-for="control in selectedControls"
                :key="controlKey(control)"
                class="grid gap-3 border-b border-border/60 pb-4 last:border-b-0 sm:grid-cols-[minmax(0,1fr)_minmax(12rem,20rem)] sm:items-center"
              >
                <div>
                  <div class="text-sm font-medium">{{ control.title }}</div>
                  <div v-if="control.description" class="mt-1 text-xs text-muted-foreground">
                    {{ control.description }}
                  </div>
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
                      class="h-9 min-w-0"
                      @update:model-value="setControlValue(control, $event)"
                    />
                    <Button size="sm" :disabled="Boolean(busyActionKey)" @click="runControl(control)">Apply</Button>
                  </template>
                  <Button v-else size="sm" :disabled="Boolean(busyActionKey)" @click="runControl(control)">
                    <RiPlayLine class="mr-2 h-4 w-4" />
                    {{ busyActionKey === controlKey(control) ? 'Running...' : control.title }}
                  </Button>
                </div>
              </div>
            </div>

            <div v-else-if="activeTab === 'commands'" class="space-y-4">
              <div v-if="selectedCommands.length === 0" class="text-sm text-muted-foreground">
                This plugin does not declare Studio commands.
              </div>
              <div
                v-for="command in selectedCommands"
                :key="commandKey(command)"
                class="border-b border-border/60 pb-4 last:border-b-0"
              >
                <div class="flex flex-wrap items-start justify-between gap-3">
                  <div class="min-w-0">
                    <div class="flex items-center gap-2 text-sm font-medium">
                      <RiCommandLine class="h-4 w-4 text-muted-foreground" />
                      <span>{{ command.title }}</span>
                    </div>
                    <p v-if="command.description" class="mt-1 text-xs text-muted-foreground">
                      {{ command.description }}
                    </p>
                    <div class="mt-1 flex flex-wrap gap-x-3 text-[10px] text-muted-foreground">
                      <span v-if="command.slash" class="font-mono">{{ command.slash }}</span>
                      <span>{{ command.category }}</span>
                      <span>{{ command.location }}</span>
                    </div>
                  </div>
                  <Button size="sm" :disabled="Boolean(busyActionKey)" @click="runCommand(command)">
                    <RiPlayLine class="mr-2 h-4 w-4" />
                    {{ busyActionKey === commandKey(command) ? 'Running...' : 'Run' }}
                  </Button>
                </div>
                <label v-if="hasCommandInput(command)" class="mt-3 grid gap-1.5">
                  <span class="text-xs text-muted-foreground">Input (JSON object)</span>
                  <textarea
                    v-model="commandInputs[commandKey(command)]"
                    rows="4"
                    placeholder="{}"
                    class="w-full rounded-md border border-input bg-background px-3 py-2 font-mono text-xs"
                  />
                </label>
              </div>
            </div>

            <div v-else class="space-y-2">
              <div v-if="logs.length === 0" class="text-sm text-muted-foreground">No plugin logs recorded.</div>
              <div v-for="entry in logs" :key="entry.seq" class="border-b border-border/50 py-2 last:border-b-0">
                <div class="flex flex-wrap gap-x-3 text-[10px] text-muted-foreground">
                  <span>#{{ entry.seq }}</span>
                  <span>{{ new Date(entry.timestamp_ms).toLocaleString() }}</span>
                  <span class="font-semibold uppercase">{{ entry.level }}</span>
                  <span>{{ entry.source }}</span>
                </div>
                <div class="mt-1 whitespace-pre-wrap break-words font-mono text-xs">{{ entry.message }}</div>
              </div>
            </div>
          </div>

          <div v-if="resultText(lastResult)" class="mt-6 border-t border-border/60 pt-4">
            <div class="text-xs font-medium text-muted-foreground">Last result</div>
            <pre class="mt-2 max-h-72 overflow-auto whitespace-pre-wrap break-words rounded bg-muted/40 p-3 text-xs">{{
              resultText(lastResult)
            }}</pre>
          </div>
        </template>
      </div>
    </div>
  </div>
</template>
