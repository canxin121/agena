import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { compileScript, compileTemplate, parse } from '@vue/compiler-sfc'
import { transformSync } from 'esbuild'
import * as vue from 'vue'

import {
  PERMISSION_RULE_KINDS,
  type PermissionRuleRow,
} from '../src/components/settings/permissions/permissionRuleTable'

const source = readFileSync(
  new URL('../src/components/settings/permissions/PermissionRuleTable.vue', import.meta.url),
  'utf8',
)
const { descriptor } = parse(source)
const script = compileScript(descriptor, { id: 'permission-rule-table-test' })
const compiled = transformSync(script.content, {
  loader: 'ts',
  format: 'cjs',
}).code
const template = compileTemplate({
  source: descriptor.template!.content,
  filename: 'PermissionRuleTable.vue',
  id: 'permission-rule-table-test',
  compilerOptions: { bindingMetadata: script.bindings },
})
if (template.errors.length) throw new Error(template.errors.map(String).join('\n'))
const templateModule = { exports: {} as { render?: (...args: any[]) => any } }
new Function('require', 'module', 'exports', transformSync(template.code, { format: 'cjs' }).code)(
  () => ({ ...vue, renderSlot: (slots: Record<string, () => unknown>, name: string) => slots[name]?.() || [] }),
  templateModule,
  templateModule.exports,
)
function nodes(root: any): any[] {
  if (Array.isArray(root)) return root.flatMap(nodes)
  if (!root || typeof root !== 'object') return []
  return [root, ...nodes(Array.isArray(root.children) ? root.children : [])]
}
function renderTable(editor: ReturnType<typeof table>) {
  const setup = { ...vue.proxyRefs(editor.panel) } as Record<string, any>
  for (const key of Object.keys(setup)) if (key.startsWith('Ri') && setup[key] === undefined) setup[key] = { name: key }
  setup.kindIcons = Object.fromEntries(PERMISSION_RULE_KINDS.map((kind) => [kind, { name: kind + 'Icon' }]))
  return templateModule.exports.render!({ $st: (key: string) => key, $slots: {} }, [], editor.props, setup, {}, {})
}
const scopes: vue.EffectScope[] = []
afterEach(() => {
  for (const scope of scopes.splice(0)) scope.stop()
})

function table(kind: PermissionRuleRow['kind'], rows: PermissionRuleRow[], disabled = false) {
  const events: Array<{ name: string; args: unknown[] }> = []
  const modules: Record<string, unknown> = {
    vue,
    '@remixicon/vue': {},
    '@/i18n/settingsText': { settingsText: (key: string) => key },
  }
  const module = { exports: {} as { default?: { setup: (props: unknown, context: unknown) => any } } }
  const require = (id: string) => {
    if (id.endsWith('.vue')) return {}
    if (!(id in modules)) throw new Error('Unexpected component dependency: ' + id)
    return modules[id]
  }
  new Function('require', 'module', 'exports', compiled)(require, module, module.exports)
  const scope = vue.effectScope()
  scopes.push(scope)
  const props = vue.reactive({ kind, rows, selectedKeys: [] as string[], disabled })
  const panel = scope.run(() =>
    module.exports.default!.setup(props, {
      expose() {},
      emit(name: string, ...args: unknown[]) {
        events.push({ name, args })
      },
    }),
  )!
  return { panel, props, events }
}

function rule(
  key: string,
  kind: PermissionRuleRow['kind'],
  overrides: Partial<PermissionRuleRow> = {},
): PermissionRuleRow {
  return {
    key,
    kind,
    target: key,
    condition: '',
    mode: 'ask',
    modeLabel: 'Ask',
    source: 'local',
    sourceLabel: 'Workspace',
    selectable: true,
    ...overrides,
  }
}

