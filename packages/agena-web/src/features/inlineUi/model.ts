import { validateUiProgram, renderUi, uiMarkdown, type UiState } from './program'
import type { InlineDocument, InlineBlock } from '@/generated/agenaState'
export type { InlineDocument, InlineBlock } from '@/generated/agenaState'

export const INLINE_UI_MAX_BYTES = 60 * 1024
export const INLINE_UI_MAX_BLOCKS = 24

const record = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object' && !Array.isArray(v)
const check = (ok: boolean, message: string): void => {
  if (!ok) throw new Error(message)
}
function object(value: unknown, path: string, keys: string[]): asserts value is Record<string, unknown> {
  check(
    record(value) && Object.keys(value).every((key) => keys.includes(key)),
    `${path}: invalid object or unknown fields`,
  )
}
function text(value: unknown, path: string, max: number, required = true): void {
  check(
    typeof value === 'string' &&
      (!required || !!value.trim()) &&
      Array.from(value).length <= max &&
      !/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/u.test(value),
    `${path}: invalid text`,
  )
}
function optional(value: unknown, path: string, max: number): void {
  if (value !== undefined) text(value, path, max, false)
}
function array(value: unknown, path: string, min: number, max: number): asserts value is unknown[] {
  check(Array.isArray(value) && value.length >= min && value.length <= max, `${path}: expected ${min}..${max} items`)
}
function strings(value: unknown, path: string, min: number, max: number, length: number): asserts value is string[] {
  array(value, path, min, max)
  value.forEach((v, i) => text(v, `${path}[${i}]`, length))
}

/** Defense in depth for restored/remote content. Rust is the publication authority.
 * Cross-client fixtures cover defaults, bounds, unknown fields and copy semantics.
 * No HTML, dynamic component names, executable callbacks or remote bindings. */
export function parseInlineDocument(value: unknown): InlineDocument {
  const encoded = JSON.stringify(value)
  check(
    typeof encoded === 'string' && new TextEncoder().encode(encoded).byteLength <= INLINE_UI_MAX_BYTES,
    'Inline document exceeds 60 KiB',
  )
  object(value, 'document', ['version', 'title', 'blocks'])
  check(value.version === 1, 'Unsupported inline document version')
  optional(value.title, 'title', 160)
  array(value.blocks, 'blocks', 1, INLINE_UI_MAX_BLOCKS)
  const ids = new Set<string>()
  value.blocks.forEach((raw, index) => {
    check(record(raw), `blocks[${index}]: invalid block`)
    const b = raw as Record<string, unknown>,
      p = `blocks[${index}]`
    check(
      typeof b.id === 'string' && /^[A-Za-z0-9_-]{1,80}$/.test(b.id) && !ids.has(b.id),
      `${p}.id: invalid or duplicate id`,
    )
    ids.add(b.id as string)
    optional(b.title, `${p}.title`, 160)
    switch (b.type) {
      case 'panel':
        object(b, p, ['type', 'id', 'program'])
        validateUiProgram(b.program)
        break
      case 'paragraph':
        object(b, p, ['type', 'id', 'text'])
        text(b.text, `${p}.text`, 6000)
        break
      case 'callout':
        object(b, p, ['type', 'id', 'title', 'tone', 'text'])
        text(b.text, `${p}.text`, 3000)
        check(b.tone === undefined || ['note', 'tip', 'warning'].includes(String(b.tone)), `${p}.tone: invalid tone`)
        break
      case 'details':
        object(b, p, ['type', 'id', 'title', 'text'])
        text(b.title, `${p}.title`, 160)
        text(b.text, `${p}.text`, 6000)
        break
      case 'steps':
        object(b, p, ['type', 'id', 'title', 'items'])
        array(b.items, `${p}.items`, 1, 12)
        b.items.forEach((item, j) => {
          object(item, `${p}.items[${j}]`, ['label', 'title', 'text', 'icon'])
          optional(item.label, `${p}.items[${j}].label`, 24)
          text(item.title, `${p}.items[${j}].title`, 160)
          text(item.text, `${p}.items[${j}].text`, 3000)
          check(
            item.icon === undefined ||
              ['number', 'shield', 'layers', 'sparkles', 'link', 'check', 'search', 'code'].includes(String(item.icon)),
            `${p}.items[${j}].icon: invalid icon`,
          )
        })
        break
      case 'comparison':
        object(b, p, ['type', 'id', 'title', 'columns', 'rows'])
        strings(b.columns, `${p}.columns`, 2, 4, 120)
        array(b.rows, `${p}.rows`, 1, 16)
        b.rows.forEach((row, j) => {
          object(row, `${p}.rows[${j}]`, ['label', 'values'])
          text(row.label, `${p}.rows[${j}].label`, 160)
          strings(
            row.values,
            `${p}.rows[${j}].values`,
            (b.columns as string[]).length,
            (b.columns as string[]).length,
            1500,
          )
        })
        break
      case 'metrics':
        object(b, p, ['type', 'id', 'title', 'items'])
        array(b.items, `${p}.items`, 1, 6)
        b.items.forEach((item, j) => {
          object(item, `${p}.items[${j}]`, ['label', 'value', 'detail'])
          text(item.label, `${p}.items[${j}].label`, 120)
          text(item.value, `${p}.items[${j}].value`, 120)
          optional(item.detail, `${p}.items[${j}].detail`, 500)
        })
        break
      case 'table':
        object(b, p, ['type', 'id', 'title', 'columns', 'rows'])
        strings(b.columns, `${p}.columns`, 1, 8, 120)
        array(b.rows, `${p}.rows`, 1, 40)
        b.rows.forEach((row, j) => {
          array(row, `${p}.rows[${j}]`, (b.columns as string[]).length, (b.columns as string[]).length)
          row.forEach((cell, k) => text(cell, `${p}.rows[${j}][${k}]`, 1500, false))
        })
        break
      case 'chart': {
        object(b, p, ['type', 'id', 'title', 'chart_type', 'labels', 'series', 'x_label', 'y_label'])
        text(b.title, `${p}.title`, 160)
        optional(b.x_label, `${p}.x_label`, 120)
        optional(b.y_label, `${p}.y_label`, 120)
        check(['line', 'bar', 'area', 'pie'].includes(String(b.chart_type)), `${p}.chart_type: unsupported chart`)
        strings(b.labels, `${p}.labels`, 1, 128, 100)
        array(b.series, `${p}.series`, 1, 6)
        let hasNumber = false,
          hasPositive = false
        b.series.forEach((series, j) => {
          object(series, `${p}.series[${j}]`, ['name', 'values'])
          text(series.name, `${p}.series[${j}].name`, 120)
          array(series.values, `${p}.series[${j}].values`, (b.labels as string[]).length, (b.labels as string[]).length)
          series.values.forEach((v) => {
            if (v === null) return
            check(typeof v === 'number' && Number.isFinite(v) && Math.abs(v) <= 1e100, `${p}.series: nonfinite value`)
            hasNumber = true
            if ((v as number) > 0) hasPositive = true
            if (b.chart_type === 'pie') check((v as number) >= 0, `${p}.series: negative pie value`)
          })
        })
        check(hasNumber, `${p}.series: chart has no numeric data`)
        if (b.chart_type === 'pie')
          check(b.series.length === 1 && hasPositive, `${p}.series: pie needs one positive series`)
        break
      }
      default:
        throw new Error(`${p}.type: unsupported inline content`)
    }
  })
  return value as unknown as InlineDocument
}

