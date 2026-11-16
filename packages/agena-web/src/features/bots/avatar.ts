export const BOT_AVATAR_SIZE = 128
export const MAX_BOT_AVATAR_FILE_BYTES = 5 * 1024 * 1024
export const MAX_BOT_AVATAR_DATA_URL_BYTES = 256 * 1024
export const MAX_BOT_AVATAR_DIMENSION = 8192
export const MAX_BOT_AVATAR_PIXELS = 16 * 1024 * 1024
export const BOT_AVATAR_ACCEPT = 'image/png,image/jpeg,image/webp'

export type BotAvatarErrorCode = 'unsupported' | 'tooLarge' | 'dimensions' | 'read'

export class BotAvatarError extends Error {
  readonly code: BotAvatarErrorCode

  constructor(code: BotAvatarErrorCode) {
    super(code)
    this.name = 'BotAvatarError'
    this.code = code
  }
}

type ImageSize = { width: number; height: number }

export function validateBotAvatarDimensions(width: number, height: number): ImageSize {
  if (
    !Number.isSafeInteger(width) ||
    !Number.isSafeInteger(height) ||
    width < 1 ||
    height < 1 ||
    width > MAX_BOT_AVATAR_DIMENSION ||
    height > MAX_BOT_AVATAR_DIMENSION ||
    width * height > MAX_BOT_AVATAR_PIXELS
  ) {
    throw new BotAvatarError('dimensions')
  }
  return { width, height }
}

// Read dimensions before invoking an image decoder: a small compressed upload
// can otherwise allocate a very large bitmap. File MIME types are only hints.
export function botAvatarImageSize(bytes: Uint8Array): ImageSize {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  const tag = (offset: number, value: string) =>
    offset + value.length <= bytes.length &&
    Array.from(value).every((char, i) => bytes[offset + i] === char.charCodeAt(0))
  const uint24 = (offset: number) => bytes[offset]! + bytes[offset + 1]! * 256 + bytes[offset + 2]! * 65536
  if (
    bytes.length >= 24 &&
    bytes[0] === 137 &&
    tag(1, 'PNG\r\n\u001a\n') &&
    view.getUint32(8) === 13 &&
    tag(12, 'IHDR')
  ) {
    return validateBotAvatarDimensions(view.getUint32(16), view.getUint32(20))
  }
  if (bytes.length >= 4 && bytes[0] === 0xff && bytes[1] === 0xd8) {
    let offset = 2
    while (offset < bytes.length) {
      if (bytes[offset++] !== 0xff) break
      while (bytes[offset] === 0xff) offset++
      const marker = bytes[offset++]
      if (marker === undefined || marker === 0xda || marker === 0xd9) break
      if (marker === 0x01 || (marker >= 0xd0 && marker <= 0xd8)) continue
      if (offset + 2 > bytes.length) break
      const length = view.getUint16(offset)
      if (length < 2 || offset + length > bytes.length) break
      if (marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc && length >= 8) {
        return validateBotAvatarDimensions(view.getUint16(offset + 5), view.getUint16(offset + 3))
      }
      offset += length
    }
    throw new BotAvatarError('read')
  }
  if (bytes.length >= 20 && tag(0, 'RIFF') && tag(8, 'WEBP')) {
    const end = view.getUint32(4, true) + 8
    if (end > bytes.length || end < 20) throw new BotAvatarError('read')
    let size: ImageSize | undefined
    for (let offset = 12; offset + 8 <= end; ) {
      const length = view.getUint32(offset + 4, true)
      const data = offset + 8
      if (data + length > end) throw new BotAvatarError('read')
      let chunk: ImageSize | undefined
      if (tag(offset, 'VP8X') && length >= 10) {
        chunk = validateBotAvatarDimensions(uint24(data + 4) + 1, uint24(data + 7) + 1)
      } else if (tag(offset, 'VP8 ') && length >= 10 && tag(data + 3, '\u009d\u0001\u002a')) {
        chunk = validateBotAvatarDimensions(
          view.getUint16(data + 6, true) & 0x3fff,
          view.getUint16(data + 8, true) & 0x3fff,
        )
      } else if (tag(offset, 'VP8L') && length >= 5 && bytes[data] === 0x2f) {
        const bits = view.getUint32(data + 1, true)
        chunk = validateBotAvatarDimensions((bits & 0x3fff) + 1, ((bits >>> 14) & 0x3fff) + 1)
      } else if (tag(offset, 'ANMF') && length >= 16) {
        // Animated WebP frames also have their own allocation bounds.
        validateBotAvatarDimensions(uint24(data + 6) + 1, uint24(data + 9) + 1)
      }
      size ??= chunk
      offset = data + length + (length % 2)
    }
    if (size) return size
    throw new BotAvatarError('read')
  }
  throw new BotAvatarError('unsupported')
}

export function botAvatarCrop(width: number, height: number) {
  validateBotAvatarDimensions(width, height)
  const size = Math.min(width, height)
  return { x: (width - size) / 2, y: (height - size) / 2, size }
}

function checkAborted(signal?: AbortSignal) {
  if (signal?.aborted) throw new DOMException('Avatar processing cancelled', 'AbortError')
}

export async function normalizeBotAvatar(file: File, signal?: AbortSignal): Promise<string> {
  checkAborted(signal)
  if (file.size > MAX_BOT_AVATAR_FILE_BYTES) throw new BotAvatarError('tooLarge')
  if (file.type && !BOT_AVATAR_ACCEPT.split(',').includes(file.type)) throw new BotAvatarError('unsupported')
  let bitmap: ImageBitmap | undefined
  try {
    const bytes = new Uint8Array(await file.arrayBuffer())
    checkAborted(signal)
    botAvatarImageSize(bytes)
    bitmap = await createImageBitmap(file, { imageOrientation: 'from-image' })
    checkAborted(signal)
    const crop = botAvatarCrop(bitmap.width, bitmap.height)
    const canvas = document.createElement('canvas')
    canvas.width = BOT_AVATAR_SIZE
    canvas.height = BOT_AVATAR_SIZE
    const context = canvas.getContext('2d')
    if (!context) throw new BotAvatarError('read')
    context.imageSmoothingEnabled = true
    context.imageSmoothingQuality = 'high'
    context.drawImage(bitmap, crop.x, crop.y, crop.size, crop.size, 0, 0, BOT_AVATAR_SIZE, BOT_AVATAR_SIZE)
    const data = canvas.toDataURL('image/png')
    if (!botAvatarSource(data)) throw new BotAvatarError('read')
    return data
  } catch (cause) {
    if (signal?.aborted) checkAborted(signal)
    if (cause instanceof BotAvatarError) throw cause
    throw new BotAvatarError('read')
  } finally {
    bitmap?.close()
  }
}

// Persisted avatars are normalized PNG thumbnails. Never turn a profile value
// into an arbitrary remote image request.
export function botAvatarSource(value?: string | null): string | null {
  if (!value || value.length > MAX_BOT_AVATAR_DATA_URL_BYTES || value.trim() !== value) return null
  return /^data:image\/png;base64,[A-Za-z0-9+/]+={0,2}$/.test(value) ? value : null
}
