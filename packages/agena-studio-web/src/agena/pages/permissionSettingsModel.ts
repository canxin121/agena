import type { PermissionMode } from '../lib/agenaApi'

export type PermissionZoneModes = {
  read: PermissionMode
  write: PermissionMode
}

export type PermissionEditorModel = {
  path: {
    workspace: PermissionZoneModes
    external: PermissionZoneModes
    rules: Record<string, PermissionZoneModes>
  }
  network: {
    internet: PermissionMode
    private: PermissionMode
    loopback: PermissionMode
    rules: Record<string, PermissionMode>
  }
  entries: {
    tags: Record<string, PermissionMode>
    names: Record<string, PermissionMode>
    rules: Record<string, Record<string, PermissionMode>>
  }
}

export type PermissionOverviewCounts = {
  pathRules: number
  networkRules: number
  tagRules: number
  nameRules: number
  commandRules: number
}

export function permissionModeLabel(mode: PermissionMode): string {
  return mode.charAt(0).toUpperCase() + mode.slice(1)
}

export function permissionModeBadgeClass(mode: PermissionMode): 'success' | 'warn' | 'danger' {
  if (mode === 'allow') return 'success'
  if (mode === 'ask') return 'warn'
  return 'danger'
}

export function createPermissionEditorModel(): PermissionEditorModel {
  return {
    path: {
      workspace: { read: 'allow', write: 'ask' },
      external: { read: 'ask', write: 'ask' },
      rules: {},
    },
    network: {
      internet: 'ask',
      private: 'deny',
      loopback: 'deny',
      rules: {},
    },
    entries: {
      tags: {},
      names: {},
      rules: {},
    },
  }
}

export function clonePermissionEditorModel(model: PermissionEditorModel): PermissionEditorModel {
  return JSON.parse(JSON.stringify(model)) as PermissionEditorModel
}

export function normalizePermissionEditorModel(value: unknown): PermissionEditorModel {
  const root = isRecord(value) ? value : {}
  const fallback = createPermissionEditorModel()
  const entriesRoot = isRecord(root.entries) ? root.entries : isRecord(root.tools) ? root.tools : {}

  return {
    path: {
      workspace: normalizePermissionZoneModes(root.path, 'workspace', fallback.path.workspace),
      external: normalizePermissionZoneModes(root.path, 'external', fallback.path.external),
      rules: normalizePathRules(root.path),
    },
    network: {
      internet: normalizePermissionMode(
        isRecord(root.network) ? root.network.internet : undefined,
        fallback.network.internet,
      ),
      private: normalizePermissionMode(
        isRecord(root.network) ? root.network.private : undefined,
        fallback.network.private,
      ),
      loopback: normalizePermissionMode(
        isRecord(root.network) ? root.network.loopback : undefined,
        fallback.network.loopback,
      ),
      rules: normalizeModeRecord(isRecord(root.network) ? root.network.rules : undefined),
    },
    entries: {
      tags: normalizeModeRecord(isRecord(entriesRoot) ? entriesRoot.tags : undefined),
      names: normalizeModeRecord(isRecord(entriesRoot) ? entriesRoot.names : undefined),
      rules: normalizeCommandRules(isRecord(entriesRoot) ? entriesRoot.rules : undefined),
    },
  }
}

export function summarizePermissionEditorModel(model: PermissionEditorModel): PermissionOverviewCounts {
  return {
    pathRules: Object.keys(model.path.rules).length,
    networkRules: Object.keys(model.network.rules).length,
    tagRules: Object.keys(model.entries.tags).length,
    nameRules: Object.keys(model.entries.names).length,
    commandRules: countCommandPatternRows(model.entries.rules),
  }
}

export function countPermissionDraftChanges(current: PermissionEditorModel, baseline: PermissionEditorModel): number {
  return countChangedLeaves(current, baseline)
}

export function commandRuleRows(value: Record<string, PermissionMode> | undefined) {
  if (!isRecord(value)) return []
  return Object.entries(value)
    .sort(([leftPattern], [rightPattern]) => {
      const lengthDiff = rightPattern.length - leftPattern.length
      if (lengthDiff !== 0) return lengthDiff
      return leftPattern.localeCompare(rightPattern)
    })
    .map(([pattern, access]) => ({
      pattern,
      access: normalizePermissionMode(access, 'ask'),
    }))
}

export function suggestDuplicateKey(baseKey: string, existingKeys: Iterable<string>): string {
  const normalizedBase = baseKey.trim() || 'copy'
  const seen = new Set<string>()
  for (const key of existingKeys) {
    seen.add(key)
  }

  let candidate = `${normalizedBase} copy`
  let index = 2
  while (seen.has(candidate)) {
    candidate = `${normalizedBase} copy ${index}`
    index += 1
  }
  return candidate
}

export function replaceRecord<T>(target: Record<string, T>, source: Record<string, T>) {
  for (const key of Object.keys(target)) {
    delete target[key]
  }
  Object.assign(target, source)
}

