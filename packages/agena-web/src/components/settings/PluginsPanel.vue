<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { RiCommandLine, RiPlayLine, RiRefreshLine } from '@remixicon/vue'

import MarkdownRenderer from '@/components/markdown/MarkdownRenderer.vue'
import PluginContractEditor from '@/components/settings/PluginContractEditor.vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import { settingsText as st } from '@/i18n/settingsText'
import { apiJson } from '@/lib/api'
import {
  clonePluginJson,
  type PluginHostEffect,
  type PluginOperationCatalogItem,
  type PluginOperationResult,
  type PluginSettingsContract,
  type PluginSettingsState,
  type PluginSettingsUpdateResponse,
} from '@/lib/pluginOperations'
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

type PluginTool = {
  name?: string
  summary?: string
  description?: string
  tags?: string[]
  docs?: { summary?: string; help?: string }
}

type PluginManifest = {
  namespace?: string
  name?: string
  version?: string
  summary?: string | null
  help?: string | null
  authors?: string[]
  transports?: string[]
  tools?: PluginTool[]
  operations?: Array<{ id?: string; title?: string }>
  skills?: Array<{ name?: string; description?: string }>
  settings?: PluginSettingsContract | null
}

type PluginActivationBlock = {
  code: string
  message: string
  dependencies?: string[]
}

type PluginActivation = {
  requires?: string[]
  after?: string[]
  blocked?: PluginActivationBlock | null
}

type PluginServiceMethod = {
  id: string
  input: PluginSettingsContract
  output: PluginSettingsContract
}

type PluginServiceExport = {
  id: string
  api_version: number
  methods?: PluginServiceMethod[]
}

type PluginServiceImport = {
  id: string
  api_version: number
  optional: boolean
  provider?: string | null
}

type PluginServiceImportInspect = PluginServiceImport & {
  resolved_provider?: string | null
  methods?: PluginServiceMethod[]
  state: string
}

type PluginServiceInspect = {
  exports?: PluginServiceExport[]
  imports?: PluginServiceImportInspect[]
}

type PluginEffectDescriptor = {
  id: number
  kind: string
  label: string
  registered_at_ms: number
  state: 'active' | 'disposing' | 'disposed' | 'failed' | string
  error?: string | null
}

type PluginEffectScopeInspect = {
  plugin_id: string
  generation: number
  lifecycle: 'active' | 'disposing' | 'disposed' | 'failed' | string
  accepting: boolean
  active_leases: number
  cancelled: boolean
  effects?: PluginEffectDescriptor[]
  errors?: string[]
}

type PluginInspectResponse = {
  plugin?: {
    status?: PluginStatus
    manifest?: PluginManifest | null
    activation?: PluginActivation | null
    services?: PluginServiceInspect | null
    effects?: PluginEffectScopeInspect | null
    configured_plugin?: {
      enabled?: boolean
      package?: JsonValue
      config?: JsonValue
      activation?: { requires?: string[]; after?: string[] }
    } | null
    authority?: {
      trust_level?: string
      provenance?: string[]
      plugin_capabilities?: string[]
      tool_capabilities?: Record<string, string[]>
    } | null
  }
}

type PluginReloadDecision = {
  plugin_id: string
  action: 'add' | 'reuse' | 'restart' | 'remove' | 'disabled' | 'blocked' | string
  reasons?: string[]
  triggered_by?: string[]
}

type PluginArchitectureNode = {
  plugin_id: string
  enabled: boolean
  status: PluginStatus
  activation_epoch?: string | null
  blocked?: PluginActivationBlock | null
  service_exports?: PluginServiceExport[]
  service_imports?: PluginServiceImport[]
}

type PluginDependencyEdge = {
  consumer_id: string
  provider_id: string
  kind: 'explicit' | 'required_service' | 'optional_service' | string
  service_id?: string | null
  api_version?: number | null
}

type PluginArchitectureEffect = PluginEffectDescriptor & {
  plugin_id: string
}

type PluginPipelineHandler = {
  owner: string
  id: string
  priority: number
  registration: number
}

type PluginArchitecturePipeline = {
  definition: {
    id: string
    mode: string
    durable: boolean
    scoped: boolean
  }
  failure_policy?: 'abort' | 'continue' | null
  handlers?: PluginPipelineHandler[]
}

type PluginScopedRegistration = {
  key: string
  owner: string
  generation: number
  layer: { kind: 'global' } | { kind: 'scope'; scope: string }
}

type PluginProfileChange = {
  profile: string
  plugin_id: string
  action: 'replace' | 'patch' | 'disable' | 'remove' | string
  paths?: string[]
}

type PluginArchitectureCatalog = {
  profiles?: {
    applied_profiles?: string[]
    changes?: PluginProfileChange[]
  }
  reload?: { decisions?: PluginReloadDecision[] }
  plugins?: PluginArchitectureNode[]
  dependencies?: PluginDependencyEdge[]
  effects?: PluginArchitectureEffect[]
  pipelines?: PluginArchitecturePipeline[]
  tool_registrations?: PluginScopedRegistration[]
  operation_registrations?: PluginScopedRegistration[]
}

type PluginSurfaceCatalogResponse = {
  catalog?: {
    operations?: PluginOperationCatalogItem[]
  }
  tool_registry_generation?: number
}

type PluginStatusListResponse = { items?: PluginStatus[] }

type PluginLog = {
  seq: number
  timestamp_ms: number
  level: string
  source: string
  message: string
  fields?: JsonValue
}

type PluginLogsResponse = { plugin_id: string; logs?: PluginLog[] }
type PanelTab = 'overview' | 'settings' | 'operations' | 'tools' | 'logs' | 'diagnostics'

