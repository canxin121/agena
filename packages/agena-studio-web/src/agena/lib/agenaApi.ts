import { emitAuthRequired, extractAuthRequiredMessageFromBodyText } from '../../lib/authEvents'
import { apiJson, apiText, apiUrl } from '../../lib/api'
import { buildActiveUiAuthHeaders } from '../../lib/uiAuthToken'
import { normalizeSseBuffer, parseSseEventBlock } from './sse'
import type { ProviderModelPricing, ProviderModelThinkingMode, ProviderModelSpeedMode } from './providerApi'

export * from './providerApi'

export type StudioHealth = {
  status: string
  generation: number
  loadedAt: string
  workspaceRoot: string
  configPath: string
  configFound: boolean
  activeMode?: string | null
  providerIds: string[]
  sessionRuntimeAvailable: boolean
}

export type RuntimeSkill = {
  name: string
  description: string
  aliases: string[]
  source_path?: string | null
}

export type ScheduledJobRunResource = {
  triggered_at: string
  finished_at: string
  status: 'submitted' | 'skipped' | 'failed' | string
  session_id?: number | null
  error_message?: string | null
}

export type ScheduledJobResource = {
  id: string
  kind: 'cron' | 'once' | string
  expression?: string | null
  at?: string | null
  prompt: string
  owner_session_id?: number | null
  next_fire_at?: string | null
  last_fired_at?: string | null
  last_run?: ScheduledJobRunResource | null
}

export type SessionAutomationResource = {
  job_count: number
  latest_job?: ScheduledJobResource | null
}

export type RuntimeAutomationResource = {
  enabled: boolean
  job_count: number
  recent_jobs: ScheduledJobResource[]
}

export type RuntimeStatus = {
  generation: number
  loaded_at: string
  workspace_root: string
  config_path: string
  config_found: boolean
  auth_store_path: string
  provider_ids: string[]
  plugin_count: number
  session_runtime_available: boolean
  watch_paths: string[]
  reload: {
    enabled: boolean
    interval_secs: number
  }
  session_gc: {
    enabled: boolean
    interval_secs: number
  }
  session_cache?: {
    max_sessions: number
    ttl_secs: number
    max_bytes: number
    entry_count: number
    total_bytes: number
    hits: number
    misses: number
    inserts: number
    evictions: number
  } | null
  model_catalog?: ModelCatalogSummary | null
  background_tasks: RuntimeBackgroundTask[]
  automation: RuntimeAutomationResource
  operator: {
    mcp: {
      server_count: number
      tool_count: number
      servers: Array<{
        name: string
        tool_count: number
      }>
    }
    lsp: {
      server_count: number
      diagnostics_count: number
      files_with_diagnostics: number
      servers: Array<{
        name: string
        command: string
        file_extensions: string[]
        root_markers: string[]
      }>
    }
    agents: {
      default_agent: string
      total_count: number
      agents: Array<{
        name: string
        description: string
        permission?: AgentPermissionConfig
        defaults?: {
          provider?: string | null
          adapter?: string | null
          model?: string | null
          thinking_mode?: string | null
          speed_mode?: string | null
          verbosity?: string | null
          parallel_tool_calls?: boolean | null
        }
        scope: 'project' | 'user' | 'bundled'
        source_path?: string | null
      }>
    }
    skills: {
      skill_count: number
      command_count: number
      skills: RuntimeSkill[]
      commands: RuntimeSkill[]
    }
    ui?: PluginUiCatalogSnapshot
  }
}

export type RuntimeBackgroundTaskKind =
  | 'model_catalog_refresh'
  | 'runtime_reload'
  | 'marketplace_registry_sync'
  | 'marketplace_plugin_install'
  | 'marketplace_plugin_uninstall'
  | 'marketplace_plugin_upgrade'

export type RuntimeBackgroundTaskOrigin = 'system' | 'user'

export type RuntimeBackgroundTaskStatus = 'running' | 'succeeded' | 'failed' | 'cancelled'

export type RuntimeBackgroundTask = {
  id: string
  kind: RuntimeBackgroundTaskKind
  origin: RuntimeBackgroundTaskOrigin
  title: string
  status: RuntimeBackgroundTaskStatus
  message?: string | null
  error_message?: string | null
  created_at: string
  started_at: string
  finished_at?: string | null
  cancellable: boolean
}

export type RuntimeBackgroundTaskListResponse = {
  items: RuntimeBackgroundTask[]
}

export type RuntimeBackgroundTaskCancelResponse = {
  task: RuntimeBackgroundTask
}

export type RuntimeBackgroundTaskStartResponse = {
  started: boolean
  task: RuntimeBackgroundTask
}

export type PluginStatus = {
  plugin_id: string
  kind: string
  state: string
  pid?: number | null
  restart_count: number
  last_exit_code?: number | null
  last_restart_at_ms?: number | null
  last_error?: string | null
}

export type PluginAuthoritySummary = {
  trust_level: string
  provenance: string[]
  plugin_capabilities: string[]
  entry_capabilities: Record<string, string[]>
}

export type PluginInspect = {
  status: PluginStatus
  manifest?: Record<string, unknown> | null
  authority?: PluginAuthoritySummary | null
  entry?: Record<string, unknown> | null
}

export type PluginUiAction =
  | { kind: 'none' }
  | { kind: 'invoke_tool'; tool: string; input?: Record<string, unknown> | null; submit_output_as_prompt?: boolean }
  | { kind: 'open_route'; route: string }
  | { kind: 'open_url'; url: string }
  | { kind: 'submit_prompt'; prompt: string }

export type PluginTuiStatuslineSegment = {
  plugin_id: string
  segment_id: string
  content: string
  priority: number
  color?: string | null
}

export type PluginUiThemePalette = {
  plugin_id: string
  id: string
  display_name: string
  colors: Record<string, string>
}

export type PluginTuiContentBlock = {
  plugin_id: string
  id: string
  title: string
  body?: string
  location: string
  priority: number
  color?: string | null
}

export type PluginStudioCommand = {
  plugin_id: string
  id: string
  title: string
  description?: string
  category: string
  slash?: string | null
  aliases?: string[]
  usage?: string | null
  location: string
  action: PluginUiAction
}

export type PluginStudioControlOption = {
  label: string
  value: string
  description?: string
}

export type PluginStudioControl = {
  plugin_id: string
  id: string
  title: string
  description?: string
  location: string
  kind: string
  options?: PluginStudioControlOption[]
  value?: unknown
  action: PluginUiAction
}

export type PluginStudioView = {
  plugin_id: string
  id: string
  title: string
  description?: string
  location: string
  kind: string
  content?: string | null
  url?: string | null
  controls?: PluginStudioControl[]
}

export type PluginUiCatalog = {
  tui: {
    statusline_segments?: PluginTuiStatuslineSegment[]
    themes?: PluginUiThemePalette[]
    content_blocks?: PluginTuiContentBlock[]
  }
  studio: {
    commands?: PluginStudioCommand[]
    controls?: PluginStudioControl[]
    views?: PluginStudioView[]
  }
}

export type ToolRegistryChangeKind = 'registered' | 'updated' | 'removed'

export type ToolRegistryChangedEvent = {
  kind: ToolRegistryChangeKind
  generation: number
  timestamp_ms: number
  plugin_id: string
  original_name: string
  exposed_name: string
  tool?: Record<string, unknown> | null
}

export type PluginUiCatalogSnapshot = {
  catalog: PluginUiCatalog
  tool_registry_generation: number
  tool_registry_last_event?: ToolRegistryChangedEvent | null
}

export type PluginToolRegistryChangesResponse = {
  generation: number
  events: ToolRegistryChangedEvent[]
}

export type PluginUiToolInvokeResponse = {
  plugin_id: string
  tool: string
  title: string
  output_text: string
  payload?: unknown
  metadata?: Record<string, string>
}

export type PluginUiActionRunResponse = {
  plugin_id: string
  action_id: string
  action: PluginUiAction
  result?: PluginUiToolInvokeResponse | null
}

export type PluginLogEntry = {
  seq: number
  plugin_id: string
  level: string
  target?: string | null
  message: string
  timestamp_ms: number
}

export type MarketplacePluginResource = {
  plugin_id: string
  name: string
  description: string
  homepage?: string | null
  version_count: number
  latest_version?: string | null
  latest_kind?: string | null
  latest_platform?: string | null
}

export type MarketplaceInstalledPluginResource = {
  plugin_id: string
  version: string
  kind: string
  platform: string
  binary_path: string
  config_path: string
  sha256?: string | null
  installed_at: string
  registry_id: string
  registry_url: string
  archive_extracted: boolean
}

