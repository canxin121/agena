import { transcriptGraphemes } from './transcriptTextCursor'

export type TranscriptDomBoundary = { node: Text; offset: number }

export type TranscriptTextSegment = {
  node: Text
  nodeStart: number
  nodeEnd: number
  start: number
  end: number
}

export type TranscriptTextProjection = {
  text: string
  segments: TranscriptTextSegment[]
}

export type TranscriptVisualRow = {
  top: number
  right: number
  bottom: number
  left: number
  centerY: number
  height: number
  fragments: TranscriptVisualFragment[]
}

export type TranscriptVisualFragment = {
  node: Text
  nodeStart: number
  nodeEnd: number
  rect: DOMRect
}

type VisibleTextSlice = {
  node: Text
  nodeStart: number
  nodeEnd: number
}

type ClipBounds = {
  left: number
  right: number
  top: number
  bottom: number
}

const excludedTextSelector =
  '[data-transcript-chrome="true"], [aria-hidden="true"], input, textarea, select, option, script, style'

function clippingBounds(node: Text, root: HTMLElement): ClipBounds | 'hidden' | null {
  let bounds: ClipBounds | null = null
  let current: HTMLElement | null = node.parentElement
  while (current) {
    const style = window.getComputedStyle(current)
    if (
      style.display === 'none' ||
      style.visibility === 'hidden' ||
      style.visibility === 'collapse' ||
      style.contentVisibility === 'hidden' ||
      Number(style.opacity) === 0
    ) {
      return 'hidden'
    }
    const clipsX = style.overflowX === 'hidden' || style.overflowX === 'clip'
    const clipsY = style.overflowY === 'hidden' || style.overflowY === 'clip'
    if (clipsX || clipsY) {
      const rect = current.getBoundingClientRect()
      bounds ||= {
        left: Number.NEGATIVE_INFINITY,
        right: Number.POSITIVE_INFINITY,
        top: Number.NEGATIVE_INFINITY,
        bottom: Number.POSITIVE_INFINITY,
      }
      if (clipsX) {
        bounds.left = Math.max(bounds.left, rect.left)
        bounds.right = Math.min(bounds.right, rect.right)
      }
      if (clipsY) {
        bounds.top = Math.max(bounds.top, rect.top)
        bounds.bottom = Math.min(bounds.bottom, rect.bottom)
      }
    }
    if (current === root) break
    current = current.parentElement
  }
  return bounds
}

function rangeRects(node: Text, start = 0, end = node.data.length): DOMRect[] {
  const range = document.createRange()
  range.setStart(node, start)
  range.setEnd(node, end)
  return Array.from(range.getClientRects()).filter((rect) => rect.width > 0 && rect.height > 0)
}

function rectCenterWithinClip(rect: DOMRect, clip: ClipBounds): boolean {
  const centerX = (rect.left + rect.right) / 2
  const centerY = (rect.top + rect.bottom) / 2
  return centerX >= clip.left && centerX <= clip.right && centerY >= clip.top && centerY <= clip.bottom
}

function visibleSlicesForNode(node: Text, root: HTMLElement): VisibleTextSlice[] {
  if (!node.data || !node.parentElement || !rangeRects(node).length) return []
  const clip = clippingBounds(node, root)
  if (clip === 'hidden') return []
  if (!clip) return [{ node, nodeStart: 0, nodeEnd: node.data.length }]

  const slices: VisibleTextSlice[] = []
  let sliceStart = -1
  let sliceEnd = -1
  for (const grapheme of transcriptGraphemes(node.data)) {
    const visible = rangeRects(node, grapheme.start, grapheme.end).some((rect) => rectCenterWithinClip(rect, clip))
    if (visible) {
      if (sliceStart < 0) sliceStart = grapheme.start
      sliceEnd = grapheme.end
      continue
    }
    if (sliceStart >= 0) {
      slices.push({ node, nodeStart: sliceStart, nodeEnd: sliceEnd })
      sliceStart = -1
      sliceEnd = -1
    }
  }
  if (sliceStart >= 0) slices.push({ node, nodeStart: sliceStart, nodeEnd: sliceEnd })
  return slices
}

