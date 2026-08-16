import type { TextRange } from './transcriptTextCursor'

export type TranscriptSearchEntry = {
  key: string
  text: string
  start: number
  end: number
}

export type TranscriptSearchMatch = {
  key: string
  textStart: number
  textEnd: number
  globalStart: number
  globalEnd: number
}

function asciiCaseInsensitive(query: string): boolean {
  return /^[\x00-\x7F]*$/u.test(query)
}

export function transcriptSearchRanges(text: string, query: string): TextRange[] {
  const needle = query.trim()
  if (!needle) return []

  const ranges: TextRange[] = []
  let cursor = 0
  if (asciiCaseInsensitive(needle)) {
    const lowerText = text.toLocaleLowerCase()
    const lowerNeedle = needle.toLocaleLowerCase()
    while (cursor < lowerText.length) {
      const found = lowerText.indexOf(lowerNeedle, cursor)
      if (found < 0) break
      ranges.push({ start: found, end: found + lowerNeedle.length })
      cursor = found + Math.max(1, lowerNeedle.length)
    }
  } else {
    while (cursor < text.length) {
      const found = text.indexOf(needle, cursor)
      if (found < 0) break
      ranges.push({ start: found, end: found + needle.length })
      cursor = found + Math.max(1, needle.length)
    }
  }
  return ranges
}

export function collectTranscriptSearchMatches(
  entries: TranscriptSearchEntry[],
  query: string,
): TranscriptSearchMatch[] {
  const matches: TranscriptSearchMatch[] = []
  for (const entry of entries) {
    for (const range of transcriptSearchRanges(entry.text, query)) {
      matches.push({
        key: entry.key,
        textStart: range.start,
        textEnd: range.end,
        globalStart: entry.start + range.start,
        globalEnd: entry.start + range.end,
      })
    }
  }
  return matches.sort((left, right) => left.globalStart - right.globalStart || left.globalEnd - right.globalEnd)
}

export function nextTranscriptSearchMatchIndex(
  matches: TranscriptSearchMatch[],
  anchor: number,
  forward: boolean,
): number {
  if (!matches.length) return -1
  if (forward) {
    const next = matches.findIndex((match) => match.globalEnd > anchor)
    return next >= 0 ? next : 0
  }
  let previous = -1
  for (let index = 0; index < matches.length; index += 1) {
    if (matches[index].globalStart < anchor) previous = index
  }
  return previous >= 0 ? previous : matches.length - 1
}
