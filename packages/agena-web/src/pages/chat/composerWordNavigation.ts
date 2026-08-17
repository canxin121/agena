import { transcriptGraphemes } from './transcriptTextCursor'

export type ComposerTextRange = { start: number; end: number }

const COMPOSER_WORD_CHARACTER = /[\p{L}\p{N}_]/u

function clampComposerOffset(text: string, offset: number): number {
  const graphemes = transcriptGraphemes(text)
  if (!graphemes.length) return 0
  const target = Math.max(0, Math.min(text.length, offset))
  const containing = graphemes.find((item) => target >= item.start && target < item.end)
  if (containing) return containing.start
  if (target >= (graphemes.at(-1)?.end ?? 0)) return graphemes.at(-1)?.start ?? 0
  return 0
}

/** Move over one complete grapheme, matching the TUI editor's atomic cursor. */
export function previousComposerGraphemeBoundary(text: string, cursor: number): number {
  const target = Math.max(0, Math.min(text.length, cursor))
  if (target === 0) return 0
  const graphemes = transcriptGraphemes(text)
  for (let index = graphemes.length - 1; index >= 0; index -= 1) {
    const item = graphemes[index]
    if (item && item.end <= target) return item.start
    if (item && item.start < target && target < item.end) return item.start
  }
  return 0
}

/** Move over one complete grapheme, matching the TUI editor's atomic cursor. */
export function nextComposerGraphemeBoundary(text: string, cursor: number): number {
  const target = Math.max(0, Math.min(text.length, cursor))
  if (target >= text.length) return text.length
  const graphemes = transcriptGraphemes(text)
  const item = graphemes.find(
    (candidate) => candidate.start >= target || (candidate.start < target && target < candidate.end),
  )
  return item?.end ?? text.length
}

export function composerLineStart(text: string, cursor: number): number {
  const target = Math.max(0, Math.min(text.length, cursor))
  return text.lastIndexOf('\n', Math.max(0, target - 1)) + 1
}

export function composerLineEnd(text: string, cursor: number): number {
  const target = Math.max(0, Math.min(text.length, cursor))
  const newline = text.indexOf('\n', target)
  return newline < 0 ? text.length : newline
}

function isComposerWordGrapheme(grapheme: string): boolean {
  return Array.from(grapheme).some((character) => COMPOSER_WORD_CHARACTER.test(character))
}

function previousAtomicBoundary(text: string, position: number): number {
  const graphemes = transcriptGraphemes(text)
  for (let index = graphemes.length - 1; index >= 0; index -= 1) {
    const item = graphemes[index]
    if (item && item.end <= position) return item.start
  }
  return 0
}

function nextAtomicBoundary(text: string, position: number): number {
  const graphemes = transcriptGraphemes(text)
  for (const item of graphemes) {
    if (item.start >= position) return item.end
  }
  return text.length
}

export function previousComposerWordBoundary(text: string, cursor: number): number {
  let position = clampComposerOffset(text, cursor)
  while (position > 0) {
    const start = previousAtomicBoundary(text, position)
    if (isComposerWordGrapheme(text.slice(start, position))) break
    position = start
  }
  while (position > 0) {
    const start = previousAtomicBoundary(text, position)
    if (!isComposerWordGrapheme(text.slice(start, position))) break
    position = start
  }
  return position
}

export function nextComposerWordBoundary(text: string, cursor: number): number {
  let position = clampComposerOffset(text, cursor)
  while (position < text.length) {
    const end = nextAtomicBoundary(text, position)
    if (isComposerWordGrapheme(text.slice(position, end))) break
    position = end
  }
  while (position < text.length) {
    const end = nextAtomicBoundary(text, position)
    if (!isComposerWordGrapheme(text.slice(position, end))) break
    position = end
  }
  return position
}

export function composerWordRangeBefore(text: string, cursor: number): ComposerTextRange {
  return {
    start: previousComposerWordBoundary(text, cursor),
    end: Math.max(0, Math.min(text.length, cursor)),
  }
}

export function composerWordRangeAfter(text: string, cursor: number): ComposerTextRange {
  return {
    start: Math.max(0, Math.min(text.length, cursor)),
    end: nextComposerWordBoundary(text, cursor),
  }
}
