import {
  fetchRuntimeStatus,
  getPlugin,
  getSettings,
  listAuthProviders,
  listPermissionRules,
  listPluginLogs,
  listPlugins,
  listProviders,
  listSessions,
  listWorkspaces,
  type AuthProvider,
  type ConfigSettingsReadResponse,
  type PermissionRuleResource,
  type PluginInspect,
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

export type ToolDescriptionMode = 'detailed' | 'brief'
export type ToolDescriptionOverride = 'tool_default' | 'detailed' | 'brief'
export type PluginUiDisplayMode = 'detailed' | 'summary'
export type PluginUiDisplayOverride = 'default' | 'detailed' | 'summary'

export type SettingsPluginPromptPresentationSnapshot = {
  defaultMode: ToolDescriptionMode
  fileDefaultMode: ToolDescriptionMode | null
  effectivePluginOverrides: Record<string, ToolDescriptionOverride>
  effectiveToolOverrides: Record<string, ToolDescriptionOverride>
  filePluginOverrides: Record<string, ToolDescriptionOverride>
  fileToolOverrides: Record<string, ToolDescriptionOverride>
}

export type SettingsPluginUiPresentationSnapshot = {
  defaultMode: PluginUiDisplayMode
  fileDefaultMode: PluginUiDisplayMode | null
  effectivePluginOverrides: Record<string, PluginUiDisplayOverride>
  effectiveToolOverrides: Record<string, PluginUiDisplayOverride>
  filePluginOverrides: Record<string, PluginUiDisplayOverride>
  fileToolOverrides: Record<string, PluginUiDisplayOverride>
}

export type SettingsPluginToolSnapshot = {
  toolName: string
  toolKey: string
  description: string
  summary: string
  help: string
  tags: string[]
  declaredPromptMode: ToolDescriptionMode | null
  declaredUiDisplayMode: PluginUiDisplayMode | null
  filePromptOverride: ToolDescriptionOverride | null
  effectivePromptOverride: ToolDescriptionOverride | null
  effectivePromptMode: ToolDescriptionMode
  fileUiDisplayOverride: PluginUiDisplayOverride | null
  effectiveUiDisplayOverride: PluginUiDisplayOverride | null
  effectiveUiDisplayMode: PluginUiDisplayMode
}

export type SettingsPluginEntrySnapshot = {
  pluginId: string
  displayName: string
  kind: string
  disabled: boolean
  source: 'file' | 'runtime'
  filePresent: boolean
  manifestAvailable: boolean
  entry: Record<string, unknown>
  description: string
  summary: string
  help: string
  declaredPromptDefault: ToolDescriptionMode | null
  effectivePromptMode: ToolDescriptionMode
  declaredUiDefault: PluginUiDisplayMode | null
  filePromptOverride: ToolDescriptionOverride | null
  effectivePromptOverride: ToolDescriptionOverride | null
  fileUiDisplayOverride: PluginUiDisplayOverride | null
  effectiveUiDisplayOverride: PluginUiDisplayOverride | null
  effectiveUiDisplayMode: PluginUiDisplayMode
  tools: SettingsPluginToolSnapshot[]
}

export type SettingsPluginsConfigSnapshot = {
  configPath: string
  configFound: boolean
  promptPresentation: SettingsPluginPromptPresentationSnapshot
  uiPresentation: SettingsPluginUiPresentationSnapshot
  plugins: SettingsPluginEntrySnapshot[]
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
  permissionConfig: ConfigSettingsReadResponse
  settingsPlugins: SettingsPluginsConfigSnapshot
  runtime: RuntimeStatus
  providers: ProviderSummary[]
  providerModels: Record<string, ProviderModel[]>
  permissionRules: PermissionRuleResource[]
}

export type PluginsSectionData = {
  plugins: PluginStatus[]
  pluginUiPresentation: SettingsPluginUiPresentationSnapshot
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
  const [authProviders, permissionRules, permissionConfig, runtime, providers, plugins, effectivePlugins, filePlugins] =
    await Promise.all([
      listAuthProviders(),
      listPermissionRules(permissionSearch),
      getSettings({ path: 'permission', source: 'effective' }),
      fetchRuntimeStatus(),
      listProviders(),
      listPlugins(),
      getSettings({ path: 'plugins', source: 'effective' }),
      getSettings({ path: 'plugins', source: 'file' }),
    ])

  const pluginDetails = await Promise.allSettled(plugins.map((plugin) => getPlugin(plugin.plugin_id)))

  return {
    authProviders,
    permissionConfig,
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
  const [plugins, runtime, workspaces, effectivePlugins] = await Promise.all([
    listPlugins(),
    fetchRuntimeStatus(),
    listWorkspaces(),
    getSettings({ path: 'plugins', source: 'effective' }),
  ])
  const effectiveRoot = readRecord(effectivePlugins.value)

  return {
    plugins,
    pluginUiPresentation: readPluginUiPresentationSnapshot(effectiveRoot, {}),
    runtime,
    workspaces,
    selectedWorkspaceId: pickWorkspaceId(input.selectedWorkspaceId, workspaces),
    selectedPluginId: pickNextPluginId(input.selectedPluginId, plugins),
  }
}

export async function loadPluginLogsSnapshot(pluginId: string): Promise<PluginLogEntry[]> {
  return listPluginLogs(pluginId, { limit: 50 })
}

function readArray(value: unknown): unknown[] {
  return Array.isArray(value) ? value : []
}

function readRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {}
}

function readString(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

function readStringArray(value: unknown): string[] {
  return readArray(value)
    .map((item) => readString(item))
    .filter((item) => item.length > 0)
}

function readManifestToolDefinition(entry: Record<string, unknown>) {
  const model = readRecord(entry.model)
  const docs = readRecord(entry.docs)
  const permissions = readRecord(entry.permissions)
  const display = readRecord(entry.display)
  return {
    name: readString(entry.name) || 'unnamed entry',
    description: readString(model.description) || readString(docs.summary) || 'No description provided.',
    summary: readString(docs.summary),
    help: readString(docs.help),
    tags: readStringArray(permissions.tags),
    descriptionMode: readOptionalToolDescriptionMode(display.description_mode),
    uiDisplayMode: readOptionalPluginUiDisplayMode(display.ui_display_mode),
  }
}

function readPluginEntry(value: unknown): Record<string, unknown> {
  return readRecord(value)
}

function readOptionalToolDescriptionMode(value: unknown): ToolDescriptionMode | null {
  if (value === 'detailed') return 'detailed'
  if (value === 'brief' || value === 'help') return 'brief'
  return null
}

function readToolDescriptionMode(value: unknown, fallback: ToolDescriptionMode): ToolDescriptionMode {
  return readOptionalToolDescriptionMode(value) ?? fallback
}

function readOptionalToolDescriptionOverride(value: unknown): ToolDescriptionOverride | null {
  if (value === 'detailed') return 'detailed'
  if (value === 'brief' || value === 'help') return 'brief'
  if (value === 'tool_default' || value === 'default' || value === 'inherit') return 'tool_default'
  return null
}

function readOptionalPluginUiDisplayMode(value: unknown): PluginUiDisplayMode | null {
  return value === 'detailed' || value === 'summary' ? value : null
}

function readPluginUiDisplayMode(value: unknown, fallback: PluginUiDisplayMode): PluginUiDisplayMode {
  return readOptionalPluginUiDisplayMode(value) ?? fallback
}

function readOptionalPluginUiDisplayOverride(value: unknown): PluginUiDisplayOverride | null {
  if (value === 'default' || value === 'inherit') return 'default'
  if (value === 'detailed' || value === 'summary') return value
  return null
}

function readOverrideMap<TOverride extends string>(
  value: unknown,
  readOverride: (candidate: unknown) => TOverride | null,
): Record<string, TOverride> {
  const result: Record<string, TOverride> = {}
  for (const [key, candidate] of Object.entries(readRecord(value))) {
    const override = readOverride(candidate)
    if (!override) continue
    result[key] = override
  }
  return result
}

function canonicalToolKey(pluginId: string, toolName: string): string {
  return `${pluginId}/${toolName}`
}

function toolOverrideKeys(pluginId: string, toolName: string): string[] {
  const exposedName = canonicalToolKey(pluginId, toolName)
  return [exposedName, `${pluginId}/${toolName}`, `${pluginId}/${exposedName}`, `${pluginId}.${toolName}`, toolName]
}

function findToolOverride<TOverride extends string>(
  overrides: Record<string, TOverride>,
  pluginId: string,
  toolName: string,
): TOverride | null {
  for (const key of toolOverrideKeys(pluginId, toolName)) {
    if (Object.prototype.hasOwnProperty.call(overrides, key)) {
      return overrides[key] ?? null
    }
  }
  return null
}

function resolveToolDescriptionOverride(
  override: ToolDescriptionOverride,
  toolDefault: ToolDescriptionMode | null,
  fallback: ToolDescriptionMode,
): ToolDescriptionMode {
  if (override === 'detailed') return 'detailed'
  if (override === 'brief') return 'brief'
  return toolDefault ?? fallback
}

function resolveToolDescriptionModeForTool(
  presentation: SettingsPluginPromptPresentationSnapshot,
  pluginId: string,
  toolName: string,
  toolDefault: ToolDescriptionMode | null,
): ToolDescriptionMode {
  const toolOverride = findToolOverride(presentation.effectiveToolOverrides, pluginId, toolName)
  if (toolOverride) {
    return resolveToolDescriptionOverride(toolOverride, toolDefault, presentation.defaultMode)
  }
  const pluginOverride = presentation.effectivePluginOverrides[pluginId] ?? null
  if (pluginOverride) {
    return resolveToolDescriptionOverride(pluginOverride, toolDefault, presentation.defaultMode)
  }
  return toolDefault ?? presentation.defaultMode
}

function resolveToolDescriptionModeForPlugin(
  presentation: SettingsPluginPromptPresentationSnapshot,
  pluginId: string,
  pluginDefault: ToolDescriptionMode | null,
): ToolDescriptionMode {
  const pluginOverride = presentation.effectivePluginOverrides[pluginId] ?? null
  if (pluginOverride) {
    return resolveToolDescriptionOverride(pluginOverride, pluginDefault, presentation.defaultMode)
  }
  return pluginDefault ?? presentation.defaultMode
}

function resolveUiDisplayOverride(
  override: PluginUiDisplayOverride,
  fallback: PluginUiDisplayMode,
): PluginUiDisplayMode {
  if (override === 'summary') return 'summary'
  if (override === 'detailed') return 'detailed'
  return fallback
}

function resolveUiDisplayModeForPlugin(
  presentation: SettingsPluginUiPresentationSnapshot,
  pluginId: string,
  pluginDefault: PluginUiDisplayMode | null,
): PluginUiDisplayMode {
  const pluginOverride = presentation.effectivePluginOverrides[pluginId] ?? null
  if (pluginOverride) {
    return resolveUiDisplayOverride(pluginOverride, pluginDefault ?? presentation.defaultMode)
  }
  return pluginDefault ?? presentation.defaultMode
}

function resolveUiDisplayModeForTool(
  presentation: SettingsPluginUiPresentationSnapshot,
  pluginId: string,
  toolName: string,
  toolDefault: PluginUiDisplayMode | null,
  pluginDefault: PluginUiDisplayMode | null,
): PluginUiDisplayMode {
  const toolOverride = findToolOverride(presentation.effectiveToolOverrides, pluginId, toolName)
  if (toolOverride) {
    return resolveUiDisplayOverride(toolOverride, toolDefault ?? pluginDefault ?? presentation.defaultMode)
  }
  const pluginOverride = presentation.effectivePluginOverrides[pluginId] ?? null
  if (pluginOverride) {
    return resolveUiDisplayOverride(pluginOverride, toolDefault ?? pluginDefault ?? presentation.defaultMode)
  }
  return toolDefault ?? resolveUiDisplayModeForPlugin(presentation, pluginId, pluginDefault)
}

function readPluginPromptPresentationSnapshot(
  effectiveRoot: Record<string, unknown>,
  fileRoot: Record<string, unknown>,
): SettingsPluginPromptPresentationSnapshot {
  const effectivePolicy = readRecord(effectiveRoot.policy)
  const filePolicy = readRecord(fileRoot.policy)
  const effectiveToolPresentation = readRecord(effectivePolicy.tool_presentation)
  const fileToolPresentation = readRecord(filePolicy.tool_presentation)
  return {
    defaultMode: readToolDescriptionMode(effectiveToolPresentation.default_mode, 'detailed'),
    fileDefaultMode: readOptionalToolDescriptionMode(fileToolPresentation.default_mode),
    effectivePluginOverrides: readOverrideMap(effectiveToolPresentation.plugins, readOptionalToolDescriptionOverride),
    effectiveToolOverrides: readOverrideMap(effectiveToolPresentation.tools, readOptionalToolDescriptionOverride),
    filePluginOverrides: readOverrideMap(fileToolPresentation.plugins, readOptionalToolDescriptionOverride),
    fileToolOverrides: readOverrideMap(fileToolPresentation.tools, readOptionalToolDescriptionOverride),
  }
}

function readPluginUiPresentationSnapshot(
  effectiveRoot: Record<string, unknown>,
  fileRoot: Record<string, unknown>,
): SettingsPluginUiPresentationSnapshot {
  const effectivePolicy = readRecord(effectiveRoot.policy)
  const filePolicy = readRecord(fileRoot.policy)
  const effectiveUiPresentation = readRecord(effectivePolicy.ui_presentation)
  const fileUiPresentation = readRecord(filePolicy.ui_presentation)
  return {
    defaultMode: readPluginUiDisplayMode(effectiveUiPresentation.default_mode, 'detailed'),
    fileDefaultMode: readOptionalPluginUiDisplayMode(fileUiPresentation.default_mode),
    effectivePluginOverrides: readOverrideMap(effectiveUiPresentation.plugins, readOptionalPluginUiDisplayOverride),
    effectiveToolOverrides: readOverrideMap(effectiveUiPresentation.tools, readOptionalPluginUiDisplayOverride),
    filePluginOverrides: readOverrideMap(fileUiPresentation.plugins, readOptionalPluginUiDisplayOverride),
    fileToolOverrides: readOverrideMap(fileUiPresentation.tools, readOptionalPluginUiDisplayOverride),
  }
}

function readManifestTools(
  pluginId: string,
  manifestRecord: Record<string, unknown>,
  promptPresentation: SettingsPluginPromptPresentationSnapshot,
  uiPresentation: SettingsPluginUiPresentationSnapshot,
): SettingsPluginToolSnapshot[] {
  const manifestPromptDefault = readOptionalToolDescriptionMode(manifestRecord.tool_description_mode)
  const manifestUiDefault = readOptionalPluginUiDisplayMode(manifestRecord.ui_display_mode)
  return readArray(manifestRecord.tools || manifestRecord.entries)
    .map((entry) => readRecord(entry))
    .map((entry) => {
      const tool = readManifestToolDefinition(entry)
      const toolName = tool.name
      const declaredPromptMode = tool.descriptionMode ?? manifestPromptDefault
      const declaredUiDisplayMode = tool.uiDisplayMode ?? manifestUiDefault
      return {
        toolName,
        toolKey: canonicalToolKey(pluginId, toolName),
        description: tool.description,
        summary: tool.summary,
        help: tool.help,
        tags: tool.tags,
        declaredPromptMode,
        declaredUiDisplayMode,
        filePromptOverride: findToolOverride(promptPresentation.fileToolOverrides, pluginId, toolName),
        effectivePromptOverride: findToolOverride(promptPresentation.effectiveToolOverrides, pluginId, toolName),
        effectivePromptMode: resolveToolDescriptionModeForTool(
          promptPresentation,
          pluginId,
          toolName,
          declaredPromptMode,
        ),
        fileUiDisplayOverride: findToolOverride(uiPresentation.fileToolOverrides, pluginId, toolName),
        effectiveUiDisplayOverride: findToolOverride(uiPresentation.effectiveToolOverrides, pluginId, toolName),
        effectiveUiDisplayMode: resolveUiDisplayModeForTool(
          uiPresentation,
          pluginId,
          toolName,
          declaredUiDisplayMode,
          manifestUiDefault,
        ),
      } satisfies SettingsPluginToolSnapshot
    })
}

function buildPluginSnapshot(
  pluginId: string,
  fileEntry: unknown,
  runtimeEntry: Record<string, unknown> | undefined,
  inspect: PluginInspect | null,
  promptPresentation: SettingsPluginPromptPresentationSnapshot,
  uiPresentation: SettingsPluginUiPresentationSnapshot,
): SettingsPluginEntrySnapshot {
  const entry = readPluginEntry(fileEntry ?? runtimeEntry ?? {})
  const manifestRecord = readRecord(inspect?.manifest)
  const tools = readManifestTools(pluginId, manifestRecord, promptPresentation, uiPresentation)
  const declaredUiDefault = readOptionalPluginUiDisplayMode(manifestRecord.ui_display_mode)
  const disabled = entry.disabled === true
  const kind = readString(entry.kind) || readString(inspect?.status.kind) || 'unknown'
  return {
    pluginId,
    displayName: readString(manifestRecord.name) || pluginId,
    kind,
    disabled,
    source: fileEntry ? 'file' : 'runtime',
    filePresent: fileEntry != null,
    manifestAvailable: Object.keys(manifestRecord).length > 0,
    entry,
    description:
      readString(manifestRecord.description) || readString(manifestRecord.summary) || 'Manifest unavailable.',
    summary: readString(manifestRecord.summary) || '',
    help: readString(manifestRecord.help) || '',
    declaredPromptDefault: readOptionalToolDescriptionMode(manifestRecord.tool_description_mode),
    effectivePromptMode: resolveToolDescriptionModeForPlugin(
      promptPresentation,
      pluginId,
      readOptionalToolDescriptionMode(manifestRecord.tool_description_mode),
    ),
    declaredUiDefault,
    filePromptOverride: promptPresentation.filePluginOverrides[pluginId] ?? null,
    effectivePromptOverride: promptPresentation.effectivePluginOverrides[pluginId] ?? null,
    fileUiDisplayOverride: uiPresentation.filePluginOverrides[pluginId] ?? null,
    effectiveUiDisplayOverride: uiPresentation.effectivePluginOverrides[pluginId] ?? null,
    effectiveUiDisplayMode: resolveUiDisplayModeForPlugin(uiPresentation, pluginId, declaredUiDefault),
    tools,
  }
}

function readSettingsPluginsConfig(
  effective: { config_path: string; config_found: boolean; value: unknown },
  file: { value: unknown },
  runtimePluginDetails: PromiseSettledResult<PluginInspect>[],
): SettingsPluginsConfigSnapshot {
  const effectiveRoot = readRecord(effective.value)
  const fileRoot = readRecord(file.value)
  const promptPresentation = readPluginPromptPresentationSnapshot(effectiveRoot, fileRoot)
  const uiPresentation = readPluginUiPresentationSnapshot(effectiveRoot, fileRoot)
  const filePluginEntries = readRecord(fileRoot.list)
  const runtimePluginEntries = new Map<string, Record<string, unknown>>()
  const runtimePluginInspect = new Map<string, PluginInspect>()

  for (const result of runtimePluginDetails) {
    if (result.status !== 'fulfilled') continue
    const pluginId = result.value.status.plugin_id.trim()
    if (!pluginId) continue
    runtimePluginEntries.set(pluginId, readPluginEntry(result.value.entry))
    runtimePluginInspect.set(pluginId, result.value)
  }

  const pluginIds = new Set<string>([...Object.keys(filePluginEntries), ...runtimePluginEntries.keys()])
  const plugins = Array.from(pluginIds)
    .sort((left, right) => left.localeCompare(right))
    .map((pluginId) =>
      buildPluginSnapshot(
        pluginId,
        filePluginEntries[pluginId],
        runtimePluginEntries.get(pluginId),
        runtimePluginInspect.get(pluginId) ?? null,
        promptPresentation,
        uiPresentation,
      ),
    )

  return {
    configPath: effective.config_path,
    configFound: effective.config_found,
    promptPresentation,
    uiPresentation,
    plugins,
  }
}
