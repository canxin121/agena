<script setup lang="ts">
import { computed, inject, ref } from 'vue'
import { routerKey } from 'vue-router'

import { runPluginUiAction, type PluginInspect, type PluginLogEntry, type PluginStatus } from '@/agena/lib/agenaApi'
import type { SettingsPluginUiPresentationSnapshot } from './runtimePageLoaders'

const props = defineProps<{
  canTogglePluginConfig: boolean
  plugins: PluginStatus[]
  pluginUiPresentation: SettingsPluginUiPresentationSnapshot | null
  selectedPlugin: PluginInspect | null
  pluginLogs: PluginLogEntry[]
  pluginLoading: boolean
  loadPluginDetails: (pluginId: string) => void | Promise<void>
  openPluginManifestInWorkspace: () => void
  openPluginLogsWorkspacePath: () => void
  setSelectedPluginDisabled: (disabled: boolean) => void | Promise<void>
}>()

type ManifestStudioUiAction = { kind: string; [key: string]: unknown }
type ManifestStudioControlOption = {
  label: string
  value: string
  description?: string
}
type ManifestStudioUiItem = {
  id: string
  title: string
  description?: string
  category?: string
  location?: string
  kind?: string
  options?: ManifestStudioControlOption[]
  value?: unknown
  content?: string
  url?: string
  action?: ManifestStudioUiAction
  controls?: ManifestStudioUiItem[]
}

type ManifestFact = {
  label: string
  value: string
}

type ManifestEntrySummary = {
  name: string
  description: string
  help: string
  effectiveUiDisplayMode: 'detailed' | 'summary'
  effectiveUiDisplaySource: string
  summary: string
  tags: string[]
  facts: string[]
}

const pluginUiMessage = ref('')
const pluginUiError = ref('')
const controlDrafts = ref<Record<string, unknown>>({})
const router = inject(routerKey, null)
const manifest = computed(() => props.selectedPlugin?.manifest ?? null)
const authority = computed(() => props.selectedPlugin?.authority ?? null)
const selectedPluginDisabled = computed(() => readRecord(props.selectedPlugin?.entry).disabled === true)
const selectedPluginEntryKind = computed(() => {
  const kind = readRecord(props.selectedPlugin?.entry).kind
  return typeof kind === 'string' && kind.trim() ? kind.trim() : 'unknown'
})

function readString(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

function readArray(value: unknown): unknown[] {
  return Array.isArray(value) ? value : []
}

function readRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {}
}

function readStringArray(value: unknown): string[] {
  return readArray(value)
    .map((item) => readString(item))
    .filter((item) => item.length > 0)
}

function readOptionalUiDisplayMode(value: unknown): 'detailed' | 'summary' | null {
  return value === 'detailed' || value === 'summary' ? value : null
}

function readOptionalUiDisplayOverride(value: unknown): 'default' | 'detailed' | 'summary' | null {
  if (value === 'default' || value === 'inherit') return 'default'
  if (value === 'detailed' || value === 'summary') return value
  return null
}

function globalDisplayMode(): 'detailed' | 'summary' {
  return readOptionalUiDisplayMode(props.pluginUiPresentation?.defaultMode) || 'detailed'
}

function resolveDisplayOverride(
  override: 'default' | 'detailed' | 'summary',
  fallback: 'detailed' | 'summary',
): 'detailed' | 'summary' {
  if (override === 'summary') return 'summary'
  if (override === 'detailed') return 'detailed'
  return fallback
}

function toolOverrideKeys(pluginId: string, toolName: string): string[] {
  const exposedName = `${pluginId}/${toolName}`
  return [exposedName, `${pluginId}/${toolName}`, `${pluginId}/${exposedName}`, `${pluginId}.${toolName}`, toolName]
}

function findToolUiOverride(pluginId: string, toolName: string): 'default' | 'detailed' | 'summary' | null {
  const overrides = props.pluginUiPresentation?.effectiveToolOverrides || {}
  for (const key of toolOverrideKeys(pluginId, toolName)) {
    if (!Object.prototype.hasOwnProperty.call(overrides, key)) continue
    return readOptionalUiDisplayOverride(overrides[key]) || null
  }
  return null
}

