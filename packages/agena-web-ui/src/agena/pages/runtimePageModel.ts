import type {
  AuthProvider,
  PluginLogEntry,
  PluginStatus,
  ProviderModel,
  RuntimeStatus,
  SessionExecutionResource,
} from '@/agena/lib/agenaApi'
import {
  formatSessionExecutionModelLabel,
  pendingPermissionRequests,
  pendingUserInputRequests,
} from '@/agena/lib/agenaApi'

export type OperatorCard = {
  label: string
  value: string
}

export type SessionExecutionFact = {
  label: string
  value: string
  mono?: boolean
}

export function buildOperatorCards(runtime: RuntimeStatus | null): OperatorCard[] {
  if (!runtime) return []
  return [
    { label: 'Generation', value: String(runtime.generation) },
    { label: 'Tool Registry', value: String(runtime.operator.ui?.tool_registry_generation ?? 0) },
    { label: 'Providers', value: String(runtime.provider_ids.length) },
    { label: 'Plugins', value: String(runtime.plugin_count) },
    { label: 'Agent', value: runtime.operator.agent_id },
    { label: 'MCP Servers', value: String(runtime.operator.mcp.server_count) },
    { label: 'LSP Servers', value: String(runtime.operator.lsp.server_count) },
    { label: 'Skills', value: String(runtime.operator.skills.skill_count) },
  ]
}

export function buildRuntimeSnapshotFacts(runtime: RuntimeStatus | null): SessionExecutionFact[] {
  if (!runtime) return []
  return [
    { label: 'Generation', value: String(runtime.generation) },
    { label: 'Loaded At', value: runtime.loaded_at },
    { label: 'Workspace Root', value: runtime.workspace_root, mono: true },
    { label: 'Config Path', value: runtime.config_path, mono: true },
    { label: 'Config Found', value: runtime.config_found ? 'yes' : 'no' },
    { label: 'Auth Store', value: runtime.auth_store_path, mono: true },
    { label: 'Tool Registry Generation', value: String(runtime.operator.ui?.tool_registry_generation ?? 0) },
    {
      label: 'Tool Registry Last Event',
      value: runtime.operator.ui?.tool_registry_last_event?.model_name || 'n/a',
      mono: Boolean(runtime.operator.ui?.tool_registry_last_event?.model_name),
    },
    { label: 'Providers', value: runtime.provider_ids.join(', ') || 'none' },
    { label: 'Session Runtime', value: runtime.session_runtime_available ? 'enabled' : 'disabled' },
    { label: 'Automation', value: runtime.automation.enabled ? 'enabled' : 'disabled' },
    { label: 'Scheduled Jobs', value: String(runtime.automation.job_count) },
  ]
}

export function buildSessionCacheFacts(runtime: RuntimeStatus | null): SessionExecutionFact[] {
  const cache = runtime?.session_cache
  if (!cache) return []
  return [
    { label: 'Entries', value: String(cache.entry_count) },
    { label: 'Total Bytes', value: String(cache.total_bytes) },
    { label: 'Max Bytes', value: String(cache.max_bytes) },
    { label: 'Hits / Misses', value: `${cache.hits} / ${cache.misses}` },
    { label: 'Inserts / Evictions', value: `${cache.inserts} / ${cache.evictions}` },
    { label: 'TTL', value: `${cache.ttl_secs}s` },
    { label: 'Max Sessions', value: String(cache.max_sessions) },
  ]
}

export function buildAuthProviderFacts(provider: AuthProvider): SessionExecutionFact[] {
  return [
    { label: 'Configured', value: provider.configured ? 'yes' : 'no' },
    { label: 'Credential', value: provider.credential_present ? 'present' : 'missing' },
    { label: 'Credential Type', value: provider.credential_type || 'unknown' },
    { label: 'Credential Issuer', value: provider.credential_issuer || 'n/a' },
    { label: 'Preview', value: provider.key_preview || 'n/a', mono: Boolean(provider.key_preview) },
    { label: 'Expires At', value: provider.expires_at || 'n/a' },
    { label: 'Expired', value: provider.expired == null ? 'unknown' : provider.expired ? 'yes' : 'no' },
    { label: 'Account', value: provider.account_id || 'n/a' },
    { label: 'Username', value: provider.username || 'n/a' },
    { label: 'Display Name', value: provider.display_name || 'n/a' },
    { label: 'Email', value: provider.email || 'n/a' },
    { label: 'Enterprise URL', value: provider.enterprise_url || 'n/a', mono: Boolean(provider.enterprise_url) },
    { label: 'API Key Write', value: provider.api_key_write_supported ? 'yes' : 'no' },
    { label: 'Refresh', value: provider.refresh_supported ? 'yes' : 'no' },
    { label: 'Browser Login', value: provider.browser_login_kind || 'n/a' },
    { label: 'Device Login', value: provider.device_login_kind || 'n/a' },
  ]
}

export function mergePluginLogs(current: PluginLogEntry[], incoming: PluginLogEntry[]): PluginLogEntry[] {
  const merged = new Map<number, PluginLogEntry>()
  for (const entry of current) merged.set(entry.seq, entry)
  for (const entry of incoming) merged.set(entry.seq, entry)
  return [...merged.values()].sort((left, right) => left.seq - right.seq)
}

export function pluginLogCursor(entries: PluginLogEntry[]): number | null {
  return entries.length ? (entries[entries.length - 1]?.seq ?? null) : null
}

export function formatProviderModel(model: ProviderModel): string {
  return model.display_name?.trim() || model.id
}

export function buildExecutionFacts(execution: SessionExecutionResource | null): SessionExecutionFact[] {
  if (!execution) return []
  const context = execution.execution
  return [
    { label: 'Session', value: `#${execution.session.id} · ${execution.session.title}` },
    {
      label: 'Execution',
      value: execution.active_execution
        ? `${execution.active_execution.phase} · ${execution.active_execution.execution_id}`
        : 'idle',
    },
    { label: 'Workflow', value: execution.workflow_state },
    { label: 'Latest Event', value: execution.latest_event_seq == null ? 'n/a' : String(execution.latest_event_seq) },
    { label: 'Agent', value: context.agent_id },
    { label: 'Execution Access', value: context.execution_access },
    { label: 'Task', value: context.task_id || 'n/a' },
    {
      label: 'Model',
      value: formatSessionExecutionModelLabel(context) || 'n/a',
    },
    {
      label: 'Workspace Root',
      value: context.effective_workspace_root || 'n/a',
      mono: Boolean(context.effective_workspace_root),
    },
    { label: 'Pending Permissions', value: String(pendingPermissionRequests(execution).length) },
    { label: 'Pending User Input', value: String(pendingUserInputRequests(execution).length) },
  ]
}

export function pickNextPluginId(currentPluginId: string, plugins: PluginStatus[]): string {
  if (currentPluginId && plugins.some((plugin) => plugin.plugin_id === currentPluginId)) {
    return currentPluginId
  }
  return plugins[0]?.plugin_id || ''
}
