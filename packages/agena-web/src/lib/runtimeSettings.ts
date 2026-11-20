import { apiJson } from '@/lib/api'
import { notifyModelConfigurationChanged } from './modelConfigurationApi'
import { captureResourceObservation, invalidateResourcePrefix } from './resourceSync'
import { readBatchedRuntimeSettingSources, retireRuntimeSettingReads } from './runtimeSettingReads'
import type { JsonObject, JsonValue } from '@/types/json'

export type RuntimeSettingsSource = 'effective' | 'file'
export type RuntimeSettingsLayer = 'global' | 'workspace'

export type RuntimeSettingReadResponse = {
  config_path?: string
  config_found?: boolean
  revision?: string
  source?: RuntimeSettingsSource
  path?: string | null
  workspace_id?: number | null
  active_workspace?: boolean
  value?: JsonValue
  /** Authoritative local presence, including explicit nulls in opaque UI preferences. */
  has_override?: boolean
  /** The actual local key that owns this value; an ancestor can override a whole group. */
  override_path?: string
  /** An opaque ancestor must be reset before this leaf can be written. */
  requires_group_reset?: boolean
  /** Program defaults for this path. Explicit null is a known empty default. */
  default_value?: JsonValue
  /** Schema-resolved value through this layer, excluding later layers and runtime overrides. */
  layer_value?: JsonValue
  /** Actual parent value before this layer is applied; absent for the global layer. */
  inherited_value?: JsonValue
}

export type RuntimeSettingEditResponse = {
  config_path?: string
  config_found?: boolean
  before_revision?: string
  after_revision?: string
  operation?: string
  path?: string | null
  dry_run?: boolean
  changed?: boolean
  created?: boolean
  deleted?: boolean
  validated?: boolean
  reload_requested?: boolean
  reload_required?: boolean
  reload?: {
    previous_generation?: number
    generation?: number
    loaded_at?: string
  } | null
  previous?: JsonValue
  current?: JsonValue
}

export type RuntimeSettingsReadBundle = {
  effective: RuntimeSettingReadResponse
  file: RuntimeSettingReadResponse
  global: RuntimeSettingReadResponse
  workspace: RuntimeSettingReadResponse
}

export type RuntimeSettingsEditOptions = {
  dry_run?: boolean
  validate?: boolean
  reload?: boolean
  expected_revision?: string
}

type RuntimeSettingsLayerEditOptions = RuntimeSettingsEditOptions & {
  workspace_id?: string
}

function optionsBody(options?: RuntimeSettingsEditOptions) {
  return {
    dry_run: options?.dry_run ?? false,
    validate: options?.validate ?? true,
    reload: options?.reload ?? true,
    ...(options?.expected_revision !== undefined ? { expected_revision: options.expected_revision } : {}),
  }
}

function reconcileModelSettings(
  scope: number,
  path: string | undefined,
  result: RuntimeSettingEditResponse,
  options?: RuntimeSettingsEditOptions,
  changes?: JsonObject,
) {
  if (options?.dry_run || result.dry_run || result.changed === false || options?.reload === false) return
  if (scope !== captureResourceObservation('sessions').scope) return
  const settingPath = path || ''
  const profilePath = (key: string) =>
    key === 'profiles' || key.startsWith('profiles.') || key === 'default_profile' || key === 'default_subagent_profile'
  const namedPermissionPath = (key: string) =>
    key === 'permission_profile' || key === 'access_control' || key.startsWith('access_control.')
  if (profilePath(settingPath) || (!settingPath && (!changes || Object.keys(changes).some(profilePath)))) {
    // Profiles can alter the effective model and permissions without changing
    // session metadata. Retire cached projections even when their version is unchanged.
    invalidateResourcePrefix('session:', 'mutation')
    notifyModelConfigurationChanged(scope, false)
  } else if (
    namedPermissionPath(settingPath) ||
    (!settingPath && changes && Object.keys(changes).some(namedPermissionPath))
  ) {
    invalidateResourcePrefix('session:', 'mutation')
  }
  const defaultOnly =
    settingPath === 'providers.default_selection' ||
    settingPath.startsWith('providers.default_selection.') ||
    (settingPath === 'providers' && !!changes && Object.keys(changes).every((key) => key === 'default_selection'))
  const providersChanged =
    defaultOnly ||
    settingPath === 'providers' ||
    settingPath.startsWith('providers.') ||
    (!settingPath &&
      (!changes || Object.keys(changes).some((key) => key === 'providers' || key.startsWith('providers.'))))
  if (providersChanged) notifyModelConfigurationChanged(scope, !defaultOnly)
}

function settingsQuery(path: string | undefined, source: RuntimeSettingsSource): string {
  const params = new URLSearchParams({ source })
  if (path) params.set('path', path)
  return `/api/v1/settings?${params.toString()}`
}

function layerQuery(path?: string, workspaceId?: string): string {
  const params = new URLSearchParams()
  if (path) params.set('path', path)
  if (workspaceId) params.set('workspace_id', workspaceId)
  return params.toString()
}

export async function getRuntimeSetting(
  path: string,
  source: RuntimeSettingsSource = 'effective',
): Promise<RuntimeSettingReadResponse> {
  return await apiJson<RuntimeSettingReadResponse>(settingsQuery(path, source))
}

