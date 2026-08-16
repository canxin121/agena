export type TextRange = { start: number; end: number }
export type TextPosition = { line: number; column: number }

type Grapheme = TextRange & { text: string }
type VimWordPosition = Grapheme & {
  line: number
  endOfLine: boolean
  emptyLine: boolean
}

const VIM_WHITE = 0
const VIM_PUNCTUATION = 1
const VIM_KEYWORD = 2
const VIM_EMOJI = 3
const emojiCharacter = /\p{Emoji}/u

// Mirrors Vim's utf_class_buf() interval table. Values greater than 1 are
// deliberately distinct: Vim treats transitions between Latin, Hiragana,
// Katakana, CJK, Hangul, Braille, superscript, and subscript text as word
// boundaries. See Vim src/mbyte.c and cls() in src/textobject.c.
const vimUnicodeClassIntervals: ReadonlyArray<readonly [number, number, number]> = [
  [0x037e, 0x037e, VIM_PUNCTUATION],
  [0x0387, 0x0387, VIM_PUNCTUATION],
  [0x055a, 0x055f, VIM_PUNCTUATION],
  [0x0589, 0x0589, VIM_PUNCTUATION],
  [0x05be, 0x05be, VIM_PUNCTUATION],
  [0x05c0, 0x05c0, VIM_PUNCTUATION],
  [0x05c3, 0x05c3, VIM_PUNCTUATION],
  [0x05f3, 0x05f4, VIM_PUNCTUATION],
  [0x060c, 0x060c, VIM_PUNCTUATION],
  [0x061b, 0x061b, VIM_PUNCTUATION],
  [0x061f, 0x061f, VIM_PUNCTUATION],
  [0x066a, 0x066d, VIM_PUNCTUATION],
  [0x06d4, 0x06d4, VIM_PUNCTUATION],
  [0x0700, 0x070d, VIM_PUNCTUATION],
  [0x0964, 0x0965, VIM_PUNCTUATION],
  [0x0970, 0x0970, VIM_PUNCTUATION],
  [0x0df4, 0x0df4, VIM_PUNCTUATION],
  [0x0e4f, 0x0e4f, VIM_PUNCTUATION],
  [0x0e5a, 0x0e5b, VIM_PUNCTUATION],
  [0x0f04, 0x0f12, VIM_PUNCTUATION],
  [0x0f3a, 0x0f3d, VIM_PUNCTUATION],
  [0x0f85, 0x0f85, VIM_PUNCTUATION],
  [0x104a, 0x104f, VIM_PUNCTUATION],
  [0x10fb, 0x10fb, VIM_PUNCTUATION],
  [0x1361, 0x1368, VIM_PUNCTUATION],
  [0x166d, 0x166e, VIM_PUNCTUATION],
  [0x1680, 0x1680, VIM_WHITE],
  [0x169b, 0x169c, VIM_PUNCTUATION],
  [0x16eb, 0x16ed, VIM_PUNCTUATION],
  [0x1735, 0x1736, VIM_PUNCTUATION],
  [0x17d4, 0x17dc, VIM_PUNCTUATION],
  [0x1800, 0x180a, VIM_PUNCTUATION],
  [0x2000, 0x200b, VIM_WHITE],
  [0x200c, 0x2027, VIM_PUNCTUATION],
  [0x2028, 0x2029, VIM_WHITE],
  [0x202a, 0x202e, VIM_PUNCTUATION],
  [0x202f, 0x202f, VIM_WHITE],
  [0x2030, 0x205e, VIM_PUNCTUATION],
  [0x205f, 0x205f, VIM_WHITE],
  [0x2060, 0x206f, VIM_PUNCTUATION],
  [0x2070, 0x207f, 0x2070],
  [0x2080, 0x2094, 0x2080],
  [0x20a0, 0x27ff, VIM_PUNCTUATION],
  [0x2800, 0x28ff, 0x2800],
  [0x2900, 0x2998, VIM_PUNCTUATION],
  [0x29d8, 0x29db, VIM_PUNCTUATION],
  [0x29fc, 0x29fd, VIM_PUNCTUATION],
  [0x2e00, 0x2e7f, VIM_PUNCTUATION],
  [0x3000, 0x3000, VIM_WHITE],
  [0x3001, 0x3020, VIM_PUNCTUATION],
  [0x3030, 0x3030, VIM_PUNCTUATION],
  [0x303d, 0x303d, VIM_PUNCTUATION],
  [0x3040, 0x309f, 0x3040],
  [0x30a0, 0x30ff, 0x30a0],
  [0x3300, 0x9fff, 0x4e00],
  [0xac00, 0xd7a3, 0xac00],
  [0xf900, 0xfaff, 0x4e00],
  [0xfd3e, 0xfd3f, VIM_PUNCTUATION],
  [0xfe30, 0xfe6b, VIM_PUNCTUATION],
  [0xff00, 0xff0f, VIM_PUNCTUATION],
  [0xff1a, 0xff20, VIM_PUNCTUATION],
  [0xff3b, 0xff40, VIM_PUNCTUATION],
  [0xff5b, 0xff65, VIM_PUNCTUATION],
  [0x1d000, 0x1d24f, VIM_PUNCTUATION],
  [0x1d400, 0x1d7ff, VIM_PUNCTUATION],
  [0x1f000, 0x1f2ff, VIM_PUNCTUATION],
  [0x1f300, 0x1f9ff, VIM_PUNCTUATION],
  [0x20000, 0x2a6df, 0x4e00],
  [0x2a700, 0x2b73f, 0x4e00],
  [0x2b740, 0x2b81f, 0x4e00],
  [0x2f800, 0x2fa1f, 0x4e00],
]

