import MarkdownIt from 'markdown-it'
import { parseInlineDocument, inlineDocumentMarkdown, type InlineDocument } from './model'

export type InlineUiReferences = Record<string, InlineDocument>
export type InlineTextSegment =
  | { kind: 'markdown'; key: string; content: string }
  | { kind: 'reference'; key: string; reference: string }
  | { kind: 'pending-reference'; key: string }
export const INLINE_REFERENCE_PREFIX = '```inline-ui'
export const MAX_INLINE_REFERENCES = 32
const parser = new MarkdownIt({ html: true })
const referencePattern = /^ui_([1-9][0-9]{0,18})_[0-9a-f]{32}$/

export function validInlineReference(value: unknown): value is string {
  if (typeof value !== 'string') return false
  const match = referencePattern.exec(value)
  return !!match && BigInt(match[1]!) <= 9223372036854775807n
}

/** Only an explicitly closed, top-level `inline-ui` backtick fence embeds.
 * Quoted/listed examples and surrounding longer code fences stay literal.
 * Only this syntax is active; other spellings remain ordinary text. */
export function splitInlineReferences(content: string, streaming = false): InlineTextSegment[] {
  if (!content.includes('inline-ui')) return [{ kind: 'markdown', key: 'text:0', content }]
  const lines = content.split('\n'),
    offsets = [0]
  for (const line of lines) offsets.push(offsets[offsets.length - 1]! + line.length + 1)
  const segments: InlineTextSegment[] = []
  let previous = 0,
    count = 0
  for (const token of parser.parse(content, {})) {
    if (token.level !== 0 || !token.map) continue
    const [from, to] = token.map
    const start = offsets[from]!,
      end = Math.min(offsets[to]!, content.length)
    let next: InlineTextSegment | null = null
    if (token.type === 'fence' && token.info.trim() === 'inline-ui' && token.markup.startsWith('`')) {
      const closing = /^ {0,3}(`{3,})[ \t]*\r?$/.exec(lines[to - 1] || '')
      // Markdown also accepts open fences at EOF; an embed requires an actual
      // closing line of at least the opener's length, with no trailing text.
      const closed = to > from + 1 && !!closing && closing[1]!.length >= token.markup.length
      const reference = token.content.trim()
      if (closed && validInlineReference(reference)) next = { kind: 'reference', key: `reference:${start}`, reference }
      else if (
        !closed &&
        streaming &&
        end === content.length &&
        new TextEncoder().encode(token.content).byteLength <= 512
      )
        next = { kind: 'pending-reference', key: `pending:${start}` }
    }
    if (!next) continue
    if (start > previous)
      segments.push({ kind: 'markdown', key: `text:${previous}`, content: content.slice(previous, start) })
    segments.push(next)
    previous = end
    if (++count >= MAX_INLINE_REFERENCES) break
  }
  if (previous < content.length || !segments.length)
    segments.push({ kind: 'markdown', key: `text:${previous}`, content: content.slice(previous) })
  return segments
}

export function mergeInlineReferences(previous: InlineUiReferences | null, value: unknown): InlineUiReferences | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return previous
  const next = Object.assign(Object.create(null), previous || {}) as InlineUiReferences
  for (const [reference, document] of Object.entries(value)) {
    if (!validInlineReference(reference)) continue
    if (!Object.hasOwn(next, reference) && Object.keys(next).length >= MAX_INLINE_REFERENCES) break
    try {
      next[reference] = parseInlineDocument(document)
    } catch {
      /* invalid remote data is never rendered */
    }
  }
  return next
}

export function expandInlineReferences(
  content: string,
  refs: InlineUiReferences,
  streaming = false,
  renderDocument: (document: InlineDocument, reference: string) => string = (document) =>
    inlineDocumentMarkdown(document),
): string {
  return splitInlineReferences(content, streaming)
    .map((segment) => {
      if (segment.kind === 'markdown') return segment.content
      if (segment.kind === 'pending-reference') return ''
      const document = refs[segment.reference]
      return (
        (document
          ? renderDocument(document, segment.reference)
          : `> Inline content unavailable (${segment.reference}).`) + '\n'
      )
    })
    .join('')
}

/** Reference-style links in prose before an embed may be defined after it.
 * Pass the original Markdown context to each prose fragment, never append
 * synthetic definitions (which could leak into an unfinished code block). */
export function inlineMarkdownReferences(content: string): Record<string, { href: string; title: string }> {
  const environment: { references?: Record<string, { href: string; title: string }> } = {}
  parser.parse(content, environment)
  return environment.references || {}
}
