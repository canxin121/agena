import { mcpAppResourceContents, mcpAppResourceUri, record } from './resources'

export const MAX_MCP_APP_MEDIA_BYTES = 2 * 1024 * 1024
const MAX_MEDIA_FILES = 4
const MAX_RESOURCE_LINKS = 4
const MAX_TEXT_BYTES = 16_384
const encoder = new TextEncoder()

export type McpAppMedia = { kind: 'image' | 'audio'; filename: string; blob: Blob }
type MessageMedia = Omit<McpAppMedia, 'filename'> & { extension: string }
type MessageBlock = { text: string } | MessageMedia | { uri: string }
export type McpAppMessageContent = {
  text: string
  media?: McpAppMedia[]
  resourceLinks?: { uris: string[]; blocks: MessageBlock[]; structured: string }
}

function bytesMatch(bytes: Uint8Array, signature: string, offset = 0) {
  for (let index = 0; index < signature.length; index++) {
    if (bytes[offset + index] !== signature.charCodeAt(index)) return false
  }
  return true
}

function isAdts(bytes: Uint8Array) {
  return bytes.length > 1 && bytes[0] === 0xff && (bytes[1]! & 0xf6) === 0xf0
}

const formats = new Map<string, [extension: string, matches: (bytes: Uint8Array) => boolean]>([
  ['image/png', ['png', (bytes) => bytesMatch(bytes, '\x89PNG\r\n\x1a\n')]],
  ['image/jpeg', ['jpg', (bytes) => bytesMatch(bytes, '\xff\xd8\xff')]],
  ['image/gif', ['gif', (bytes) => bytesMatch(bytes, 'GIF89a') || bytesMatch(bytes, 'GIF87a')]],
  ['image/webp', ['webp', (bytes) => bytesMatch(bytes, 'RIFF') && bytesMatch(bytes, 'WEBP', 8)]],
  ['audio/wav', ['wav', (bytes) => bytesMatch(bytes, 'RIFF') && bytesMatch(bytes, 'WAVE', 8)]],
  ['audio/flac', ['flac', (bytes) => bytesMatch(bytes, 'fLaC')]],
  [
    'audio/aiff',
    ['aiff', (bytes) => bytesMatch(bytes, 'FORM') && (bytesMatch(bytes, 'AIFF', 8) || bytesMatch(bytes, 'AIFC', 8))],
  ],
  ['audio/aac', ['aac', isAdts]],
  [
    'audio/mpeg',
    [
      'mp3',
      (bytes) =>
        bytesMatch(bytes, 'ID3') ||
        (bytes.length > 1 && bytes[0] === 0xff && (bytes[1]! & 0xe0) === 0xe0 && !isAdts(bytes)),
    ],
  ],
  ['audio/ogg', ['ogg', (bytes) => bytesMatch(bytes, 'OggS')]],
  [
    'audio/mp4',
    [
      'm4a',
      (bytes) => bytesMatch(bytes, 'ftyp', 4) && ['M4A ', 'M4B ', 'M4P '].some((brand) => bytesMatch(bytes, brand, 8)),
    ],
  ],
])

const mimeAliases = new Map([
  ['image/jpg', 'image/jpeg'],
  ['audio/wave', 'audio/wav'],
  ['audio/x-wav', 'audio/wav'],
  ['audio/x-aiff', 'audio/aiff'],
  ['audio/x-flac', 'audio/flac'],
])

function decodeMedia(
  kind: McpAppMedia['kind'],
  mime: unknown,
  data: unknown,
  availableBytes: number,
): MessageMedia | null {
  if (typeof mime !== 'string' || typeof data !== 'string' || data.length > Math.ceil(availableBytes / 3) * 4)
    return null
  const normalized = mime.trim().toLowerCase()
  const mediaType = mimeAliases.get(normalized) ?? normalized
  const format = formats.get(mediaType)
  if (!format || !mediaType.startsWith(`${kind}/`)) return null
  try {
    const decoded = atob(data)
    if (!decoded.length || decoded.length > availableBytes) return null
    const bytes = new Uint8Array(decoded.length)
    for (let index = 0; index < bytes.length; index++) bytes[index] = decoded.charCodeAt(index)
    const [extension, matches] = format
    if (!matches(bytes)) return null
    return { kind, extension, blob: new Blob([bytes], { type: mediaType }) }
  } catch {
    return null
  }
}

