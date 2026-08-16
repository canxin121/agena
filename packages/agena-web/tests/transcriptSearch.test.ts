import { describe, expect, test } from 'bun:test'

import {
  collectTranscriptSearchMatches,
  nextTranscriptSearchMatchIndex,
  transcriptSearchRanges,
  type TranscriptSearchMatch,
} from '../src/pages/chat/transcriptSearch'

describe('transcript search', () => {
  test('finds all ASCII case-insensitive matches without changing offsets', () => {
    expect(transcriptSearchRanges('aB ab', 'ab')).toEqual([
      { start: 0, end: 2 },
      { start: 3, end: 5 },
    ])
  })

  test('collects ranges across transcript entries into global offsets', () => {
    const matches = collectTranscriptSearchMatches(
      [
        { key: 'part:1', text: 'alpha', start: 0, end: 5 },
        { key: 'part:2', text: ' beta', start: 6, end: 11 },
      ],
      'a',
    )

    expect(matches).toEqual([
      { key: 'part:1', textStart: 0, textEnd: 1, globalStart: 0, globalEnd: 1 },
      { key: 'part:1', textStart: 4, textEnd: 5, globalStart: 4, globalEnd: 5 },
      { key: 'part:2', textStart: 4, textEnd: 5, globalStart: 10, globalEnd: 11 },
    ])
  })

  test('finds the next and previous match from a cursor anchor', () => {
    const matches: TranscriptSearchMatch[] = [
      { key: 'a', textStart: 0, textEnd: 2, globalStart: 0, globalEnd: 2 },
      { key: 'b', textStart: 0, textEnd: 2, globalStart: 10, globalEnd: 12 },
      { key: 'c', textStart: 0, textEnd: 2, globalStart: 20, globalEnd: 22 },
    ]

    expect(nextTranscriptSearchMatchIndex(matches, 12, true)).toBe(2)
    expect(nextTranscriptSearchMatchIndex(matches, 10, true)).toBe(1)
    expect(nextTranscriptSearchMatchIndex(matches, 10, false)).toBe(0)
    expect(nextTranscriptSearchMatchIndex(matches, 0, false)).toBe(2)
  })
})