export type MarketplaceOutdatedPluginResource = {
  plugin_id: string
  installed_version: string
  latest_version: string
}

export type MarketplaceInstallOutcomeResource = {
  plugin_id: string
  version: string
  kind: string
  artifact_path: string
  config_path: string
  dry_run: boolean
}

export type MarketplaceUninstallOutcomeResource = {
  plugin_id: string
  version: string
  config_path: string
}

export type MarketplaceUpgradeOutcomeResource = {
  plugin_id: string
  previous_version: string
  installed_version: string
  upgraded: boolean
  outcome?: MarketplaceInstallOutcomeResource | null
}

export type RuntimeReloadResponse = {
  cause: string
  previous_generation: number
  generation: number
  loaded_at: string
}

export type UsagePeriod = 'today' | 'last_7_days' | 'last_30_days' | 'month_to_date' | 'all_time'

export type UsageTotals = {
  runs: number
  sessions: number
  input_tokens: number
  output_tokens: number
  reasoning_tokens: number
  cache_write_tokens: number
  cache_read_tokens: number
  total_tokens: number
  cache_input_tokens: number
  cache_hit_rate: number
  total_cost_usd: number
  recorded_cost_usd: number
  estimated_cost_usd: number
  unpriced_runs: number
}

export type UsageDailyBreakdown = UsageTotals & {
  date: string
}

export type ProviderUsageBreakdown = UsageTotals & {
  provider_id: string
}

export type ModelUsageBreakdown = UsageTotals & {
  provider_id: string
  model_id: string
}

export type SessionUsageBreakdown = UsageTotals & {
  session_id: number
  title: string
  is_subagent: boolean
  first_message_at: string
  last_message_at: string
}

export type UsageStats = {
  generated_at: string
  period: UsagePeriod
  period_label: string
  from?: string | null
  to?: string | null
  totals: UsageTotals
  by_day: UsageDailyBreakdown[]
  by_provider: ProviderUsageBreakdown[]
  by_model: ModelUsageBreakdown[]
  by_session: SessionUsageBreakdown[]
}

export type ProviderSummary = {
  provider_id: string
  defaults: {
    adapter?: string | null
    model: string
  }
  adapters?: ProviderAdapterSummary[]
  native_tools?: {
    enabled: boolean
    model_count: number
    bindings?: Array<{
      tool: string
      route: string
    }>
  } | null
}

export type ProviderAdapterSummary = {
  adapter_id: string
  enabled: boolean
  configured_model_count: number
}

export type ModelCatalogSourceKind = 'generated' | 'cache'

export type ModelCatalogEntry = {
  model_id: string
  source: ModelCatalogSourceKind
  source_label?: string | null
  display_name?: string | null
  origin?: string | null
  lifecycle?: string | null
  context_window_tokens?: number | null
  max_input_tokens?: number | null
  max_output_tokens?: number | null
  description?: string | null
  knowledge_cutoff?: string | null
  release_date?: string | null
  last_updated?: string | null
  open_weights?: boolean | null
  default_thinking_mode?: string | null
  supports_parallel_tool_calls?: boolean | null
  supports_verbosity?: boolean | null
  default_verbosity?: string | null
  default_temperature?: string | null
  default_top_p?: string | null
  default_top_k?: number | null
  assistant_reasoning_interleaved?: boolean | null
  assistant_reasoning_field?: string | null
  output_modalities?: string[] | null
  pricing?: ProviderModelPricing | null
  thinking_modes?: Record<string, ProviderModelThinkingMode>
  speed_modes?: Record<string, ProviderModelSpeedMode>
  input?: unknown
  features?: unknown
  capabilities?: Record<string, unknown>
}

export type ModelCatalogResponse = {
  refreshing: boolean
  last_refresh_at?: string | null
  last_successful_source?: ModelCatalogSourceKind | null
  last_error?: string | null
  entry_count: number
}

export type ModelCatalogSummary = ModelCatalogResponse

export type ModelCatalogRefreshResponse = {
  started: boolean
  task: RuntimeBackgroundTask
  summary: ModelCatalogSummary
}

export type ModelCatalogListResponse = {
  summary: ModelCatalogSummary
  total: number
  offset: number
  limit: number
  available_origins: string[]
  items: ModelCatalogEntry[]
}

export type ModelCatalogLookupResponse = {
  items: ModelCatalogEntry[]
}

export type ModelCatalogListQuery = {
  q?: string
  origin?: string
  offset?: number
  limit?: number
}

export type ConfigSettingsPatchRequest = {
  path?: string | null
  changes: Record<string, unknown>
  dry_run?: boolean
  validate?: boolean
  reload?: boolean
}

export type ConfigSettingsSetRequest = {
  path: string
  value: unknown
  dry_run?: boolean
  validate?: boolean
  reload?: boolean
}

export type ConfigSettingsDeleteRequest = {
  path: string
  dry_run?: boolean
  validate?: boolean
  reload?: boolean
}

export type ConfigSettingsSource = 'effective' | 'file'

export type ConfigSettingsGetRequest = {
  path?: string | null
  source?: ConfigSettingsSource
}

export type ConfigSettingsReadResponse = {
  config_path: string
  config_found: boolean
  source: ConfigSettingsSource
  path?: string | null
  value: unknown
}

export type ConfigSettingsEditResponse = {
  config_path: string
  config_found: boolean
  operation: string
  path?: string | null
  dry_run: boolean
  changed: boolean
  created: boolean
  deleted: boolean
  validated: boolean
  reload_requested: boolean
  reload_required: boolean
  reload?: RuntimeReloadResponse | null
  previous: unknown
  current: unknown
}

export type AuthProvider = {
  provider_id: string
  configured: boolean
  credential_present: boolean
  credential_type?: string | null
  key_preview?: string | null
  expires_at?: string | null
  expired?: boolean | null
  account_id?: string | null
  enterprise_url?: string | null
  username?: string | null
  display_name?: string | null
  email?: string | null
  avatar_url?: string | null
}

export type AuthBrowserStartResponse = {
  provider_id: string
  instance_url?: string | null
  authorize_url: string
  state: string
  pkce_verifier: string
}

export type AuthDeviceStartResponse = {
  provider_id: string
  enterprise_domain?: string | null
  verification_url: string
  user_code: string
  device_code: string
  interval_seconds: number
}

export type AuthLoginResultResponse = {
  completed: boolean
  provider?: AuthProvider | null
}

export type WorkspaceResource = {
  id: number
  path: string
  created_at: string
  updated_at: string
  session_count?: number | null
}

export type SessionGoalStatus = 'active' | 'paused' | 'completed' | string

export type SessionGoalResource = {
  id: number
  session_id: number
  objective: string
  status: SessionGoalStatus
  created_at: string
  updated_at: string
  completed_at?: string | null
}

export type PermissionMode = 'allow' | 'ask' | 'deny'

export type PathAccessModes = {
  read?: PermissionMode
  write?: PermissionMode
}

export type PathAccessRule = PathAccessModes | string

export type PathPermissionConfig = {
  workspace?: PathAccessModes
  external?: PathAccessModes
  rules?: Record<string, PathAccessRule>
}

export type NetworkPermissionConfig = {
  internet?: PermissionMode
  private?: PermissionMode
  loopback?: PermissionMode
  rules?: Record<string, PermissionMode>
}

export type ToolPermissionRules = PermissionMode | Record<string, PermissionMode>

export type ToolPermissionConfig = {
  tags?: Record<string, PermissionMode>
  names?: Record<string, PermissionMode>
  plugin?: Record<string, PermissionMode>
  rules?: Record<string, ToolPermissionRules>
}

export type PermissionConfig = {
  path?: PathPermissionConfig
  network?: NetworkPermissionConfig
  entries?: ToolPermissionConfig
  tools?: ToolPermissionConfig
}

export type AgentPermissionConfig = PermissionConfig

export type PermissionSubjectKind = 'tool' | 'path_access' | 'network_access'

export type PermissionRuleResource = {
  id: number
  action_key: string
  subject_kind: string
  tool_name?: string | null
  qualifier?: string | null
  path_access_kind?: string | null
  workspace_root?: string | null
  target_path?: string | null
  network_target?: string | null
  network_host?: string | null
  network_port?: number | null
  mode: PermissionMode
  scope: string
  session_id?: number | null
  workspace_id?: number | null
  source: string
  reason?: string | null
  operator?: string | null
  revoked_at?: string | null
  revoked_reason?: string | null
  revoked_by?: string | null
  created_at: string
  updated_at: string
}

export type WorkspaceFileKind = 'directory' | 'file' | 'symlink' | 'other'

export type WorkspaceFileNode = {
  name: string
  path: string
  kind: WorkspaceFileKind
  size?: number | null
  children?: WorkspaceFileNode[] | null
}

