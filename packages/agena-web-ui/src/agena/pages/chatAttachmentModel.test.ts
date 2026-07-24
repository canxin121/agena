import { describe, expect, test } from 'bun:test'

import {
  detectComposerAttachmentKind,
  formatComposerAttachmentSize,
  MAX_COMPOSER_ATTACHMENT_BYTES,
  validateComposerAttachment,
} from './chatAttachmentModel'

describe('chatAttachmentModel', () => {
  test('detects common attachment kinds from MIME and filename hints', () => {
    expect(detectComposerAttachmentKind('image/png', 'capture.bin')).toBe('image')
    expect(detectComposerAttachmentKind('', 'document.PDF')).toBe('pdf')
    expect(detectComposerAttachmentKind('audio/mpeg', 'voice')).toBe('audio')
    expect(detectComposerAttachmentKind('application/json', 'data.json')).toBe('file')
  })

  test('enforces empty, size, and image-only constraints', () => {
    expect(validateComposerAttachment({ name: 'empty.txt', size: 0, type: 'text/plain' })).toBe('empty.txt is empty.')
    expect(validateComposerAttachment({ name: 'huge.bin', size: MAX_COMPOSER_ATTACHMENT_BYTES + 1, type: '' })).toBe(
      'huge.bin exceeds the 50 MB attachment limit.',
    )
    expect(validateComposerAttachment({ name: 'notes.txt', size: 12, type: 'text/plain' }, true)).toBe(
      'notes.txt is not a supported image.',
    )
  })

  test('formats browser attachment sizes', () => {
    expect(formatComposerAttachmentSize(12)).toBe('12 B')
    expect(formatComposerAttachmentSize(2048)).toBe('2.0 KB')
    expect(formatComposerAttachmentSize(2 * 1024 * 1024)).toBe('2.0 MB')
  })
})
