<script setup lang="ts">
import { normalizePluginSettingsView, type PluginSettingsTab } from '@/components/settings/pluginSettingsNavigation'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, useId, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { onBeforeRouteLeave, onBeforeRouteUpdate, useRoute, useRouter } from 'vue-router'
import {
  RiArrowRightLine,
  RiCheckboxCircleLine,
  RiCommandLine,
  RiFileList2Line,
  RiInformationLine,
  RiPlayLine,
  RiPlugLine,
  RiRefreshLine,
  RiSearchLine,
  RiSettings3Line,
  RiStethoscopeLine,
  RiToolsLine,
} from '@remixicon/vue'

import MarkdownRenderer from '@/components/markdown/MarkdownRenderer.vue'
import PluginContractEditor from '@/components/settings/PluginContractEditor.vue'
import PluginEmptyState from '@/components/settings/plugins/PluginEmptyState.vue'
import SettingsSaveBar from '@/components/settings/SettingsSaveBar.vue'
import { pluginStateLabel } from '@/components/settings/pluginLabels'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import { settingsText as st } from '@/i18n/settingsText'
import { apiJson } from '@/lib/api'
import { confirmAction } from '@/lib/appConfirm'
import {
  localizePluginManifest,
  localizePluginSettings,
  localizedDocumentationField,
  type PluginDocumentationTranslation,
} from '@/lib/pluginDocumentation'
import {
  clonePluginJson,
  type PluginHostEffect,
  type PluginCommandCatalogItem,
  type PluginCommandResult,
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
  summary?: string | null
  translations?: Record<string, Pick<PluginDocumentationTranslation, 'summary'> | undefined>
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
  docs?: {
    summary?: string
    help?: string
    translations?: Record<string, { summary?: string | null; help?: string | null } | undefined>
  }
}

type PluginManifest = {
  namespace?: string
  name?: string
  version?: string
  summary?: string | null
  help?: string | null
  translations?: Record<string, PluginDocumentationTranslation | undefined>
  authors?: string[]
  transports?: string[]
  tools?: PluginTool[]
  commands?: Array<{ id?: string; title?: string }>
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
      settings?: JsonValue
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
  command_registrations?: PluginScopedRegistration[]
}

