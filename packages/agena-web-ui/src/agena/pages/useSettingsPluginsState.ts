import { computed, type Ref } from 'vue'

import {
  deleteSettings,
  patchSettings,
  setSettings,
  type ConfigSettingsDeleteRequest,
  type ConfigSettingsEditResponse,
  type ConfigSettingsPatchRequest,
  type ConfigSettingsSetRequest,
} from '../lib/agenaApi'
import type {
  PluginUiDisplayMode,
  PluginUiDisplayOverride,
  SettingsPluginEntrySnapshot,
  SettingsPluginToolSnapshot,
  SettingsPluginsConfigSnapshot,
  ToolDescriptionMode,
  ToolDescriptionOverride,
} from './runtimePageLoaders'

const TOOL_PROMPT_POLICY_PATH = 'plugins.policy.tool_presentation'
const TOOL_UI_POLICY_PATH = 'plugins.policy.ui_presentation'

export type SettingsPluginsStateInput = {
  actionError: Ref<string>
  actionMessage: Ref<string>
  load: () => Promise<void>
  settingsPlugins: Ref<SettingsPluginsConfigSnapshot | null>
}

export type SettingsPluginsStateDeps = {
  deleteSettings: (input: ConfigSettingsDeleteRequest) => Promise<ConfigSettingsEditResponse>
  patchSettings: (input: ConfigSettingsPatchRequest) => Promise<ConfigSettingsEditResponse>
  setSettings: (input: ConfigSettingsSetRequest) => Promise<ConfigSettingsEditResponse>
}

const defaultDeps: SettingsPluginsStateDeps = {
  deleteSettings,
  patchSettings,
  setSettings,
}

function toolPromptModeLabel(mode: ToolDescriptionMode): string {
  return mode === 'brief' ? 'Brief' : 'Detailed'
}

function toolPromptOverrideLabel(mode: ToolDescriptionOverride | null): string {
  if (mode === 'brief') return 'Brief'
  if (mode === 'detailed') return 'Detailed'
  return 'Declared Default'
}

function uiDisplayModeLabel(mode: PluginUiDisplayMode): string {
  return mode === 'summary' ? 'Summary' : 'Detailed'
}

function uiDisplayOverrideLabel(mode: PluginUiDisplayOverride | null): string {
  if (mode === 'summary') return 'Summary'
  if (mode === 'detailed') return 'Detailed'
  return 'Declared Default'
}

function formatFileSummary(settings: SettingsPluginsConfigSnapshot | null): string {
  if (!settings) return 'not loaded'
  const prompt = settings.promptPresentation.fileDefaultMode
  const ui = settings.uiPresentation.fileDefaultMode
  const parts = [
    prompt == null ? null : `prompt=${toolPromptModeLabel(prompt)}`,
    ui == null ? null : `ui=${uiDisplayModeLabel(ui)}`,
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
    entry.source === 'file' ? 'file config' : 'runtime only',
    entry.kind || 'unknown',
    entry.manifestAvailable ? `${entry.tools.length} tools` : 'manifest unavailable',
    entry.disabled ? 'disabled · skipped on reload' : 'enabled · loads on reload',
  ]
  return facts.join(' · ')
}

function primaryPluginText(entry: SettingsPluginEntrySnapshot): string {
  if (entry.effectiveUiDisplayMode === 'summary') {
    return entry.summary || entry.description || 'No summary available.'
  }
  return entry.description || entry.summary || 'No description available.'
}

function secondaryPluginText(entry: SettingsPluginEntrySnapshot): string {
  if (entry.effectiveUiDisplayMode === 'detailed' && entry.summary && entry.summary !== entry.description) {
    return entry.summary
  }
  return ''
}

function primaryToolText(tool: SettingsPluginToolSnapshot): string {
  if (tool.effectiveUiDisplayMode === 'summary') {
    return tool.summary || tool.description || 'No summary available.'
  }
  return tool.description || tool.summary || 'No description available.'
}

function secondaryToolText(tool: SettingsPluginToolSnapshot): string {
  if (tool.effectiveUiDisplayMode === 'detailed' && tool.summary && tool.summary !== tool.description) {
    return tool.summary
  }
  return ''
}

