import type { ComposerSegment } from './composerInput'

/**
 * The composer owns its undo history.
 *
 * A contenteditable cannot lean on the browser's undo stack here. That stack
 * lives on the document, not on the editing host, so Cmd/Ctrl+Z pressed in one
 * workspace pane can revert the text a *different* pane owns; the panes share
 * one document. The app also writes into the editor programmatically (draft
 * sync, attachment chips, range replacement), which is exactly the case where
 * the built-in stack stops matching what the user sees. One history per
 * composer instance keeps undo local to the pane that asked for it.
 */

/** Consecutive typing folds into one step while it stays inside this window. */
export const COMPOSER_HISTORY_COALESCE_MS = 400
/** Bounded memory: the oldest step is dropped first. */
export const COMPOSER_HISTORY_LIMIT = 200

export type ComposerHistoryState = {
  entries: ComposerSegment[][]
  index: number
  /** Timestamp until which further typing extends the current step. */
  coalesceUntil: number
}

export type ComposerHistoryCommand = 'undo' | 'redo'

export type ComposerHistoryStep = {
  state: ComposerHistoryState
  segments: ComposerSegment[]
}

function cloneSegment(segment: ComposerSegment): ComposerSegment {
  return segment.type === 'text' ? { type: 'text', text: segment.text } : { type: 'attachment', id: segment.id }
}

export function cloneComposerSegments(segments: readonly ComposerSegment[]): ComposerSegment[] {
  return segments.map(cloneSegment)
}

/** Stable key, so an edit that left the document unchanged is not a new step. */
export function composerSegmentsKey(segments: readonly ComposerSegment[]): string {
  return segments
    .map((segment) =>
      segment.type === 'text' ? `text:${segment.text.length}:${segment.text}` : `attachment:${segment.id}`,
    )
    .join('\u0000')
}

/** A fresh history whose single entry is the current document. */
export function createComposerHistory(segments: readonly ComposerSegment[] = []): ComposerHistoryState {
  return { entries: [cloneComposerSegments(segments)], index: 0, coalesceUntil: 0 }
}

/**
 * Record the document after an edit. `coalesce` is for typing, where the
 * browser would also keep one step per burst; structural edits (attachment
 * chips, newlines, pasted text) always start a new step.
 */
export function recordComposerHistoryStep(
  state: ComposerHistoryState,
  segments: readonly ComposerSegment[],
  opts: { coalesce: boolean; at: number },
): ComposerHistoryState {
  const next = cloneComposerSegments(segments)
  const current = state.entries[state.index]
  if (current && composerSegmentsKey(current) === composerSegmentsKey(next)) return state

  const at = Number.isFinite(opts.at) ? opts.at : 0
  const canCoalesce = opts.coalesce && state.index === state.entries.length - 1 && at <= state.coalesceUntil
  if (canCoalesce) {
    const entries = state.entries.slice()
    entries[state.index] = next
    return { entries, index: state.index, coalesceUntil: at + COMPOSER_HISTORY_COALESCE_MS }
  }

  // Editing after an undo drops the redo branch, like every editor does.
  const entries = state.entries.slice(0, state.index + 1)
  entries.push(next)
  while (entries.length > COMPOSER_HISTORY_LIMIT) entries.shift()

  return {
    entries,
    index: entries.length - 1,
    coalesceUntil: opts.coalesce ? at + COMPOSER_HISTORY_COALESCE_MS : 0,
  }
}

export function undoComposerHistory(state: ComposerHistoryState): ComposerHistoryStep | null {
  if (state.index <= 0) return null
  const index = state.index - 1
  return {
    state: { entries: state.entries, index, coalesceUntil: 0 },
    segments: cloneComposerSegments(state.entries[index] || []),
  }
}

export function redoComposerHistory(state: ComposerHistoryState): ComposerHistoryStep | null {
  if (state.index >= state.entries.length - 1) return null
  const index = state.index + 1
  return {
    state: { entries: state.entries, index, coalesceUntil: 0 },
    segments: cloneComposerSegments(state.entries[index] || []),
  }
}

/** Attachment ids a snapshot wants to place in the editor. */
export function composerAttachmentIds(segments: readonly ComposerSegment[]): Set<string> {
  return new Set(segments.filter((segment) => segment.type === 'attachment').map((segment) => segment.id))
}

/**
 * The composer's props own staged attachments, not the history: a chip may only
 * come back while the draft still holds that file.  A file removed meanwhile has
 * already released its local URL, so restoring the snapshot's text without the
 * chip keeps the typed content instead of resurrecting an empty one.
 */
export function restorableComposerSegments(
  segments: readonly ComposerSegment[],
  knownAttachmentIds: ReadonlySet<string>,
): ComposerSegment[] {
  const out: ComposerSegment[] = []
  const placed = new Set<string>()
  for (const segment of segments) {
    if (segment.type === 'text') {
      out.push(cloneSegment(segment))
      continue
    }
    if (!knownAttachmentIds.has(segment.id) || placed.has(segment.id)) continue
    placed.add(segment.id)
    out.push({ type: 'attachment', id: segment.id })
  }
  return out
}

/**
 * Which history command a key event asks for. Redo is Shift+Cmd/Ctrl+Z on every
 * platform, plus Ctrl+Y where that is the platform convention.
 */
export function resolveComposerHistoryCommand(event: {
  key?: string
  metaKey?: boolean
  ctrlKey?: boolean
  altKey?: boolean
  shiftKey?: boolean
}): ComposerHistoryCommand | null {
  const key = String(event.key || '').toLowerCase()
  if (!event.metaKey && !event.ctrlKey) return null
  if (event.altKey) return null

  if (key === 'z') return event.shiftKey ? 'redo' : 'undo'
  if (key === 'y' && event.ctrlKey && !event.metaKey && !event.shiftKey) return 'redo'
  return null
}
