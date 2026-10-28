/** Bounded declarative expression/state runtime. Mirrors the Rust UiProgram;
 * parity fixtures exercise both implementations. No eval, functions or DOM API. */
import type { UiKind, UiProgram, UiStateField } from '@/generated/agenaState'
export type { UiAction, UiKind, UiNode, UiProgram, UiStateField } from '@/generated/agenaState'
export type UiState = Record<string, unknown>
export type UiResolvedNode = {
  id: string
  kind: UiKind
  props: Record<string, unknown>
  children: UiResolvedNode[]
  bind: string | null
  value: unknown
  field: UiStateField | null
  action: string | null
}
export type UiView = { root: UiResolvedNode; state: UiState; computed: UiState }
export type UiActionResult = { state: UiState; copy: string | null }
export const MAX_UI_NODES = 128
export const UI_EVALUATION_BUDGET = 30000
export const containerKinds = new Set<UiKind>(['stack', 'row', 'grid', 'card', 'details'])
export const controlKinds = new Set<UiKind>([
  'tabs',
  'select',
  'checkbox',
  'switch',
  'slider',
  'number',
  'input',
  'tasks',
  'button',
])
const containers = ['title', 'description', 'columns', 'gap', 'tone']
const input = ['label', 'description', 'disabled']
const numericInput = [...input, 'prefix', 'suffix']
export const UI_PROPS: Record<UiKind, readonly string[]> = {
  stack: containers,
  row: containers,
  grid: containers,
  card: containers,
  text: ['text', 'tone', 'size'],
  heading: ['text', 'tone', 'size'],
  metric: ['label', 'value', 'prefix', 'suffix', 'detail', 'trend', 'tone'],
  badge: ['text', 'tone'],
  divider: [],
  tabs: input,
  select: input,
  checkbox: input,
  switch: input,
  slider: numericInput,
  number: numericInput,
  input: [...input, 'placeholder'],
  button: ['label', 'tone', 'disabled'],
  progress: ['label', 'value', 'max', 'detail', 'tone'],
  chart: ['title', 'chart_type', 'labels', 'series', 'x_label', 'y_label'],
  table: ['title', 'columns', 'rows', 'empty'],
  tasks: ['title', 'items', 'disabled', 'empty'],
  timeline: ['title', 'items'],
  callout: ['title', 'text', 'tone'],
  details: ['title'],
}
const operators = new Set([
  'add',
  'sub',
  'mul',
  'div',
  'min',
  'max',
  'round',
  'clamp',
  'eq',
  'ne',
  'lt',
  'lte',
  'gt',
  'gte',
  'and',
  'or',
  'not',
  'if',
  'coalesce',
  'concat',
  'length',
  'sum',
  'average',
  'filter',
  'map',
  'pluck',
  'includes',
  'format',
])
const record = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object' && !Array.isArray(v)
const fail = (message: string): never => {
  throw new Error(message)
}
const assert = (condition: unknown, message: string) => {
  if (!condition) fail(message)
}
export const uiIdentifier = (value: unknown): value is string =>
  typeof value === 'string' &&
  /^[A-Za-z0-9_-]{1,80}$/.test(value) &&
  !['__proto__', 'prototype', 'constructor'].includes(value)
const text = (v: unknown, max = 6000): v is string =>
  typeof v === 'string' && Array.from(v).length <= max && !/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/u.test(v)
const numeric = (v: unknown): number =>
  typeof v === 'number' && Number.isFinite(v) && Math.abs(v) <= 1e100 ? v : fail('Expected a finite number')
