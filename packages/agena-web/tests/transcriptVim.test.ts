import { describe, expect, test } from 'bun:test'

import { resolveTranscriptVimAction } from '../src/pages/chat/transcriptVim'

describe('transcript Vim keymap parity', () => {
  test('maps TUI navigation, modes, counts, search, paging, and message jumps', () => {
    expect(resolveTranscriptVimAction({ key: 'i' })).toEqual({ type: 'insert' })
    expect(resolveTranscriptVimAction({ key: 'v' })).toEqual({ type: 'visual', mode: 'character' })
    expect(resolveTranscriptVimAction({ key: 'V', shiftKey: true })).toEqual({ type: 'visual', mode: 'line' })
    expect(resolveTranscriptVimAction({ key: 'v', ctrlKey: true })).toEqual({ type: 'visual', mode: 'block' })
    expect(resolveTranscriptVimAction({ key: '7' })).toEqual({ type: 'count', digit: 7 })
    expect(resolveTranscriptVimAction({ key: 'j' })).toEqual({ type: 'move', direction: 'down' })
    expect(resolveTranscriptVimAction({ key: 'k', ctrlKey: true })).toEqual({
      type: 'message',
      direction: 'previous',
    })
    expect(resolveTranscriptVimAction({ key: '/' })).toEqual({ type: 'search', direction: 'forward' })
    expect(resolveTranscriptVimAction({ key: 'N', shiftKey: true })).toEqual({
      type: 'search-repeat',
      reverse: true,
    })
    expect(resolveTranscriptVimAction({ key: 'd', ctrlKey: true })).toEqual({
      type: 'page',
      direction: 'down',
      half: true,
    })
  })

  test('reserves r and U in transcript navigation like the TUI', () => {
    expect(resolveTranscriptVimAction({ key: 'r' })).toBeNull()
    expect(resolveTranscriptVimAction({ key: 'U', shiftKey: true })).toBeNull()
  })

  test('maps operator-adjacent motions and find commands', () => {
    expect(resolveTranscriptVimAction({ key: 'w' })).toEqual({
      type: 'word',
      direction: 'forward',
      edge: 'start',
      big: false,
    })
    expect(resolveTranscriptVimAction({ key: '$', shiftKey: true })).toEqual({ type: 'line', edge: 'end' })
    expect(resolveTranscriptVimAction({ key: 'T', shiftKey: true })).toEqual({
      type: 'find',
      direction: 'backward',
      till: true,
    })
    expect(resolveTranscriptVimAction({ key: ';' })).toEqual({ type: 'repeat-find', reverse: false })
  })

  test('does not steal modified browser/application shortcuts', () => {
    expect(resolveTranscriptVimAction({ key: 'j', metaKey: true })).toBeNull()
    expect(resolveTranscriptVimAction({ key: 'k', altKey: true })).toBeNull()
  })
})
