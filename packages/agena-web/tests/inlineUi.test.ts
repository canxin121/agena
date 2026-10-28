import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { parseInlineDocument, inlineDocumentMarkdown } from '../src/features/inlineUi/model'
const fixture = JSON.parse(readFileSync(new URL('../../../examples/inline-ui/reply.json', import.meta.url), 'utf8'))
const markdown = readFileSync(new URL('../../../examples/inline-ui/reply.md', import.meta.url), 'utf8').trimEnd()
const sample = () => structuredClone(fixture)

describe('Inline Generative UI wire contract', () => {
  test('renders all eight kinds and copies exactly the Rust/TUI projection', () => {
    const doc = parseInlineDocument(sample())
    expect(doc.blocks).toHaveLength(8)
    expect(inlineDocumentMarkdown(doc)).toBe(markdown)
  })
  test('rejects unknown versions, ids, content and executable fields', () => {
    for (const mutate of [
      (v: any) => {
        v.version = 2
      },
      (v: any) => {
        v.blocks[1].id = v.blocks[0].id
      },
      (v: any) => {
        v.blocks[0].html = '<script>alert(1)</script>'
      },
      (v: any) => {
        v.blocks[0].type = 'application'
      },
      (v: any) => {
        v.blocks[0].id = '../escape'
      },
      (v: any) => {
        v.blocks[0].text = '\x1b[2J'
      },
      (v: any) => {
        v.blocks[0].text = 'x'.repeat(65536)
      },
      (v: any) => {
        v.blocks = []
      },
      (v: any) => {
        v.blocks[2].rows[0].values = ['missing column']
      },
      (v: any) => {
        v.blocks[5].rows[0] = []
      },
      (v: any) => {
        v.blocks[7].series[0].values = [0, 1]
      },
      (v: any) => {
        v.blocks[7].series[0].values = [null, null, null]
      },
    ]) {
      const value = sample()
      mutate(value)
      expect(() => parseInlineDocument(value)).toThrow()
    }
  })
  test('keeps zeros and gaps distinct and rejects invalid pie data', () => {
    const value = sample()
    value.blocks[7].series[0].values = [0, null, 2]
    expect(inlineDocumentMarkdown(parseInlineDocument(value))).toContain('| A | 0 |')
    expect(inlineDocumentMarkdown(parseInlineDocument(value))).toContain('| B | — |')
    value.blocks[7].chart_type = 'pie'
    value.blocks[7].series[0].values = [0, null, -2]
    expect(() => parseInlineDocument(value)).toThrow()
    value.blocks[7].series[0].values = [0, null, 0]
    expect(() => parseInlineDocument(value)).toThrow()
  })
  test('escapes plain text in copy without turning it into executable markup', () => {
    const doc = parseInlineDocument({
      version: 1,
      blocks: [{ type: 'paragraph', id: 'x', text: '<script> [run](javascript:evil)\n```agena-ui\n{}\n```' }],
    })
    const result = inlineDocumentMarkdown(doc)
    expect(result).not.toContain('<script>')
    expect(result).not.toContain('```agena-ui')
    expect(result).toContain('\\[run\\]')
  })
  test('uses Unicode scalar limits rather than UTF-16 length', () => {
    expect(() =>
      parseInlineDocument({ version: 1, blocks: [{ type: 'paragraph', id: 'x', text: '🌱'.repeat(6000) }] }),
    ).not.toThrow()
  })
})

import { projectLocalPart } from '../src/pages/chat/transcriptProjection'

test('preparation is an ordinary tool operation, never an automatically displayed reply', () => {
  const part = projectLocalPart(
    {
      id: '42',
      agenaKind: 'tool_call',
      agenaRole: 'assistant',
      partState: 'completed',
      agenaContent: { name: 'inline_ui.prepare' },
      agenaPresentation: {
        title: 'Prepared inline content',
        summary: 'Embed it in the answer',
        blocks: [{ type: 'text', text: 'Prepared' }],
      },
    },
    'assistant',
  )
  expect(part.kind).toBe('operation')
  expect(part.toggleable).toBe(true)
  expect(part.defaultExpanded).toBe(false)
  expect(part.copyText).not.toContain(markdown)
})