export type WorkspaceFileTreeResource = {
  workspace_id: number
  root: string
  path: string
  entries: WorkspaceFileNode[]
}

export type GitStatusResource = {
  workspace_root: string
  git_available: boolean
  repo: boolean
  gh_available: boolean
  branch?: string | null
  upstream?: string | null
  ahead?: number | null
  behind?: number | null
  staged_files: number
  unstaged_files: number
  untracked_files: number
  changed_files: number
  clean: boolean
  worktree_active_sessions: number
  worktree_managed_dirs: number
}

export type SessionResource = {
  id: number
  parent_id?: number | null
  depth?: number
  root_id?: number
  workspace_id: number
  title: string
  version: number
  is_subagent?: boolean
  created_at: string
  updated_at: string
  message_count: number
  child_session_count: number
  last_message_at?: string | null
  goal?: SessionGoalResource | null
}

export type SessionTreeResource = SessionResource

export type RewindCheckpointEntryResource = {
  message_id: number
  role: string
  preview: string
}

export type RewindCheckpointResource = {
  schema: number
  at_ms: number
  target_message_id: number
  dropped: RewindCheckpointEntryResource[]
}

export type MessagePart = {
  id: number
  message_id: number
  part_index: number
  status: string
  kind: string
  name?: string | null
  summary?: string | null
  has_detail?: boolean
  operation_id?: string | null
  created_at: string
  content?: Record<string, unknown> | null
}

export type MessageResource = {
  id: number
  session_id: number
  role: 'user' | 'assistant' | 'system'
  state: string
  created_at: string
  updated_at: string
  metadata: Record<string, unknown>
  usage?: Record<string, unknown> | null
  part_count: number
  parts?: MessagePart[] | null
}

export type PartLoadMode = 'none' | 'summary' | 'full'

export type PermissionRiskLevel = 'low' | 'medium' | 'high' | 'critical'

export type PermissionDecisionTraceStep = {
  source_kind: 'static_policy' | 'persisted_rule' | 'plugin_advice' | 'managed_policy'
  summary: string
  source?: string | null
  scope?: 'session' | 'workspace' | 'global' | null
  operator?: string | null
}

export type PermissionRequest = {
  request_id: string
  session_id?: number | null
  action: Record<string, unknown>
  related_actions?: Record<string, unknown>[]
  requested_actions?: Record<string, unknown>[]
  reason: string
  explanation?: string
  source?: string | null
  scope?: 'session' | 'workspace' | 'global' | null
  operator?: string | null
  risk?: PermissionRiskLevel | null
  trace?: PermissionDecisionTraceStep[]
  created_at: string
}

export type UserInputQuestion = {
  id: string
  header?: string
  question: string
  options?: Array<{
    label: string
    description?: string
  }>
  multiple?: boolean
  allow_custom?: boolean
}

export type UserInputRequest = {
  request_id: string
  session_id?: number | null
  questions: UserInputQuestion[]
  created_at: string
}

export type PendingInteractiveRequest =
  | ({ kind: 'permission' } & PermissionRequest)
  | ({ kind: 'user_input' } & UserInputRequest)

export type SessionExecutionContextResource = {
  agent_profile?: string | null
  active_skill_name?: string | null
  system_prompt_override?: string | null
  effective_permission?: PermissionConfig
  model_provider_id?: string | null
  model_adapter_id?: string | null
  model_id?: string | null
  model_thinking_mode?: string | null
  model_speed_mode?: string | null
  model_verbosity?: string | null
  model_parallel_tool_calls?: boolean | null
  effective_workspace_root?: string | null
  task_id?: string | null
}

export type SessionUsageLimitBasis = 'context_window' | 'prompt_threshold'

export type SessionUsageResource = {
  measured_prompt_tokens?: number | null
  current_tokens: number
  projected_tokens?: number | null
  limit_tokens?: number | null
  limit_basis?: SessionUsageLimitBasis | null
  reserved_tokens?: number | null
  model_context_window_tokens?: number | null
  model_max_input_tokens?: number | null
  model_max_output_tokens?: number | null
}

export type SessionExecutionResource = {
  session: SessionResource
  blocked: boolean
  run_state: 'idle' | 'awaiting_model' | string
  latest_event_seq?: number | null
  automation?: SessionAutomationResource | null
  execution: SessionExecutionContextResource
  pending_interactive_requests: PendingInteractiveRequest[]
  pending_permission_requests: PermissionRequest[]
  pending_user_input_requests: UserInputRequest[]
  goal?: SessionGoalResource | null
  usage: SessionUsageResource
}

export function formatSessionExecutionModelLabel(
  context:
    | Pick<SessionExecutionContextResource, 'model_provider_id' | 'model_adapter_id' | 'model_id'>
    | null
    | undefined,
): string | null {
  const providerId = String(context?.model_provider_id || '').trim()
  const adapterId = String(context?.model_adapter_id || '').trim()
  const modelId = String(context?.model_id || '').trim()
  if (!providerId && !modelId) return null
  const resolvedProviderId = providerId || 'auto'
  const resolvedModelId = modelId || 'default'
  return adapterId
    ? `${resolvedProviderId}/${adapterId}/${resolvedModelId}`
    : `${resolvedProviderId}/${resolvedModelId}`
}

export type DomainEventRecord = {
  id?: string | null
  seq_global: number
  seq_session?: number | null
  session_id?: number | null
  workspace_id?: number | null
  created_at: string
  causation_id?: string | null
  correlation_id?: string | null
  envelope_schema?: number
  kind: string
  payload: Record<string, unknown>
}

export type EventNotification =
  | {
      kind: 'event'
      data: {
        subscription: string
        event: DomainEventRecord
      }
    }
  | {
      kind: 'lagged'
      data: {
        subscription: string
        skipped: number
      }
    }
  | {
      kind: 'resumed'
      data: {
        subscription: string
        up_to_seq_global: number
      }
    }
  | {
      kind: 'subscription_closed'
      data: {
        subscription: string
        reason: string
      }
    }

export type SessionEventStreamHandle = {
  close: () => void
}

export type NotificationStreamHandle = {
  close: () => void
}

type PaginatedResponse<T> = {
  items: T[]
  page?: {
    limit: number
    returned: number
    has_more: boolean
    next_cursor?: string | null
    order: 'asc' | 'desc'
  }
}

async function collectPagedItems<T>(
  fetchPage: (cursor?: string) => Promise<PaginatedResponse<T>>,
  options?: {
    merge?: 'append' | 'prepend'
    maxPages?: number
    resourceName?: string
  },
): Promise<T[]> {
  const merge = options?.merge ?? 'append'
  const maxPages = Math.max(1, Math.trunc(options?.maxPages ?? 100))
  const resourceName = options?.resourceName ?? 'paged resource'
  let cursor: string | undefined
  let items: T[] = []
  const seenCursors = new Set<string>()

  for (let pageIndex = 0; pageIndex < maxPages; pageIndex += 1) {
    const response = await fetchPage(cursor)
    const chunk = response.items ?? []
    items = merge === 'prepend' ? chunk.concat(items) : items.concat(chunk)

    const nextCursor = response.page?.next_cursor ?? undefined
    if (!response.page?.has_more || !nextCursor) {
      return items
    }

    if (seenCursors.has(nextCursor)) {
      throw new Error(`Pagination cursor repeated while loading ${resourceName}`)
    }
    seenCursors.add(nextCursor)
    cursor = nextCursor
  }

  throw new Error(`Pagination exceeded ${maxPages} pages while loading ${resourceName}`)
}

function extractErrorCode(bodyText: string): string {
  const txt = String(bodyText || '').trim()
  if (!txt) return ''

  try {
    const parsed = JSON.parse(txt) as unknown
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return ''
    const record = parsed as Record<string, unknown>
    if (typeof record.code === 'string') return record.code.trim()

    const nested = record.error
    if (nested && typeof nested === 'object' && !Array.isArray(nested)) {
      const nestedCode = (nested as Record<string, unknown>).code
      if (typeof nestedCode === 'string') return nestedCode.trim()
    }
  } catch {
    // ignore non-json payloads
  }

  return ''
}

export async function fetchStudioHealth(): Promise<StudioHealth> {
  return await apiJson<StudioHealth>('/health')
}

export async function fetchRuntimeStatus(): Promise<RuntimeStatus> {
  return await apiJson<RuntimeStatus>('/api/v1/runtime')
}