function vimUnicodeWordClass(codePoint: number): number {
  let lower = 0
  let upper = vimUnicodeClassIntervals.length - 1
  while (lower <= upper) {
    const middle = Math.floor((lower + upper) / 2)
    const interval = vimUnicodeClassIntervals[middle]
    if (!interval) break
    if (codePoint < interval[0]) upper = middle - 1
    else if (codePoint > interval[1]) lower = middle + 1
    else return interval[2]
  }
  return VIM_KEYWORD
}

export function transcriptGraphemes(text: string): Grapheme[] {
  if (!text) return []
  if (typeof Intl !== 'undefined' && 'Segmenter' in Intl) {
    const segmenter = new Intl.Segmenter(undefined, { granularity: 'grapheme' })
    return [...segmenter.segment(text)].map((item) => ({
      start: item.index,
      end: item.index + item.segment.length,
      text: item.segment,
    }))
  }

  const output: Grapheme[] = []
  let offset = 0
  for (const value of Array.from(text)) {
    output.push({ start: offset, end: offset + value.length, text: value })
    offset += value.length
  }
  return output
}

function graphemeIndexAt(graphemes: Grapheme[], offset: number): number {
  if (!graphemes.length) return -1
  const clamped = Math.max(0, offset)
  const containing = graphemes.findIndex((item) => clamped >= item.start && clamped < item.end)
  if (containing >= 0) return containing
  if (clamped >= graphemes.at(-1)!.end) return graphemes.length - 1
  return 0
}

export function clampTranscriptOffset(text: string, offset: number): number {
  const graphemes = transcriptGraphemes(text)
  if (!graphemes.length) return 0
  return graphemes[graphemeIndexAt(graphemes, offset)]?.start ?? 0
}

export function transcriptLineRange(text: string, offset: number): TextRange {
  const cursor = Math.max(0, Math.min(text.length, offset))
  const start = text.lastIndexOf('\n', Math.max(0, cursor - 1)) + 1
  const newline = text.indexOf('\n', cursor)
  return { start, end: newline < 0 ? text.length : newline }
}

/** Inclusive whole-line selection represented as an exclusive text range. */
export function transcriptVisualLineRange(text: string, anchor: number, head: number): TextRange {
  const first = transcriptLineRange(text, Math.min(anchor, head))
  const last = transcriptLineRange(text, Math.max(anchor, head))
  return { start: first.start, end: last.end }
}

