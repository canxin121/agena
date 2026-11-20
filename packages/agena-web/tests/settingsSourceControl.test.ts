import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { compileScript, parse } from '@vue/compiler-sfc'
import { transformSync } from 'esbuild'
import * as vue from 'vue'

const source = readFileSync(new URL('../src/components/settings/SettingsSourceControl.vue', import.meta.url), 'utf8')
const code = transformSync(
  compileScript(parse(source).descriptor, { id: 'setting-source-test', inlineTemplate: true }).content,
  {
    loader: 'ts',
    format: 'cjs',
  },
).code
function control(initial: Record<string, unknown> = {}) {
  const translate = (key: string) => key
  const events: unknown[][] = []
  const modules: Record<string, unknown> = {
    vue,
    '@remixicon/vue': { RiResetLeftLine: { name: 'RiResetLeftLine' } },
    '@/components/ui/IconButton.vue': { name: 'IconButton' },
    '@/components/ui/SegmentedControl.vue': { name: 'SegmentedControl' },
    '@/components/ui/SegmentedButton.vue': { name: 'SegmentedButton' },
    '@/i18n/settingsText': { settingsText: translate },
  }
  const module = { exports: {} as any }
  new Function('require', 'module', 'exports', code)((id: string) => modules[id], module, module.exports)
  const props = vue.reactive({
    modelValue: 'inherit',
    scope: 'workspace',
    label: 'Workspace files',
    inheritedValue: 'Read and write',
    inheritedSource: 'Global layer',
    showInheritedSummary: true,
    showInheritedValue: true,
    disabled: false,
    inheritDisabled: false,
    localDisabled: false,
    ...initial,
  })
  const render = module.exports.default.setup(props, { expose() {}, emit: (...event: unknown[]) => events.push(event) })
  function nodes() {
    const result: any[] = []
    const visit = (node: any) => {
      if (!node || typeof node !== 'object') return
      result.push(node)
      const children = Array.isArray(node.children) ? node.children : node.children?.default?.() || []
      for (const child of children) visit(child)
    }
    visit(render({ $st: translate }, []))
    return result
  }
  const choice = (value: string) => nodes().find((node) => node.props?.['data-setting-source-choice'] === value)
  const reset = () => nodes().find((node) => Object.hasOwn(node.props || {}, 'data-setting-reset'))
  const textContent = (node: any): string => {
    if (typeof node === 'string') return node
    if (node?.type === vue.Comment) return ''
    if (typeof node?.children === 'string') return node.children
    const children = Array.isArray(node?.children) ? node.children : node?.children?.default?.() || []
    return children.map(textContent).join('')
  }
  const choiceText = (value: string) => textContent(choice(value)).trim()
  return { props, events, choice, choiceText, reset, nodes }
}

