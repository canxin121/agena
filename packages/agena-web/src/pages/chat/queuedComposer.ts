import type { JsonValue, JsonObject } from '@/types/json'
import type { AttachedFile } from './useChatAttachments'
import type { ComposerSegment } from './composerInput'

function asRecord(value: JsonValue): JsonObject {
  return value && typeof value === 'object' && !Array.isArray(value) ? value : {}
}

export function restoredComposerText(document: JsonValue): string {
  if (!Array.isArray(document)) return ''
  return document
    .map((node) => {
      const record = asRecord(node)
      return record.type === 'text' && typeof record.text === 'string' ? record.text : ''
    })
    .join('')
}

export function restoredComposerFiles(document: JsonValue): AttachedFile[] {
  if (!Array.isArray(document)) return []
  return document.flatMap((node, index) => {
    const record = asRecord(node)
    if (record.type !== 'activity') return []
    const activity = asRecord(record.activity)
    const payload = asRecord(activity.payload)
    const reference = asRecord(payload.reference)
    const path = typeof reference.path === 'string' ? reference.path.trim() : ''
    const filename =
      [payload.name, payload.label, path.split('/').pop(), payload.text]
        .find((value) => typeof value === 'string' && value.trim())
        ?.toString()
        .slice(0, 120) || 'attachment'
    const size =
      typeof payload.size_bytes === 'number' && Number.isFinite(payload.size_bytes)
        ? Math.max(0, payload.size_bytes)
        : 0
    const id = typeof activity.id === 'string' && activity.id.trim() ? activity.id : `restored-${Date.now()}-${index}`
    return [
      {
        id,
        filename,
        size,
        mime: typeof payload.media_type === 'string' ? payload.media_type : 'application/octet-stream',
        ...(path ? { serverPath: path } : { composerNode: node }),
        delivery: payload.delivery === 'model_input' ? ('model_input' as const) : ('reference' as const),
        state: 'ready' as const,
      },
    ]
  })
}

/** Merge a successful backend take without replacing the current draft. */
export function appendQueuedComposerDocuments(
  currentSegments: ComposerSegment[],
  currentFiles: AttachedFile[],
  documents: JsonValue[],
) {
  const segments = currentSegments.map((segment) => ({ ...segment }))
  const files = [...currentFiles]
  for (const document of documents) {
    const restoredFiles = restoredComposerFiles(document)
    const ids = new Set(files.map((file) => file.id))
    files.push(...restoredFiles.filter((file) => !ids.has(file.id)))
    if (segments.some((segment) => segment.type !== 'text' || segment.text))
      segments.push({ type: 'text', text: '\n\n' })
    let fileIndex = 0
    if (Array.isArray(document))
      for (const node of document) {
        const item = asRecord(node)
        if (item.type === 'text' && typeof item.text === 'string') segments.push({ type: 'text', text: item.text })
        else if (item.type === 'activity') {
          const file = restoredFiles[fileIndex++]
          if (file) segments.push({ type: 'attachment', id: file.id })
        }
      }
  }
  const text = segments
    .filter((segment): segment is Extract<ComposerSegment, { type: 'text' }> => segment.type === 'text')
    .map((segment) => segment.text)
    .join('')
  return { segments, files, text }
}
