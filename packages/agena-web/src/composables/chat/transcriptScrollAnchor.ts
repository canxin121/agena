export type TranscriptScrollAnchor = {
  element: HTMLElement | null
  offset: number
  top: number
  height: number
}

/** Anchor a visible content row rather than the changing height of the tail. */
export function captureTranscriptScrollAnchor(scroller: HTMLElement): TranscriptScrollAnchor {
  const top = scroller.getBoundingClientRect().top
  const rows = Array.from(scroller.querySelectorAll<HTMLElement>('[data-transcript-key]'))
  const bottom = top + scroller.clientHeight
  // Transcript siblings are vertically ordered. Read O(log n) boxes instead
  // of laying out every preceding content-visibility subtree when a tab hides.
  const firstVisible = (candidates: HTMLElement[]) => {
    let low = 0
    let high = candidates.length
    while (low < high) {
      const middle = (low + high) >>> 1
      if (candidates[middle]!.getBoundingClientRect().bottom <= top) low = middle + 1
      else high = middle
    }
    const row = candidates[low]
    return row && row.getBoundingClientRect().top < bottom ? row : null
  }
  const leaves: HTMLElement[] = []
  const parents: HTMLElement[] = []
  for (const row of rows) (row.querySelector('[data-transcript-key]') ? parents : leaves).push(row)
  const element = firstVisible(leaves) || firstVisible(parents)
  return {
    element,
    offset: element ? element.getBoundingClientRect().top - top : 0,
    top: scroller.scrollTop,
    height: scroller.scrollHeight,
  }
}

export function restoreTranscriptScrollAnchor(scroller: HTMLElement, anchor: TranscriptScrollAnchor): void {
  if (anchor.element && scroller.contains(anchor.element)) {
    const delta = anchor.element.getBoundingClientRect().top - scroller.getBoundingClientRect().top - anchor.offset
    // Account for user scrolling while the fetch was in flight.
    scroller.scrollTop += delta + scroller.scrollTop - anchor.top
  } else {
    scroller.scrollTop += scroller.scrollHeight - anchor.height
  }
}
