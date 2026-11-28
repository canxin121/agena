import type { JsonObject } from '@/types/json'

function attachmentResourceKind(mime: string, filename: string): 'file' | 'image' | 'audio' | 'video' | 'pdf' {
  const mediaType = mime.trim().toLowerCase()
  if (mediaType.startsWith('image/')) return 'image'
  if (mediaType.startsWith('audio/')) return 'audio'
  if (mediaType.startsWith('video/')) return 'video'
  if (
    mediaType === 'application/pdf' ||
    String(filename || '')
      .trim()
      .toLowerCase()
      .endsWith('.pdf')
  )
    return 'pdf'
  return 'file'
}

export function resourceComposerNode(input: {
  sha256?: string
  delivery?: 'reference' | 'model_input'
  path: string
  filename: string
  mime: string
  size?: number
}): JsonObject {
  const mediaType = String(input.mime || '').trim()
  return {
    type: 'activity',
    activity: {
      id: globalThis.crypto.randomUUID(),
      ...(input.sha256 ? { provenance: { content_hash: input.sha256 } } : {}),
      payload: {
        activity_type: 'resource',
        delivery: input.delivery || 'reference',
        kind: attachmentResourceKind(mediaType, input.filename),
        reference: { reference_type: 'workspace_path', path: input.path },
        name: input.filename,
        ...(mediaType ? { media_type: mediaType } : {}),
        ...(typeof input.size === 'number' ? { size_bytes: input.size } : {}),
      },
    },
  }
}
