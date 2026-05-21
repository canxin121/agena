import { computed, type Ref } from 'vue'

import {
  patchSettings,
  setSettings,
  type ConfigSettingsEditResponse,
  type ConfigSettingsPatchRequest,
  type ConfigSettingsSetRequest,
} from '../lib/agenaApi'
import type {
  SettingsPluginEntrySnapshot,
  SettingsPluginsConfigSnapshot,
  ToolDescriptionMode,
} from './runtimePageLoaders'

export type SettingsPluginsStateInput = {
  actionError: Ref<string>
  actionMessage: Ref<string>
  load: () => Promise<void>
  settingsPlugins: Ref<SettingsPluginsConfigSnapshot | null>
}

export type SettingsPluginsStateDeps = {
  patchSettings: (input: ConfigSettingsPatchRequest) => Promise<ConfigSettingsEditResponse>
  setSettings: (input: ConfigSettingsSetRequest) => Promise<ConfigSettingsEditResponse>
}

const defaultDeps: SettingsPluginsStateDeps = {
  patchSettings,
  setSettings,
}

function modeLabel(mode: ToolDescriptionMode): string {
  return mode === 'help' ? 'Help' : 'Detailed'
}

function formatFileSummary(settings: SettingsPluginsConfigSnapshot | null): string {
  if (!settings) return 'not loaded'
  const parts = [
    settings.fileEnabled == null ? null : `enabled=${settings.fileEnabled ? 'on' : 'off'}`,
    settings.fileDefaultMode == null ? null : `default=${modeLabel(settings.fileDefaultMode)}`,
  ].filter(Boolean)
  return parts.length ? parts.join(' · ') : 'inherits defaults'
}

function readRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {}
}

function clonePluginEntry(entry: SettingsPluginEntrySnapshot, disabled: boolean): Record<string, unknown> {
  const value = JSON.parse(JSON.stringify(entry.entry)) as Record<string, unknown>
  const record = readRecord(value)
  record.disabled = disabled
  return record
}

function pluginEntrySummary(entry: SettingsPluginEntrySnapshot): string {
  const facts = [
    entry.source === 'file' ? 'file' : 'runtime',
    entry.kind || 'unknown',
    entry.disabled ? 'disabled · skipped on reload' : 'enabled · loads on reload',
  ]
  return facts.join(' · ')
}

export function useSettingsPluginsState(input: SettingsPluginsStateInput, deps: SettingsPluginsStateDeps = defaultDeps) {
  const pluginEntries = computed(() => input.settingsPlugins.value?.pluginEntries ?? [])
  const summaryFacts = computed(() => {
    const settings = input.settingsPlugins.value
    if (!settings) return []
    return [
      { label: 'Config Path', value: settings.configPath || 'n/a' },
      { label: 'Config Found', value: settings.configFound ? 'yes' : 'no' },
      { label: 'Enabled', value: settings.enabled ? 'on' : 'off' },
      { label: 'Default Tool Description', value: modeLabel(settings.defaultMode) },
      { label: 'File Override', value: formatFileSummary(settings) },
      {
        label: 'Plugin Overrides',
        value: String(settings.toolPresentationPluginOverridesCount),
      },
      {
        label: 'Tool Overrides',
        value: String(settings.toolPresentationToolOverridesCount),
      },
      {
        label: 'Plugin Entries',
        value: String(settings.pluginEntries.length),
      },
    ]
  })

  const enabled = computed(() => input.settingsPlugins.value?.enabled ?? true)
  const defaultMode = computed(() => input.settingsPlugins.value?.defaultMode ?? 'detailed')
  const modeOptions: Array<{ label: string; value: ToolDescriptionMode; description: string }> = [
    {
      label: 'Detailed',
      value: 'detailed',
      description: 'Expose the model-visible description text as-is.',
    },
    {
      label: 'Help',
      value: 'help',
      description: 'Keep tool descriptions short and push details into help.',
    },
  ]

  async function togglePluginsEnabled() {
    const settings = input.settingsPlugins.value
    if (!settings) return
    input.actionError.value = ''
    input.actionMessage.value = ''
    try {
      await deps.patchSettings({
        path: 'plugins',
        changes: { enabled: !settings.enabled },
        validate: true,
        reload: true,
      })
      input.actionMessage.value = settings.enabled
        ? 'Plugins disabled; runtime reloaded.'
        : 'Plugins enabled; runtime reloaded.'
      await input.load()
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  async function setDefaultToolDescriptionMode(mode: ToolDescriptionMode) {
    const settings = input.settingsPlugins.value
    if (!settings || settings.defaultMode === mode) return
    input.actionError.value = ''
    input.actionMessage.value = ''
    try {
      await deps.patchSettings({
        path: 'plugins.tool_presentation',
        changes: { default_mode: mode },
        validate: true,
        reload: true,
      })
      input.actionMessage.value = `Tool descriptions now default to ${modeLabel(mode)} mode.`
      await input.load()
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  async function togglePluginEntryDisabled(entry: SettingsPluginEntrySnapshot) {
    input.actionError.value = ''
    input.actionMessage.value = ''
    try {
      await deps.setSettings({
        path: `plugins.list.${JSON.stringify(entry.pluginId)}`,
        value: clonePluginEntry(entry, !entry.disabled),
        validate: true,
        reload: true,
      })
      input.actionMessage.value = entry.disabled
        ? `Enabled plugin ${entry.pluginId}; config kept and runtime reloaded.`
        : `Disabled plugin ${entry.pluginId}; config kept and runtime reloaded.`
      await input.load()
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  return {
    actionError: input.actionError,
    actionMessage: input.actionMessage,
    defaultMode,
    enabled,
    load: input.load,
    modeOptions,
    pluginEntries,
    setDefaultToolDescriptionMode,
    pluginEntrySummary,
    togglePluginEntryDisabled,
    summaryFacts,
    togglePluginsEnabled,
  }
}