function pluginDisplayMode(
  pluginId: string,
  pluginDefault: 'detailed' | 'summary' | null,
): 'detailed' | 'summary' {
  const pluginOverride = readOptionalUiDisplayOverride(props.pluginUiPresentation?.effectivePluginOverrides?.[pluginId])
  if (pluginOverride) return resolveDisplayOverride(pluginOverride, pluginDefault || globalDisplayMode())
  return pluginDefault || globalDisplayMode()
}

function toolDisplayMode(
  pluginId: string,
  toolName: string,
  toolDefault: 'detailed' | 'summary' | null,
  pluginDefault: 'detailed' | 'summary' | null,
): 'detailed' | 'summary' {
  const toolOverride = findToolUiOverride(pluginId, toolName)
  if (toolOverride) return resolveDisplayOverride(toolOverride, toolDefault || pluginDefault || globalDisplayMode())
  const pluginOverride = readOptionalUiDisplayOverride(props.pluginUiPresentation?.effectivePluginOverrides?.[pluginId])
  if (pluginOverride) {
    return resolveDisplayOverride(pluginOverride, toolDefault || pluginDefault || globalDisplayMode())
  }
  if (toolDefault) return toolDefault
  return pluginDisplayMode(pluginId, pluginDefault)
}

function pluginDisplaySource(
  pluginId: string,
  pluginDefault: 'detailed' | 'summary' | null,
): string {
  const pluginOverride = readOptionalUiDisplayOverride(props.pluginUiPresentation?.effectivePluginOverrides?.[pluginId])
  if (pluginOverride === 'summary' || pluginOverride === 'detailed') return 'plugin override'
  if (pluginDefault) return 'plugin default'
  return 'global default'
}

function toolDisplaySource(
  pluginId: string,
  toolName: string,
  toolDefault: 'detailed' | 'summary' | null,
  pluginDefault: 'detailed' | 'summary' | null,
): string {
  const toolOverride = findToolUiOverride(pluginId, toolName)
  if (toolOverride === 'summary' || toolOverride === 'detailed') return 'tool override'
  const pluginOverride = readOptionalUiDisplayOverride(props.pluginUiPresentation?.effectivePluginOverrides?.[pluginId])
  if (pluginOverride === 'summary' || pluginOverride === 'detailed') return 'plugin override'
  if (toolDefault && toolDefault !== pluginDefault) return 'tool default'
  if (pluginDefault) return 'plugin default'
  return 'global default'
}

function primaryManifestText(
  mode: 'detailed' | 'summary',
  description: string,
  summary: string,
  fallback: string,
): string {
  if (mode === 'summary') return summary || description || fallback
  return description || summary || fallback
}

function secondaryManifestText(mode: 'detailed' | 'summary', description: string, summary: string): string {
  if (mode === 'detailed' && summary && summary !== description) return summary
  return ''
}

const selectedPluginDisplayMode = computed<'detailed' | 'summary'>(() => {
  const pluginId = props.selectedPlugin?.status.plugin_id || ''
  const pluginDefault = readOptionalUiDisplayMode(readRecord(manifest.value).ui_display_mode)
  return pluginId ? pluginDisplayMode(pluginId, pluginDefault) : 'detailed'
})

const selectedPluginDisplaySource = computed<string>(() => {
  const pluginId = props.selectedPlugin?.status.plugin_id || ''
  const pluginDefault = readOptionalUiDisplayMode(readRecord(manifest.value).ui_display_mode)
  return pluginId ? pluginDisplaySource(pluginId, pluginDefault) : 'global default'
})

