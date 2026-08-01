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
import { ref, watch } from 'vue'

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
    permissionConfig: state.permissionConfig,
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
  const permissionScope = ref<'global' | 'session'>('global')

  function queryValue(value: unknown): string {
    if (Array.isArray(value)) return typeof value[0] === 'string' ? value[0] : ''
    return typeof value === 'string' ? value : ''
  }

  function syncPermissionRoute() {
    const permissionMode = queryValue(input.route.query.mode).toLowerCase()
    const hasSessionQuery = Boolean(queryValue(input.route.query.session) || queryValue(input.route.query.session_id))
    const normalizedPermissionScope =
      permissionMode === 'session' || permissionMode === 'current' || (!permissionMode && hasSessionQuery)
        ? 'session'
        : permissionMode === 'workspace' || permissionMode === 'project'
          ? 'workspace'
          : permissionMode === 'global' || permissionMode === 'config' || !permissionMode
            ? 'global'
            : null

    if (normalizedPermissionScope) {
      permissionScope.value = normalizedPermissionScope === 'session' ? 'session' : 'global'
      permissions.permissionScopeFilter.value = normalizedPermissionScope
      permissions.permissionDraft.scope = normalizedPermissionScope
    } else if (permissionMode === 'effective') {
      permissionScope.value = 'global'
      permissions.permissionScopeFilter.value = 'all'
      permissions.permissionStatusFilter.value = 'active'
    } else if (['list', 'rules', 'manage'].includes(permissionMode)) {
      permissionScope.value = 'global'
      permissions.permissionStatusFilter.value = 'active'
    } else if (permissionMode === 'new') {
      permissionScope.value = 'global'
      permissions.resetPermissionDraft()
    }
  }

  watch(() => [input.route.query.mode, input.route.query.session, input.route.query.session_id], syncPermissionRoute, {
    immediate: true,
  })

  const configuration = useSettingsConfigurationState({
    actionError: shared.actionError,
    actionMessage: shared.actionMessage,
  })
  watch(
    () => input.route.query.search,
    (value) => {
      configuration.search.value = queryValue(value).trim()
    },
    { immediate: true },
  )
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
    permissionScope,
    selectedSessionId: state.selectedSessionId,
    sessionExecution: state.sessionExecution,
    providers,
    tabs: shell.tabs,
  }
}
