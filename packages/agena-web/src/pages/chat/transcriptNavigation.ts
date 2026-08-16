import type { TranscriptDisplayPart } from '@/components/chat/messageList.types'

export type TranscriptPageDirection = 'up' | 'down'
export type TranscriptPageBoundary = 'start' | 'end' | null

export type TranscriptPageTarget = {
  top: number
  boundary: TranscriptPageBoundary
}

function visibleHeadlineText(value: unknown): string {
  return String(value || '')
    .trim()
    .replace(/\s+/gu, ' ')
}

/** Copy/navigation text must mirror the currently visible transcript. */
export function transcriptPartNavigationText(part: TranscriptDisplayPart, expanded: boolean): string {
  const full = String(part.copyText || '').trim()
  if (!part.toggleable || expanded) return full

  const title = visibleHeadlineText(part.title)
  const summary = visibleHeadlineText(part.summary)
  const preview = [title, summary && summary !== title ? summary : ''].filter(Boolean).join(' · ')
  if (preview) return preview
  return full.split(/\r?\n/u, 1)[0]?.trim() || ''
}

export function resolveTranscriptPageTarget(input: {
  scrollTop: number
  clientHeight: number
  scrollHeight: number
  direction: TranscriptPageDirection
  half: boolean
  count?: number
}): TranscriptPageTarget {
  const scrollTop = Number.isFinite(input.scrollTop) ? Math.max(0, input.scrollTop) : 0
  const clientHeight = Number.isFinite(input.clientHeight) ? Math.max(0, input.clientHeight) : 0
  const scrollHeight = Number.isFinite(input.scrollHeight) ? Math.max(0, input.scrollHeight) : 0
  const maxTop = Math.max(0, scrollHeight - clientHeight)
  const count = Number.isFinite(input.count) ? Math.max(1, Math.floor(input.count || 1)) : 1
  const distance = clientHeight * (input.half ? 0.5 : 0.9) * count
  const signedDistance = input.direction === 'down' ? distance : -distance
  const top = Math.max(0, Math.min(maxTop, scrollTop + signedDistance))
  const boundary =
    input.direction === 'down' && top >= maxTop ? 'end' : input.direction === 'up' && top <= 0 ? 'start' : null
  return { top, boundary }
}

export type TranscriptScrollBoundary = 'top' | 'bottom' | null

/**
 * Detect whether the scroll viewport has reached the top or bottom of its
 * scrollable range. Wheel scrolls re-anchor the transcript cursor to a fixed
 * screen position, so without a boundary clamp content that never passes
 * through that anchor (the very first/last line) would be unreachable; callers
 * land the cursor on the boundary line when this returns a boundary.
 */
export function transcriptScrollBoundary(input: {
  scrollTop: number
  clientHeight: number
  scrollHeight: number
}): TranscriptScrollBoundary {
  const scrollTop = Number.isFinite(input.scrollTop) ? Math.max(0, input.scrollTop) : 0
  const clientHeight = Number.isFinite(input.clientHeight) ? Math.max(0, input.clientHeight) : 0
  const scrollHeight = Number.isFinite(input.scrollHeight) ? Math.max(0, input.scrollHeight) : 0
  const maxScrollTop = Math.max(0, scrollHeight - clientHeight)
  if (scrollTop >= maxScrollTop - 1) return 'bottom'
  if (maxScrollTop > 0 && scrollTop <= 1) return 'top'
  return null
}
