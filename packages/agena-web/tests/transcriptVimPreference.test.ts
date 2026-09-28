import { describe, expect, test } from 'bun:test'

import { resolveTranscriptVimEnabled } from '../src/pages/chat/transcriptVimPreference'

describe('transcript Vim preference', () => {
  test('defaults to on for desktop and off for mobile', () => {
    expect(resolveTranscriptVimEnabled(undefined, false)).toBe(true)
    expect(resolveTranscriptVimEnabled(undefined, true)).toBe(false)
  })

  test('a stored value always wins over the device default', () => {
    expect(resolveTranscriptVimEnabled(true, true)).toBe(true)
    expect(resolveTranscriptVimEnabled(false, false)).toBe(false)
  })

  test('non-boolean storage falls back to the device default', () => {
    expect(resolveTranscriptVimEnabled('true', true)).toBe(false)
    expect(resolveTranscriptVimEnabled(null, false)).toBe(true)
  })
})
