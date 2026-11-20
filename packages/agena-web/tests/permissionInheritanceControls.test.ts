import { afterEach, describe, expect, test } from 'bun:test'
import { createPermissionEditor, disposePermissionEditors } from './helpers/permissionStudioHarness'

afterEach(disposePermissionEditors)

describe('permission inheritance controls', () => {
  test.each(['global', 'workspace', 'session'] as const)(
    '%s unavailable base values never appear as allowed permissions or copied local values',
    async (scope) => {
      const { panel, writes } = await createPermissionEditor({}, null, { scope })
      expect(panel.displayedToolDefault.value).toBeUndefined()
      expect(panel.toolDefaultValueLabel.value).toBe('Unavailable')
      expect(panel.displayedReadOnlyMode.value).toBeUndefined()
      expect(panel.readOnlyValueMode.value).toBe('__unavailable__')
      expect(panel.readOnlyValueLabel.value).toBe('Unavailable')
      expect(panel.readOnlyInheritedValueLabel()).toBe('Unavailable')
      expect(panel.pathModeLabel(panel.displayedPathMode('workspace'))).toBe('Unavailable')
      expect(panel.ruleRowsByKind.value.command).toEqual([])
      panel.setPathSource('workspace', 'local')
      panel.setToolDefaultSource('local')
      panel.setReadOnlySource('local')
      expect(panel.config.value).toEqual({})
      await panel.save()
      expect(writes).toHaveLength(0)
      panel.setPathDefault('workspace', 'deny')
      panel.setToolDefault('ask')
      panel.setReadOnlyMode('deny')
      await panel.save()
      expect(writes.at(-1)?.value).toEqual({
        path: { workspace: 'deny' },
        tools: { default: 'ask', rules: { '*': { 'read-only': 'deny' } } },
      })
      expect(panel.displayedReadOnlyMode.value).toBe('deny')
      expect(panel.displayedToolDefault.value).toBe('ask')
    },
  )

  test('read-only Auto stays unresolved until the actual tool default is known', async () => {
    const { panel } = await createPermissionEditor({ tools: { rules: { '*': { 'read-only': 'auto' } } } }, null)
    expect(panel.readOnlyValueMode.value).toBe('auto')
    expect(panel.displayedReadOnlyMode.value).toBeUndefined()
    expect(panel.readOnlyModeOptions.value.find((option: any) => option.value === 'auto').label).toContain(
      'Unavailable',
    )
    panel.setToolDefault('deny')
    expect(panel.displayedReadOnlyMode.value).toBe('deny')
  })

  test('a known explicit empty wildcard group retains the runtime read-only fallback', async () => {
    const { panel } = await createPermissionEditor({ tools: { rules: { '*': {} } } }, null)
    expect(panel.readOnlySource.value).toBe('local')
    expect(panel.displayedReadOnlyMode.value).toBe('allow')
    expect(panel.displayedReadOnlyUnknown.value).toBe(false)
  })

  for (const scope of ['workspace', 'session'] as const) {
    test(`${scope} pins inherited defaults without changing their behavior and restores only their overrides`, async () => {
      const parent = {
        path: { workspace: 'readwrite', external: 'readonly' },
        tools: { default: 'ask', rules: { '*': { 'read-only': 'auto' } } },
      } as const
      const editor = await createPermissionEditor({ path: { rules: { '/local': 'deny' } } }, parent, {
        scope,
        sessionId: '42',
      })
      const { panel } = editor
      expect(panel.pathSource('workspace')).toBe('inherit')
      expect(panel.toolDefaultSource.value).toBe('inherit')
      expect(panel.readOnlySource.value).toBe('inherit')
      expect(panel.readOnlyValueMode.value).toBe('auto')
      panel.setPathSource('workspace', 'local')
      panel.setPathSource('external', 'local')
      panel.setToolDefaultSource('local')
      panel.setReadOnlySource('local')
      expect(panel.pathSource('workspace')).toBe('local')
      expect(panel.toolDefaultSource.value).toBe('local')
      expect(panel.readOnlySource.value).toBe('local')
      expect(panel.config.value).toEqual({
        path: { workspace: 'readwrite', external: 'readonly', rules: { '/local': 'deny' } },
        tools: { default: 'ask', rules: { '*': { 'read-only': 'auto' } } },
      })
      await panel.save()
      expect(editor.writes.at(-1)?.scope).toBe(scope)
      if (scope === 'session') expect(editor.sessionWrites.at(-1)?.sessionId).toBe('42')
      panel.setPathSource('workspace', 'inherit')
      panel.setPathSource('external', 'inherit')
      panel.setToolDefaultSource('inherit')
      panel.setReadOnlySource('inherit')
      await panel.save()
      expect(editor.writes.at(-1)?.value).toEqual({ path: { rules: { '/local': 'deny' } } })
      expect(panel.inheritedPermission.value).toEqual(parent)
      expect(panel.pathSource('workspace')).toBe('inherit')
      expect(panel.dirty.value).toBe(false)
    })
  }

  test('pinning read-only Auto retains its relationship to subsequent local default edits', async () => {
    const { panel } = await createPermissionEditor(
      {},
      { tools: { default: 'deny', rules: { '*': { 'read-only': 'auto' } } } },
      { scope: 'workspace' },
    )
    panel.setReadOnlySource('local')
    panel.setToolDefaultSource('local')
    panel.setToolDefault('ask')
    expect(panel.config.value.tools.rules['*']['read-only']).toBe('auto')
    expect(panel.displayedReadOnlyMode.value).toBe('ask')
    panel.setReadOnlySource('inherit')
    expect(panel.config.value).toEqual({ tools: { default: 'ask' } })
    expect(panel.displayedReadOnlyMode.value).toBe('ask')
  })

  test('invalid inherited values cannot be copied as valid local defaults', async () => {
    const { panel } = await createPermissionEditor({}, {
      path: { workspace: 'invalid' },
      tools: { default: 'invalid', rules: { '*': 'deny' } },
    } as any)
    panel.setPathSource('workspace', 'local')
    panel.setToolDefaultSource('local')
    panel.setReadOnlySource('local')
    expect(panel.config.value).toEqual({})
    expect(panel.dirty.value).toBe(false)
  })

  test('editing an inherited network value overrides only that field', async () => {
    const network = {
      version: 2,
      default: 'deny',
      inbound: 'deny',
      hostLoopback: 'deny',
      allow: [{ to: '192.0.2.10', ports: [443] }],
      deny: [{ to: '198.51.100.10' }],
    } as const
    const { panel, writes } = await createPermissionEditor({ tools: { default: 'ask' } }, { network } as any)
    panel.setNetworkMode('default', 'allow')
    expect(panel.networkCustom.value).toBe(true)
    expect(panel.config.value.network).toEqual({ version: 2, default: 'allow' })
    expect(panel.inheritedPermission.value.network).toEqual(network)
    await panel.save()
    panel.setNetworkSource('default', 'inherit')
    await panel.save()
    expect(writes.at(-1)?.value).toEqual({ tools: { default: 'ask' } })
    expect(panel.networkMode('default')).toBe('deny')
  })

  test('inherited paths and tool names can be copied and restored individually', async () => {
    const parent = { path: { rules: { '/docs': 'readonly' } }, tools: { names: { 'agena.fs.read': 'ask' } } } as const
    const { panel } = await createPermissionEditor({}, parent, { scope: 'workspace' })
    await panel.changeRuleSource(panel.ruleRowsByKind.value.filesystem[0])
    await panel.changeRuleSource(panel.ruleRowsByKind.value['tool-name'][0])
    expect(panel.config.value).toEqual(parent)
    for (const kind of ['filesystem', 'tool-name']) {
      const row = panel.ruleRowsByKind.value[kind][0]
      expect(row.source).toBe('local')
      expect(row.sourceAction).toBe('inherit')
      expect(row.removable).toBe(false)
      await panel.changeRuleSource(row)
    }
    expect(panel.config.value).toEqual({})
    expect(panel.dirty.value).toBe(false)
  })

  test('command source controls copy and restore the shared table with explicit confirmation', async () => {
    const rules = { routine: 'auto', 'git *': 'ask', 'git status': 'allow', dangerous: 'deny' } as const
    const otherRules = { 'custom.tool': 'ask' } as const
    const editor = await createPermissionEditor(
      { tools: { rules: otherRules } },
      { tools: { rules: { 'shell.*': rules } } },
      { scope: 'workspace' },
    )
    const { panel } = editor
    panel.copyCommandRules()
    expect(panel.config.value.tools.rules).toEqual({ ...otherRules, 'shell.*': rules })
    expect(Object.keys(panel.config.value.tools.rules['shell.*'])).toEqual(Object.keys(rules))
    panel.setCommandMode('git status', 'deny')
    await panel.save()
    editor.setConfirmation(false)
    await panel.restoreCommandRules()
    expect(panel.config.value.tools.rules['shell.*']['git status']).toBe('deny')
    expect(editor.confirmations.at(-1)?.title).toBe('Restore inherited command rules?')
    editor.setConfirmation(true)
    await panel.restoreCommandRules()
    expect(panel.config.value.tools.rules).toEqual(otherRules)
    expect(panel.ruleRowsByKind.value.command.every((row: any) => row.source === 'inherited')).toBe(true)
    await panel.save()
    expect(editor.writes.at(-1)?.value).toEqual({ tools: { rules: otherRules } })
  })

  test('an effective scope and a pending raw draft cannot materialize overrides through source controls', async () => {
    const { panel } = await createPermissionEditor({}, undefined, { scope: 'effective' })
    panel.setPathSource('workspace', 'local')
    panel.setToolDefaultSource('local')
    panel.setReadOnlySource('local')
    panel.copyCommandRules()
    expect(panel.config.value).toEqual({})
    const draft = await createPermissionEditor()
    draft.panel.rawJson.value = '{'
    draft.panel.setPathSource('workspace', 'local')
    draft.panel.copyCommandRules()
    expect(draft.panel.config.value).toEqual({})
  })
})
