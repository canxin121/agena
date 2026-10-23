import type { JsonValue, JsonObject } from '@/types/json'
import type { AttachedFile } from './useChatAttachments'

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
