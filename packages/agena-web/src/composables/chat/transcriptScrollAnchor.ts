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
  const visible = (row: HTMLElement) => {
    const bounds = row.getBoundingClientRect()
    return bounds.bottom > top && bounds.top < top + scroller.clientHeight
  }
  const element =
    rows.find((row) => !row.querySelector('[data-transcript-key]') && visible(row)) || rows.find(visible) || null
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