const number = (v: number) => numeric(v === 0 ? 0 : v)
const bool = (v: unknown): boolean => (typeof v === 'boolean' ? v : fail('Expected a boolean'))
const array = (v: unknown): unknown[] => (Array.isArray(v) ? v : fail('Expected an array'))
const own = (v: unknown, key: string): unknown => (record(v) && Object.hasOwn(v, key) ? v[key] : null)
export function uiDisplay(v: unknown): string {
  return v == null ? '—' : typeof v === 'string' ? v : typeof v === 'object' ? JSON.stringify(v) : String(v)
}
function equal(a: unknown, b: unknown): boolean {
  if (a === b) return true
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((v, i) => equal(v, b[i]))
  if (record(a) && record(b)) {
    const keys = Object.keys(a)
    return keys.length === Object.keys(b).length && keys.every((k) => Object.hasOwn(b, k) && equal(a[k], b[k]))
  }
  return false
}
function safeValue(value: unknown, depth = 0, budget = { count: 0 }): void {
  if (++budget.count > 12000 || depth > 24) fail('UI value complexity budget exceeded')
  if (typeof value === 'string') assert(text(value), 'Invalid UI text')
  else if (typeof value === 'number') numeric(value)
  else if (Array.isArray(value)) {
    assert(value.length <= 1024, 'UI array exceeds 1024 items')
    value.forEach((v) => safeValue(v, depth + 1, budget))
  } else if (record(value)) {
    assert(Object.keys(value).length <= 128, 'UI object exceeds 128 fields')
    for (const [k, v] of Object.entries(value)) {
      assert(text(k, 120) && k.trim() && !['__proto__', 'prototype', 'constructor'].includes(k), 'Unsafe UI key')
      safeValue(v, depth + 1, budget)
    }
  } else assert(value == null || typeof value === 'boolean', 'Unsupported UI value')
}
function expression(value: unknown, depth = 0): void {
  assert(depth <= 24, 'Expression depth exceeded')
  if (record(value)) {
    if (Object.hasOwn(value, '$get')) {
      assert(
        Object.keys(value).length === 1 && typeof value.$get === 'string' && value.$get.length <= 240,
        'Invalid lookup',
      )
      const pieces = String(value.$get).split('.')
      assert(
        pieces.length >= 2 &&
          pieces.length <= 16 &&
          ['state', 'data', 'computed', 'item'].includes(pieces[0]!) &&
          pieces.slice(1).every(uiIdentifier),
        'Invalid lookup path',
      )
    } else if (Object.hasOwn(value, '$op')) {
      const op = String(value.$op),
        args = array(value.args)
      assert(Object.keys(value).length === 2 && operators.has(op) && args.length <= 32, 'Unsupported operator')
      const valid = ['if', 'clamp'].includes(op)
        ? args.length === 3
        : ['not', 'length', 'sum', 'average'].includes(op)
          ? args.length === 1
          : ['sub', 'div', 'eq', 'ne', 'lt', 'lte', 'gt', 'gte', 'filter', 'map', 'pluck', 'includes'].includes(op)
            ? args.length === 2
            : ['round', 'format'].includes(op)
              ? args.length >= 1 && args.length <= 2
              : args.length >= 1
      assert(valid, 'Wrong number of operator arguments')
      args.forEach((arg) => expression(arg, depth + 1))
    } else if (Object.hasOwn(value, '$literal')) assert(Object.keys(value).length === 1, 'Invalid literal')
    else
      for (const [key, v] of Object.entries(value)) {
        assert(!key.startsWith('$'), 'Unknown expression')
        expression(v, depth + 1)
      }
  } else if (Array.isArray(value)) value.forEach((v) => expression(v, depth + 1))
}
export function stateAccepts(field: UiStateField, value: unknown): boolean {
  switch (field.kind) {
    case 'boolean':
      return typeof value === 'boolean'
    case 'number':
      return (
        typeof value === 'number' &&
        Number.isFinite(value) &&
        Math.abs(value) <= 1e100 &&
        (field.min == null || value >= field.min) &&
        (field.max == null || value <= field.max)
      )
    case 'text':
      return text(value, field.max_length ?? 1024)
    case 'choice':
      return typeof value === 'string' && (field.options || []).some((o) => o.value === value)
    case 'selection':
      return (
        Array.isArray(value) && value.length <= 256 && value.every(uiIdentifier) && new Set(value).size === value.length
      )
  }
}
export function reconcileUiState(program: UiProgram, previous: UiState = {}): UiState {
  const state: UiState = Object.create(null)
  for (const [key, field] of Object.entries(program.state || {}))
    state[key] = stateAccepts(field, previous[key]) ? previous[key] : structuredClone(field.initial)
  return state
}
// Keep these deterministic allocation charges in sync with the Rust evaluator.
// A short expression can otherwise expand a tiny input into an enormous result.
const MAX_RESULT_BYTES = 256 * 1024
const MAX_PRODUCED_BYTES = 4 * 1024 * 1024
const resultEncoder = new TextEncoder()
function resultSize(value: unknown): number {
  let bytes = 0
  function visit(item: unknown, depth: number): void {
    bytes += 16
    assert(depth <= 64 && bytes <= MAX_RESULT_BYTES, 'UI result budget exceeded')
    if (typeof item === 'string') bytes += resultEncoder.encode(item).byteLength
    else if (Array.isArray(item)) for (const child of item) visit(child, depth + 1)
    else if (record(item))
      for (const [key, child] of Object.entries(item)) {
        bytes += resultEncoder.encode(key).byteLength
        visit(child, depth + 1)
      }
    assert(bytes <= MAX_RESULT_BYTES, 'UI result budget exceeded')
  }
  visit(value, 0)
  return bytes
}