const manifestFacts = computed<ManifestFact[]>(() => {
  const data = manifest.value
  if (!data || typeof data !== 'object' || Array.isArray(data)) return []
  const manifestRecord = data as Record<string, unknown>
  const entries = readArray(manifestRecord.tools || manifestRecord.entries)
  const ui = readRecord(manifestRecord.ui)
  const studio = readRecord(ui.studio)
  const tui = readRecord(ui.tui)
  const description = readString(manifestRecord.description)
  const summary = readString(manifestRecord.summary)
  const declaredDisplayDefault = readOptionalUiDisplayMode(manifestRecord.ui_display_mode)
  return [
    { label: 'Name', value: readString(manifestRecord.name) || 'n/a' },
    { label: 'Version', value: readString(manifestRecord.version) || 'n/a' },
    {
      label: selectedPluginDisplayMode.value === 'summary' ? 'Summary' : 'Description',
      value: primaryManifestText(selectedPluginDisplayMode.value, description, summary, 'n/a'),
    },
    { label: 'Display Mode', value: selectedPluginDisplayMode.value },
    { label: 'Display Default', value: declaredDisplayDefault || 'runtime default' },
    { label: 'Display Source', value: selectedPluginDisplaySource.value },
    { label: 'Authors', value: readStringArray(manifestRecord.authors).join(', ') || 'n/a' },
    { label: 'Transports', value: readStringArray(manifestRecord.transports).join(', ') || 'n/a' },
    { label: 'Hooks', value: readStringArray(manifestRecord.hooks).join(', ') || 'n/a' },
    { label: 'Entries', value: String(entries.length) },
    {
      label: 'Studio UI',
      value: `${readArray(studio.commands).length} commands · ${readArray(studio.controls).length} controls · ${readArray(studio.views).length} views`,
    },
    {
      label: 'TUI UI',
      value: `${readArray(tui.statusline_segments).length} statusline · ${readArray(tui.themes).length} themes · ${readArray(tui.content_blocks).length} blocks`,
    },
  ]
})

const authorityFacts = computed<ManifestFact[]>(() => {
  const data = authority.value
  if (!data || typeof data !== 'object' || Array.isArray(data)) return []
  const authorityRecord = data as Record<string, unknown>
  const entryCapabilities = readRecord(authorityRecord.entry_capabilities)
  return [
    { label: 'Trust Level', value: readString(authorityRecord.trust_level) || 'n/a' },
    { label: 'Provenance', value: readStringArray(authorityRecord.provenance).join(', ') || 'n/a' },
    {
      label: 'Plugin Capabilities',
      value: readStringArray(authorityRecord.plugin_capabilities).join(', ') || 'n/a',
    },
    {
      label: 'Entry Capability Sets',
      value: String(Object.keys(entryCapabilities).length),
    },
  ]
})

const manifestEntries = computed<ManifestEntrySummary[]>(() => {
  const data = manifest.value
  if (!data || typeof data !== 'object' || Array.isArray(data)) return []
  const manifestRecord = data as Record<string, unknown>
  const pluginId = props.selectedPlugin?.status.plugin_id || ''
  const pluginDefault = readOptionalUiDisplayMode(manifestRecord.ui_display_mode)
  return readArray(manifestRecord.tools || manifestRecord.entries)
    .map((entry) => readRecord(entry))
    .map((entry) => {
      const tags = readStringArray(entry.tags)
      const toolName = readString(entry.name) || 'unnamed entry'
      const toolDeclaredDefault = readOptionalUiDisplayMode(entry.ui_display_mode)
      const toolDefault = toolDeclaredDefault || pluginDefault
      const effectiveUiDisplayMode = pluginId
        ? toolDisplayMode(pluginId, toolName, toolDefault, pluginDefault)
        : 'detailed'
      const facts = [
        entry.strict === true ? 'strict' : null,
        entry.streaming === true ? 'streaming' : null,
        entry.concurrency_safe === true ? 'concurrency-safe' : null,
        readString(entry.description_mode) ? `mode=${readString(entry.description_mode)}` : null,
        toolDefault ? `ui_default=${toolDefault}` : null,
        pluginId ? `ui_source=${toolDisplaySource(pluginId, toolName, toolDeclaredDefault, pluginDefault)}` : null,
      ].filter(Boolean) as string[]
      return {
        name: toolName,
        description: readString(entry.description) || readString(entry.summary) || 'No description provided.',
        summary: readString(entry.summary) || '',
        help: readString(entry.help) || '',
        effectiveUiDisplayMode,
        effectiveUiDisplaySource: pluginId
          ? toolDisplaySource(pluginId, toolName, toolDeclaredDefault, pluginDefault)
          : 'global default',
        tags,
        facts,
      }
    })
})

