import type { JsonObject, JsonValue } from '@/types/json'

export type BuiltinAcpAgentId = 'claude-code' | 'antigravity'

export type BuiltinAcpAgentSpec = {
  id: BuiltinAcpAgentId
  defaultName: string
  defaultCommand: string
  defaultArgs: readonly string[]
  defaultClientTools: boolean
}

export const BUILTIN_ACP_AGENTS: readonly BuiltinAcpAgentSpec[] = [
  {
    id: 'claude-code',
    defaultName: 'Claude Code',
    defaultCommand: 'claude-agent-acp',
    defaultArgs: [],
    defaultClientTools: true,
  },
  {
    id: 'antigravity',
    defaultName: 'Antigravity',
    defaultCommand: 'agy',
    defaultArgs: ['acp', '--process-per-session'],
    defaultClientTools: false,
  },
] as const

export function findBuiltinAcpAgent(id: string): BuiltinAcpAgentSpec | undefined {
  return BUILTIN_ACP_AGENTS.find((entry) => entry.id === id)
}

export function isBuiltinAcpAgent(id: string): id is BuiltinAcpAgentId {
  return Boolean(findBuiltinAcpAgent(id))
}

export type AcpAgentConfigEntry = {
  name?: string
  command?: string
  args?: string[]
  env?: Record<string, string>
  client_tools?: boolean
  writable_paths?: string[]
  disabled?: boolean
}

export type AcpConfigDocument = {
  agents: Record<string, AcpAgentConfigEntry>
}

export type AcpAgentEnvRow = {
  key: string
  value: string
}

export type AcpAgentDraft = {
  id: string
  name: string
  command: string
  argsText: string
  envRows: AcpAgentEnvRow[]
  clientTools: boolean
  writablePathsText: string
  disabled: boolean
}

export type AcpAgentStatusKind = 'disabled' | 'overridden' | 'builtin' | 'custom'

export type AcpAgentValidationError =
  | { code: 'invalid_id' }
  | { code: 'duplicate_id' }
  | { code: 'invalid_name' }
  | { code: 'missing_command' }
  | { code: 'invalid_command' }
  | { code: 'invalid_args' }
  | { code: 'too_many_env' }
  | { code: 'invalid_env_key'; key: string }
  | { code: 'duplicate_env_key'; key: string }
  | { code: 'invalid_env_value'; key: string }
  | { code: 'invalid_writable_path'; path: string }
  | { code: 'too_many_writable_paths' }

const ACP_AGENT_ID_RE = /^[a-z0-9][a-z0-9_-]{0,63}$/
const ACP_ENV_KEY_RE = /^[A-Za-z_][A-Za-z0-9_]{0,127}$/
const CONTROL_CHAR_RE = /[\u0000-\u001f\u007f]/

export function isValidAcpAgentId(value: string): boolean {
  return ACP_AGENT_ID_RE.test(value)
}

export function isValidAcpEnvKey(key: string): boolean {
  return ACP_ENV_KEY_RE.test(key) && !key.toUpperCase().startsWith('AGENA_')
}

export function isAbsoluteAcpPath(path: string): boolean {
  const trimmed = path.trim()
  if (!trimmed) return false
  return trimmed.startsWith('/') || /^[A-Za-z]:[\\/]/.test(trimmed) || trimmed.startsWith('\\\\')
}