export function replacePermissionEditorModel(target: PermissionEditorModel, source: PermissionEditorModel) {
  target.path.workspace = { ...source.path.workspace }
  target.path.external = { ...source.path.external }
  replaceRecord(target.path.rules, source.path.rules)
  target.network.internet = source.network.internet
  target.network.private = source.network.private
  target.network.loopback = source.network.loopback
  replaceRecord(target.network.rules, source.network.rules)
  replaceRecord(target.entries.tags, source.entries.tags)
  replaceRecord(target.entries.names, source.entries.names)
  replaceRecord(target.entries.rules, source.entries.rules)
}

function normalizePathRules(value: unknown): Record<string, PermissionZoneModes> {
  if (!isRecord(value) || !isRecord(value.rules)) return {}

  const out: Record<string, PermissionZoneModes> = {}
  for (const [key, rule] of Object.entries(value.rules)) {
    out[key] = normalizePathRule(rule)
  }
  return out
}

function normalizePathRule(value: unknown): PermissionZoneModes {
  if (typeof value === 'string') {
    return decodePathRuleString(value)
  }
  if (!isRecord(value)) {
    return { read: 'ask', write: 'ask' }
  }
  return {
    read: normalizePermissionMode(value.read, 'ask'),
    write: normalizePermissionMode(value.write, 'ask'),
  }
}

function decodePathRuleString(value: string): PermissionZoneModes {
  const normalized = value.trim().toLowerCase().replaceAll('-', '_')
  switch (normalized) {
    case 'allow':
      return { read: 'allow', write: 'allow' }
    case 'ask':
      return { read: 'ask', write: 'ask' }
    case 'deny':
    case 'none':
      return { read: 'deny', write: 'deny' }
    case 'read':
    case 'read_only':
    case 'ro':
      return { read: 'allow', write: 'deny' }
    case 'write':
    case 'write_only':
    case 'wo':
      return { read: 'deny', write: 'allow' }
    case 'read_write':
    case 'rw':
      return { read: 'allow', write: 'allow' }
    default:
      return { read: 'ask', write: 'ask' }
  }
}

function normalizePermissionZoneModes(
  root: unknown,
  key: 'workspace' | 'external',
  fallback: PermissionZoneModes,
): PermissionZoneModes {
  const section = isRecord(root) && isRecord(root[key]) ? root[key] : null
  if (!section) return { ...fallback }
  return {
    read: normalizePermissionMode(section.read, fallback.read),
    write: normalizePermissionMode(section.write, fallback.write),
  }
}

function normalizeModeRecord(value: unknown): Record<string, PermissionMode> {
  if (!isRecord(value)) return {}
  const out: Record<string, PermissionMode> = {}
  for (const [key, mode] of Object.entries(value)) {
    out[key] = normalizePermissionMode(mode, 'ask')
  }
  return out
}

function normalizeCommandRules(value: unknown): Record<string, Record<string, PermissionMode>> {
  if (!isRecord(value)) return {}
  const out: Record<string, Record<string, PermissionMode>> = {}
  for (const [key, rule] of Object.entries(value)) {
    out[key] = normalizeCommandRule(rule)
  }
  return out
}

function normalizeCommandRule(value: unknown): Record<string, PermissionMode> {
  if (typeof value === 'string') {
    return { '*': normalizePermissionMode(value, 'ask') }
  }
  if (!isRecord(value)) {
    return {}
  }
  const out: Record<string, PermissionMode> = {}
  for (const [pattern, mode] of Object.entries(value)) {
    out[pattern] = normalizePermissionMode(mode, 'ask')
  }
  return out
}

function countCommandPatternRows(value: Record<string, Record<string, PermissionMode>>): number {
  return Object.values(value).reduce((count, patterns) => count + Object.keys(patterns).length, 0)
}

function countChangedLeaves(current: unknown, baseline: unknown): number {
  if (isRecord(current) && isRecord(baseline)) {
    const keys = new Set([...Object.keys(current), ...Object.keys(baseline)])
    let total = 0
    for (const key of keys) {
      total += countChangedLeaves(current[key], baseline[key])
    }
    return total
  }
  if (isRecord(current)) {
    return countLeaves(current)
  }
  if (isRecord(baseline)) {
    return countLeaves(baseline)
  }
  return Object.is(current, baseline) ? 0 : 1
}

function countLeaves(value: unknown): number {
  if (!isRecord(value)) {
    return 1
  }
  let total = 0
  for (const child of Object.values(value)) {
    total += countLeaves(child)
  }
  return total
}

function normalizePermissionMode(value: unknown, fallback: PermissionMode): PermissionMode {
  if (value === 'allow' || value === 'ask' || value === 'deny') {
    return value
  }
  if (typeof value === 'string') {
    const normalized = value.trim().toLowerCase()
    if (normalized === 'allow' || normalized === 'ask' || normalized === 'deny') {
      return normalized
    }
  }
  return fallback
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value)
}
