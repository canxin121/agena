import {
  fetchRuntimeStatus,
  getSettings,
  getPlugin,
  listAuthProviders,
  listPermissionRules,
  listPluginLogs,
  listPlugins,
  listProviders,
  listSessions,
  listWorkspaces,
  type AuthProvider,
  type PluginInspect,
  type PermissionRuleResource,
  type PluginLogEntry,
  type PluginStatus,
  type ProviderModel,
  type ProviderSummary,
  type RuntimeStatus,
  type SessionResource,
  type WorkspaceResource,
} from '../lib/agenaApi'
import { pickNextPluginId } from './runtimePageModel'
import { pickSessionId, pickWorkspaceId } from './runtimePageStateModel'

export type ToolDescriptionMode = 'detailed' | 'help'

export type SettingsPluginsConfigSnapshot = {
  configPath: string
  configFound: boolean
  enabled: boolean
  defaultMode: ToolDescriptionMode
  fileEnabled: boolean | null
  fileDefaultMode: ToolDescriptionMode | null
  pluginEntries: SettingsPluginEntrySnapshot[]
  toolPresentationPluginOverridesCount: number
  toolPresentationToolOverridesCount: number
}

export type SettingsPluginEntrySnapshot = {
  pluginId: string
  kind: string
  disabled: boolean
  source: 'file' | 'runtime'
  filePresent: boolean
  entry: Record<string, unknown>
}

export type RuntimeSectionData = {
  runtime: RuntimeStatus
  providers: ProviderSummary[]
  providerModels: Record<string, ProviderModel[]>
  workspaces: WorkspaceResource[]
  sessions: SessionResource[]
  selectedWorkspaceId: number | null
  selectedSessionId: number | null
}

export type SettingsSectionData = {
  authProviders: AuthProvider[]
  settingsPlugins: SettingsPluginsConfigSnapshot
  runtime: RuntimeStatus
  providers: ProviderSummary[]
  providerModels: Record<string, ProviderModel[]>
  permissionRules: PermissionRuleResource[]
}

export type PluginsSectionData = {
  plugins: PluginStatus[]
  runtime: RuntimeStatus
  workspaces: WorkspaceResource[]
  selectedWorkspaceId: number | null
  selectedPluginId: string
}

export async function loadRuntimeSectionData(input: {
  selectedWorkspaceId: number | null
  selectedSessionId: number | null
}): Promise<RuntimeSectionData> {
  const [runtime, providers, workspaces] = await Promise.all([fetchRuntimeStatus(), listProviders(), listWorkspaces()])

  const selectedWorkspaceId = pickWorkspaceId(input.selectedWorkspaceId, workspaces)
  const sessions = selectedWorkspaceId ? await listSessions(selectedWorkspaceId) : []
  const selectedSessionId = pickSessionId(input.selectedSessionId, sessions)

  return {
    runtime,
    providers,
    providerModels: {},
    workspaces,
    sessions,
    selectedWorkspaceId,
    selectedSessionId,
  }
}

export async function loadSettingsSectionData(permissionSearch: string): Promise<SettingsSectionData> {
  const [authProviders, permissionRules, runtime, providers, plugins, effectivePlugins, filePlugins] = await Promise.all([
    listAuthProviders(),
    listPermissionRules(permissionSearch),
    fetchRuntimeStatus(),
    listProviders(),
    listPlugins(),
    getSettings({ path: 'plugins', source: 'effective' }),
    getSettings({ path: 'plugins', source: 'file' }),
  ])

  const pluginDetails = await Promise.allSettled(plugins.map((plugin) => getPlugin(plugin.plugin_id)))

  return {
    authProviders,
    settingsPlugins: readSettingsPluginsConfig(effectivePlugins, filePlugins, pluginDetails),
    runtime,
    providers,
    providerModels: {},
    permissionRules,
  }
}

export async function loadPluginsSectionData(input: {
  selectedPluginId: string
  selectedWorkspaceId: number | null
}): Promise<PluginsSectionData> {
  const [plugins, runtime, workspaces] = await Promise.all([listPlugins(), fetchRuntimeStatus(), listWorkspaces()])

  return {
    plugins,
    runtime,
    workspaces,
    selectedWorkspaceId: pickWorkspaceId(input.selectedWorkspaceId, workspaces),
    selectedPluginId: pickNextPluginId(input.selectedPluginId, plugins),
  }
}

export async function loadPluginLogsSnapshot(pluginId: string): Promise<PluginLogEntry[]> {
  return listPluginLogs(pluginId, { limit: 50 })
}

function readRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {}
}

function readPluginEntry(value: unknown): Record<string, unknown> {
  return readRecord(value)
}

function readOptionalBoolean(value: unknown): boolean | null {
  return typeof value === 'boolean' ? value : null
}

function readOptionalToolDescriptionMode(value: unknown): ToolDescriptionMode | null {
  return value === 'detailed' || value === 'help' ? value : null
}

function readToolDescriptionMode(value: unknown, fallback: ToolDescriptionMode): ToolDescriptionMode {
  return readOptionalToolDescriptionMode(value) ?? fallback
}

function readSettingsPluginsConfig(
  effective: { config_path: string; config_found: boolean; value: unknown },
  file: { value: unknown },
  runtimePluginDetails: PromiseSettledResult<PluginInspect>[],
): SettingsPluginsConfigSnapshot {
  const effectiveRoot = readRecord(effective.value)
  const effectiveToolPresentation = readRecord(effectiveRoot.tool_presentation)
  const fileRoot = readRecord(file.value)
  const fileToolPresentation = readRecord(fileRoot.tool_presentation)
  const filePluginEntries = readRecord(fileRoot.list)
  const runtimePluginEntries = new Map<string, Record<string, unknown>>()
  for (const result of runtimePluginDetails) {
    if (result.status !== 'fulfilled') continue
    const pluginId = result.value.status.plugin_id
    if (!pluginId.trim()) continue
    runtimePluginEntries.set(pluginId, readPluginEntry(result.value.entry))
  }
  const pluginIds = new Set<string>([
    ...Object.keys(filePluginEntries),
    ...runtimePluginEntries.keys(),
  ])
  const pluginEntries = Array.from(pluginIds)
    .sort((a, b) => a.localeCompare(b))
    .map((pluginId) => {
      const fileEntry = filePluginEntries[pluginId]
      const runtimeEntry = runtimePluginEntries.get(pluginId)
      const entry = readPluginEntry(fileEntry ?? runtimeEntry ?? {})
      const disabled = entry.disabled === true
      const kind = typeof entry.kind === 'string' && entry.kind.trim() ? entry.kind.trim() : 'unknown'
      return {
        pluginId,
        kind,
        disabled,
        source: fileEntry ? 'file' : 'runtime',
        filePresent: fileEntry != null,
        entry,
      } satisfies SettingsPluginEntrySnapshot
    })
  return {
    configPath: effective.config_path,
    configFound: effective.config_found,
    enabled: typeof effectiveRoot.enabled === 'boolean' ? effectiveRoot.enabled : true,
    defaultMode: readToolDescriptionMode(effectiveToolPresentation.default_mode, 'detailed'),
    fileEnabled: readOptionalBoolean(fileRoot.enabled),
    fileDefaultMode: readOptionalToolDescriptionMode(fileToolPresentation.default_mode),
    pluginEntries,
    toolPresentationPluginOverridesCount: Object.keys(readRecord(effectiveToolPresentation.plugins)).length,
    toolPresentationToolOverridesCount: Object.keys(readRecord(effectiveToolPresentation.tools)).length,
  }
}