const escape = (value: string) => value.replace(/[\\`*_{}\[\]<>()#+.!|\-]/g, '\\$&')
const cell = (value: string) => escape(value).replace(/\n/g, ' / ').replace(/\t/g, ' ')
function table(columns: string[], rows: string[][]): string {
  const row = (values: string[]) => `| ${values.map(cell).join(' | ')} |`
  return [row(columns), `| ${columns.map(() => '---').join(' | ')} |`, ...rows.map(row)].join('\n')
}

/** Shared semantics with InlineDocument::to_markdown; never copy DOM layout. */
export function inlineDocumentMarkdown(document: InlineDocument, panelStates: Record<string, UiState> = {}): string {
  const sections = document.title ? [`### ${escape(document.title)}`] : []
  for (const block of document.blocks) {
    let title = 'title' in block ? block.title || '' : '',
      body = ''
    switch (block.type) {
      case 'panel':
        body = uiMarkdown(renderUi(block.program, panelStates[block.id] || {}))
        break
      case 'paragraph':
      case 'details':
        body = escape(block.text)
        break
      case 'callout':
        body = block.text
          .split('\n')
          .map((line) => `> ${escape(line)}`)
          .join('\n')
        break
      case 'steps':
        body = block.items
          .map(
            (item, i) =>
              `${i + 1}. **${item.label ? escape(item.label) + ' · ' : ''}${escape(item.title)}**\n\n${item.text
                .split('\n')
                .map((line) => '   ' + escape(line))
                .join('\n')}`,
          )
          .join('\n\n')
        break
      case 'comparison':
        body = table(
          ['', ...block.columns],
          block.rows.map((row) => [row.label, ...row.values]),
        )
        break
      case 'metrics':
        body = block.items
          .map(
            (item) =>
              `- **${escape(item.label)}:** ${escape(item.value)}${item.detail ? ' — ' + escape(item.detail) : ''}`,
          )
          .join('\n')
        break
      case 'table':
        body = table(block.columns, block.rows)
        break
      case 'chart':
        body = table(
          [block.x_label || '', ...block.series.map((s) => (block.y_label ? `${s.name} (${block.y_label})` : s.name))],
          block.labels.map((label, i) => [
            label,
            ...block.series.map((s) => (s.values[i] == null ? '—' : String(s.values[i]))),
          ]),
        )
        break
    }
    sections.push(title ? `#### ${escape(title)}\n\n${body}` : body)
  }
  return sections.join('\n\n')
}

export function inlineChartSpec(block: Extract<InlineBlock, { type: 'chart' }>) {
  return {
    type: block.chart_type,
    title: block.title,
    labels: block.labels,
    series: block.series,
    xLabel: block.x_label,
    yLabel: block.y_label,
  }
}
