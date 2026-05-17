import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import { useRuntimeSectionState } from './useRuntimeSectionState'
import { useSectionPanelRegistry } from './useSectionPanelRegistry'
import { createSettingsDesktopPanelState } from './useSettingsDesktopPageState'
import { createSettingsPermissionsPanelState } from './useSettingsPermissionsPageState'
import { createSettingsProvidersPanelState } from './useSettingsProvidersPageState'
import { createSettingsSectionShellState } from './useSettingsSectionShellState'

export function useSettingsPageState(input: { route: RouteLocationNormalizedLoaded; router: Router }) {
  const { shared, state } = useRuntimeSectionState({ ...input, section: 'settings' })

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

  const permissions = createSettingsPermissionsPanelState({
    permissionDraft: state.permissionDraft,
    editPermissionRule: state.editPermissionRule,
    editingPermissionRuleId: state.editingPermissionRuleId,
    filteredPermissionRules: state.filteredPermissionRules,
    permissionModeFilter: state.permissionModeFilter,
    permissionRuleFacts: state.permissionRuleFacts,
    permissionRuleLabel: state.permissionRuleLabel,
    permissionRulePreview: state.permissionRulePreview,
    permissionScopeFilter: state.permissionScopeFilter,
    permissionSearch: state.permissionSearch,
    permissionStatusFilter: state.permissionStatusFilter,
    permissionSubjectFilter: state.permissionSubjectFilter,
    savePermissionRule: state.savePermissionRule,
    resetPermissionDraft: state.resetPermissionDraft,
    revokePermissionRuleAction: state.revokePermissionRuleAction,
    deletePermissionRuleAction: state.deletePermissionRuleAction,
  })

  const desktop = createSettingsDesktopPanelState({
    backendErrorFacts: state.desktopBackendErrorFacts,
    backendUrl: state.desktopBackendUrl,
    config: state.desktopConfig,
    configFacts: state.desktopConfigFacts,
    enabled: state.desktopEnabled,
    form: state.desktopForm,
    installerAssetName: state.desktopInstallerAssetName,
    installerUpdateUrl: state.desktopInstallerUpdateUrl,
    notice: state.desktopNotice,
    runtimeFacts: state.desktopRuntimeFacts,
    saving: state.desktopSaving,
    serviceUpdateUrl: state.desktopServiceUpdateUrl,
    statusFacts: state.desktopStatusFacts,
    updateFacts: state.desktopUpdateFacts,
    updateProgressPercent: state.desktopUpdateProgressPercent,
    updateRunning: state.desktopUpdateRunning,
    loadPanel: state.loadDesktopPanel,
    openBackendUrlAction: state.openDesktopBackendUrlAction,
    openConfigAction: state.openDesktopConfigAction,
    refreshUpdateProgressAction: state.refreshDesktopUpdateProgressAction,
    restartBackendAction: state.restartDesktopBackendAction,
    runInstallerUpdateAction: state.runDesktopInstallerUpdateAction,
    runServiceUpdateAction: state.runDesktopServiceUpdateAction,
    saveConfigAction: state.saveDesktopConfigAction,
  })

  const shell = createSettingsSectionShellState({
    activeSettingsTab: state.activeSettingsTab,
    visibleTabs: state.visibleTabs,
  })

  const panelRegistry = useSectionPanelRegistry({
    activeTab: shell.activeTab,
    panels: {
      providers,
      permissions,
      desktop,
    },
  })

  return {
    activeSettingsTab: shell.activeTab,
    actionError: shared.actionError,
    actionMessage: shared.actionMessage,
    currentPanel: panelRegistry.currentPanel,
    desktop,
    load: shared.load,
    loading: shared.loading,
    pageDescription: shared.pageDescription,
    pageTitle: shared.pageTitle,
    panels: panelRegistry.panels,
    permissions,
    providers,
    tabs: shell.tabs,
  }
}
