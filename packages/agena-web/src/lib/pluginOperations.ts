import { apiJson } from './api'
import type { JsonValue } from '../types/json'

export type JsonRecord = Record<string, JsonValue>

export type PluginSettingsConstraints = {
  minimum?: number | null
  maximum?: number | null
  exclusive_minimum?: number | null
  exclusive_maximum?: number | null
  multiple_of?: number | null
  min_length?: number | null
  max_length?: number | null
  pattern?: string | null
  min_items?: number | null
  max_items?: number | null
  max_entries?: number | null
}

export function clonePluginJson<T extends JsonValue>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

export function pluginJsonRecord(value: JsonValue): JsonRecord | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  return value as JsonRecord
}

export function pluginCommandInvocationBody(input: {
  command: Pick<PluginCommandCatalogItem, 'slash'>
  sessionId: number | null
  rawArgs: string
}) {
  return {
    input: {},
    session_id: input.sessionId,
    slash: input.command.slash || null,
    raw: String(input.rawArgs || '').trim(),
  }
}

export type PluginSettingsOption = {
  id: string
  title: string
  description?: string
  value: JsonValue
}

export type PluginSettingsVariant = {
  id: string
  title: string
  description?: string
  tag: JsonValue
  fields?: PluginSettingsNode[]
}

export type PluginSettingsNode = {
  id: string
  path: string
  title: string
  description?: string
  required?: boolean
  default?: JsonValue
  constraints?: PluginSettingsConstraints
  sensitive?: boolean
  secret?: boolean
  kind:
    | 'boolean'
    | 'text'
    | 'secret_reference'
    | 'integer'
    | 'number'
    | 'choice'
    | 'multi_choice'
    | 'path'
    | 'url'
    | 'duration'
    | 'object'
    | 'list'
    | 'record'
    | 'tagged_variant'
    | 'json'
  options?: PluginSettingsOption[]
  path_kind?: 'any' | 'file' | 'directory'
  fields?: PluginSettingsNode[]
  item?: PluginSettingsNode
  value?: PluginSettingsNode
  discriminator?: string
  variants?: PluginSettingsVariant[]
  max_bytes?: number
  max_depth?: number
}

export type PluginSettingsContract = {
  version: number
  root: PluginSettingsNode
}

export type PluginSettingsDiagnostic = { path: string; message: string }

export type PluginSettingsState = {
  plugin_id: string
  contract: PluginSettingsContract
  defaults: JsonValue
  configured: JsonValue
  effective: JsonValue
  diagnostics?: PluginSettingsDiagnostic[]
}

export type PluginSettingsUpdateResponse = {
  settings: PluginSettingsState
  reload_required: boolean
}

export type PluginCommandDocs = {
  summary?: string | null
  summary_key?: string | null
  help?: string | null
  help_key?: string | null
  usage?: string | null
  examples?: string[]
}

export type PluginCommandCatalogItem = {
  plugin_id: string
  accepts_empty_input: boolean
  default_input: JsonValue
  id: string
  title: string
  group: string
  category?: string | null
  slash?: string | null
  aliases?: string[]
  docs?: PluginCommandDocs
  input: PluginSettingsContract
  discoverability?: {
    catalog?: boolean
    palette?: boolean
    slash?: boolean
  }
  target:
    | { kind: 'client'; action: string }
    | { kind: 'method'; handler: string }
    | { kind: 'tool'; tool: string }
}

export type PluginCommandStatus = 'succeeded' | 'failed' | 'cancelled' | 'unavailable' | 'permission_required'

export type PluginCommandDiagnostic = {
  code: string
  message: string
  path?: string | null
  sensitive?: boolean
}

export type PluginHostEffect =
  | { kind: 'navigate'; path: string }
  | { kind: 'open_url'; url: string }
  | { kind: 'insert_prompt'; prompt: string }
  | { kind: 'refresh_plugin_surface'; plugin_id: string }

export type PluginCommandResult = {
  status: PluginCommandStatus
  title: string
  summary: string
  detail?: string | null
  output?: JsonValue
  diagnostics?: PluginCommandDiagnostic[]
  retryable?: boolean
  effects?: PluginHostEffect[]
}

export async function executePluginSlashCommand(input: {
  command: Pick<PluginCommandCatalogItem, 'plugin_id' | 'id' | 'slash'>
  sessionId: number | null
  rawArgs: string
}): Promise<PluginCommandResult> {
  const response = await apiJson<{ result: PluginCommandResult }>(
    `/api/v1/plugins/${encodeURIComponent(input.command.plugin_id)}/commands/${encodeURIComponent(input.command.id)}/invoke`,
    {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(pluginCommandInvocationBody(input)),
    },
  )
  if (!response?.result) throw new Error('The server omitted the plugin command result.')
  return response.result
}