describe('separate permission rule tables', () => {
  for (const kind of PERMISSION_RULE_KINDS) {
    test(`${kind} rejects other rule kinds in display, counts, actions and selection`, () => {
      const ownRules = [
        rule('local-' + kind, kind),
        rule('parent-' + kind, kind, { source: 'inherited', sourceLabel: 'Inherited', selectable: false }),
      ]
      const otherRules = PERMISSION_RULE_KINDS.filter((value) => value !== kind).map((value) =>
        rule('other-' + value, value, { removable: true, editAction: true }),
      )
      const rows = [...ownRules, ...otherRules]
      const { panel, props, events } = table(kind, rows)

      props.selectedKeys = rows.map((row) => row.key)
      expect(panel.query.value).toBe('')
      expect(panel.kindRows.value).toEqual(ownRules)
      expect(panel.visibleRows.value).toEqual(ownRules)
      expect(panel.visibleSelectable.value).toEqual([ownRules[0]])
      expect(panel.selectedCount.value).toBe(1)
      expect(panel.hasCondition.value).toBe(kind === 'network')
      expect(panel.hasActions.value).toBe(false)
      panel.selectVisible()
      expect(events).toEqual([{ name: 'select-visible', args: [[ownRules[0]], false] }])

      props.rows[0]!.removable = true
      expect(panel.hasActions.value).toBe(true)
      props.rows.splice(0, ownRules.length)
      expect(panel.kindRows.value).toEqual([])
      expect(panel.hasSelectable.value).toBe(false)
      expect(panel.hasActions.value).toBe(false)
      expect(panel.selectedCount.value).toBe(0)
    })
  }

  test('keeps searches and select-all confined to each independent table', () => {
    const rows = [
      rule('local-git', 'command', { target: 'git status' }),
      rule('inherited-git', 'command', {
        target: 'agena.shell.exec',
        condition: 'git diff',
        source: 'inherited',
        sourceLabel: 'Inherited',
        selectable: false,
      }),
      rule('hidden-command', 'command', { target: 'ls' }),
      rule('hidden-network', 'network', { target: 'git.example.test' }),
    ]
    const { panel, props, events } = table('command', rows)
    const network = table('network', rows)
    panel.query.value = ' GIT '

    expect(panel.visibleRows.value.map((row: PermissionRuleRow) => row.key)).toEqual(['local-git', 'inherited-git'])
    expect(network.panel.query.value).toBe('')
    expect(network.panel.visibleRows.value).toEqual([rows[3]])
    panel.selectVisible()
    expect(events).toEqual([{ name: 'select-visible', args: [[rows[0]], true] }])

    props.selectedKeys = ['local-git', 'hidden-network']
    expect(panel.allVisibleSelected.value).toBe(true)
    expect(panel.selectedCount.value).toBe(1)
    expect(network.panel.selectedCount.value).toBe(0)
    panel.selectVisible()
    expect(events.at(-1)).toEqual({ name: 'select-visible', args: [[rows[0]], false] })

    panel.query.value = 'nothing matches'
    panel.selectVisible()
    expect(events).toHaveLength(2)
    panel.clearSearch()
    expect(panel.visibleRows.value).toEqual(rows.slice(0, 3))
    network.panel.query.value = 'not found'
    expect(network.panel.visibleRows.value).toEqual([])
    expect(panel.visibleRows.value).toEqual(rows.slice(0, 3))
  })

  test('blocks individual and visible selection while disabled and never selects a read-only row', () => {
    const rows = [
      rule('local', 'filesystem'),
      rule('parent', 'filesystem', { source: 'inherited', sourceLabel: 'Inherited', selectable: false }),
      rule('other-kind', 'tool-name'),
    ]
    const { panel, props, events } = table('filesystem', rows, true)
    const checked = { target: { checked: true } } as unknown as Event

    panel.selectVisible()
    panel.selectRow(rows[0], checked)
    expect(events).toEqual([])

    props.disabled = false
    panel.selectRow(rows[1], checked)
    panel.selectRow(rows[2], checked)
    expect(events).toEqual([])
    panel.selectRow(rows[0], checked)
    expect(events).toEqual([{ name: 'select', args: [rows[0], true] }])
  })

  test('keeps select-all in the always-visible toolbar with filtered, mixed and disabled states', () => {
    const rows = [
      rule('first', 'filesystem'),
      rule('second', 'filesystem'),
      rule('parent', 'filesystem', { source: 'inherited', sourceLabel: 'Global layer', selectable: false }),
    ]
    const editor = table('filesystem', rows)
    function selectAll() {
      const rendered = nodes(renderTable(editor))
      const toolbar = rendered.find((node) => node.type === 'header')
      const tableHead = rendered.find((node) => node.type === 'thead')
      const controls = nodes(toolbar).filter((node) => node.props?.['aria-label'] === 'Select all permission rules')
      expect(controls).toHaveLength(1)
      expect(nodes(tableHead).some((node) => node.type === 'input')).toBe(false)
      return controls[0]
    }
    let control = selectAll()
    expect(control.props.disabled).toBe(false)
    expect(control.props.checked).toBe(false)
    expect(control.props.indeterminate).toBe(false)
    control.props.onChange()
    expect(editor.events.at(-1)).toEqual({ name: 'select-visible', args: [rows.slice(0, 2), true] })

    editor.props.selectedKeys = ['first']
    control = selectAll()
    expect(control.props.checked).toBe(false)
    expect(control.props.indeterminate).toBe(true)
    editor.panel.query.value = 'first'
    control = selectAll()
    expect(control.props.checked).toBe(true)
    expect(control.props.indeterminate).toBe(false)
    control.props.onChange()
    expect(editor.events.at(-1)).toEqual({ name: 'select-visible', args: [[rows[0]], false] })

    editor.panel.query.value = 'parent'
    control = selectAll()
    expect(control.props.disabled).toBe(true)
    control.props.onChange()
    expect(editor.events).toHaveLength(2)
    editor.panel.clearSearch()
    editor.props.disabled = true
    control = selectAll()
    expect(control.props.disabled).toBe(true)
    control.props.onChange()
    expect(editor.events).toHaveLength(2)
  })

  test('searches a flat command table and displays only its pattern column', () => {
    const rows = ['no-op', 'routine', 'dangerous'].map((target) => rule(target, 'command', { target }))
    const editor = table('command', rows)
    const headers = nodes(renderTable(editor)).filter((node) => node.type === 'th')
    expect(headers.map((node) => node.children)).toContain('Command pattern')
    expect(headers.map((node) => node.children)).not.toContain('Tool name')
    expect(editor.panel.visibleRows.value).toEqual(rows)
    editor.panel.query.value = 'dangerous'
    expect(editor.panel.visibleRows.value).toEqual([rows[2]])
    const renderedRows = nodes(renderTable(editor)).filter((node) => node.props?.['data-permission-rule'] === 'command')
    expect(renderedRows.map((node) => node.props['data-rule-target'])).toEqual(['dangerous'])
  })

  test('edits command patterns and keeps inherited values read-only', () => {
    const rows = [
      rule('local', 'command', { target: 'git status', targetEditable: true }),
      rule('inherited', 'command', {
        target: 'dangerous',
        source: 'inherited',
        targetEditable: true,
        readOnly: true,
        selectable: false,
        modeOptions: [{ value: 'deny', label: 'Deny' }],
      }),
    ]
    const editor = table('command', rows)
    const rendered = nodes(renderTable(editor)).filter((node) => node.props?.['data-permission-rule'] === 'command')
    const target = (index: number) =>
      nodes(rendered[index]).find((node) => node.props?.['data-permission-row-action'] === 'rename-target')
    const input = target(0)
    expect(input.props.value).toBe('git status')
    expect(input.props.disabled).toBeFalsy()
    const event = { target: { value: 'git diff' } } as unknown as Event
    input.props.onChange(event)
    expect(editor.events.at(-1)).toEqual({ name: 'rename-target', args: [rows[0], event] })
    expect(target(1).props.disabled).toBe(true)
    const mode = nodes(rendered[1]).find((node) => node.props?.options)
    expect(mode.props.disabled).toBe(true)
  })
})