export function transcriptLinePosition(text: string, offset: number): TextPosition {
  const cursor = clampTranscriptOffset(text, offset)
  const before = text.slice(0, cursor)
  const line = before.split('\n').length - 1
  const range = transcriptLineRange(text, cursor)
  const column = transcriptGraphemes(text.slice(range.start, cursor)).length
  return { line, column }
}

export function transcriptOffsetAtLineColumn(text: string, line: number, column: number): number {
  const lines = text.split('\n')
  const lineIndex = Math.max(0, Math.min(lines.length - 1, line))
  let start = 0
  for (let index = 0; index < lineIndex; index += 1) start += (lines[index]?.length ?? 0) + 1
  const graphemes = transcriptGraphemes(lines[lineIndex] || '')
  if (!graphemes.length) return start
  const target = graphemes[Math.max(0, Math.min(graphemes.length - 1, column))]
  return start + (target?.start ?? 0)
}

export function moveTranscriptGrapheme(text: string, offset: number, forward: boolean, count = 1): number {
  const range = transcriptLineRange(text, offset)
  const graphemes = transcriptGraphemes(text.slice(range.start, range.end))
  if (!graphemes.length) return range.start
  let index = graphemeIndexAt(graphemes, Math.max(0, offset - range.start))
  for (let step = 0; step < Math.max(1, count); step += 1) {
    const next = forward ? index + 1 : index - 1
    if (next < 0 || next >= graphemes.length) break
    index = next
  }
  return range.start + (graphemes[index]?.start ?? 0)
}

function vimWordClass(value: string, bigWord: boolean): number {
  if (value.includes('\n')) return VIM_WHITE
  const character = Array.from(value)[0]
  if (!character) return VIM_WHITE
  const codePoint = character.codePointAt(0) ?? 0
  let wordClass: number

  // Vim applies the buffer's default 'iskeyword' only to Latin-1. The
  // transcript has no filetype-local override, so this is Vim's default
  // @,48-57,_,192-255 value.
  if (codePoint < 0x100) {
    if (character === '\0' || character === ' ' || character === '\t' || codePoint === 0xa0) wordClass = VIM_WHITE
    else if (
      character === '_' ||
      (character >= '0' && character <= '9') ||
      (character >= 'A' && character <= 'Z') ||
      (character >= 'a' && character <= 'z') ||
      codePoint === 0xb5 ||
      codePoint >= 0xc0
    ) {
      wordClass = VIM_KEYWORD
    } else wordClass = VIM_PUNCTUATION
  } else if (emojiCharacter.test(character)) {
    wordClass = VIM_EMOJI
  } else {
    wordClass = vimUnicodeWordClass(codePoint)
  }

  return bigWord && wordClass !== VIM_WHITE ? VIM_PUNCTUATION : wordClass
}

function vimWordPositions(text: string): VimWordPosition[] {
  const positions: VimWordPosition[] = []
  let line = 0
  let lineHasText = false
  for (const grapheme of transcriptGraphemes(text)) {
    const newline = grapheme.text.indexOf('\n')
    if (newline >= 0) {
      const start = grapheme.start + newline
      positions.push({ start, end: grapheme.end, text: '', line, endOfLine: true, emptyLine: !lineHasText })
      line += 1
      lineHasText = false
      continue
    }
    positions.push({ ...grapheme, line, endOfLine: false, emptyLine: false })
    lineHasText = true
  }
  positions.push({ start: text.length, end: text.length, text: '', line, endOfLine: true, emptyLine: !lineHasText })
  return positions
}