const studioUi = computed(() => {
  const ui = manifest.value && typeof manifest.value === 'object' && !Array.isArray(manifest.value) ? (manifest.value as Record<string, unknown>).ui : null
  if (!ui || typeof ui !== 'object' || Array.isArray(ui)) {
    return { commands: [], controls: [], views: [] } as {
      commands: ManifestStudioUiItem[]
      controls: ManifestStudioUiItem[]
      views: ManifestStudioUiItem[]
    }
  }
  const studio = (ui as { studio?: unknown }).studio
  if (!studio || typeof studio !== 'object' || Array.isArray(studio)) {
    return { commands: [], controls: [], views: [] }
  }
  const source = studio as { commands?: unknown; controls?: unknown; views?: unknown }
  return {
    commands: Array.isArray(source.commands) ? (source.commands as ManifestStudioUiItem[]) : [],
    controls: Array.isArray(source.controls) ? (source.controls as ManifestStudioUiItem[]) : [],
    views: Array.isArray(source.views) ? (source.views as ManifestStudioUiItem[]) : [],
  }
})

const hasStudioUi = computed(
  () => studioUi.value.commands.length > 0 || studioUi.value.controls.length > 0 || studioUi.value.views.length > 0,
)
const hasManifestEntries = computed(() => manifestEntries.value.length > 0)

function controlKind(control: ManifestStudioUiItem): string {
  return String(control.kind || 'button')
    .trim()
    .toLowerCase()
}

function controlKey(control: ManifestStudioUiItem): string {
  const pluginId = props.selectedPlugin?.status.plugin_id || ''
  return `${pluginId}:${control.id}`
}

function initialControlValue(control: ManifestStudioUiItem): unknown {
  const kind = controlKind(control)
  if (control.value !== undefined) return control.value
  if (kind === 'checkbox' || kind === 'toggle' || kind === 'switch') return false
  if (kind === 'select') return control.options?.[0]?.value ?? ''
  if (kind === 'number') return 0
  return ''
}

function controlValue(control: ManifestStudioUiItem): unknown {
  const key = controlKey(control)
  if (Object.prototype.hasOwnProperty.call(controlDrafts.value, key)) {
    return controlDrafts.value[key]
  }
  return initialControlValue(control)
}

function setControlValue(control: ManifestStudioUiItem, value: unknown) {
  controlDrafts.value = {
    ...controlDrafts.value,
    [controlKey(control)]: value,
  }
}

function setTextControlValue(control: ManifestStudioUiItem, event: Event) {
  const target = event.target as HTMLInputElement | null
  setControlValue(control, target?.value ?? '')
}

function setNumberControlValue(control: ManifestStudioUiItem, event: Event) {
  const target = event.target as HTMLInputElement | null
  const raw = target?.value ?? ''
  setControlValue(control, raw === '' ? null : Number(raw))
}

function setBooleanControlValue(control: ManifestStudioUiItem, event: Event) {
  const target = event.target as HTMLInputElement | null
  setControlValue(control, Boolean(target?.checked))
}

function setSelectControlValue(control: ManifestStudioUiItem, event: Event) {
  const target = event.target as HTMLSelectElement | null
  setControlValue(control, target?.value ?? '')
}

async function runStudioUiItem(item: ManifestStudioUiItem, input?: Record<string, unknown>) {
  const pluginId = props.selectedPlugin?.status.plugin_id
  if (!pluginId || !item.id) return
  pluginUiMessage.value = ''
  pluginUiError.value = ''
  try {
    if (item.action?.kind === 'open_route') {
      if (!router) return
      await router.push(String(item.action.route || '/plugins'))
      return
    }
    if (item.action?.kind === 'open_url') {
      if (typeof window !== 'undefined') window.open(String(item.action.url || ''), '_blank', 'noopener,noreferrer')
      return
    }
    if (item.action?.kind === 'submit_prompt') {
      if (!router) return
      await router.push({ path: '/chat', query: { prompt: String(item.action.prompt || '') } })
      return
    }
    const response = await runPluginUiAction({ pluginId, actionId: item.id, payload: input })
    pluginUiMessage.value = response.result?.output_text || `Ran ${item.title || item.id}.`
  } catch (err) {
    pluginUiError.value = err instanceof Error ? err.message : String(err)
  }
}