describe('shared settings source control', () => {
  test('narrow controls shrink the inherited summary while retaining both actions and the complete source title', () => {
    const inheritedValue = 'A long inherited value that must remain available when its summary is truncated'
    const picker = control({ scope: 'session', inheritedSource: 'Parent configuration', inheritedValue })
    const group = picker.nodes().find((node) => node.props?.['data-setting-source'] === 'inherit')
    expect(group.props.class).toContain('min-w-0')
    expect(group.props.class).toContain('max-w-full')
    expect(group.props.class).not.toContain('shrink-0')
    expect(picker.choice('inherit').props.class).toContain('min-w-0')
    expect(picker.choice('local').props.class).toContain('shrink-0')
    expect(picker.choiceText('inherit')).toStartWith('Inherit')
    expect(picker.choiceText('local')).toBe('This scope')
    expect(picker.choice('inherit').props.title).toBe(`Use inherited value · Parent configuration · ${inheritedValue}`)
    const summary = picker.nodes().find((node) => Object.hasOwn(node.props || {}, 'data-setting-inherited'))
    expect(summary.props.class).toContain('truncate')
    expect(summary.props.title).toBe(`Parent configuration · ${inheritedValue}`)
    picker.choice('local').props.onClick()
    picker.props.modelValue = 'local'
    picker.choice('inherit').props.onClick()
    expect(picker.events).toEqual([
      ['update:modelValue', 'local'],
      ['update:modelValue', 'inherit'],
    ])
  })

  test('the inherited summary can show the parent alone when the adjacent editor shows the value', () => {
    const picker = control({ showInheritedValue: false })
    expect(
      picker.nodes().find((node) => Object.hasOwn(node.props || {}, 'data-setting-inherited-source'))?.children,
    ).toBe('Global layer')
    expect(picker.nodes().some((node) => Object.hasOwn(node.props || {}, 'data-setting-inherited-value'))).toBe(false)
    picker.props.modelValue = 'local'
    expect(picker.nodes().some((node) => Object.hasOwn(node.props || {}, 'data-setting-inherited'))).toBe(false)
  })

  test('permissions can show only the source choices while keeping the full inheritance title', () => {
    const picker = control({ showInheritedSummary: false })
    expect(picker.choiceText('inherit')).toBe('Inherit')
    expect(picker.choiceText('local')).toBe('This scope')
    expect(picker.nodes().some((node) => Object.hasOwn(node.props || {}, 'data-setting-inherited'))).toBe(false)
    expect(picker.choice('inherit').props.title).toBe('Use inherited value · Global layer · Read and write')
    expect(picker.choice('inherit').props['aria-pressed']).toBe(true)
    picker.choice('local').props.onClick()
    expect(picker.events).toEqual([['update:modelValue', 'local']])
  })

  test('unknown inherited values show their parent context without fabricating a value', () => {
    const picker = control({ scope: 'session', inheritedSource: '', inheritedValue: 'Unavailable' })
    expect(
      picker.nodes().find((node) => Object.hasOwn(node.props || {}, 'data-setting-inherited-source'))?.children,
    ).toBe('Parent configuration')
    expect(
      picker.nodes().find((node) => Object.hasOwn(node.props || {}, 'data-setting-inherited-value'))?.children,
    ).toBe('Unavailable')
    picker.choice('local').props.onClick()
    expect(picker.events).toEqual([['update:modelValue', 'local']])
  })

  test('global settings expose a trailing reset action without selectable source states', () => {
    const picker = control({ scope: 'global', modelValue: 'local' })
    expect(picker.choice('inherit')).toBeUndefined()
    expect(picker.choice('local')).toBeUndefined()
    expect(picker.nodes().some((node) => Object.hasOwn(node.props || {}, 'data-setting-source'))).toBe(false)
    expect(picker.reset().props.tooltip).toBe('Reset to defaults · Read and write')
    expect(picker.reset().props['aria-label']).toBe('Workspace files: Reset to defaults')
    expect(picker.reset().props['aria-pressed']).toBeUndefined()
    expect(picker.reset().props.class).toContain('order-last')
    picker.reset().props.onClick()
    expect(picker.events).toEqual([['update:modelValue', 'inherit']])
    picker.props.scope = 'workspace'
    expect(picker.reset()).toBeUndefined()
    expect(picker.choiceText('inherit')).toStartWith('Inherit')
    expect(picker.choice('inherit').props['aria-label']).toBe('Workspace files: Use inherited value')
    expect(picker.events).toHaveLength(1)
  })

  test.each([
    { modelValue: 'inherit' },
    { modelValue: 'local', disabled: true },
    { modelValue: 'local', inheritDisabled: true },
  ])('global reset is unavailable when no reset can be applied: %o', (state) => {
    const picker = control({ scope: 'global', ...state })
    expect(picker.reset().props.disabled).toBe(true)
    picker.reset().props.onClick()
    expect(picker.events).toEqual([])
    expect(picker.choice('local')).toBeUndefined()
  })

  test.each(['workspace', 'session'])('%s source controls retain inheritance terminology', (scope) => {
    const picker = control({ scope })
    expect(picker.choiceText('inherit')).toStartWith('Inherit')
    expect(
      picker.nodes().find((node) => Object.hasOwn(node.props || {}, 'data-setting-inherited-source'))?.children,
    ).toBe('Global layer')
    expect(
      picker.nodes().find((node) => Object.hasOwn(node.props || {}, 'data-setting-inherited-value'))?.children,
    ).toBe('Read and write')
    expect(picker.choice('inherit').props.title).toStartWith('Use inherited value')
  })

  test('keeps source separate from value and toggles in one deliberate action', () => {
    const picker = control()
    expect(picker.choice('inherit').props['aria-pressed']).toBe(true)
    expect(picker.choice('local').props['aria-pressed']).toBe(false)
    expect(picker.choice('inherit').props.title).toBe('Use inherited value · Global layer · Read and write')
    expect(picker.choice('local').props['aria-label']).toBe('Workspace files: Set in this layer')
    picker.choice('inherit').props.onClick()
    expect(picker.events).toEqual([])
    picker.choice('local').props.onClick()
    expect(picker.events).toEqual([['update:modelValue', 'local']])
    picker.props.modelValue = 'local'
    expect(picker.choice('local').props['aria-pressed']).toBe(true)
    picker.choice('inherit').props.onClick()
    expect(picker.events.at(-1)).toEqual(['update:modelValue', 'inherit'])
  })

  test('unavailable values and guarded resets cannot emit source changes', () => {
    const picker = control({ localDisabled: true })
    picker.choice('local').props.onClick()
    expect(picker.events).toEqual([])
    picker.props.modelValue = 'local'
    picker.props.inheritDisabled = true
    picker.choice('inherit').props.onClick()
    expect(picker.events).toEqual([])
    picker.props.inheritDisabled = false
    picker.props.disabled = true
    picker.choice('inherit').props.onClick()
    expect(picker.events).toEqual([])
  })

  test('effective policy is labelled without offering writable layer choices', () => {
    const picker = control({ modelValue: 'effective' })
    expect(picker.nodes().some((node) => node.props?.['data-setting-source'] === 'effective')).toBe(true)
    expect(picker.choice('inherit')).toBeUndefined()
    expect(picker.choice('local')).toBeUndefined()
  })
})
