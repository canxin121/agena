import type { JsonValue } from '@/types/json'

export type PermissionMode = 'allow' | 'auto' | 'ask' | 'deny'
export type PathPermissionMode = 'readonly' | 'readwrite' | 'deny'
export type PermissionConfig = {
  path?: {
    workspace?: PathPermissionMode
    external?: PathPermissionMode
    rules?: Record<string, PathPermissionMode>
  }
  network?: {
    internet?: PermissionMode
    private?: PermissionMode
    loopback?: PermissionMode
    rules?: Record<string, PermissionMode>
  }
  tools?: {
    default?: PermissionMode
    names?: Record<string, PermissionMode>
    rules?: Record<string, PermissionMode | Record<string, PermissionMode>>
  }
  approval_model?: JsonValue
}

export function renamePermissionRule<T>(rules: Record<string, T>, from: string, to: string): Record<string, T> {
  // Entry order determines which overlapping command or network rule wins.
  return Object.fromEntries(Object.entries(rules).map(([key, value]) => [key === from ? to : key, value]))
}

type MessageParams = Record<string, string>
type Translate = (key: string, params?: MessageParams) => string

export class PermissionConfigError extends Error {
  readonly key: string
  readonly params: MessageParams

  constructor(key: string, params: MessageParams = {}) {
    super(key.replace(/\{(\w+)\}/g, (_, name: string) => params[name] ?? `{${name}}`))
    this.name = 'PermissionConfigError'
    this.key = key
    this.params = params
  }
}

export function permissionConfigErrorMessage(reason: unknown, translate: Translate): string {
  return reason instanceof PermissionConfigError
    ? translate(reason.key, reason.params)
    : reason instanceof Error
      ? reason.message
      : String(reason)
}

export function isPermissionMode(value: unknown): value is PermissionMode {
  return value === 'allow' || value === 'auto' || value === 'ask' || value === 'deny'
}

export function isPathPermissionMode(value: unknown): value is PathPermissionMode {
  return value === 'readonly' || value === 'readwrite' || value === 'deny'
}

function objectAt(value: unknown, field: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new PermissionConfigError('Expected an object at {field}.', { field })
  return value as Record<string, unknown>
}

function knownFields(value: Record<string, unknown>, keys: string[], field: string) {
  for (const key of Object.keys(value)) {
    if (!keys.includes(key))
      throw new PermissionConfigError('Unknown permission field: {field}.', { field: `${field}.${key}` })
  }
}

function accessMode(value: unknown, field: string): PathPermissionMode {
  if (isPathPermissionMode(value)) return value
  if (value && typeof value === 'object' && !Array.isArray(value)) {
    const legacy = value as Record<string, unknown>
    knownFields(legacy, ['read', 'write'], field)
    if (legacy.read === 'allow' && legacy.write === 'deny') return 'readonly'
    if (legacy.read === 'allow' && legacy.write === 'allow') return 'readwrite'
    if (legacy.read === 'deny' && legacy.write === 'deny') return 'deny'
    throw new PermissionConfigError(
      'Legacy path permission at {field} needs an explicit choice of readonly, readwrite, or deny. Move ask/auto to tool approval.',
      { field },
    )
  }
  throw new PermissionConfigError('Invalid permission value at {field}: expected {expected}.', {
    field,
    expected: 'readonly | readwrite | deny',
  })
}

