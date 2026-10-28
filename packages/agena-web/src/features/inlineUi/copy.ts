import { apiJson } from '@/lib/api'
import { readContent, type ContentPage } from '@/lib/content'
import { readContentForCopy } from '@/lib/contentCopy'
import { inlineDocumentFromPresentation, inlineDocumentMarkdown } from './model'
import { readInlineReference } from './api'
import {
  splitInlineReferences,
  expandInlineReferences,
  mergeInlineReferences,
  type InlineUiReferences,
} from './references'

type ReplyCopyPart = {
  agenaKind?: string
  type?: string
  agenaContent?: unknown
  agenaPresentation?: unknown
  agenaInlineUi?: unknown
}
const record = (value: unknown): Record<string, unknown> =>
  value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : {}

/** Snapshot each text source, then replace only explicit body references. Prepared
 * tool results are not included. The raw content/cursor API remains lossless. */
export async function replyTextForCopy<T extends ReplyCopyPart>(
  parts: readonly T[],
  plainText: (part: T) => string,
  sessionId: string | null,
  assistant: boolean,
): Promise<string> {
  const sections: string[] = []
  let documents: InlineUiReferences = Object.create(null)
  for (const part of parts) {
    const legacy = inlineDocumentFromPresentation(part.agenaPresentation)
    if (legacy) {
      sections.push(inlineDocumentMarkdown(legacy))
      continue
    }
    documents = mergeInlineReferences(documents, part.agenaInlineUi) || documents
    const content = record(part.agenaContent)
    let text = ''
    if ((part.agenaKind || part.type) === 'text' && sessionId && Array.isArray(content.resources)) {
      for (const value of content.resources) {
        const resource = record(value)
        if (resource.kind !== 'text' || typeof resource.resource_id !== 'string') continue
        const id = resource.resource_id
        const first = await apiJson<ContentPage>(
          `/api/v1/sessions/${encodeURIComponent(sessionId)}/content/${encodeURIComponent(id)}?max_bytes=1048576`,
        )
        const snapshot = first.resource.cursor
        await readContentForCopy(
          snapshot,
          async (cursor) => {
            const page = cursor.sequence === 0 ? first : await readContent(sessionId, id, cursor)
            documents = mergeInlineReferences(documents, page.inline_ui) || documents
            return page
          },
          'Some answer content is no longer available',
          'Answer exceeds the copy limit',
          async (chunk) => {
            text += chunk
          },
        )
      }
    }
    text += plainText(part)
    if (assistant) {
      const references = [
        ...new Set(splitInlineReferences(text).flatMap((s) => (s.kind === 'reference' ? [s.reference] : []))),
      ]
      for (const reference of references) {
        if (documents[reference] || !sessionId) continue
        try {
          documents[reference] = await readInlineReference(sessionId, reference)
        } catch {
          /* explicit fallback below */
        }
      }
      text = expandInlineReferences(text, documents)
    }
    if (text.trim()) sections.push(text)
  }
  return sections.join('\n\n')
}
