import type { ComputedRef, Ref } from 'vue'
import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'

import type { ProviderModel } from '../lib/agenaApi'
import { createRuntimeMarketplaceActions } from './useRuntimeMarketplaceActions'
import { useRuntimeDesktopActions } from './useRuntimeDesktopActions'
import { useRuntimeNavigationState } from './useRuntimeNavigationState'
import { useRuntimePermissionActions } from './useRuntimePermissionActions'
import { useRuntimePluginDetails } from './useRuntimePluginDetails'
import { useRuntimeProviderActions } from './useRuntimeProviderActions'
import { useRuntimeRouteLifecycle } from './useRuntimeRouteLifecycle'
import { useRuntimeRouteState } from './useRuntimeRouteState'
import { useRuntimeSectionLoadActions } from './useRuntimeSectionLoadActions'
import { useRuntimeSessionWorkflowActions } from './useRuntimeSessionWorkflowActions'
import type { RuntimeRouteSection } from './runtimePageStateModel'
import type { ReturnTypeOfUseRuntimePageStore } from './useRuntimePageStore.types'

export type RuntimePageAssemblyInput = {
  routePath: Ref<string>
  routeQuery: RouteLocationNormalizedLoaded['query']
  router: Pick<Router, 'push' | 'replace'>
  routeSection: Ref<RuntimeRouteSection>
  desktopBackendUrl: ComputedRef<string>
  desktopEnabled: ComputedRef<boolean>
  selectedPluginManifest: Ref<Record<string, unknown> | null>
} & ReturnTypeOfUseRuntimePageStore

export function replaceProviderModelsRecord(providerModels: Record<string, ProviderModel[]>, nextProviderModels: Record<string, ProviderModel[]>) {
  for (const key of Object.keys(providerModels)) {
    delete providerModels[key]
  }
  Object.assign(providerModels, nextProviderModels)
}

