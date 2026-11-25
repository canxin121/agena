import { afterEach, describe, expect, test } from 'bun:test'
import { nextTick } from 'vue'
import { canonicalPermissionToolName, type PermissionConfig } from '../src/lib/permissionConfig'
import { createPermissionEditor, disposePermissionEditors } from './helpers/permissionStudioHarness'

afterEach(disposePermissionEditors)

describe('permission policy presentation', () => {
  test('projects four separate groups with local and inherited rules kept in their own type', async () => {
    const { panel } = await createPermissionEditor(
      {
        path: { rules: { '/docs': 'readonly' } },
        network: { allow: [{ to: '192.0.2.10', ports: [443] }], deny: [{ to: '198.51.100.9' }] },
        tools: { names: { 'agena.fs.read': 'ask' }, rules: { 'agena.shell.exec': { 'git status': 'allow' } } },
      },
      {
        path: { rules: { '<state>': 'readwrite' } },
        tools: { names: { 'agena.fs.write': 'deny' }, rules: { 'agena.shell.spawn': { 'npm test': 'ask' } } },
      },
    )
    expect(panel.ruleGroups.value.map((group: { kind: string }) => group.kind)).toEqual([
      'filesystem',
      'network',
      'tool-name',
      'command',
    ])
    for (const group of panel.ruleGroups.value) {
      expect(group.rows).toHaveLength(group.kind === 'command' ? 1 : 2)
      expect(group.rows.every((row: { kind: string }) => row.kind === group.kind)).toBe(true)
      expect(group.selectedKeys).toEqual([])
    }
    expect(panel.ruleRowsByKind.value.network.map((row: { key: string }) => row.key)).toEqual([
      'network:allow:0',
      'network:deny:0',
    ])
    expect(panel.ruleRowsByKind.value.filesystem[1].source).toBe('inherited')
    expect(panel.dirty.value).toBe(false)
  })

  test('selection and bulk actions remain isolated when different rule groups are selected together', async () => {
    const { panel } = await createPermissionEditor({
      path: { rules: { '/docs': 'readwrite' } },
      tools: { names: { 'agena.fs.read': 'ask' }, rules: { 'agena.shell.exec': { 'git status': 'allow' } } },
    })
    const groups = panel.ruleRowsByKind.value
    panel.selectVisibleRuleRows('filesystem', [...groups.filesystem, ...groups['tool-name']], true)
    expect(panel.selectedRuleKeysByKind.value).toEqual({
      filesystem: ['path:/docs'],
      network: [],
      'tool-name': [],
      command: [],
    })
    panel.selectVisibleRuleRows('tool-name', groups['tool-name'], true)
    panel.selectVisibleRuleRows('command', groups.command, true)
    expect(panel.ruleGroups.value.map((group: { selectedKeys: string[] }) => group.selectedKeys.length)).toEqual([
      1, 0, 1, 1,
    ])
    panel.bulkPathMode.value = 'deny'
    panel.applySelectedPathMode()
    expect(panel.config.value.path.rules['/docs']).toBe('deny')
    expect(panel.config.value.tools.names['agena.fs.read']).toBe('ask')
    expect(panel.config.value.tools.rules['shell.*']['git status']).toBe('allow')
    panel.clearPathRuleSelection()
    expect(panel.selectedRuleKeysByKind.value.filesystem).toEqual([])
    expect(panel.selectedRuleKeysByKind.value['tool-name']).toHaveLength(1)
    expect(panel.selectedRuleKeysByKind.value.command).toHaveLength(1)
  })

  test('network row identity keeps action and original list index across separate group edits', async () => {
    const { panel } = await createPermissionEditor({
      network: {
        allow: [{ to: '192.0.2.10' }, { to: '192.0.2.20' }],
        deny: [{ to: '198.51.100.9', ports: [443] }],
      },
    })
    const denied = panel.ruleRowsByKind.value.network.find(
      (row: { networkAction: string }) => row.networkAction === 'deny',
    )
    panel.editRuleRow(denied)
    expect(panel.networkRuleDraft.value).toMatchObject({ action: 'deny', index: 0, to: '198.51.100.9' })
    panel.networkRuleDraft.value.ports = '9443'
    panel.applyNetworkRule()
    panel.removeRuleRow(panel.ruleRowsByKind.value.network[0])
    const remaining = panel.ruleRowsByKind.value.network[0]
    expect(remaining).toMatchObject({ key: 'network:allow:0', target: '192.0.2.20' })
    panel.editRuleRow(remaining)
    expect(panel.networkRuleDraft.value).toMatchObject({ action: 'allow', index: 0, to: '192.0.2.20' })
    expect(panel.config.value.network.deny).toEqual([{ to: '198.51.100.9', ports: [9443] }])
  })

  test('group navigation keeps the draft and current settings window', async () => {
    const editor = await createPermissionEditor()
    Object.assign(editor.route.query, { windowId: 'settings-window' })
    editor.panel.setToolDefault('ask')
    editor.panel.selectRuleGroup('command')
    await nextTick()
    expect(editor.route.hash).toBe('#permission-rules-command')
    expect(editor.route.query).toMatchObject({ view: 'access', scope: 'global', windowId: 'settings-window' })
    expect(editor.panel.config.value.tools.default).toBe('ask')
    expect(editor.panel.dirty.value).toBe(true)
    expect(editor.writes).toHaveLength(0)
  })

  test('normalizes only registered tool identities, including monitor commands', () => {
    expect(canonicalPermissionToolName('shell.exec')).toBe('agena.shell.exec')
    expect(canonicalPermissionToolName('agena_shell_read')).toBe('agena.shell.read')
    expect(canonicalPermissionToolName('monitor.start')).toBe('agena.monitor.start')
    expect(canonicalPermissionToolName('my_custom_tool')).toBe('my_custom_tool')
    expect(canonicalPermissionToolName('shell_exec')).toBe('shell_exec')
    expect(canonicalPermissionToolName('Shell.Exec')).toBe('Shell.Exec')
  })

  test('opening and cancelling an inherited network rule leaves no local policy', async () => {
    const { panel } = await createPermissionEditor()
    panel.beginRule('network')
    expect(panel.config.value).toEqual({})
    expect(panel.networkRuleDraft.value.action).toBe('allow')
    expect(panel.dirty.value).toBe(true)
    panel.cancelNetworkRule()
    expect(panel.config.value).toEqual({})
    expect(panel.dirty.value).toBe(false)
  })

  test('same-identity aliases replace the parent group and edits keep one canonical spelling', async () => {
    const { panel } = await createPermissionEditor(
      {
        tools: {
          names: { 'shell.exec': 'ask' },
          rules: { agena_shell_exec: { 'git status': 'allow', ROUTINE: 'auto' } },
        },
      },
      {
        tools: {
          names: { 'agena.shell.exec': 'allow' },
          rules: { 'agena.shell.exec': { dangerous: 'deny' }, 'monitor.start': { 'npm test': 'ask' } },
        },
      },
    )
    expect(panel.toolNameRules.value).toEqual([['agena.shell.exec', 'ask']])
    expect(panel.inheritedToolNameRules.value).toHaveLength(0)
    expect(panel.inheritedCommandRules.value).toEqual([])
    expect(panel.commandRules.value).toEqual([
      ['git status', 'allow'],
      ['routine', 'auto'],
    ])
    panel.setToolNameMode('agena.shell.exec', 'deny')
    panel.setCommandMode('git status', 'ask')
    expect(panel.config.value.tools.names).toEqual({ 'agena.shell.exec': 'deny' })
    expect(panel.config.value.tools.rules).toEqual({ 'shell.*': { 'git status': 'ask', routine: 'auto' } })
    panel.setCommandMode('routine', 'deny')
    expect(Object.keys(panel.config.value.tools.rules['shell.*'])).toEqual(['git status', 'routine'])
    expect(panel.config.value.tools.rules['shell.*'].routine).toBe('deny')
    panel.removeCommandRule('routine')
    expect(panel.config.value.tools.rules).toEqual({ 'shell.*': { 'git status': 'ask' } })
  })

  test('merges inherited terminal aliases and routes mutations by command pattern', async () => {
    const { panel } = await createPermissionEditor(
      {},
      {
        tools: {
          rules: { 'shell.exec': { 'git *': 'ask', 'git status': 'allow' }, agena_shell_watch: { 'npm *': 'deny' } },
        },
      },
    )
    panel.setCommandMode('git status', 'deny')
    expect(panel.config.value.tools.rules).toEqual({
      'shell.*': { 'git *': 'ask', 'git status': 'deny', 'npm *': 'deny' },
    })
    const row = panel.ruleRowsByKind.value.command.find((item: { target: string }) => item.target === 'git status')
    panel.selectRuleRow(row, true)
    expect(panel.selectedCommandRules.value).toEqual([['git status', 'deny']])
    panel.setRuleRowMode(row, 'ask')
    expect(panel.config.value.tools.rules['shell.*']['git status']).toBe('ask')
    panel.removeRuleRow(row)
    expect(panel.config.value.tools.rules['shell.*']).toEqual({ 'git *': 'ask', 'npm *': 'deny' })
  })

  test('alias duplicate creation is rejected and rename removes the old spelling', async () => {
    const { panel } = await createPermissionEditor({
      tools: {
        names: { 'shell.exec': 'ask' },
        rules: { agena_shell_exec: { 'git status': 'allow', 'git push *': 'deny' } },
      },
    })
    panel.newToolName.value = 'agena.shell.exec'
    panel.addToolName()
    expect(panel.error.value).toContain('already exists')
    expect(panel.config.value.tools.names).toEqual({ 'shell.exec': 'ask' })
    panel.renameToolName('agena.shell.exec', { target: { value: 'shell.spawn' } })
    expect(panel.config.value.tools.names).toEqual({ 'agena.shell.spawn': 'ask' })
    panel.renameCommandRule('git status', { target: { value: 'git diff' } })
    expect(panel.config.value.tools.rules).toEqual({
      'shell.*': { 'git diff': 'allow', 'git push *': 'deny' },
    })
  })

  test('empty local read-only group displays its own Allow fallback and clear restores the parent', async () => {
    const { panel } = await createPermissionEditor(
      { tools: { rules: { '*': {} } } },
      { tools: { rules: { '*': { READ_ONLY: 'ask' } } } },
    )
    expect(panel.displayedReadOnlyMode.value).toBe('allow')
    expect(panel.readOnlyEditorMode.value).toBe('allow')
    panel.setReadOnlyMode('')
    expect(panel.displayedReadOnlyMode.value).toBe('ask')
    expect(panel.readOnlyEditorMode.value).toBe('')
  })

  test('read-only class aliases follow ASCII case and underscore normalization when edited or cleared', async () => {
    const { panel } = await createPermissionEditor({
      tools: { rules: { '*': { 'read-only': 'allow', READ_ONLY: 'deny', ReadOnly: 'ask' } } },
    })
    expect(panel.displayedReadOnlyMode.value).toBe('ask')
    panel.setReadOnlyMode('auto')
    expect(panel.config.value.tools.rules['*']).toEqual({ 'read-only': 'auto' })
    panel.setReadOnlyMode('')
    expect(panel.config.value.tools?.rules?.['*']).toBeUndefined()
  })
  test('shows inherited values and fixed read-only fallback without materializing a draft', async () => {
    const { panel, writes } = await createPermissionEditor(
      {},
      {
        path: { workspace: 'readonly', external: 'deny' },
        network: { default: 'deny' },
        tools: { default: 'ask' },
      },
    )
    expect(panel.displayedToolDefault.value).toBe('ask')
    expect(panel.displayedReadOnlyMode.value).toBe('allow')
    expect(panel.toolInheritanceLabel()).toBe('Default · Ask')
    expect(panel.readOnlyInheritanceLabel()).toBe('Default · Allow')
    expect(panel.displayedPathMode('workspace')).toBe('readonly')
    expect(panel.displayedPathMode('external')).toBe('deny')
    expect(panel.networkMode('default')).toBe('deny')
    expect(panel.config.value).toEqual({})
    expect(panel.dirty.value).toBe(false)
    expect(writes).toHaveLength(0)
  })

  test('read-only Auto follows the current default, including an unsaved local override', async () => {
    const { panel } = await createPermissionEditor(
      {},
      {
        tools: { default: 'deny', rules: { '*': { 'read-only': 'auto' } } },
      },
    )
    expect(panel.displayedReadOnlyMode.value).toBe('deny')
    panel.setToolDefault('ask')
    expect(panel.displayedReadOnlyMode.value).toBe('ask')
    expect(panel.readOnlyInheritanceLabel()).toBe('Default · Ask')
    expect(panel.readOnlyModeOptions.value.find((item: { value: string }) => item.value === 'auto').label).toBe(
      'Ask · Follow tool default',
    )
    panel.setReadOnlyMode('allow')
    expect(panel.displayedReadOnlyMode.value).toBe('allow')
  })

  test.each(['global', 'workspace', 'session'] as const)(
    '%s keeps equal-valued overrides explicit and restoring inheritance removes only the selected defaults',
    async (scope) => {
      const initial = {
        path: { rules: { '/keep': 'readonly' } },
        tools: { names: { 'agena.fs.read': 'ask' } },
      } as const
      const inherited = {
        path: { workspace: 'readwrite', external: 'readonly' },
        network: { default: 'allow', inbound: 'deny', hostLoopback: 'deny' },
        tools: { default: 'allow', rules: { '*': { 'read-only': 'allow' } } },
      } as const
      const editor = await createPermissionEditor(initial, inherited, { scope, sessionId: '42' })
      const { panel } = editor

      expect(panel.modeAt('workspace')).toBe('')
      expect(panel.modeAt('external')).toBe('')
      expect(panel.pathInheritanceLabel('workspace')).toBe(
        `${scope === 'global' ? 'Default' : 'Inherit'} · Read and write`,
      )
      expect(panel.pathInheritanceLabel('external')).toBe(`${scope === 'global' ? 'Default' : 'Inherit'} · Read only`)
      expect(panel.toolInheritanceLabel()).toBe(`${scope === 'global' ? 'Default' : 'Inherit'} · Allow`)
      expect(panel.readOnlyEditorMode.value).toBe('')
      expect(panel.readOnlyInheritanceLabel()).toBe(`${scope === 'global' ? 'Default' : 'Inherit'} · Allow`)
      expect(panel.networkCustom.value).toBe(false)
      expect(panel.config.value).toEqual(initial)
      expect(panel.dirty.value).toBe(false)

      panel.setPathSource('workspace', 'local')
      panel.setPathSource('external', 'local')
      panel.setToolDefaultSource('local')
      panel.setReadOnlySource('local')
      panel.setPathDefault('workspace', 'readwrite')
      panel.setPathDefault('external', 'readonly')
      panel.setToolDefault('allow')
      panel.setReadOnlyMode('allow')
      if (scope === 'global') panel.setNetworkMode('default', inherited.network.default)
      else panel.setNetworkSource('default', 'local')
      expect(panel.modeAt('workspace')).toBe('readwrite')
      expect(panel.modeAt('external')).toBe('readonly')
      expect(panel.readOnlyEditorMode.value).toBe('allow')
      expect(panel.networkCustom.value).toBe(true)
      expect(panel.dirty.value).toBe(true)
      await panel.save()
      expect(editor.writes.at(-1)).toEqual({
        path: 'permission',
        scope,
        value: {
          path: { ...initial.path, workspace: 'readwrite', external: 'readonly' },
          network: { version: 2, default: inherited.network.default },
          tools: { ...initial.tools, default: 'allow', rules: { '*': { 'read-only': 'allow' } } },
        },
      })

      panel.setPathDefault('workspace', '')
      panel.setPathDefault('external', '')
      panel.setToolDefault('')
      panel.setReadOnlyMode('')
      panel.setNetworkSource('default', 'inherit')
      expect(panel.config.value).toEqual(initial)
      await panel.save()
      expect(editor.writes.at(-1)).toEqual({ path: 'permission', scope, value: initial })

      editor.setInheritedPolicy({
        path: { workspace: 'deny', external: 'deny' },
        network: { default: 'deny' },
        tools: { default: 'ask', rules: { '*': { 'read-only': 'deny' } } },
      })
      await panel.load()
      expect(panel.pathInheritanceLabel('workspace')).toBe(`${scope === 'global' ? 'Default' : 'Inherit'} · Deny`)
      expect(panel.pathInheritanceLabel('external')).toBe(`${scope === 'global' ? 'Default' : 'Inherit'} · Deny`)
      expect(panel.toolInheritanceLabel()).toBe(`${scope === 'global' ? 'Default' : 'Inherit'} · Ask`)
      expect(panel.readOnlyInheritanceLabel()).toBe(`${scope === 'global' ? 'Default' : 'Inherit'} · Deny`)
      expect(panel.networkMode('default')).toBe('deny')
      expect(panel.config.value).toEqual(initial)
      expect(panel.dirty.value).toBe(false)
      if (scope === 'session') expect(editor.sessionWrites.map((write) => write.sessionId)).toEqual(['42', '42'])
    },
  )

  test('read-only follow-default remains an explicit Auto policy while inheritance stays unset', async () => {
    const { panel, sessionWrites } = await createPermissionEditor(
      {},
      { tools: { default: 'allow', rules: { '*': { 'read-only': 'allow' } } } },
      { scope: 'session', sessionId: '42' },
    )
    panel.setToolDefaultSource('local')
    panel.setToolDefault('ask')
    expect(panel.readOnlyEditorMode.value).toBe('')
    expect(panel.displayedReadOnlyMode.value).toBe('allow')
    expect(panel.readOnlyInheritanceLabel()).toBe('Inherit · Allow')
    panel.setReadOnlySource('local')
    panel.setReadOnlyMode('auto')
    expect(panel.readOnlyEditorMode.value).toBe('auto')
    expect(panel.displayedReadOnlyMode.value).toBe('ask')
    expect(panel.readOnlyModeOptions.value.find((option: { value: string }) => option.value === 'auto')?.label).toBe(
      'Ask · Follow tool default',
    )
    await panel.save()
    expect(sessionWrites).toEqual([
      { sessionId: '42', value: { tools: { default: 'ask', rules: { '*': { 'read-only': 'auto' } } } } },
    ])
    panel.setReadOnlyMode('')
    expect(panel.readOnlyEditorMode.value).toBe('')
    expect(panel.displayedReadOnlyMode.value).toBe('allow')
    await panel.save()
    expect(sessionWrites.at(-1)).toEqual({ sessionId: '42', value: { tools: { default: 'ask' } } })
  })

  test('returning unsaved defaults to inheritance restores a clean draft without empty override containers', async () => {
    const initial = { tools: { names: { 'agena.fs.read': 'ask' } } } as const
    const { panel, writes } = await createPermissionEditor(initial)
    const changes = [
      [() => panel.setPathDefault('workspace', 'readwrite'), () => panel.setPathDefault('workspace', '')],
      [() => panel.setToolDefault('allow'), () => panel.setToolDefault('')],
      [() => panel.setReadOnlyMode('allow'), () => panel.setReadOnlyMode('')],
      [() => panel.setNetworkMode('default', 'allow'), () => panel.setNetworkSource('default', 'inherit')],
    ]
    for (const [override, inherit] of changes) {
      override!()
      expect(panel.dirty.value).toBe(true)
      inherit!()
      expect(panel.config.value).toEqual(initial)
      expect(panel.dirty.value).toBe(false)
    }
    expect(writes).toHaveLength(0)
  })

  test('read-only aliases use ordered last-match semantics and write one canonical class', async () => {
    const { panel } = await createPermissionEditor({
      tools: { default: 'ask', rules: { '*': { 'read-only': 'allow', read_only: 'deny', readonly: 'auto' } } },
    })
    expect(panel.readOnlyEditorMode.value).toBe('auto')
    expect(panel.displayedReadOnlyMode.value).toBe('ask')
    panel.setReadOnlyMode('deny')
    expect(panel.config.value.tools.rules['*']).toEqual({ 'read-only': 'deny' })
    expect(panel.displayedReadOnlyMode.value).toBe('deny')
  })

  test('an invalid scalar wildcard stays visible as raw configuration without inventing an effective mode', async () => {
    const { panel } = await createPermissionEditor({ tools: { rules: { '*': 'deny' } } })
    expect(panel.readOnlyPolicy(panel.config.value)).toBeUndefined()
    expect(panel.displayedReadOnlyMode.value).toBeUndefined()
    expect(panel.readOnlyEditorInvalid.value).toBe(true)
    expect(panel.readOnlyEditorMode.value).toBe('__invalid__')
    expect(panel.readOnlyInheritanceLabel()).toBe('Default · Allow')
    expect(panel.rawJson.value).toContain('"*": "deny"')
    panel.setReadOnlyMode('ask')
    expect(panel.config.value.tools.rules['*']).toEqual({ 'read-only': 'ask' })
  })

  test('restoring inheritance repairs invalid local read-only policy without hiding the parent option', async () => {
    const { panel } = await createPermissionEditor(
      { tools: { rules: { '*': 'deny' } } },
      { tools: { rules: { '*': { 'read-only': 'ask' } } } },
    )
    expect(panel.readOnlyInheritanceLabel()).toBe('Default · Ask')
    panel.setReadOnlyMode('')
    expect(panel.readOnlyEditorInvalid.value).toBe(false)
    expect(panel.readOnlyEditorMode.value).toBe('')
    expect(panel.displayedReadOnlyMode.value).toBe('ask')
    expect(panel.config.value).toEqual({})
  })

  test('an invalid inherited read-only policy is identified without pretending the fallback is known', async () => {
    const { panel } = await createPermissionEditor({}, { tools: { rules: { '*': 'deny' } } })
    expect(panel.readOnlyEditorInvalid.value).toBe(false)
    expect(panel.readOnlyEditorMode.value).toBe('')
    expect(panel.readOnlyInheritanceLabel()).toBe('Default · Invalid configuration')
    expect(panel.displayedReadOnlyMode.value).toBeUndefined()
    expect(panel.config.value).toEqual({})
  })

  test('unknown wildcard classes stay invalid and cannot claim inheritance while preserving the unknown entries', async () => {
    const initial = { tools: { rules: { '*': { 'read-only': 'deny', 'custom-class': 'ask' } } } } as const
    const { panel } = await createPermissionEditor(initial, { tools: { rules: { '*': { 'read-only': 'ask' } } } })
    expect(panel.readOnlyEditorInvalid.value).toBe(true)
    expect(panel.readOnlyEditorMode.value).toBe('__invalid__')
    expect(panel.displayedReadOnlyMode.value).toBeUndefined()
    expect(panel.readOnlyInheritanceBlocked.value).toBe(true)
    panel.setReadOnlyMode('')
    expect(panel.config.value).toEqual(initial)
    expect(panel.dirty.value).toBe(false)
  })

  test.each(['unsupported-mode', 42])(
    'invalid path and tool defaults %s remain distinct from absent overrides and can be restored to inheritance',
    async (invalid) => {
      const initial = { path: { workspace: invalid, external: invalid }, tools: { default: invalid } }
      const { panel, sessionWrites } = await createPermissionEditor(
        initial as unknown as PermissionConfig,
        { path: { workspace: 'readwrite', external: 'readonly' }, tools: { default: 'ask' } },
        { scope: 'session', sessionId: '42' },
      )
      expect(panel.modeAt('workspace')).toBe('__invalid__')
      expect(panel.modeAt('external')).toBe('__invalid__')
      expect(panel.toolDefaultEditorMode.value).toBe('__invalid__')
      expect(panel.displayedToolDefault.value).toBeUndefined()
      expect(panel.config.value).toEqual(initial)
      expect(panel.dirty.value).toBe(false)
      panel.setPathDefault('workspace', '')
      panel.setPathDefault('external', '')
      panel.setToolDefault('')
      expect(panel.modeAt('workspace')).toBe('')
      expect(panel.modeAt('external')).toBe('')
      expect(panel.toolDefaultEditorMode.value).toBe('')
      expect(panel.config.value).toEqual({})
      await panel.save()
      expect(sessionWrites).toEqual([{ sessionId: '42', value: {} }])
      expect(panel.dirty.value).toBe(false)
    },
  )

  test('null optional defaults retain their established absent-value meaning', async () => {
    const { panel, writes } = await createPermissionEditor(
      { path: { workspace: null, external: null }, tools: { default: null } } as unknown as PermissionConfig,
      { path: { workspace: 'readwrite', external: 'readonly' }, tools: { default: 'ask' } },
    )
    expect(panel.modeAt('workspace')).toBe('')
    expect(panel.modeAt('external')).toBe('')
    expect(panel.toolDefaultEditorMode.value).toBe('')
    expect(panel.displayedToolDefault.value).toBe('ask')
    expect(panel.config.value).toEqual({ path: {}, tools: {} })
    expect(panel.dirty.value).toBe(false)
    expect(writes).toHaveLength(0)
  })

  test('rule projection overlays individual names and paths but replaces complete command groups', async () => {
    const { panel } = await createPermissionEditor(
      {
        path: { rules: { '/docs': 'readonly' } },
        tools: {
          names: { 'agena.fs.read': 'deny' },
          rules: { 'agena.shell.exec': { 'git status': 'allow' } },
        },
      },
      {
        path: { rules: { '/docs': 'readwrite', '<state>': 'readwrite' } },
        tools: {
          names: { 'agena.fs.read': 'allow', 'agena.fs.write': 'ask' },
          rules: {
            'agena.shell.exec': { routine: 'auto', 'git push *': 'ask' },
            'agena.shell.spawn': { dangerous: 'deny', 'npm test': 'allow' },
            '*': { 'read-only': 'allow' },
          },
        },
      },
    )
    expect(panel.inheritedPathRules.value).toEqual([['<state>', 'readwrite']])
    expect(panel.inheritedToolNameRules.value).toEqual([['agena.fs.write', 'ask']])
    expect(panel.inheritedCommandRules.value).toEqual([])
    expect(panel.ruleRowsByKind.value.filesystem).toHaveLength(2)
    expect(panel.ruleRowsByKind.value['tool-name']).toHaveLength(2)
    expect(panel.ruleRowsByKind.value.command).toHaveLength(1)
    expect(panel.dirty.value).toBe(false)
  })

  test('first command override copies the shared inherited command table and preserves rule order', async () => {
    const parentRules = { routine: 'auto', 'git *': 'ask', 'git status': 'allow', dangerous: 'deny' } as const
    const { panel, writes } = await createPermissionEditor(
      {},
      {
        tools: { rules: { 'agena.shell.exec': parentRules } },
      },
    )
    panel.setCommandMode('git status', 'deny')
    expect(panel.config.value.tools.rules['shell.*']).toEqual({ ...parentRules, 'git status': 'deny' })
    expect(Object.keys(panel.config.value.tools.rules['shell.*'])).toEqual(Object.keys(parentRules))
    expect(panel.inheritedPermission.value.tools.rules['shell.*']).toEqual(parentRules)
    expect(panel.inheritedCommandRules.value).toHaveLength(0)
    expect(panel.commandRules.value).toHaveLength(4)
    await panel.save()
    expect(writes).toHaveLength(1)
    expect(writes[0]?.value).toEqual({
      tools: { rules: { 'shell.*': { ...parentRules, 'git status': 'deny' } } },
    })
  })

  test('a first category edit preserves inherited custom patterns and shell scalar defaults', async () => {
    const { panel } = await createPermissionEditor(
      {},
      {
        tools: { rules: { 'agena.shell.exec': { 'git push *': 'deny', routine: 'auto' }, 'agena.shell.spawn': 'ask' } },
      },
    )
    panel.setCommandMode('routine', 'allow')
    panel.setCommandMode('npm test', 'allow')
    expect(panel.config.value.tools.rules).toEqual({
      'shell.*': { 'git push *': 'deny', routine: 'allow', 'npm test': 'allow' },
    })
  })

  test('configuration target follows server layer metadata, independent of selected client directory', async () => {
    const editor = await createPermissionEditor({}, undefined, {
      configPaths: { global: '/runtime/global/agena.json', workspace: '/runtime/project/agena.json' },
    })
    expect(editor.panel.configPath.value).toBe('/runtime/global/agena.json')
    await editor.switchScope('workspace')
    expect(editor.panel.configPath.value).toBe('/runtime/project/agena.json')
    editor.directory.currentDirectory = '/other/client/project'
    await nextTick()
    expect(editor.panel.configPath.value).toBe('/runtime/project/agena.json')
    editor.panel.setToolDefaultSource('local')
    editor.panel.setToolDefault('ask')
    await editor.panel.save()
    expect(editor.writes[0]?.scope).toBe('workspace')
    await editor.switchScope('session')
    expect(editor.panel.configPath.value).toBe('')
  })
})