function isRecord(value: unknown): value is JsonObject {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

export function parseAcpConfigDocument(raw: JsonValue | undefined): AcpConfigDocument {
  if (!isRecord(raw) || !isRecord(raw.agents)) return { agents: {} }
  const agents: Record<string, AcpAgentConfigEntry> = {}
  for (const [id, entryValue] of Object.entries(raw.agents)) {
    if (!isRecord(entryValue)) continue
    const entry: AcpAgentConfigEntry = {}
    if (typeof entryValue.name === 'string' && entryValue.name.trim()) entry.name = entryValue.name
    if (typeof entryValue.command === 'string' && entryValue.command.trim()) entry.command = entryValue.command
    if (Array.isArray(entryValue.args)) {
      const args = entryValue.args.filter((item): item is string => typeof item === 'string')
      if (args.length) entry.args = args
    }
    if (isRecord(entryValue.env)) {
      const env: Record<string, string> = {}
      for (const [k, v] of Object.entries(entryValue.env)) {
        if (typeof v === 'string') env[k] = v
      }
      if (Object.keys(env).length) entry.env = env
    }
    if (typeof entryValue.client_tools === 'boolean') entry.client_tools = entryValue.client_tools
    if (Array.isArray(entryValue.writable_paths)) {
      const paths = entryValue.writable_paths.filter((item): item is string => typeof item === 'string' && item.trim().length > 0)
      if (paths.length) entry.writable_paths = paths
    }
    if (entryValue.disabled === true) entry.disabled = true
    agents[id] = entry
  }
  return { agents }
}

export function hasCustomOverride(entry: AcpAgentConfigEntry | undefined): boolean {
  if (!entry) return false
  return Boolean(
    entry.disabled ||
      entry.name?.trim() ||
      entry.command?.trim() ||
      (entry.args && entry.args.length > 0) ||
      (entry.env && Object.keys(entry.env).length > 0) ||
      entry.client_tools !== undefined ||
      (entry.writable_paths && entry.writable_paths.length > 0),
  )
}

export function acpAgentStatus(id: string, entry: AcpAgentConfigEntry | undefined): AcpAgentStatusKind {
  if (entry?.disabled) return 'disabled'
  if (isBuiltinAcpAgent(id)) return hasCustomOverride(entry) ? 'overridden' : 'builtin'
  return 'custom'
}

export function resolveAcpAgentSummary(id: string, entry: AcpAgentConfigEntry | undefined): {
  id: string
  name: string
  command: string
  args: string[]
  builtin: boolean
  disabled: boolean
  status: AcpAgentStatusKind
  envCount: number
} {
  const builtin = findBuiltinAcpAgent(id)
  if (builtin) {
    const overrideCommand = entry?.command?.trim() || ''
    const command = overrideCommand || builtin.defaultCommand
    const args = entry?.args?.length || overrideCommand ? [...(entry?.args ?? [])] : [...builtin.defaultArgs]
    return {
      id,
      name: entry?.name?.trim() || builtin.defaultName,
      command,
      args,
      builtin: true,
      disabled: entry?.disabled === true,
      status: acpAgentStatus(id, entry),
      envCount: Object.keys(entry?.env ?? {}).length,
    }
  }
  return {
    id,
    name: entry?.name?.trim() || id,
    command: entry?.command?.trim() || '',
    args: [...(entry?.args ?? [])],
    builtin: false,
    disabled: entry?.disabled === true,
    status: acpAgentStatus(id, entry),
    envCount: Object.keys(entry?.env ?? {}).length,
  }
}

export function listAcpAgentIds(doc: AcpConfigDocument): string[] {
  const builtins = BUILTIN_ACP_AGENTS.map((entry) => entry.id as string)
  const custom = Object.keys(doc.agents)
    .filter((id) => !isBuiltinAcpAgent(id))
    .sort((a, b) => a.localeCompare(b))
  return [...builtins, ...custom]
}

export function draftFromAcpAgent(id: string, entry: AcpAgentConfigEntry | undefined): AcpAgentDraft {
  const builtin = findBuiltinAcpAgent(id)
  return {
    id,
    name: entry?.name ?? '',
    command: entry?.command ?? '',
    argsText: (entry?.args ?? []).join('\n'),
    envRows: Object.entries(entry?.env ?? {})
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([key, value]) => ({ key, value })),
    clientTools: entry?.client_tools ?? (builtin ? builtin.defaultClientTools : true),
    writablePathsText: (entry?.writable_paths ?? []).join('\n'),
    disabled: entry?.disabled === true,
  }
}

export function blankCustomAcpAgentDraft(): AcpAgentDraft {
  return {
    id: '',
    name: '',
    command: '',
    argsText: '',
    envRows: [],
    clientTools: true,
    writablePathsText: '',
    disabled: false,
  }
}

export function parseMultilineList(text: string): string[] {
  return text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
}