const PANEL_TABS: Array<{ id: PanelTab; label: string }> = [
  { id: 'overview', label: st('Overview') },
  { id: 'settings', label: st('Settings') },
  { id: 'operations', label: st('Operations') },
  { id: 'tools', label: st('Tools') },
  { id: 'logs', label: st('Logs') },
  { id: 'diagnostics', label: st('Diagnostics') },
]

const PANEL_TAB_IDS = new Set<PanelTab>(PANEL_TABS.map((tab) => tab.id))

const route = useRoute()
const router = useRouter()
const chat = useChatStore()
const toasts = useToastsStore()

const loading = ref(false)
const error = ref('')
const statuses = ref<PluginStatus[]>([])
const catalog = ref<PluginSurfaceCatalogResponse | null>(null)
const architecture = ref<PluginArchitectureCatalog | null>(null)
const selectedPluginId = ref('')
const selectedInspect = ref<PluginInspectResponse | null>(null)
const logs = ref<PluginLog[]>([])
const detailLoading = ref(false)
const detailError = ref('')
const activeTab = ref<PanelTab>('overview')
const settingsState = ref<PluginSettingsState | null>(null)
const settingsDraft = ref<JsonValue>({})
const settingsSaving = ref(false)
const operationDrafts = ref<Record<string, JsonValue>>({})
const busyOperationKey = ref('')
const lastOperationResult = ref<PluginOperationResult | null>(null)
const pluginQuery = ref('')
const transportFilter = ref('')
const stateFilter = ref('')

const sortedStatuses = computed(() => [...statuses.value].sort((left, right) => left.plugin_id.localeCompare(right.plugin_id)))
const transportOptions = computed(() =>
  [...new Set(sortedStatuses.value.map((status) => status.kind).filter(Boolean))].map((value) => ({ value, label: value })),
)
const stateOptions = computed(() =>
  [...new Set(sortedStatuses.value.map((status) => status.state).filter(Boolean))].map((value) => ({ value, label: value })),
)
const filteredStatuses = computed(() => {
  const query = pluginQuery.value.trim().toLowerCase()
  return sortedStatuses.value.filter((status) => {
    if (transportFilter.value && status.kind !== transportFilter.value) return false
    if (stateFilter.value && status.state !== stateFilter.value) return false
    if (!query) return true
    return `${status.plugin_id}
${status.kind}
${status.state}`.toLowerCase().includes(query)
  })
})
const selectedStatus = computed(() => statuses.value.find((status) => status.plugin_id === selectedPluginId.value) || null)
const selectedPlugin = computed(() => selectedInspect.value?.plugin || null)
const selectedManifest = computed(() => selectedPlugin.value?.manifest || null)
const selectedActivation = computed(() => selectedPlugin.value?.activation || null)
const selectedAuthority = computed(() => selectedPlugin.value?.authority || null)
const selectedArchitectureNode = computed(
  () => architecture.value?.plugins?.find((node) => node.plugin_id === selectedPluginId.value) || null,
)
const selectedReloadDecision = computed(
  () => architecture.value?.reload?.decisions?.find((decision) => decision.plugin_id === selectedPluginId.value) || null,
)
const appliedProfiles = computed(() => architecture.value?.profiles?.applied_profiles || [])
const selectedProfileChanges = computed(() =>
  (architecture.value?.profiles?.changes || []).filter((change) => change.plugin_id === selectedPluginId.value),
)
const selectedBlocked = computed(
  () => selectedActivation.value?.blocked || selectedArchitectureNode.value?.blocked || null,
)
const selectedServiceExports = computed(() =>
  Array.isArray(selectedPlugin.value?.services?.exports)
    ? selectedPlugin.value!.services!.exports!
    : Array.isArray(selectedArchitectureNode.value?.service_exports)
      ? selectedArchitectureNode.value!.service_exports!
      : [],
)
const selectedServiceImports = computed(() =>
  Array.isArray(selectedPlugin.value?.services?.imports) ? selectedPlugin.value!.services!.imports! : [],
)
const selectedEffects = computed<PluginEffectDescriptor[]>(() => {
  if (Array.isArray(selectedPlugin.value?.effects?.effects)) return selectedPlugin.value!.effects!.effects!
  return (architecture.value?.effects || []).filter((effect) => effect.plugin_id === selectedPluginId.value)
})
const selectedEffectLifecycle = computed(() => selectedPlugin.value?.effects?.lifecycle || 'not_started')
const selectedPipelineHandlers = computed(() =>
  (architecture.value?.pipelines || [])
    .map((pipeline) => ({
      ...pipeline,
      handlers: (pipeline.handlers || []).filter((handler) => handler.owner === selectedPluginId.value),
    }))
    .filter((pipeline) => pipeline.handlers.length > 0),
)
const selectedOperationRegistrations = computed(() =>
  (architecture.value?.operation_registrations || []).filter((entry) => entry.owner === selectedPluginId.value),
)
const selectedToolRegistrations = computed(() =>
  (architecture.value?.tool_registrations || []).filter((entry) => entry.owner === selectedPluginId.value),
)
const selectedIncomingDependencies = computed(() =>
  (architecture.value?.dependencies || []).filter((edge) => edge.consumer_id === selectedPluginId.value),
)
const selectedOutgoingDependencies = computed(() =>
  (architecture.value?.dependencies || []).filter((edge) => edge.provider_id === selectedPluginId.value),
)
const allOperations = computed(() => (Array.isArray(catalog.value?.catalog?.operations) ? catalog.value!.catalog!.operations! : []))
const selectedOperations = computed(() => allOperations.value.filter((operation) => operation.plugin_id === selectedPluginId.value))
const selectedTools = computed(() => (Array.isArray(selectedManifest.value?.tools) ? selectedManifest.value!.tools! : []))
const settingsDiagnostics = computed(() => (Array.isArray(settingsState.value?.diagnostics) ? settingsState.value!.diagnostics! : []))
const settingsDirty = computed(() => {
  if (!settingsState.value) return false
  return JSON.stringify(settingsDraft.value) !== JSON.stringify(settingsState.value.effective)
})
const activeSessionId = computed(() => {
  const id = Number(chat.selectedSessionId)
  return Number.isSafeInteger(id) && id > 0 ? id : null
})