/** Lexical checks only. The server resolves aliases, filesystem identity and backend capabilities. */
export function canonicalPermissionPath(value: string): string {
  if (!value || /[\u0000-\u001f\u007f-\u009f]/.test(value))
    throw new PermissionConfigError('Path rule cannot be empty or contain control characters.')
  if (value.trim() !== value)
    throw new PermissionConfigError('Path rule {path} must not have surrounding whitespace.', { path: value })
  let path = value.replace(/\\/g, '/')
  // Only the former full-subtree suffix has a safe concrete-path equivalent.
  if (path.endsWith('/**')) path = path.slice(0, -3)
  if (!path)
    throw new PermissionConfigError('Path rule {path} must name a specific directory instead of a filesystem root.', {
      path: value,
    })
  if (/[*?]/.test(path))
    throw new PermissionConfigError('Path rule {path} must name a file or directory; wildcards are not supported.', {
      path: value,
    })
  if (path.startsWith('~'))
    throw new PermissionConfigError('Path rule {path} must use <home> instead of ~.', { path: value })

  let prefix = '<workspace>'
  let suffix = path
  if (path.startsWith('<')) {
    const alias = /^<(workspace|cwd|home|tmp|state)>(?=\/|$)/.exec(path)
    if (!alias) throw new PermissionConfigError('Path rule {path} uses an unknown path alias.', { path: value })
    prefix = alias[1] === 'cwd' ? '<workspace>' : alias[0]
    suffix = path.slice(alias[0].length)
  } else if (path.startsWith('//')) {
    throw new PermissionConfigError('Path rule {path} must name a local path; network shares are unsupported.', {
      path: value,
    })
  } else if (path.startsWith('/')) {
    prefix = '/'
  } else if (path[1] === ':') {
    if (!/^[A-Za-z]:\//.test(path))
      throw new PermissionConfigError('Path rule {path} must use an absolute Windows path such as C:/work/output.', {
        path: value,
      })
    prefix = path[0]!.toUpperCase() + ':/'
    suffix = path.slice(3)
  }

  const parts = suffix.split('/').filter((part) => part && part !== '.')
  if (parts.includes('..'))
    throw new PermissionConfigError('Path rule {path} must not contain parent traversal.', { path: value })
  if (parts.some((part) => /[<>:]/.test(part)))
    throw new PermissionConfigError('Path rule {path} has a misplaced alias or an unsupported alternate data stream.', {
      path: value,
    })
  if (!parts.length && prefix.endsWith('/'))
    throw new PermissionConfigError('Path rule {path} must name a specific directory instead of a filesystem root.', {
      path: value,
    })
  return parts.length ? prefix + (prefix.endsWith('/') ? '' : '/') + parts.join('/') : prefix
}

function permissionMode(value: unknown, field: string): PermissionMode {
  if (!isPermissionMode(value))
    throw new PermissionConfigError('Invalid permission value at {field}: expected {expected}.', {
      field,
      expected: 'allow | auto | ask | deny',
    })
  return value
}

function modeMap(value: unknown, field: string): Record<string, PermissionMode> {
  return Object.fromEntries(
    Object.entries(objectAt(value, field)).map(([key, mode]) => [
      key,
      permissionMode(mode, `${field}[${JSON.stringify(key)}]`),
    ]),
  )
}

/** Normalize deterministic legacy values, then validate before replacing a draft or submitting it. */
export function parsePermissionConfig(value: unknown): PermissionConfig {
  const raw = objectAt(value, 'permission')
  knownFields(raw, ['path', 'network', 'tools', 'approval_model'], 'permission')
  const result: PermissionConfig = {}
  if (raw.path != null) {
    const path = objectAt(raw.path, 'permission.path')
    knownFields(path, ['workspace', 'external', 'rules'], 'permission.path')
    result.path = {}
    for (const scope of ['workspace', 'external'] as const) {
      if (path[scope] != null) result.path[scope] = accessMode(path[scope], `permission.path.${scope}`)
    }
    if (path.rules !== undefined) {
      const entries = Object.entries(objectAt(path.rules, 'permission.path.rules')).map(([original, value]) => ({
        original,
        key: canonicalPermissionPath(original),
        mode: accessMode(value, `permission.path.rules[${JSON.stringify(original)}]`),
        legacy: !isPathPermissionMode(value),
        subtree: original.trim().replace(/\\/g, '/').endsWith('/**'),
      }))
      const rules = new Map<string, PathPermissionMode>()
      for (const entry of entries) {
        if (
          entry.legacy &&
          entry.mode !== 'deny' &&
          !entries.some(
            (other) => other.key === entry.key && other.mode === entry.mode && other.subtree !== entry.subtree,
          )
        )
          throw new PermissionConfigError(
            'Legacy path rule {path} needs an explicit mode choice because directory rules now include descendants.',
            { path: entry.original },
          )
        if (rules.has(entry.key) && rules.get(entry.key) !== entry.mode)
          throw new PermissionConfigError('Path rule {path} has conflicting modes after normalization.', {
            path: entry.key,
          })
        rules.set(entry.key, entry.mode)
      }
      for (const [path, mode] of rules) {
        if (mode === 'deny') continue
        for (const [parent, parentMode] of rules) {
          if (path === parent || !path.startsWith(parent + '/') || parentMode === mode) continue
          if (parentMode === 'deny')
            throw new PermissionConfigError('Path rule {path} cannot reopen denied path {parent}.', { path, parent })
          throw new PermissionConfigError(
            'Path rule {path} conflicts with {parent}: use separate roots for readonly and readwrite access.',
            { path, parent },
          )
        }
      }
      result.path.rules = Object.fromEntries(rules)
    }
  }
  if (raw.network != null) {
    const network = objectAt(raw.network, 'permission.network')
    knownFields(network, ['internet', 'private', 'loopback', 'rules'], 'permission.network')
    result.network = {}
    for (const zone of ['internet', 'private', 'loopback'] as const) {
      if (network[zone] != null) result.network[zone] = permissionMode(network[zone], `permission.network.${zone}`)
    }
    if (network.rules !== undefined) result.network.rules = modeMap(network.rules, 'permission.network.rules')
  }
  if (raw.tools != null) {
    const tools = objectAt(raw.tools, 'permission.tools')
    knownFields(tools, ['default', 'names', 'rules'], 'permission.tools')
    result.tools = {}
    if (tools.default != null) result.tools.default = permissionMode(tools.default, 'permission.tools.default')
    if (tools.names !== undefined) result.tools.names = modeMap(tools.names, 'permission.tools.names')
    if (tools.rules !== undefined)
      result.tools.rules = Object.fromEntries(
        Object.entries(objectAt(tools.rules, 'permission.tools.rules')).map(([tool, rules]) => [
          tool,
          typeof rules === 'string'
            ? permissionMode(rules, `permission.tools.rules[${JSON.stringify(tool)}]`)
            : modeMap(rules, `permission.tools.rules[${JSON.stringify(tool)}]`),
        ]),
      )
  }
  if (raw.approval_model != null) result.approval_model = raw.approval_model as JsonValue
  return result
}
