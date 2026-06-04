<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import type {
  PluginUiDisplayMode,
  PluginUiDisplayOverride,
  SettingsPluginEntrySnapshot,
  SettingsPluginToolSnapshot,
  ToolDescriptionMode,
  ToolDescriptionOverride,
} from './runtimePageLoaders'

const props = defineProps<{
  actionError: string
  actionMessage: string
  load: () => void | Promise<void>
  pluginHasOverrides: (entry: SettingsPluginEntrySnapshot) => boolean
  pluginEntrySummary: (entry: SettingsPluginEntrySnapshot) => string
  pluginPromptSourceSummary: (entry: SettingsPluginEntrySnapshot) => string
  pluginUiSourceSummary: (entry: SettingsPluginEntrySnapshot) => string
  plugins: SettingsPluginEntrySnapshot[]
  primaryPluginText: (entry: SettingsPluginEntrySnapshot) => string
  primaryToolText: (tool: SettingsPluginToolSnapshot) => string
  promptModeLabel: (mode: ToolDescriptionMode) => string
  promptOverrideLabel: (mode: ToolDescriptionOverride | null) => string
  promptOverrideOptions: Array<{ label: string; value: ToolDescriptionOverride; description: string }>
  secondaryPluginText: (entry: SettingsPluginEntrySnapshot) => string
  secondaryToolText: (tool: SettingsPluginToolSnapshot) => string
  setPluginPromptOverride: (pluginId: string, mode: ToolDescriptionOverride) => void | Promise<void>
  setPluginUiDisplayOverride: (pluginId: string, mode: PluginUiDisplayOverride) => void | Promise<void>
  setToolPromptOverride: (pluginId: string, toolName: string, mode: ToolDescriptionOverride) => void | Promise<void>
  setToolUiDisplayOverride: (pluginId: string, toolName: string, mode: PluginUiDisplayOverride) => void | Promise<void>
  summaryFacts: Array<{ label: string; value: string }>
  togglePluginEntryDisabled: (entry: SettingsPluginEntrySnapshot) => void | Promise<void>
  toolPromptSourceSummary: (entry: SettingsPluginEntrySnapshot, tool: SettingsPluginToolSnapshot) => string
  toolUiSourceSummary: (entry: SettingsPluginEntrySnapshot, tool: SettingsPluginToolSnapshot) => string
  uiDisplayModeLabel: (mode: PluginUiDisplayMode) => string
  uiDisplayOverrideLabel: (mode: PluginUiDisplayOverride | null) => string
  uiDisplayOverrideOptions: Array<{ label: string; value: PluginUiDisplayOverride; description: string }>
}>()

const pluginSearch = ref('')
const showChangedOnly = ref(false)
const currentPage = ref(1)
const PLUGINS_PER_PAGE = 8

function matchesTool(tool: SettingsPluginToolSnapshot, search: string): boolean {
  if (!search) return true
  const haystack = [tool.toolName, tool.description, tool.summary, tool.help, ...tool.tags].join(' ').toLowerCase()
  return haystack.includes(search)
}

function visibleTools(entry: SettingsPluginEntrySnapshot): SettingsPluginToolSnapshot[] {
  const search = pluginSearch.value.trim().toLowerCase()
  if (!search) return entry.tools
  return entry.tools.filter((tool) => matchesTool(tool, search))
}

const visiblePlugins = computed(() => {
  const search = pluginSearch.value.trim().toLowerCase()
  return props.plugins.filter((entry) => {
    if (showChangedOnly.value && !props.pluginHasOverrides(entry)) return false
    if (!search) return true
    const pluginText = [entry.pluginId, entry.displayName, entry.kind, entry.description, entry.summary].join(' ').toLowerCase()
    return pluginText.includes(search) || visibleTools(entry).length > 0
  })
})

const visiblePluginCountLabel = computed(() => {
  if (visiblePlugins.value.length === props.plugins.length) return String(props.plugins.length)
  return `${visiblePlugins.value.length} / ${props.plugins.length}`
})

const visibleSummaryFacts = computed(() =>
  props.summaryFacts.filter((fact) => !['Prompt Default', 'UI Default', 'File Override'].includes(fact.label)),
)

const totalPages = computed(() => Math.max(1, Math.ceil(visiblePlugins.value.length / PLUGINS_PER_PAGE)))

const pagedPlugins = computed(() => {
  const start = (currentPage.value - 1) * PLUGINS_PER_PAGE
  return visiblePlugins.value.slice(start, start + PLUGINS_PER_PAGE)
})

watch(
  visiblePlugins,
  () => {
    currentPage.value = Math.min(currentPage.value, totalPages.value)
    if (currentPage.value < 1) currentPage.value = 1
  },
  { immediate: true },
)

