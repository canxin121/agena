import { apiJson } from '@/lib/api'
import type { JsonObject, JsonValue } from '@/types/json'

export type RuntimeSettingsSource = 'effective' | 'file'
export type RuntimeSettingsLayer = 'global' | 'workspace'

export type RuntimeSettingReadResponse = {
  config_path?: string
  config_found?: boolean
  source?: RuntimeSettingsSource
  path?: string | null
  value?: JsonValue
}

export type RuntimeSettingEditResponse = {
  config_path?: string
  config_found?: boolean
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
}

function optionsBody(options?: RuntimeSettingsEditOptions): Required<RuntimeSettingsEditOptions> {
  return {
    dry_run: options?.dry_run ?? false,
    validate: options?.validate ?? true,
    reload: options?.reload ?? true,
  }
}

function settingsQuery(path: string | undefined, source: RuntimeSettingsSource): string {
  const params = new URLSearchParams({ source })
  if (path) params.set('path', path)
  return `/api/v1/settings?${params.toString()}`
}

function layerQuery(path: string | undefined): string {
  const params = new URLSearchParams()
  if (path) params.set('path', path)
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
): Promise<RuntimeSettingReadResponse> {
  const query = layerQuery(path)
  return await apiJson<RuntimeSettingReadResponse>(`/api/v1/settings/layers/${layer}${query ? `?${query}` : ''}`)
}

export async function readRuntimeSettingSources(path: string): Promise<RuntimeSettingsReadBundle> {
  const [effective, file, global, workspace] = await Promise.all([
    getRuntimeSetting(path, 'effective'),
    getRuntimeSetting(path, 'file'),
    getRuntimeSettingLayer('global', path),
    getRuntimeSettingLayer('workspace', path),
  ])
  return { effective, file, global, workspace }
}

export async function setRuntimeSetting(
  path: string,
  value: JsonValue,
  options?: RuntimeSettingsEditOptions,
  layer?: RuntimeSettingsLayer,
): Promise<RuntimeSettingEditResponse> {
  const body = JSON.stringify({ path, value, ...optionsBody(options) })
  return await apiJson<RuntimeSettingEditResponse>(layer ? `/api/v1/settings/layers/${layer}` : '/api/v1/settings', {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body,
  })
}

export async function patchRuntimeSettings(
  path: string | undefined,
  changes: JsonObject,
  options?: RuntimeSettingsEditOptions,
): Promise<RuntimeSettingEditResponse> {
  return await apiJson<RuntimeSettingEditResponse>('/api/v1/settings', {
    method: 'PATCH',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      ...(path ? { path } : {}),
      changes,
      ...optionsBody(options),
    }),
  })
}

export async function deleteRuntimeSetting(
  path: string,
  options?: RuntimeSettingsEditOptions,
  layer?: RuntimeSettingsLayer,
): Promise<RuntimeSettingEditResponse> {
  const editOptions = optionsBody(options)
  const params = new URLSearchParams({
    path,
    dry_run: String(editOptions.dry_run),
    validate: String(editOptions.validate),
    reload: String(editOptions.reload),
  })
  return await apiJson<RuntimeSettingEditResponse>(
    `${layer ? `/api/v1/settings/layers/${layer}` : '/api/v1/settings'}?${params.toString()}`,
    { method: 'DELETE' },
  )
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
