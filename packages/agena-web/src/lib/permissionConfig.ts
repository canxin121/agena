import type { JsonValue } from '@/types/json'

export type PermissionMode = 'allow' | 'auto' | 'ask' | 'deny'
export const SHELL_COMMAND_RULE_GROUP = 'shell.*'
export type PathPermissionMode = 'readonly' | 'readwrite' | 'deny'
export type NetworkAction = 'allow' | 'deny'
export type NetworkProtocol = 'tcp' | 'udp' | 'icmp' | 'any'
export type NetworkRule = {
  to: string
  protocol?: NetworkProtocol
  ports?: Array<number | string>
  except?: string[]
}
export type NetworkPermissionConfig = {
  version?: 2
  default?: NetworkAction
  allow?: NetworkRule[]
  deny?: NetworkRule[]
  inbound?: NetworkAction
  hostLoopback?: NetworkAction
}
export type PermissionConfig = {
  path?: {
    workspace?: PathPermissionMode
    external?: PathPermissionMode
    rules?: Record<string, PathPermissionMode>
  }
  network?: NetworkPermissionConfig
  tools?: {
    default?: PermissionMode
    names?: Record<string, PermissionMode>
    rules?: Record<string, PermissionMode | Record<string, PermissionMode>>
  }
  plugins?: Record<string, boolean>
  approval_model?: JsonValue
}

/** Match the runtime defaults for an independent permission configuration. */
export function completePermissionConfig(config: PermissionConfig): PermissionConfig {
  return {
    ...config,
    path: { workspace: 'deny', external: 'deny', ...config.path },
    network: {
      version: 2,
      default: 'deny',
      inbound: 'deny',
      hostLoopback: 'deny',
      allow: [],
      deny: [],
      ...config.network,
    },
    tools: { default: 'deny', ...config.tools },
  }
}

// Keep the registered identities in agena-domain/tool_permission_config.rs.
// Other tool names, including third-party names with underscores, are exact IDs.
const permissionToolAliases = new Map<string, string>()
for (const operation of [
  'exec',
  'spawn',
  'watch',
  'open',
  'write',
  'read',
  'logs',
  'list',
  'stop',
  'resize',
  'signal',
]) {
  const canonical = `agena.shell.${operation}`
  permissionToolAliases.set(`shell.${operation}`, canonical)
  permissionToolAliases.set(`agena_shell_${operation}`, canonical)
}
permissionToolAliases.set('monitor.start', 'agena.monitor.start')
permissionToolAliases.set('agena_monitor_start', 'agena.monitor.start')

export function canonicalPermissionToolName(name: string): string {
  return permissionToolAliases.get(name) || name
}

export function canonicalPermissionToolEntries<T>(values: Record<string, T> | undefined): Array<[string, T]> {
  const projected = new Map<string, T>()
  for (const name of Object.keys(values || {}).sort()) {
    const canonical = canonicalPermissionToolName(name)
    if (name === canonical || !projected.has(canonical)) projected.set(canonical, values![name]!)
  }
  return Array.from(projected).sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0))
}

