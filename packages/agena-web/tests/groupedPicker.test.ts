import { afterEach, describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { compileScript, parse } from '@vue/compiler-sfc'
import { transformSync } from 'esbuild'
import * as vue from 'vue'

const source = readFileSync(new URL('../src/components/ui/GroupedPicker.vue', import.meta.url), 'utf8')
const descriptor = parse(source).descriptor
const componentCode = transformSync(
  compileScript(descriptor, { id: 'grouped-picker-test', inlineTemplate: true }).content,
  { loader: 'ts', format: 'cjs' },
).code
const scopes: vue.EffectScope[] = []
afterEach(() => {
  for (const scope of scopes.splice(0)) scope.stop()
})

function picker(overrides: Record<string, unknown> = {}) {
  const events: Array<{ name: string; args: unknown[] }> = []
  const menu = { name: 'OptionMenu' }
  const modules: Record<string, unknown> = {
    vue: { ...vue, renderSlot: () => vue.createCommentVNode('empty trailing slot') },
    '@/components/ui/OptionMenu.vue': menu,
    '@/components/ui/PickerTrigger.vue': { name: 'PickerTrigger' },
    '@/lib/utils': { cn: (...values: unknown[]) => values.filter(Boolean).join(' ') },
    '@/stores/ui': { useUiStore: () => ({ isCompactTouch: true }) },
  }
  const module = {
    exports: {} as { default: { setup: (props: unknown, context: unknown) => (...args: unknown[]) => any } },
  }
  new Function('require', 'module', 'exports', componentCode)(
    (id: string) => {
      if (!(id in modules)) throw new Error('Unexpected dependency ' + id)
      return modules[id]
    },
    module,
    module.exports,
  )
  const props = vue.reactive({
    label: 'Project A',
    groups: [
      {
        id: 'projects',
        title: 'Projects',
        items: [
          { id: '1', label: 'Alpha', description: '/a' },
          { id: '2', label: 'Beta', description: '/b' },
        ],
      },
    ],
    query: undefined,
    filterMode: 'internal',
    searchable: true,
    selectedId: '',
    page: 2,
    pageCount: 4,
    disabled: false,
    triggerClass: '',
    menuClass: '',
    collapsibleGroups: true,
    size: 'sm',
    ...overrides,
  })
  const scope = vue.effectScope()
  scopes.push(scope)
  const render = scope.run(() =>
    module.exports.default.setup(props, {
      expose() {},
      emit: (name: string, ...args: unknown[]) => events.push({ name, args }),
    }),
  )!
  function tree() {
    return render({ $slots: {} }, [])
  }
  function find(node: any, predicate: (node: any) => boolean): any {
    if (!node || typeof node !== 'object') return null
    if (predicate(node)) return node
    for (const child of Array.isArray(node.children) ? node.children : []) {
      const found = find(child, predicate)
      if (found) return found
    }
    return null
  }
  return {
    props,
    events,
    menu: () => {
      const node = find(tree(), (node) => node.type === menu)
      // Vue normalizes these attribute names when mounting the child component.
      return {
        ...node,
        props: Object.fromEntries(
          Object.entries(node.props).map(([key, value]) => [
            key.replace(/-([a-z])/g, (_, letter) => letter.toUpperCase()),
            value,
          ]),
        ),
      }
    },
  }
}

describe('shared grouped picker', () => {
  test('legacy project consumers delegate local search and retain external pagination', () => {
    const wrapper = picker()
    let menu = wrapper.menu()
    expect(menu.props.filterMode).toBe('internal')
    expect(menu.props.externalPage).toBe(2)
    expect(menu.props.externalPageCount).toBe(4)
    menu.props['onUpdate:query']('beta')
    menu = wrapper.menu()
    expect(menu.props.query).toBe('beta')
    expect(menu.props.groups[0].items).toHaveLength(2)
    expect(menu.props.externalPageCount).toBe(4)
    expect(menu.props.isCompactTouch).toBe(true)
  })

  test('controlled external searches and page requests reach the catalog without filtering the current page locally', () => {
    const wrapper = picker({ query: '', filterMode: 'external' })
    let menu = wrapper.menu()
    menu.props['onUpdate:query']('older')
    expect(wrapper.events).toContainEqual({ name: 'update:query', args: ['older'] })
    wrapper.props.query = 'older'
    menu = wrapper.menu()
    expect(menu.props.query).toBe('older')
    expect(menu.props.groups[0].items).toHaveLength(2)
    menu.props.onRequestPage(3)
    expect(wrapper.events).toContainEqual({ name: 'request-page', args: [3] })
  })

  test('open and refresh events are forwarded and disabled choices never emit selections', () => {
    const wrapper = picker()
    const menu = wrapper.menu()
    menu.props['onUpdate:open'](true)
    menu.props.onRefresh()
    expect(wrapper.events).toContainEqual({ name: 'update:open', args: [true] })
    expect(wrapper.events).toContainEqual({ name: 'refresh', args: [] })
    wrapper.props.disabled = true
    menu.props.onSelect({ id: '1', label: 'Alpha' })
    expect(wrapper.events.some((event) => event.name === 'select')).toBe(false)
  })
})