function goToPreviousPage() {
  currentPage.value = Math.max(1, currentPage.value - 1)
}

function goToNextPage() {
  currentPage.value = Math.min(totalPages.value, currentPage.value + 1)
}

function applyPromptOverride(entry: SettingsPluginEntrySnapshot, event: Event) {
  const target = event.target as HTMLSelectElement | null
  const next = (target?.value || 'tool_default') as ToolDescriptionOverride
  void props.setPluginPromptOverride(entry.pluginId, next)
}

function applyUiOverride(entry: SettingsPluginEntrySnapshot, event: Event) {
  const target = event.target as HTMLSelectElement | null
  const next = (target?.value || 'default') as PluginUiDisplayOverride
  void props.setPluginUiDisplayOverride(entry.pluginId, next)
}

function applyToolPromptOverride(entry: SettingsPluginEntrySnapshot, tool: SettingsPluginToolSnapshot, event: Event) {
  const target = event.target as HTMLSelectElement | null
  const next = (target?.value || 'tool_default') as ToolDescriptionOverride
  void props.setToolPromptOverride(entry.pluginId, tool.toolName, next)
}

function applyToolUiOverride(entry: SettingsPluginEntrySnapshot, tool: SettingsPluginToolSnapshot, event: Event) {
  const target = event.target as HTMLSelectElement | null
  const next = (target?.value || 'default') as PluginUiDisplayOverride
  void props.setToolUiDisplayOverride(entry.pluginId, tool.toolName, next)
}

function promptCurrentLabel(mode: ToolDescriptionMode, override: ToolDescriptionOverride | null): string {
  if (override === 'brief' || override === 'detailed') return props.promptOverrideLabel(override)
  return `Declared Default -> ${props.promptModeLabel(mode)}`
}

function uiCurrentLabel(mode: PluginUiDisplayMode, override: PluginUiDisplayOverride | null): string {
  if (override === 'summary' || override === 'detailed') return props.uiDisplayOverrideLabel(override)
  return `Declared Default -> ${props.uiDisplayModeLabel(mode)}`
}

function promptOptionLabel(mode: ToolDescriptionMode, option: { value: ToolDescriptionOverride; label: string }): string {
  if (option.value === 'tool_default') {
    return `Declared Default -> ${props.promptModeLabel(mode)}`
  }
  return option.label
}

function uiOptionLabel(mode: PluginUiDisplayMode, option: { value: PluginUiDisplayOverride; label: string }): string {
  if (option.value === 'default') {
    return `Declared Default -> ${props.uiDisplayModeLabel(mode)}`
  }
  return option.label
}

function promptOptionDescription(
  mode: ToolDescriptionMode,
  option: { value: ToolDescriptionOverride; description: string },
  current: ToolDescriptionOverride | null,
): string {
  if ((current || 'tool_default') === option.value && option.value === 'tool_default') {
    return `Current result: ${props.promptModeLabel(mode)}. ${option.description}`
  }
  return option.description
}

function uiOptionDescription(
  mode: PluginUiDisplayMode,
  option: { value: PluginUiDisplayOverride; description: string },
  current: PluginUiDisplayOverride | null,
): string {
  if ((current || 'default') === option.value && option.value === 'default') {
    return `Current result: ${props.uiDisplayModeLabel(mode)}. ${option.description}`
  }
  return option.description
}
</script>

