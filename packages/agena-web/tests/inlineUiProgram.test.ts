import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { parseInlineDocument } from '../src/features/inlineUi/model'
import {
  activateUi,
  inputUi,
  renderUi,
  validateUiProgram,
  uiMarkdown,
  reconcileUiState,
  type UiProgram,
  type UiState,
} from '../src/features/inlineUi/program'
const fixture = JSON.parse(
  readFileSync(new URL('../../../examples/inline-ui/live-dashboard.json', import.meta.url), 'utf8'),
)
const oracle = JSON.parse(
  readFileSync(new URL('../../../examples/inline-ui/program-cases.json', import.meta.url), 'utf8'),
)
const program = (): UiProgram => structuredClone(fixture.blocks[0].program)

describe('stateful inline UI shared Rust/Web semantics', () => {
  test('validates and renders a composable dashboard through the public document parser', () => {
    const document = parseInlineDocument(structuredClone(fixture))
    expect(document.blocks[0]?.type).toBe('panel')
    expect(validateUiProgram(program()).nodes.length).toBe(45)
  })
  test('matches Rust states, linked calculations, visible trees, clipboard effects and exact Markdown after every interaction', () => {
    const p = validateUiProgram(program())
    let state: UiState = {}
    for (const expected of oracle.cases) {
      let copy: string | null = null
      const op = expected.operation
      if (op.type === 'input') state = inputUi(p, state, op.id, op.value).state
      if (op.type === 'activate') {
        const result = activateUi(p, state, op.id)
        state = result.state
        copy = result.copy
      }
      const view = renderUi(p, state)
      state = view.state
      expect(view).toEqual(expected.view)
      expect(uiMarkdown(view)).toBe(expected.markdown)
      expect(copy).toBe(expected.copy)
    }
  })
  test('keeps user selections when AI data changes and initializes only new state', () => {
    const p = program()
    let state = inputUi(p, {}, 'channel', 'paid').state
    state = inputUi(p, state, 'navigation', 'simulation').state
    state = inputUi(p, state, 'budget', 60000).state
    const next = structuredClone(p)
    next.data!.processed = 6
    next.data!.workLabel = '检查完成'
    next.state!.extra = { kind: 'boolean', initial: true }
    const reconciled = reconcileUiState(next, state)
    expect(reconciled.page).toBe('simulation')
    expect(reconciled.budget).toBe(60000)
    expect(reconciled.channel).toBe('paid')
    expect(reconciled.extra).toBe(true)
    next.state!.budget!.max = 20000
    expect(reconcileUiState(next, state).budget).toBe(12000)
  })
  test('rejects forged hidden/disabled control events and bad values without mutating the prior state', () => {
    const p = program(),
      before = renderUi(p).state,
      unchanged = structuredClone(before)
    expect(() => inputUi(p, before, 'budget', 60000)).toThrow()
    expect(() => inputUi(p, before, 'channel', 'unknown')).toThrow()
    const next = program()
    next.nodes.find((n) => n.id === 'channel')!.props!.disabled = true
    expect(() => inputUi(next, before, 'channel', 'paid')).toThrow()
    expect(before).toEqual(unchanged)
  })
  test('button sequences roll back if a later set is invalid and never execute an external capability', () => {
    const p = program()
    p.actions!.copySummary = [
      { type: 'set', key: 'page', value: 'tasks' },
      { type: 'set', key: 'budget', value: 100000 },
    ]
    const before = renderUi(p).state
    expect(() => activateUi(p, before, 'copy')).toThrow()
    expect(before.page).toBe('overview')
    expect(before.budget).toBe(12000)
    expect(() =>
      validateUiProgram({ ...p, actions: { hack: [{ type: 'fetch', url: 'https://example.com' }] } }),
    ).toThrow()
  })
  test('rejects arbitrary code, property escape, unsafe paths, cycles, invalid tree and unreachable nodes', () => {
    const edits: ((p: any) => void)[] = [
      (p) => (p.nodes[0].props.html = '<script>alert(1)</script>'),
      (p) => (p.nodes[0].props.style = 'position:fixed'),
      (p) => p.nodes[0].children.push('root'),
      (p) => p.nodes[0].children.push('not-found'),
      (p) => p.nodes.push({ id: 'unused', kind: 'text', props: { text: 'unused' } }),
      (p) => (p.computed.evil = { $op: 'eval', args: ['alert(1)'] }),
      (p) => (p.computed.evil = { $get: 'window.location' }),
      (p) => (p.computed.evil = { $get: 'state.__proto__' }),
      (p) => (p.computed.loop = { $get: 'computed.loop' }),
      (p) => (p.computed.loop = { $op: 'if', args: [false, { $get: 'computed.loop' }, 0] }),
      (p) => delete p.nodes.find((node: any) => node.id === 'channel').props.label,
      (p) => (p.computed.divide = { $op: 'div', args: [1, 0] }),
      (p) => (p.state.budget.initial = '12000'),
    ]
    for (const mutate of edits) {
      const p = program()
      mutate(p)
      expect(() => validateUiProgram(p)).toThrow()
    }
  })
  test('short-circuits guarded computations and treats null as missing, not zero', () => {
    const p = program()
    p.computed!.guard = {
      $op: 'if',
      args: [false, { $op: 'div', args: [1, 0] }, { $op: 'coalesce', args: [null, 0, 5] }],
    }
    p.nodes.find((n) => n.id === 'revenue')!.props!.value = { $get: 'computed.guard' }
    expect(validateUiProgram(p)).toBe(p)
    expect(renderUi(p).computed.guard).toBe(0)
  })
})

