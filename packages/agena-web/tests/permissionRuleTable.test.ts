import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { compileScript, parse } from '@vue/compiler-sfc'
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
const compiled = transformSync(compileScript(descriptor, { id: 'permission-rule-table-test' }).content, {
  loader: 'ts',
  format: 'cjs',
}).code
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
      expect(panel.hasCondition.value).toBe(kind === 'network' || kind === 'command')
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
      rule('local-git', 'command', { target: 'agena.shell.exec', condition: 'git status' }),
      rule('inherited-git', 'command', {
        target: 'agena.shell.exec',
        condition: 'git diff',
        source: 'inherited',
        sourceLabel: 'Inherited',
        selectable: false,
      }),
      rule('hidden-command', 'command', { target: 'agena.shell.exec', condition: 'ls' }),
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
})
