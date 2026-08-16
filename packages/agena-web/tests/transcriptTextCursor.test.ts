import { describe, expect, test } from 'bun:test'

import {
  findTranscriptCharacter,
  moveTranscriptGrapheme,
  moveTranscriptWord,
  transcriptLinePosition,
  transcriptOffsetAtLineColumn,
  transcriptParagraphRange,
  transcriptWordRange,
} from '../src/pages/chat/transcriptTextCursor'

describe('transcript text cursor', () => {
  test('moves by Unicode grapheme without splitting emoji or crossing a line', () => {
    const text = '你🙂好\nnext'
    expect(moveTranscriptGrapheme(text, 0, true, 2)).toBe('你🙂'.length)
    expect(moveTranscriptGrapheme(text, '你🙂好'.length - 1, true, 2)).toBe('你🙂'.length)
  })

  test('preserves logical columns across transcript lines', () => {
    const text = 'alpha beta\n短行\nthird line'
    expect(transcriptLinePosition(text, 8)).toEqual({ line: 0, column: 8 })
    expect(transcriptOffsetAtLineColumn(text, 2, 8)).toBe(text.indexOf('third line') + 8)
    expect(transcriptOffsetAtLineColumn(text, 1, 8)).toBe(text.indexOf('短行') + 1)
  })

  test('implements Vim word starts, ends, backward ends, and WORDs', () => {
    const text = 'one.two  three-four'
    expect(moveTranscriptWord(text, 0, { forward: true, toEnd: false, bigWord: false })).toBe(3)
    expect(moveTranscriptWord(text, 3, { forward: true, toEnd: false, bigWord: false })).toBe(4)
    expect(moveTranscriptWord(text, 4, { forward: true, toEnd: true, bigWord: false })).toBe(6)
    expect(moveTranscriptWord(text, text.indexOf('three'), { forward: false, toEnd: true, bigWord: false })).toBe(6)
    expect(moveTranscriptWord(text, 0, { forward: true, toEnd: true, bigWord: true })).toBe(6)
  })

  test('find respects direction, count, and till offsets on the current line', () => {
    const text = 'a-b-c-d\nother'
    expect(findTranscriptCharacter(text, 0, '-', { forward: true, till: false, count: 2 })).toBe(3)
    expect(findTranscriptCharacter(text, 0, 'd', { forward: true, till: true })).toBe(5)
    expect(findTranscriptCharacter(text, 7, '-', { forward: false, till: true })).toBe(6)
  })

  test('selects inner/around word and paragraph text objects', () => {
    const text = 'alpha beta  gamma\n\nsecond paragraph\n\nthird'
    const innerWord = transcriptWordRange(text, text.indexOf('beta') + 1, false)
    expect(text.slice(innerWord.start, innerWord.end)).toBe('beta')
    const aroundWord = transcriptWordRange(text, text.indexOf('beta') + 1, true)
    expect(text.slice(aroundWord.start, aroundWord.end)).toBe('beta  ')
    const paragraph = transcriptParagraphRange(text, text.indexOf('second') + 2, false)
    expect(text.slice(paragraph.start, paragraph.end)).toBe('second paragraph')
  })
})
