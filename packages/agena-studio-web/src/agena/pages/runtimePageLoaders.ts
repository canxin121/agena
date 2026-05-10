import {
  fetchRuntimeStatus,
  listAuthProviders,
  listPermissionRules,
  listPluginLogs,
  listPlugins,
  listProviderModels,
  listProviders,
  listSessions,
  listWorkspaces,
  type AuthProvider,
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
  permissionRules: PermissionRuleResource[]
}

export type PluginsSectionData = {
  plugins: PluginStatus[]
  workspaces: WorkspaceResource[]
  selectedWorkspaceId: number | null
  selectedPluginId: string
}

export async function loadRuntimeSectionData(input: {
  selectedWorkspaceId: number | null
  selectedSessionId: number | null
}): Promise<RuntimeSectionData> {
  const [runtime, providers, workspaces] = await Promise.all([
    fetchRuntimeStatus(),
    listProviders(),
    listWorkspaces(),
  ])

  const providerModels = Object.fromEntries(
    await Promise.all(
      providers.map(async (provider) => [provider.provider_id, await listProviderModels(provider.provider_id)] as const),
    ),
  ) as Record<string, ProviderModel[]>

  const selectedWorkspaceId = pickWorkspaceId(input.selectedWorkspaceId, workspaces)
  const sessions = selectedWorkspaceId ? await listSessions(selectedWorkspaceId) : []
  const selectedSessionId = pickSessionId(input.selectedSessionId, sessions)

  return {
    runtime,
    providers,
    providerModels,
    workspaces,
    sessions,
    selectedWorkspaceId,
    selectedSessionId,
  }
}

export async function loadSettingsSectionData(permissionSearch: string): Promise<SettingsSectionData> {
  const [authProviders, permissionRules] = await Promise.all([
    listAuthProviders(),
    listPermissionRules(permissionSearch),
  ])

  return {
    authProviders,
    permissionRules,
  }
}

export async function loadPluginsSectionData(input: {
  selectedPluginId: string
  selectedWorkspaceId: number | null
}): Promise<PluginsSectionData> {
  const [plugins, workspaces] = await Promise.all([
    listPlugins(),
    listWorkspaces(),
  ])

  return {
    plugins,
    workspaces,
    selectedWorkspaceId: pickWorkspaceId(input.selectedWorkspaceId, workspaces),
    selectedPluginId: pickNextPluginId(input.selectedPluginId, plugins),
  }
}

export async function loadPluginLogsSnapshot(pluginId: string): Promise<PluginLogEntry[]> {
  return listPluginLogs(pluginId, { limit: 50 })
}
