import { describe, expect, test } from 'bun:test'

import { composerQueuePreview, createComposerQueueItem } from './chatQueueModel'

describe('chatQueueModel', () => {
  test('normalizes text previews and falls back to attachment counts', () => {
    expect(composerQueuePreview(createComposerQueueItem('  hello\n  world ', []))).toBe('hello world')
    expect(composerQueuePreview(createComposerQueueItem('', [{} as never, {} as never]))).toBe('2 attachment(s)')
    expect(composerQueuePreview(createComposerQueueItem('', [], [{} as never]))).toBe('1 Skill reference(s)')
  })

  test('truncates long previews without exceeding the requested length', () => {
    const preview = composerQueuePreview(createComposerQueueItem('abcdefghijk', []), 8)
    expect(preview).toBe('abcdefg…')
    expect(preview.length).toBe(8)
  })
})
