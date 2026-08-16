export type TextRange = { start: number; end: number }
export type TextPosition = { line: number; column: number }

type Grapheme = TextRange & { text: string }
type WordClass = 'space' | 'keyword' | 'punctuation'

const wordCharacter = /[\p{L}\p{N}_]/u

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

function wordClass(value: string, bigWord: boolean): WordClass {
  if (/^\s+$/u.test(value)) return 'space'
  if (bigWord || wordCharacter.test(value)) return 'keyword'
  return 'punctuation'
}

function transcriptWordMotionTarget(
  graphemes: Grapheme[],
  current: number,
  options: { forward: boolean; toEnd: boolean; bigWord: boolean },
): number {
  const len = graphemes.length
  if (current < 0 || current >= len) return -1
  const classify = (candidate: number) => wordClass(graphemes[candidate]?.text || '', options.bigWord)

  if (options.forward && !options.toEnd) {
    // Vim fwd_word(): always move at least one grapheme, then leave the
    // current word run, cross whitespace, and land on the next word start.
    const currentClass = classify(current)
    let index = current + 1
    if (index >= len) return len - 1
    if (currentClass !== 'space') {
      while (index < len && classify(index) === currentClass) index += 1
      if (index >= len) return len - 1
    }
    while (index < len && classify(index) === 'space') index += 1
    return index < len ? index : len - 1
  }

  if (options.forward) {
    // Vim end_word(): move one grapheme first. Inside the current word, go to
    // its end; otherwise cross whitespace and go to the next word end.
    const currentClass = classify(current)
    let index = current + 1
    if (index >= len) return len - 1
    if (currentClass !== 'space' && classify(index) === currentClass) {
      while (index < len && classify(index) === currentClass) index += 1
      return Math.max(current, index - 1)
    }
    while (index < len && classify(index) === 'space') index += 1
    if (index >= len) return len - 1
    const targetClass = classify(index)
    while (index + 1 < len && classify(index + 1) === targetClass) index += 1
    return index
  }

  if (!options.toEnd) {
    // Vim bck_word(): step one grapheme backward, skip whitespace, then move
    // to the start of the preceding word run.
    if (current === 0) return -1
    let index = current - 1
    while (classify(index) === 'space') {
      if (index === 0) return 0
      index -= 1
    }
    const target = classify(index)
    while (index > 0 && classify(index - 1) === target) index -= 1
    return index
  }

  // Vim bckend_word(): step one grapheme backward, leave the current word run,
  // cross whitespace, and stop on the previous word end.
  if (current === 0) return -1
  const currentClass = classify(current)
  let index = current - 1
  if (currentClass !== 'space') {
    while (index >= 0 && classify(index) === currentClass) index -= 1
  }
  while (index >= 0 && classify(index) === 'space') index -= 1
  return index >= 0 ? index : 0
}

export function moveTranscriptWord(
  text: string,
  offset: number,
  options: { forward: boolean; toEnd: boolean; bigWord: boolean; count?: number },
): number {
  const graphemes = transcriptGraphemes(text)
  if (!graphemes.length) return 0
  let index = graphemeIndexAt(graphemes, offset)

  for (let motion = 0; motion < Math.max(1, options.count || 1); motion += 1) {
    const target = transcriptWordMotionTarget(graphemes, index, options)
    if (target < 0 || target === index) break
    index = target
  }

  return graphemes[index]?.start ?? 0
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
  if (wordClass(graphemes[index]?.text || '', bigWord) === 'space') {
    const next = graphemes.findIndex(
      (item, candidate) => candidate >= index && wordClass(item.text, bigWord) !== 'space',
    )
    if (next >= 0) index = next
  }
  const current = wordClass(graphemes[index]?.text || '', bigWord)
  let start = index
  let end = index
  while (start > 0 && wordClass(graphemes[start - 1]?.text || '', bigWord) === current) start -= 1
  while (end + 1 < graphemes.length && wordClass(graphemes[end + 1]?.text || '', bigWord) === current) end += 1
  if (around) {
    while (end + 1 < graphemes.length && wordClass(graphemes[end + 1]?.text || '', bigWord) === 'space') end += 1
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
