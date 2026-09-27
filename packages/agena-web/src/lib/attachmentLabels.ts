import type { JsonValue } from '@/types/json'

type JsonRecord = Record<string, JsonValue>

/**
 * Canonical attachment display-label vocabulary.
 *
 * The durable transcript projection, the presentation projection, the
 * optimistic composer rows, and message actions all derive attachment labels
 * through this helper so one attachment has one label everywhere.
 */
export type AttachmentLabelFields = {
  title?: unknown
  filename?: unknown
  name?: unknown
  path?: unknown
  fileId?: unknown
  mime?: unknown
}

function text(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

/** Canonical precedence: explicit title, filename-ish fields, then ids. */
export function attachmentLabel(fields: AttachmentLabelFields): string {
  return (
    text(fields.title) ||
    text(fields.filename) ||
    text(fields.name) ||
    text(fields.path) ||
    text(fields.fileId) ||
    text(fields.mime)
  )
}

/** The same vocabulary for raw attachment payload records (`file_ref`). */
export function attachmentLabelFromRecord(value: unknown): string {
  const item = (typeof value === 'object' && value !== null && !Array.isArray(value) ? value : {}) as JsonRecord
  return attachmentLabel({
    title: item.title,
    filename: item.filename,
    name: item.name,
    path: item.path,
    fileId: item.file_id,
    mime: item.mime,
  })
}

/**
 * Basename label for URL-only attachments. Empty for `data:` URLs and opaque
 * values, so callers keep their own fallback vocabulary.
 */
export function attachmentLabelFromUrl(url: unknown): string {
  const value = text(url)
  if (!value || value.startsWith('data:')) return ''
  try {
    const parsed = new URL(value, 'http://localhost')
    const last = parsed.pathname.split('/').filter(Boolean).pop()
    return last ? last.trim() : ''
  } catch {
    return ''
  }
}