export async function fetchUsageStats(
  input: {
    period?: UsagePeriod
    from?: string | null
    to?: string | null
  } = {},
): Promise<UsageStats> {
  const params = new URLSearchParams()
  if (input.period) params.set('period', input.period)
  if (input.from) params.set('from', input.from)
  if (input.to) params.set('to', input.to)
  const suffix = params.toString()
  return await apiJson<UsageStats>(`/api/v1/usage${suffix ? `?${suffix}` : ''}`)
}

export async function reloadRuntime(): Promise<RuntimeBackgroundTaskStartResponse> {
  return await apiJson<RuntimeBackgroundTaskStartResponse>('/api/v1/runtime/reload', {
    method: 'POST',
  })
}

export async function patchSettings(input: ConfigSettingsPatchRequest): Promise<ConfigSettingsEditResponse> {
  return await apiJson<ConfigSettingsEditResponse>('/api/v1/settings', {
    method: 'PATCH',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(input),
  })
}

export async function setSettings(input: ConfigSettingsSetRequest): Promise<ConfigSettingsEditResponse> {
  return await apiJson<ConfigSettingsEditResponse>('/api/v1/settings', {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(input),
  })
}

export async function deleteSettings(input: ConfigSettingsDeleteRequest): Promise<ConfigSettingsEditResponse> {
  const params = new URLSearchParams()
  params.set('path', input.path.trim())
  if (input.dry_run) params.set('dry_run', 'true')
  if (input.validate !== undefined) params.set('validate', String(input.validate))
  if (input.reload !== undefined) params.set('reload', String(input.reload))
  return await apiJson<ConfigSettingsEditResponse>(`/api/v1/settings?${params.toString()}`, {
    method: 'DELETE',
  })
}

export async function getSettings(input: ConfigSettingsGetRequest = {}): Promise<ConfigSettingsReadResponse> {
  const params = new URLSearchParams()
  if (input.path?.trim()) params.set('path', input.path.trim())
  if (input.source) params.set('source', input.source)
  const suffix = params.toString()
  return await apiJson<ConfigSettingsReadResponse>(`/api/v1/settings${suffix ? `?${suffix}` : ''}`)
}

export async function listModelCatalogEntries(query: ModelCatalogListQuery = {}): Promise<ModelCatalogListResponse> {
  const params = new URLSearchParams()
  if (query.q?.trim()) params.set('q', query.q.trim())
  if (query.origin?.trim()) params.set('origin', query.origin.trim())
  if (query.offset !== undefined) params.set('offset', String(query.offset))
  if (query.limit !== undefined) params.set('limit', String(query.limit))
  const suffix = params.toString()
  return await apiJson<ModelCatalogListResponse>(`/api/v1/model-catalog${suffix ? `?${suffix}` : ''}`)
}

export async function lookupModelCatalogEntries(modelIds: string[]): Promise<ModelCatalogEntry[]> {
  const response = await apiJson<ModelCatalogLookupResponse>('/api/v1/model-catalog/lookup', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ model_ids: modelIds }),
  })
  return response.items ?? []
}

export async function refreshModelCatalog(): Promise<ModelCatalogRefreshResponse> {
  return await apiJson<ModelCatalogRefreshResponse>('/api/v1/model-catalog/refresh', {
    method: 'POST',
  })
}

export async function listRuntimeBackgroundTasks(): Promise<RuntimeBackgroundTask[]> {
  const response = await apiJson<RuntimeBackgroundTaskListResponse>('/api/v1/runtime/tasks')
  return response.items ?? []
}

export async function cancelRuntimeBackgroundTask(taskId: string): Promise<RuntimeBackgroundTask> {
  const response = await apiJson<RuntimeBackgroundTaskCancelResponse>(
    `/api/v1/runtime/tasks/${encodeURIComponent(taskId)}/cancel`,
    {
      method: 'POST',
    },
  )
  return response.task
}

export async function listPlugins(): Promise<PluginStatus[]> {
  const response = await apiJson<{ entries: PluginStatus[] }>('/api/v1/plugins')
  return response.entries ?? []
}

export async function fetchPluginUiCatalogSnapshot(): Promise<PluginUiCatalogSnapshot> {
  return await apiJson<PluginUiCatalogSnapshot>('/api/v1/plugins/ui')
}

export async function fetchPluginUiCatalog(): Promise<PluginUiCatalog> {
  const response = await fetchPluginUiCatalogSnapshot()
  return response.catalog
}

export async function listPluginToolRegistryChanges(input?: {
  afterGeneration?: number | null
  limit?: number
}): Promise<PluginToolRegistryChangesResponse> {
  const params = new URLSearchParams()
  if (input?.afterGeneration && input.afterGeneration > 0) {
    params.set('after_generation', String(Math.trunc(input.afterGeneration)))
  }
  if (input?.limit && input.limit > 0) {
    params.set('limit', String(Math.trunc(input.limit)))
  }
  const suffix = params.toString()
  return await apiJson<PluginToolRegistryChangesResponse>(
    `/api/v1/plugins/tools/changes${suffix ? `?${suffix}` : ''}`,
  )
}

export async function invokePluginUiTool(input: {
  tool: string
  pluginId?: string
  payload?: Record<string, unknown>
  sessionId?: number | null
}): Promise<PluginUiToolInvokeResponse> {
  return await apiJson<PluginUiToolInvokeResponse>('/api/v1/plugins/ui/invoke-tool', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      tool: input.tool.trim(),
      ...(input.pluginId?.trim() ? { plugin_id: input.pluginId.trim() } : {}),
      ...(input.payload ? { input: input.payload } : {}),
      ...(input.sessionId === null || input.sessionId === undefined ? {} : { session_id: input.sessionId }),
    }),
  })
}

export async function runPluginUiAction(input: {
  pluginId: string
  actionId: string
  payload?: Record<string, unknown>
  sessionId?: number | null
}): Promise<PluginUiActionRunResponse> {
  return await apiJson<PluginUiActionRunResponse>(
    `/api/v1/plugins/${encodeURIComponent(input.pluginId)}/ui/actions/${encodeURIComponent(input.actionId)}`,
    {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        ...(input.payload ? { input: input.payload } : {}),
        ...(input.sessionId === null || input.sessionId === undefined ? {} : { session_id: input.sessionId }),
      }),
    },
  )
}

export async function getPlugin(pluginId: string): Promise<PluginInspect> {
  const response = await apiJson<{ plugin: PluginInspect }>(`/api/v1/plugins/${encodeURIComponent(pluginId)}`)
  return response.plugin
}

export async function listPluginLogs(
  pluginId: string,
  options?: {
    limit?: number
    afterSeq?: number
  },
): Promise<PluginLogEntry[]> {
  const params = new URLSearchParams()
  params.set('limit', String(options?.limit ?? 50))
  if (options?.afterSeq !== undefined) {
    params.set('after_seq', String(options.afterSeq))
  }
  const response = await apiJson<{ entries: PluginLogEntry[] }>(
    `/api/v1/plugins/${encodeURIComponent(pluginId)}/logs?${params.toString()}`,
  )
  return response.entries ?? []
}

export async function searchMarketplacePlugins(input: {
  registryId?: string
  registryUrl: string
  query?: string
  refresh?: boolean
}): Promise<MarketplacePluginResource[]> {
  const response = await apiJson<{ entries: MarketplacePluginResource[] }>('/api/v1/plugins/marketplace/search', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      ...(input.registryId?.trim() ? { registry_id: input.registryId.trim() } : {}),
      registry_url: input.registryUrl.trim(),
      ...(input.query?.trim() ? { query: input.query.trim() } : {}),
      ...(input.refresh ? { refresh: true } : {}),
    }),
  })
  return response.entries ?? []
}

export async function syncMarketplaceRegistry(input: {
  registryId?: string
  registryUrl: string
}): Promise<RuntimeBackgroundTaskStartResponse> {
  return await apiJson<RuntimeBackgroundTaskStartResponse>('/api/v1/plugins/marketplace/sync', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      ...(input.registryId?.trim() ? { registry_id: input.registryId.trim() } : {}),
      registry_url: input.registryUrl.trim(),
    }),
  })
}

export async function listMarketplaceInstalledPlugins(): Promise<MarketplaceInstalledPluginResource[]> {
  const response = await apiJson<{ entries: MarketplaceInstalledPluginResource[] }>(
    '/api/v1/plugins/marketplace/installed',
  )
  return response.entries ?? []
}

export async function listMarketplaceOutdatedPlugins(): Promise<MarketplaceOutdatedPluginResource[]> {
  const response = await apiJson<{ entries: MarketplaceOutdatedPluginResource[] }>(
    '/api/v1/plugins/marketplace/outdated',
  )
  return response.entries ?? []
}

