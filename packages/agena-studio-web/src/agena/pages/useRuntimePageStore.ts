import { reactive, ref } from 'vue'

import type {
  AuthProvider,
  AuthBrowserStartResponse,
  AuthDeviceStartResponse,
  GlobalEventRecord,
  MarketplaceInstalledPluginResource,
  MarketplaceOutdatedPluginResource,
  MarketplacePluginResource,
  PermissionMode,
  PermissionRuleResource,
  PluginInspect,
  PluginLogEntry,
  PluginStatus,
  ProviderModel,
  ProviderSummary,
  RuntimeStatus,
  SessionExecutionResource,
  SessionResource,
  TimelineEventRecord,
  WorkspaceResource,
} from '../lib/agenaApi'
import type {
  DesktopBackendStatus,
  DesktopConfig,
  DesktopRuntimeInfo,
  DesktopUpdateProgress,
} from '../../lib/desktopConfig'
import type { PluginsTab, RuntimeTab, SettingsTab } from './runtimePageStateModel'

export function useRuntimePageStore() {
  const activeTab = ref<RuntimeTab>('overview')
  const activeSettingsTab = ref<SettingsTab>('providers')
  const activePluginsTab = ref<PluginsTab>('installed')
  const runtime = ref<RuntimeStatus | null>(null)
  const providers = ref<ProviderSummary[]>([])
  const providerModels = reactive<Record<string, ProviderModel[]>>({})
  const authProviders = ref<AuthProvider[]>([])
  const permissionRules = ref<PermissionRuleResource[]>([])
  const plugins = ref<PluginStatus[]>([])
  const workspaces = ref<WorkspaceResource[]>([])
  const sessions = ref<SessionResource[]>([])
  const selectedWorkspaceId = ref<number | null>(null)
  const selectedSessionId = ref<number | null>(null)
  const sessionExecution = ref<SessionExecutionResource | null>(null)
  const sessionTimeline = ref<TimelineEventRecord[]>([])
  const globalEvents = ref<GlobalEventRecord[]>([])
  const selectedPluginId = ref('')
  const selectedPlugin = ref<PluginInspect | null>(null)
  const pluginLogs = ref<PluginLogEntry[]>([])
  const pluginLogPollTimer = ref<ReturnType<typeof setInterval> | null>(null)
  const loading = ref(false)
  const pluginLoading = ref(false)
  const workflowLoading = ref(false)
  const desktopSaving = ref(false)
  const actionError = ref('')
  const actionMessage = ref('')
  const desktopNotice = ref('')
  const drafts = reactive<Record<string, string>>({})
  const browserAuthCodeDrafts = reactive<Record<string, string>>({})
  const browserAuthInstanceDrafts = reactive<Record<string, string>>({})
  const browserAuthStartState = reactive<Record<string, AuthBrowserStartResponse | null>>({})
  const deviceAuthEnterpriseDrafts = reactive<Record<string, string>>({})
  const deviceAuthStartState = reactive<Record<string, AuthDeviceStartResponse | null>>({})
  const permissionSearch = ref('')
  const permissionModeFilter = ref<'all' | PermissionMode>('all')
  const permissionScopeFilter = ref<'all' | 'session' | 'workspace' | 'global'>('all')
  const permissionSubjectFilter = ref<'all' | 'builtin_tool' | 'path_access'>('all')
  const permissionStatusFilter = ref<'all' | 'active' | 'revoked'>('active')
  const marketplaceQuery = ref('')
  const runtimeSkillQuery = ref('')
  const mcpQuery = ref('')
  const lspQuery = ref('')
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
    subjectKind: 'builtin_tool' | 'path_access'
    toolName: string
    qualifier: string
    pathAccessKind: string
    workspaceRoot: string
    targetPath: string
    scope: 'session' | 'workspace' | 'global'
    sessionId: string
    mode: PermissionMode
  }>({
    subjectKind: 'builtin_tool',
    toolName: '',
    qualifier: '',
    pathAccessKind: 'read',
    workspaceRoot: '',
    targetPath: '',
    scope: 'workspace',
    sessionId: '',
    mode: 'ask',
  })
  const editingPermissionRuleId = ref<number | null>(null)
  const desktopConfig = ref<DesktopConfig | null>(null)
  const desktopStatus = ref<DesktopBackendStatus | null>(null)
  const desktopRuntimeState = ref<DesktopRuntimeInfo | null>(null)
  const desktopUpdate = ref<DesktopUpdateProgress | null>(null)
  const desktopServiceUpdateUrl = ref('')
  const desktopInstallerUpdateUrl = ref('')
  const desktopInstallerAssetName = ref('')
  const desktopUpdateRunning = ref(false)
  const desktopForm = reactive({
    autostart_on_boot: false,
    host: '',
    port: '',
    workspace_root: '',
    agena_config_path: '',
    database_path: '',
    database_url: '',
    backend_log_level: '',
    ui_cookie_samesite: '',
  })

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
    desktopConfig,
    desktopForm,
    desktopInstallerAssetName,
    desktopInstallerUpdateUrl,
    desktopNotice,
    desktopRuntimeState,
    desktopSaving,
    desktopServiceUpdateUrl,
    desktopStatus,
    desktopUpdate,
    desktopUpdateRunning,
    deviceAuthEnterpriseDrafts,
    deviceAuthStartState,
    drafts,
    editingPermissionRuleId,
    globalEvents,
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
