import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import { createSettingsPluginsPanelState } from './useSettingsPluginsPageState'
import { useRuntimePageState } from './useRuntimePageState'
import { useRuntimeSectionState } from './useRuntimeSectionState'
import { useSectionPanelRegistry } from './useSectionPanelRegistry'
import { createSettingsPermissionsPanelState } from './useSettingsPermissionsPageState'
import { createSettingsProvidersPanelState } from './useSettingsProvidersPageState'
import { createSettingsSectionShellState } from './useSettingsSectionShellState'
import { useSettingsConfigurationState } from './useSettingsConfigurationState'
import { useSettingsMemoryState } from './useSettingsMemoryState'

export function useSettingsPageState(input: { route: RouteLocationNormalizedLoaded; router: Router }) {
  const { shared, state } = useRuntimeSectionState<ReturnType<typeof useRuntimePageState>>({
    ...input,
    section: 'settings',
  })

  const providers = createSettingsProvidersPanelState({
    actionError: shared.actionError,
    actionMessage: shared.actionMessage,
    authProviders: state.authProviders,
    browserAuthCodeDrafts: state.browserAuthCodeDrafts,
    browserAuthInstanceDrafts: state.browserAuthInstanceDrafts,
    browserAuthStartState: state.browserAuthStartState,
    catalogEntries: state.catalogEntries,
    deviceAuthEnterpriseDrafts: state.deviceAuthEnterpriseDrafts,
    deviceAuthStartState: state.deviceAuthStartState,
    drafts: state.drafts,
    load: shared.load,
    providerModels: state.providerModels,
    providers: state.providers,
    finishBrowserAuth: state.finishBrowserAuth,
    pollDeviceAuth: state.pollDeviceAuth,
    saveApiKey: state.saveApiKey,
    refreshCredential: state.refreshCredential,
    clearCredential: state.clearCredential,
    startBrowserAuth: state.startBrowserAuth,
    startDeviceAuth: state.startDeviceAuth,
  })

  const permissions = createSettingsPermissionsPanelState(
    {
      permissionConfig: state.permissionConfig,
      editingPermissionRuleId: state.editingPermissionRuleId,
      filteredPermissionRules: state.filteredPermissionRules,
      permissionDraft: state.permissionDraft,
      permissionModeFilter: state.permissionModeFilter,
      permissionRuleFacts: state.permissionRuleFacts,
      permissionRuleLabel: state.permissionRuleLabel,
      permissionRulePreview: state.permissionRulePreview,
      permissionScopeFilter: state.permissionScopeFilter,
      permissionSearch: state.permissionSearch,
      permissionStatusFilter: state.permissionStatusFilter,
      permissionSubjectFilter: state.permissionSubjectFilter,
      deletePermissionRuleAction: state.deletePermissionRuleAction,
      editPermissionRule: state.editPermissionRule,
      resetPermissionDraft: state.resetPermissionDraft,
      revokePermissionRuleAction: state.revokePermissionRuleAction,
      savePermissionRule: state.savePermissionRule,
    },
    shared,
  )
  const permissionMode = typeof input.route.query.mode === 'string' ? input.route.query.mode.toLowerCase() : ''
  const normalizedPermissionScope =
    permissionMode === 'session' || permissionMode === 'current'
      ? 'session'
      : permissionMode === 'workspace' || permissionMode === 'project'
        ? 'workspace'
        : permissionMode === 'global' || permissionMode === 'config'
          ? 'global'
          : null
  if (normalizedPermissionScope) {
    permissions.permissionScopeFilter.value = normalizedPermissionScope
    permissions.permissionDraft.scope = normalizedPermissionScope
  } else if (permissionMode === 'effective') {
    permissions.permissionScopeFilter.value = 'all'
    permissions.permissionStatusFilter.value = 'active'
  } else if (['list', 'rules', 'manage'].includes(permissionMode)) {
    permissions.permissionStatusFilter.value = 'active'
  } else if (permissionMode === 'new') {
    permissions.resetPermissionDraft()
  }

  const configuration = useSettingsConfigurationState({
    actionError: shared.actionError,
    actionMessage: shared.actionMessage,
  })
  configuration.search.value = typeof input.route.query.search === 'string' ? input.route.query.search.trim() : ''
  const memory = useSettingsMemoryState({
    actionError: shared.actionError,
    actionMessage: shared.actionMessage,
  })

  const plugins = createSettingsPluginsPanelState({
    actionError: shared.actionError,
    actionMessage: shared.actionMessage,
    load: shared.load,
    settingsPlugins: state.settingsPlugins,
  })

  const shell = createSettingsSectionShellState({
    activeSettingsTab: state.activeSettingsTab,
    visibleTabs: state.visibleTabs,
  })

  const panelRegistry = useSectionPanelRegistry({
    activeTab: shell.activeTab,
    panels: {
      providers,
      configuration,
      memory,
      plugins,
      permissions,
    },
  })

  return {
    activeSettingsTab: shell.activeTab,
    actionError: shared.actionError,
    actionMessage: shared.actionMessage,
    currentPanel: panelRegistry.currentPanel,
    load: shared.load,
    loading: shared.loading,
    pageDescription: shared.pageDescription,
    pageTitle: shared.pageTitle,
    panels: panelRegistry.panels,
    configuration,
    memory,
    plugins,
    permissions,
    providers,
    tabs: shell.tabs,
  }
}