export function validateAcpAgentDraft(
  draft: AcpAgentDraft,
  existingIds: readonly string[],
  isCreating: boolean,
): AcpAgentValidationError[] {
  const errors: AcpAgentValidationError[] = []
  const id = draft.id.trim()
  const builtin = findBuiltinAcpAgent(id)

  if (!isValidAcpAgentId(id)) {
    errors.push({ code: 'invalid_id' })
  } else if (isCreating && (existingIds.includes(id) || Boolean(builtin))) {
    errors.push({ code: 'duplicate_id' })
  }

  const name = draft.name.trim()
  if (draft.name.length > 0 && (!name || name.length > 128 || CONTROL_CHAR_RE.test(draft.name))) {
    errors.push({ code: 'invalid_name' })
  }

  const command = draft.command.trim()
  if (!draft.disabled && !builtin && !command) {
    errors.push({ code: 'missing_command' })
  } else if (draft.command.length > 0 && (!command || command.length > 4096 || CONTROL_CHAR_RE.test(draft.command))) {
    errors.push({ code: 'invalid_command' })
  }

  const args = parseMultilineList(draft.argsText)
  if (args.length > 128 || args.some((arg) => arg.length > 4096 || CONTROL_CHAR_RE.test(arg))) {
    errors.push({ code: 'invalid_args' })
  }

  const nonEmptyEnv = draft.envRows.filter((row) => row.key.trim() !== '' || row.value !== '')
  if (nonEmptyEnv.length > 64) {
    errors.push({ code: 'too_many_env' })
  }
  const seenKeys = new Set<string>()
  for (const row of nonEmptyEnv) {
    const key = row.key.trim()
    if (!isValidAcpEnvKey(key)) {
      errors.push({ code: 'invalid_env_key', key })
      continue
    }
    if (seenKeys.has(key)) {
      errors.push({ code: 'duplicate_env_key', key })
    }
    seenKeys.add(key)
    if (row.value.length > 16384 || row.value.includes('\0')) {
      errors.push({ code: 'invalid_env_value', key })
    }
  }

  const paths = parseMultilineList(draft.writablePathsText)
  if (paths.length > 16) {
    errors.push({ code: 'too_many_writable_paths' })
  }
  for (const path of paths) {
    if (!isAbsoluteAcpPath(path)) {
      errors.push({ code: 'invalid_writable_path', path })
    }
  }

  return errors
}

export function buildAcpAgentEntryFromDraft(draft: AcpAgentDraft): AcpAgentConfigEntry {
  const id = draft.id.trim()
  const builtin = findBuiltinAcpAgent(id)
  const entry: AcpAgentConfigEntry = {}

  const name = draft.name.trim()
  if (name && (!builtin || name !== builtin.defaultName)) {
    entry.name = name
  }

  const command = draft.command.trim()
  if (command && (!builtin || command !== builtin.defaultCommand)) {
    entry.command = command
  }

  const args = parseMultilineList(draft.argsText)
  if (args.length > 0) {
    entry.args = args
  }

  const env: Record<string, string> = {}
  for (const row of draft.envRows) {
    const key = row.key.trim()
    if (!key) continue
    env[key] = row.value
  }
  if (Object.keys(env).length > 0) {
    entry.env = env
  }

  const defaultClientTools = builtin ? builtin.defaultClientTools : true
  if (draft.clientTools !== defaultClientTools) {
    entry.client_tools = draft.clientTools
  }

  const writablePaths = parseMultilineList(draft.writablePathsText)
  if (writablePaths.length > 0) {
    entry.writable_paths = writablePaths
  }

  if (draft.disabled) {
    entry.disabled = true
  }

  return entry
}

export function applyAcpAgentDraft(doc: AcpConfigDocument, draft: AcpAgentDraft): AcpConfigDocument {
  const id = draft.id.trim()
  const nextAgents: Record<string, AcpAgentConfigEntry> = { ...doc.agents }
  const entry = buildAcpAgentEntryFromDraft(draft)
  if (isBuiltinAcpAgent(id) && !hasCustomOverride(entry)) {
    delete nextAgents[id]
  } else {
    nextAgents[id] = entry
  }
  return { agents: nextAgents }
}

export function removeAcpAgentFromConfig(doc: AcpConfigDocument, id: string): AcpConfigDocument {
  const nextAgents: Record<string, AcpAgentConfigEntry> = { ...doc.agents }
  delete nextAgents[id]
  return { agents: nextAgents }
}

export function serializeAcpConfigDocument(doc: AcpConfigDocument): JsonObject | null {
  const entries = Object.entries(doc.agents)
    .filter(([id, entry]) => !isBuiltinAcpAgent(id) || hasCustomOverride(entry))
    .sort(([a], [b]) => a.localeCompare(b))
  if (entries.length === 0) return null
  const agents: JsonObject = {}
  for (const [id, entry] of entries) {
    const obj: JsonObject = {}
    if (entry.name) obj.name = entry.name
    if (entry.command) obj.command = entry.command
    if (entry.args && entry.args.length) obj.args = entry.args
    if (entry.env && Object.keys(entry.env).length) obj.env = entry.env
    if (entry.client_tools !== undefined) obj.client_tools = entry.client_tools
    if (entry.writable_paths && entry.writable_paths.length) obj.writable_paths = entry.writable_paths
    if (entry.disabled) obj.disabled = true
    agents[id] = obj
  }
  return { agents }
}

export function formatAcpCommandPreview(command: string, args: readonly string[]): string {
  const trimmed = command.trim()
  if (!trimmed) return ''
  const formatArg = (arg: string) => (/[\s"'\\]/.test(arg) ? JSON.stringify(arg) : arg)
  return [trimmed, ...args.map(formatArg)].join(' ')
}