export async function getRuntimeSettingLayer(
  layer: RuntimeSettingsLayer,
  path: string,
  options?: { workspaceId?: string; signal?: AbortSignal },
): Promise<RuntimeSettingReadResponse> {
  const query = layerQuery(path, options?.workspaceId)
  return await apiJson<RuntimeSettingReadResponse>(`/api/v1/settings/layers/${layer}${query ? `?${query}` : ''}`, {
    signal: options?.signal,
  })
}

export function readRuntimeSettingSources(path: string, signal?: AbortSignal): Promise<RuntimeSettingsReadBundle> {
  return readBatchedRuntimeSettingSources(path, signal)
}

const settingChangeListeners = new Set<() => void>()
export function subscribeRuntimeSettingsChanges(listener: () => void): () => void {
  settingChangeListeners.add(listener)
  return () => settingChangeListeners.delete(listener)
}
function notifySettingsChanged(
  scope: number,
  result: RuntimeSettingEditResponse,
  options?: RuntimeSettingsEditOptions,
) {
  if (options?.dry_run || result.dry_run || result.changed === false) return
  if (scope !== captureResourceObservation('sessions').scope) return
  for (const listener of settingChangeListeners) {
    try {
      listener()
    } catch {
      /* A view error must not invalidate a successful write. */
    }
  }
}

async function editRuntimeSettings(url: string, init: RequestInit): Promise<RuntimeSettingEditResponse> {
  retireRuntimeSettingReads()
  try {
    return await apiJson<RuntimeSettingEditResponse>(url, init)
  } finally {
    retireRuntimeSettingReads()
  }
}

function settingsMutationPath(layer?: RuntimeSettingsLayer, workspaceId?: string) {
  if (workspaceId && layer !== 'workspace') throw new Error('A workspace ID requires the workspace settings layer.')
  return layer ? `/api/v1/settings/layers/${layer}` : '/api/v1/settings'
}

export async function setRuntimeSetting(
  path: string,
  value: JsonValue,
  options?: RuntimeSettingsLayerEditOptions,
  layer?: RuntimeSettingsLayer,
): Promise<RuntimeSettingEditResponse> {
  const scope = captureResourceObservation('sessions').scope
  const body = JSON.stringify({ path, value, ...optionsBody(options) })
  const target = settingsMutationPath(layer, options?.workspace_id)
  const query = layerQuery(undefined, options?.workspace_id)
  const result = await editRuntimeSettings(`${target}${query ? `?${query}` : ''}`, {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body,
  })
  reconcileModelSettings(scope, path, result, options)
  notifySettingsChanged(scope, result, options)
  return result
}

export async function patchRuntimeSettings(
  path: string | undefined,
  changes: JsonObject,
  options?: RuntimeSettingsEditOptions,
): Promise<RuntimeSettingEditResponse> {
  const scope = captureResourceObservation('sessions').scope
  const result = await editRuntimeSettings('/api/v1/settings', {
    method: 'PATCH',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      ...(path ? { path } : {}),
      changes,
      ...optionsBody(options),
    }),
  })
  reconcileModelSettings(scope, path, result, options, changes)
  notifySettingsChanged(scope, result, options)
  return result
}

export async function deleteRuntimeSetting(
  path: string,
  options?: RuntimeSettingsLayerEditOptions,
  layer?: RuntimeSettingsLayer,
): Promise<RuntimeSettingEditResponse> {
  const scope = captureResourceObservation('sessions').scope
  const editOptions = optionsBody(options)
  const params = new URLSearchParams({
    path,
    dry_run: String(editOptions.dry_run),
    validate: String(editOptions.validate),
    reload: String(editOptions.reload),
  })
  if (editOptions.expected_revision !== undefined) params.set('expected_revision', editOptions.expected_revision)
  const target = settingsMutationPath(layer, options?.workspace_id)
  if (options?.workspace_id) params.set('workspace_id', options.workspace_id)
  const result = await editRuntimeSettings(`${target}?${params.toString()}`, { method: 'DELETE' })
  reconcileModelSettings(scope, path, result, options)
  notifySettingsChanged(scope, result, options)
  return result
}

export async function getResolvedRuntimeConfig(): Promise<JsonValue> {
  return await apiJson<JsonValue>('/api/v1/config/resolved')
}

export async function validateRuntimeSettings(path?: string): Promise<JsonValue> {
  return await apiJson<JsonValue>('/api/v1/settings/validate', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(path ? { path } : {}),
  })
}

export function jsonPathForKey(parent: string, key: string): string {
  const normalized = String(key || '')
  // Dotted tool ids and plugin ids must remain one JSON path segment. This is
  // the same quoted-path form used by the runtime and TUI.
  const segment = /^[A-Za-z0-9_-]+$/.test(normalized)
    ? normalized
    : `"${normalized.replaceAll('\\', '\\\\').replaceAll('"', '\\"')}"`
  return parent ? `${parent}.${segment}` : segment
}

export function settingValue<T>(response: RuntimeSettingReadResponse | null | undefined, fallback: T): T {
  return response && response.value !== undefined && response.value !== null ? (response.value as T) : fallback
}

export function hasPersistedSetting(response: RuntimeSettingReadResponse | null | undefined): boolean {
  if (typeof response?.has_override === 'boolean') return response.has_override
  return Boolean(response?.config_found && response?.value !== undefined && response?.value !== null)
}

export function displayJsonValue(value: JsonValue): string {
  if (value === undefined || value === null) return '—'
  if (typeof value === 'string') return value
  try {
    return JSON.stringify(value)
  } catch {
    return String(value)
  }
}
