import type { JsonValue } from '@/types/json'

export type PermissionMode = 'allow' | 'auto' | 'ask' | 'deny'
export type PathPermissionMode = 'readonly' | 'readwrite' | 'deny'
export type NetworkAction = 'allow' | 'deny'
export type NetworkProtocol = 'tcp' | 'udp' | 'icmp' | 'any'
export type NetworkRule = {
  to?: Array<{ cidr: string; except?: string[] }>
  ports?: Array<{ protocol?: NetworkProtocol; port?: number; endPort?: number }>
}
export type NetworkPermissionConfig = {
  egress?: { default?: NetworkAction; allow?: NetworkRule[]; deny?: NetworkRule[] }
  ingress?: { default?: NetworkAction; hostLoopback?: NetworkAction }
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
  approval_model?: JsonValue
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

function networkRules(value: unknown, field: string): NetworkRule[] {
  return arrayAt(value, field).map((value, index) => {
    const at = field + '[' + index + ']'
    const rule = objectAt(value, at)
    knownFields(rule, ['to', 'ports'], at)
    const result: NetworkRule = {}
    if (rule.to !== undefined) {
      result.to = arrayAt(rule.to, at + '.to', true).map((value, peerIndex) => {
        const peerAt = at + '.to[' + peerIndex + ']'
        const peer = objectAt(value, peerAt)
        knownFields(peer, ['cidr', 'except'], peerAt)
        const cidr = networkCidr(peer.cidr, peerAt + '.cidr')
        const result: NonNullable<NetworkRule['to']>[number] = { cidr: cidr.value }
        if (peer.except !== undefined) {
          result.except = arrayAt(peer.except, peerAt + '.except').map((value, exceptIndex) => {
            const exceptAt = peerAt + '.except[' + exceptIndex + ']'
            const excluded = networkCidr(value, exceptAt)
            if (
              excluded.bits !== cidr.bits ||
              excluded.prefix < cidr.prefix ||
              excluded.address >> BigInt(cidr.bits - cidr.prefix) !== cidr.address >> BigInt(cidr.bits - cidr.prefix)
            ) {
              throw new PermissionConfigError('Excluded CIDR at {field} must be contained in {cidr}.', {
                field: exceptAt,
                cidr: cidr.value,
              })
            }
            return excluded.value
          })
        }
        return result
      })
    }
    if (rule.ports !== undefined) {
      result.ports = arrayAt(rule.ports, at + '.ports', true).map((value, portIndex) => {
        const portAt = at + '.ports[' + portIndex + ']'
        const selector = objectAt(value, portAt)
        knownFields(selector, ['protocol', 'port', 'endPort'], portAt)
        const result: NonNullable<NetworkRule['ports']>[number] = {}
        if (selector.protocol !== undefined) {
          if (typeof selector.protocol !== 'string' || !['tcp', 'udp', 'icmp', 'any'].includes(selector.protocol)) {
            throw new PermissionConfigError('Invalid permission value at {field}: expected {expected}.', {
              field: portAt + '.protocol',
              expected: 'tcp | udp | icmp | any',
            })
          }
          result.protocol = selector.protocol as NetworkProtocol
        }
        for (const name of ['port', 'endPort'] as const) {
          if (selector[name] !== undefined) {
            const port = selector[name]
            if (typeof port !== 'number' || !Number.isInteger(port) || port < 1 || port > 65535)
              throw new PermissionConfigError('Port at {field} must be an integer from 1 to 65535.', {
                field: portAt + '.' + name,
              })
            result[name] = port
          }
        }
        if (result.endPort !== undefined && (result.port === undefined || result.endPort < result.port))
          throw new PermissionConfigError(
            'endPort at {field} requires port and must be greater than or equal to port.',
            { field: portAt },
          )
        if (result.protocol === 'icmp' && (result.port !== undefined || result.endPort !== undefined))
          throw new PermissionConfigError('ICMP rule at {field} must not specify ports.', { field: portAt })
        return result
      })
    }
    return result
  })
}

/** Parse native MXC network policy. Backend availability is validated by the server. */
export function parseNetworkPermissionConfig(value: unknown): NetworkPermissionConfig {
  const field = 'permission.network'
  const raw = objectAt(value, field)
  knownFields(raw, ['egress', 'ingress'], field)
  const result: NetworkPermissionConfig = {}
  if (raw.egress !== undefined) {
    const egress = objectAt(raw.egress, field + '.egress')
    knownFields(egress, ['default', 'allow', 'deny'], field + '.egress')
    result.egress = {}
    if (egress.default !== undefined) result.egress.default = networkAction(egress.default, field + '.egress.default')
    for (const action of ['allow', 'deny'] as const) {
      if (egress[action] !== undefined)
        result.egress[action] = networkRules(egress[action], field + '.egress.' + action)
    }
  }
  if (raw.ingress !== undefined) {
    const ingress = objectAt(raw.ingress, field + '.ingress')
    knownFields(ingress, ['default', 'hostLoopback'], field + '.ingress')
    result.ingress = {}
    for (const action of ['default', 'hostLoopback'] as const) {
      if (ingress[action] !== undefined)
        result.ingress[action] = networkAction(ingress[action], field + '.ingress.' + action)
    }
  }
  return result
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
  if (raw.network !== undefined) result.network = parseNetworkPermissionConfig(raw.network)
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
