import { afterEach, describe, expect, test } from 'bun:test'
import { createPermissionEditor, disposePermissionEditors } from './helpers/permissionStudioHarness'

afterEach(disposePermissionEditors)

const defaults = {
  path: { workspace: 'readwrite', external: 'readonly', rules: { '<state>': 'readwrite' } },
  network: {
    version: 2,
    default: 'allow',
    inbound: 'deny',
    hostLoopback: 'deny',
    allow: [{ to: '192.0.2.10', protocol: 'tcp', ports: [443] }],
    deny: [{ to: '198.51.100.10' }],
  },
  tools: {
    default: 'allow',
    names: { 'agena.web.fetch': 'allow' },
    rules: {
      '*': { 'read-only': 'auto' },
      'shell.*': { 'no-op': 'allow', routine: 'allow', dangerous: 'deny' },
    },
  },
} as const

describe('global permissions edit values and restore defaults', () => {
  test('schema-resolved values are displayed without materializing them into the raw permission draft', async () => {
    const resolved = {
      ...defaults,
      path: { ...defaults.path, workspace: 'deny' },
      network: { ...defaults.network, default: 'deny' },
      tools: { ...defaults.tools, default: 'ask' },
    }
    const { panel, writes } = await createPermissionEditor({}, defaults as any, { resolvedPolicy: resolved as any })
    expect(panel.displayedPathMode('workspace')).toBe('deny')
    expect(panel.networkMode('default')).toBe('deny')
    expect(panel.displayedToolDefault.value).toBe('ask')
    expect(panel.displayedReadOnlyMode.value).toBe('ask')
    expect(panel.config.value).toEqual({})
    expect(JSON.parse(panel.rawJson.value)).toEqual({})
    await panel.save()
    expect(writes).toEqual([])
    panel.setPathDefault('external', 'deny')
    await panel.save()
    expect(writes.at(-1)?.value).toEqual({ path: { external: 'deny' } })
    expect(panel.resolvedPermission.value).toBeNull()
    expect(panel.displayedPathMode('external')).toBe('deny')
  })

  test('older global responses retain their default projection through the legacy alias', async () => {
    const { panel } = await createPermissionEditor({}, defaults as any, { legacySettingsMetadata: true })
    expect(panel.displayedPathMode('workspace')).toBe('readwrite')
    expect(panel.displayedToolDefault.value).toBe('allow')
    expect(panel.config.value).toEqual({})
  })

  test('refreshing a dirty raw draft cannot attach a different server projection after discard', async () => {
    const options = { resolvedPolicy: { ...defaults, tools: { ...defaults.tools, default: 'ask' } } as any }
    const editor = await createPermissionEditor({ tools: { default: 'ask' } }, defaults as any, options)
    const { panel, writes } = editor
    panel.setPathDefault('workspace', 'deny')
    editor.setServerPolicy({ tools: { default: 'deny' } })
    options.resolvedPolicy = { ...defaults, tools: { ...defaults.tools, default: 'deny' } }
    await panel.load()
    expect(panel.config.value).toEqual({ path: { workspace: 'deny' }, tools: { default: 'ask' } })
    panel.discard()
    expect(panel.config.value).toEqual({ tools: { default: 'ask' } })
    expect(panel.displayedToolDefault.value).toBe('ask')
    expect(panel.cleanResolvedPermission.value).toBeNull()
    await panel.save()
    expect(writes).toEqual([])
  })

  test('missing values are displayed without materializing a selectable default layer', async () => {
    const { panel, writes } = await createPermissionEditor({}, defaults as any)
    expect(panel.displayedPathMode('workspace')).toBe('readwrite')
    expect(panel.networkMode('default')).toBe('allow')
    expect(panel.inheritedCommandRules.value).toContainEqual(['routine', 'allow'])
    panel.setPathSource('workspace', 'local')
    panel.setToolDefaultSource('local')
    panel.setReadOnlySource('local')
    panel.setNetworkSource('default', 'local')
    panel.setNetworkSource('allow', 'local')
    panel.copyCommandRules()
    expect(panel.config.value).toEqual({})
    expect(panel.dirty.value).toBe(false)
    expect(writes).toEqual([])
  })

  test('direct editing writes only edited settings and restoring one preserves the others', async () => {
    const { panel, writes } = await createPermissionEditor({}, defaults as any)
    panel.setPathDefault('workspace', 'deny')
    panel.setNetworkMode('inbound', 'deny')
    panel.setToolDefault('ask')
    await panel.save()
    expect(writes.at(-1)?.value).toEqual({
      path: { workspace: 'deny' },
      network: { version: 2, inbound: 'deny' },
      tools: { default: 'ask' },
    })
    panel.setPathSource('workspace', 'inherit')
    expect(panel.displayedPathMode('workspace')).toBe('readwrite')
    await panel.save()
    expect(writes.at(-1)?.value).toEqual({
      network: { version: 2, inbound: 'deny' },
      tools: { default: 'ask' },
    })
    expect(panel.restoreSourceLabel.value).toBe('Reset to defaults')
  })

  test.each([
    ['filesystem', '<state>'],
    ['tool-name', 'agena.web.fetch'],
  ] as const)('built-in %s rules are directly editable and have a separate reset action', async (kind, target) => {
    const { panel } = await createPermissionEditor({}, defaults as any)
    const original = panel.ruleRowsByKind.value[kind].find((row: any) => row.target === target)
    expect(original.modeOptions.length).toBeGreaterThan(0)
    expect(original.sourceLabel).toBe('')
    expect(original.sourceAction).toBeUndefined()
    expect(original.resetAction).toBe(false)
    panel.setRuleRowMode(original, 'deny')
    const changed = panel.ruleRowsByKind.value[kind].find((row: any) => row.target === target)
    expect(changed.mode).toBe('deny')
    expect(changed.sourceAction).toBeUndefined()
    expect(changed.resetAction).toBe(true)
    await panel.resetRuleRow(changed)
    expect(panel.ruleRowsByKind.value[kind].find((row: any) => row.target === target).mode).toBe(original.mode)
    expect(panel.config.value).toEqual({})
    expect(panel.dirty.value).toBe(false)
  })

  test('editing a built-in command copies the shared table and reset restores it', async () => {
    const editor = await createPermissionEditor({ path: { external: 'deny' } }, defaults as any)
    const { panel } = editor
    const row = panel.ruleRowsByKind.value.command.find((item: any) => item.target === 'routine')
    panel.setRuleRowMode(row, 'ask')
    expect(panel.config.value.tools.rules['shell.*']).toEqual({
      'no-op': 'allow',
      routine: 'ask',
      dangerous: 'deny',
    })
    editor.setConfirmation(false)
    await panel.restoreCommandRules()
    expect(panel.config.value.tools.rules['shell.*'].routine).toBe('ask')
    expect(editor.confirmations.at(-1)?.confirmText).toBe('Reset to defaults')
    expect(editor.confirmations.at(-1)?.title).toBe('Restore default command rules?')
    editor.setConfirmation(true)
    await panel.restoreCommandRules()
    expect(panel.config.value).toEqual({ path: { external: 'deny' } })
    expect(panel.inheritedCommandRules.value).toContainEqual(['routine', 'allow'])
  })

  test('network rule editing and reset affect one list while explicit empty lists remain local', async () => {
    const { panel, writes } = await createPermissionEditor({ network: { version: 2, deny: [] } }, defaults as any)
    panel.editNetworkRule('allow', 0)
    panel.networkRuleDraft.value.ports = '8443'
    expect(panel.applyNetworkRule()).toBe(true)
    await panel.save()
    expect(writes.at(-1)?.value).toEqual({
      network: { version: 2, allow: [{ to: '192.0.2.10', protocol: 'tcp', ports: [8443] }], deny: [] },
    })
    panel.setNetworkSource('allow', 'inherit')
    await panel.save()
    expect(writes.at(-1)?.value).toEqual({ network: { version: 2, deny: [] } })
    expect(panel.networkRules('allow')[0].ports).toEqual([443])
  })

  test('restoring the permission page uses a clear global confirmation and leaves a saveable reset', async () => {
    const editor = await createPermissionEditor({ tools: { default: 'deny' } }, defaults as any)
    await editor.panel.clearSource()
    expect(editor.confirmations.at(-1)?.title).toBe('Restore default global permissions?')
    expect(editor.panel.displayedToolDefault.value).toBe('allow')
    expect(editor.writes).toHaveLength(0)
    await editor.panel.save()
    expect(editor.writes).toEqual([{ path: 'permission', value: undefined, scope: 'global' }])
  })

  test('an unconfigured global network JSON editor can save an explicit offline policy directly', async () => {
    const { panel, writes } = await createPermissionEditor({}, defaults as any)
    expect(JSON.parse(panel.networkJson.value)).toEqual({ version: 2 })
    expect(panel.dirty.value).toBe(false)
    await panel.save()
    expect(writes).toHaveLength(0)
    panel.networkJson.value = '{}'
    await panel.save()
    expect(writes.at(-1)?.value).toEqual({
      network: { version: 2, default: 'deny', inbound: 'deny', hostLoopback: 'deny', allow: [], deny: [] },
    })
    expect(panel.networkMode('default')).toBe('deny')
  })
})