test('bounds resolved value expansion and aggregate allocation, not only the input or operator count', () => {
  const p = program()
  p.data!.repeatRows = Array(64).fill(0)
  p.data!.largeLabel = 'x'.repeat(6000)
  p.computed!.expanded = { $op: 'map', args: [{ $get: 'data.repeatRows' }, { $get: 'data.largeLabel' }] }
  expect(() => validateUiProgram(p)).toThrow('result budget')
  const many = program()
  many.data!.repeatRows = Array(16).fill(0)
  many.data!.largeLabel = 'x'.repeat(6000)
  for (let i = 0; i < 48; i++)
    many.computed![`expanded${i}`] = { $op: 'map', args: [{ $get: 'data.repeatRows' }, { $get: 'data.largeLabel' }] }
  expect(() => validateUiProgram(many)).toThrow('result budget')
})

test('the reported growth-rate and task example matches native TUI results', () => {
  const document = parseInlineDocument(
    JSON.parse(readFileSync(new URL('../../../examples/inline-ui/terminal-simulation.json', import.meta.url), 'utf8')),
  )
  const panel = document.blocks.find((block) => block.type === 'panel')
  if (!panel || panel.type !== 'panel') throw new Error('Missing simulation panel')
  const p = validateUiProgram(panel.program)
  expect(renderUi(p).computed.projection).toBe(678)
  let state = inputUi(p, {}, 'growth', 30).state
  expect(renderUi(p, state).computed.projection).toBe(735)
  expect(renderUi(p, state).computed.delta).toBe(170)
  state = inputUi(p, state, 'tasks', ['slider', 'tasks', 'toggle']).state
  expect(renderUi(p, state).computed.progress).toBe(100)
  state = inputUi(p, state, 'assumptions', true).state
  expect(uiMarkdown(renderUi(p, state))).toContain('计算假设')
  expect(activateUi(p, state, 'copy').copy).toBe('推演总值：735；增长率：30%')
  state = activateUi(p, state, 'reset').state
  expect(state.growth).toBe(20)
  expect(state.done).toEqual([])
  expect(state.showFormula).toBe(false)
})

test('a choice filter recomputes the existing dashboard chart and table together', () => {
  const p = validateUiProgram(program())
  const state = inputUi(p, {}, 'channel', 'paid').state
  const view = renderUi(p, state)
  const find = (node: any, id: string): any =>
    node.id === id ? node : node.children?.map((child: any) => find(child, id)).find(Boolean)
  expect(find(view.root, 'revenueChart').props.labels).toHaveLength(2)
  expect(find(view.root, 'salesTable').props.rows).toHaveLength(2)
})

test('the single-guide seven-node example validates and keeps local slider state through data updates', () => {
  const source = JSON.parse(
    readFileSync(new URL('../../../examples/inline-ui/quick-start.json', import.meta.url), 'utf8'),
  )
  const update = JSON.parse(
    readFileSync(new URL('../../../examples/inline-ui/quick-start-update.json', import.meta.url), 'utf8'),
  ).update
  const document = parseInlineDocument(structuredClone(source))
  expect(document.blocks[0]?.type).toBe('panel')
  const p = validateUiProgram(structuredClone(source.blocks[0].program))
  expect(p.nodes).toHaveLength(7)
  const initial = renderUi(p)
  expect(initial.root.children.some((node) => node.id === 'warning')).toBe(false)
  expect(initial.root.children.find((node) => node.id === 'summary')!.props!.value).toBe(150)
  const selected = inputUi(p, initial.state, 'slider', 50).state
  const after = renderUi(p, selected)
  expect(after.root.children.find((node) => node.id === 'summary')!.props!.value).toBe(180)
  expect(after.root.children.some((node) => node.id === 'warning')).toBe(true)
  expect(activateUi(p, selected, 'reset').state.percent).toBe(25)
  const next = structuredClone(p)
  Object.assign(next.data!, update.patch.data)
  expect(reconcileUiState(next, selected).percent).toBe(50)
  expect(renderUi(next, selected).root.children.find((node) => node.id === 'summary')!.props!.value).toBe(240)
  expect(() => inputUi(next, selected, 'slider', 110)).toThrow()
  expect(selected.percent).toBe(50)
})