function pluginPromptSourceSummary(entry: SettingsPluginEntrySnapshot): string {
  if (entry.filePromptOverride === 'detailed' || entry.filePromptOverride === 'brief') {
    return `Prompt is forced to ${toolPromptModeLabel(entry.effectivePromptMode)} by a plugin override.`
  }
  if (entry.declaredPromptDefault) {
    return `Prompt follows the plugin declared default (${toolPromptModeLabel(entry.declaredPromptDefault)}).`
  }
  return `Prompt falls back to the global default (${toolPromptModeLabel(entry.effectivePromptMode)}).`
}

function pluginUiSourceSummary(entry: SettingsPluginEntrySnapshot): string {
  if (entry.fileUiDisplayOverride === 'detailed' || entry.fileUiDisplayOverride === 'summary') {
    return `UI text is forced to ${uiDisplayModeLabel(entry.effectiveUiDisplayMode)} by a plugin override.`
  }
  if (entry.declaredUiDefault) {
    return `UI text follows the plugin declared default (${uiDisplayModeLabel(entry.declaredUiDefault)}).`
  }
  return `UI text falls back to the global default (${uiDisplayModeLabel(entry.effectiveUiDisplayMode)}).`
}

function toolPromptSourceSummary(entry: SettingsPluginEntrySnapshot, tool: SettingsPluginToolSnapshot): string {
  if (tool.filePromptOverride === 'detailed' || tool.filePromptOverride === 'brief') {
    return `Prompt is forced to ${toolPromptModeLabel(tool.effectivePromptMode)} by a tool override.`
  }
  if (entry.filePromptOverride === 'detailed' || entry.filePromptOverride === 'brief') {
    return `Prompt inherits the plugin override (${toolPromptModeLabel(tool.effectivePromptMode)}).`
  }
  if (tool.declaredPromptMode) {
    return `Prompt follows the tool declared default (${toolPromptModeLabel(tool.declaredPromptMode)}).`
  }
  if (entry.declaredPromptDefault) {
    return `Prompt falls back to the plugin declared default (${toolPromptModeLabel(entry.declaredPromptDefault)}).`
  }
  return `Prompt falls back to the global default (${toolPromptModeLabel(tool.effectivePromptMode)}).`
}

function toolUiSourceSummary(entry: SettingsPluginEntrySnapshot, tool: SettingsPluginToolSnapshot): string {
  if (tool.fileUiDisplayOverride === 'detailed' || tool.fileUiDisplayOverride === 'summary') {
    return `UI text is forced to ${uiDisplayModeLabel(tool.effectiveUiDisplayMode)} by a tool override.`
  }
  if (entry.fileUiDisplayOverride === 'detailed' || entry.fileUiDisplayOverride === 'summary') {
    return `UI text inherits the plugin override (${uiDisplayModeLabel(tool.effectiveUiDisplayMode)}).`
  }
  if (tool.declaredUiDisplayMode) {
    return `UI text follows the tool declared default (${uiDisplayModeLabel(tool.declaredUiDisplayMode)}).`
  }
  if (entry.declaredUiDefault) {
    return `UI text falls back to the plugin declared default (${uiDisplayModeLabel(entry.declaredUiDefault)}).`
  }
  return `UI text falls back to the global default (${uiDisplayModeLabel(tool.effectiveUiDisplayMode)}).`
}

function pluginHasOverrides(entry: SettingsPluginEntrySnapshot): boolean {
  return (
    entry.disabled ||
    entry.filePromptOverride != null ||
    entry.fileUiDisplayOverride != null ||
    entry.tools.some((tool) => tool.filePromptOverride != null || tool.fileUiDisplayOverride != null)
  )
}

function promptOverridePath(pluginId: string, toolName?: string): string {
  if (!toolName) {
    return `${TOOL_PROMPT_POLICY_PATH}.plugins.${JSON.stringify(pluginId)}`
  }
  return `${TOOL_PROMPT_POLICY_PATH}.tools.${JSON.stringify(`${pluginId}/${toolName}`)}`
}

function uiDisplayOverridePath(pluginId: string, toolName?: string): string {
  if (!toolName) {
    return `${TOOL_UI_POLICY_PATH}.plugins.${JSON.stringify(pluginId)}`
  }
  return `${TOOL_UI_POLICY_PATH}.tools.${JSON.stringify(`${pluginId}/${toolName}`)}`
}