class Evaluator {
  cache: UiState = Object.create(null)
  active = new Set<string>()
  steps = 0
  depth = 0
  producedBytes = 0
  readonly program: UiProgram
  readonly state: UiState
  constructor(program: UiProgram, state: UiState) {
    this.program = program
    this.state = state
  }
  computed(key: string): unknown {
    if (Object.hasOwn(this.cache, key)) return this.cache[key]
    assert(!this.active.has(key), `Cyclic computed dependency: ${key}`)
    assert(Object.hasOwn(this.program.computed || {}, key), `Unknown computed value: ${key}`)
    this.active.add(key)
    const result = this.eval(this.program.computed![key])
    this.active.delete(key)
    this.cache[key] = result
    return result
  }
  eval(value: unknown, item: unknown = null): unknown {
    assert(++this.steps <= UI_EVALUATION_BUDGET && ++this.depth <= 64, 'UI evaluation budget exceeded')
    try {
      const result = this.inner(value, item)
      this.producedBytes += resultSize(result)
      assert(this.producedBytes <= MAX_PRODUCED_BYTES, 'UI result budget exceeded')
      return result
    } finally {
      this.depth--
    }
  }
  private inner(value: unknown, item: unknown): unknown {
    if (record(value)) {
      if (Object.hasOwn(value, '$literal')) return value.$literal
      if (Object.hasOwn(value, '$get')) {
        const [root, key = '', ...path] = String(value.$get).split('.')
        let current =
          root === 'state'
            ? own(this.state, key)
            : root === 'data'
              ? own(this.program.data, key)
              : root === 'computed'
                ? this.computed(key)
                : root === 'item'
                  ? own(item, key)
                  : null
        for (const part of path) current = own(current, part)
        return current
      }
      if (Object.hasOwn(value, '$op')) {
        const op = String(value.$op),
          args = array(value.args)
        if (op === 'if') return this.eval(bool(this.eval(args[0], item)) ? args[1] : args[2], item)
        if (op === 'and' || op === 'or') {
          for (const arg of args) {
            const v = bool(this.eval(arg, item))
            if (v === (op === 'or')) return v
          }
          return op === 'and'
        }
        if (op === 'coalesce') {
          for (const arg of args) {
            const v = this.eval(arg, item)
            if (v != null) return v
          }
          return null
        }
        if (op === 'map' || op === 'filter') {
          const rows = array(this.eval(args[0], item))
          assert(rows.length <= 1024, 'Collection too large')
          return op === 'map'
            ? rows.map((row) => this.eval(args[1], row))
            : rows.filter((row) => bool(this.eval(args[1], row)))
        }
        const values = args.map((v) => this.eval(v, item))
        switch (op) {
          case 'add':
            return number(values.map(numeric).reduce((a, b) => a + b, 0))
          case 'mul':
            return number(values.map(numeric).reduce((a, b) => a * b, 1))
          case 'sub':
            return number(numeric(values[0]) - numeric(values[1]))
          case 'div': {
            const denominator = numeric(values[1])
            assert(denominator !== 0, 'Division by zero')
            return number(numeric(values[0]) / denominator)
          }
          case 'min':
            return number(Math.min(...values.map(numeric)))
          case 'max':
            return number(Math.max(...values.map(numeric)))
          case 'round':
          case 'format': {
            const digits = values.length > 1 ? numeric(values[1]) : 0
            assert(Number.isInteger(digits) && digits >= 0 && digits <= 6, 'Invalid decimal digits')
            const value = numeric(values[0]),
              scale = 10 ** digits
            const rounded = number((Math.sign(value) * Math.floor(Math.abs(value * scale) + 0.5)) / scale)
            return op === 'format' ? rounded.toFixed(digits) : rounded
          }
          case 'clamp': {
            const value = numeric(values[0]),
              min = numeric(values[1]),
              max = numeric(values[2])
            assert(min <= max, 'Invalid clamp range')
            return number(Math.min(max, Math.max(min, value)))
          }
          case 'eq':
            return equal(values[0], values[1])
          case 'ne':
            return !equal(values[0], values[1])
          case 'lt':
            return numeric(values[0]) < numeric(values[1])
          case 'lte':
            return numeric(values[0]) <= numeric(values[1])
          case 'gt':
            return numeric(values[0]) > numeric(values[1])
          case 'gte':
            return numeric(values[0]) >= numeric(values[1])
          case 'not':
            return !bool(values[0])
          case 'concat': {
            const result = values.map(uiDisplay).join('')
            assert(new TextEncoder().encode(result).byteLength <= 32768, 'Concatenated text too large')
            return result
          }
          case 'length':
            return Array.isArray(values[0])
              ? values[0].length
              : typeof values[0] === 'string'
                ? Array.from(values[0]).length
                : fail('length expects array or text')
          case 'sum':
          case 'average': {
            const rows = array(values[0])
            const sum = rows.map(numeric).reduce((a, b) => a + b, 0)
            return number(op === 'average' && rows.length ? sum / rows.length : sum)
          }
          case 'pluck': {
            const key = values[1]
            assert(uiIdentifier(key), 'Invalid field name')
            return array(values[0]).map((row) => own(row, key as string))
          }
          case 'includes':
            return Array.isArray(values[0])
              ? values[0].some((v) => equal(v, values[1]))
              : typeof values[0] === 'string' && typeof values[1] === 'string'
                ? values[0].includes(values[1])
                : fail('Invalid includes operands')
          default:
            return fail('Unsupported operation')
        }
      }
      const result: Record<string, unknown> = Object.create(null)
      for (const [key, v] of Object.entries(value)) result[key] = this.eval(v, item)
      return result
    }
    return Array.isArray(value) ? value.map((v) => this.eval(v, item)) : value
  }
}
function validateProps(kind: UiKind, props: Record<string, unknown>): void {
  if (controlKinds.has(kind) && kind !== 'tasks')
    assert(text(props.label, 160) && props.label.trim(), 'Controls require a text label')
  for (const [key, value] of Object.entries(props)) {
    if (typeof value === 'string') assert(text(value), 'Resolved UI text exceeds limits')
    if (key === 'disabled') assert(typeof value === 'boolean', 'disabled must be boolean')
    else if (key === 'columns' && kind === 'grid')
      assert(Number.isInteger(value) && Number(value) >= 1 && Number(value) <= 4, 'Grid columns must be 1..4')
    else if (key === 'tone')
      assert(['neutral', 'info', 'success', 'warning', 'danger', 'primary'].includes(String(value)), 'Invalid tone')
    else if (key === 'gap' || key === 'size') assert(['sm', 'md', 'lg'].includes(String(value)), 'Invalid size token')
    else if ((key === 'value' || key === 'max') && kind === 'progress')
      assert(numeric(value) >= 0, 'Progress must be nonnegative')
    else if (key === 'columns' && kind === 'table') {
      const columns = array(value)
      assert(
        columns.length >= 1 &&
          columns.length <= 8 &&
          columns.every((c) => record(c) && uiIdentifier(c.key) && typeof c.label === 'string'),
        'Invalid table columns',
      )
    } else if (key === 'rows')
      assert(Array.isArray(value) && value.length <= 256 && value.every(record), 'Invalid table rows')
    else if (key === 'items') {
      const items = array(value)
      assert(
        items.length <= 128 &&
          items.every((v) => record(v) && uiIdentifier(v.id) && typeof v.title === 'string') &&
          new Set(items.map((v) => (v as Record<string, unknown>).id)).size === items.length,
        'Invalid or duplicate items',
      )
    } else if (['labels', 'series', 'chart_type'].includes(key)) {
      /* checked together below */
    } else
      assert(value == null || typeof value === 'string' || typeof value === 'number', `${key} requires text or number`)
  }
  if (kind === 'chart') {
    const labels = array(props.labels),
      series = array(props.series)
    assert(labels.length <= 128 && labels.every((v) => text(v, 100) && v.trim()), 'Invalid chart labels')
    assert(['line', 'bar', 'area', 'pie'].includes(String(props.chart_type || 'line')), 'Invalid chart type')
    assert(series.length <= 6 && (series.length > 0 || labels.length === 0), 'Invalid chart series count')
    for (const item of series)
      assert(
        record(item) &&
          text(item.name, 120) &&
          item.name.trim() &&
          Array.isArray(item.values) &&
          item.values.length === labels.length &&
          item.values.every((v) => v === null || (typeof v === 'number' && Number.isFinite(v) && Math.abs(v) <= 1e100)),
        'Invalid chart series',
      )
    if (labels.length && props.chart_type === 'pie') {
      const values = (series[0] as { values: unknown[] }).values
      assert(
        series.length === 1 &&
          values.every((v) => v === null || Number(v) >= 0) &&
          values.some((v) => typeof v === 'number' && v > 0),
        'Invalid pie values',
      )
    }
    if (labels.length)
      assert(
        series.some((s) => (s as { values: unknown[] }).values.some((v) => v !== null)),
        'Chart has no numeric data',
      )
  }
}
export function validateUiProgram(raw: unknown): UiProgram {
  assert(record(raw), 'Invalid UI program')
  const program = raw as UiProgram
  assert(
    Object.keys(program).every((k) => ['root', 'nodes', 'data', 'state', 'computed', 'actions'].includes(k)),
    'Unknown program field',
  )
  safeValue(raw)
  assert(
    uiIdentifier(program.root) &&
      Array.isArray(program.nodes) &&
      program.nodes.length > 0 &&
      program.nodes.length <= MAX_UI_NODES,
    'Invalid program root/nodes',
  )
  const state = program.state || {},
    computed = program.computed || {},
    actions = program.actions || {},
    data = program.data || {}
  assert(
    record(state) &&
      record(computed) &&
      record(actions) &&
      record(data) &&
      Object.keys(state).length <= 48 &&
      Object.keys(computed).length <= 64 &&
      Object.keys(actions).length <= 64 &&
      Object.keys(data).length <= 64,
    'Program namespace budget exceeded',
  )
  const ids = new Set<string>()
  for (const node of program.nodes) {
    assert(
      record(node) &&
        Object.keys(node).every((k) => ['id', 'kind', 'props', 'children', 'bind', 'visible', 'action'].includes(k)),
      'Unknown node field',
    )
    assert(uiIdentifier(node.id) && !ids.has(node.id) && Object.hasOwn(UI_PROPS, node.kind), 'Invalid/duplicate node')
    ids.add(node.id)
    assert(
      Array.isArray(node.children || []) &&
        (node.children || []).length <= 48 &&
        (containerKinds.has(node.kind) || !(node.children || []).length),
      'Invalid children',
    )
    assert(record(node.props || {}), 'Invalid props')
    for (const [key, value] of Object.entries(node.props || {})) {
      assert(UI_PROPS[node.kind].includes(key), `Unsupported property ${key}`)
      expression(value)
    }
    if (node.visible != null) expression(node.visible)
    if (controlKinds.has(node.kind) && node.kind !== 'button') assert(node.bind, 'Control requires bind')
    if (node.bind != null) {
      const expected = ['tabs', 'select'].includes(node.kind)
        ? 'choice'
        : ['checkbox', 'switch'].includes(node.kind)
          ? 'boolean'
          : ['slider', 'number'].includes(node.kind)
            ? 'number'
            : node.kind === 'input'
              ? 'text'
              : node.kind === 'tasks'
                ? 'selection'
                : null
      assert(expected && state[node.bind]?.kind === expected, 'Control state type mismatch')
    }
    if (node.action != null) assert(node.kind === 'button' && Object.hasOwn(actions, node.action), 'Invalid action')
    if (node.kind === 'button') assert(node.action, 'Button requires action')
  }
  for (const [key, field] of Object.entries(state)) {
    assert(
      uiIdentifier(key) &&
        record(field) &&
        Object.keys(field).every((k) =>
          ['kind', 'initial', 'min', 'max', 'step', 'options', 'max_length'].includes(k),
        ) &&
        stateAccepts(field, field.initial) &&
        (field.max_length ?? 1024) <= 6000 &&
        (field.options || []).length <= 64,
      'Invalid state declaration',
    )
    if (field.kind === 'number')
      assert(
        field.min != null && field.max != null && numeric(field.min) <= numeric(field.max),
        'Numeric state requires bounds',
      )
    if (field.step != null) assert(numeric(field.step) > 0, 'Numeric step must be positive')
    const options = field.options || []
    assert(
      options.every(
        (o) =>
          record(o) &&
          uiIdentifier(o.value) &&
          text(o.label, 160) &&
          o.label.trim() &&
          Object.keys(o).every((k) => k === 'value' || k === 'label'),
      ) && new Set(options.map((o) => o.value)).size === options.length,
      'Invalid state options',
    )
  }
  for (const key of Object.keys(data)) assert(uiIdentifier(key), 'Invalid data key')
  for (const [key, value] of Object.entries(computed)) {
    assert(uiIdentifier(key), 'Invalid computed key')
    expression(value)
  }
  function dependencies(value: unknown, found: Set<string>) {
    if (record(value)) {
      if (Object.hasOwn(value, '$literal')) return
      if (typeof value.$get === 'string' && value.$get.startsWith('computed.')) found.add(value.$get.split('.')[1]!)
      for (const v of Object.values(value)) dependencies(v, found)
    } else if (Array.isArray(value)) value.forEach((v) => dependencies(v, found))
  }
  const done = new Set<string>(),
    active = new Set<string>()
  function visitComputed(key: string) {
    if (done.has(key)) return
    assert(!active.has(key), `Cyclic computed dependency ${key}`)
    assert(Object.hasOwn(computed, key), `Unknown computed dependency ${key}`)
    active.add(key)
    const links = new Set<string>()
    dependencies(computed[key], links)
    links.forEach(visitComputed)
    active.delete(key)
    done.add(key)
  }
  Object.keys(computed).forEach(visitComputed)
  for (const [key, list] of Object.entries(actions)) {
    assert(uiIdentifier(key) && Array.isArray(list) && list.length > 0 && list.length <= 16, 'Invalid action list')
    for (const action of list) {
      assert(record(action), 'Invalid action')
      const allowed =
        action.type === 'set'
          ? ['type', 'key', 'value']
          : action.type === 'toggle'
            ? ['type', 'key']
            : action.type === 'reset'
              ? ['type']
              : action.type === 'copy'
                ? ['type', 'text']
                : []
      assert(allowed.length && Object.keys(action).every((k) => allowed.includes(k)), 'Unknown action/field')
      if (action.type === 'set') {
        assert(state[action.key], 'Unknown state')
        expression(action.value)
      }
      if (action.type === 'toggle') assert(state[action.key]?.kind === 'boolean', 'Toggle requires boolean')
      if (action.type === 'copy') expression(action.text)
    }
  }
  const byId = new Map(program.nodes.map((node) => [node.id, node])),
    visited = new Set<string>()
  function walk(id: string, depth: number) {
    assert(depth <= 12 && !visited.has(id) && byId.has(id), 'Cyclic/shared/missing subtree')
    visited.add(id)
    for (const child of byId.get(id)!.children || []) walk(child, depth + 1)
  }
  walk(program.root, 0)
  assert(visited.size === byId.size, 'Unreachable nodes')
  const evaluator = new Evaluator(program, reconcileUiState(program))
  Object.keys(computed).forEach((key) => evaluator.computed(key))
  renderUi(program)
  return program
}
export function renderUi(program: UiProgram, previous: UiState = {}): UiView {
  const state = reconcileUiState(program, previous),
    evaluator = new Evaluator(program, state),
    nodes = new Map(program.nodes.map((node) => [node.id, node]))
  function resolve(id: string, depth: number): UiResolvedNode | null {
    assert(depth <= 12, 'Render depth exceeded')
    const node = nodes.get(id)
    assert(node, 'Missing node')
    if (node!.visible != null && !bool(evaluator.eval(node!.visible))) return null
    const props: Record<string, unknown> = Object.create(null)
    for (const [key, value] of Object.entries(node!.props || {})) props[key] = evaluator.eval(value)
    validateProps(node!.kind, props)
    return {
      id,
      kind: node!.kind,
      props,
      children: (node!.children || [])
        .map((child) => resolve(child, depth + 1))
        .filter((child): child is UiResolvedNode => !!child),
      bind: node!.bind ?? null,
      value: node!.bind ? state[node!.bind] : null,
      field:
        node!.bind && program.state?.[node!.bind]
          ? {
              ...program.state[node!.bind]!,
              options: program.state[node!.bind]!.options || [],
              max_length: program.state[node!.bind]!.max_length ?? 1024,
            }
          : null,
      action: node!.action ?? null,
    }
  }
  const root = resolve(program.root, 0)
  assert(root, 'Root must be visible')
  return { root: root!, state, computed: evaluator.cache }
}
export function findUiNode(node: UiResolvedNode, id: string): UiResolvedNode | null {
  if (node.id === id) return node
  for (const child of node.children) {
    const found = findUiNode(child, id)
    if (found) return found
  }
  return null
}
export function inputUi(program: UiProgram, previous: UiState, nodeId: string, value: unknown): UiActionResult {
  const view = renderUi(program, previous),
    node = findUiNode(view.root, nodeId)
  assert(node && node.bind && node.props.disabled !== true, 'Control is hidden/disabled')
  const field = program.state?.[node!.bind!]
  assert(field && stateAccepts(field, value), 'Invalid input value')
  const state = { ...view.state, [node!.bind!]: value }
  renderUi(program, state)
  return { state, copy: null }
}
export function activateUi(program: UiProgram, previous: UiState, nodeId: string): UiActionResult {
  const view = renderUi(program, previous),
    node = findUiNode(view.root, nodeId)
  assert(node?.kind === 'button' && node.props.disabled !== true && node.action, 'Button is hidden/disabled')
  let state = view.state,
    copy: string | null = null
  for (const action of program.actions?.[node!.action!] || []) {
    if (action.type === 'set') {
      const value = new Evaluator(program, state).eval(action.value)
      assert(
        program.state?.[action.key] && stateAccepts(program.state[action.key]!, value),
        'Action produced invalid state',
      )
      state = { ...state, [action.key]: value }
    } else if (action.type === 'toggle') state = { ...state, [action.key]: !bool(state[action.key]) }
    else if (action.type === 'reset') state = reconcileUiState(program)
    else {
      const value = new Evaluator(program, state).eval(action.text)
      assert(typeof value === 'string' && new TextEncoder().encode(value).byteLength <= 32768, 'Invalid clipboard text')
      copy = value as string
    }
  }
  renderUi(program, state)
  return { state, copy }
}
const md = (text: string) => text.replace(/[\\`*_{}\[\]<>()#+.!|\-]/g, '\\$&')
/** Semantic fallback, not scraped DOM. Rust and Web parity fixtures compare it. */
export function uiMarkdown(view: UiView): string {
  function nodeText(node: UiResolvedNode, depth: number): string {
    const p = (key: string) => (node.props[key] == null ? '' : uiDisplay(node.props[key])),
      parts: string[] = []
    if (p('title')) parts.push(`${'#'.repeat(Math.min(depth + 3, 6))} ${md(p('title'))}`)
    let body = ''
    switch (node.kind) {
      case 'stack':
      case 'row':
      case 'grid':
      case 'card':
        body = md(p('description'))
        break
      case 'text':
        body = md(p('text'))
        break
      case 'heading':
        body = `#### ${md(p('text'))}`
        break
      case 'metric':
        body = `**${md(p('label'))}:** ${md(p('prefix'))}${md(p('value'))}${md(p('suffix'))}${p('detail') ? ' — ' + md(p('detail')) : ''}`
        break
      case 'badge':
        body = `**${md(p('text'))}**`
        break
      case 'divider':
        body = '---'
        break
      case 'tabs':
      case 'select':
        body = `**${md(p('label'))}:** ${md(node.field?.options?.find((o) => o.value === node.value)?.label || String(node.value || ''))}`
        break
      case 'checkbox':
      case 'switch':
        body = `[${node.value === true ? 'x' : ' '}] ${md(p('label'))}`
        break
      case 'slider':
      case 'number':
      case 'input':
        body = `**${md(p('label'))}:** ${md(p('prefix'))}${md(uiDisplay(node.value))}${md(p('suffix'))}`
        break
      case 'button':
        body = `[ ${md(p('label'))} ]`
        break
      case 'progress': {
        const ratio =
            Number(node.props.max ?? 100) > 0
              ? Math.max(0, Math.min(1, Number(node.props.value || 0) / Number(node.props.max ?? 100)))
              : 0,
          n = Math.floor(ratio * 16 + 0.5)
        body = `**${md(p('label'))}:** ${'■'.repeat(n)}${'□'.repeat(16 - n)} ${(ratio * 100).toFixed(0)}%${p('detail') ? ' — ' + md(p('detail')) : ''}`
        break
      }
      case 'tasks':
      case 'timeline':
        body = array(node.props.items || [])
          .map((row) => {
            const r = row as Record<string, unknown>
            return `- ${node.kind === 'tasks' ? '[' + (Array.isArray(node.value) && node.value.includes(r.id) ? 'x' : ' ') + '] ' : ''}**${md(String(r.title || ''))}**${r.detail == null ? '' : ' — ' + md(uiDisplay(r.detail))}`
          })
          .join('\n')
        break
      case 'callout':
        body = '> ' + md(p('text')).replace(/\n/g, '\n> ')
        break
      case 'details':
        break
      case 'table': {
        const cols = array(node.props.columns || []) as { key: string; label: string }[],
          rows = array(node.props.rows || []) as Record<string, unknown>[]
        body = [
          `| ${cols.map((c) => md(c.label)).join(' | ')} |`,
          `| ${cols.map(() => '---').join(' | ')} |`,
          ...rows.map((row) => `| ${cols.map((c) => md(uiDisplay(row[c.key])).replace(/\n/g, ' / ')).join(' | ')} |`),
        ].join('\n')
        break
      }
      case 'chart': {
        const labels = array(node.props.labels || []),
          series = array(node.props.series || []) as { name: string; values: unknown[] }[],
          cols = [md(p('x_label')), ...series.map((s) => md(s.name))]
        body = [
          ...(p('y_label') ? [md(p('y_label'))] : []),
          `| ${cols.join(' | ')} |`,
          `| ${cols.map(() => '---').join(' | ')} |`,
          ...labels.map(
            (label, i) => `| ${[md(uiDisplay(label)), ...series.map((s) => md(uiDisplay(s.values[i])))].join(' | ')} |`,
          ),
        ].join('\n')
        break
      }
    }
    if (body) parts.push(body)
    for (const child of node.children)
      parts.push(nodeText(child, depth + (['card', 'details'].includes(node.kind) ? 1 : 0)))
    return parts.join('\n\n')
  }
  return nodeText(view.root, 0)
}