function transcriptSelectableTextSlices(element: HTMLElement): VisibleTextSlice[] {
  const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT)
  const slices: VisibleTextSlice[] = []
  let current = walker.nextNode()
  while (current) {
    const textNode = current as Text
    const parent = textNode.parentElement
    if (parent && !parent.closest(excludedTextSelector)) slices.push(...visibleSlicesForNode(textNode, element))
    current = walker.nextNode()
  }
  return slices
}

export function transcriptSelectableTextNodes(element: HTMLElement): Text[] {
  return [...new Set(transcriptSelectableTextSlices(element).map((slice) => slice.node))]
}

function nearestFlowContainer(node: Text, root: HTMLElement): HTMLElement {
  let current = node.parentElement
  while (current && current !== root) {
    const display = window.getComputedStyle(current).display
    if (
      display === 'block' ||
      display === 'list-item' ||
      display === 'table-row' ||
      display === 'table-cell' ||
      display === 'table-caption'
    ) {
      return current
    }
    current = current.parentElement
  }
  return root
}

function firstLayoutRect(node: Text, start = 0, end = node.data.length): DOMRect | null {
  const range = document.createRange()
  range.setStart(node, start)
  range.setEnd(node, end)
  return Array.from(range.getClientRects()).find((rect) => rect.width > 0 && rect.height > 0) || null
}

function lastLayoutRect(node: Text, start = 0, end = node.data.length): DOMRect | null {
  const range = document.createRange()
  range.setStart(node, start)
  range.setEnd(node, end)
  return (
    Array.from(range.getClientRects())
      .filter((rect) => rect.width > 0 && rect.height > 0)
      .at(-1) || null
  )
}

function hasExplicitLineBreakBetween(previous: Text, current: Text): boolean {
  const range = document.createRange()
  try {
    range.setStartAfter(previous)
    range.setEndBefore(current)
    return Boolean(range.cloneContents().querySelector('br'))
  } catch {
    return false
  }
}

function flowSeparator(previous: VisibleTextSlice, current: VisibleTextSlice, root: HTMLElement): string {
  if (hasExplicitLineBreakBetween(previous.node, current.node)) return '\n'
  if (nearestFlowContainer(previous.node, root) === nearestFlowContainer(current.node, root)) return ''
  const previousRect = lastLayoutRect(previous.node, previous.nodeStart, previous.nodeEnd)
  const currentRect = firstLayoutRect(current.node, current.nodeStart, current.nodeEnd)
  if (!previousRect || !currentRect) return '\n'
  const overlap = Math.min(previousRect.bottom, currentRect.bottom) - Math.max(previousRect.top, currentRect.top)
  return overlap >= Math.min(previousRect.height, currentRect.height) * 0.5 ? '\t' : '\n'
}

/**
 * Build the cursor model from text the browser actually laid out. Markdown
 * source markers and hidden transcript chrome never enter this coordinate
 * system, so every offset can be translated back to a real DOM caret.
 */
export function transcriptTextProjection(element: HTMLElement): TranscriptTextProjection {
  const slices = transcriptSelectableTextSlices(element)
  const segments: TranscriptTextSegment[] = []
  let text = ''
  let previous: VisibleTextSlice | null = null
  for (const slice of slices) {
    const { node, nodeStart, nodeEnd } = slice
    if (previous && previous.node !== node) text += flowSeparator(previous, slice, element)
    const start = text.length
    text += node.data.slice(nodeStart, nodeEnd)
    segments.push({ node, nodeStart, nodeEnd, start, end: text.length })
    previous = slice
  }
  return { text, segments }
}