function vimPositionIndexAt(positions: VimWordPosition[], offset: number): number {
  const clamped = Math.max(0, offset)
  const containing = positions.findIndex(
    (position) => !position.endOfLine && clamped >= position.start && clamped < position.end,
  )
  if (containing >= 0) return containing

  let nearest = 0
  for (let index = 0; index < positions.length; index += 1) {
    const position = positions[index]
    if (!position || position.start > clamped) break
    nearest = index
  }
  const position = positions[nearest]
  if (position?.endOfLine && !position.emptyLine) {
    const previous = positions[nearest - 1]
    if (previous && previous.line === position.line && !previous.endOfLine) return nearest - 1
  }
  return nearest
}

function adjustVimForwardTarget(positions: VimWordPosition[], index: number): number {
  const position = positions[index]
  if (!position?.endOfLine || position.emptyLine) return index
  const previous = positions[index - 1]
  return previous && previous.line === position.line && !previous.endOfLine ? index - 1 : index
}

function transcriptWordMotionTarget(
  positions: VimWordPosition[],
  current: number,
  options: { forward: boolean; toEnd: boolean; bigWord: boolean },
): number {
  if (current < 0 || current >= positions.length) return -1
  const classify = (candidate: number) =>
    positions[candidate]?.endOfLine ? VIM_WHITE : vimWordClass(positions[candidate]?.text || '', options.bigWord)
  const increment = (index: number) => (index + 1 < positions.length ? index + 1 : -1)
  const decrement = (index: number) => (index > 0 ? index - 1 : -1)

  if (options.forward && !options.toEnd) {
    // Equivalent to Vim's fwd_word(): the explicit end-of-line positions are
    // Vim's NUL class. They prevent same-class text on adjacent lines from
    // being merged, and a genuinely empty line is a motion destination.
    const startClass = classify(current)
    let index = increment(current)
    if (index < 0) return -1
    if (startClass !== VIM_WHITE) {
      while (classify(index) === startClass) {
        const next = increment(index)
        if (next < 0) return adjustVimForwardTarget(positions, index)
        index = next
      }
    }
    while (classify(index) === VIM_WHITE) {
      if (positions[index]?.endOfLine && positions[index]?.emptyLine) break
      const next = increment(index)
      if (next < 0) return adjustVimForwardTarget(positions, index)
      index = next
    }
    return adjustVimForwardTarget(positions, index)
  }

  if (options.forward) {
    // Equivalent to normal-mode end_word(..., stop = FALSE, empty = FALSE):
    // unlike w, Vim's e/E crosses empty lines to the next word end.
    const startClass = classify(current)
    let index = increment(current)
    if (index < 0) return -1
    if (startClass !== VIM_WHITE && classify(index) === startClass) {
      while (classify(index) === startClass) {
        const next = increment(index)
        if (next < 0) return adjustVimForwardTarget(positions, index)
        index = next
      }
    } else {
      while (classify(index) === VIM_WHITE) {
        const next = increment(index)
        if (next < 0) return adjustVimForwardTarget(positions, index)
        index = next
      }
      const targetClass = classify(index)
      while (classify(index) === targetClass) {
        const next = increment(index)
        if (next < 0) return adjustVimForwardTarget(positions, index)
        index = next
      }
    }
    const previous = decrement(index)
    return previous < 0 ? current : previous
  }

  if (!options.toEnd) {
    // Equivalent to bck_word(..., stop = FALSE). Empty lines, but not lines
    // containing spaces, are individual stops.
    let index = decrement(current)
    if (index < 0) return -1
    while (classify(index) === VIM_WHITE) {
      if (positions[index]?.endOfLine && positions[index]?.emptyLine) return index
      const previous = decrement(index)
      if (previous < 0) return index
      index = previous
    }
    const targetClass = classify(index)
    while (classify(index) === targetClass) {
      const previous = decrement(index)
      if (previous < 0) return index
      index = previous
    }
    return increment(index)
  }

  // Equivalent to bckend_word(): leave the current run, cross NUL/whitespace,
  // and stop on the preceding word end (or on an empty line).
  const startClass = classify(current)
  let index = decrement(current)
  if (index < 0) return -1
  if (startClass !== VIM_WHITE) {
    while (classify(index) === startClass) {
      const previous = decrement(index)
      if (previous < 0) return index
      index = previous
    }
  }
  while (classify(index) === VIM_WHITE) {
    if (positions[index]?.endOfLine && positions[index]?.emptyLine) return index
    const previous = decrement(index)
    if (previous < 0) return index
    index = previous
  }
  return index
}

