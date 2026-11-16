import { describe, expect, test } from 'bun:test'

import {
  canonicalPermissionPath,
  parsePermissionConfig,
  parseNetworkPermissionConfig,
  permissionConfigErrorMessage,
  renamePermissionRule,
  type PermissionMode,
} from '../src/lib/permissionConfig'

describe('sandbox-compatible permission configuration', () => {
  test('renaming a command rule preserves later restrictions', () => {
    const commands: Record<string, PermissionMode> = { 'git *': 'allow', 'git push *': 'deny' }
    const config = parsePermissionConfig({
      tools: { rules: { 'agena.shell.exec': renamePermissionRule(commands, 'git *', 'git p*') } },
    })
    expect(Object.entries(config.tools!.rules!['agena.shell.exec']!)).toEqual([
      ['git p*', 'allow'],
      ['git push *', 'deny'],
    ])
    expect(Object.keys(commands)).toEqual(['git *', 'git push *'])
  })

  test('keeps canonical path strings and unrelated tool approval', () => {
    const config = {
      path: {
        workspace: 'deny',
        external: 'deny',
        rules: {
          '<workspace>/input': 'readonly',
          '<workspace>/output': 'readwrite',
          '<workspace>/secrets': 'deny',
        },
      },
      network: { egress: { default: 'deny' }, ingress: { default: 'deny', hostLoopback: 'deny' } },
      tools: { names: { 'agena.shell.exec': 'ask' } },
    }
    expect(parsePermissionConfig(config)).toEqual(config)
  })

  test('normalizes deterministic legacy defaults and matched root/subtree pairs', () => {
    expect(
      parsePermissionConfig({
        path: {
          workspace: { read: 'allow', write: 'deny' },
          external: { read: 'deny', write: 'deny' },
          rules: {
            '<workspace>/input': { read: 'allow', write: 'deny' },
            '<workspace>/input/**': { read: 'allow', write: 'deny' },
          },
        },
      }),
    ).toEqual({
      path: {
        workspace: 'readonly',
        external: 'deny',
        rules: { '<workspace>/input': 'readonly' },
      },
    })
  })

  test.each([
    { read: 'allow', write: 'ask' },
    { read: 'auto', write: 'auto' },
    { read: 'deny', write: 'allow' },
    { read: 'allow' },
    { write: 'deny' },
  ])('does not silently broaden legacy approval or incomplete modes %p', (mode) => {
    expect(() => parsePermissionConfig({ path: { workspace: mode } })).toThrow('explicit choice')
  })

  test.each(['<workspace>/folder', '<workspace>/folder/**', '<workspace>/file.txt'])(
    'requires explicit confirmation for an unpaired legacy rule %s',
    (path) => {
      const config = { path: { rules: { [path]: { read: 'allow', write: 'deny' } } } }
      const original = JSON.stringify(config)
      expect(() => parsePermissionConfig(config)).toThrow('explicit mode choice')
      expect(JSON.stringify(config)).toBe(original)
    },
  )

  test('rejects legacy pair collisions instead of selecting an order', () => {
    expect(() =>
      parsePermissionConfig({
        path: { rules: { '<workspace>/input': 'readonly', '<workspace>/input/**': 'readwrite' } },
      }),
    ).toThrow('conflicting modes')
  })

  test.each(['**/*.key', '<workspace>/secret?.txt', '<workspace>/prefix*'])('rejects filename wildcards %s', (path) =>
    expect(() => parsePermissionConfig({ path: { rules: { [path]: 'deny' } } })).toThrow('wildcards'),
  )

  test.each(['<unknown>/file', '<workspace>/../secrets', '<home>\u0000/file'])(
    'rejects unresolved or unsafe paths %s',
    (path) => expect(() => parsePermissionConfig({ path: { rules: { [path]: 'readonly' } } })).toThrow(),
  )

  test('denied ancestors cannot be reopened in either JSON order', () => {
    for (const rules of [
      { '<workspace>/secrets': 'deny', '<workspace>/secrets/public.txt': 'readonly' },
      { '<workspace>/secrets/public.txt': 'readonly', '<workspace>/secrets': 'deny' },
    ]) {
      expect(() => parsePermissionConfig({ path: { rules } })).toThrow('cannot reopen denied path')
    }
    expect(
      parsePermissionConfig({
        path: { rules: { '<workspace>/secrets': 'deny', '<workspace>/secrets-public': 'readonly' } },
      }).path?.rules?.['<workspace>/secrets-public'],
    ).toBe('readonly')
  })

  test('default deny permits a concrete single-file exception', () => {
    expect(
      parsePermissionConfig({
        path: { workspace: 'deny', rules: { '<workspace>/result.txt': 'readwrite' } },
      }).path?.rules,
    ).toEqual({ '<workspace>/result.txt': 'readwrite' })
  })

  test('normalizes duplicate identical directory boundaries without a wildcard policy', () => {
    expect(
      parsePermissionConfig({
        path: { rules: { '<workspace>/input/': 'readonly', '<workspace>/input/**': 'readonly' } },
      }).path?.rules,
    ).toEqual({ '<workspace>/input': 'readonly' })
    expect(canonicalPermissionPath('C:\\project\\output')).toBe('C:/project/output')
  })

  test('normalizes workspace aliases, relative paths, and Windows drive letters consistently', () => {
    for (const input of ['input', './input/', '<cwd>/input', '<workspace>/./input']) {
      expect(canonicalPermissionPath(input)).toBe('<workspace>/input')
    }
    expect(canonicalPermissionPath('c:\\work\\output')).toBe('C:/work/output')
    expect(() =>
      parsePermissionConfig({
        path: { rules: { input: 'readonly', '<cwd>/input': 'readwrite' } },
      }),
    ).toThrow('conflicting modes')
  })

  test.each([
    '/',
    '/**',
    'C:/',
    'c:relative',
    '//server/share',
    '~/.ssh',
    ' file ',
    '<home>/file:stream',
    'dir/<home>',
  ])('rejects unsupported path syntax %s', (path) => expect(() => canonicalPermissionPath(path)).toThrow())

  test('allows legacy deny migration because it only tightens access', () => {
    expect(
      parsePermissionConfig({
        path: { rules: { '<workspace>/secrets': { read: 'deny', write: 'deny' } } },
      }).path?.rules,
    ).toEqual({ '<workspace>/secrets': 'deny' })
  })

  test('rejects nested readonly/readwrite grants independently of input order', () => {
    for (const [parent, child] of [
      ['readonly', 'readwrite'],
      ['readwrite', 'readonly'],
    ]) {
      for (const entries of [
        [
          ['<workspace>/work', parent],
          ['<workspace>/work/child', child],
        ],
        [
          ['<workspace>/work/child', child],
          ['<workspace>/work', parent],
        ],
      ]) {
        expect(() => parsePermissionConfig({ path: { rules: Object.fromEntries(entries) } })).toThrow('separate roots')
      }
    }
    expect(
      parsePermissionConfig({
        path: { rules: { '<workspace>/work': 'readwrite', '<workspace>/work/private': 'deny' } },
      }).path?.rules?.['<workspace>/work/private'],
    ).toBe('deny')
  })

  test('rejects unknown and invalid fields without dropping them', () => {
    expect(() => parsePermissionConfig({ path: { default: 'deny' } })).toThrow('permission.path.default')
    expect(() => parsePermissionConfig({ path: { workspace: 'ask' } })).toThrow('readonly | readwrite | deny')
    expect(() => parsePermissionConfig({ tools: { names: { 'agena.shell.exec': 'maybe' } } })).toThrow(
      'permission.tools.names',
    )
  })

  test('formats validation messages through the settings translator', () => {
    try {
      parsePermissionConfig({ path: { workspace: 'invalid' } })
      throw new Error('expected validation failure')
    } catch (reason) {
      expect(permissionConfigErrorMessage(reason, (key, params) => `translated:${key}:${params?.field}`)).toContain(
        'translated:Invalid permission value',
      )
    }
  })
})

