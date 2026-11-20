import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { compileScript, compileTemplate, parse } from '@vue/compiler-sfc'
import { transformSync } from 'esbuild'
import * as vue from 'vue'
import { createPermissionEditor, disposePermissionEditors } from './helpers/permissionStudioHarness'

afterEach(disposePermissionEditors)
const parent = {
  path: { workspace: 'readwrite', external: 'readonly', rules: { '/shared': 'readonly' } },
  network: {
    version: 2,
    default: 'allow',
    inbound: 'deny',
    hostLoopback: 'deny',
    allow: [{ to: '192.0.2.10' }],
    deny: [{ to: '198.51.100.10' }],
  },
  tools: {
    default: 'ask',
    names: { 'agena.fs.read': 'allow' },
    rules: { '*': { 'read-only': 'deny' }, 'shell.*': { routine: 'ask', dangerous: 'deny' } },
  },
} as const

function template(file: string) {
  const { descriptor } = parse(readFileSync(new URL(file, import.meta.url), 'utf8'))
  const script = compileScript(descriptor, { id: file })
  const result = compileTemplate({
    source: descriptor.template!.content,
    filename: file,
    id: file,
    compilerOptions: { bindingMetadata: script.bindings },
  })
  if (result.errors.length) throw new Error(String(result.errors))
  const compiled = transformSync(result.code, { loader: 'ts', format: 'cjs' }).code
  const module = { exports: {} as { render?: (...args: any[]) => any } }
  new Function('require', 'module', 'exports', compiled)(
    () => ({ ...vue, renderSlot: (slots: any, name: string) => slots[name]?.() || [] }),
    module,
    module.exports,
  )
  return module.exports.render!
}
const panelTemplate = template('../src/components/settings/PermissionStudioPanel.vue')
const rowTemplate = template('../src/components/settings/permissions/PermissionPolicyRow.vue')
function nodes(root: any, expandSlots = true): any[] {
  if (Array.isArray(root)) return root.flatMap((node) => nodes(node, expandSlots))
  if (!root || typeof root !== 'object') return []
  const child = root.children
  return [
    root,
    ...nodes(Array.isArray(child) ? child : [], expandSlots),
    ...(expandSlots && child && typeof child === 'object' && !Array.isArray(child)
      ? Object.entries(child)
          .filter(([key, value]) => key !== '_' && typeof value === 'function')
          .flatMap(([, value]) => nodes((value as (props: object) => unknown)({}), expandSlots))
      : []),
  ]
}
function renderPanel(editor: Awaited<ReturnType<typeof createPermissionEditor>>) {
  const setup = { ...vue.proxyRefs(editor.panel) } as Record<string, any>
  for (const key of Object.keys(setup)) if (key.startsWith('Ri') && setup[key] === undefined) setup[key] = { name: key }
  return panelTemplate({ $st: (key: string) => key }, [], editor.props, setup, {}, {})
}

