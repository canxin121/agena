import type { TranscriptTextSegment } from './transcriptDomCursor'
import type { TranscriptSearchMatch } from './transcriptSearch'

export type SearchHighlightRange = { node: Text; start: number; end: number; active: boolean }
export const MAX_TRANSCRIPT_SEARCH_HIGHLIGHT_RANGES = 2048

/** Decorative highlights are bounded independently of the complete search
 * index. Always retain the active occurrence, even outside the preview budget.
 * Sorted ranges and segments are joined in one pass instead of filtering the
 * complete text-node list for every occurrence.
 */
export function transcriptSearchHighlightRanges(
  segments: readonly TranscriptTextSegment[],
  matches: readonly TranscriptSearchMatch[],
  active: TranscriptSearchMatch | null,
  budget = MAX_TRANSCRIPT_SEARCH_HIGHLIGHT_RANGES,
): SearchHighlightRange[] {
  const selected = matches.slice(0, Math.max(0, budget))
  if (active && !selected.includes(active)) selected.push(active)
  selected.sort((left, right) => left.textStart - right.textStart)
  const ranges: SearchHighlightRange[] = []
  let segmentIndex = 0
  let ordinary = 0
  for (const match of selected) {
    const isActive = match === active
    if (!isActive && ordinary >= budget) continue
    while (segmentIndex < segments.length && segments[segmentIndex]!.end <= match.textStart) segmentIndex++
    for (let index = segmentIndex; index < segments.length; index++) {
      const segment = segments[index]!
      if (segment.start >= match.textEnd) break
      if (!isActive && ordinary >= budget) break
      const start = segment.nodeStart + Math.max(match.textStart, segment.start) - segment.start
      const end = segment.nodeStart + Math.min(match.textEnd, segment.end) - segment.start
      if (end <= start) continue
      ranges.push({ node: segment.node, start, end, active: isActive })
      if (!isActive) ordinary++
    }
  }
  return ranges
}