watch(
  selectedOperations,
  (operations) => {
    const next: Record<string, JsonValue> = {}
    for (const operation of operations) {
      const key = operationKey(operation)
      next[key] = Object.prototype.hasOwnProperty.call(operationDrafts.value, key)
        ? operationDrafts.value[key]
        : clonePluginJson(operation.default_input)
    }
    operationDrafts.value = next
  },
  { immediate: true },
)

function panelTabFromRequest(value: unknown): PanelTab | null {
  const tab = String(value || '').trim() as PanelTab
  return PANEL_TAB_IDS.has(tab) ? tab : null
}

function statusFailure(status: PluginStatus | null): string {
  return String(status?.last_failure?.user?.fallback || status?.last_failure?.rendered || status?.last_failure?.message || '').trim()
}

function statusTone(state: string): string {
  if (state === 'running') return 'text-success'
  if (state === 'restarting') return 'text-warning'
  if (state === 'stopped') return 'text-muted-foreground'
  return 'text-destructive'
}

function preferredPluginId(): string {
  const contributedIds = new Set(allOperations.value.map((operation) => operation.plugin_id))
  return sortedStatuses.value.find((status) => contributedIds.has(status.plugin_id))?.plugin_id || sortedStatuses.value[0]?.plugin_id || ''
}

function operationKey(operation: PluginOperationCatalogItem): string {
  return `${operation.plugin_id}:${operation.id}`
}

function operationValue(operation: PluginOperationCatalogItem): JsonValue {
  const current = operationDrafts.value[operationKey(operation)]
  return current === undefined ? clonePluginJson(operation.default_input) : current
}

function setOperationValue(operation: PluginOperationCatalogItem, value: JsonValue) {
  operationDrafts.value = { ...operationDrafts.value, [operationKey(operation)]: value }
}

function resultPayload(value: JsonValue | undefined): string {
  if (value === undefined || value === null) return ''
  return JSON.stringify(value, null, 2)
}

function showOperationFeedback(result: PluginOperationResult) {
  const message = [result.title, result.summary].filter((value) => String(value || '').trim()).join(': ')
  if (result.status === 'succeeded') toasts.push('success', message || st('Plugin operation completed'))
  else if (result.status === 'failed') toasts.push('error', result.detail?.trim() || message || st('Plugin operation failed'))
  else toasts.push('info', result.detail?.trim() || message || st('Plugin operation {status}', { status: result.status }))
}

async function applyOperationEffect(effect: PluginHostEffect) {
  if (effect.kind === 'navigate') {
    if (!effect.path.startsWith('/')) throw new Error(st('Plugin navigation must use an application-relative path.'))
    await router.push(effect.path)
    return
  }
  if (effect.kind === 'open_url') {
    const url = new URL(effect.url, window.location.href)
    if (url.protocol !== 'http:' && url.protocol !== 'https:') throw new Error(st('Plugin URLs must use HTTP or HTTPS.'))
    window.open(url.toString(), '_blank', 'noopener,noreferrer')
    return
  }
  if (effect.kind === 'insert_prompt') {
    const sessionId = String(chat.selectedSessionId || '').trim()
    if (!sessionId) {
      toasts.push('info', st('Open a chat session before inserting this prompt.'))
      return
    }
    const previous = chat.getComposerDraft(sessionId).trim()
    chat.setComposerDraft(sessionId, previous ? `${previous}\n${effect.prompt}` : effect.prompt)
    await router.push('/chat')
    return
  }
  if (effect.kind === 'refresh_plugin_surface') await loadSelectedPlugin()
}

async function runOperation(operation: PluginOperationCatalogItem) {
  const key = operationKey(operation)
  if (busyOperationKey.value) return
  busyOperationKey.value = key
  lastOperationResult.value = null
  try {
    const response = await apiJson<{ result: PluginOperationResult }>(
      `/api/v1/plugins/${encodeURIComponent(operation.plugin_id)}/operations/${encodeURIComponent(operation.id)}/invoke`,
      {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          input: operationValue(operation),
          session_id: activeSessionId.value,
          slash: operation.slash || null,
          raw: '',
        }),
      },
    )
    if (!response?.result) throw new Error(st('The server omitted the plugin operation result.'))
    lastOperationResult.value = response.result
    showOperationFeedback(response.result)
    for (const effect of response.result.effects || []) await applyOperationEffect(effect)
  } catch (err) {
    toasts.push('error', err instanceof Error ? err.message : String(err))
  } finally {
    busyOperationKey.value = ''
  }
}

async function saveSettings() {
  if (!settingsState.value || settingsSaving.value) return
  settingsSaving.value = true
  try {
    const response = await apiJson<PluginSettingsUpdateResponse>(
      `/api/v1/plugins/${encodeURIComponent(settingsState.value.plugin_id)}/settings`,
      {
        method: 'PUT',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ value: settingsDraft.value }),
      },
    )
    settingsState.value = response.settings
    settingsDraft.value = clonePluginJson(response.settings.effective)
    toasts.push('success', response.reload_required ? st('Plugin settings saved and Runtime reloaded') : st('Plugin settings saved'))
    await refresh()
  } catch (err) {
    toasts.push('error', err instanceof Error ? err.message : String(err))
  } finally {
    settingsSaving.value = false
  }
}

function resetSettings() {
  if (settingsState.value) settingsDraft.value = clonePluginJson(settingsState.value.defaults)
}