export function moveTranscriptWord(
  text: string,
  offset: number,
  options: { forward: boolean; toEnd: boolean; bigWord: boolean; count?: number },
): number {
  const positions = vimWordPositions(text)
  let index = vimPositionIndexAt(positions, offset)

  for (let motion = 0; motion < Math.max(1, options.count || 1); motion += 1) {
    const target = transcriptWordMotionTarget(positions, index, options)
    if (target < 0 || target === index) break
    index = target
  }

  return positions[index]?.start ?? 0
}

export function findTranscriptCharacter(
  text: string,
  offset: number,
  target: string,
  options: { forward: boolean; till: boolean; count?: number },
): number {
  const range = transcriptLineRange(text, offset)
  const graphemes = transcriptGraphemes(text.slice(range.start, range.end))
  if (!graphemes.length) return offset
  const current = graphemeIndexAt(graphemes, offset - range.start)
  let found = -1
  let cursor = current
  for (let match = 0; match < Math.max(1, options.count || 1); match += 1) {
    const indexes = options.forward
      ? Array.from({ length: graphemes.length - cursor - 1 }, (_, index) => cursor + index + 1)
      : Array.from({ length: cursor }, (_, index) => cursor - index - 1)
    found = indexes.find((index) => graphemes[index]?.text.includes(target)) ?? -1
    if (found < 0) return offset
    cursor = found
  }
  if (options.till) {
    found += options.forward ? -1 : 1
    found = Math.max(0, Math.min(graphemes.length - 1, found))
  }
  return range.start + (graphemes[found]?.start ?? 0)
}

export function transcriptWordRange(text: string, offset: number, around: boolean, bigWord = false): TextRange {
  const graphemes = transcriptGraphemes(text)
  if (!graphemes.length) return { start: 0, end: 0 }
  let index = graphemeIndexAt(graphemes, offset)
  if (vimWordClass(graphemes[index]?.text || '', bigWord) === VIM_WHITE) {
    const next = graphemes.findIndex(
      (item, candidate) => candidate >= index && vimWordClass(item.text, bigWord) !== VIM_WHITE,
    )
    if (next >= 0) index = next
  }
  const current = vimWordClass(graphemes[index]?.text || '', bigWord)
  let start = index
  let end = index
  while (start > 0 && vimWordClass(graphemes[start - 1]?.text || '', bigWord) === current) start -= 1
  while (end + 1 < graphemes.length && vimWordClass(graphemes[end + 1]?.text || '', bigWord) === current) end += 1
  if (around) {
    while (end + 1 < graphemes.length && vimWordClass(graphemes[end + 1]?.text || '', bigWord) === VIM_WHITE) end += 1
  }
  return { start: graphemes[start]?.start ?? 0, end: graphemes[end]?.end ?? text.length }
}

export function transcriptParagraphRange(text: string, offset: number, around: boolean): TextRange {
  const cursor = Math.max(0, Math.min(text.length, offset))
  const before = text.slice(0, cursor)
  const previousBreak = Math.max(before.lastIndexOf('\n\n'), before.lastIndexOf('\r\n\r\n'))
  const start = previousBreak < 0 ? 0 : previousBreak + (around ? 0 : 2)
  const after = text.slice(cursor)
  const nextLf = after.indexOf('\n\n')
  const nextCrLf = after.indexOf('\r\n\r\n')
  const candidates = [nextLf, nextCrLf].filter((value) => value >= 0)
  const relativeEnd = candidates.length ? Math.min(...candidates) : after.length
  const delimiter = relativeEnd < after.length && around ? (after.startsWith('\r\n\r\n', relativeEnd) ? 4 : 2) : 0
  return { start, end: cursor + relativeEnd + delimiter }
}
