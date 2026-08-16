import { describe, expect, test } from 'bun:test'

import {
  findTranscriptCharacter,
  moveTranscriptGrapheme,
  moveTranscriptWord,
  transcriptLinePosition,
  transcriptOffsetAtLineColumn,
  transcriptParagraphRange,
  transcriptSelectionEnd,
  transcriptSelectionText,
  transcriptVisualLineRange,
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
    expect(moveTranscriptWord(text, 0, { forward: true, toEnd: false, bigWord: true })).toBe(9)
    expect(moveTranscriptWord(text, 6, { forward: true, toEnd: false, bigWord: false })).toBe(9)
    expect(moveTranscriptWord(text, 2, { forward: true, toEnd: false, bigWord: false })).toBe(3)
    expect(moveTranscriptWord(text, 2, { forward: true, toEnd: false, bigWord: false, count: 2 })).toBe(4)
  })

  test('e and ge stop at the current and previous word end instead of advancing too far', () => {
    const text = 'one two three'
    expect(moveTranscriptWord(text, 2, { forward: true, toEnd: true, bigWord: false })).toBe(6)
    expect(moveTranscriptWord(text, 2, { forward: true, toEnd: true, bigWord: false, count: 2 })).toBe(12)
    expect(moveTranscriptWord(text, 5, { forward: false, toEnd: true, bigWord: false })).toBe(2)
    expect(moveTranscriptWord(text, text.indexOf('three'), { forward: false, toEnd: true, bigWord: false })).toBe(6)
    expect(moveTranscriptWord(text, 8, { forward: true, toEnd: true, bigWord: false, count: 2 })).toBe(12)
  })

  test('e from the end of a word moves to the next word end like Vim', () => {
    const text = 'one two three'
    expect(moveTranscriptWord(text, 2, { forward: true, toEnd: true, bigWord: false })).toBe(6)
    expect(moveTranscriptWord(text, 6, { forward: true, toEnd: true, bigWord: false })).toBe(12)
  })

  test('ge from inside a word moves to the previous word end like Vim', () => {
    const text = 'one two three'
    expect(moveTranscriptWord(text, 5, { forward: false, toEnd: true, bigWord: false })).toBe(2)
    expect(moveTranscriptWord(text, 10, { forward: false, toEnd: true, bigWord: false })).toBe(6)
  })

  test('w reaches the last character of a final word and ge reaches leading whitespace', () => {
    expect(moveTranscriptWord('one foo', 4, { forward: true, toEnd: false, bigWord: false })).toBe(6)
    expect(moveTranscriptWord('   foo', 3, { forward: false, toEnd: true, bigWord: false })).toBe(0)
  })

  test('backward word starts skip whitespace and respect repeated motion', () => {
    const text = 'one   two'
    expect(moveTranscriptWord(text, text.indexOf('two') + 1, { forward: false, toEnd: false, bigWord: false })).toBe(6)
    expect(
      moveTranscriptWord(text, text.indexOf('two') + 1, { forward: false, toEnd: false, bigWord: false, count: 2 }),
    ).toBe(0)
  })

  test('word classes use Unicode punctuation and letters instead of ASCII-only rules', () => {
    const text = '中文，继续'
    expect(moveTranscriptWord(text, 0, { forward: true, toEnd: false, bigWord: false })).toBe(2)
    expect(moveTranscriptWord(text, 0, { forward: true, toEnd: true, bigWord: false })).toBe(1)
    expect(moveTranscriptWord(text, 2, { forward: true, toEnd: true, bigWord: false })).toBe(4)
    expect(moveTranscriptWord(text, text.indexOf('继续'), { forward: false, toEnd: false, bigWord: false })).toBe(2)
    expect(moveTranscriptWord(text, text.indexOf('继续'), { forward: false, toEnd: true, bigWord: false })).toBe(2)
  })

  test('does not merge equal word classes across line boundaries', () => {
    const text = 'foo\nbar'
    const bar = text.indexOf('bar')

    // Verified against Vim 9.1 fwd_word(), end_word(), bck_word(), and
    // bckend_word(): the line-ending NUL is class 0 even without visible
    // whitespace in the transcript projection.
    expect(moveTranscriptWord(text, 0, { forward: true, toEnd: false, bigWord: false })).toBe(bar)
    expect(moveTranscriptWord(text, 0, { forward: true, toEnd: true, bigWord: false })).toBe(2)
    expect(moveTranscriptWord(text, bar, { forward: false, toEnd: false, bigWord: false })).toBe(0)
    expect(moveTranscriptWord(text, bar, { forward: false, toEnd: true, bigWord: false })).toBe(2)
  })

  test('matches Vim empty-line stops and e crossing behavior', () => {
    const text = 'foo\n\nbar'
    const blankLine = 4
    const bar = text.indexOf('bar')

    expect(moveTranscriptWord(text, 0, { forward: true, toEnd: false, bigWord: false })).toBe(blankLine)
    expect(moveTranscriptWord(text, 0, { forward: true, toEnd: false, bigWord: false, count: 2 })).toBe(bar)
    expect(moveTranscriptWord(text, bar, { forward: false, toEnd: false, bigWord: false })).toBe(blankLine)
    expect(moveTranscriptWord(text, bar, { forward: false, toEnd: false, bigWord: false, count: 2 })).toBe(0)
    expect(moveTranscriptWord(text, 2, { forward: true, toEnd: true, bigWord: false })).toBe(text.length - 1)
    expect(moveTranscriptWord(text, bar, { forward: false, toEnd: true, bigWord: false })).toBe(blankLine)
    expect(moveTranscriptWord(text, bar, { forward: false, toEnd: true, bigWord: false, count: 2 })).toBe(2)
  })

  test('uses Vim script and emoji classes while WORD ignores those boundaries', () => {
    const mixedScripts = 'a中b'
    expect(moveTranscriptWord(mixedScripts, 0, { forward: true, toEnd: false, bigWord: false })).toBe(1)
    expect(moveTranscriptWord(mixedScripts, 0, { forward: true, toEnd: false, bigWord: false, count: 2 })).toBe(
      'a中'.length,
    )
    expect(moveTranscriptWord(mixedScripts, 0, { forward: true, toEnd: true, bigWord: false })).toBe(1)

    const emojiAndPunctuation = 'a🙂.b'
    expect(moveTranscriptWord(emojiAndPunctuation, 1, { forward: true, toEnd: false, bigWord: false })).toBe(3)
    expect(moveTranscriptWord(emojiAndPunctuation, 1, { forward: true, toEnd: true, bigWord: false })).toBe(3)

    const word = 'a中.b x'
    expect(moveTranscriptWord(word, 0, { forward: true, toEnd: false, bigWord: true })).toBe(word.indexOf('x'))
    expect(moveTranscriptWord(word, 0, { forward: true, toEnd: true, bigWord: true })).toBe(word.indexOf('b'))
  })

  test('uses Vim whitespace classes rather than JavaScript generic whitespace', () => {
    const emSpace = 'a\u2003b'
    expect(moveTranscriptWord(emSpace, 0, { forward: true, toEnd: false, bigWord: false })).toBe(2)

    // U+0085 satisfies JavaScript \s, but Vim classifies it as punctuation.
    const nextLineControl = 'a\u0085.b'
    expect(moveTranscriptWord(nextLineControl, 1, { forward: true, toEnd: false, bigWord: false })).toBe(3)
  })

  test("matches Vim's default Latin-1 iskeyword table", () => {
    expect(moveTranscriptWord('aª.b', 0, { forward: true, toEnd: false, bigWord: false })).toBe(1)
    expect(moveTranscriptWord('aº.b', 0, { forward: true, toEnd: false, bigWord: false })).toBe(1)
    expect(moveTranscriptWord('aµ.b', 0, { forward: true, toEnd: false, bigWord: false })).toBe(2)
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

  test('expands visual-line selections to complete lines in either direction', () => {
    const text = 'alpha one\nbeta two\ngamma three'
    const forward = transcriptVisualLineRange(text, text.indexOf('one'), text.indexOf('two'))
    expect(text.slice(forward.start, forward.end)).toBe('alpha one\nbeta two')

    const backward = transcriptVisualLineRange(text, text.indexOf('three'), text.indexOf('beta'))
    expect(text.slice(backward.start, backward.end)).toBe('beta two\ngamma three')
  })

  test('character selections include the final grapheme in either direction', () => {
    const text = '你🙂好 world'
    const anchor = 0
    // head lands at the start of 好; the grapheme at the head is included like
    // the original visual-mode yank semantics.
    expect(transcriptSelectionText(text, anchor, '你🙂'.length)).toBe('你🙂好')
    // head lands at the start of the emoji; the emoji grapheme is included.
    expect(transcriptSelectionText(text, anchor, '你'.length)).toBe('你🙂')
    // Reversed anchor/head order yields the same text.
    expect(transcriptSelectionText(text, '你🙂'.length, anchor)).toBe('你🙂好')
    // Offsets beyond the text clamp to the end.
    expect(transcriptSelectionText(text, 3, 999)).toBe(text.slice(3))
    // Selection ending at the very end of the text stays at the end.
    expect(transcriptSelectionText(text, 0, text.length)).toBe(text)
  })

  test('transcriptSelectionEnd extends to the end of the containing grapheme', () => {
    const text = 'a🙂b'
    expect(transcriptSelectionEnd(text, 0)).toBe(1)
    expect(transcriptSelectionEnd(text, 1)).toBe('a🙂'.length)
    expect(transcriptSelectionEnd(text, 2)).toBe('a🙂'.length)
    expect(transcriptSelectionEnd(text, text.length)).toBe(text.length)
  })
})