async function loadSelectedPlugin() {
  const id = selectedPluginId.value
  selectedInspect.value = null
  logs.value = []
  settingsState.value = null
  settingsDraft.value = {}
  detailError.value = ''
  lastOperationResult.value = null
  if (!id) return
  detailLoading.value = true
  try {
    const [inspect, logResponse] = await Promise.all([
      apiJson<PluginInspectResponse>(`/api/v1/plugins/${encodeURIComponent(id)}`),
      apiJson<PluginLogsResponse>(`/api/v1/plugins/${encodeURIComponent(id)}/logs?limit=200`),
    ])
    selectedInspect.value = inspect
    logs.value = Array.isArray(logResponse?.logs) ? logResponse.logs : []
    if (inspect.plugin?.manifest?.settings) {
      const loadedSettings = await apiJson<PluginSettingsState>(
        `/api/v1/plugins/${encodeURIComponent(id)}/settings`,
      )
      settingsState.value = loadedSettings
      settingsDraft.value = clonePluginJson(loadedSettings.effective)
    }
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
    const [statusData, catalogData, architectureData] = await Promise.all([
      apiJson<PluginStatusListResponse | PluginStatus[]>('/api/v1/plugins'),
      apiJson<PluginSurfaceCatalogResponse>('/api/v1/plugins/surface'),
      apiJson<PluginArchitectureCatalog>('/api/v1/plugins/architecture'),
    ])
    statuses.value = Array.isArray(statusData) ? statusData : Array.isArray(statusData?.items) ? statusData.items : []
    catalog.value = catalogData && typeof catalogData === 'object' ? catalogData : null
    architecture.value = architectureData && typeof architectureData === 'object' ? architectureData : null
    const ids = new Set(statuses.value.map((status) => status.plugin_id))
    const requested = String(route.query.plugin || '').trim()
    const target = requested && ids.has(requested) ? requested : ids.has(selectedPluginId.value) ? selectedPluginId.value : preferredPluginId()
    if (selectedPluginId.value === target) await loadSelectedPlugin()
    else selectedPluginId.value = target
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
    statuses.value = []
    catalog.value = null
    architecture.value = null
  } finally {
    loading.value = false
  }
}

async function selectPlugin(pluginId: string) {
  selectedPluginId.value = pluginId
  await router.replace({ query: { ...route.query, plugin: pluginId, pluginTab: activeTab.value } })
}

async function selectTab(tab: PanelTab) {
  activeTab.value = tab
  await router.replace({ query: { ...route.query, plugin: selectedPluginId.value, pluginTab: tab } })
}

watch(selectedPluginId, () => {
  activeTab.value = panelTabFromRequest(route.query.pluginTab) || 'overview'
  void loadSelectedPlugin()
})

watch(
  () => [route.query.plugin, route.query.pluginTab] as const,
  ([plugin, tab]) => {
    const requested = String(plugin || '').trim()
    if (requested && statuses.value.some((status) => status.plugin_id === requested)) selectedPluginId.value = requested
    const requestedTab = panelTabFromRequest(tab)
    if (requestedTab) activeTab.value = requestedTab
  },
)

onMounted(() => void refresh())
</script>