export async function installMarketplacePlugin(input: {
  spec: string
  registryId?: string
  registryUrl: string
  force?: boolean
  dryRun?: boolean
  allowUnverified?: boolean
  refresh?: boolean
  requireSignature?: boolean
}): Promise<RuntimeBackgroundTaskStartResponse> {
  return await apiJson<RuntimeBackgroundTaskStartResponse>('/api/v1/plugins/marketplace/install', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      spec: input.spec.trim(),
      ...(input.registryId?.trim() ? { registry_id: input.registryId.trim() } : {}),
      registry_url: input.registryUrl.trim(),
      ...(input.force ? { force: true } : {}),
      ...(input.dryRun ? { dry_run: true } : {}),
      ...(input.allowUnverified ? { allow_unverified: true } : {}),
      ...(input.refresh ? { refresh: true } : {}),
      ...(input.requireSignature ? { require_signature: true } : {}),
    }),
  })
}

export async function uninstallMarketplacePlugin(input: {
  pluginId: string
  cascade?: boolean
}): Promise<RuntimeBackgroundTaskStartResponse> {
  return await apiJson<RuntimeBackgroundTaskStartResponse>('/api/v1/plugins/marketplace/uninstall', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      plugin_id: input.pluginId.trim(),
      ...(input.cascade ? { cascade: true } : {}),
    }),
  })
}

export async function upgradeMarketplacePlugins(input: {
  pluginId?: string
  all?: boolean
  registryId?: string
  registryUrl?: string
}): Promise<RuntimeBackgroundTaskStartResponse> {
  return await apiJson<RuntimeBackgroundTaskStartResponse>('/api/v1/plugins/marketplace/upgrade', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      ...(input.pluginId?.trim() ? { plugin_id: input.pluginId.trim() } : {}),
      ...(input.all ? { all: true } : {}),
      ...(input.registryId?.trim() ? { registry_id: input.registryId.trim() } : {}),
      ...(input.registryUrl?.trim() ? { registry_url: input.registryUrl.trim() } : {}),
    }),
  })
}

export async function listAuthProviders(): Promise<AuthProvider[]> {
  return await apiJson<AuthProvider[]>('/api/v1/auth/providers')
}

export async function setProviderApiKey(providerId: string, apiKey: string): Promise<void> {
  await apiJson(`/api/v1/auth/providers/${encodeURIComponent(providerId)}/api-key`, {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ api_key: apiKey }),
  })
}

export async function deleteProviderCredential(providerId: string): Promise<void> {
  await apiJson(`/api/v1/auth/providers/${encodeURIComponent(providerId)}`, {
    method: 'DELETE',
  })
}

export async function refreshProviderCredential(providerId: string): Promise<void> {
  await apiJson(`/api/v1/auth/providers/${encodeURIComponent(providerId)}/refresh`, {
    method: 'POST',
  })
}

export async function startOpenAiBrowserAuth(redirectUri: string): Promise<AuthBrowserStartResponse> {
  return await apiJson<AuthBrowserStartResponse>('/api/v1/auth/providers/openai/browser/start', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ redirect_uri: redirectUri }),
  })
}

export async function finishOpenAiBrowserAuth(input: {
  code: string
  pkceVerifier: string
  redirectUri: string
}): Promise<AuthLoginResultResponse> {
  return await apiJson<AuthLoginResultResponse>('/api/v1/auth/providers/openai/browser/finish', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      code: input.code,
      pkce_verifier: input.pkceVerifier,
      redirect_uri: input.redirectUri,
    }),
  })
}

export async function startOpenAiDeviceAuth(): Promise<AuthDeviceStartResponse> {
  return await apiJson<AuthDeviceStartResponse>('/api/v1/auth/providers/openai/device/start', {
    method: 'POST',
  })
}

export async function pollOpenAiDeviceAuth(input: {
  deviceCode: string
  userCode: string
}): Promise<AuthLoginResultResponse> {
  return await apiJson<AuthLoginResultResponse>('/api/v1/auth/providers/openai/device/poll', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      device_code: input.deviceCode,
      user_code: input.userCode,
    }),
  })
}

export async function startGitLabBrowserAuth(input: {
  instanceUrl: string
  redirectUri: string
}): Promise<AuthBrowserStartResponse> {
  return await apiJson<AuthBrowserStartResponse>('/api/v1/auth/providers/gitlab/browser/start', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      instance_url: input.instanceUrl,
      redirect_uri: input.redirectUri,
    }),
  })
}

export async function finishGitLabBrowserAuth(input: {
  instanceUrl: string
  code: string
  pkceVerifier: string
  redirectUri: string
}): Promise<AuthLoginResultResponse> {
  return await apiJson<AuthLoginResultResponse>('/api/v1/auth/providers/gitlab/browser/finish', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      instance_url: input.instanceUrl,
      code: input.code,
      pkce_verifier: input.pkceVerifier,
      redirect_uri: input.redirectUri,
    }),
  })
}

export async function startAtomGitBrowserAuth(providerId: string): Promise<AuthBrowserStartResponse> {
  return await apiJson<AuthBrowserStartResponse>('/api/v1/auth/providers/atomgit/browser/start', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ provider_id: providerId }),
  })
}

export async function pollAtomGitBrowserAuth(input: {
  providerId: string
  state: string
}): Promise<AuthLoginResultResponse> {
  return await apiJson<AuthLoginResultResponse>('/api/v1/auth/providers/atomgit/browser/poll', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      provider_id: input.providerId,
      state: input.state,
    }),
  })
}

export async function startCopilotDeviceAuth(enterpriseDomain?: string): Promise<AuthDeviceStartResponse> {
  return await apiJson<AuthDeviceStartResponse>('/api/v1/auth/providers/github-copilot/device/start', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(enterpriseDomain?.trim() ? { enterprise_domain: enterpriseDomain.trim() } : {}),
  })
}

export async function pollCopilotDeviceAuth(input: {
  deviceCode: string
  enterpriseDomain?: string
}): Promise<AuthLoginResultResponse> {
  return await apiJson<AuthLoginResultResponse>('/api/v1/auth/providers/github-copilot/device/poll', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      device_code: input.deviceCode,
      ...(input.enterpriseDomain?.trim() ? { enterprise_domain: input.enterpriseDomain.trim() } : {}),
    }),
  })
}

export async function listWorkspaces(): Promise<WorkspaceResource[]> {
  return await collectPagedItems(
    (cursor) =>
      apiJson<PaginatedResponse<WorkspaceResource>>(
        `/api/v1/workspaces?limit=100&include_session_count=true${cursor ? `&cursor=${encodeURIComponent(cursor)}` : ''}`,
      ),
    { resourceName: 'workspaces' },
  )
}

export async function resolveWorkspace(path: string, createIfMissing: boolean): Promise<WorkspaceResource> {
  return await apiJson<WorkspaceResource>('/api/v1/workspaces/resolve', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      path,
      create_if_missing: createIfMissing,
    }),
  })
}

export async function createWorkspace(path: string): Promise<WorkspaceResource> {
  return await apiJson<WorkspaceResource>('/api/v1/workspaces', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ path }),
  })
}

export async function updateWorkspace(input: { workspaceId: number; path: string }): Promise<WorkspaceResource> {
  return await apiJson<WorkspaceResource>(`/api/v1/workspaces/${input.workspaceId}`, {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ path: input.path }),
  })
}

export async function deleteWorkspace(workspaceId: number): Promise<WorkspaceResource> {
  return await apiJson<WorkspaceResource>(`/api/v1/workspaces/${workspaceId}`, {
    method: 'DELETE',
  })
}

export async function listPermissionRules(search = ''): Promise<PermissionRuleResource[]> {
  return await collectPagedItems(
    (cursor) => {
      const params = new URLSearchParams({ limit: '100' })
      if (cursor) params.set('cursor', cursor)
      if (search.trim()) params.set('search', search.trim())
      return apiJson<PaginatedResponse<PermissionRuleResource>>(`/api/v1/permission-rules?${params.toString()}`)
    },
    { resourceName: 'permission rules' },
  )
}

export async function createPermissionRule(input: {
  actionKey?: string
  subjectKind?: PermissionSubjectKind
  toolName?: string
  qualifier?: string
  pathAccessKind?: string
  workspaceRoot?: string
  targetPath?: string
  networkTarget?: string
  networkHost?: string
  networkPort?: number
  scope?: 'session' | 'workspace' | 'global'
  sessionId?: number
  mode: PermissionMode
}): Promise<PermissionRuleResource> {
  return await apiJson<PermissionRuleResource>('/api/v1/permission-rules', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      ...(input.actionKey ? { action_key: input.actionKey } : {}),
      ...(input.subjectKind ? { subject_kind: input.subjectKind } : {}),
      ...(input.toolName ? { tool_name: input.toolName } : {}),
      ...(input.qualifier ? { qualifier: input.qualifier } : {}),
      ...(input.pathAccessKind ? { path_access_kind: input.pathAccessKind } : {}),
      ...(input.workspaceRoot ? { workspace_root: input.workspaceRoot } : {}),
      ...(input.targetPath ? { target_path: input.targetPath } : {}),
      ...(input.networkTarget ? { network_target: input.networkTarget } : {}),
      ...(input.networkHost ? { network_host: input.networkHost } : {}),
      ...(input.networkPort !== undefined ? { network_port: input.networkPort } : {}),
      ...(input.scope ? { scope: input.scope } : {}),
      ...(input.sessionId !== undefined ? { session_id: input.sessionId } : {}),
      mode: input.mode,
    }),
  })
}

