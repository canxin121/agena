import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import {
  splitInlineReferences,
  expandInlineReferences,
  mergeInlineReferences,
} from '../src/features/inlineUi/references'
import { parseInlineDocument, inlineDocumentMarkdown } from '../src/features/inlineUi/model'
import { ContentBuffer, type ContentPage } from '../src/lib/content'
import { replyTextForCopy } from '../src/features/inlineUi/copy'
const fixture = JSON.parse(
  readFileSync(new URL('../../../examples/inline-ui/references.json', import.meta.url), 'utf8'),
)
const document = parseInlineDocument(
  JSON.parse(readFileSync(new URL('../../../examples/inline-ui/reply.json', import.meta.url), 'utf8')),
)
const reference = fixture.reference as string
const marker = '```inline-ui\n' + reference + '\n```'
const refs = { [reference]: document }

describe('references embedded inside a normal answer', () => {
  for (const example of fixture.cases)
    test(example.name, () => {
      if (example.pending !== undefined)
        expect(
          splitInlineReferences(example.text, example.streaming).filter((s) => s.kind === 'pending-reference'),
        ).toHaveLength(example.pending)
      expect(splitInlineReferences(example.text, example.streaming).filter((s) => s.kind === 'reference')).toHaveLength(
        example.references,
      )
      if (example.literal) {
        expect(expandInlineReferences(example.text, refs, example.streaming)).toBe(example.text)
        expect(splitInlineReferences(example.text, example.streaming).every((s) => s.kind === 'markdown')).toBe(true)
      }
    })
  test('preserves the exact surrounding Markdown and keeps code examples literal', () => {
    const text = `**Before**\n\n${marker}\n\nAfter\n\n\`\`\`\`text\n${marker}\n\`\`\`\`\n`
    expect(expandInlineReferences(text, refs)).toBe(
      `**Before**\n\n${inlineDocumentMarkdown(document)}\n\nAfter\n\n\`\`\`\`text\n${marker}\n\`\`\`\`\n`,
    )
  })
  test('does not leak unfinished stream syntax and does not hide invalid final text', () => {
    const partial = 'Before\n\n```inline-ui\nui_123_'
    expect(expandInlineReferences(partial, refs, true)).toBe('Before\n\n')
    expect(expandInlineReferences(partial, refs, false)).toBe(partial)
    expect(expandInlineReferences(marker, {})).toContain('unavailable')
  })
  test('buffers raw text and byte cursors unchanged while retaining the authorized sidecar', () => {
    const buffer = new ContentBuffer(),
      epoch = '12345678-1234-1234-1234-123456789012'
    const text = `前文\n\n${marker}\n\n后文`,
      bytes = new TextEncoder().encode(text).byteLength
    const split = text.indexOf('abcdef') + 4
    const parts = [text.slice(0, split), text.slice(split)]
    let at = 0
    for (const [index, text] of parts.entries()) {
      at += new TextEncoder().encode(text).byteLength
      const cursor = { epoch, sequence: at }
      const page: ContentPage = {
        inline_ui: index === 1 ? refs : {},
        resource: {
          resource_id: 'content-1',
          kind: 'text',
          owner_session_id: 1,
          part_id: 2,
          state: index === 1 ? 'complete' : 'active',
          cursor: { epoch, sequence: bytes },
          committed_cursor: cursor,
          dropped_bytes: 0,
          retained_ranges: [],
          capture_error: null,
          total_bytes: bytes,
        },
        chunks: [{ cursor, captured_at_ms: 0, payload: { type: 'text', text } }],
        next_cursor: cursor,
        has_more: index === 0,
        gap: false,
      }
      expect(buffer.apply(page).missing).toBe(false)
    }
    expect(buffer.cursor?.sequence).toBe(bytes)
    expect(buffer.chunks.map((c) => (c.payload.type === 'text' ? c.payload.text : '')).join('')).toBe(text)
    expect(expandInlineReferences(text, buffer.inlineUi || {})).toContain(inlineDocumentMarkdown(document))
  })
  test('whole-answer copy inserts contents in position, excludes preparation, leaves user examples literal', async () => {
    const parts = [
      { agenaKind: 'tool_call', text: '', agenaContent: { name: 'inline_ui.prepare' } },
      { agenaKind: 'text', text: `Before\n\n${marker}\n\nAfter`, agenaInlineUi: refs },
    ]
    expect(await replyTextForCopy(parts, (p) => p.text, null, true)).toBe(
      `Before\n\n${inlineDocumentMarkdown(document)}\n\nAfter`,
    )
    expect(await replyTextForCopy(parts.slice(1), (p) => p.text, null, false)).toBe(`Before\n\n${marker}\n\nAfter`)
  })
  test('rejects invalid sidecar references and unsafe document values', () => {
    expect(Object.keys(mergeInlineReferences(null, { bad: document }) || {})).toHaveLength(0)
    expect(Object.keys(mergeInlineReferences(null, { [reference]: { version: 2 } }) || {})).toHaveLength(0)
  })
})

import { inlineMarkdownReferences } from '../src/features/inlineUi/references'
import { renderMarkdown } from '../src/lib/markdown'

test('Markdown reference links work across rich-content boundaries without changing the prose', () => {
  const text = `Read [the source][evidence].\n\n${marker}\n\n[evidence]: https://example.com/source "Evidence"\n`
  const refs = inlineMarkdownReferences(text)
  const first = splitInlineReferences(text)[0]!
  expect(first.kind).toBe('markdown')
  if (first.kind !== 'markdown') return
  expect(renderMarkdown(first.content, undefined, refs)).toContain('href="https://example.com/source"')
  expect(first.content).not.toContain('[evidence]:')
})