async function runStudioUiControl(control: ManifestStudioUiItem) {
  await runStudioUiItem(control, { value: controlValue(control) })
}
</script>

<template>
  <div class="grid two">
    <section class="card">
      <h3>Plugins</h3>
      <div v-if="props.plugins.length" class="list">
        <button
          v-for="plugin in props.plugins"
          :key="plugin.plugin_id"
          class="list-item"
          style="width: 100%; text-align: left"
          @click="props.loadPluginDetails(plugin.plugin_id)"
        >
          <div class="page-header" style="align-items: flex-start">
            <div>
              <div>
                <strong>{{ plugin.plugin_id }}</strong>
              </div>
              <div class="muted">{{ plugin.kind }} · {{ plugin.state }}</div>
              <div v-if="plugin.last_error" class="muted">{{ plugin.last_error }}</div>
            </div>
            <span class="badge">restarts {{ plugin.restart_count }}</span>
          </div>
        </button>
      </div>
      <p v-else class="muted">No configured plugins.</p>
    </section>

    <section class="card">
      <div class="page-header" style="align-items: flex-start">
        <div>
          <h3>Plugin Detail</h3>
          <p class="muted">Inspect manifest and recent logs, then jump to the shared plugin settings.</p>
        </div>
        <div class="button-row" style="flex-wrap: wrap">
          <button class="button" :disabled="!props.selectedPlugin" @click="props.openPluginManifestInWorkspace">
            Open Plugin Settings
          </button>
          <button
            class="button warn"
            :disabled="!props.selectedPlugin || !props.canTogglePluginConfig"
            @click="props.setSelectedPluginDisabled(!selectedPluginDisabled)"
          >
            {{ selectedPluginDisabled ? 'Enable Plugin' : 'Disable Plugin' }}
          </button>
        </div>
      </div>
      <p v-if="props.selectedPlugin && !props.canTogglePluginConfig" class="muted">
        This plugin does not expose a writable config entry.
      </p>
        <div v-if="props.selectedPlugin" class="stack">
          <div><strong>ID:</strong> {{ props.selectedPlugin.status.plugin_id }}</div>
          <div><strong>Kind:</strong> {{ props.selectedPlugin.status.kind }}</div>
          <div><strong>State:</strong> {{ props.selectedPlugin.status.state }}</div>
          <div class="button-row" style="align-items: center; flex-wrap: wrap">
            <strong>Config:</strong>
            <span class="badge">{{ selectedPluginEntryKind }}</span>
            <span v-if="selectedPluginDisabled" class="badge warn">disabled</span>
            <span v-else class="badge success">enabled</span>
            <span class="badge">ui={{ selectedPluginDisplayMode }}</span>
            <span class="badge">{{ selectedPluginDisplaySource }}</span>
          </div>
          <div><strong>PID:</strong> {{ props.selectedPlugin.status.pid ?? 'n/a' }}</div>
          <div><strong>Last Exit:</strong> {{ props.selectedPlugin.status.last_exit_code ?? 'n/a' }}</div>
          <div><strong>Last Restart:</strong> {{ props.selectedPlugin.status.last_restart_at_ms ?? 'n/a' }}</div>
        <div v-if="manifestFacts.length" class="stack">
          <div><strong>Manifest Summary:</strong></div>
          <div v-for="fact in manifestFacts" :key="fact.label" class="muted">
            <strong>{{ fact.label }}:</strong> {{ fact.value }}
          </div>
        </div>
        <div v-if="authorityFacts.length" class="stack">
          <div><strong>Authority:</strong></div>
          <div v-for="fact in authorityFacts" :key="fact.label" class="muted">
            <strong>{{ fact.label }}:</strong> {{ fact.value }}
          </div>
        </div>
        <div v-if="hasManifestEntries" class="stack">
          <div><strong>Tool Entries:</strong></div>
          <div class="list">
            <div v-for="entry in manifestEntries" :key="entry.name" class="list-item">
              <div class="page-header" style="align-items: flex-start">
                <div>
                  <div class="button-row" style="align-items: center; flex-wrap: wrap">
                    <strong>{{ entry.name }}</strong>
                    <span v-if="entry.facts.includes('strict')" class="badge warn">strict</span>
                    <span v-if="entry.facts.includes('streaming')" class="badge">streaming</span>
                    <span class="badge">{{ entry.effectiveUiDisplayMode }}</span>
                    <span class="badge">{{ entry.effectiveUiDisplaySource }}</span>
                  </div>
                  <div class="muted">
                    {{ primaryManifestText(entry.effectiveUiDisplayMode, entry.description, entry.summary, 'No description provided.') }}
                  </div>
                  <div
                    v-if="secondaryManifestText(entry.effectiveUiDisplayMode, entry.description, entry.summary)"
                    class="muted"
                  >
                    {{ secondaryManifestText(entry.effectiveUiDisplayMode, entry.description, entry.summary) }}
                  </div>
                  <div
                    v-if="entry.help && entry.effectiveUiDisplayMode === 'detailed'"
                    class="muted mono"
                    style="white-space: pre-wrap"
                  >
                    {{ entry.help }}
                  </div>
                  <div v-if="entry.tags.length" class="muted mono">tags={{ entry.tags.join(', ') }}</div>
                  <div v-if="entry.facts.length" class="muted mono">{{ entry.facts.join(' · ') }}</div>
                </div>
              </div>
            </div>
          </div>
        </div>
        <div v-if="hasStudioUi" class="stack">
          <div><strong>Studio UI:</strong></div>
          <p v-if="pluginUiMessage" class="muted">{{ pluginUiMessage }}</p>
          <p v-if="pluginUiError" class="muted">{{ pluginUiError }}</p>
          <div v-if="studioUi.commands.length" class="list">
            <button
              v-for="command in studioUi.commands"
              :key="`command-${command.id}`"
              class="list-item"
              style="width: 100%; text-align: left"
              @click="runStudioUiItem(command)"
            >
              <div class="page-header" style="align-items: flex-start">
                <div>
                  <strong>{{ command.title || command.id }}</strong>
                  <div class="muted">{{ command.description || command.category || 'command' }}</div>
                </div>
                <span class="badge">{{ command.location || 'command_palette' }}</span>
              </div>
            </button>
          </div>
          <div v-if="studioUi.controls.length" class="list">
            <div v-for="control in studioUi.controls" :key="`control-${control.id}`" class="list-item">
              <div class="page-header" style="align-items: flex-start">
                <div>
                  <strong>{{ control.title || control.id }}</strong>
                  <div class="muted">{{ control.description || control.kind || 'control' }}</div>
                </div>
                <span class="badge">{{ control.location || 'plugin_panel' }}</span>
              </div>
              <div class="button-row" style="flex-wrap: wrap">
                <button v-if="controlKind(control) === 'button'" class="button" @click="runStudioUiItem(control)">
                  {{ control.title || control.id }}
                </button>
                <label v-else-if="['checkbox', 'toggle', 'switch'].includes(controlKind(control))" class="muted">
                  <input
                    type="checkbox"
                    :checked="Boolean(controlValue(control))"
                    @change="setBooleanControlValue(control, $event)"
                  />
                  {{ control.title || control.id }}
                </label>
                <select
                  v-else-if="controlKind(control) === 'select'"
                  class="input"
                  :value="String(controlValue(control) ?? '')"
                  @input="setSelectControlValue(control, $event)"
                >
                  <option v-for="option in control.options || []" :key="option.value" :value="option.value">
                    {{ option.label || option.value }}
                  </option>
                </select>
                <input
                  v-else-if="controlKind(control) === 'number'"
                  class="input"
                  type="number"
                  :value="String(controlValue(control) ?? '')"
                  @input="setNumberControlValue(control, $event)"
                />
                <input
                  v-else
                  class="input"
                  type="text"
                  :value="String(controlValue(control) ?? '')"
                  @input="setTextControlValue(control, $event)"
                />
                <button v-if="controlKind(control) !== 'button'" class="button" @click="runStudioUiControl(control)">
                  Apply
                </button>
              </div>
            </div>
          </div>
          <div v-if="studioUi.views.length" class="list">
            <div v-for="view in studioUi.views" :key="`view-${view.id}`" class="list-item">
              <div class="page-header" style="align-items: flex-start">
                <div>
                  <strong>{{ view.title || view.id }}</strong>
                  <div class="muted">{{ view.description || view.kind || 'view' }}</div>
                </div>
                <span class="badge">{{ view.location || 'plugins' }}</span>
              </div>
              <pre v-if="view.content" class="mono" style="white-space: pre-wrap">{{ view.content }}</pre>
              <a v-if="view.url" class="button" :href="view.url" target="_blank" rel="noreferrer">Open View</a>
              <div v-if="view.controls?.length" class="list">
                <div v-for="control in view.controls" :key="`view-control-${view.id}-${control.id}`" class="list-item">
                  <div class="page-header" style="align-items: flex-start">
                    <div>
                      <strong>{{ control.title || control.id }}</strong>
                      <div class="muted">{{ control.description || control.kind || 'control' }}</div>
                    </div>
                    <span class="badge">{{ control.location || view.location || 'plugins' }}</span>
                  </div>
                  <div class="button-row" style="flex-wrap: wrap">
                    <button v-if="controlKind(control) === 'button'" class="button" @click="runStudioUiItem(control)">
                      {{ control.title || control.id }}
                    </button>
                    <label v-else-if="['checkbox', 'toggle', 'switch'].includes(controlKind(control))" class="muted">
                      <input
                        type="checkbox"
                        :checked="Boolean(controlValue(control))"
                        @change="setBooleanControlValue(control, $event)"
                      />
                      {{ control.title || control.id }}
                    </label>
                    <select
                      v-else-if="controlKind(control) === 'select'"
                      class="input"
                      :value="String(controlValue(control) ?? '')"
                      @input="setSelectControlValue(control, $event)"
                    >
                      <option v-for="option in control.options || []" :key="option.value" :value="option.value">
                        {{ option.label || option.value }}
                      </option>
                    </select>
                    <input
                      v-else-if="controlKind(control) === 'number'"
                      class="input"
                      type="number"
                      :value="String(controlValue(control) ?? '')"
                      @input="setNumberControlValue(control, $event)"
                    />
                    <input
                      v-else
                      class="input"
                      type="text"
                      :value="String(controlValue(control) ?? '')"
                      @input="setTextControlValue(control, $event)"
                    />
                    <button
                      v-if="controlKind(control) !== 'button'"
                      class="button"
                      @click="runStudioUiControl(control)"
                    >
                      Apply
                    </button>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
        <div><strong>Manifest:</strong></div>
        <pre class="mono" style="white-space: pre-wrap">{{
          JSON.stringify(props.selectedPlugin.manifest ?? {}, null, 2)
        }}</pre>
        <div><strong>Recent Logs:</strong></div>
        <div v-if="props.pluginLogs.length" class="list">
          <div v-for="entry in props.pluginLogs" :key="entry.seq" class="list-item">
            <div class="page-header" style="align-items: flex-start">
              <strong>#{{ entry.seq }}</strong>
              <span class="badge">{{ entry.level }}</span>
            </div>
            <div class="muted">{{ entry.target || 'plugin' }} · {{ entry.timestamp_ms }}</div>
            <div class="muted">{{ entry.message }}</div>
          </div>
        </div>
        <div v-else class="muted">No retained logs.</div>
      </div>
      <p v-else-if="props.pluginLoading" class="muted">Loading plugin detail…</p>
      <p v-else class="muted">Select a plugin to inspect.</p>
    </section>
  </div>
</template>
