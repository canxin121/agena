import { afterEach, describe, expect, test } from 'bun:test'
import { nextTick } from 'vue'
import { canonicalPermissionToolName } from '../src/lib/permissionConfig'
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
      expect(group.rows).toHaveLength(2)
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
    expect(panel.config.value.tools.rules['agena.shell.exec']['git status']).toBe('allow')
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
    expect(canonicalPermissionToolName('agena_shell_logs')).toBe('agena.shell.logs')
    expect(canonicalPermissionToolName('monitor.start')).toBe('agena.monitor.start')
    expect(canonicalPermissionToolName('my_custom_tool')).toBe('my_custom_tool')
    expect(canonicalPermissionToolName('shell_exec')).toBe('shell_exec')
    expect(canonicalPermissionToolName('Shell.Exec')).toBe('Shell.Exec')
  })

  test('opening a network rule requires an explicit custom policy so cancellation cannot change inheritance', async () => {
    const { panel } = await createPermissionEditor()
    panel.beginRule('network')
    expect(panel.config.value).toEqual({})
    expect(panel.networkRuleDraft.value).toBeNull()
    expect(panel.dirty.value).toBe(false)
    panel.setNetworkSource('custom')
    const custom = structuredClone(JSON.parse(JSON.stringify(panel.config.value)))
    panel.beginRule('network')
    expect(panel.networkRuleDraft.value.action).toBe('allow')
    panel.cancelNetworkRule()
    expect(panel.config.value).toEqual(custom)
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
    expect(panel.inheritedCommandRules.value).toEqual([
      { tool: 'agena.monitor.start', command: 'npm test', mode: 'ask' },
    ])
    expect(panel.classMode('agena.shell.exec', 'routine')).toBe('auto')
    expect(panel.classInheritanceLabel('agena.shell.exec', 'dangerous')).toBe('Default / unset')
    panel.setToolNameMode('agena.shell.exec', 'deny')
    panel.setCommandMode('agena.shell.exec', 'git status', 'ask')
    expect(panel.config.value.tools.names).toEqual({ 'agena.shell.exec': 'deny' })
    expect(panel.config.value.tools.rules).toEqual({ 'agena.shell.exec': { 'git status': 'ask', ROUTINE: 'auto' } })
    panel.writeRule('agena.shell.exec', 'routine', 'deny')
    expect(Object.keys(panel.config.value.tools.rules['agena.shell.exec'])).toEqual(['git status', 'ROUTINE'])
    expect(panel.classMode('agena.shell.exec', 'routine')).toBe('deny')
    panel.writeRule('agena.shell.exec', 'routine', '')
    expect(panel.config.value.tools.rules).toEqual({ 'agena.shell.exec': { 'git status': 'ask' } })
  })

  test('copying inherited aliases preserves complete groups and the table routes mutations by canonical identity', async () => {
    const { panel } = await createPermissionEditor(
      {},
      {
        tools: {
          rules: { 'shell.exec': { 'git *': 'ask', 'git status': 'allow' }, agena_monitor_start: { 'npm *': 'deny' } },
        },
      },
    )
    panel.setCommandMode('agena.shell.exec', 'git status', 'deny')
    panel.setCommandMode('agena.monitor.start', 'npm test', 'allow')
    expect(panel.config.value.tools.rules).toEqual({
      'agena.shell.exec': { 'git *': 'ask', 'git status': 'deny' },
      'agena.monitor.start': { 'npm *': 'deny', 'npm test': 'allow' },
    })
    const row = panel.ruleRowsByKind.value.command.find(
      (item: { condition: string }) => item.condition === 'git status',
    )
    panel.selectRuleRow(row, true)
    expect(panel.selectedCommandRules.value).toEqual([
      { tool: 'agena.shell.exec', command: 'git status', mode: 'deny' },
    ])
    panel.setRuleRowMode(row, 'ask')
    expect(panel.config.value.tools.rules['agena.shell.exec']['git status']).toBe('ask')
    panel.removeRuleRow(row)
    expect(panel.config.value.tools.rules['agena.shell.exec']).toEqual({ 'git *': 'ask' })
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
    panel.renameCommandRule('agena.shell.exec', 'git status', { target: { value: 'git diff' } })
    expect(panel.config.value.tools.rules).toEqual({
      'agena.shell.exec': { 'git diff': 'allow', 'git push *': 'deny' },
    })
  })

  test('empty local read-only group displays its own Allow fallback and clear restores the parent', async () => {
    const { panel } = await createPermissionEditor(
      { tools: { rules: { '*': {} } } },
      { tools: { rules: { '*': { READ_ONLY: 'ask' } } } },
    )
    expect(panel.displayedReadOnlyMode.value).toBe('allow')
    expect(panel.readOnlyEditorMode.value).toBe('allow')
    panel.writeRule('*', 'read-only', '')
    expect(panel.displayedReadOnlyMode.value).toBe('ask')
    expect(panel.readOnlyEditorMode.value).toBe('')
  })

  test('read-only class aliases follow ASCII case and underscore normalization when edited or cleared', async () => {
    const { panel } = await createPermissionEditor({
      tools: { rules: { '*': { 'read-only': 'allow', READ_ONLY: 'deny', ReadOnly: 'ask' } } },
    })
    expect(panel.displayedReadOnlyMode.value).toBe('ask')
    panel.writeRule('*', 'read-only', 'auto')
    expect(panel.config.value.tools.rules['*']).toEqual({ 'read-only': 'auto' })
    panel.writeRule('*', 'read-only', '')
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
    expect(panel.toolInheritanceLabel()).toBe('Inherit · Ask')
    expect(panel.readOnlyInheritanceLabel()).toBe('Inherit · Allow')
    expect(panel.sectionSummary('filesystem')).toBe('Workspace: Read only · External: Deny')
    expect(panel.sectionSummary('network')).toBe('Outbound: Deny')
    expect(panel.sectionSummary('tools')).toBe('Default: Ask · Read only: Allow')
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
    expect(panel.readOnlyInheritanceLabel()).toBe('Inherit · Ask')
    expect(panel.readOnlyModeOptions.value.find((item: { value: string }) => item.value === 'auto').label).toBe(
      'Ask · Follow tool default',
    )
    panel.writeRule('*', 'read-only', 'allow')
    expect(panel.displayedReadOnlyMode.value).toBe('allow')
  })

  test('read-only aliases use ordered last-match semantics and write one canonical class', async () => {
    const { panel } = await createPermissionEditor({
      tools: { default: 'ask', rules: { '*': { 'read-only': 'allow', read_only: 'deny', readonly: 'auto' } } },
    })
    expect(panel.classMode('*', 'read-only')).toBe('auto')
    expect(panel.displayedReadOnlyMode.value).toBe('ask')
    panel.writeRule('*', 'read-only', 'deny')
    expect(panel.config.value.tools.rules['*']).toEqual({ 'read-only': 'deny' })
    expect(panel.displayedReadOnlyMode.value).toBe('deny')
  })

  test('an invalid scalar wildcard stays visible as raw configuration without inventing an effective mode', async () => {
    const { panel } = await createPermissionEditor({ tools: { rules: { '*': 'deny' } } })
    expect(panel.readOnlyPolicy(panel.config.value)).toBeUndefined()
    expect(panel.displayedReadOnlyMode.value).toBeUndefined()
    expect(panel.readOnlyInheritanceLabel()).toBe('—')
    expect(panel.rawJson.value).toContain('"*": "deny"')
    panel.writeRule('*', 'read-only', 'ask')
    expect(panel.config.value.tools.rules['*']).toEqual({ 'read-only': 'ask' })
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
    expect(panel.inheritedCommandRules.value).toEqual([
      { tool: 'agena.shell.spawn', command: 'dangerous', mode: 'deny' },
      { tool: 'agena.shell.spawn', command: 'npm test', mode: 'allow' },
    ])
    expect(panel.sectionRuleCount('filesystem')).toBe(2)
    expect(panel.sectionRuleCount('tools')).toBe(5)
    expect(panel.dirty.value).toBe(false)
  })

  test('first command override copies the entire inherited tool group and preserves rule order', async () => {
    const parentRules = { routine: 'auto', 'git *': 'ask', 'git status': 'allow', dangerous: 'deny' } as const
    const { panel, writes } = await createPermissionEditor(
      {},
      {
        tools: { rules: { 'agena.shell.exec': parentRules } },
      },
    )
    panel.setCommandMode('agena.shell.exec', 'git status', 'deny')
    expect(panel.config.value.tools.rules['agena.shell.exec']).toEqual({ ...parentRules, 'git status': 'deny' })
    expect(Object.keys(panel.config.value.tools.rules['agena.shell.exec'])).toEqual(Object.keys(parentRules))
    expect(panel.inheritedPermission.value.tools.rules['agena.shell.exec']).toEqual(parentRules)
    expect(panel.inheritedCommandRules.value).toHaveLength(0)
    expect(panel.visibleCommandRules.value).toHaveLength(4)
    await panel.save()
    expect(writes).toHaveLength(1)
    expect(writes[0]?.value).toEqual({
      tools: { rules: { 'agena.shell.exec': { ...parentRules, 'git status': 'deny' } } },
    })
  })

  test('a first category edit preserves inherited custom patterns and shell scalar defaults', async () => {
    const { panel } = await createPermissionEditor(
      {},
      {
        tools: { rules: { 'agena.shell.exec': { 'git push *': 'deny', routine: 'auto' }, 'agena.shell.spawn': 'ask' } },
      },
    )
    panel.writeRule('agena.shell.exec', 'routine', 'allow')
    panel.setCommandMode('agena.shell.spawn', 'npm test', 'allow')
    expect(panel.config.value.tools.rules).toEqual({
      'agena.shell.exec': { 'git push *': 'deny', routine: 'allow' },
      'agena.shell.spawn': { '*': 'ask', 'npm test': 'allow' },
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
    editor.panel.setToolDefault('ask')
    await editor.panel.save()
    expect(editor.writes[0]?.scope).toBe('workspace')
    await editor.switchScope('session')
    expect(editor.panel.configPath.value).toBe('')
  })
})
