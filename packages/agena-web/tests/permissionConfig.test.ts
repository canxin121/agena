import { describe, expect, test } from 'bun:test'

import {
  canonicalPermissionPath,
  parsePermissionConfig,
  permissionConfigErrorMessage,
  renamePermissionRule,
  type PermissionMode,
} from '../src/lib/permissionConfig'

describe('sandbox-compatible permission configuration', () => {
  test('renaming a command or network rule preserves later restrictions', () => {
    const commands: Record<string, PermissionMode> = { 'git *': 'allow', 'git push *': 'deny' }
    const network: Record<string, PermissionMode> = { '10.0.0.0/8': 'allow', '10.0.0.1:443': 'deny' }
    const config = parsePermissionConfig({
      tools: { rules: { 'agena.shell.exec': renamePermissionRule(commands, 'git *', 'git p*') } },
      network: { rules: renamePermissionRule(network, '10.0.0.0/8', '10.0.0.0/16') },
    })
    const saved = JSON.parse(JSON.stringify(config))
    expect(Object.entries(saved.tools.rules['agena.shell.exec'])).toEqual([
      ['git p*', 'allow'],
      ['git push *', 'deny'],
    ])
    expect(Object.entries(saved.network.rules)).toEqual([
      ['10.0.0.0/16', 'allow'],
      ['10.0.0.1:443', 'deny'],
    ])
    expect(Object.keys(commands)).toEqual(['git *', 'git push *'])
    expect(Object.keys(network)).toEqual(['10.0.0.0/8', '10.0.0.1:443'])
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
      network: { internet: 'deny', private: 'deny', loopback: 'deny' },
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
