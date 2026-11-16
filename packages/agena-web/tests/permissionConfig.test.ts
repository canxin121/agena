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
      network: { default: 'deny', inbound: 'deny', hostLoopback: 'deny' },
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

  test('nested readonly and readwrite directory rules retain their modes in either JSON order', () => {
    for (const rules of [
      {
        '<workspace>/tree': 'readonly',
        '<workspace>/tree/output': 'readwrite',
        '<workspace>/tree/output/reference': 'readonly',
      },
      {
        '<workspace>/tree': 'readwrite',
        '<workspace>/tree/reference': 'readonly',
        '<workspace>/tree/reference/output': 'readwrite',
      },
    ]) {
      for (const entries of [Object.entries(rules), Object.entries(rules).reverse()]) {
        const config = { path: { workspace: 'readwrite', external: 'readonly', rules: Object.fromEntries(entries) } }
        expect(parsePermissionConfig(config)).toEqual(config)
      }
    }
  })

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

  test('filesystem and volume roots normalize as concrete directory grants', () => {
    for (const [input, expected] of [
      ['/', '/'],
      ['/**', '/'],
      ['/./', '/'],
      ['C:/', 'C:/'],
      ['c:\\', 'C:/'],
      ['C:/**', 'C:/'],
    ]) {
      expect(canonicalPermissionPath(input!)).toBe(expected)
      for (const mode of ['readonly', 'readwrite'])
        expect(parsePermissionConfig({ path: { rules: { [input!]: mode } } }).path?.rules).toEqual({
          [expected!]: mode,
        })
    }
    for (const path of ['C:', 'C:relative', '//server/share', '/tmp/*.key'])
      expect(() => canonicalPermissionPath(path)).toThrow()
  })

  test('denied filesystem and volume roots still prevent reopening descendants', () => {
    for (const [root, child] of [
      ['/', '/tmp/public'],
      ['C:/', 'C:/public'],
    ])
      expect(() => parsePermissionConfig({ path: { rules: { [root!]: 'deny', [child!]: 'readwrite' } } })).toThrow(
        'cannot reopen denied path',
      )
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

  test.each(['C:', 'c:relative', '//server/share', '~/.ssh', ' file ', '<home>/file:stream', 'dir/<home>'])(
    'rejects unsupported path syntax %s',
    (path) => expect(() => canonicalPermissionPath(path)).toThrow(),
  )

  test('allows legacy deny migration because it only tightens access', () => {
    expect(
      parsePermissionConfig({
        path: { rules: { '<workspace>/secrets': { read: 'deny', write: 'deny' } } },
      }).path?.rules,
    ).toEqual({ '<workspace>/secrets': 'deny' })
  })

  test('accepts nested readonly/readwrite grants independently of input order', () => {
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
        expect(parsePermissionConfig({ path: { rules: Object.fromEntries(entries) } }).path?.rules).toEqual(
          Object.fromEntries(entries),
        )
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

describe('flat MXC network configuration', () => {
  test('keeps one target, one protocol and concise port ranges', () => {
    const network = {
      default: 'deny',
      allow: [
        { to: '192.0.2.10', protocol: 'tcp', ports: [443, '8000-8099'] },
        { to: '10.20.0.0/16', except: ['10.20.10.0/24'], protocol: 'udp', ports: [53] },
        { to: '2001:db8::10', protocol: 'icmp' },
        { to: '*', protocol: 'tcp', ports: [443] },
      ],
      deny: [{ to: '192.0.2.11' }],
      inbound: 'deny',
      hostLoopback: 'deny',
    }
    expect(parseNetworkPermissionConfig(network)).toEqual(network)
  })

  test('a present empty object stays an explicit offline policy', () => {
    expect(parsePermissionConfig({ network: {} })).toEqual({ network: {} })
    expect(parsePermissionConfig({})).toEqual({})
    expect(parseNetworkPermissionConfig({ default: 'deny' })).toEqual({ default: 'deny' })
    expect(parseNetworkPermissionConfig({ allow: [], deny: [] })).toEqual({ allow: [], deny: [] })
    expect(parseNetworkPermissionConfig({ allow: [{ to: '*', except: [] }] })).toEqual({
      allow: [{ to: '*', except: [] }],
    })
  })

  test.each([
    ['permission.network', null],
    ['permission.network.default', { default: null }],
    ['permission.network.allow', { allow: null }],
    ['permission.network.deny', { deny: null }],
    ['permission.network.inbound', { inbound: null }],
    ['permission.network.hostLoopback', { hostLoopback: null }],
    ['permission.network.allow[0].to', { allow: [{ to: null }] }],
    ['permission.network.allow[0].protocol', { allow: [{ to: '*', protocol: null }] }],
    ['permission.network.allow[0].ports', { allow: [{ to: '*', ports: null }] }],
    ['permission.network.allow[0].ports[0]', { allow: [{ to: '*', ports: [null] }] }],
    ['permission.network.allow[0].except', { allow: [{ to: '192.0.2.0/24', except: null }] }],
  ])('rejects explicit null at %s', (field, network) => {
    expect(() => parsePermissionConfig({ network })).toThrow(field as string)
  })

  test.each([
    { egress: { default: 'allow' } },
    { ingress: { default: 'allow' } },
    { internet: 'allow' },
    { rules: { 'example.com': 'allow' } },
    { default: 'ask' },
    { host_loopback: 'deny' },
    { allow: [{}] },
    { allow: [{ to: [] }] },
    { allow: [{ to: [{ cidr: '192.0.2.1/32' }] }] },
    { allow: [{ to: 'example.com' }] },
    { allow: [{ to: 'https://example.com' }] },
    { allow: [{ to: '192.0.2.10:443' }] },
    { allow: [{ to: ' 192.0.2.10' }] },
    { allow: [{ to: '10.00.0.1' }] },
    { allow: [{ to: '::/129' }] },
    { allow: [{ to: '1::2::3/128' }] },
    { allow: [{ to: '*', except: ['192.0.2.0/24'] }] },
    { allow: [{ to: '*', except: null }] },
    { allow: [{ to: '10.0.0.0/8', except: ['192.168.0.0/16'] }] },
    { allow: [{ to: '10.0.0.0/8', except: ['::/0'] }] },
    { allow: [{ to: '*', ports: [] }] },
    { allow: [{ to: '*', ports: [0] }] },
    { allow: [{ to: '*', ports: [65536] }] },
    { allow: [{ to: '*', ports: [443.5] }] },
    { allow: [{ to: '*', ports: ['443'] }] },
    { allow: [{ to: '*', ports: ['65535-65536'] }] },
    { allow: [{ to: '*', ports: ['443-80'] }] },
    { allow: [{ to: '*', ports: ['0-443'] }] },
    { allow: [{ to: '*', ports: ['080-443'] }] },
    { allow: [{ to: '*', ports: ['80 - 443'] }] },
    { allow: [{ to: '*', ports: [{ port: 443 }] }] },
    { allow: [{ to: '*', protocol: 'icmp', ports: [443] }] },
    { allow: [{ to: '*', protocol: 'sctp' }] },
  ])('rejects unsupported flat policy without changing it: %p', (network) => {
    const before = JSON.stringify(network)
    expect(() => parseNetworkPermissionConfig(network)).toThrow()
    expect(JSON.stringify(network)).toBe(before)
  })

  test.each([
    { allow: [{ to: '192.0.2.7/24' }] },
    { allow: [{ to: '2001:db8::1/64' }] },
    { allow: [{ to: '::ffff:192.0.2.7/120' }] },
    { deny: [{ to: '192.0.2.0/24', except: ['192.0.2.129/25'] }] },
    { deny: [{ to: '2001:db8::/32', except: ['2001:db8::1/64'] }] },
  ])('rejects nonzero CIDR host bits: %p', (network) => {
    expect(() => parseNetworkPermissionConfig(network)).toThrow('network address')
  })

  test('accepts IPv4, IPv6, wildcard, and explicit any protocol', () => {
    for (const to of [
      '*',
      '192.0.2.1',
      '2001:db8::1',
      '0.0.0.0/0',
      '192.0.2.1/32',
      '::/0',
      '2001:db8::1/128',
      '::ffff:192.0.2.1/128',
    ]) {
      expect(parseNetworkPermissionConfig({ allow: [{ to }] }).allow?.[0]?.to).toBe(to)
    }
    expect(
      parseNetworkPermissionConfig({ allow: [{ to: '*', protocol: 'any', ports: [53, '53-53'] }] }).allow?.[0]?.ports,
    ).toEqual([53, '53-53'])
  })
})