export async function updatePermissionRule(input: {
  id: number
  actionKey?: string
  subjectKind?: PermissionSubjectKind
  toolName?: string
  qualifier?: string
  pathAccessKind?: string
  workspaceRoot?: string
  targetPath?: string
  networkTarget?: string
  networkHost?: string
  networkPort?: number
  scope?: 'session' | 'workspace' | 'global'
  sessionId?: number
  mode: PermissionMode
}): Promise<PermissionRuleResource> {
  return await apiJson<PermissionRuleResource>(`/api/v1/permission-rules/${input.id}`, {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      ...(input.actionKey ? { action_key: input.actionKey } : {}),
      ...(input.subjectKind ? { subject_kind: input.subjectKind } : {}),
      ...(input.toolName ? { tool_name: input.toolName } : {}),
      ...(input.qualifier ? { qualifier: input.qualifier } : {}),
      ...(input.pathAccessKind ? { path_access_kind: input.pathAccessKind } : {}),
      ...(input.workspaceRoot ? { workspace_root: input.workspaceRoot } : {}),
      ...(input.targetPath ? { target_path: input.targetPath } : {}),
      ...(input.networkTarget ? { network_target: input.networkTarget } : {}),
      ...(input.networkHost ? { network_host: input.networkHost } : {}),
      ...(input.networkPort !== undefined ? { network_port: input.networkPort } : {}),
      ...(input.scope ? { scope: input.scope } : {}),
      ...(input.sessionId !== undefined ? { session_id: input.sessionId } : {}),
      mode: input.mode,
    }),
  })
}

export async function revokePermissionRule(id: number, reason?: string): Promise<PermissionRuleResource> {
  return await apiJson<PermissionRuleResource>(`/api/v1/permission-rules/${id}/revoke`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(reason ? { reason } : {}),
  })
}

export async function deletePermissionRule(id: number): Promise<PermissionRuleResource> {
  return await apiJson<PermissionRuleResource>(`/api/v1/permission-rules/${id}`, {
    method: 'DELETE',
  })
}

export async function getGitStatus(): Promise<GitStatusResource> {
  return await apiJson<GitStatusResource>('/api/v1/git/status')
}

export async function initGitProject(): Promise<GitStatusResource> {
  return await apiJson<GitStatusResource>('/api/v1/project/git/init', {
    method: 'POST',
  })
}

export async function getVcsDiffRaw(): Promise<string> {
  return await apiText('/api/v1/vcs/diff/raw', {
    headers: {
      accept: 'text/plain',
    },
  })
}

export async function listWorkspaceFileTree(input: {
  workspaceId: number
  path?: string
  depth?: number
  limit?: number
}): Promise<WorkspaceFileTreeResource> {
  const params = new URLSearchParams()
  if (input.path?.trim()) params.set('path', input.path.trim())
  if (input.depth !== undefined) params.set('depth', String(input.depth))
  if (input.limit !== undefined) params.set('limit', String(input.limit))
  const query = params.toString()
  return await apiJson<WorkspaceFileTreeResource>(
    `/api/v1/workspaces/${input.workspaceId}/files${query ? `?${query}` : ''}`,
  )
}

export async function listSessions(
  workspaceId: number,
  options?: {
    parentId?: number | null
    roots?: boolean
    search?: string
  },
): Promise<SessionResource[]> {
  return await collectPagedItems(
    (cursor) => {
      const params = new URLSearchParams({
        workspace_id: String(workspaceId),
        limit: '100',
      })
      if (cursor) params.set('cursor', cursor)
      if (options?.parentId !== undefined && options.parentId !== null) {
        params.set('parent_id', String(options.parentId))
      }
      if (options?.roots) {
        params.set('roots', 'true')
      }
      if (options?.search?.trim()) {
        params.set('search', options.search.trim())
      }
      return apiJson<PaginatedResponse<SessionResource>>(`/api/v1/sessions?${params.toString()}`)
    },
    { resourceName: 'sessions' },
  )
}

export async function createSession(input: {
  workspaceId: number
  title: string
  parentId?: number | null
}): Promise<SessionResource> {
  return await apiJson<SessionResource>('/api/v1/sessions', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      workspace_id: input.workspaceId,
      title: input.title,
      parent_id: input.parentId ?? null,
    }),
  })
}

export async function getSession(sessionId: number): Promise<SessionResource> {
  return await apiJson<SessionResource>(`/api/v1/sessions/${sessionId}`)
}

export async function updateSession(input: {
  sessionId: number
  title: string
  parentId?: number | null
  version?: number | null
}): Promise<SessionResource> {
  const headers: Record<string, string> = {
    'content-type': 'application/json',
  }
  if (input.version !== undefined && input.version !== null) {
    headers['if-match'] = String(input.version)
  }
  return await apiJson<SessionResource>(`/api/v1/sessions/${input.sessionId}`, {
    method: 'PUT',
    headers,
    body: JSON.stringify({
      title: input.title,
      parent_id: input.parentId ?? null,
    }),
  })
}

export async function deleteSession(input: { sessionId: number; version?: number | null }): Promise<SessionResource> {
  const headers: Record<string, string> = {}
  if (input.version !== undefined && input.version !== null) {
    headers['if-match'] = String(input.version)
  }
  return await apiJson<SessionResource>(`/api/v1/sessions/${input.sessionId}`, {
    method: 'DELETE',
    headers,
  })
}

export async function getSessionState(sessionId: number): Promise<SessionExecutionResource> {
  return await apiJson<SessionExecutionResource>(`/api/v1/sessions/${sessionId}/state`)
}

export async function getSessionGoal(sessionId: number): Promise<SessionGoalResource | null> {
  return await apiJson<SessionGoalResource | null>(`/api/v1/sessions/${sessionId}/goal`)
}

export async function setSessionGoal(input: {
  sessionId: number
  objective?: string
  status?: SessionGoalStatus
}): Promise<SessionGoalResource> {
  return await apiJson<SessionGoalResource>(`/api/v1/sessions/${input.sessionId}/goal`, {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      ...(input.objective?.trim() ? { objective: input.objective.trim() } : {}),
      ...(input.status ? { status: input.status } : {}),
    }),
  })
}

export async function completeSessionGoal(sessionId: number): Promise<SessionGoalResource> {
  return await apiJson<SessionGoalResource>(`/api/v1/sessions/${sessionId}/goal/complete`, {
    method: 'POST',
  })
}

export async function clearSessionGoal(sessionId: number): Promise<{ ok: boolean }> {
  return await apiJson<{ ok: boolean }>(`/api/v1/sessions/${sessionId}/goal`, {
    method: 'DELETE',
  })
}

export async function exportSessionJsonl(sessionId: number): Promise<string> {
  const response = await fetch(apiUrl(`/api/v1/sessions/${sessionId}/export`), {
    headers: buildActiveUiAuthHeaders(),
  })
  if (!response.ok) {
    throw new Error(`Failed to export session ${sessionId}`)
  }
  return await response.text()
}

export async function importSessionJsonl(jsonl: string): Promise<SessionExecutionResource> {
  return await apiJson<SessionExecutionResource>('/api/v1/sessions/import', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonl }),
  })
}

export async function listMessages(sessionId: number): Promise<MessageResource[]> {
  return await collectPagedItems(
    (cursor) =>
      apiJson<PaginatedResponse<MessageResource>>(
        `/api/v1/sessions/${sessionId}/messages?parts=summary&limit=100${
          cursor ? `&cursor=${encodeURIComponent(cursor)}` : ''
        }`,
      ),
    { merge: 'prepend', maxPages: 1000, resourceName: 'session messages' },
  )
}

export async function listSessionTimeline(
  sessionId: number,
  options?: {
    afterSeq?: number
    limit?: number
  },
): Promise<DomainEventRecord[]> {
  const params = new URLSearchParams()
  if (options?.afterSeq !== undefined) params.set('after_seq', String(options.afterSeq))
  if (options?.limit !== undefined) params.set('limit', String(options.limit))
  const query = params.toString()
  const response = await apiJson<PaginatedResponse<DomainEventRecord>>(
    `/api/v1/sessions/${sessionId}/events${query ? `?${query}` : ''}`,
  )
  return response.items ?? []
}

