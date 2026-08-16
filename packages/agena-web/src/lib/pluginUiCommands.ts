import { apiJson } from './api'
import type { JsonObject, JsonValue } from '../types/json'

export type PluginUiAction = {
  kind: string
  tool?: string
  command?: string
  input?: JsonValue
  tab?: string | null
  url?: string
  prompt?: string
  submit_output_as_prompt?: boolean
}

export type PluginSlashCommand = {
  pluginId: string
  id: string
  slash: string
  inputSchema?: JsonValue
  action: PluginUiAction
}

export type PluginCommandEffect =
  | { kind: 'none' }
  | { kind: 'message'; text: string }
  | { kind: 'submit_prompt'; prompt: string }
  | { kind: 'open_plugin_workbench'; pluginId: string; tab: string }
  | { kind: 'open_url'; url: string }

function asRecord(value: JsonValue): JsonObject | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  return value as JsonObject
}

function mergeInput(base: JsonValue, overlay: JsonValue): JsonValue {
  const baseRecord = asRecord(base)
  const overlayRecord = asRecord(overlay)
  if (baseRecord && overlayRecord) return { ...baseRecord, ...overlayRecord }
  if (overlay !== undefined && overlay !== null) return overlay
  if (base !== undefined && base !== null) return base
  return {}
}

function parsedLiteral(raw: string, schema?: JsonValue): JsonValue {
  let parsed: JsonValue
  try {
    parsed = JSON.parse(raw)
  } catch {
    parsed = raw
  }
  const schemaRecord = asRecord(schema)
  const expected = schemaRecord?.type
  const matches = (kind: string) => {
    if (kind === 'string') return typeof parsed === 'string'
    if (kind === 'integer') return typeof parsed === 'number' && Number.isInteger(parsed)
    if (kind === 'number') return typeof parsed === 'number'
    if (kind === 'boolean') return typeof parsed === 'boolean'
    if (kind === 'object') return Boolean(asRecord(parsed))
    if (kind === 'array') return Array.isArray(parsed)
    if (kind === 'null') return parsed === null
    return true
  }
  const accepted =
    typeof expected === 'string'
      ? matches(expected)
      : Array.isArray(expected)
        ? expected.some((kind) => typeof kind === 'string' && matches(kind))
        : true
  return accepted ? parsed : raw
}

export function parsePluginCommandInput(command: PluginSlashCommand, rawArgs: string): JsonValue {
  const raw = String(rawArgs || '').trim()
  if (!raw) return {}
  const schema = asRecord(command.inputSchema)
  if (!schema) return { args: raw }
  const schemaType = typeof schema.type === 'string' ? schema.type : ''
  const properties = asRecord(schema.properties)

  try {
    const parsed = JSON.parse(raw) as JsonValue
    if (schemaType !== 'object' || asRecord(parsed)) return parsed
    const names = properties ? Object.keys(properties) : []
    if (names.length === 1) return { [names[0]!]: parsed }
  } catch {
    // Fall through to the schema-aware shorthand parser.
  }

  if (schemaType === 'object' && properties) {
    const names = Object.keys(properties)
    if (names.length === 1) {
      const name = names[0]!
      return { [name]: parsedLiteral(raw, properties[name]) }
    }

    const aliases = new Map<string, string>()
    for (const [name, propertySchema] of Object.entries(properties)) {
      const property = asRecord(propertySchema)
      const values = property?.['x-agena-aliases']
      if (!Array.isArray(values)) continue
      for (const alias of values) {
        if (typeof alias === 'string' && alias.trim()) aliases.set(alias.trim(), name)
      }
    }
    const output: JsonObject = {}
    let valid = true
    for (const token of raw.split(/\s+/)) {
      const separator = token.indexOf('=')
      if (separator <= 0) {
        valid = false
        break
      }
      const rawName = token.slice(0, separator)
      const name = Object.prototype.hasOwnProperty.call(properties, rawName) ? rawName : aliases.get(rawName)
      if (!name) {
        valid = false
        break
      }
      output[name] = parsedLiteral(token.slice(separator + 1), properties[name])
    }
    if (valid && Object.keys(output).length > 0) return output
  }

  const literal = parsedLiteral(raw, schema)
  if (typeof literal !== 'string' || schemaType === 'string') return literal
  return { args: raw }
}