export function transcriptDomBoundaryForOffset(
  element: HTMLElement,
  offset: number,
  direction: 'forward' | 'backward' = 'forward',
  existingProjection?: TranscriptTextProjection,
): TranscriptDomBoundary | null {
  const projection = existingProjection || transcriptTextProjection(element)
  if (!projection.segments.length) return null
  const target = Math.max(0, Math.min(projection.text.length, offset))
  const containing = projection.segments.find((segment) => target >= segment.start && target < segment.end)
  if (containing) return { node: containing.node, offset: containing.nodeStart + target - containing.start }

  const previous = [...projection.segments].reverse().find((segment) => segment.end <= target)
  const next = projection.segments.find((segment) => segment.start >= target)
  if (direction === 'backward' && previous) {
    return { node: previous.node, offset: previous.nodeEnd }
  }
  if (next) return { node: next.node, offset: next.nodeStart }
  const last = projection.segments.at(-1)!
  return { node: last.node, offset: last.nodeEnd }
}

export function transcriptProjectionOffsetForBoundary(
  element: HTMLElement,
  boundary: TranscriptDomBoundary,
  existingProjection?: TranscriptTextProjection,
): number | null {
  const projection = existingProjection || transcriptTextProjection(element)
  const segment = projection.segments.find(
    (candidate) =>
      candidate.node === boundary.node &&
      boundary.offset >= candidate.nodeStart &&
      boundary.offset <= candidate.nodeEnd,
  )
  if (!segment) return null
  return segment.start + Math.max(0, Math.min(segment.nodeEnd, boundary.offset) - segment.nodeStart)
}

function firstTextBoundary(node: Node): TranscriptDomBoundary | null {
  if (node instanceof Text) return { node, offset: 0 }
  for (const child of Array.from(node.childNodes)) {
    const boundary = firstTextBoundary(child)
    if (boundary) return boundary
  }
  return null
}

function lastTextBoundary(node: Node): TranscriptDomBoundary | null {
  if (node instanceof Text) return { node, offset: node.data.length }
  const children = Array.from(node.childNodes)
  for (let index = children.length - 1; index >= 0; index -= 1) {
    const child = children[index]
    if (!child) continue
    const boundary = lastTextBoundary(child)
    if (boundary) return boundary
  }
  return null
}

function normalizeCaretBoundary(node: Node, offset: number): TranscriptDomBoundary | null {
  if (node instanceof Text) {
    return { node, offset: Math.max(0, Math.min(node.data.length, offset)) }
  }
  const children = Array.from(node.childNodes)
  const next = children[Math.max(0, Math.min(children.length - 1, offset))]
  if (next && offset < children.length) return firstTextBoundary(next)
  const previous = children[Math.max(0, Math.min(children.length - 1, offset - 1))]
  return previous ? lastTextBoundary(previous) : firstTextBoundary(node)
}

export function transcriptCaretBoundaryAtPoint(x: number, y: number): TranscriptDomBoundary | null {
  const caretPosition = document.caretPositionFromPoint?.(x, y)
  if (caretPosition) {
    const boundary = normalizeCaretBoundary(caretPosition.offsetNode, caretPosition.offset)
    if (boundary) return boundary
  }

  const legacyDocument = document as Document & {
    caretRangeFromPoint?: (clientX: number, clientY: number) => Range | null
  }
  const range = legacyDocument.caretRangeFromPoint?.(x, y)
  return range ? normalizeCaretBoundary(range.startContainer, range.startOffset) : null
}

function sameVisualRow(row: TranscriptVisualRow, rect: DOMRect): boolean {
  const overlap = Math.min(row.bottom, rect.bottom) - Math.max(row.top, rect.top)
  return overlap >= Math.min(row.height, rect.height) * 0.5 || Math.abs(row.centerY - (rect.top + rect.bottom) / 2) <= 2
}