export function useSettingsPluginsState(input: SettingsPluginsStateInput, deps: SettingsPluginsStateDeps = defaultDeps) {
  const plugins = computed(() => input.settingsPlugins.value?.plugins ?? [])
  const summaryFacts = computed(() => {
    const settings = input.settingsPlugins.value
    if (!settings) return []
    return [
      { label: 'Config Path', value: settings.configPath || 'n/a' },
      { label: 'Config Found', value: settings.configFound ? 'yes' : 'no' },
      { label: 'Prompt Default', value: toolPromptModeLabel(settings.promptPresentation.defaultMode) },
      { label: 'UI Default', value: uiDisplayModeLabel(settings.uiPresentation.defaultMode) },
      { label: 'File Override', value: formatFileSummary(settings) },
      {
        label: 'Prompt Overrides',
        value: `${Object.keys(settings.promptPresentation.effectivePluginOverrides).length} plugin · ${Object.keys(settings.promptPresentation.effectiveToolOverrides).length} tool`,
      },
      {
        label: 'UI Overrides',
        value: `${Object.keys(settings.uiPresentation.effectivePluginOverrides).length} plugin · ${Object.keys(settings.uiPresentation.effectiveToolOverrides).length} tool`,
      },
      {
        label: 'Plugin Entries',
        value: String(settings.plugins.length),
      },
    ]
  })

  const promptDefaultMode = computed(() => input.settingsPlugins.value?.promptPresentation.defaultMode ?? 'detailed')
  const uiDefaultMode = computed(() => input.settingsPlugins.value?.uiPresentation.defaultMode ?? 'detailed')

  const promptModeOptions: Array<{ label: string; value: ToolDescriptionMode; description: string }> = [
    {
      label: 'Detailed',
      value: 'detailed',
      description: 'Expose the model-visible description text as-is.',
    },
    {
      label: 'Brief',
      value: 'brief',
      description: 'Keep tool descriptions short and push detail into tool help.',
    },
  ]

  const promptOverrideOptions: Array<{ label: string; value: ToolDescriptionOverride; description: string }> = [
    {
      label: 'Declared Default',
      value: 'tool_default',
      description: 'Remove the file override and follow the plugin or tool declared default chain.',
    },
    {
      label: 'Detailed',
      value: 'detailed',
      description: 'Force full prompt definitions for this plugin or tool.',
    },
    {
      label: 'Brief',
      value: 'brief',
      description: 'Force compact prompt definitions for this plugin or tool.',
    },
  ]

  const uiDisplayModeOptions: Array<{ label: string; value: PluginUiDisplayMode; description: string }> = [
    {
      label: 'Detailed',
      value: 'detailed',
      description: 'Show the full description and any extra help in plugin inspectors.',
    },
    {
      label: 'Summary',
      value: 'summary',
      description: 'Prefer short summary text in plugin inspectors and tool listings.',
    },
  ]

  const uiDisplayOverrideOptions: Array<{ label: string; value: PluginUiDisplayOverride; description: string }> = [
    {
      label: 'Declared Default',
      value: 'default',
      description: 'Remove the file override and follow the plugin or tool declared UI default chain.',
    },
    {
      label: 'Detailed',
      value: 'detailed',
      description: 'Always show detailed plugin and tool text in the UI.',
    },
    {
      label: 'Summary',
      value: 'summary',
      description: 'Always prefer summary text in the UI.',
    },
  ]

  async function commitScalarSetting(
    operation: () => Promise<ConfigSettingsEditResponse>,
    successMessage: string,
  ) {
    input.actionError.value = ''
    input.actionMessage.value = ''
    try {
      await operation()
      input.actionMessage.value = successMessage
      await input.load()
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    }
  }

  async function setDefaultToolDescriptionMode(mode: ToolDescriptionMode) {
    const settings = input.settingsPlugins.value
    if (!settings || settings.promptPresentation.defaultMode === mode) return
    await commitScalarSetting(
      () =>
        deps.patchSettings({
          path: TOOL_PROMPT_POLICY_PATH,
          changes: { default_mode: mode },
          validate: true,
          reload: true,
        }),
      `Tool prompt mode now defaults to ${toolPromptModeLabel(mode)}.`,
    )
  }

  async function setDefaultUiDisplayMode(mode: PluginUiDisplayMode) {
    const settings = input.settingsPlugins.value
    if (!settings || settings.uiPresentation.defaultMode === mode) return
    await commitScalarSetting(
      () =>
        deps.patchSettings({
          path: TOOL_UI_POLICY_PATH,
          changes: { default_mode: mode },
          validate: true,
          reload: true,
        }),
      `Plugin UI display now defaults to ${uiDisplayModeLabel(mode)}.`,
    )
  }

  async function setPluginPromptOverride(pluginId: string, mode: ToolDescriptionOverride) {
    const path = promptOverridePath(pluginId)
    await commitScalarSetting(
      () =>
        mode === 'tool_default'
          ? deps.deleteSettings({ path, validate: true, reload: true })
          : deps.setSettings({ path, value: mode, validate: true, reload: true }),
      mode === 'tool_default'
        ? `Prompt override cleared for ${pluginId}; tool defaults now apply.`
        : `Prompt override for ${pluginId} set to ${toolPromptOverrideLabel(mode)}.`,
    )
  }

  async function setToolPromptOverride(pluginId: string, toolName: string, mode: ToolDescriptionOverride) {
    const path = promptOverridePath(pluginId, toolName)
    await commitScalarSetting(
      () =>
        mode === 'tool_default'
          ? deps.deleteSettings({ path, validate: true, reload: true })
          : deps.setSettings({ path, value: mode, validate: true, reload: true }),
      mode === 'tool_default'
        ? `Prompt override cleared for ${pluginId}/${toolName}.`
        : `Prompt override for ${pluginId}/${toolName} set to ${toolPromptOverrideLabel(mode)}.`,
    )
  }

  async function setPluginUiDisplayOverride(pluginId: string, mode: PluginUiDisplayOverride) {
    const path = uiDisplayOverridePath(pluginId)
    await commitScalarSetting(
      () =>
        mode === 'default'
          ? deps.deleteSettings({ path, validate: true, reload: true })
          : deps.setSettings({ path, value: mode, validate: true, reload: true }),
      mode === 'default'
        ? `UI display override cleared for ${pluginId}.`
        : `UI display override for ${pluginId} set to ${uiDisplayOverrideLabel(mode)}.`,
    )
  }

  async function setToolUiDisplayOverride(pluginId: string, toolName: string, mode: PluginUiDisplayOverride) {
    const path = uiDisplayOverridePath(pluginId, toolName)
    await commitScalarSetting(
      () =>
        mode === 'default'
          ? deps.deleteSettings({ path, validate: true, reload: true })
          : deps.setSettings({ path, value: mode, validate: true, reload: true }),
      mode === 'default'
        ? `UI display override cleared for ${pluginId}/${toolName}.`
        : `UI display override for ${pluginId}/${toolName} set to ${uiDisplayOverrideLabel(mode)}.`,
    )
  }

  async function togglePluginEntryDisabled(entry: SettingsPluginEntrySnapshot) {
    await commitScalarSetting(
      () =>
        deps.setSettings({
          path: `plugins.list.${JSON.stringify(entry.pluginId)}`,
          value: clonePluginEntry(entry, !entry.disabled),
          validate: true,
          reload: true,
        }),
      entry.disabled
        ? `Enabled plugin ${entry.pluginId}; config kept and runtime reloaded.`
        : `Disabled plugin ${entry.pluginId}; config kept and runtime reloaded.`,
    )
  }

  return {
    actionError: input.actionError,
    actionMessage: input.actionMessage,
    load: input.load,
    pluginEntrySummary,
    pluginHasOverrides,
    pluginPromptSourceSummary,
    plugins,
    primaryPluginText,
    primaryToolText,
    promptDefaultMode,
    promptModeOptions,
    promptOverrideLabel: toolPromptOverrideLabel,
    promptOverrideOptions,
    secondaryPluginText,
    secondaryToolText,
    setDefaultToolDescriptionMode,
    setDefaultUiDisplayMode,
    setPluginPromptOverride,
    setPluginUiDisplayOverride,
    setToolPromptOverride,
    setToolUiDisplayOverride,
    summaryFacts,
    togglePluginEntryDisabled,
    toolPromptModeLabel,
    toolPromptSourceSummary,
    uiDefaultMode,
    uiDisplayModeLabel,
    uiDisplayModeOptions,
    uiDisplayOverrideLabel,
    uiDisplayOverrideOptions,
    pluginUiSourceSummary,
    toolUiSourceSummary,
  }
}