describe('native MXC network configuration', () => {
  test.each([
    ['permission.network', null],
    ['permission.network.egress', { egress: null }],
    ['permission.network.ingress', { ingress: null }],
    ['permission.network.egress.default', { egress: { default: null } }],
    ['permission.network.egress.allow', { egress: { allow: null } }],
    ['permission.network.egress.deny', { egress: { deny: null } }],
    ['permission.network.ingress.default', { ingress: { default: null } }],
    ['permission.network.ingress.hostLoopback', { ingress: { hostLoopback: null } }],
    ['permission.network.egress.allow[0].to', { egress: { allow: [{ to: null }] } }],
    ['permission.network.egress.allow[0].ports', { egress: { allow: [{ ports: null }] } }],
    [
      'permission.network.egress.allow[0].to[0].except',
      { egress: { allow: [{ to: [{ cidr: '192.0.2.0/24', except: null }] }] } },
    ],
    ['permission.network.egress.allow[0].ports[0].protocol', { egress: { allow: [{ ports: [{ protocol: null }] }] } }],
    ['permission.network.egress.allow[0].ports[0].port', { egress: { allow: [{ ports: [{ port: null }] }] } }],
    ['permission.network.egress.allow[0].ports[0].endPort', { egress: { allow: [{ ports: [{ endPort: null }] }] } }],
  ])('rejects explicit null instead of inheriting or widening %s', (field, network) => {
    expect(() => parsePermissionConfig({ network })).toThrow(field as string)
  })

  test.each([
    { egress: { allow: [{ to: [{ cidr: '192.0.2.7/24' }] }] } },
    { egress: { allow: [{ to: [{ cidr: '2001:db8::1/64' }] }] } },
    { egress: { allow: [{ to: [{ cidr: '::ffff:192.0.2.7/120' }] }] } },
    { egress: { deny: [{ to: [{ cidr: '192.0.2.0/24', except: ['192.0.2.129/25'] }] }] } },
    { egress: { deny: [{ to: [{ cidr: '2001:db8::/32', except: ['2001:db8::1/64'] }] }] } },
  ])('rejects nonzero CIDR host bits before native compilation: %p', (network) => {
    expect(() => parseNetworkPermissionConfig(network)).toThrow('network address')
  })

  test('preserves complex policies and explicit empty override lists', () => {
    const network = {
      egress: {
        default: 'deny',
        allow: [
          {
            to: [
              { cidr: '10.20.0.0/16', except: ['10.20.10.0/24'] },
              { cidr: 'fd20:20::/48', except: ['fd20:20:0:10::/64'] },
            ],
            ports: [
              { protocol: 'tcp', port: 443 },
              { protocol: 'udp', port: 8000, endPort: 8099 },
            ],
          },
        ],
        deny: [],
      },
      ingress: { default: 'deny', hostLoopback: 'deny' },
    }
    expect(parseNetworkPermissionConfig(network)).toEqual(network)
    expect(parseNetworkPermissionConfig({})).toEqual({})
    expect(parseNetworkPermissionConfig({ egress: { allow: [{}] } })).toEqual({ egress: { allow: [{}] } })
  })

  test.each([
    { internet: 'allow' },
    { rules: { 'example.com': 'allow' } },
    { egress: { default: 'ask' } },
    { ingress: { host_loopback: 'deny' } },
    { egress: { allow: [{ to: [] }] } },
    { egress: { allow: [{ ports: [] }] } },
    { egress: { allow: [{ to: [{ cidr: 'example.com/32' }] }] } },
    { egress: { allow: [{ to: [{ cidr: 'https://example.com' }] }] } },
    { egress: { allow: [{ to: [{ cidr: '127.0.0.1' }] }] } },
    { egress: { allow: [{ to: [{ cidr: '10.0.0.0/8', except: ['192.168.0.0/16'] }] }] } },
    { egress: { allow: [{ to: [{ cidr: '10.0.0.0/8', except: ['::/0'] }] }] } },
    { egress: { allow: [{ to: [{ cidr: '::/129' }] }] } },
    { egress: { allow: [{ to: [{ cidr: '1::2::3/128' }] }] } },
    { egress: { allow: [{ to: [{ cidr: '10.00.0.1/32' }] }] } },
    { egress: { allow: [{ ports: [{ port: 0 }] }] } },
    { egress: { allow: [{ ports: [{ port: 65536 }] }] } },
    { egress: { allow: [{ ports: [{ port: 443.5 }] }] } },
    { egress: { allow: [{ ports: [{ port: '443' }] }] } },
    { egress: { allow: [{ ports: [{ endPort: 443 }] }] } },
    { egress: { allow: [{ ports: [{ port: 443, endPort: 80 }] }] } },
    { egress: { allow: [{ ports: [{ protocol: 'icmp', port: 443 }] }] } },
    { egress: { allow: [{ ports: [{ protocol: 'sctp' }] }] } },
  ])('rejects unsupported native policy without silently deleting it: %p', (network) => {
    const before = JSON.stringify(network)
    expect(() => parseNetworkPermissionConfig(network)).toThrow()
    expect(JSON.stringify(network)).toBe(before)
  })

  test('accepts full IPv4/IPv6 ranges, literal hosts, ICMP and any protocol', () => {
    for (const cidr of ['0.0.0.0/0', '192.0.2.1/32', '::/0', '2001:db8::1/128', '::ffff:192.0.2.1/128']) {
      expect(
        parseNetworkPermissionConfig({ egress: { allow: [{ to: [{ cidr }] }] } }).egress?.allow?.[0]?.to?.[0]?.cidr,
      ).toBe(cidr)
    }
    for (const ports of [[{ protocol: 'icmp' }], [{ protocol: 'any' }], [{ port: 53 }]]) {
      expect(parseNetworkPermissionConfig({ egress: { allow: [{ ports }] } }).egress?.allow?.[0]?.ports).toEqual(ports)
    }
  })
})