<template>
  <div class="settings-page">
    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Agena Runtime</p>
          <h3 class="settings-panel-title">Plugin Policies</h3>
        </div>
        <button class="button ghost" @click="props.load">Refresh</button>
      </div>

      <p class="muted">
        Manage model-facing prompt definitions separately from how plugin and tool metadata is rendered in the web and TUI inspectors.
      </p>

      <p v-if="props.actionMessage" class="muted">{{ props.actionMessage }}</p>
      <p v-if="props.actionError" class="muted">{{ props.actionError }}</p>

      <div class="settings-summary">
        <div v-for="fact in visibleSummaryFacts" :key="fact.label" class="summary-item">
          <div class="summary-label">{{ fact.label }}</div>
          <div class="summary-value">{{ fact.value }}</div>
        </div>
      </div>
    </section>

    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Plugin Groups</p>
          <h3 class="settings-panel-title">Plugin Policy Studio</h3>
        </div>
        <span class="badge">{{ props.plugins.length }}</span>
      </div>

      <p class="muted">
        Edit prompt definition mode and Web/TUI display mode per plugin and per tool. Default selections follow the declared plugin or tool defaults instead of forcing a global mode.
      </p>

      <div class="button-row" style="margin-top: 12px; align-items: center; flex-wrap: wrap">
        <input v-model="pluginSearch" class="input" placeholder="Filter plugins or tools" style="min-width: 280px" />
        <label class="muted" style="display: inline-flex; align-items: center; gap: 8px">
          <input v-model="showChangedOnly" type="checkbox" />
          Only changed entries
        </label>
      </div>

      <div v-if="props.plugins.length === 0" class="muted" style="margin-top: 12px">
        No plugin entries are currently available.
      </div>

      <div v-else-if="visiblePlugins.length === 0" class="muted" style="margin-top: 12px">
        No plugins matched the current filter.
      </div>

      <div v-else class="stack" style="margin-top: 16px">
        <div class="button-row" style="justify-content: space-between; align-items: center">
          <span class="muted">Showing {{ visiblePluginCountLabel }} plugin entries.</span>
          <span class="muted">Page {{ currentPage }} / {{ totalPages }} · Search also matches tool names and tags.</span>
        </div>

        <div class="button-row" style="justify-content: flex-end; gap: 8px">
          <button class="button ghost" :disabled="currentPage <= 1" @click="goToPreviousPage">Previous Page</button>
          <button class="button ghost" :disabled="currentPage >= totalPages" @click="goToNextPage">Next Page</button>
        </div>

        <details v-for="entry in pagedPlugins" :key="entry.pluginId" class="settings-panel" open>
          <summary class="page-header" style="cursor: pointer; list-style: none; align-items: flex-start">
            <div>
              <div class="button-row" style="align-items: center; flex-wrap: wrap">
                <strong>{{ entry.displayName }}</strong>
                <span class="badge mono">{{ entry.pluginId }}</span>
                <span class="badge">{{ entry.kind }}</span>
                <span class="badge">{{ entry.source }}</span>
                <span v-if="props.pluginHasOverrides(entry)" class="badge warn">changed</span>
                <span v-if="entry.disabled" class="badge warn">disabled</span>
                <span v-else class="badge success">enabled</span>
                <span class="badge">prompt={{ props.promptModeLabel(entry.effectivePromptMode) }}</span>
                <span class="badge">ui={{ props.uiDisplayModeLabel(entry.effectiveUiDisplayMode) }}</span>
              </div>
              <div class="muted">{{ props.primaryPluginText(entry) }}</div>
              <div v-if="props.secondaryPluginText(entry)" class="muted">{{ props.secondaryPluginText(entry) }}</div>
              <div class="muted mono">{{ props.pluginEntrySummary(entry) }}</div>
              <div class="muted mono">
                prompt={{ promptCurrentLabel(entry.effectivePromptMode, entry.filePromptOverride) }}
                · prompt default={{ entry.declaredPromptDefault ? props.promptModeLabel(entry.declaredPromptDefault) : 'runtime default' }}
                · ui={{ uiCurrentLabel(entry.effectiveUiDisplayMode, entry.fileUiDisplayOverride) }}
                · ui default={{ entry.declaredUiDefault ? props.uiDisplayModeLabel(entry.declaredUiDefault) : 'runtime default' }}
              </div>
              <div class="muted">{{ props.pluginPromptSourceSummary(entry) }}</div>
              <div class="muted">{{ props.pluginUiSourceSummary(entry) }}</div>
              <div v-if="entry.help && entry.effectiveUiDisplayMode === 'detailed'" class="muted mono" style="white-space: pre-wrap">
                {{ entry.help }}
              </div>
            </div>
            <div class="button-row" style="flex-wrap: wrap" @click.stop>
              <button class="button" @click="props.togglePluginEntryDisabled(entry)">
                {{ entry.disabled ? 'Enable' : 'Disable' }} Entry
              </button>
            </div>
          </summary>

          <div class="stack" style="margin-top: 16px">
            <div class="button-row" style="align-items: flex-start; flex-wrap: wrap">
              <label class="stack" style="min-width: 280px">
                <span class="muted">Prompt Override</span>
                <select class="input" :value="entry.filePromptOverride || 'tool_default'" @change="applyPromptOverride(entry, $event)">
                  <option v-for="option in props.promptOverrideOptions" :key="option.value" :value="option.value">
                    {{ promptOptionLabel(entry.effectivePromptMode, option) }}
                  </option>
                </select>
                <span class="muted">
                  {{
                    promptOptionDescription(
                      entry.effectivePromptMode,
                      props.promptOverrideOptions.find((option) => option.value === (entry.filePromptOverride || 'tool_default')) || props.promptOverrideOptions[0],
                      entry.filePromptOverride,
                    )
                  }}
                </span>
              </label>

              <label class="stack" style="min-width: 280px">
                <span class="muted">UI Display Override</span>
                <select class="input" :value="entry.fileUiDisplayOverride || 'default'" @change="applyUiOverride(entry, $event)">
                  <option v-for="option in props.uiDisplayOverrideOptions" :key="option.value" :value="option.value">
                    {{ uiOptionLabel(entry.effectiveUiDisplayMode, option) }}
                  </option>
                </select>
                <span class="muted">
                  {{
                    uiOptionDescription(
                      entry.effectiveUiDisplayMode,
                      props.uiDisplayOverrideOptions.find((option) => option.value === (entry.fileUiDisplayOverride || 'default')) || props.uiDisplayOverrideOptions[0],
                      entry.fileUiDisplayOverride,
                    )
                  }}
                </span>
              </label>
            </div>

            <div v-if="visibleTools(entry).length === 0" class="muted">
              {{ entry.manifestAvailable ? 'This plugin does not declare tools.' : 'Tool metadata is unavailable until the plugin can be inspected at runtime.' }}
            </div>

            <div v-else class="list">
              <div v-for="tool in visibleTools(entry)" :key="tool.toolKey" class="list-item">
                <div class="page-header" style="align-items: flex-start">
                  <div>
                    <div class="button-row" style="align-items: center; flex-wrap: wrap">
                      <strong>{{ tool.toolName }}</strong>
                      <span class="badge">prompt={{ props.promptModeLabel(tool.effectivePromptMode) }}</span>
                      <span class="badge">ui={{ props.uiDisplayModeLabel(tool.effectiveUiDisplayMode) }}</span>
                      <span v-if="tool.filePromptOverride || tool.fileUiDisplayOverride" class="badge warn">changed</span>
                      <span v-if="tool.declaredPromptMode" class="badge">declared={{ props.promptModeLabel(tool.declaredPromptMode) }}</span>
                      <span v-if="tool.declaredUiDisplayMode" class="badge">ui-default={{ props.uiDisplayModeLabel(tool.declaredUiDisplayMode) }}</span>
                    </div>
                    <div class="muted">{{ props.primaryToolText(tool) }}</div>
                    <div v-if="props.secondaryToolText(tool)" class="muted">{{ props.secondaryToolText(tool) }}</div>
                    <div class="muted">{{ props.toolPromptSourceSummary(entry, tool) }}</div>
                    <div class="muted">{{ props.toolUiSourceSummary(entry, tool) }}</div>
                    <div v-if="tool.help && tool.effectiveUiDisplayMode === 'detailed'" class="muted mono" style="white-space: pre-wrap">
                      {{ tool.help }}
                    </div>
                    <div v-if="tool.tags.length" class="muted mono">tags={{ tool.tags.join(', ') }}</div>
                  </div>
                </div>

                <div class="button-row" style="margin-top: 12px; align-items: flex-start; flex-wrap: wrap">
                  <label class="stack" style="min-width: 280px">
                    <span class="muted">Prompt Override</span>
                    <select class="input" :value="tool.filePromptOverride || 'tool_default'" @change="applyToolPromptOverride(entry, tool, $event)">
                      <option v-for="option in props.promptOverrideOptions" :key="option.value" :value="option.value">
                        {{ promptOptionLabel(tool.effectivePromptMode, option) }}
                      </option>
                    </select>
                    <span class="muted">
                      {{
                        promptOptionDescription(
                          tool.effectivePromptMode,
                          props.promptOverrideOptions.find((option) => option.value === (tool.filePromptOverride || 'tool_default')) || props.promptOverrideOptions[0],
                          tool.filePromptOverride,
                        )
                      }}
                    </span>
                  </label>

                  <label class="stack" style="min-width: 280px">
                    <span class="muted">UI Display Override</span>
                    <select class="input" :value="tool.fileUiDisplayOverride || 'default'" @change="applyToolUiOverride(entry, tool, $event)">
                      <option v-for="option in props.uiDisplayOverrideOptions" :key="option.value" :value="option.value">
                        {{ uiOptionLabel(tool.effectiveUiDisplayMode, option) }}
                      </option>
                    </select>
                    <span class="muted">
                      {{
                        uiOptionDescription(
                          tool.effectiveUiDisplayMode,
                          props.uiDisplayOverrideOptions.find((option) => option.value === (tool.fileUiDisplayOverride || 'default')) || props.uiDisplayOverrideOptions[0],
                          tool.fileUiDisplayOverride,
                        )
                      }}
                    </span>
                  </label>
                </div>
              </div>
            </div>
          </div>
        </details>

        <div class="button-row" style="justify-content: flex-end; gap: 8px">
          <button class="button ghost" :disabled="currentPage <= 1" @click="goToPreviousPage">Previous Page</button>
          <button class="button ghost" :disabled="currentPage >= totalPages" @click="goToNextPage">Next Page</button>
        </div>
      </div>
    </section>
  </div>
</template>
