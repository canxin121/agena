import { describe, expect, test } from 'bun:test'
import {
  BOT_AVATAR_SIZE,
  BotAvatarError,
  MAX_BOT_AVATAR_DATA_URL_BYTES,
  MAX_BOT_AVATAR_DIMENSION,
  MAX_BOT_AVATAR_FILE_BYTES,
  MAX_BOT_AVATAR_PIXELS,
  botAvatarCrop,
  botAvatarImageSize,
  botAvatarSource,
  normalizeBotAvatar,
  validateBotAvatarDimensions,
} from '../src/features/bots/avatar'

const text = (value: string) => new TextEncoder().encode(value)
function png(width: number, height: number) {
  const bytes = new Uint8Array(33)
  bytes.set([137, 80, 78, 71, 13, 10, 26, 10])
  bytes.set(text('IHDR'), 12)
  const view = new DataView(bytes.buffer)
  view.setUint32(8, 13)
  view.setUint32(16, width)
  view.setUint32(20, height)
  return bytes
}
function jpeg(width: number, height: number) {
  const bytes = new Uint8Array([255, 216, 255, 224, 0, 4, 0, 0, 255, 194, 0, 11, 8, 0, 0, 0, 0, 1, 1, 17, 0, 255, 217])
  const view = new DataView(bytes.buffer)
  view.setUint16(13, height)
  view.setUint16(15, width)
  return bytes
}
function webp(kind: 'VP8 ' | 'VP8L' | 'VP8X', width: number, height: number) {
  const length = kind === 'VP8L' ? 5 : 10
  const bytes = new Uint8Array(20 + length + (length % 2))
  bytes.set(text('RIFF'))
  bytes.set(text('WEBP'), 8)
  bytes.set(text(kind), 12)
  const view = new DataView(bytes.buffer)
  view.setUint32(4, bytes.length - 8, true)
  view.setUint32(16, length, true)
  if (kind === 'VP8L') {
    bytes[20] = 0x2f
    view.setUint32(21, (width - 1) | ((height - 1) << 14), true)
  } else if (kind === 'VP8X') {
    for (const [offset, value] of [
      [24, width - 1],
      [27, height - 1],
    ] as const) {
      bytes[offset] = value & 255
      bytes[offset + 1] = (value >>> 8) & 255
      bytes[offset + 2] = (value >>> 16) & 255
    }
  } else {
    bytes.set([0x9d, 0x01, 0x2a], 23)
    view.setUint16(26, width, true)
    view.setUint16(28, height, true)
  }
  return bytes
}

describe('Bot avatar image input', () => {
  test('reads PNG, progressive JPEG and each WebP header before decoding', () => {
    for (const bytes of [
      png(640, 480),
      jpeg(640, 480),
      ...(['VP8 ', 'VP8L', 'VP8X'] as const).map((kind) => webp(kind, 640, 480)),
    ]) {
      expect(botAvatarImageSize(bytes)).toEqual({ width: 640, height: 480 })
      // Byte offsets from a file buffer must be respected.
      const buffer = new Uint8Array(bytes.length + 13)
      buffer.set(bytes, 7)
      expect(botAvatarImageSize(buffer.subarray(7, 7 + bytes.length))).toEqual({ width: 640, height: 480 })
    }
  })
  test('rejects oversized dimensions from every image header', () => {
    for (const bytes of [
      png(MAX_BOT_AVATAR_DIMENSION + 1, 1),
      jpeg(MAX_BOT_AVATAR_DIMENSION + 1, 1),
      webp('VP8 ', MAX_BOT_AVATAR_DIMENSION + 1, 1),
      webp('VP8L', MAX_BOT_AVATAR_DIMENSION + 1, 1),
      webp('VP8X', MAX_BOT_AVATAR_DIMENSION + 1, 1),
      png(4097, 4096),
    ]) {
      expect(() => botAvatarImageSize(bytes)).toThrow(new BotAvatarError('dimensions'))
    }
    expect(validateBotAvatarDimensions(4096, 4096)).toEqual({ width: 4096, height: 4096 })
    expect(MAX_BOT_AVATAR_PIXELS).toBe(4096 * 4096)
    for (const [width, height] of [
      [0, 1],
      [1, 0],
      [NaN, 1],
      [1.5, 2],
      [Infinity, 2],
    ]) {
      expect(() => validateBotAvatarDimensions(width!, height!)).toThrow(new BotAvatarError('dimensions'))
    }
  })
  test('rejects unsupported or malformed containers without decoding', () => {
    expect(() => botAvatarImageSize(text('<svg/>'))).toThrow(new BotAvatarError('unsupported'))
    expect(() => botAvatarImageSize(new Uint8Array([255, 216, 255, 224, 255, 255]))).toThrow(new BotAvatarError('read'))
    const truncated = webp('VP8X', 20, 20).slice(0, 25)
    expect(() => botAvatarImageSize(truncated)).toThrow(new BotAvatarError('read'))
  })
  test('normalization enforces file and pixel limits before a browser decoder is needed', async () => {
    await expect(
      normalizeBotAvatar(new File([new Uint8Array(MAX_BOT_AVATAR_FILE_BYTES + 1)], 'large.png', { type: 'image/png' })),
    ).rejects.toThrow(new BotAvatarError('tooLarge'))
    await expect(normalizeBotAvatar(new File([png(1, 1)], 'wrong.svg', { type: 'image/svg+xml' }))).rejects.toThrow(
      new BotAvatarError('unsupported'),
    )
    await expect(
      normalizeBotAvatar(new File([png(8193, 1)], 'large-pixels.png', { type: 'image/png' })),
    ).rejects.toThrow(new BotAvatarError('dimensions'))
    const abort = new AbortController()
    abort.abort()
    await expect(normalizeBotAvatar(new File([png(1, 1)], 'cancelled.png'), abort.signal)).rejects.toMatchObject({
      name: 'AbortError',
    })
  })
  test('center-crops portrait and landscape images for a 128-pixel thumbnail', () => {
    expect(BOT_AVATAR_SIZE).toBe(128)
    expect(botAvatarCrop(400, 200)).toEqual({ x: 100, y: 0, size: 200 })
    expect(botAvatarCrop(200, 401)).toEqual({ x: 0, y: 100.5, size: 200 })
    expect(botAvatarCrop(128, 128)).toEqual({ x: 0, y: 0, size: 128 })
  })
  test('display accepts only bounded persisted PNG data URLs', () => {
    const avatar = 'data:image/png;base64,AA=='
    expect(botAvatarSource(avatar)).toBe(avatar)
    for (const invalid of [
      undefined,
      null,
      '',
      'https://example.test/avatar.png',
      '/avatar.png',
      'data:image/svg+xml,<svg/>',
      'data:image/jpeg;base64,AA==',
      avatar + '\n',
      'data:image/png;base64,' + 'A'.repeat(MAX_BOT_AVATAR_DATA_URL_BYTES),
    ]) {
      expect(botAvatarSource(invalid)).toBeNull()
    }
  })
})
