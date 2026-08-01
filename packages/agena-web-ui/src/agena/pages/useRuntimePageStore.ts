import { reactive, ref } from 'vue'

import type {
  AuthProvider,
  AuthBrowserStartResponse,
  AuthDeviceStartResponse,
  DomainEventRecord,
  ConfigSettingsReadResponse,
  MarketplaceInstalledPluginResource,
  MarketplaceOutdatedPluginResource,
  MarketplacePluginResource,
  ModelCatalogEntry,
  PermissionMode,
  PermissionRuleResource,
  PermissionSubjectKind,
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
import type { SettingsPluginUiPresentationSnapshot, SettingsPluginsConfigSnapshot } from './runtimePageLoaders'
import type { PluginsTab, RuntimeTab, SettingsTab } from './runtimePageStateModel'

export function useRuntimePageStore() {
  const activeTab = ref<RuntimeTab>('overview')
  const activeSettingsTab = ref<SettingsTab>('providers')
  const activePluginsTab = ref<PluginsTab>('installed')
  const runtime = ref<RuntimeStatus | null>(null)
  const providers = ref<ProviderSummary[]>([])
  const providerModels = reactive<Record<string, ProviderModel[]>>({})
  const authProviders = ref<AuthProvider[]>([])
  const permissionConfig = ref<ConfigSettingsReadResponse | null>(null)
  const permissionRules = ref<PermissionRuleResource[]>([])
  const plugins = ref<PluginStatus[]>([])
  const settingsPlugins = ref<SettingsPluginsConfigSnapshot | null>(null)
  const pluginUiPresentation = ref<SettingsPluginUiPresentationSnapshot | null>(null)
  const workspaces = ref<WorkspaceResource[]>([])
  const sessions = ref<SessionResource[]>([])
  const selectedWorkspaceId = ref<number | null>(null)
  const selectedSessionId = ref<number | null>(null)
  const sessionExecution = ref<SessionExecutionResource | null>(null)
  const sessionTimeline = ref<DomainEventRecord[]>([])
  const globalEvents = ref<DomainEventRecord[]>([])
  const selectedPluginId = ref('')
  const selectedPlugin = ref<PluginInspect | null>(null)
  const pluginLogs = ref<PluginLogEntry[]>([])
  const pluginLogPollTimer = ref<ReturnType<typeof setInterval> | null>(null)
  const loading = ref(false)
  const pluginLoading = ref(false)
  const workflowLoading = ref(false)
  const actionError = ref('')
  const actionMessage = ref('')
  const interactiveRequestInFlight = reactive<Record<string, boolean>>({})
  const drafts = reactive<Record<string, string>>({})
  const browserAuthCodeDrafts = reactive<Record<string, string>>({})
  const browserAuthInstanceDrafts = reactive<Record<string, string>>({})
  const browserAuthStartState = reactive<Record<string, AuthBrowserStartResponse | null>>({})
  const deviceAuthEnterpriseDrafts = reactive<Record<string, string>>({})
  const deviceAuthStartState = reactive<Record<string, AuthDeviceStartResponse | null>>({})
  const permissionSearch = ref('')
  const permissionModeFilter = ref<'all' | PermissionMode>('all')
  const permissionScopeFilter = ref<'all' | 'session' | 'workspace' | 'global'>('all')
  const permissionSubjectFilter = ref<'all' | PermissionSubjectKind>('all')
  const permissionStatusFilter = ref<'all' | 'active' | 'revoked'>('active')
  const marketplaceQuery = ref('')
  const runtimeSkillQuery = ref('')
  const mcpQuery = ref('')
  const lspQuery = ref('')
  const catalogEntries = ref<ModelCatalogEntry[]>([])
  const marketplaceRegistryUrl = ref('')
  const marketplaceRegistryId = ref('default')
  const marketplaceInstallSpec = ref('')
  const marketplaceAllowUnverified = ref(false)
  const marketplaceRequireSignature = ref(false)
  const marketplaceRefreshIndex = ref(false)
  const marketplaceCascadeUninstall = ref(false)
  const marketplaceLoading = ref(false)
  const marketplacePlugins = ref<MarketplacePluginResource[]>([])
  const marketplaceInstalled = ref<MarketplaceInstalledPluginResource[]>([])
  const marketplaceOutdated = ref<MarketplaceOutdatedPluginResource[]>([])
  const permissionDraft = reactive<{
    subjectKind: PermissionSubjectKind
    toolName: string
    qualifier: string
    pathAccessKind: string
    workspaceRoot: string
    targetPath: string
    networkTarget: string
    networkPort: string
    scope: 'session' | 'workspace' | 'global'
    sessionId: string
    mode: PermissionMode
  }>({
    subjectKind: 'tool',
    toolName: '',
    qualifier: '',
    pathAccessKind: 'read',
    workspaceRoot: '',
    targetPath: '',
    networkTarget: '',
    networkPort: '',
    scope: 'workspace',
    sessionId: '',
    mode: 'auto',
  })
  const editingPermissionRuleId = ref<number | null>(null)
  return {
    activePluginsTab,
    activeSettingsTab,
    activeTab,
    actionError,
    actionMessage,
    authProviders,
    browserAuthCodeDrafts,
    browserAuthInstanceDrafts,
    browserAuthStartState,
    catalogEntries,
    deviceAuthEnterpriseDrafts,
    deviceAuthStartState,
    drafts,
    editingPermissionRuleId,
    globalEvents,
    interactiveRequestInFlight,
    loading,
    lspQuery,
    marketplaceAllowUnverified,
    marketplaceCascadeUninstall,
    marketplaceInstallSpec,
    marketplaceInstalled,
    marketplaceLoading,
    marketplaceOutdated,
    marketplacePlugins,
    marketplaceQuery,
    marketplaceRefreshIndex,
    marketplaceRegistryId,
    marketplaceRegistryUrl,
    marketplaceRequireSignature,
    mcpQuery,
    permissionConfig,
    permissionDraft,
    permissionModeFilter,
    permissionRules,
    permissionScopeFilter,
    permissionSearch,
    permissionStatusFilter,
    permissionSubjectFilter,
    pluginLoading,
    pluginLogPollTimer,
    pluginLogs,
    plugins,
    pluginUiPresentation,
    settingsPlugins,
    providerModels,
    providers,
    runtime,
    runtimeSkillQuery,
    selectedPlugin,
    selectedPluginId,
    selectedSessionId,
    selectedWorkspaceId,
    sessionExecution,
    sessions,
    sessionTimeline,
    workflowLoading,
    workspaces,
  }
}