export async function getMessage(messageId: number, parts: PartLoadMode = 'summary'): Promise<MessageResource> {
  return await apiJson<MessageResource>(`/api/v1/messages/${messageId}?parts=${encodeURIComponent(parts)}`)
}

export async function listMessageParts(messageId: number, mode: PartLoadMode = 'summary'): Promise<MessagePart[]> {
  return await apiJson<MessagePart[]>(`/api/v1/messages/${messageId}/parts?mode=${encodeURIComponent(mode)}`)
}

export async function getMessagePart(partId: number): Promise<MessagePart> {
  return await apiJson<MessagePart>(`/api/v1/message-parts/${partId}`)
}

export async function listGlobalEvents(options?: {
  sinceSeqGlobal?: number
  limit?: number
  scopeKind?: 'global' | 'workspace' | 'session'
  workspaceId?: number
  sessionId?: number
  kinds?: string[]
}): Promise<DomainEventRecord[]> {
  const params = new URLSearchParams()
  if (options?.sinceSeqGlobal !== undefined) params.set('since_seq_global', String(options.sinceSeqGlobal))
  if (options?.limit !== undefined) params.set('limit', String(options.limit))
  if (options?.scopeKind && options.scopeKind !== 'global') {
    params.set('scope_kind', options.scopeKind)
  }
  if (options?.workspaceId !== undefined) params.set('workspace_id', String(options.workspaceId))
  if (options?.sessionId !== undefined) params.set('session_id', String(options.sessionId))
  if (options?.kinds?.length) {
    params.set(
      'kinds',
      options.kinds
        .map((item) => item.trim())
        .filter(Boolean)
        .join(','),
    )
  }
  const query = params.toString()
  const response = await apiJson<PaginatedResponse<DomainEventRecord>>(`/api/v1/events${query ? `?${query}` : ''}`)
  return response.items ?? []
}

export function streamSessionEvents(
  sessionId: number,
  options: {
    afterSeq?: number | null
    pollIntervalMs?: number
    onEvent: (event: DomainEventRecord) => void
    onError?: (error: Error) => void
    onOpen?: () => void
  },
): SessionEventStreamHandle {
  const controller = new AbortController()
  const decoder = new TextDecoder()
  const pollIntervalMs = Math.max(50, Math.trunc(options.pollIntervalMs ?? 250))
  let closed = false
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null
  let afterSeq = Math.max(0, Math.trunc(options.afterSeq ?? 0))

  const scheduleReconnect = (delayMs: number) => {
    if (closed || reconnectTimer) return
    reconnectTimer = setTimeout(() => {
      reconnectTimer = null
      void connect()
    }, delayMs)
  }

  const close = () => {
    closed = true
    controller.abort()
    if (reconnectTimer) {
      clearTimeout(reconnectTimer)
      reconnectTimer = null
    }
  }

  const handleEventBlock = (block: string) => {
    const parsed = parseSseEventBlock(block)
    if (!parsed.data) return

    if (parsed.event === 'error') {
      options.onError?.(new Error(parsed.data))
      return
    }

    if (parsed.event !== 'session_event') return

    const record = JSON.parse(parsed.data) as DomainEventRecord
    if (typeof record.seq_global === 'number' && Number.isFinite(record.seq_global)) {
      afterSeq = Math.max(afterSeq, record.seq_global)
    } else if (parsed.id) {
      const seq = Number(parsed.id)
      if (Number.isFinite(seq)) {
        afterSeq = Math.max(afterSeq, seq)
      }
    }
    options.onEvent(record)
  }

  const readResponseStream = async (response: Response) => {
    const reader = response.body?.getReader()
    if (!reader) {
      throw new Error('Session event stream response body is unavailable')
    }

    let buffer = ''
    while (!closed) {
      const { done, value } = await reader.read()
      buffer = normalizeSseBuffer(buffer + decoder.decode(value ?? new Uint8Array(), { stream: !done }))

      let boundary = buffer.indexOf('\n\n')
      while (boundary >= 0) {
        const block = buffer.slice(0, boundary).trim()
        buffer = buffer.slice(boundary + 2)
        if (block) {
          handleEventBlock(block)
        }
        boundary = buffer.indexOf('\n\n')
      }

      if (done) {
        const trailing = buffer.trim()
        if (trailing) {
          handleEventBlock(trailing)
        }
        return
      }
    }
  }

  const connect = async () => {
    if (closed) return

    try {
      const authHeaders = buildActiveUiAuthHeaders()
      const url = new URL(apiUrl(`/api/v1/sessions/${sessionId}/events/stream`))
      if (afterSeq > 0) {
        url.searchParams.set('after_seq', String(afterSeq))
      }
      url.searchParams.set('poll_interval_ms', String(pollIntervalMs))

      const response = await fetch(url.toString(), {
        method: 'GET',
        signal: controller.signal,
        credentials: authHeaders.authorization ? 'omit' : 'include',
        headers: {
          accept: 'text/event-stream',
          ...(authHeaders.authorization ? authHeaders : {}),
        },
      })

      if (!response.ok) {
        const bodyText = await response.text().catch(() => '')
        const extractedMessage = extractAuthRequiredMessageFromBodyText(bodyText)
        const message = extractedMessage || bodyText.trim() || `Request failed (${response.status})`
        const code = extractErrorCode(bodyText)
        const isUiAuthRequired =
          response.status === 401 &&
          (code === 'auth_required' || message.trim().toLowerCase() === 'ui authentication required')
        if (isUiAuthRequired) {
          emitAuthRequired({
            message,
            status: response.status,
            code: code || 'auth_required',
            url: url.toString(),
          })
        }
        throw new Error(message)
      }

      options.onOpen?.()
      await readResponseStream(response)

      if (!closed) {
        scheduleReconnect(250)
      }
    } catch (error) {
      if (closed || controller.signal.aborted) return
      options.onError?.(error instanceof Error ? error : new Error(String(error)))
      scheduleReconnect(1_000)
    }
  }

  void connect()

  return { close }
}

export function streamNotifications(options: {
  sinceSeqGlobal?: number | null
  scopeKind?: 'global' | 'workspace' | 'session'
  workspaceId?: number | null
  sessionId?: number | null
  kinds?: string[]
  reconnectDelayMs?: number
  onNotification: (notification: EventNotification) => void
  onError?: (error: Error) => void
  onOpen?: () => void
}): NotificationStreamHandle {
  const controller = new AbortController()
  const decoder = new TextDecoder()
  let closed = false
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null
  let sinceSeqGlobal = Math.max(0, Math.trunc(options.sinceSeqGlobal ?? 0))
  const reconnectDelayMs = Math.max(100, Math.trunc(options.reconnectDelayMs ?? 1000))

  const scheduleReconnect = (delayMs: number) => {
    if (closed || reconnectTimer) return
    reconnectTimer = setTimeout(() => {
      reconnectTimer = null
      void connect()
    }, delayMs)
  }

  const close = () => {
    closed = true
    controller.abort()
    if (reconnectTimer) {
      clearTimeout(reconnectTimer)
      reconnectTimer = null
    }
  }

  const handleEventBlock = (block: string) => {
    const parsed = parseSseEventBlock(block)
    if (!parsed.data) return

    if (parsed.event !== 'notification') return

    const notification = JSON.parse(parsed.data) as EventNotification
    if (notification.kind === 'event') {
      const seq = Number(notification.data.event.seq_global)
      if (Number.isFinite(seq)) {
        sinceSeqGlobal = Math.max(sinceSeqGlobal, seq)
      }
    }
    if (notification.kind === 'subscription_closed') {
      options.onNotification(notification)
      close()
      return
    }
    options.onNotification(notification)
  }

  const readResponseStream = async (response: Response) => {
    const reader = response.body?.getReader()
    if (!reader) {
      throw new Error('Notification stream response body is unavailable')
    }

    let buffer = ''
    while (!closed) {
      const { done, value } = await reader.read()
      buffer = normalizeSseBuffer(buffer + decoder.decode(value ?? new Uint8Array(), { stream: !done }))

      let boundary = buffer.indexOf('\n\n')
      while (boundary >= 0) {
        const block = buffer.slice(0, boundary).trim()
        buffer = buffer.slice(boundary + 2)
        if (block) {
          handleEventBlock(block)
        }
        boundary = buffer.indexOf('\n\n')
      }

      if (done) {
        const trailing = buffer.trim()
        if (trailing) {
          handleEventBlock(trailing)
        }
        return
      }
    }
  }

  const connect = async () => {
    if (closed) return

    try {
      const authHeaders = buildActiveUiAuthHeaders()
      const url = new URL(apiUrl('/api/v1/events/stream'))
      url.searchParams.set('scope_kind', options.scopeKind ?? 'global')
      if (sinceSeqGlobal > 0) {
        url.searchParams.set('since_seq_global', String(sinceSeqGlobal))
      }
      if (options.workspaceId !== null && options.workspaceId !== undefined) {
        url.searchParams.set('workspace_id', String(Math.trunc(options.workspaceId)))
      }
      if (options.sessionId !== null && options.sessionId !== undefined) {
        url.searchParams.set('session_id', String(Math.trunc(options.sessionId)))
      }
      if (options.kinds?.length) {
        url.searchParams.set(
          'kinds',
          options.kinds
            .map((value) => String(value || '').trim())
            .filter(Boolean)
            .join(','),
        )
      }

      const response = await fetch(url.toString(), {
        method: 'GET',
        signal: controller.signal,
        credentials: authHeaders.authorization ? 'omit' : 'include',
        headers: {
          accept: 'text/event-stream',
          ...(authHeaders.authorization ? authHeaders : {}),
        },
      })

      if (!response.ok) {
        const bodyText = await response.text().catch(() => '')
        const extractedMessage = extractAuthRequiredMessageFromBodyText(bodyText)
        const message = extractedMessage || bodyText.trim() || `Request failed (${response.status})`
        const code = extractErrorCode(bodyText)
        const isUiAuthRequired =
          response.status === 401 &&
          (code === 'auth_required' || message.trim().toLowerCase() === 'ui authentication required')
        if (isUiAuthRequired) {
          emitAuthRequired({
            message,
            status: response.status,
            code: code || 'auth_required',
            url: url.toString(),
          })
        }
        throw new Error(message)
      }

      options.onOpen?.()
      await readResponseStream(response)

      if (!closed) {
        scheduleReconnect(250)
      }
    } catch (error) {
      if (closed || controller.signal.aborted) return
      options.onError?.(error instanceof Error ? error : new Error(String(error)))
      scheduleReconnect(reconnectDelayMs)
    }
  }

  void connect()

  return { close }
}

