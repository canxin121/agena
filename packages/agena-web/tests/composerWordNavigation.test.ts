import { describe, expect, test } from 'bun:test'

import {
  composerWordRangeAfter,
  composerWordRangeBefore,
  nextComposerWordBoundary,
  previousComposerWordBoundary,
} from '../src/pages/chat/composerWordNavigation'

describe('composer word navigation', () => {
  test('option/ctrl left and right use the same boundaries as the TUI editor', () => {
    const text = 'one two three'
    expect(previousComposerWordBoundary(text, text.length)).toBe(8)
    expect(previousComposerWordBoundary(text, 8)).toBe(4)
    expect(nextComposerWordBoundary(text, 0)).toBe(3)
    expect(nextComposerWordBoundary(text, 4)).toBe(7)
    expect(nextComposerWordBoundary(text, 7)).toBe(13)
  })

  test('word separators split punctuation without splitting normal words', () => {
    const text = 'one.two-three'
    expect(previousComposerWordBoundary(text, text.length)).toBe(8)
    expect(composerWordRangeBefore(text, text.length)).toEqual({ start: 8, end: text.length })
    expect(composerWordRangeAfter(text, 0)).toEqual({ start: 0, end: 3 })
  })

  test('backward and forward word deletion cover one grapheme-safe word', () => {
    const text = 'one two three'
    expect(composerWordRangeBefore(text, text.length)).toEqual({ start: 8, end: 13 })
    expect(composerWordRangeAfter(text, 0)).toEqual({ start: 0, end: 3 })
  })

  test('non-ASCII punctuation also separates composer words', () => {
    const text = '你好，世界'
    expect(previousComposerWordBoundary(text, text.length)).toBe(3)
    expect(nextComposerWordBoundary(text, 0)).toBe(2)
    expect(composerWordRangeBefore(text, text.length)).toEqual({ start: 3, end: 5 })
    expect(composerWordRangeAfter(text, 0)).toEqual({ start: 0, end: 2 })
  })
})