<template>
  <div class="space-y-6">
    <div class="flex items-start justify-between gap-3">
      <div>
        <div class="text-lg font-medium">{{ $st('Plugin Workbench') }}</div>
        <div class="mt-1 text-sm text-muted-foreground">
          {{ $st('Dependency-aware lifecycle, shared settings contracts, server-owned operations, tools, logs and diagnostics.') }}
        </div>
      </div>
      <IconButton
        variant="outline"
        size="md"
        :tooltip="loading ? $st('Refreshing plugins') : $st('Refresh plugins')"
        :aria-label="loading ? $st('Refreshing plugins') : $st('Refresh plugins')"
        :disabled="loading"
        @click="refresh"
      >
        <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
      </IconButton>
    </div>

    <div v-if="error" class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
      {{ error }}
    </div>

    <section class="grid gap-3 rounded-lg border border-border/60 bg-muted/10 p-3 md:grid-cols-[minmax(0,1fr)_12rem_12rem]">
      <SearchInput
        v-model="pluginQuery"
        :placeholder="$st('Search plugins')"
        :show-search-button="false"
        :input-aria-label="$st('Search plugins')"
      />
      <OptionPicker
        v-model="transportFilter"
        :options="transportOptions"
        :title="$st('Transport filter')"
        :empty-label="$st('All transports')"
      />
      <OptionPicker
        v-model="stateFilter"
        :options="stateOptions"
        :title="$st('State filter')"
        :empty-label="$st('All states')"
      />
    </section>
    <div v-if="loading && statuses.length === 0" class="text-sm text-muted-foreground">{{ $st('Loading plugins…') }}</div>
    <div v-else-if="statuses.length === 0" class="text-sm text-muted-foreground">{{ $st('No plugins are loaded.') }}</div>

    <div v-else class="grid min-h-[38rem] grid-cols-1 border-y border-border/60 md:grid-cols-[15rem_minmax(0,1fr)]">
      <nav class="border-b border-border/60 py-3 md:border-b-0 md:border-r md:pr-3" :aria-label="$st('Configured plugins')">
        <button
          v-for="status in filteredStatuses"
          :key="status.plugin_id"
          type="button"
          class="flex w-full items-start justify-between gap-2 rounded px-2.5 py-2 text-left hover:bg-muted/40"
          :class="selectedPluginId === status.plugin_id ? 'bg-muted/60 text-foreground' : 'text-foreground/80'"
          @click="selectPlugin(status.plugin_id)"
        >
          <span class="min-w-0">
            <span class="block truncate font-mono text-xs font-semibold">{{ status.plugin_id }}</span>
            <span class="mt-0.5 block text-[11px] text-muted-foreground">{{ status.kind }}</span>
          </span>
          <span class="mt-0.5 h-2 w-2 shrink-0 rounded-full bg-current" :class="statusTone(status.state)" />
        </button>
        <div v-if="filteredStatuses.length === 0" class="px-3 py-8 text-center text-xs text-muted-foreground">
          {{ $st('No matching plugins.') }}
        </div>
      </nav>

      <div class="min-w-0 py-4 md:pl-5">
        <div v-if="detailLoading" class="text-sm text-muted-foreground">{{ $st('Loading plugin details…') }}</div>
        <div v-else-if="detailError" class="break-words text-sm text-destructive">{{ detailError }}</div>
        <template v-else-if="selectedStatus">
          <header class="flex flex-wrap items-start justify-between gap-3">
            <div class="min-w-0">
              <h2 class="break-all font-mono text-base font-semibold">{{ selectedPluginId }}</h2>
              <p v-if="selectedManifest?.summary" class="mt-1 text-sm text-muted-foreground">{{ selectedManifest.summary }}</p>
              <div class="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
                <span :class="statusTone(selectedStatus.state)">{{ selectedStatus.state }}</span>
                <span>{{ $st('transport:') }} {{ selectedStatus.kind }}</span>
                <span v-if="selectedManifest?.version">{{ $st('version:') }} {{ selectedManifest.version }}</span>
                <span>{{ selectedOperations.length }} {{ $st('operations') }}</span>
                <span>{{ selectedTools.length }} {{ $st('tools') }}</span>
                <span v-if="catalog?.tool_registry_generation !== undefined">{{ $st('tool registry:') }} {{ catalog.tool_registry_generation }}</span>
              </div>
            </div>
          </header>

          <div class="mt-5 flex gap-1 overflow-x-auto border-b border-border/60" role="tablist">
            <button
              v-for="tab in PANEL_TABS"
              :key="tab.id"
              type="button"
              role="tab"
              :aria-selected="activeTab === tab.id"
              class="shrink-0 border-b-2 px-3 py-2 text-xs font-medium"
              :class="activeTab === tab.id ? 'border-primary text-foreground' : 'border-transparent text-muted-foreground'"
              @click="selectTab(tab.id)"
            >
              {{ tab.label }}
            </button>
          </div>

          <div class="mt-5">
            <section v-if="activeTab === 'overview'" class="space-y-6">
              <div v-if="selectedBlocked" class="rounded-md border border-destructive/30 bg-destructive/5 p-4">
                <div class="font-mono text-xs font-semibold text-destructive">{{ selectedBlocked.code }}</div>
                <div class="mt-2 text-sm text-destructive">{{ selectedBlocked.message }}</div>
                <div v-if="selectedBlocked.dependencies?.length" class="mt-2 text-xs text-destructive/80">
                  {{ $st('Dependencies:') }} {{ selectedBlocked.dependencies.join(', ') }}
                </div>
              </div>

              <div v-if="appliedProfiles.length" class="rounded-md border border-border/60 bg-muted/20 p-4">
                <div class="text-xs font-medium text-muted-foreground">{{ $st('Applied plugin profiles') }}</div>
                <div class="mt-2 flex flex-wrap gap-1.5">
                  <span v-for="profile in appliedProfiles" :key="profile" class="rounded bg-muted px-2 py-1 font-mono text-[10px]">
                    {{ profile }}
                  </span>
                </div>
                <div v-if="selectedProfileChanges.length" class="mt-3 divide-y divide-border/50 rounded-md border border-border/50">
                  <div
                    v-for="change in selectedProfileChanges"
                    :key="`${change.profile}:${change.action}`"
                    class="px-3 py-2 text-xs"
                  >
                    <div class="flex flex-wrap items-center justify-between gap-2">
                      <span class="font-mono">{{ change.profile }}</span>
                      <span class="text-muted-foreground">{{ change.action }}</span>
                    </div>
                    <div v-if="change.paths?.length" class="mt-2 flex flex-wrap gap-1.5">
                      <span v-for="path in change.paths" :key="path" class="rounded bg-background px-2 py-1 font-mono text-[10px] text-muted-foreground">
                        {{ path }}
                      </span>
                    </div>
                  </div>
                </div>
              </div>

              <div v-if="selectedReloadDecision" class="rounded-md border border-border/60 bg-muted/20 p-4">
                <div class="flex flex-wrap items-center justify-between gap-2">
                  <div class="text-xs font-medium text-muted-foreground">{{ $st('Current reload decision') }}</div>
                  <span class="rounded bg-muted px-2 py-1 font-mono text-[10px]">{{ selectedReloadDecision.action }}</span>
                </div>
                <div class="mt-2 text-xs text-muted-foreground">
                  {{ selectedReloadDecision.reasons?.length ? selectedReloadDecision.reasons.join(' · ') : $st('No restart reason; the transport is reusable.') }}
                </div>
                <div v-if="selectedReloadDecision.triggered_by?.length" class="mt-2 text-xs text-muted-foreground">
                  {{ $st('Triggered by:') }} <span class="font-mono">{{ selectedReloadDecision.triggered_by.join(', ') }}</span>
                </div>
              </div>

              <dl class="grid gap-x-6 gap-y-4 sm:grid-cols-2">
                <div>
                  <dt class="text-xs text-muted-foreground">{{ $st('Required plugins') }}</dt>
                  <dd class="mt-1 text-sm">{{ selectedActivation?.requires?.join(', ') || $st('None') }}</dd>
                </div>
                <div>
                  <dt class="text-xs text-muted-foreground">{{ $st('Load after') }}</dt>
                  <dd class="mt-1 text-sm">{{ selectedActivation?.after?.join(', ') || $st('None') }}</dd>
                </div>
                <div>
                  <dt class="text-xs text-muted-foreground">{{ $st('Activation epoch') }}</dt>
                  <dd class="mt-1 break-all font-mono text-xs">{{ selectedArchitectureNode?.activation_epoch || $st('Not computed') }}</dd>
                </div>
                <div>
                  <dt class="text-xs text-muted-foreground">{{ $st('Effect scope') }}</dt>
                  <dd class="mt-1 font-mono text-sm">{{ selectedEffectLifecycle }}</dd>
                </div>
                <div>
                  <dt class="text-xs text-muted-foreground">{{ $st('Trust level') }}</dt>
                  <dd class="mt-1 font-mono text-sm">{{ selectedAuthority?.trust_level || $st('Not reported') }}</dd>
                </div>
                <div>
                  <dt class="text-xs text-muted-foreground">{{ $st('Authors') }}</dt>
                  <dd class="mt-1 text-sm">{{ selectedManifest?.authors?.join(', ') || $st('Not reported') }}</dd>
                </div>
                <div>
                  <dt class="text-xs text-muted-foreground">{{ $st('Provenance') }}</dt>
                  <dd class="mt-1 text-xs">{{ selectedAuthority?.provenance?.join(' · ') || $st('Not reported') }}</dd>
                </div>
                <div>
                  <dt class="text-xs text-muted-foreground">{{ $st('Restarts') }}</dt>
                  <dd class="mt-1 font-mono text-sm">{{ selectedStatus.restart_count }}</dd>
                </div>
              </dl>

              <div v-if="selectedIncomingDependencies.length || selectedOutgoingDependencies.length" class="grid gap-4 lg:grid-cols-2">
                <div v-if="selectedIncomingDependencies.length">
                  <h3 class="text-xs font-medium text-muted-foreground">{{ $st('Depends on') }}</h3>
                  <div class="mt-2 divide-y divide-border/50 rounded-md border border-border/60">
                    <div
                      v-for="edge in selectedIncomingDependencies"
                      :key="`${edge.provider_id}:${edge.kind}:${edge.service_id || ''}:${edge.api_version || ''}`"
                      class="px-3 py-2 text-xs"
                    >
                      <div class="flex flex-wrap items-center justify-between gap-2">
                        <span class="font-mono">{{ edge.provider_id }}</span>
                        <span class="text-muted-foreground">{{ edge.kind }}</span>
                      </div>
                      <div v-if="edge.service_id" class="mt-1 font-mono text-[10px] text-muted-foreground">
                        {{ edge.service_id }}@v{{ edge.api_version }}
                      </div>
                    </div>
                  </div>
                </div>
                <div v-if="selectedOutgoingDependencies.length">
                  <h3 class="text-xs font-medium text-muted-foreground">{{ $st('Required by') }}</h3>
                  <div class="mt-2 divide-y divide-border/50 rounded-md border border-border/60">
                    <div
                      v-for="edge in selectedOutgoingDependencies"
                      :key="`${edge.consumer_id}:${edge.kind}:${edge.service_id || ''}:${edge.api_version || ''}`"
                      class="px-3 py-2 text-xs"
                    >
                      <div class="flex flex-wrap items-center justify-between gap-2">
                        <span class="font-mono">{{ edge.consumer_id }}</span>
                        <span class="text-muted-foreground">{{ edge.kind }}</span>
                      </div>
                      <div v-if="edge.service_id" class="mt-1 font-mono text-[10px] text-muted-foreground">
                        {{ edge.service_id }}@v{{ edge.api_version }}
                      </div>
                    </div>
                  </div>
                </div>
              </div>

              <div v-if="selectedServiceImports.length || selectedServiceExports.length" class="grid gap-4 lg:grid-cols-2">
                <div v-if="selectedServiceImports.length">
                  <h3 class="text-xs font-medium text-muted-foreground">{{ $st('Service imports') }}</h3>
                  <div class="mt-2 divide-y divide-border/50 rounded-md border border-border/60">
                    <div
                      v-for="service in selectedServiceImports"
                      :key="`${service.id}:${service.api_version}`"
                      class="px-3 py-2 text-xs"
                    >
                      <div class="flex flex-wrap items-center justify-between gap-2">
                        <span class="font-mono">{{ service.id }}@v{{ service.api_version }}</span>
                        <span class="font-mono text-muted-foreground">{{ service.state }}</span>
                      </div>
                      <div class="mt-1 text-muted-foreground">
                        {{ service.resolved_provider || service.provider || $st('No provider bound') }} · {{ service.optional ? $st('optional') : $st('required') }}
                      </div>
                      <div v-if="service.methods?.length" class="mt-2 flex flex-wrap gap-1.5">
                        <span v-for="method in service.methods" :key="method.id" class="rounded bg-muted px-2 py-1 font-mono text-[10px]">
                          {{ method.id }}
                        </span>
                      </div>
                    </div>
                  </div>
                </div>
                <div v-if="selectedServiceExports.length">
                  <h3 class="text-xs font-medium text-muted-foreground">{{ $st('Service exports') }}</h3>
                  <div class="mt-2 divide-y divide-border/50 rounded-md border border-border/60">
                    <div
                      v-for="service in selectedServiceExports"
                      :key="`${service.id}:${service.api_version}`"
                      class="px-3 py-2 text-xs"
                    >
                      <div class="flex flex-wrap items-center justify-between gap-2">
                        <span class="font-mono">{{ service.id }}@v{{ service.api_version }}</span>
                        <span class="text-muted-foreground">{{ $st('provider') }}</span>
                      </div>
                      <div v-if="service.methods?.length" class="mt-2 flex flex-wrap gap-1.5">
                        <span v-for="method in service.methods" :key="method.id" class="rounded bg-muted px-2 py-1 font-mono text-[10px]">
                          {{ method.id }}
                        </span>
                      </div>
                    </div>
                  </div>
                </div>
              </div>

              <div
                v-if="selectedPipelineHandlers.length || selectedToolRegistrations.length || selectedOperationRegistrations.length"
                class="grid gap-4 lg:grid-cols-2"
              >
                <div v-if="selectedPipelineHandlers.length">
                  <h3 class="text-xs font-medium text-muted-foreground">{{ $st('Typed pipeline handlers') }}</h3>
                  <div class="mt-2 divide-y divide-border/50 rounded-md border border-border/60">
                    <div v-for="pipeline in selectedPipelineHandlers" :key="pipeline.definition.id" class="px-3 py-2 text-xs">
                      <div class="flex flex-wrap items-center justify-between gap-2">
                        <span class="font-mono">{{ pipeline.definition.id }}</span>
                        <span class="text-muted-foreground">
                          {{ pipeline.definition.mode }}
                          · {{ pipeline.definition.durable ? $st('durable') : $st('live') }}
                          · {{ pipeline.definition.scoped ? $st('scoped') : $st('global') }}
                          <template v-if="pipeline.failure_policy"> · {{ pipeline.failure_policy }} {{ $st('on error') }}</template>
                        </span>
                      </div>
                      <div class="mt-2 space-y-1">
                        <div v-for="handler in pipeline.handlers" :key="handler.registration" class="flex items-center justify-between gap-2">
                          <span>{{ handler.id }}</span>
                          <span class="font-mono text-[10px] text-muted-foreground">{{ $st('priority') }} {{ handler.priority }}</span>
                        </div>
                      </div>
                    </div>
                  </div>
                </div>
                <div v-if="selectedToolRegistrations.length">
                  <h3 class="text-xs font-medium text-muted-foreground">{{ $st('Scoped tool registrations') }}</h3>
                  <div class="mt-2 divide-y divide-border/50 rounded-md border border-border/60">
                    <div v-for="entry in selectedToolRegistrations" :key="`${entry.generation}:${entry.key}`" class="px-3 py-2 text-xs">
                      <div class="font-mono">{{ entry.key }}</div>
                      <div class="mt-1 text-muted-foreground">
                        {{ entry.layer.kind === 'global' ? $st('global') : entry.layer.scope }} · {{ $st('generation') }} {{ entry.generation }}
                      </div>
                    </div>
                  </div>
                </div>
                <div v-if="selectedOperationRegistrations.length">
                  <h3 class="text-xs font-medium text-muted-foreground">{{ $st('Scoped operation registrations') }}</h3>
                  <div class="mt-2 divide-y divide-border/50 rounded-md border border-border/60">
                    <div v-for="entry in selectedOperationRegistrations" :key="`${entry.generation}:${entry.key}`" class="px-3 py-2 text-xs">
                      <div class="font-mono">{{ entry.key }}</div>
                      <div class="mt-1 text-muted-foreground">
                        {{ entry.layer.kind === 'global' ? $st('global') : entry.layer.scope }} · {{ $st('generation') }} {{ entry.generation }}
                      </div>
                    </div>
                  </div>
                </div>
              </div>

              <div v-if="selectedAuthority?.plugin_capabilities?.length">
                <h3 class="text-xs font-medium text-muted-foreground">{{ $st('Plugin capabilities') }}</h3>
                <div class="mt-2 flex flex-wrap gap-1.5">
                  <span v-for="capability in selectedAuthority.plugin_capabilities" :key="capability" class="rounded bg-muted px-2 py-1 font-mono text-[11px]">
                    {{ capability }}
                  </span>
                </div>
              </div>
            </section>

            <section v-else-if="activeTab === 'settings'" class="space-y-5">
              <div v-if="!selectedManifest?.settings" class="text-sm text-muted-foreground">{{ $st('This plugin does not expose editable settings.') }}</div>
              <template v-else-if="settingsState">
                <PluginContractEditor
                  :node="settingsState.contract.root"
                  :model-value="settingsDraft"
                  :disabled="settingsSaving"
                  @update:model-value="settingsDraft = $event"
                />
                <div v-if="settingsDiagnostics.length" class="rounded-md border border-destructive/30 bg-destructive/5 p-3">
                  <div v-for="diagnostic in settingsDiagnostics" :key="`${diagnostic.path}:${diagnostic.message}`" class="text-xs text-destructive">
                    <span v-if="diagnostic.path" class="font-mono">{{ diagnostic.path }}:</span> {{ diagnostic.message }}
                  </div>
                </div>
                <div class="flex flex-wrap justify-end gap-2 border-t border-border/60 pt-4">
                  <Button variant="outline" size="sm" :disabled="settingsSaving" @click="resetSettings">{{ $st('Reset to defaults') }}</Button>
                  <Button size="sm" :disabled="settingsSaving || !settingsDirty" @click="saveSettings">
                    {{ settingsSaving ? $st('Saving...') : $st('Save settings') }}
                  </Button>
                </div>
              </template>
            </section>

            <section v-else-if="activeTab === 'operations'" class="space-y-5">
              <div v-if="selectedOperations.length === 0" class="text-sm text-muted-foreground">{{ $st('This plugin does not expose user operations.') }}</div>
              <article v-for="operation in selectedOperations" :key="operationKey(operation)" class="space-y-3 border-b border-border/60 pb-5 last:border-b-0">
                <div class="flex flex-wrap items-start justify-between gap-3">
                  <div class="min-w-0">
                    <div class="flex items-center gap-2 text-sm font-medium">
                      <RiCommandLine class="h-4 w-4 text-muted-foreground" />
                      <span>{{ operation.title }}</span>
                    </div>
                    <p v-if="operation.description" class="mt-1 text-xs text-muted-foreground">{{ operation.description }}</p>
                    <div class="mt-1 flex flex-wrap gap-x-3 text-[10px] text-muted-foreground">
                      <span class="font-mono">{{ operation.id }}</span>
                      <span v-if="operation.slash" class="font-mono">/{{ String(operation.slash).replace(/^\/+/, '') }}</span>
                      <span>{{ operation.category || operation.group }}</span>
                      <span>{{ operation.target.kind }}</span>
                    </div>
                  </div>
                  <Button size="sm" :disabled="Boolean(busyOperationKey)" @click="runOperation(operation)">
                    <RiPlayLine class="mr-2 h-4 w-4" />
                    {{ busyOperationKey === operationKey(operation) ? $st('Running...') : $st('Run') }}
                  </Button>
                </div>
                <PluginContractEditor
                  :node="operation.input.root"
                  :model-value="operationValue(operation)"
                  :disabled="Boolean(busyOperationKey)"
                  @update:model-value="setOperationValue(operation, $event)"
                />
              </article>

              <div v-if="lastOperationResult" class="rounded-md border border-border/70 p-4">
                <div class="flex flex-wrap items-center justify-between gap-2">
                  <div class="text-sm font-semibold">{{ lastOperationResult.title }}</div>
                  <span class="rounded bg-muted px-2 py-1 font-mono text-[10px]">{{ lastOperationResult.status }}</span>
                </div>
                <p v-if="lastOperationResult.summary" class="mt-2 text-sm text-muted-foreground">{{ lastOperationResult.summary }}</p>
                <pre v-if="lastOperationResult.detail" class="mt-3 max-h-72 overflow-auto whitespace-pre-wrap break-words rounded bg-muted/40 p-3 text-xs">{{ lastOperationResult.detail }}</pre>
                <pre v-if="resultPayload(lastOperationResult.output)" class="mt-3 max-h-72 overflow-auto whitespace-pre-wrap break-words rounded bg-muted/40 p-3 text-xs">{{ resultPayload(lastOperationResult.output) }}</pre>
                <div v-for="diagnostic in lastOperationResult.diagnostics || []" :key="`${diagnostic.code}:${diagnostic.path || ''}`" class="mt-2 text-xs text-destructive">
                  <span class="font-mono">{{ diagnostic.code }}</span>: {{ diagnostic.message }}
                </div>
              </div>
            </section>

            <section v-else-if="activeTab === 'tools'" class="space-y-4">
              <div v-if="selectedTools.length === 0" class="text-sm text-muted-foreground">{{ $st('This plugin does not register tools.') }}</div>
              <div v-for="tool in selectedTools" :key="String(tool.name || '')" class="border-b border-border/60 pb-4 last:border-b-0">
                <div class="font-mono text-sm font-semibold">{{ tool.name || $st('Unnamed tool') }}</div>
                <p class="mt-1 text-xs text-muted-foreground">{{ tool.summary || tool.docs?.summary || tool.description || $st('No summary.') }}</p>
                <div v-if="tool.tags?.length" class="mt-2 flex flex-wrap gap-1.5">
                  <span v-for="tag in tool.tags" :key="tag" class="rounded bg-muted px-2 py-1 font-mono text-[10px]">{{ tag }}</span>
                </div>
              </div>
            </section>

            <section v-else-if="activeTab === 'logs'" class="space-y-2">
              <div v-if="logs.length === 0" class="text-sm text-muted-foreground">{{ $st('No plugin logs recorded.') }}</div>
              <div v-for="entry in logs" :key="entry.seq" class="border-b border-border/50 py-2 last:border-b-0">
                <div class="flex flex-wrap gap-x-3 text-[10px] text-muted-foreground">
                  <span>#{{ entry.seq }}</span>
                  <span>{{ new Date(entry.timestamp_ms).toLocaleString() }}</span>
                  <span class="font-semibold uppercase">{{ entry.level }}</span>
                  <span>{{ entry.source }}</span>
                </div>
                <div class="mt-1 whitespace-pre-wrap break-words font-mono text-xs">{{ entry.message }}</div>
              </div>
            </section>

            <section v-else class="space-y-4">
              <MarkdownRenderer v-if="selectedManifest?.help" :content="selectedManifest.help" source-path="" />
              <div v-if="statusFailure(selectedStatus)" class="rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
                {{ statusFailure(selectedStatus) }}
              </div>
              <div v-if="selectedBlocked" class="rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
                {{ selectedBlocked.message }}
              </div>
              <div v-for="diagnostic in settingsDiagnostics" :key="`${diagnostic.path}:${diagnostic.message}`" class="rounded-md border border-warning/30 bg-warning/5 p-3 text-xs">
                <span v-if="diagnostic.path" class="font-mono">{{ diagnostic.path }}:</span> {{ diagnostic.message }}
              </div>
              <div v-if="selectedEffects.length" class="space-y-2">
                <h3 class="text-xs font-medium text-muted-foreground">{{ $st('Owned effects') }}</h3>
                <div v-for="effect in selectedEffects" :key="effect.id" class="rounded-md border border-border/60 px-3 py-2 text-xs">
                  <div class="flex flex-wrap items-center justify-between gap-2">
                    <span class="font-mono">#{{ effect.id }} · {{ effect.kind }}</span>
                    <span class="font-mono text-muted-foreground">{{ effect.state }}</span>
                  </div>
                  <div class="mt-1 text-muted-foreground">{{ effect.label }}</div>
                  <div v-if="effect.error" class="mt-1 text-destructive">{{ effect.error }}</div>
                </div>
              </div>
              <div v-if="!statusFailure(selectedStatus) && !selectedBlocked && settingsDiagnostics.length === 0 && selectedEffects.every((effect) => effect.state !== 'failed')" class="text-sm text-muted-foreground">
                {{ $st('No plugin diagnostics are currently reported.') }}
              </div>
            </section>
          </div>
        </template>
      </div>
    </div>
  </div>
</template>
