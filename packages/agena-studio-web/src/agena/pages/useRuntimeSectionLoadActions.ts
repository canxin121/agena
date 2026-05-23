import type { Ref } from 'vue'

import type {
  AuthProvider,
  DomainEventRecord,
  ModelCatalogEntry,
  PermissionRuleResource,
  PluginInspect,
  PluginLogEntry,
  PluginStatus,
  ProviderModel,
  ProviderSummary,
  RuntimeStatus,
  SessionExecutionResource,
  SessionResource,
  WorkspaceResource,
} from '../lib/agenaApi'
import {
  loadPluginsSectionData,
  loadRuntimeSectionData,
  loadSettingsSectionData,
} from './runtimePageLoaders'
import type { SettingsPluginsConfigSnapshot } from './runtimePageLoaders'
import type { PluginsTab, SettingsTab } from './runtimePageStateModel'

export type RuntimeSectionLoadActionsInput = {
  actionError: Ref<string>
  activePluginsTab: Ref<PluginsTab>
  activeSettingsTab: Ref<SettingsTab>
  authProviders: Ref<AuthProvider[]>
  catalogEntries: Ref<ModelCatalogEntry[]>
  desktopEnabled: Ref<boolean>
  loadDesktopPanel: () => Promise<void>
  loadMarketplacePanel: () => Promise<void>
  loadPluginDetails: (pluginId: string) => Promise<void>
  loadSessionExecution: (sessionId: number) => Promise<void>
  loading: Ref<boolean>
  permissionRules: Ref<PermissionRuleResource[]>
  permissionSearch: Ref<string>
  pluginLogs: Ref<PluginLogEntry[]>
  plugins: Ref<PluginStatus[]>
  settingsPlugins: Ref<SettingsPluginsConfigSnapshot | null>
  providers: Ref<ProviderSummary[]>
  replaceProviderModels: (providerModels: Record<string, ProviderModel[]>) => void
  routeSection: Ref<'runtime' | 'settings' | 'plugins'>
  runtime: Ref<RuntimeStatus | null>
  selectedPlugin: Ref<PluginInspect | null>
  selectedPluginId: Ref<string>
  selectedSessionId: Ref<number | null>
  selectedWorkspaceId: Ref<number | null>
  sessionExecution: Ref<SessionExecutionResource | null>
  sessionTimeline: Ref<DomainEventRecord[]>
  sessions: Ref<SessionResource[]>
  stopPluginLogPolling: () => void
  workspaces: Ref<WorkspaceResource[]>
}

export type RuntimeSectionLoadActionsDeps = {
  loadPluginsSectionData: typeof loadPluginsSectionData
  loadRuntimeSectionData: typeof loadRuntimeSectionData
  loadSettingsSectionData: typeof loadSettingsSectionData
}

const defaultDeps: RuntimeSectionLoadActionsDeps = {
  loadPluginsSectionData,
  loadRuntimeSectionData,
  loadSettingsSectionData,
}

export function useRuntimeSectionLoadActions(
  input: RuntimeSectionLoadActionsInput,
  deps: RuntimeSectionLoadActionsDeps = defaultDeps,
) {
  async function loadRuntimeSection() {
    const data = await deps.loadRuntimeSectionData({
      selectedWorkspaceId: input.selectedWorkspaceId.value,
      selectedSessionId: input.selectedSessionId.value,
    })
    input.runtime.value = data.runtime
    input.catalogEntries.value = []
    input.providers.value = data.providers
    input.workspaces.value = data.workspaces
    input.replaceProviderModels(data.providerModels)
    input.sessions.value = data.sessions
    input.selectedWorkspaceId.value = data.selectedWorkspaceId
    input.selectedSessionId.value = data.selectedSessionId

    if (data.selectedSessionId != null) {
      await input.loadSessionExecution(data.selectedSessionId)
      return
    }
    input.sessionExecution.value = null
    input.sessionTimeline.value = []
  }

  async function loadSettingsSection() {
    const data = await deps.loadSettingsSectionData(input.permissionSearch.value)
    input.authProviders.value = data.authProviders
    input.runtime.value = data.runtime
    input.catalogEntries.value = []
    input.providers.value = data.providers
    input.replaceProviderModels(data.providerModels)
    input.permissionRules.value = data.permissionRules
    input.settingsPlugins.value = data.settingsPlugins

    if (input.activeSettingsTab.value === 'desktop' && input.desktopEnabled.value) {
      await input.loadDesktopPanel()
    }
  }

  async function loadPluginsSection() {
    const data = await deps.loadPluginsSectionData({
      selectedPluginId: input.selectedPluginId.value,
      selectedWorkspaceId: input.selectedWorkspaceId.value,
    })
    input.plugins.value = data.plugins
    input.workspaces.value = data.workspaces
    input.selectedWorkspaceId.value = data.selectedWorkspaceId
    input.selectedPluginId.value = data.selectedPluginId

    if (data.selectedPluginId) {
      await input.loadPluginDetails(data.selectedPluginId)
    } else {
      input.selectedPlugin.value = null
      input.pluginLogs.value = []
      input.stopPluginLogPolling()
    }

    if (input.activePluginsTab.value === 'marketplace') {
      await input.loadMarketplacePanel()
    }
  }

  async function load() {
    input.loading.value = true
    input.actionError.value = ''
    try {
      if (input.routeSection.value === 'runtime') {
        await loadRuntimeSection()
      } else if (input.routeSection.value === 'settings') {
        await loadSettingsSection()
      } else {
        await loadPluginsSection()
      }
    } catch (err) {
      input.actionError.value = err instanceof Error ? err.message : String(err)
    } finally {
      input.loading.value = false
    }
  }

  return {
    load,
    loadPluginsSection,
    loadRuntimeSection,
    loadSettingsSection,
  }
}