function effectFromClientAction(pluginId: string, action: PluginUiAction): PluginCommandEffect | null {
  if (action.kind === 'none') return { kind: 'none' }
  if (action.kind === 'submit_prompt') return { kind: 'submit_prompt', prompt: String(action.prompt || '') }
  if (action.kind === 'open_url') return { kind: 'open_url', url: String(action.url || '') }
  if (action.kind === 'open_plugin_workbench') {
    return {
      kind: 'open_plugin_workbench',
      pluginId,
      tab: String(action.tab || ''),
    }
  }
  return null
}

export async function executePluginSlashCommand(input: {
  command: PluginSlashCommand
  catalog: PluginSlashCommand[]
  sessionId: number
  rawArgs: string
}): Promise<PluginCommandEffect> {
  const { command, catalog, sessionId } = input
  const rawArgs = String(input.rawArgs || '').trim()
  let action = command.action
  let actionInput = parsePluginCommandInput(command, rawArgs)

  for (let depth = 0; depth <= 8; depth += 1) {
    const clientEffect = effectFromClientAction(command.pluginId, action)
    if (clientEffect) return clientEffect

    if (action.kind === 'invoke_tool') {
      const tool = String(action.tool || '').trim()
      if (!tool) throw new Error('The plugin command did not provide a tool name.')
      const response = await apiJson<JsonValue>('/api/v1/plugins/ui/invoke-tool', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          plugin_id: command.pluginId,
          tool,
          input: asRecord(mergeInput(action.input, actionInput)) || {},
          session_id: sessionId,
        }),
      })
      const result = asRecord(response)
      const output = typeof result?.output_text === 'string' ? result.output_text.trim() : ''
      if (!output) return { kind: 'none' }
      return action.submit_output_as_prompt === true
        ? { kind: 'submit_prompt', prompt: output }
        : { kind: 'message', text: output }
    }

    if (action.kind !== 'invoke_command') {
      throw new Error(`Unsupported plugin command action: ${action.kind || 'unknown'}`)
    }

    const commandId = String(action.command || '').trim()
    if (!commandId) throw new Error('The plugin command did not provide a command id.')
    const response = await apiJson<JsonValue>(
      `/api/v1/plugins/${encodeURIComponent(command.pluginId)}/commands/${encodeURIComponent(commandId)}`,
      {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          input: mergeInput(action.input, actionInput),
          session_id: sessionId,
          slash: command.slash,
          raw: rawArgs,
        }),
      },
    )
    const output = asRecord(asRecord(response)?.result)
    if (!output) return { kind: 'none' }
    const outputKind = String(output.kind || '')
    const outputClientEffect = effectFromClientAction(command.pluginId, output as PluginUiAction)
    if (outputClientEffect) return outputClientEffect
    if (outputKind === 'message') return { kind: 'message', text: String(output.text || '') }
    if (outputKind === 'invoke_tool') {
      action = output as PluginUiAction
      actionInput = {}
      continue
    }
    if (outputKind === 'invoke_command') {
      const nextId = String(output.command || '').trim()
      const target = catalog.find((item) => item.pluginId === command.pluginId && item.id === nextId)
      action =
        target?.action?.kind && target.action.kind !== 'none'
          ? target.action
          : ({ kind: 'invoke_command', command: nextId } satisfies PluginUiAction)
      actionInput = output.input ?? {}
      continue
    }
    throw new Error(`Unsupported plugin command output: ${outputKind || 'unknown'}`)
  }

  throw new Error('Plugin command recursion limit reached.')
}