type PluginSurfaceCatalogResponse = {
  catalog?: {
    commands?: PluginCommandCatalogItem[]
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
type PanelTab = PluginSettingsTab

const props = defineProps<{ tab: PanelTab }>()
const activeTab = computed(() => props.tab)

const route = useRoute()
const router = useRouter()
const { locale } = useI18n()
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
const settingsState = ref<PluginSettingsState | null>(null)
const settingsDraft = ref<JsonValue>({})
const settingsSaving = ref(false)
const commandDrafts = ref<Record<string, JsonValue>>({})
const busyCommandKey = ref('')
const lastCommandResult = ref<PluginCommandResult | null>(null)
const lastCommandKey = ref('')
const pluginList = ref<HTMLElement | null>(null)
const pluginQuery = ref('')
const stateFilter = ref('')
const commandQuery = ref('')
const toolQuery = ref('')
const logQuery = ref('')
const logLevel = ref('')
const logsLoading = ref(false)
const logsError = ref('')
const settingsError = ref('')
const catalogError = ref('')
const architectureError = ref('')
const saveError = ref('')
const detailBody = ref<HTMLElement | null>(null)
const panelId = useId()
let detailRequest = 0
let logRequest = 0
let disposed = false
let logsLoadedPluginId = ''

const tabs = computed<Array<{ id: PanelTab; label: string; count?: number }>>(() => [
  { id: 'overview', label: st('Overview') },
  { id: 'settings', label: st('Settings') },
  { id: 'tools', label: st('Tools'), count: selectedTools.value.length },
  { id: 'commands', label: st('Commands'), count: selectedCommands.value.length },
  { id: 'logs', label: st('Logs') },
  { id: 'diagnostics', label: st('Diagnostics'), count: diagnosticCount.value || undefined },
])
const runningCount = computed(() => statuses.value.filter((status) => status.state === 'running').length)
const failedCount = computed(() => statuses.value.filter((status) => status.state === 'failed').length)
const pluginOptions = computed(() =>
  sortedStatuses.value.map((status) => ({
    value: status.plugin_id,
    label: status.plugin_id,
    description: [pluginSummaries.value.get(status.plugin_id), pluginStateLabel(status.state)]
      .filter(Boolean)
      .join(' · '),
  })),
)

const sortedStatuses = computed(() =>
  [...statuses.value].sort((left, right) => left.plugin_id.localeCompare(right.plugin_id)),
)
const pluginSummaries = computed(() => {
  const language = String(locale.value || 'en-US')
  return new Map(
    statuses.value.map((status) => {
      // Older servers may omit list documentation. The selected manifest can
      // still supply it without requesting every plugin's inspect endpoint.
      const manifest = status.plugin_id === selectedPluginId.value ? selectedInspect.value?.plugin?.manifest : null
      const summary =
        localizedDocumentationField(manifest?.summary, manifest?.translations, 'summary', language) ||
        localizedDocumentationField(status.summary, status.translations, 'summary', language)
      return [status.plugin_id, summary || '']
    }),
  )
})
const stateOptions = computed(() =>
  [...new Set(sortedStatuses.value.map((status) => status.state).filter(Boolean))].map((value) => ({
    value,
    label: pluginStateLabel(value),
  })),
)
const filteredStatuses = computed(() => {
  const query = pluginQuery.value.trim().toLowerCase()
  return sortedStatuses.value.filter((status) => {
    if (stateFilter.value && status.state !== stateFilter.value) return false
    if (!query) return true
    return `${status.plugin_id} ${pluginSummaries.value.get(status.plugin_id)} ${status.state} ${pluginStateLabel(status.state)}`
      .toLowerCase()
      .includes(query)
  })
})
const selectedStatus = computed(
  () => statuses.value.find((status) => status.plugin_id === selectedPluginId.value) || null,
)
const selectedSummary = computed(() => pluginSummaries.value.get(selectedPluginId.value))
const selectedPlugin = computed(() => selectedInspect.value?.plugin || null)
const selectedManifest = computed(() => {
  const manifest = selectedPlugin.value?.manifest
  return manifest ? localizePluginManifest(manifest, String(locale.value || 'en-US')) : null
})
const localizedSettings = computed(() =>
  settingsState.value
    ? localizePluginSettings(
        settingsState.value.contract,
        selectedPlugin.value?.manifest?.translations,
        String(locale.value || 'en-US'),
      )
    : null,
)
const selectedActivation = computed(() => selectedPlugin.value?.activation || null)
const selectedAuthority = computed(() => selectedPlugin.value?.authority || null)
const selectedArchitectureNode = computed(
  () => architecture.value?.plugins?.find((node) => node.plugin_id === selectedPluginId.value) || null,
)
const selectedReloadDecision = computed(
  () =>
    architecture.value?.reload?.decisions?.find((decision) => decision.plugin_id === selectedPluginId.value) || null,
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
const selectedCommandRegistrations = computed(() =>
  (architecture.value?.command_registrations || []).filter((entry) => entry.owner === selectedPluginId.value),
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
const allCommands = computed(() =>
  Array.isArray(catalog.value?.catalog?.commands) ? catalog.value!.catalog!.commands! : [],
)
const selectedCommands = computed(() =>
  allCommands.value.filter((command) => command.plugin_id === selectedPluginId.value),
)
const selectedTools = computed(() =>
  Array.isArray(selectedManifest.value?.tools) ? selectedManifest.value!.tools! : [],
)
const settingsDiagnostics = computed(() =>
  Array.isArray(settingsState.value?.diagnostics) ? settingsState.value!.diagnostics! : [],
)
const settingsDirty = computed(() => {
  if (!settingsState.value) return false
  return JSON.stringify(settingsDraft.value) !== JSON.stringify(settingsState.value.effective)
})
const diagnosticCount = computed(
  () =>
    Number(selectedStatus.value?.state === 'failed' || Boolean(statusFailure(selectedStatus.value))) +
    Number(Boolean(selectedBlocked.value)) +
    settingsDiagnostics.value.length +
    selectedEffects.value.filter((effect) => effect.state === 'failed').length,
)
const filteredCommands = computed(() =>
  selectedCommands.value.filter((command) =>
    matchesQuery(commandQuery.value, [command.id, command.title, command.slash, command.docs?.summary]),
  ),
)
const filteredTools = computed(() =>
  selectedTools.value.filter((tool) =>
    matchesQuery(toolQuery.value, [
      tool.name,
      tool.summary,
      tool.docs?.summary,
      tool.description,
      ...(tool.tags || []),
    ]),
  ),
)
const logLevelOptions = computed(() =>
  [...new Set(logs.value.map((entry) => entry.level.toLowerCase()))].map((level) => ({
    value: level,
    label: level.toUpperCase(),
  })),
)
const filteredLogs = computed(() =>
  logs.value.filter(
    (entry) =>
      (!logLevel.value || entry.level.toLowerCase() === logLevel.value) &&
      matchesQuery(logQuery.value, [entry.message, entry.source, entry.level, resultPayload(entry.fields)]),
  ),
)
const overviewActions = computed(() => [
  {
    tab: 'settings' as const,
    label: st('Settings'),
    icon: RiSettings3Line,
    hint: selectedManifest.value?.settings ? st('Configure this plugin') : st('No editable settings'),
  },
  {
    tab: 'tools' as const,
    label: st('Tools'),
    icon: RiToolsLine,
    hint: st('{count} tools available', { count: selectedTools.value.length }),
  },
  {
    tab: 'commands' as const,
    label: st('Commands'),
    icon: RiCommandLine,
    hint: st('{count} commands available', { count: selectedCommands.value.length }),
  },
])
const activeSessionId = computed(() => {
  const id = Number(chat.selectedSessionId)
  return Number.isSafeInteger(id) && id > 0 ? id : null
})

watch(
  selectedCommands,
  (commands) => {
    const next: Record<string, JsonValue> = {}
    for (const command of commands) {
      const key = commandKey(command)
      next[key] = Object.prototype.hasOwnProperty.call(commandDrafts.value, key)
        ? commandDrafts.value[key]
        : clonePluginJson(command.default_input)
    }
    commandDrafts.value = next
  },
  { immediate: true },
)

function statusFailure(status: PluginStatus | null): string {
  return String(
    status?.last_failure?.user?.fallback || status?.last_failure?.rendered || status?.last_failure?.message || '',
  ).trim()
}

function statusTone(state: string): string {
  if (state === 'running') return 'text-emerald-700 dark:text-emerald-400'
  if (state === 'restarting') return 'text-amber-700 dark:text-amber-400'
  if (state === 'failed') return 'text-destructive'
  return 'text-muted-foreground'
}

function matchesQuery(query: string, parts: Array<string | null | undefined>): boolean {
  return parts.filter(Boolean).join(' ').toLowerCase().includes(query.trim().toLowerCase())
}

function clearFilters() {
  pluginQuery.value = ''
  stateFilter.value = ''
}

function showFailedPlugins() {
  stateFilter.value = 'failed'
  const failed = statuses.value.find((status) => status.state === 'failed')
  if (failed) void selectPlugin(failed.plugin_id)
}

async function selectTab(tab: PanelTab) {
  await router.push({
    query: {
      ...route.query,
      view: 'plugin-workbench',
      plugin: selectedPluginId.value,
      pluginTab: tab === 'overview' ? undefined : tab,
    },
    hash: route.hash,
  })
}

function onTabKeydown(event: KeyboardEvent, index: number) {
  let next = index
  if (event.key === 'ArrowRight') next = (index + 1) % tabs.value.length
  else if (event.key === 'ArrowLeft') next = (index + tabs.value.length - 1) % tabs.value.length
  else if (event.key === 'Home') next = 0
  else if (event.key === 'End') next = tabs.value.length - 1
  else return
  event.preventDefault()
  const buttons = (event.currentTarget as HTMLElement).parentElement?.querySelectorAll<HTMLButtonElement>(
    '[role="tab"]',
  )
  buttons?.[next]?.focus()
  void selectTab(tabs.value[next]!.id)
}

async function browseMarketplace() {
  const { plugin: _plugin, pluginTab: _tab, ...query } = route.query
  await router.push({ query: { ...query, view: 'marketplace' }, hash: route.hash })
}

async function canLeavePlugin() {
  if (settingsSaving.value) return false
  if (!settingsDirty.value) return true
  return confirmAction({
    title: st('Discard unsaved plugin changes?'),
    description: st('Your changes to {plugin} have not been saved.', { plugin: selectedPluginId.value }),
    confirmText: st('Discard changes'),
    cancelText: st('Keep editing'),
  })
}

onBeforeRouteUpdate((to) => {
  if (
    to.params.section === 'plugins-tools' &&
    normalizePluginSettingsView(to.query.view || 'plugin-workbench') === 'plugin-workbench' &&
    String(to.query.plugin || selectedPluginId.value) === selectedPluginId.value
  )
    return true
  return canLeavePlugin()
})
onBeforeRouteLeave(canLeavePlugin)

function beforeUnload(event: BeforeUnloadEvent) {
  if (!settingsDirty.value) return
  event.preventDefault()
  event.returnValue = ''
}

function preferredPluginId(): string {
  const contributedIds = new Set(allCommands.value.map((command) => command.plugin_id))
  return (
    sortedStatuses.value.find((status) => contributedIds.has(status.plugin_id))?.plugin_id ||
    sortedStatuses.value[0]?.plugin_id ||
    ''
  )
}

function commandKey(command: PluginCommandCatalogItem): string {
  return `${command.plugin_id}:${command.id}`
}

function commandValue(command: PluginCommandCatalogItem): JsonValue {
  const current = commandDrafts.value[commandKey(command)]
  return current === undefined ? clonePluginJson(command.default_input) : current
}

function setCommandValue(command: PluginCommandCatalogItem, value: JsonValue) {
  commandDrafts.value = { ...commandDrafts.value, [commandKey(command)]: value }
}

function resultPayload(value: JsonValue | undefined): string {
  if (value === undefined || value === null) return ''
  return JSON.stringify(value, null, 2)
}

function showCommandFeedback(result: PluginCommandResult) {
  const message = [result.title, result.summary].filter((value) => String(value || '').trim()).join(': ')
  if (result.status === 'succeeded') toasts.push('success', message || st('Plugin command completed'))
  else if (result.status === 'failed')
    toasts.push('error', result.detail?.trim() || message || st('Plugin command failed'))
  else toasts.push('info', result.detail?.trim() || message || st('Plugin command {status}', { status: result.status }))
}

async function applyCommandEffect(effect: PluginHostEffect) {
  if (effect.kind === 'navigate') {
    if (!effect.path.startsWith('/')) throw new Error(st('Plugin navigation must use an application-relative path.'))
    await router.push(effect.path)
    return
  }
  if (effect.kind === 'open_url') {
    const url = new URL(effect.url, window.location.href)
    if (url.protocol !== 'http:' && url.protocol !== 'https:')
      throw new Error(st('Plugin URLs must use HTTP or HTTPS.'))
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

async function runCommand(command: PluginCommandCatalogItem) {
  const key = commandKey(command)
  if (busyCommandKey.value) return
  busyCommandKey.value = key
  lastCommandKey.value = key
  lastCommandResult.value = null
  try {
    const response = await apiJson<{ result: PluginCommandResult }>(
      `/api/v1/plugins/${encodeURIComponent(command.plugin_id)}/commands/${encodeURIComponent(command.id)}/invoke`,
      {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          input: commandValue(command),
          session_id: activeSessionId.value,
          slash: command.slash || null,
          raw: '',
        }),
      },
    )
    if (!response?.result) throw new Error(st('The server omitted the plugin command result.'))
    if (disposed || command.plugin_id !== selectedPluginId.value) return
    lastCommandResult.value = response.result
    showCommandFeedback(response.result)
    for (const effect of response.result.effects || []) await applyCommandEffect(effect)
  } catch (err) {
    toasts.push('error', err instanceof Error ? err.message : String(err))
  } finally {
    busyCommandKey.value = ''
  }
}

async function saveSettings() {
  if (!settingsState.value || settingsSaving.value) return
  settingsSaving.value = true
  saveError.value = ''
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
    toasts.push(
      'success',
      response.reload_required ? st('Plugin settings saved and Runtime reloaded') : st('Plugin settings saved'),
    )
    await refresh()
  } catch (err) {
    saveError.value = err instanceof Error ? err.message : String(err)
    toasts.push('error', saveError.value)
  } finally {
    settingsSaving.value = false
  }
}

function resetSettings() {
  if (settingsState.value) settingsDraft.value = clonePluginJson(settingsState.value.defaults)
}

function discardSettings() {
  if (settingsState.value) settingsDraft.value = clonePluginJson(settingsState.value.effective)
  saveError.value = ''
}

async function loadSelectedPlugin() {
  const id = selectedPluginId.value
  const request = ++detailRequest
  const isCurrent = () => !disposed && request === detailRequest && id === selectedPluginId.value
  // Refresh runtime data without replacing a draft the user is still editing.
  const preserveDraft = settingsState.value?.plugin_id === id && settingsDirty.value
  detailError.value = ''
  settingsError.value = ''
  if (!id) return
  detailLoading.value = true
  try {
    const inspect = await apiJson<PluginInspectResponse>(`/api/v1/plugins/${encodeURIComponent(id)}`)
    if (!isCurrent()) return
    if (!inspect.plugin) throw new Error(st('Plugin details are unavailable.'))
    selectedInspect.value = inspect
    if (inspect.plugin.manifest?.settings && !preserveDraft) {
      try {
        const loadedSettings = await apiJson<PluginSettingsState>(`/api/v1/plugins/${encodeURIComponent(id)}/settings`)
        if (!isCurrent()) return
        settingsState.value = loadedSettings
        settingsDraft.value = clonePluginJson(loadedSettings.effective)
      } catch (err) {
        if (isCurrent()) settingsError.value = err instanceof Error ? err.message : String(err)
      }
    }
  } catch (err) {
    if (isCurrent()) detailError.value = err instanceof Error ? err.message : String(err)
  } finally {
    if (isCurrent()) detailLoading.value = false
  }
}

async function loadLogs() {
  const id = selectedPluginId.value
  if (!id) return
  const request = ++logRequest
  const isCurrent = () => !disposed && request === logRequest && id === selectedPluginId.value
  logsLoading.value = true
  logsError.value = ''
  try {
    const response = await apiJson<PluginLogsResponse>(`/api/v1/plugins/${encodeURIComponent(id)}/logs?limit=200`)
    if (!isCurrent()) return
    logs.value = Array.isArray(response.logs) ? response.logs : []
    logsLoadedPluginId = id
  } catch (err) {
    if (isCurrent()) logsError.value = err instanceof Error ? err.message : String(err)
  } finally {
    if (isCurrent()) logsLoading.value = false
  }
}

async function refresh() {
  if (loading.value) return
  loading.value = true
  error.value = ''
  catalogError.value = ''
  architectureError.value = ''
  try {
    const [statusResult, catalogResult, architectureResult] = await Promise.allSettled([
      apiJson<PluginStatusListResponse | PluginStatus[]>('/api/v1/plugins'),
      apiJson<PluginSurfaceCatalogResponse>('/api/v1/plugins/surface'),
      apiJson<PluginArchitectureCatalog>('/api/v1/plugins/architecture'),
    ])
    if (disposed) return
    if (statusResult.status === 'rejected') throw statusResult.reason
    const statusData = statusResult.value
    statuses.value = Array.isArray(statusData) ? statusData : Array.isArray(statusData?.items) ? statusData.items : []
    if (catalogResult.status === 'fulfilled') catalog.value = catalogResult.value
    else catalogError.value = String(catalogResult.reason)
    if (architectureResult.status === 'fulfilled') architecture.value = architectureResult.value
    else architectureError.value = String(architectureResult.reason)
    const ids = new Set(statuses.value.map((status) => status.plugin_id))
    const requested = String(route.query.plugin || '').trim()
    const target =
      requested && ids.has(requested)
        ? requested
        : ids.has(selectedPluginId.value)
          ? selectedPluginId.value
          : preferredPluginId()
    if (selectedPluginId.value === target) {
      await loadSelectedPlugin()
      if (activeTab.value === 'logs') await loadLogs()
    } else selectedPluginId.value = target
    if (!disposed && requested !== target) {
      await router.replace({ query: { ...route.query, plugin: target || undefined }, hash: route.hash })
    }
  } catch (err) {
    if (!disposed) error.value = err instanceof Error ? err.message : String(err)
  } finally {
    if (!disposed) loading.value = false
  }
}

async function selectPlugin(pluginId: string) {
  if (!pluginId || pluginId === selectedPluginId.value) return
  await router.push({ query: { ...route.query, plugin: pluginId }, hash: route.hash })
}

watch(selectedPluginId, () => {
  selectedInspect.value = null
  settingsState.value = null
  settingsDraft.value = {}
  saveError.value = ''
  logs.value = []
  logsError.value = ''
  logsLoading.value = false
  logsLoadedPluginId = ''
  logRequest++
  commandQuery.value = ''
  toolQuery.value = ''
  logQuery.value = ''
  logLevel.value = ''
  lastCommandResult.value = null
  void loadSelectedPlugin()
})

watch([selectedPluginId, activeTab], async () => {
  if (activeTab.value === 'logs' && selectedPluginId.value && logsLoadedPluginId !== selectedPluginId.value) {
    void loadLogs()
  }
  await nextTick()
  detailBody.value?.scrollTo({ top: 0 })
  pluginList.value?.querySelector<HTMLElement>('[aria-current="true"]')?.scrollIntoView({ block: 'nearest' })
})

watch(
  () => route.query.plugin,
  (plugin) => {
    const requested = String(plugin || '').trim()
    if (requested && statuses.value.some((status) => status.plugin_id === requested)) selectedPluginId.value = requested
  },
)

onMounted(() => {
  window.addEventListener('beforeunload', beforeUnload)
  void refresh()
})
onBeforeUnmount(() => {
  disposed = true
  detailRequest++
  logRequest++
  window.removeEventListener('beforeunload', beforeUnload)
})
defineExpose({ refresh })
</script>

<template>
  <div class="flex min-w-0 flex-col gap-3 lg:h-full lg:min-h-0">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div class="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground" aria-live="polite">
        <span>{{ $st('{count} installed', { count: statuses.length }) }}</span>
        <span class="inline-flex items-center gap-1.5">
          <span class="h-1.5 w-1.5 rounded-full bg-emerald-500" aria-hidden="true" />
          {{ $st('{count} running', { count: runningCount }) }}
        </span>
        <button
          v-if="failedCount"
          type="button"
          class="text-destructive underline underline-offset-4"
          @click="showFailedPlugins"
        >
          {{ $st('{count} failed', { count: failedCount }) }}
        </button>
      </div>
      <div class="flex items-center gap-2">
        <Button variant="outline" size="sm" @click="browseMarketplace">
          {{ $st('Browse marketplace') }} <RiArrowRightLine class="ml-1.5 h-3.5 w-3.5" aria-hidden="true" />
        </Button>
        <IconButton
          variant="ghost"
          size="md"
          :tooltip="$st('Refresh plugins')"
          :aria-label="$st('Refresh plugins')"
          :disabled="loading || settingsSaving"
          @click="refresh"
        >
          <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
        </IconButton>
      </div>
    </div>

    <div
      v-if="error"
      role="alert"
      class="flex items-center justify-between gap-3 rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive"
    >
      <span class="min-w-0 break-words">{{ error }}</span>
      <Button variant="ghost" size="sm" :disabled="loading" @click="refresh">{{ $st('Retry') }}</Button>
    </div>
    <PluginEmptyState v-if="loading && !statuses.length" :title="$st('Loading plugins…')" :icon="RiRefreshLine" />
    <PluginEmptyState
      v-else-if="!statuses.length && !error"
      :title="$st('No plugins are loaded.')"
      :description="$st('Browse the marketplace to add plugins to Agena.')"
    >
      <Button variant="outline" size="sm" @click="browseMarketplace">{{ $st('Browse marketplace') }}</Button>
    </PluginEmptyState>

    <div
      v-else-if="statuses.length"
      class="grid min-w-0 overflow-clip rounded-xl border border-border/60 lg:min-h-0 lg:flex-1 lg:grid-cols-[15rem_minmax(0,1fr)] lg:grid-rows-[minmax(0,1fr)] lg:overflow-hidden"
      data-plugin-workbench
    >
      <aside class="hidden min-h-0 flex-col border-r border-border/60 bg-muted/10 lg:flex">
        <div class="space-y-2.5 border-b border-border/50 p-3">
          <div class="flex items-center justify-between text-xs text-muted-foreground">
            <span class="font-medium">{{ $st('Plugins') }}</span>
            <span class="tabular-nums">{{ filteredStatuses.length }} / {{ statuses.length }}</span>
          </div>
          <SearchInput
            v-model="pluginQuery"
            :placeholder="$st('Search name or purpose')"
            :show-search-button="false"
            :input-aria-label="$st('Search plugins')"
            :clear-aria-label="$st('Clear search')"
            :clear-title="$st('Clear search')"
            input-class="h-8 text-xs"
          />
          <OptionPicker
            v-model="stateFilter"
            :options="stateOptions"
            size="sm"
            :title="$st('State filter')"
            :empty-label="$st('All states')"
          />
        </div>
        <nav
          ref="pluginList"
          class="min-h-0 flex-1 space-y-1 overflow-y-auto overscroll-contain p-2"
          :aria-label="$st('Configured plugins')"
          data-plugin-list
        >
          <button
            v-for="status in filteredStatuses"
            :key="status.plugin_id"
            type="button"
            class="flex w-full items-start gap-2 rounded-lg px-3 py-2.5 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            :class="
              selectedPluginId === status.plugin_id
                ? 'bg-primary/10 text-foreground ring-1 ring-inset ring-primary/20'
                : 'text-foreground/80 hover:bg-muted/50'
            "
            :aria-current="selectedPluginId === status.plugin_id ? 'true' : undefined"
            :title="
              [`${status.plugin_id} · ${pluginStateLabel(status.state)}`, pluginSummaries.get(status.plugin_id)]
                .filter(Boolean)
                .join('\n')
            "
            @click="selectPlugin(status.plugin_id)"
          >
            <span class="min-w-0 flex-1">
              <span class="block truncate font-mono text-xs font-medium">{{ status.plugin_id }}</span>
              <span class="mt-1 line-clamp-2 break-words text-[11px] leading-relaxed text-muted-foreground">{{
                pluginSummaries.get(status.plugin_id) || pluginStateLabel(status.state)
              }}</span>
            </span>
            <span
              class="mt-1.25 h-1.5 w-1.5 shrink-0 rounded-full bg-current"
              :class="statusTone(status.state)"
              aria-hidden="true"
            />
            <span v-if="pluginSummaries.get(status.plugin_id)" class="sr-only">{{
              pluginStateLabel(status.state)
            }}</span>
          </button>
          <div v-if="!filteredStatuses.length" class="px-2 py-8 text-center text-xs text-muted-foreground">
            <RiSearchLine class="mx-auto mb-3 h-5 w-5" aria-hidden="true" />
            <p>{{ $st('No matching plugins.') }}</p>
            <Button variant="ghost" size="sm" class="mt-2" @click="clearFilters">{{ $st('Clear filters') }}</Button>
          </div>
        </nav>
      </aside>

      <div class="flex min-w-0 flex-col lg:min-h-0" data-plugin-detail>
        <div class="border-b border-border/60 p-3 lg:hidden">
          <OptionPicker
            :model-value="selectedPluginId"
            :options="pluginOptions"
            :include-empty="false"
            :title="$st('Select plugin')"
            :search-placeholder="$st('Search name or purpose')"
            @update:model-value="selectPlugin"
          />
        </div>
        <template v-if="selectedStatus">
          <header class="flex shrink-0 items-start gap-3 px-4 pb-4 pt-5 lg:px-5">
            <span
              class="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-border/60 bg-muted/30 text-primary"
              ><RiPlugLine class="h-5 w-5" aria-hidden="true"
            /></span>
            <div class="min-w-0 flex-1">
              <div class="flex flex-wrap items-center gap-x-3 gap-y-1">
                <h2 class="break-all font-mono text-base font-semibold">{{ selectedPluginId }}</h2>
                <span
                  v-if="selectedManifest?.version"
                  class="rounded-md bg-muted/50 px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
                  >v{{ selectedManifest.version }}</span
                >
              </div>
              <div class="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
                <span class="inline-flex items-center gap-1.5" :class="statusTone(selectedStatus.state)"
                  ><span class="h-1.5 w-1.5 rounded-full bg-current" aria-hidden="true" />{{
                    pluginStateLabel(selectedStatus.state)
                  }}</span
                >
              </div>
              <p v-if="selectedSummary" class="mt-2 break-words text-xs leading-relaxed text-muted-foreground">
                {{ selectedSummary }}
              </p>
            </div>
          </header>
          <div
            class="flex shrink-0 gap-1 overflow-x-auto border-b border-border/60 px-3 lg:px-4"
            role="tablist"
            :aria-label="$st('Plugin details')"
          >
            <button
              v-for="(tab, index) in tabs"
              :id="`${panelId}-tab-${tab.id}`"
              :key="tab.id"
              type="button"
              role="tab"
              :aria-selected="activeTab === tab.id"
              :aria-controls="`${panelId}-content`"
              :tabindex="activeTab === tab.id ? 0 : -1"
              class="relative flex shrink-0 items-center gap-1.5 whitespace-nowrap border-b-2 px-3 py-3 text-xs transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
              :class="
                activeTab === tab.id
                  ? 'border-primary font-medium text-foreground'
                  : 'border-transparent text-muted-foreground hover:text-foreground'
              "
              @click="selectTab(tab.id)"
              @keydown="onTabKeydown($event, index)"
            >
              {{ tab.label }}
              <span
                v-if="tab.count !== undefined"
                class="rounded bg-muted/60 px-1.5 py-0.5 text-[10px] tabular-nums text-muted-foreground"
                :class="tab.id === 'diagnostics' ? 'text-destructive' : ''"
                >{{ tab.count }}</span
              >
              <span
                v-if="tab.id === 'settings' && settingsDirty"
                class="h-1.5 w-1.5 rounded-full bg-amber-500"
                :title="$st('You have unsaved changes.')"
              />
            </button>
          </div>

          <div
            :id="`${panelId}-content`"
            ref="detailBody"
            role="tabpanel"
            :aria-labelledby="`${panelId}-tab-${activeTab}`"
            tabindex="0"
            class="min-w-0 p-4 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring lg:min-h-0 lg:flex-1 lg:overflow-y-auto lg:overscroll-contain lg:p-5"
            data-plugin-content
          >
            <PluginEmptyState v-if="detailLoading" :title="$st('Loading plugin details…')" :icon="RiRefreshLine" />
            <PluginEmptyState
              v-else-if="detailError"
              :title="$st('Plugin details are unavailable.')"
              :description="detailError"
              :icon="RiInformationLine"
            >
              <Button variant="outline" size="sm" @click="loadSelectedPlugin">{{ $st('Retry') }}</Button>
            </PluginEmptyState>
            <section v-else-if="activeTab === 'overview'" class="space-y-6">
              <button
                v-if="diagnosticCount"
                type="button"
                class="flex w-full items-center justify-between gap-3 rounded-lg border border-destructive/25 bg-destructive/5 px-3 py-3 text-left text-xs text-destructive"
                @click="selectTab('diagnostics')"
              >
                <span>{{
                  selectedBlocked?.message ||
                  statusFailure(selectedStatus) ||
                  $st('This plugin has diagnostics to review.')
                }}</span
                ><RiArrowRightLine class="h-4 w-4 shrink-0" />
              </button>
              <div class="grid gap-2 sm:grid-cols-3">
                <button
                  v-for="action in overviewActions"
                  :key="action.tab"
                  type="button"
                  class="group rounded-lg border border-border/60 p-3 text-left transition-colors hover:border-primary/30 hover:bg-muted/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  @click="selectTab(action.tab)"
                >
                  <span class="flex items-center justify-between"
                    ><component :is="action.icon" class="h-4 w-4 text-muted-foreground" /><RiArrowRightLine
                      class="h-3.5 w-3.5 text-muted-foreground/60 group-hover:text-primary"
                  /></span>
                  <span class="mt-3 block text-sm font-medium">{{ action.label }}</span>
                  <span class="mt-1 block text-[11px] text-muted-foreground">{{ action.hint }}</span>
                </button>
              </div>
              <div v-if="selectedManifest?.help" class="space-y-3">
                <h3 class="text-xs font-medium text-muted-foreground">{{ $st('About this plugin') }}</h3>
                <MarkdownRenderer :content="selectedManifest.help" source-path="" />
              </div>
              <dl class="grid gap-x-6 gap-y-4 border-t border-border/60 pt-4 text-xs sm:grid-cols-2">
                <div>
                  <dt class="text-muted-foreground">{{ $st('Authors') }}</dt>
                  <dd class="mt-1">{{ selectedManifest?.authors?.join(', ') || $st('Not reported') }}</dd>
                </div>
                <div>
                  <dt class="text-muted-foreground">{{ $st('Required plugins') }}</dt>
                  <dd class="mt-1 break-words font-mono">
                    {{ selectedActivation?.requires?.join(', ') || $st('None') }}
                  </dd>
                </div>
              </dl>
            </section>

            <section v-else-if="activeTab === 'settings'" class="min-w-0 space-y-4">
              <PluginEmptyState
                v-if="settingsError"
                :title="$st('Could not load plugin settings.')"
                :description="settingsError"
                :icon="RiSettings3Line"
              >
                <Button variant="outline" size="sm" @click="loadSelectedPlugin">{{ $st('Retry') }}</Button>
              </PluginEmptyState>
              <PluginEmptyState
                v-else-if="!selectedManifest?.settings"
                :title="$st('No editable settings')"
                :description="$st('This plugin does not expose editable settings.')"
                :icon="RiSettings3Line"
              />
              <template v-else-if="settingsState">
                <PluginContractEditor
                  :key="selectedPluginId"
                  :node="localizedSettings!.root"
                  :model-value="settingsDraft"
                  :disabled="settingsSaving"
                  @update:model-value="settingsDraft = $event"
                >
                  <template #actions>
                    <span
                      v-if="!settingsDirty && !settingsSaving && !saveError"
                      role="status"
                      class="inline-flex items-center gap-1.5 text-[11px] text-muted-foreground"
                    >
                      <RiCheckboxCircleLine class="h-3.5 w-3.5 shrink-0 text-emerald-500" aria-hidden="true" />
                      {{ $st('All changes are saved.') }}
                    </span>
                    <Button variant="ghost" size="sm" :disabled="settingsSaving" @click="resetSettings">{{
                      $st('Reset to defaults')
                    }}</Button>
                  </template>
                </PluginContractEditor>
                <div
                  v-if="settingsDiagnostics.length"
                  class="rounded-lg border border-destructive/30 bg-destructive/5 p-3"
                >
                  <div
                    v-for="diagnostic in settingsDiagnostics"
                    :key="`${diagnostic.path}:${diagnostic.message}`"
                    class="text-xs text-destructive"
                  >
                    <span v-if="diagnostic.path" class="font-mono">{{ diagnostic.path }}:</span>
                    {{ diagnostic.message }}
                  </div>
                </div>
                <SettingsSaveBar
                  v-if="settingsDirty || settingsSaving || saveError"
                  :dirty="settingsDirty"
                  :saving="settingsSaving"
                  :error="saveError"
                  :save-label="$st('Save settings')"
                  sticky
                  @save="saveSettings"
                  @discard="discardSettings"
                />
              </template>
            </section>

            <section v-else-if="activeTab === 'commands'" class="space-y-3">
              <div
                v-if="catalogError"
                role="alert"
                class="rounded-lg border border-destructive/30 p-3 text-xs text-destructive"
              >
                {{ catalogError }}
                <Button variant="ghost" size="sm" :disabled="loading" @click="refresh">{{ $st('Retry') }}</Button>
              </div>
              <PluginEmptyState
                v-if="!selectedCommands.length && !catalogError"
                :title="$st('No commands available')"
                :description="$st('This plugin does not expose user commands.')"
                :icon="RiCommandLine"
              />
              <SearchInput
                v-if="selectedCommands.length"
                v-model="commandQuery"
                :placeholder="$st('Search commands')"
                :show-search-button="false"
                :input-aria-label="$st('Search commands')"
                :clear-aria-label="$st('Clear search')"
                :clear-title="$st('Clear search')"
              />
              <PluginEmptyState
                v-if="selectedCommands.length && !filteredCommands.length"
                :title="$st('No matching commands.')"
                :description="$st('Try another search term.')"
                :icon="RiSearchLine"
              />
              <details
                v-for="command in filteredCommands"
                :key="commandKey(command)"
                class="group rounded-lg border border-border/60 open:bg-muted/10"
              >
                <summary
                  class="cursor-pointer rounded-lg px-3 py-3 text-sm marker:text-muted-foreground hover:bg-muted/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  <span class="ml-1 font-medium">{{ command.title }}</span
                  ><span v-if="command.slash" class="ml-2 font-mono text-[11px] text-muted-foreground"
                    >/{{ String(command.slash).replace(/^\/+/, '') }}</span
                  >
                  <p v-if="command.docs?.summary" class="mt-1 pl-4 text-xs leading-relaxed text-muted-foreground">
                    {{ command.docs.summary }}
                  </p>
                </summary>
                <div class="space-y-3 border-t border-border/50 p-3">
                  <div class="flex flex-wrap items-center justify-between gap-3">
                    <div class="flex flex-wrap gap-x-3 text-[11px] text-muted-foreground">
                      <span class="font-mono">{{ command.id }}</span
                      ><span>{{ command.category || command.group }}</span
                      ><span>{{ command.target.kind }}</span>
                    </div>
                    <Button size="sm" :disabled="Boolean(busyCommandKey)" @click="runCommand(command)"
                      ><RiPlayLine class="mr-1.5 h-4 w-4" />{{
                        busyCommandKey === commandKey(command) ? $st('Running...') : $st('Run')
                      }}</Button
                    >
                  </div>
                  <PluginContractEditor
                    :node="command.input.root"
                    :model-value="commandValue(command)"
                    :disabled="Boolean(busyCommandKey)"
                    @update:model-value="setCommandValue(command, $event)"
                  />
                  <div
                    v-if="lastCommandResult && lastCommandKey === commandKey(command)"
                    class="space-y-3 rounded-lg border border-border/70 p-4"
                    role="status"
                  >
                    <div class="flex flex-wrap items-center justify-between gap-2">
                      <div class="text-sm font-semibold">{{ lastCommandResult.title }}</div>
                      <span class="rounded bg-muted px-2 py-1 font-mono text-[10px]">{{
                        lastCommandResult.status
                      }}</span>
                    </div>
                    <p v-if="lastCommandResult.summary" class="text-sm text-muted-foreground">
                      {{ lastCommandResult.summary }}
                    </p>
                    <pre
                      v-if="lastCommandResult.detail"
                      class="max-h-72 overflow-auto whitespace-pre-wrap break-words rounded bg-muted/40 p-3 text-xs"
                      >{{ lastCommandResult.detail }}</pre
                    >
                    <pre
                      v-if="resultPayload(lastCommandResult.output)"
                      class="max-h-72 overflow-auto whitespace-pre-wrap break-words rounded bg-muted/40 p-3 text-xs"
                      >{{ resultPayload(lastCommandResult.output) }}</pre
                    >
                    <div
                      v-for="diagnostic in lastCommandResult.diagnostics || []"
                      :key="`${diagnostic.code}:${diagnostic.path || ''}`"
                      class="text-xs text-destructive"
                    >
                      <span class="font-mono">{{ diagnostic.code }}</span
                      >: {{ diagnostic.message }}
                    </div>
                  </div>
                </div>
              </details>
            </section>

            <section v-else-if="activeTab === 'tools'" class="space-y-3">
              <PluginEmptyState
                v-if="!selectedTools.length"
                :title="$st('No tools available')"
                :description="$st('This plugin does not register tools.')"
                :icon="RiToolsLine"
              />
              <SearchInput
                v-if="selectedTools.length"
                v-model="toolQuery"
                :placeholder="$st('Search tools')"
                :show-search-button="false"
                :input-aria-label="$st('Search tools')"
                :clear-aria-label="$st('Clear search')"
                :clear-title="$st('Clear search')"
              />
              <PluginEmptyState
                v-if="selectedTools.length && !filteredTools.length"
                :title="$st('No matching tools.')"
                :description="$st('Try another search term.')"
                :icon="RiSearchLine"
              />
              <div
                v-for="tool in filteredTools"
                :key="String(tool.name || '')"
                class="border-b border-border/50 px-1 py-3 last:border-b-0"
              >
                <div class="break-all font-mono text-xs font-medium">{{ tool.name || $st('Unnamed tool') }}</div>
                <p class="mt-1.5 text-xs leading-relaxed text-muted-foreground">
                  {{ tool.summary || tool.docs?.summary || tool.description || $st('No summary.') }}
                </p>
                <div v-if="tool.tags?.length" class="mt-2 flex flex-wrap gap-1.5">
                  <span
                    v-for="tag in tool.tags"
                    :key="tag"
                    class="rounded bg-muted/50 px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
                    >{{ tag }}</span
                  >
                </div>
                <details v-if="tool.docs?.help" class="mt-2">
                  <summary class="cursor-pointer text-xs text-muted-foreground">
                    {{ $st('Tool documentation') }}
                  </summary>
                  <MarkdownRenderer class="mt-3" :content="tool.docs.help" source-path="" />
                </details>
              </div>
            </section>

            <section v-else-if="activeTab === 'logs'" class="space-y-3">
              <div class="flex flex-wrap items-center gap-2">
                <SearchInput
                  v-model="logQuery"
                  class="min-w-0 flex-1 basis-40"
                  :placeholder="$st('Search logs')"
                  :show-search-button="false"
                  :input-aria-label="$st('Search logs')"
                  :clear-aria-label="$st('Clear search')"
                  :clear-title="$st('Clear search')"
                />
                <div class="w-32">
                  <OptionPicker
                    v-model="logLevel"
                    :options="logLevelOptions"
                    :title="$st('Log level')"
                    :empty-label="$st('All levels')"
                  />
                </div>
                <IconButton
                  variant="ghost"
                  size="md"
                  :tooltip="$st('Refresh logs')"
                  :aria-label="$st('Refresh logs')"
                  :disabled="logsLoading"
                  @click="loadLogs"
                  ><RiRefreshLine class="h-4 w-4" :class="logsLoading ? 'animate-spin' : ''"
                /></IconButton>
              </div>
              <p class="text-[11px] text-muted-foreground">
                {{ $st('Up to {count} recent log entries.', { count: 200 }) }}
              </p>
              <PluginEmptyState
                v-if="logsError"
                :title="$st('Could not load plugin logs.')"
                :description="logsError"
                :icon="RiFileList2Line"
                ><Button variant="outline" size="sm" @click="loadLogs">{{ $st('Retry') }}</Button></PluginEmptyState
              >
              <PluginEmptyState
                v-else-if="logsLoading && !logs.length"
                :title="$st('Loading logs…')"
                :icon="RiRefreshLine"
              />
              <PluginEmptyState
                v-else-if="!logs.length"
                :title="$st('No plugin logs recorded.')"
                :description="
                  $st('Log entries will appear here when this plugin emits them. Refresh to check for new activity.')
                "
                :icon="RiFileList2Line"
              />
              <PluginEmptyState
                v-else-if="!filteredLogs.length"
                :title="$st('No matching logs.')"
                :description="$st('Try another search term or log level.')"
                :icon="RiSearchLine"
              />
              <div
                v-for="entry in filteredLogs"
                :key="entry.seq"
                class="border-b border-border/50 py-3 last:border-b-0"
              >
                <div class="flex flex-wrap items-center gap-x-3 gap-y-1 text-[10px] text-muted-foreground">
                  <span
                    class="rounded bg-muted/60 px-1.5 py-0.5 font-medium uppercase"
                    :class="
                      entry.level === 'error'
                        ? 'text-destructive'
                        : entry.level === 'warn'
                          ? 'text-amber-700 dark:text-amber-400'
                          : ''
                    "
                    >{{ entry.level }}</span
                  ><span>{{ new Date(entry.timestamp_ms).toLocaleString() }}</span
                  ><span>{{ entry.source }}</span
                  ><span>#{{ entry.seq }}</span>
                </div>
                <div class="mt-2 whitespace-pre-wrap break-words font-mono text-xs leading-relaxed">
                  {{ entry.message }}
                </div>
                <details v-if="entry.fields && Object.keys(entry.fields).length" class="mt-2">
                  <summary class="cursor-pointer text-[11px] text-muted-foreground">{{ $st('Log fields') }}</summary>
                  <pre
                    class="mt-2 max-h-60 overflow-auto whitespace-pre-wrap break-words rounded bg-muted/30 p-3 text-xs"
                    >{{ resultPayload(entry.fields) }}</pre
                  >
                </details>
              </div>
            </section>

            <section v-else class="space-y-4">
              <div
                v-if="architectureError"
                role="alert"
                class="rounded-lg border border-destructive/30 p-3 text-xs text-destructive"
              >
                {{ architectureError }}
                <Button variant="ghost" size="sm" :disabled="loading" @click="refresh">{{ $st('Retry') }}</Button>
              </div>
              <PluginEmptyState
                v-if="!diagnosticCount && !architectureError && !settingsError"
                :title="$st('No plugin diagnostics are currently reported.')"
                :description="$st('Inspect runtime details below when troubleshooting this plugin.')"
                :icon="RiCheckboxCircleLine"
              />
              <div v-if="settingsError" class="rounded-lg border border-destructive/30 p-3 text-xs text-destructive">
                {{ settingsError }}
              </div>
              <div
                v-if="selectedStatus.state === 'failed' || statusFailure(selectedStatus)"
                class="rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive"
              >
                {{ statusFailure(selectedStatus) || pluginStateLabel(selectedStatus.state) }}
              </div>
              <div
                v-if="selectedBlocked"
                class="rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive"
              >
                <div class="font-mono text-xs font-medium">{{ selectedBlocked.code }}</div>
                <p class="mt-1">{{ selectedBlocked.message }}</p>
                <p v-if="selectedBlocked.dependencies?.length" class="mt-2 text-xs">
                  {{ $st('Required plugins') }}:
                  <span class="font-mono">{{ selectedBlocked.dependencies.join(', ') }}</span>
                </p>
              </div>
              <div
                v-for="diagnostic in settingsDiagnostics"
                :key="`${diagnostic.path}:${diagnostic.message}`"
                class="rounded-md border border-amber-500/30 bg-amber-500/5 p-3 text-xs"
              >
                <span v-if="diagnostic.path" class="font-mono">{{ diagnostic.path }}:</span> {{ diagnostic.message }}
              </div>
              <div v-if="selectedEffects.length" class="space-y-2">
                <h3 class="text-xs font-medium text-muted-foreground">{{ $st('Owned effects') }}</h3>
                <div
                  v-for="effect in selectedEffects"
                  :key="effect.id"
                  class="rounded-md border border-border/60 px-3 py-2 text-xs"
                >
                  <div class="flex flex-wrap items-center justify-between gap-2">
                    <span class="font-mono">#{{ effect.id }} · {{ effect.kind }}</span>
                    <span class="font-mono text-muted-foreground">{{ effect.state }}</span>
                  </div>
                  <div class="mt-1 text-muted-foreground">{{ effect.label }}</div>
                  <div v-if="effect.error" class="mt-1 text-destructive">{{ effect.error }}</div>
                </div>
              </div>

              <details class="rounded-lg border border-border/60">
                <summary class="cursor-pointer rounded-lg px-4 py-3 text-xs font-medium hover:bg-muted/30">
                  <RiStethoscopeLine class="mr-2 inline h-4 w-4 text-muted-foreground" />{{ $st('Runtime details') }}
                </summary>
                <div class="space-y-6 border-t border-border/60 p-4">
                  <div v-if="catalog?.tool_registry_generation !== undefined" class="text-xs text-muted-foreground">
                    {{ $st('tool registry:') }} {{ catalog.tool_registry_generation }}
                  </div>
                  <div v-if="appliedProfiles.length" class="rounded-md border border-border/60 bg-muted/20 p-4">
                    <div class="text-xs font-medium text-muted-foreground">{{ $st('Applied plugin profiles') }}</div>
                    <div class="mt-2 flex flex-wrap gap-1.5">
                      <span
                        v-for="profile in appliedProfiles"
                        :key="profile"
                        class="rounded bg-muted px-2 py-1 font-mono text-[10px]"
                      >
                        {{ profile }}
                      </span>
                    </div>
                    <div
                      v-if="selectedProfileChanges.length"
                      class="mt-3 divide-y divide-border/50 rounded-md border border-border/50"
                    >
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
                          <span
                            v-for="path in change.paths"
                            :key="path"
                            class="rounded bg-background px-2 py-1 font-mono text-[10px] text-muted-foreground"
                          >
                            {{ path }}
                          </span>
                        </div>
                      </div>
                    </div>
                  </div>

                  <div v-if="selectedReloadDecision" class="rounded-md border border-border/60 bg-muted/20 p-4">
                    <div class="flex flex-wrap items-center justify-between gap-2">
                      <div class="text-xs font-medium text-muted-foreground">{{ $st('Current reload decision') }}</div>
                      <span class="rounded bg-muted px-2 py-1 font-mono text-[10px]">{{
                        selectedReloadDecision.action
                      }}</span>
                    </div>
                    <div class="mt-2 text-xs text-muted-foreground">
                      {{
                        selectedReloadDecision.reasons?.length
                          ? selectedReloadDecision.reasons.join(' · ')
                          : $st('No restart reason; the transport is reusable.')
                      }}
                    </div>
                    <div v-if="selectedReloadDecision.triggered_by?.length" class="mt-2 text-xs text-muted-foreground">
                      {{ $st('Triggered by:') }}
                      <span class="font-mono">{{ selectedReloadDecision.triggered_by.join(', ') }}</span>
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
                      <dd class="mt-1 break-all font-mono text-xs">
                        {{ selectedArchitectureNode?.activation_epoch || $st('Not computed') }}
                      </dd>
                    </div>
                    <div>
                      <dt class="text-xs text-muted-foreground">{{ $st('Effect scope') }}</dt>
                      <dd class="mt-1 font-mono text-sm">{{ selectedEffectLifecycle }}</dd>
                    </div>
                    <div>
                      <dt class="text-xs text-muted-foreground">{{ $st('Trust level') }}</dt>
                      <dd class="mt-1 font-mono text-sm">
                        {{ selectedAuthority?.trust_level || $st('Not reported') }}
                      </dd>
                    </div>
                    <div>
                      <dt class="text-xs text-muted-foreground">{{ $st('Authors') }}</dt>
                      <dd class="mt-1 text-sm">{{ selectedManifest?.authors?.join(', ') || $st('Not reported') }}</dd>
                    </div>
                    <div>
                      <dt class="text-xs text-muted-foreground">{{ $st('Provenance') }}</dt>
                      <dd class="mt-1 text-xs">
                        {{ selectedAuthority?.provenance?.join(' · ') || $st('Not reported') }}
                      </dd>
                    </div>
                    <div>
                      <dt class="text-xs text-muted-foreground">{{ $st('Restarts') }}</dt>
                      <dd class="mt-1 font-mono text-sm">{{ selectedStatus.restart_count }}</dd>
                    </div>
                  </dl>

                  <div
                    v-if="selectedIncomingDependencies.length || selectedOutgoingDependencies.length"
                    class="grid gap-4 lg:grid-cols-2"
                  >
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

                  <div
                    v-if="selectedServiceImports.length || selectedServiceExports.length"
                    class="grid gap-4 lg:grid-cols-2"
                  >
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
                            {{ service.resolved_provider || service.provider || $st('No provider bound') }} ·
                            {{ service.optional ? $st('optional') : $st('required') }}
                          </div>
                          <div v-if="service.methods?.length" class="mt-2 flex flex-wrap gap-1.5">
                            <span
                              v-for="method in service.methods"
                              :key="method.id"
                              class="rounded bg-muted px-2 py-1 font-mono text-[10px]"
                            >
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
                            <span
                              v-for="method in service.methods"
                              :key="method.id"
                              class="rounded bg-muted px-2 py-1 font-mono text-[10px]"
                            >
                              {{ method.id }}
                            </span>
                          </div>
                        </div>
                      </div>
                    </div>
                  </div>

                  <div
                    v-if="
                      selectedPipelineHandlers.length ||
                      selectedToolRegistrations.length ||
                      selectedCommandRegistrations.length
                    "
                    class="grid gap-4 lg:grid-cols-2"
                  >
                    <div v-if="selectedPipelineHandlers.length">
                      <h3 class="text-xs font-medium text-muted-foreground">{{ $st('Typed pipeline handlers') }}</h3>
                      <div class="mt-2 divide-y divide-border/50 rounded-md border border-border/60">
                        <div
                          v-for="pipeline in selectedPipelineHandlers"
                          :key="pipeline.definition.id"
                          class="px-3 py-2 text-xs"
                        >
                          <div class="flex flex-wrap items-center justify-between gap-2">
                            <span class="font-mono">{{ pipeline.definition.id }}</span>
                            <span class="text-muted-foreground">
                              {{ pipeline.definition.mode }}
                              · {{ pipeline.definition.durable ? $st('durable') : $st('live') }} ·
                              {{ pipeline.definition.scoped ? $st('scoped') : $st('global') }}
                              <template v-if="pipeline.failure_policy">
                                · {{ pipeline.failure_policy }} {{ $st('on error') }}</template
                              >
                            </span>
                          </div>
                          <div class="mt-2 space-y-1">
                            <div
                              v-for="handler in pipeline.handlers"
                              :key="handler.registration"
                              class="flex items-center justify-between gap-2"
                            >
                              <span>{{ handler.id }}</span>
                              <span class="font-mono text-[10px] text-muted-foreground"
                                >{{ $st('priority') }} {{ handler.priority }}</span
                              >
                            </div>
                          </div>
                        </div>
                      </div>
                    </div>
                    <div v-if="selectedToolRegistrations.length">
                      <h3 class="text-xs font-medium text-muted-foreground">{{ $st('Scoped tool registrations') }}</h3>
                      <div class="mt-2 divide-y divide-border/50 rounded-md border border-border/60">
                        <div
                          v-for="entry in selectedToolRegistrations"
                          :key="`${entry.generation}:${entry.key}`"
                          class="px-3 py-2 text-xs"
                        >
                          <div class="font-mono">{{ entry.key }}</div>
                          <div class="mt-1 text-muted-foreground">
                            {{ entry.layer.kind === 'global' ? $st('global') : entry.layer.scope }} ·
                            {{ $st('generation') }} {{ entry.generation }}
                          </div>
                        </div>
                      </div>
                    </div>
                    <div v-if="selectedCommandRegistrations.length">
                      <h3 class="text-xs font-medium text-muted-foreground">
                        {{ $st('Scoped command registrations') }}
                      </h3>
                      <div class="mt-2 divide-y divide-border/50 rounded-md border border-border/60">
                        <div
                          v-for="entry in selectedCommandRegistrations"
                          :key="`${entry.generation}:${entry.key}`"
                          class="px-3 py-2 text-xs"
                        >
                          <div class="font-mono">{{ entry.key }}</div>
                          <div class="mt-1 text-muted-foreground">
                            {{ entry.layer.kind === 'global' ? $st('global') : entry.layer.scope }} ·
                            {{ $st('generation') }} {{ entry.generation }}
                          </div>
                        </div>
                      </div>
                    </div>
                  </div>

                  <div v-if="selectedAuthority?.plugin_capabilities?.length">
                    <h3 class="text-xs font-medium text-muted-foreground">{{ $st('Plugin capabilities') }}</h3>
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
              </details>
            </section>
          </div>
        </template>
      </div>
    </div>
  </div>
</template>