export function transcriptVisualRows(elements: HTMLElement[]): TranscriptVisualRow[] {
  const fragments: TranscriptVisualFragment[] = elements
    .flatMap((element) => transcriptSelectableTextSlices(element))
    .flatMap(({ node, nodeStart, nodeEnd }) => {
      const range = document.createRange()
      range.setStart(node, nodeStart)
      range.setEnd(node, nodeEnd)
      return Array.from(range.getClientRects()).map((rect) => ({ node, nodeStart, nodeEnd, rect }))
    })
    .filter((fragment) => fragment.rect.width > 0 && fragment.rect.height > 0)
    .sort((left, right) => left.rect.top - right.rect.top || left.rect.left - right.rect.left)

  const rows: TranscriptVisualRow[] = []
  for (const fragment of fragments) {
    const { rect } = fragment
    let row: TranscriptVisualRow | undefined
    for (let index = rows.length - 1; index >= 0; index -= 1) {
      const candidate = rows[index]
      if (candidate && sameVisualRow(candidate, rect)) {
        row = candidate
        break
      }
      if (candidate && candidate.bottom < rect.top - 2) break
    }
    if (!row) {
      rows.push({
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
        left: rect.left,
        centerY: (rect.top + rect.bottom) / 2,
        height: rect.height,
        fragments: [fragment],
      })
      continue
    }
    row.fragments.push(fragment)
    row.top = Math.min(row.top, rect.top)
    row.right = Math.max(row.right, rect.right)
    row.bottom = Math.max(row.bottom, rect.bottom)
    row.left = Math.min(row.left, rect.left)
    row.centerY = (row.top + row.bottom) / 2
    row.height = row.bottom - row.top
  }
  return rows
}

function horizontalDistance(rect: DOMRect, x: number): number {
  if (x < rect.left) return rect.left - x
  if (x > rect.right) return x - rect.right
  return 0
}

function pointInsideRect(rect: DOMRect, x: number, y: number): { x: number; y: number } {
  const horizontalInset = Math.min(0.75, rect.width / 4)
  const verticalInset = Math.min(0.75, rect.height / 4)
  return {
    x: Math.max(rect.left + horizontalInset, Math.min(rect.right - horizontalInset, x)),
    y: Math.max(rect.top + verticalInset, Math.min(rect.bottom - verticalInset, y)),
  }
}

function rectBelongsToRow(rect: DOMRect, row: TranscriptVisualRow): boolean {
  return sameVisualRow(row, rect)
}

function closestBoundaryInFragment(
  fragment: TranscriptVisualFragment,
  row: TranscriptVisualRow,
  x: number,
): TranscriptDomBoundary | null {
  const graphemes = transcriptGraphemes(fragment.node.data.slice(fragment.nodeStart, fragment.nodeEnd))
  let best: { boundary: TranscriptDomBoundary; distance: number } | null = null
  for (const grapheme of graphemes) {
    const start = fragment.nodeStart + grapheme.start
    const end = fragment.nodeStart + grapheme.end
    const range = document.createRange()
    range.setStart(fragment.node, start)
    range.setEnd(fragment.node, end)
    for (const rect of Array.from(range.getClientRects())) {
      if (!rect.width || !rect.height || !rectBelongsToRow(rect, row)) continue
      const distance = horizontalDistance(rect, x)
      if (!best || distance < best.distance) {
        best = { boundary: { node: fragment.node, offset: start }, distance }
      }
      if (distance === 0) return best.boundary
    }
  }
  return best?.boundary || null
}

/** Resolve a preferred viewport column to an actual character on a laid-out row. */
export function transcriptBoundaryOnVisualRow(
  row: TranscriptVisualRow,
  preferredX: number,
): TranscriptDomBoundary | null {
  const fragments = [...row.fragments].sort(
    (left, right) =>
      horizontalDistance(left.rect, preferredX) - horizontalDistance(right.rect, preferredX) ||
      left.rect.left - right.rect.left,
  )

  for (const fragment of fragments) {
    const sample = pointInsideRect(fragment.rect, preferredX, row.centerY)
    const boundary = transcriptCaretBoundaryAtPoint(sample.x, sample.y)
    if (
      boundary &&
      row.fragments.some(
        (candidate) =>
          candidate.node === boundary.node &&
          boundary.offset >= candidate.nodeStart &&
          boundary.offset <= candidate.nodeEnd,
      )
    ) {
      return boundary
    }
  }

  for (const fragment of fragments) {
    const boundary = closestBoundaryInFragment(fragment, row, preferredX)
    if (boundary) return boundary
  }
  return null
}