export function renamePermissionRule<T>(rules: Record<string, T>, from: string, to: string): Record<string, T> {
  // Entry order determines which overlapping command rule wins.
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
  if (path.endsWith('/**')) path = path.slice(0, -2)
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

const legacyCommandTools = new Set([
  'agena.shell.exec',
  'agena.shell.spawn',
  'agena.shell.watch',
  'agena.shell.open',
  'agena.shell.write',
])
const commandRulePriority: Record<PermissionMode, number> = { allow: 0, auto: 1, ask: 2, deny: 3 }

function commandClassKeyword(pattern: string): string | undefined {
  const keyword = pattern.replace(/[A-Z]/g, (letter) => letter.toLowerCase()).replaceAll('_', '-')
  if (keyword === 'noop') return 'no-op'
  if (['no-op', 'routine', 'dangerous'].includes(keyword)) return keyword
}

function shareLegacyCommandRules(rules: NonNullable<NonNullable<PermissionConfig['tools']>['rules']>) {
  const seen = new Map<string, PermissionMode | Record<string, PermissionMode>>()
  const groups = new Map<string, Record<string, PermissionMode>>()
  for (const name of Object.keys(rules).sort()) {
    const tool = canonicalPermissionToolName(name)
    const entries = rules[name]!
    if (!legacyCommandTools.has(tool)) continue
    const previous = seen.get(tool)
    if (previous && JSON.stringify(previous) !== JSON.stringify(entries))
      throw new PermissionConfigError('Conflicting command rules for {tool}.', { tool })
    seen.set(tool, entries)
    if (typeof entries === 'object') {
      groups.set(tool, entries)
      delete rules[name]
    }
  }
  if (!groups.size || Object.hasOwn(rules, SHELL_COMMAND_RULE_GROUP)) return

  const merged = new Map<string, PermissionMode>()
  const patternLists = new Set<string>()
  for (const tool of [...groups.keys()].sort()) {
    const entries = Object.entries(groups.get(tool)!)
    const patterns = entries.filter(([pattern]) => !commandClassKeyword(pattern))
    if (patterns.length) patternLists.add(JSON.stringify(patterns))
    const normalized = new Map(entries.map(([pattern, mode]) => [commandClassKeyword(pattern) || pattern, mode]))
    for (const [pattern, mode] of normalized) {
      const previous = merged.get(pattern)
      if (!previous || commandRulePriority[previous] < commandRulePriority[mode]) merged.set(pattern, mode)
    }
  }
  const entries = [...merged]
  if (patternLists.size > 1) {
    const priority = ([pattern, mode]: [string, PermissionMode]) =>
      commandClassKeyword(pattern) ? 0 : 1 + commandRulePriority[mode]
    entries.sort((left, right) => priority(left) - priority(right))
  }
  rules[SHELL_COMMAND_RULE_GROUP] = Object.fromEntries(entries)
}

function networkAction(value: unknown, field: string): NetworkAction {
  if (value === 'allow' || value === 'deny') return value
  throw new PermissionConfigError('Invalid permission value at {field}: expected {expected}.', {
    field,
    expected: 'allow | deny',
  })
}

function arrayAt(value: unknown, field: string, nonempty = false): unknown[] {
  if (!Array.isArray(value)) throw new PermissionConfigError('Expected an array at {field}.', { field })
  if (nonempty && value.length === 0)
    throw new PermissionConfigError('Network selector at {field} must not be empty.', { field })
  return value
}

function parseIp(value: string): { address: bigint; bits: number } | null {
  const v4 = (input: string): number[] | null => {
    const octets = input.split('.')
    if (octets.length !== 4 || octets.some((part) => !/^(0|[1-9][0-9]{0,2})$/.test(part) || Number(part) > 255))
      return null
    return octets.map(Number)
  }
  if (!value.includes(':')) {
    const octets = v4(value)
    return octets ? { address: octets.reduce((result, part) => (result << 8n) | BigInt(part), 0n), bits: 32 } : null
  }
  let address = value
  if (address.includes('.')) {
    const boundary = address.lastIndexOf(':')
    const octets = v4(address.slice(boundary + 1))
    if (!octets) return null
    address =
      address.slice(0, boundary + 1) +
      ((octets[0]! << 8) | octets[1]!).toString(16) +
      ':' +
      ((octets[2]! << 8) | octets[3]!).toString(16)
  }
  const compressed = address.split('::')
  if (compressed.length > 2) return null
  const left = compressed[0] ? compressed[0].split(':') : []
  const right = compressed.length === 2 && compressed[1] ? compressed[1].split(':') : []
  if ([...left, ...right].some((part) => !/^[0-9a-fA-F]{1,4}$/.test(part))) return null
  const omitted = 8 - left.length - right.length
  if (compressed.length === 1 ? left.length !== 8 : omitted < 1) return null
  const parts = compressed.length === 1 ? left : [...left, ...Array<string>(omitted).fill('0'), ...right]
  return { address: parts.reduce((result, part) => (result << 16n) | BigInt(parseInt(part, 16)), 0n), bits: 128 }
}

function networkCidr(value: unknown, field: string) {
  if (typeof value !== 'string' || value.trim() !== value) {
    throw new PermissionConfigError('Expected an IP CIDR at {field}; domains and URLs are not supported.', { field })
  }
  const parts = value.split('/')
  const ip = parts.length === 2 ? parseIp(parts[0]!) : null
  if (!ip || !/^(0|[1-9][0-9]*)$/.test(parts[1]!) || Number(parts[1]) > ip.bits) {
    throw new PermissionConfigError('Expected an IP CIDR at {field}; domains and URLs are not supported.', { field })
  }
  const prefix = Number(parts[1])
  const network = (ip.address >> BigInt(ip.bits - prefix)) << BigInt(ip.bits - prefix)
  if (ip.address !== network) {
    throw new PermissionConfigError(
      'CIDR at {field} has nonzero host bits; use the network address for the selected prefix.',
      { field },
    )
  }
  return { value, ...ip, prefix, network }
}

function networkDestination(value: unknown, field: string) {
  if (value === '*') return null
  if (typeof value !== 'string' || !value || value.trim() !== value)
    throw new PermissionConfigError('Expected an IP address, CIDR, or * at {field}.', { field })
  if (value.includes('/')) return networkCidr(value, field)
  const ip = parseIp(value)
  if (!ip) throw new PermissionConfigError('Expected an IP address, CIDR, or * at {field}.', { field })
  return { value, ...ip, prefix: ip.bits, network: ip.address }
}

function networkPort(value: unknown, field: string): number | string {
  if (typeof value === 'number' && Number.isInteger(value) && value >= 1 && value <= 65535) return value
  if (typeof value === 'string' && /^(?:[1-9][0-9]*)-(?:[1-9][0-9]*)$/.test(value)) {
    const [start, end] = value.split('-').map(Number) as [number, number]
    if (start <= end && end <= 65535) return value
  }
  throw new PermissionConfigError(
    'Port at {field} must be an integer or a range such as 8000-8099, within 1 to 65535.',
    { field },
  )
}

function networkRules(value: unknown, field: string): NetworkRule[] {
  return arrayAt(value, field).map((value, index) => {
    const at = field + '[' + index + ']'
    const rule = objectAt(value, at)
    knownFields(rule, ['to', 'protocol', 'ports', 'except'], at)
    const destination = networkDestination(rule.to, at + '.to')
    const result: NetworkRule = { to: rule.to as string }
    if (rule.protocol !== undefined) {
      if (typeof rule.protocol !== 'string' || !['tcp', 'udp', 'icmp', 'any'].includes(rule.protocol))
        throw new PermissionConfigError('Invalid permission value at {field}: expected {expected}.', {
          field: at + '.protocol',
          expected: 'tcp | udp | icmp | any',
        })
      result.protocol = rule.protocol as NetworkProtocol
    }
    if (rule.ports !== undefined) {
      result.ports = arrayAt(rule.ports, at + '.ports', true).map((port, index) =>
        networkPort(port, at + '.ports[' + index + ']'),
      )
      if (result.protocol === 'icmp')
        throw new PermissionConfigError('ICMP rule at {field} must not specify ports.', { field: at })
    }
    if (rule.except !== undefined) {
      const exclusions = arrayAt(rule.except, at + '.except')
      if (!destination && exclusions.length)
        throw new PermissionConfigError('Excluded subnets at {field} require a concrete target CIDR.', {
          field: at + '.except',
        })
      result.except = exclusions.map((value, index) => {
        const field = at + '.except[' + index + ']'
        const excluded = networkCidr(value, field)
        if (
          !destination ||
          excluded.bits !== destination.bits ||
          excluded.prefix < destination.prefix ||
          excluded.address >> BigInt(destination.bits - destination.prefix) !==
            destination.address >> BigInt(destination.bits - destination.prefix)
        )
          throw new PermissionConfigError('Excluded CIDR at {field} must be contained in {cidr}.', {
            field,
            cidr: destination?.value || '*',
          })
        return excluded.value
      })
    }
    return result
  })
}

/** Legacy policies are complete; version 2 preserves omitted fields for per-field inheritance. */
export function parseNetworkPermissionConfig(value: unknown): NetworkPermissionConfig {
  const field = 'permission.network'
  const raw = objectAt(value, field)
  knownFields(raw, ['version', 'default', 'allow', 'deny', 'inbound', 'hostLoopback'], field)
  const legacy = raw.version === undefined
  if (!legacy && raw.version !== 2)
    throw new PermissionConfigError('Invalid permission value at {field}: expected {expected}.', {
      field: field + '.version',
      expected: '2',
    })
  const result: NetworkPermissionConfig = { version: 2 }
  for (const action of ['default', 'inbound', 'hostLoopback'] as const) {
    if (raw[action] !== undefined) result[action] = networkAction(raw[action], field + '.' + action)
    else if (legacy) result[action] = 'deny'
  }
  for (const action of ['allow', 'deny'] as const) {
    if (raw[action] !== undefined) result[action] = networkRules(raw[action], field + '.' + action)
    else if (legacy) result[action] = []
  }
  return result
}

/** Resolve only known layers; a local field replaces its parent field, including an empty rule list. */
export function mergeNetworkPermissionConfig(
  parent: NetworkPermissionConfig | null | undefined,
  local: NetworkPermissionConfig | null | undefined,
): NetworkPermissionConfig | null {
  if (parent == null && local == null) return null
  return {
    ...(parent == null ? {} : parseNetworkPermissionConfig(parent)),
    ...(local == null ? {} : parseNetworkPermissionConfig(local)),
    version: 2,
  }
}

/** Normalize deterministic legacy values, then validate before replacing a draft or submitting it. */
export function parsePermissionConfig(value: unknown): PermissionConfig {
  const raw = objectAt(value, 'permission')
  knownFields(raw, ['path', 'network', 'tools', 'plugins', 'approval_model'], 'permission')
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
          if (parentMode === 'deny' && path.startsWith(parent.endsWith('/') ? parent : parent + '/'))
            throw new PermissionConfigError('Path rule {path} cannot reopen denied path {parent}.', { path, parent })
        }
      }
      result.path.rules = Object.fromEntries(rules)
    }
  }
  if (raw.network !== undefined) result.network = parseNetworkPermissionConfig(raw.network)
  if (raw.plugins !== undefined) {
    const entries = Object.entries(objectAt(raw.plugins, 'permission.plugins'))
    for (const [id, enabled] of entries) {
      const parts = id.split('.')
      if (parts.length !== 2 || parts.some((part) => !part || part.trim() !== part))
        throw new PermissionConfigError('Invalid plugin ID {id}. Use the full plugin ID.', { id })
      if (typeof enabled !== 'boolean')
        throw new PermissionConfigError('Invalid permission value at {field}: expected {expected}.', {
          field: `permission.plugins[${JSON.stringify(id)}]`,
          expected: 'true | false',
        })
    }
    if (entries.length) result.plugins = Object.fromEntries(entries) as Record<string, boolean>
  }
  if (raw.tools != null) {
    const tools = objectAt(raw.tools, 'permission.tools')
    knownFields(tools, ['default', 'names', 'rules'], 'permission.tools')
    result.tools = {}
    if (tools.default != null) result.tools.default = permissionMode(tools.default, 'permission.tools.default')
    if (tools.names !== undefined) result.tools.names = modeMap(tools.names, 'permission.tools.names')
    if (tools.rules !== undefined) {
      result.tools.rules = Object.fromEntries(
        Object.entries(objectAt(tools.rules, 'permission.tools.rules')).map(([tool, rules]) => [
          tool,
          typeof rules === 'string' && tool !== SHELL_COMMAND_RULE_GROUP
            ? permissionMode(rules, `permission.tools.rules[${JSON.stringify(tool)}]`)
            : modeMap(rules, `permission.tools.rules[${JSON.stringify(tool)}]`),
        ]),
      )
      shareLegacyCommandRules(result.tools.rules)
    }
  }
  if (raw.approval_model != null) result.approval_model = raw.approval_model as JsonValue
  return result
}