function resourceBlock(resource: unknown, availableBytes: number): MessageBlock | null {
  if (!record(resource) || !mcpAppResourceUri(resource.uri)) return null
  if (typeof resource.text === 'string' && resource.blob === undefined) return { text: resource.text }
  if (resource.text !== undefined || typeof resource.mimeType !== 'string') return null
  const mimeType = resource.mimeType.trim().toLowerCase()
  if (mimeType.startsWith('image/')) return decodeMedia('image', mimeType, resource.blob, availableBytes)
  if (mimeType.startsWith('audio/')) return decodeMedia('audio', mimeType, resource.blob, availableBytes)
  return null
}

function collectMessageBlocks(blocks: MessageBlock[], structured: string): McpAppMessageContent | null {
  const text: string[] = []
  const media: McpAppMedia[] = []
  const uris: string[] = []
  let mediaBytes = 0
  for (const part of blocks) {
    if ('text' in part) {
      text.push(part.text)
    } else if ('uri' in part) {
      uris.push(part.uri)
    } else {
      mediaBytes += part.blob.size
      const { kind, blob, extension } = part
      media.push({ kind, blob, filename: `mcp-app-${kind}-${media.length + 1}.${extension}` })
    }
  }
  const combined = [text.join('\n').trim(), structured].filter(Boolean).join('\n\n')
  if (
    encoder.encode(combined).byteLength > MAX_TEXT_BYTES ||
    media.length > MAX_MEDIA_FILES ||
    mediaBytes > MAX_MCP_APP_MEDIA_BYTES ||
    uris.length > MAX_RESOURCE_LINKS
  )
    return null
  return {
    text: combined,
    ...(media.length ? { media } : {}),
    ...(uris.length ? { resourceLinks: { uris, blocks, structured } } : {}),
  }
}

export function mcpAppMessageContent(value: unknown, structured = ''): McpAppMessageContent | null {
  const blocks: unknown[] = Array.isArray(value) ? value : [value]
  if (blocks.length > 16) return null
  const parts: MessageBlock[] = []
  let availableBytes = MAX_MCP_APP_MEDIA_BYTES
  for (const block of blocks) {
    if (!record(block)) return null
    let part: MessageBlock | null = null
    if (block.type === 'text' && typeof block.text === 'string') {
      part = { text: block.text }
    } else if (block.type === 'image' || block.type === 'audio') {
      part = decodeMedia(block.type, block.mimeType, block.data, availableBytes)
    } else if (block.type === 'resource') {
      part = resourceBlock(block.resource, availableBytes)
    } else if (block.type === 'resource_link' && typeof block.uri === 'string' && mcpAppResourceUri(block.uri)) {
      part = { uri: block.uri }
    }
    if (!part) return null
    if ('blob' in part) availableBytes -= part.blob.size
    parts.push(part)
  }
  return collectMessageBlocks(parts, structured)
}

export async function resolveMcpAppMessageContent(
  pending: NonNullable<McpAppMessageContent['resourceLinks']>,
  read: (uri: string, signal: AbortSignal) => Promise<unknown>,
  signal: AbortSignal,
): Promise<McpAppMessageContent | null> {
  const blocks: MessageBlock[] = []
  let availableBytes = MAX_MCP_APP_MEDIA_BYTES
  for (const block of pending.blocks) {
    signal.throwIfAborted()
    const part =
      'uri' in block
        ? resourceBlock(mcpAppResourceContents(await read(block.uri, signal), block.uri), availableBytes)
        : block
    if (!part) return null
    if ('blob' in part) availableBytes -= part.blob.size
    if (availableBytes < 0) return null
    blocks.push(part)
  }
  signal.throwIfAborted()
  return collectMessageBlocks(blocks, pending.structured)
}