export function useRuntimePageAssembly(input: RuntimePageAssemblyInput) {
  let load = async () => {}
  const loadPageState = async () => {
    await load()
  }

  const navigation = useRuntimeNavigationState(
    {
      selectedSessionId: input.selectedSessionId,
      selectedWorkspaceId: input.selectedWorkspaceId,
      selectedPluginManifest: input.selectedPluginManifest,
      workspaces: input.workspaces,
    },
    { router: input.router },
  )

  const desktopActions = useRuntimeDesktopActions({
    actionError: input.actionError,
    actionMessage: input.actionMessage,
    desktopBackendUrl: input.desktopBackendUrl,
    desktopConfig: input.desktopConfig,
    desktopEnabled: input.desktopEnabled,
    desktopForm: input.desktopForm,
    desktopInstallerAssetName: input.desktopInstallerAssetName,
    desktopInstallerUpdateUrl: input.desktopInstallerUpdateUrl,
    desktopNotice: input.desktopNotice,
    desktopRuntimeState: input.desktopRuntimeState,
    desktopSaving: input.desktopSaving,
    desktopServiceUpdateUrl: input.desktopServiceUpdateUrl,
    desktopStatus: input.desktopStatus,
    desktopUpdate: input.desktopUpdate,
    desktopUpdateRunning: input.desktopUpdateRunning,
  })

  const marketplaceActions = createRuntimeMarketplaceActions(
    {
      actionError: input.actionError,
      actionMessage: input.actionMessage,
      marketplaceAllowUnverified: input.marketplaceAllowUnverified,
      marketplaceCascadeUninstall: input.marketplaceCascadeUninstall,
      marketplaceInstallSpec: input.marketplaceInstallSpec,
      marketplaceInstalled: input.marketplaceInstalled,
      marketplaceLoading: input.marketplaceLoading,
      marketplaceOutdated: input.marketplaceOutdated,
      marketplacePlugins: input.marketplacePlugins,
      marketplaceQuery: input.marketplaceQuery,
      marketplaceRefreshIndex: input.marketplaceRefreshIndex,
      marketplaceRegistryId: input.marketplaceRegistryId,
      marketplaceRegistryUrl: input.marketplaceRegistryUrl,
      marketplaceRequireSignature: input.marketplaceRequireSignature,
    },
    loadPageState,
  )

  const pluginDetails = useRuntimePluginDetails({
    actionError: input.actionError,
    activePluginsTab: input.activePluginsTab,
    pluginLoading: input.pluginLoading,
    pluginLogs: input.pluginLogs,
    pluginLogPollTimer: input.pluginLogPollTimer,
    routeSection: input.routeSection,
    selectedPlugin: input.selectedPlugin,
    selectedPluginId: input.selectedPluginId,
  })

  const providerActions = useRuntimeProviderActions({
      actionError: input.actionError,
      actionMessage: input.actionMessage,
      browserAuthCodeDrafts: input.browserAuthCodeDrafts,
      browserAuthInstanceDrafts: input.browserAuthInstanceDrafts,
      browserAuthStartState: input.browserAuthStartState,
      deviceAuthEnterpriseDrafts: input.deviceAuthEnterpriseDrafts,
      deviceAuthStartState: input.deviceAuthStartState,
      drafts: input.drafts,
      load: loadPageState,
      openUrl: (url) => {
        if (typeof window !== 'undefined') {
          window.open(url, '_blank', 'noopener,noreferrer')
        }
      },
      readRedirectUri: () => {
        if (typeof window === 'undefined') return ''
        return `${window.location.origin}/auth/callback`
      },
    })

  const sessionWorkflowActions = useRuntimeSessionWorkflowActions({
    actionError: input.actionError,
    actionMessage: input.actionMessage,
    load: loadPageState,
    globalEvents: input.globalEvents,
    selectedSessionId: input.selectedSessionId,
    selectedWorkspaceId: input.selectedWorkspaceId,
    sessionExecution: input.sessionExecution,
    sessionTimeline: input.sessionTimeline,
    sessions: input.sessions,
    workflowLoading: input.workflowLoading,
  })

  const { loadSessionExecution } = sessionWorkflowActions

  const permissionActions = useRuntimePermissionActions({
    actionError: input.actionError,
    actionMessage: input.actionMessage,
    activeSettingsTab: input.activeSettingsTab,
    editingPermissionRuleId: input.editingPermissionRuleId,
    load: loadPageState,
    loadSessionExecution,
    permissionDraft: input.permissionDraft,
    selectedSessionId: input.selectedSessionId,
    sessionExecution: input.sessionExecution,
  })

  const routeState = useRuntimeRouteState(
    {
      activePluginsTab: input.activePluginsTab,
      activeSettingsTab: input.activeSettingsTab,
      activeTab: input.activeTab,
      routePath: input.routePath,
      routeQuery: input.routeQuery,
      routeSection: input.routeSection,
    },
    { router: input.router },
  )

  const { loadDesktopPanel } = desktopActions
  const { loadMarketplacePanel } = marketplaceActions
  const { loadPluginDetails, stopPluginLogPolling, syncPluginLogPolling } = pluginDetails

  const sectionLoadActions = useRuntimeSectionLoadActions({
    actionError: input.actionError,
    activePluginsTab: input.activePluginsTab,
    activeSettingsTab: input.activeSettingsTab,
    authProviders: input.authProviders,
    desktopEnabled: input.desktopEnabled,
    loadDesktopPanel,
    loadMarketplacePanel,
    loadPluginDetails,
    loadSessionExecution,
    loading: input.loading,
    permissionRules: input.permissionRules,
    permissionSearch: input.permissionSearch,
    pluginLogs: input.pluginLogs,
    plugins: input.plugins,
    providers: input.providers,
    replaceProviderModels: (nextProviderModels) => replaceProviderModelsRecord(input.providerModels, nextProviderModels),
    routeSection: input.routeSection,
    runtime: input.runtime,
    selectedPlugin: input.selectedPlugin,
    selectedPluginId: input.selectedPluginId,
    selectedSessionId: input.selectedSessionId,
    selectedWorkspaceId: input.selectedWorkspaceId,
    sessionExecution: input.sessionExecution,
    sessionTimeline: input.sessionTimeline,
    sessions: input.sessions,
    stopPluginLogPolling,
    workspaces: input.workspaces,
  })

  load = sectionLoadActions.load

  useRuntimeRouteLifecycle({
    activePluginsTab: input.activePluginsTab,
    activeSettingsTab: input.activeSettingsTab,
    activeTab: input.activeTab,
    desktopEnabled: input.desktopEnabled,
    desktopUpdate: input.desktopUpdate,
    desktopUpdateRunning: input.desktopUpdateRunning,
    load,
    loadDesktopPanel,
    loadMarketplacePanel,
    routePath: input.routePath,
    routeSection: input.routeSection,
    stopPluginLogPolling,
    syncPluginLogPolling,
    syncTabsFromRoute: routeState.syncTabsFromRoute,
    updateRoutePath: routeState.updateRoutePath,
  })

  return {
    desktopActions,
    loadPageState,
    marketplaceActions,
    navigation,
    permissionActions,
    pluginDetails,
    providerActions,
    routeState,
    sectionLoadActions,
    sessionWorkflowActions,
  }
}