describe('inherited permissions are visible and read-only', () => {
  for (const scope of ['workspace', 'session'] as const) {
    test(`${scope} shows the parent values and rejects all inherited default mutations`, async () => {
      const editor = await createPermissionEditor({}, parent, { scope, sessionId: '42' })
      const { panel, writes } = editor
      expect(panel.inheritedSourceLabel.value).toBe(scope === 'workspace' ? 'Global layer' : 'Parent configuration')
      expect(panel.displayedPathMode('workspace')).toBe('readwrite')
      expect(panel.displayedToolDefault.value).toBe('ask')
      expect(panel.readOnlyValueMode.value).toBe('deny')
      expect(panel.inheritedCommandRules.value).toContainEqual(['routine', 'ask'])
      expect(panel.sourceValueDisabled(panel.pathSource('workspace'))).toBe(true)
      panel.setPathDefault('workspace', 'deny')
      panel.setToolDefault('deny')
      panel.setReadOnlyMode('allow')
      panel.setCommandMode('dangerous', 'allow')
      for (const field of ['default', 'inbound', 'hostLoopback']) panel.setNetworkMode(field, 'deny')
      await panel.save()
      expect(panel.config.value).toEqual({})
      expect(writes).toHaveLength(0)
      panel.setPathSource('workspace', 'local')
      panel.setPathDefault('workspace', 'deny')
      panel.setToolDefaultSource('local')
      panel.setToolDefault('deny')
      panel.setReadOnlySource('local')
      panel.setReadOnlyMode('allow')
      panel.copyCommandRules()
      panel.setCommandMode('routine', 'allow')
      await panel.save()
      expect(writes.at(-1)?.value).toMatchObject({
        path: { workspace: 'deny' },
        tools: {
          default: 'deny',
          rules: { '*': { 'read-only': 'allow' }, 'shell.*': { routine: 'allow', dangerous: 'deny' } },
        },
      })
      expect(panel.sourceValueDisabled(panel.pathSource('external'))).toBe(true)
    })

    test(`${scope} renders real disabled value controls while source controls stay actionable`, async () => {
      const editor = await createPermissionEditor({}, parent, { scope, sessionId: '42' })
      const tree = nodes(renderPanel(editor))
      const defaults = tree.filter(
        (node) => node.type === editor.panel.PermissionPolicyRow && node.props?.['data-permission-default'],
      )
      expect(defaults).toHaveLength(7)
      for (const row of defaults) {
        expect(row.props.source).toBe('inherit')
        expect(row.props['inherited-source']).toBe(scope === 'workspace' ? 'Global layer' : 'Parent configuration')
        expect(row.props.disabled).toBeFalsy()
        const picker = nodes(row.children.default()).find((node) => node.type === editor.panel.OptionPicker)
        expect(picker?.props.disabled).toBe(true)
      }
      const commandTable = tree.find(
        (node) => node.type === editor.panel.PermissionRuleTable && node.props?.kind === 'command',
      )
      expect(commandTable.props.rows.map((row: any) => row.target)).toEqual(['routine', 'dangerous'])
      expect(commandTable.props.rows.every((row: any) => row.readOnly)).toBe(true)
      const source = nodes(commandTable.children.actions()).find(
        (node) => node.type === editor.panel.SettingsSourceControl,
      )
      expect(source.props['model-value']).toBe('inherit')
      expect(source.props.disabled).toBe(false)
      const raw = tree.find((node) => node.type === 'textarea' && node.props?.['aria-label'] === 'Raw PermissionConfig')
      expect(raw.props.readonly).toBe(true)
      expect(JSON.parse(raw.props.value)).toMatchObject(parent)
    })
  }

  test('policy rows gray only the inherited editor, preserving the source choice', () => {
    const sourceControl = { name: 'SourceControl' }
    const props = {
      label: 'Workspace',
      value: 'Read and write',
      inheritedValue: 'Read and write',
      inheritedSource: 'Global layer',
      editable: true,
      source: 'inherit',
      scope: 'workspace',
      disabled: false,
    }
    const tree = nodes(
      rowTemplate({ $slots: { default: () => [] } }, [], props, { SettingsSourceControl: sourceControl }, {}, {}),
    )
    const field = tree.find((node) => node.type === 'fieldset')
    expect(field.props.disabled).toBe(true)
    expect(field.props.class).toContain('is-inherited')
    const source = tree.find((node) => node.type === sourceControl)
    expect(source.props.disabled).toBe(false)
    expect(source.props['show-inherited-summary']).toBe(false)
    const global = nodes(
      rowTemplate(
        { $slots: { default: () => [] } },
        [],
        { ...props, scope: 'global' },
        { SettingsSourceControl: sourceControl },
        {},
        {},
      ),
    )
    expect(global.find((node) => node.type === 'fieldset').props.disabled).toBeFalsy()
  })

  test('inherited rules cannot be changed through direct handlers, add collisions, or list editors', async () => {
    const { panel, writes } = await createPermissionEditor({}, parent, { scope: 'workspace' })
    for (const row of Object.values(panel.ruleRowsByKind.value).flat() as any[]) {
      expect(row.readOnly).toBe(true)
      expect(row.selectable).toBeFalsy()
      panel.setRuleRowMode(row, 'deny')
      panel.removeRuleRow(row)
      panel.editRuleRow(row)
    }
    panel.newPath.value = '/shared'
    panel.addPathRule()
    panel.newToolName.value = 'agena.fs.read'
    panel.addToolName()
    panel.newCommandPattern.value = 'npm test'
    panel.addCommandRule()
    panel.beginRule('network')
    expect(panel.networkRuleDraft.value).toBeNull()
    expect(panel.config.value).toEqual({})
    expect(writes).toHaveLength(0)
    await panel.changeRuleSource(panel.ruleRowsByKind.value.filesystem[0])
    panel.setPathRuleMode('/shared', 'deny')
    expect(panel.config.value.path.rules['/shared']).toBe('deny')
    panel.setNetworkSource('allow', 'local')
    panel.editNetworkRule('allow', 0)
    panel.setNetworkRuleAction('deny')
    expect(panel.networkRuleDraft.value.action).toBe('allow')
    panel.networkRuleDraft.value.action = 'deny'
    expect(panel.applyNetworkRule()).toBe(false)
    expect(panel.networkSource('deny')).toBe('inherit')
  })

  test('JSON cannot materialize inherited values without an explicit source choice', async () => {
    const { panel } = await createPermissionEditor({ tools: { default: 'ask' } }, parent, { scope: 'workspace' })
    expect(panel.networkJsonReadOnly.value).toBe(true)
    expect(JSON.parse(panel.networkJsonDisplay.value)).toEqual(parent.network)
    panel.networkJson.value = JSON.stringify({ version: 2, default: 'deny' })
    expect(panel.applyNetworkJson()).toBe(false)
    panel.syncRaw()
    panel.rawJson.value = JSON.stringify({ path: { workspace: 'deny' }, tools: { default: 'deny' } })
    expect(panel.applyRawJson()).toBe(false)
    expect(panel.rawJsonError.value).toContain('Switch inherited settings')
    expect(panel.config.value).toEqual({ tools: { default: 'ask' } })
    panel.syncRaw()
    panel.setNetworkSource('allow', 'local')
    panel.networkJson.value = JSON.stringify({ version: 2, allow: [], deny: [] })
    expect(panel.applyNetworkJson()).toBe(false)
    panel.syncRaw()
    panel.networkJson.value = JSON.stringify({ version: 2, allow: [] })
    expect(panel.applyNetworkJson()).toBe(true)
    expect(panel.networkSource('deny')).toBe('inherit')
    const offline = { version: 2, default: 'deny', inbound: 'deny', hostLoopback: 'deny', allow: [], deny: [] }
    panel.networkJson.value = JSON.stringify(offline)
    expect(panel.applyNetworkJson()).toBe(false)
    expect(panel.config.value.network).toEqual({ version: 2, allow: [] })
    panel.syncRaw()
    for (const field of ['default', 'inbound', 'hostLoopback', 'deny']) panel.setNetworkSource(field, 'local')
    panel.networkJson.value = JSON.stringify(offline)
    expect(panel.applyNetworkJson()).toBe(true)
    expect(panel.config.value.network).toEqual(offline)
  })

  test('a complete JSON policy consumes explicit local intents so omitted fields can inherit again', async () => {
    const { panel } = await createPermissionEditor({}, null, { scope: 'workspace' })
    for (const field of ['default', 'inbound', 'hostLoopback', 'allow', 'deny']) panel.setNetworkSource(field, 'local')
    expect(panel.config.value).toEqual({})
    expect(panel.localSourceIntents.value.size).toBe(5)
    panel.networkJson.value = JSON.stringify({
      version: 2,
      default: 'deny',
      inbound: 'deny',
      hostLoopback: 'deny',
      allow: [],
      deny: [],
    })
    expect(panel.applyNetworkJson()).toBe(true)
    expect(panel.localSourceIntents.value.size).toBe(0)
    panel.networkJson.value = JSON.stringify({ version: 2, default: 'deny' })
    expect(panel.applyNetworkJson()).toBe(true)
    expect(panel.networkSource('default')).toBe('local')
    for (const field of ['inbound', 'hostLoopback', 'allow', 'deny']) expect(panel.networkSource(field)).toBe('inherit')
  })

  test('unknown parent values unlock only after explicit local choice without writing guessed values', async () => {
    const editor = await createPermissionEditor({}, null, { scope: 'workspace' })
    const { panel, writes } = editor
    panel.setPathDefault('workspace', 'deny')
    expect(panel.config.value).toEqual({})
    panel.setPathSource('workspace', 'local')
    panel.setToolDefaultSource('local')
    panel.setReadOnlySource('local')
    panel.setNetworkSource('default', 'local')
    panel.copyCommandRules()
    expect(panel.sourceValueDisabled(panel.pathSource('workspace'))).toBe(false)
    expect(panel.networkSource('default')).toBe('local')
    expect(panel.commandSource.value).toBe('local')
    expect(panel.config.value).toEqual({})
    await panel.save()
    expect(writes).toHaveLength(0)
    panel.setPathDefault('workspace', 'deny')
    panel.setToolDefault('deny')
    panel.setReadOnlyMode('ask')
    panel.setNetworkMode('default', 'deny')
    panel.setCommandMode('dangerous', 'deny')
    await panel.save()
    expect(writes).toHaveLength(1)
    panel.discard()
    panel.setPathSource('workspace', 'inherit')
    expect(panel.sourceValueDisabled(panel.pathSource('workspace'))).toBe(true)
  })
})