export function streamPluginToolRegistryChanges(options: {
  sinceSeqGlobal?: number | null
  reconnectDelayMs?: number
  onEvent: (event: ToolRegistryChangedEvent) => void
  onLagged?: (skipped: number) => void
  onError?: (error: Error) => void
  onOpen?: () => void
}): NotificationStreamHandle {
  return streamNotifications({
    sinceSeqGlobal: options.sinceSeqGlobal,
    scopeKind: 'global',
    kinds: ['plugin_tool_registry_changed'],
    reconnectDelayMs: options.reconnectDelayMs,
    onNotification: (notification) => {
      if (notification.kind === 'lagged') {
        options.onLagged?.(notification.data.skipped)
        return
      }
      if (notification.kind !== 'event') return
      if (notification.data.event.kind !== 'plugin_tool_registry_changed') return
      options.onEvent(notification.data.event.payload as unknown as ToolRegistryChangedEvent)
    },
    onError: options.onError,
    onOpen: options.onOpen,
  })
}

export async function forkSession(input: {
  sessionId: number
  atMessageId?: number
  title?: string
}): Promise<SessionExecutionResource> {
  return await apiJson<SessionExecutionResource>(`/api/v1/sessions/${input.sessionId}/fork`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      ...(input.atMessageId !== undefined ? { at_message_id: input.atMessageId } : {}),
      ...(input.title?.trim() ? { title: input.title.trim() } : {}),
    }),
  })
}

type SessionRunOptionsInput = {
  providerId?: string
  adapterId?: string
  modelId?: string
  thinkingMode?: string
  speedMode?: string
  verbosity?: string
  parallelToolCalls?: boolean
  agentProfile?: string
}

function buildSessionRunOptionsBody(input: SessionRunOptionsInput): Record<string, unknown> {
  const body: Record<string, unknown> = {}
  const providerId = input.providerId?.trim() || ''
  const adapterId = input.adapterId?.trim() || ''
  const modelId = input.modelId?.trim() || ''
  const agentProfile = input.agentProfile?.trim() || ''

  if (providerId && modelId) {
    body.model = {
      provider_id: providerId,
      ...(adapterId ? { adapter_id: adapterId } : {}),
      model_id: modelId,
    }
  }
  if (agentProfile) {
    body.agent_profile = agentProfile
  }
  if (providerId && modelId && input.thinkingMode?.trim()) {
    body.thinking_mode = input.thinkingMode.trim()
  }
  if (providerId && modelId && input.speedMode?.trim()) {
    body.speed_mode = input.speedMode.trim()
  }
  if (providerId && modelId && input.verbosity?.trim()) {
    body.verbosity = input.verbosity.trim().toLowerCase()
  }
  if (providerId && modelId && input.parallelToolCalls !== undefined) {
    body.parallel_tool_calls = input.parallelToolCalls
  }

  return body
}

export async function continueSession(input: {
  sessionId: number
  providerId?: string
  adapterId?: string
  modelId?: string
  thinkingMode?: string
  speedMode?: string
  verbosity?: string
  parallelToolCalls?: boolean
  agentProfile?: string
}): Promise<SessionExecutionResource> {
  return await apiJson<SessionExecutionResource>(`/api/v1/sessions/${input.sessionId}/continue`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(buildSessionRunOptionsBody(input)),
  })
}

export async function cancelSessionRun(sessionId: number): Promise<{ ok: boolean }> {
  return await apiJson<{ ok: boolean }>(`/api/v1/sessions/${sessionId}/cancel`, {
    method: 'POST',
  })
}

export async function getSessionTree(rootId: number): Promise<SessionTreeResource[]> {
  return await apiJson<SessionTreeResource[]>(`/api/v1/sessions/tree/${rootId}`)
}

export async function listRewindCheckpoints(sessionId: number): Promise<RewindCheckpointResource[]> {
  return await apiJson<RewindCheckpointResource[]>(`/api/v1/sessions/${sessionId}/rewind-checkpoints`)
}

export async function submitTurn(input: {
  sessionId: number
  text: string
  providerId?: string
  adapterId?: string
  modelId?: string
  thinkingMode?: string
  speedMode?: string
  verbosity?: string
  parallelToolCalls?: boolean
  agentProfile?: string
}): Promise<SessionExecutionResource> {
  const body: Record<string, unknown> = {
    ...buildSessionRunOptionsBody(input),
    parts: [
      {
        type: 'text',
        text: input.text,
      },
    ],
  }

  return await apiJson<SessionExecutionResource>(`/api/v1/sessions/${input.sessionId}/messages`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

export async function replyPermission(input: {
  sessionId: number
  requestId: string
  kind: 'allow_once' | 'allow_always' | 'deny_once' | 'deny_always'
  reason?: string
  scope?: 'session' | 'workspace' | 'global'
  agentProfile?: string
}): Promise<SessionExecutionResource> {
  return await apiJson<SessionExecutionResource>(`/api/v1/sessions/${input.sessionId}/permission-replies`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      ...buildSessionRunOptionsBody(input),
      reply: {
        request_id: input.requestId,
        kind: input.kind,
        ...(input.reason ? { reason: input.reason } : {}),
        ...(input.scope ? { scope: input.scope } : {}),
      },
    }),
  })
}

export async function replyUserInput(input: {
  sessionId: number
  requestId: string
  answers: Record<string, string[]>
  agentProfile?: string
}): Promise<SessionExecutionResource> {
  return await apiJson<SessionExecutionResource>(`/api/v1/sessions/${input.sessionId}/user-input-replies`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      ...buildSessionRunOptionsBody(input),
      reply: {
        request_id: input.requestId,
        kind: 'submit',
        answers: input.answers,
      },
    }),
  })
}

export async function cancelUserInput(input: {
  sessionId: number
  requestId: string
  reason?: string
  agentProfile?: string
}): Promise<SessionExecutionResource> {
  return await apiJson<SessionExecutionResource>(`/api/v1/sessions/${input.sessionId}/user-input-replies`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      ...buildSessionRunOptionsBody(input),
      reply: {
        request_id: input.requestId,
        kind: 'cancel',
        ...(input.reason ? { reason: input.reason } : {}),
      },
    }),
  })
}

export async function rewindSession(input: {
  sessionId: number
  messageId: number
}): Promise<SessionExecutionResource> {
  return await apiJson<SessionExecutionResource>(`/api/v1/sessions/${input.sessionId}/rewind`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      message_id: input.messageId,
    }),
  })
}
