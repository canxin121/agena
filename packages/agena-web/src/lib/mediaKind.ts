// Media kind detection by file extension. Shared by markdown image/video/audio
// rendering and message attachment classification.

export type MediaKind = 'image' | 'video' | 'audio'

const IMAGE_EXTENSIONS = new Set([
  'png',
  'jpg',
  'jpeg',
  'gif',
  'svg',
  'webp',
  'ico',
  'icns',
  'bmp',
  'tiff',
  'tif',
  'avif',
  'heic',
  'heif',
  'jxl',
])

const VIDEO_EXTENSIONS = new Set(['mp4', 'webm', 'ogv', 'mov', 'm4v', 'mkv'])
const AUDIO_EXTENSIONS = new Set(['mp3', 'wav', 'ogg', 'oga', 'm4a', 'aac', 'flac', 'opus', 'weba'])

function decodeMaybe(raw: string): string {
  const input = String(raw || '')
  if (!input.includes('%')) return input
  try {
    return decodeURIComponent(input)
  } catch {
    return input
  }
}

function normalizePath(raw: string): string {
  return String(raw || '')
    .trim()
    .replace(/\\/g, '/')
}

function extFromHref(rawHref: string): string {
  if (!rawHref) return ''

  const cleaned = decodeMaybe(rawHref)
  const withoutHash = cleaned.split('#')[0] || ''
  const withoutQuery = withoutHash.split('?')[0] || ''
  const normalized = normalizePath(withoutQuery)
  const fileName = normalized.split('/').filter(Boolean).pop() || normalized
  const dot = fileName.lastIndexOf('.')
  if (dot < 0) return ''
  return fileName.slice(dot + 1).toLowerCase()
}

export function mediaKindFromHref(rawHref: string): MediaKind | null {
  const ext = extFromHref(rawHref)
  if (!ext) return null
  if (IMAGE_EXTENSIONS.has(ext)) return 'image'
  if (VIDEO_EXTENSIONS.has(ext)) return 'video'
  if (AUDIO_EXTENSIONS.has(ext)) return 'audio'
  return null
}
