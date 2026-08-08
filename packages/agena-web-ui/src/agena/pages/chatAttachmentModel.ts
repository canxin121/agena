import type { AttachmentKind } from '../lib/agenaApi'

export const MAX_COMPOSER_ATTACHMENT_BYTES = 50 * 1024 * 1024
export const MAX_COMPOSER_ATTACHMENTS = 8
export const MAX_COMPOSER_ATTACHMENT_TOTAL_BYTES = 64 * 1024 * 1024

export type ComposerAttachmentDraft = {
  id: string
  name: string
  mime: string
  size: number
  kind: AttachmentKind
  path: string
}

export function detectComposerAttachmentKind(mime: string, filename: string): AttachmentKind {
  const normalizedMime = mime.trim().toLowerCase()
  const normalizedName = filename.trim().toLowerCase()
  if (normalizedMime.startsWith('image/') || /\.(png|jpe?g|gif|webp|svg|bmp)$/.test(normalizedName)) return 'image'
  if (normalizedMime.startsWith('audio/') || /\.(mp3|wav|m4a|ogg|flac)$/.test(normalizedName)) return 'audio'
  if (normalizedMime.startsWith('video/') || /\.(mp4|mov|webm|avi|mkv)$/.test(normalizedName)) return 'video'
  if (normalizedMime === 'application/pdf' || normalizedName.endsWith('.pdf')) return 'pdf'
  return 'file'
}

export function validateComposerAttachment(
  file: Pick<File, 'name' | 'size' | 'type'>,
  imageOnly = false,
): string | null {
  if (!file.size) return `${file.name || 'Attachment'} is empty.`
  if (file.size > MAX_COMPOSER_ATTACHMENT_BYTES) {
    return `${file.name} exceeds the 50 MB attachment limit.`
  }
  const kind = detectComposerAttachmentKind(file.type, file.name)
  if (imageOnly && kind !== 'image') return `${file.name} is not a supported image.`
  return null
}

export function readFileBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onerror = () => reject(reader.error || new Error(`Failed to read ${file.name}.`))
    reader.onload = () => {
      const result = typeof reader.result === 'string' ? reader.result : ''
      const separator = result.indexOf(',')
      if (separator < 0) {
        reject(new Error(`Failed to encode ${file.name}.`))
        return
      }
      resolve(result.slice(separator + 1))
    }
    reader.readAsDataURL(file)
  })
}

export async function createComposerAttachmentDraft(file: File, path: string): Promise<ComposerAttachmentDraft> {
  const mime = file.type.trim() || 'application/octet-stream'
  const kind = detectComposerAttachmentKind(mime, file.name)
  const id = typeof crypto !== 'undefined' && crypto.randomUUID ? crypto.randomUUID() : `${Date.now()}-${file.name}`
  return {
    id,
    name: file.name,
    mime,
    size: file.size,
    kind,
    path,
  }
}

export function formatComposerAttachmentSize(size: number): string {
  if (size < 1024) return `${size} B`
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`
  return `${(size / 1024 / 1024).toFixed(1)} MB`
}
