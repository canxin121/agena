import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch, type ComputedRef, type Ref } from 'vue'

import type { RenderBlock, TranscriptDisplayPart } from '@/components/chat/messageList.types'
import { copyTextToClipboard } from '@/lib/clipboard'
import { resolveTranscriptPageTarget, transcriptScrollBoundary } from './transcriptNavigation'
import { resolveTranscriptVimAction, type TranscriptVimAction, type TranscriptVimMode } from './transcriptVim'
import {
  collectTranscriptSearchMatches,
  nextTranscriptSearchMatchIndex,
  transcriptSearchRanges,
  type TranscriptSearchMatch,
} from './transcriptSearch'
import {
  clampTranscriptOffset,
  findTranscriptCharacter,
  moveTranscriptGrapheme,
  moveTranscriptWord,
  transcriptGraphemes,
  transcriptLinePosition,
  transcriptLineRange,
  transcriptOffsetAtLineColumn,
  transcriptParagraphRange,
  transcriptSelectionEnd,
  transcriptSelectionText,
  transcriptVisualLineRange,
  transcriptWordRange,
} from './transcriptTextCursor'
import {
  transcriptBoundaryOnVisualRow,
  transcriptCaretBoundaryAtPoint,
  transcriptDomBoundaryForOffset,
  transcriptProjectionOffsetForBoundary,
  transcriptTextProjection,
  transcriptVisualRows,
  type TranscriptDomBoundary,
  type TranscriptTextProjection,
  type TranscriptVisualRow,
} from './transcriptDomCursor'

type ComposerExpose = {
  textareaEl?: HTMLTextAreaElement | { value: HTMLTextAreaElement | null } | null
}

type ToastsLike = { push: (kind: 'success' | 'error' | 'info', message: string, duration?: number) => void }

type PendingFind = { direction: 'forward' | 'backward'; till: boolean; count: number }
type LastFind = PendingFind & { target: string }
type CursorPoint = { key: string; offset: number }
type TextEntry = CursorPoint & {
  element: HTMLElement
  projection: TranscriptTextProjection
  text: string
  start: number
  end: number
}
type TextModel = { entries: TextEntry[]; text: string }

const NODE_SELECTOR = '[data-transcript-node][data-transcript-key]'
const MESSAGE_SELECTOR = '[data-transcript-node="message"][data-transcript-key]'
const VISUAL_BLOCK_HIGHLIGHT = 'agena-vim-block'

type CssHighlightRegistry = {
  set: (name: string, highlight: unknown) => void
  delete: (name: string) => boolean
}

type HighlightConstructor = new (...ranges: Range[]) => unknown

function composerTextarea(composer: ComposerExpose | null): HTMLTextAreaElement | null {
  const value = composer?.textareaEl
  if (!value) return null
  return value instanceof HTMLTextAreaElement ? value : value.value
}

function isEditable(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target.isContentEditable
}

function cssEscape(value: string): string {
  if (typeof CSS !== 'undefined' && typeof CSS.escape === 'function') return CSS.escape(value)
  return value.replace(/["\\]/g, '\\$&')
}

export function useChatTranscriptVim(opts: {
  enabled?: Readonly<Ref<boolean>>
  pageRef: Ref<HTMLElement | null>
  scrollEl: Ref<HTMLElement | null>
  composerRef: Ref<ComposerExpose | null>
  searchInputRef: Ref<HTMLInputElement | null>
  selectedSessionId: ComputedRef<string | null>
  renderBlocks: ComputedRef<RenderBlock[]>
  draft: Ref<string>
  clearComposer: () => void
  canAbort: ComputedRef<boolean>
  abortRun: () => void | Promise<void>
  toggleHelp: () => void
  openPlan: () => void
  togglePart: (part: TranscriptDisplayPart, expanded: boolean) => void
  isPartExpanded: (part: TranscriptDisplayPart) => boolean
  toasts: ToastsLike
}) {
  const mode = ref<TranscriptVimMode>('NAVIGATE')
  const activeNodeKey = ref('')
  const visualAnchorKey = ref('')
  const cursorOffset = ref(0)
  const preferredViewportX = ref<number | null>(null)
  const visualAnchorOffset = ref(0)
  const countPrefix = ref('')
  const pendingCommand = ref('')
  const pendingFind = ref<PendingFind | null>(null)
  const lastFind = ref<LastFind | null>(null)
  const lastVisualRange = ref<{ anchor: CursorPoint; head: CursorPoint; mode: TranscriptVimMode } | null>(null)
  const searchOpen = ref(false)
  const searchQuery = ref('')
  const searchForward = ref(true)
  const searchMatches = ref<TranscriptSearchMatch[]>([])
  const searchMatchIndex = ref(-1)
  const jumpHistory = ref<CursorPoint[]>([])
  const jumpHistoryIndex = ref(0)
  const commandEcho = ref('')
  let placementFrame = 0
  let scrollFollowFrame = 0
  let scrollSuppressionFrame = 0
  let scrollFollowSuppressed = false
  let cursorScreenAnchor: { x: number; y: number } | null = null
  let mouseSelecting = false
  let mouseSelection: { anchor: CursorPoint; head: CursorPoint } | null = null
  let suppressMouseClickUntil = 0
  let mountedRoot: HTMLElement | null = null
  let mountedScroll: HTMLElement | null = null
  let transcriptResizeObserver: ResizeObserver | null = null
  let cachedTextModel: TextModel | null = null

  function invalidateTextModel() {
    cachedTextModel = null
  }

  function suppressScrollFollow() {
    scrollFollowSuppressed = true
    if (scrollSuppressionFrame) window.cancelAnimationFrame(scrollSuppressionFrame)
    scrollSuppressionFrame = window.requestAnimationFrame(() => {
      scrollSuppressionFrame = window.requestAnimationFrame(() => {
        scrollSuppressionFrame = 0
        scrollFollowSuppressed = false
        scheduleCursorPlacement()
      })
    })
  }

  function scheduleCursorPlacement() {
    if (placementFrame) window.cancelAnimationFrame(placementFrame)
    placementFrame = window.requestAnimationFrame(() => {
      placementFrame = 0
      syncNativeSelection()
    })
  }

  function nodeElements(): HTMLElement[] {
    const root = opts.pageRef.value
    if (!root) return []
    return Array.from(root.querySelectorAll<HTMLElement>(NODE_SELECTOR)).filter(
      (element) => element.offsetParent !== null,
    )
  }

  function messageElements(): HTMLElement[] {
    const root = opts.pageRef.value
    if (!root) return []
    return Array.from(root.querySelectorAll<HTMLElement>(MESSAGE_SELECTOR)).filter(
      (element) => element.offsetParent !== null,
    )
  }

  function cursorElements(): HTMLElement[] {
    const nodes = nodeElements()
    const parts = nodes.filter((element) => element.dataset.transcriptNode === 'part')
    if (parts.length) return parts
    return nodes.filter((element) => element.dataset.transcriptNode === 'message')
  }

  function keyForElement(element: HTMLElement | null | undefined): string {
    return String(element?.dataset.transcriptKey || '').trim()
  }

  function elementForKey(key: string): HTMLElement | null {
    const root = opts.pageRef.value
    if (!root || !key) return null
    return root.querySelector<HTMLElement>(`[data-transcript-key="${cssEscape(key)}"]`)
  }

  function activeElement(): HTMLElement | null {
    return elementForKey(activeNodeKey.value)
  }

  function textEntries(): TextModel {
    if (cachedTextModel) return cachedTextModel
    const entries: TextEntry[] = []
    let combined = ''
    for (const element of cursorElements()) {
      const key = keyForElement(element)
      const projection = transcriptTextProjection(element)
      const value = projection.text
      if (!key || !value) continue
      if (combined) combined += '\n'
      const start = combined.length
      combined += value
      entries.push({ key, offset: 0, element, projection, text: value, start, end: combined.length })
    }
    cachedTextModel = { entries, text: combined }
    return cachedTextModel
  }

  function activeTextEntry(preferTail = false): TextEntry | null {
    const model = textEntries()
    let entry = model.entries.find((candidate) => candidate.key === activeNodeKey.value)
    if (!entry) {
      const messageId = activeElement()?.dataset.messageId || ''
      const matching = model.entries.filter((candidate) => candidate.element.dataset.messageId === messageId)
      entry = preferTail ? matching.at(-1) : matching[0]
    }
    return entry || (preferTail ? model.entries.at(-1) : model.entries[0]) || null
  }

  function cursorPoint(): CursorPoint | null {
    const entry = activeTextEntry()
    if (!entry) return null
    return { key: entry.key, offset: clampTranscriptOffset(entry.text, cursorOffset.value) }
  }

  function globalOffsetForPoint(point: CursorPoint, model = textEntries()): number | null {
    const entry = model.entries.find((candidate) => candidate.key === point.key)
    if (!entry) return null
    return entry.start + clampTranscriptOffset(entry.text, point.offset)
  }

  function pointForGlobalOffset(
    offset: number,
    direction: 'forward' | 'backward' = 'forward',
    model = textEntries(),
  ): CursorPoint | null {
    if (!model.entries.length) return null
    const target = Math.max(0, Math.min(model.text.length, offset))
    let entry = model.entries.find((candidate) => target >= candidate.start && target < candidate.end)
    if (!entry) {
      entry =
        direction === 'backward'
          ? [...model.entries].reverse().find((candidate) => candidate.end <= target) || model.entries[0]
          : model.entries.find((candidate) => candidate.start >= target) || model.entries.at(-1)
    }
    if (!entry) return null
    return { key: entry.key, offset: clampTranscriptOffset(entry.text, target - entry.start) }
  }

  function setCursorPoint(point: CursorPoint | null, options?: { center?: boolean; preserveX?: boolean }) {
    if (!point) return
    const entry = textEntries().entries.find((candidate) => candidate.key === point.key)
    if (!entry) return
    activeNodeKey.value = point.key
    cursorOffset.value = clampTranscriptOffset(entry.text, point.offset)
    suppressScrollFollow()
    entry.element.focus({ preventScroll: true })
    revealCursorPoint({ key: entry.key, offset: cursorOffset.value }, Boolean(options?.center))
    syncNativeSelection({ updatePreferredX: !options?.preserveX })
  }

  function ensureActive(preferTail = true): HTMLElement | null {
    const existing = activeElement()
    if (existing) return existing
    const nodes = cursorElements()
    const selected = preferTail ? nodes.at(-1) : nodes[0]
    if (!selected) {
      activeNodeKey.value = ''
      return null
    }
    activeNodeKey.value = keyForElement(selected)
    const text = transcriptTextProjection(selected).text
    cursorOffset.value = preferTail && text ? transcriptLineRange(text, text.length).start : 0
    preferredViewportX.value = null
    return selected
  }

  function pushJumpMark() {
    const point = cursorPoint()
    if (!point) return
    const current = jumpHistory.value.slice(0, jumpHistoryIndex.value)
    const previous = current.at(-1)
    if (!previous || previous.key !== point.key || previous.offset !== point.offset) current.push(point)
    jumpHistory.value = current.slice(-80)
    jumpHistoryIndex.value = jumpHistory.value.length
  }

  function selectElement(
    element: HTMLElement | null,
    options?: { center?: boolean; recordJump?: boolean; offset?: number; preserveX?: boolean },
  ) {
    if (!element) return
    const key = keyForElement(element)
    if (!key) return
    if (options?.recordJump) pushJumpMark()
    const changed = activeNodeKey.value !== key
    activeNodeKey.value = key
    const text = transcriptTextProjection(element).text
    if (typeof options?.offset === 'number') cursorOffset.value = clampTranscriptOffset(text, options.offset)
    else if (changed) cursorOffset.value = 0
    suppressScrollFollow()
    element.focus({ preventScroll: true })
    if (textEntries().entries.some((entry) => entry.key === key)) {
      revealCursorPoint({ key, offset: cursorOffset.value }, Boolean(options?.center))
    } else {
      element.scrollIntoView({ behavior: 'auto', block: options?.center ? 'center' : 'nearest', inline: 'nearest' })
    }
    if (mode.value.startsWith('VISUAL') && !visualAnchorKey.value) visualAnchorKey.value = key
    syncNativeSelection({ updatePreferredX: !options?.preserveX })
  }

  function selectNode(key: string) {
    if (!key) return
    const changed = activeNodeKey.value !== key
    activeNodeKey.value = key
    if (changed) {
      cursorOffset.value = 0
      preferredViewportX.value = null
    }
    if (mode.value.startsWith('VISUAL') && !visualAnchorKey.value) {
      visualAnchorKey.value = key
      visualAnchorOffset.value = cursorOffset.value
    }
    syncNativeSelection()
  }

  function nodeIndex(key: string, nodes = nodeElements()): number {
    return nodes.findIndex((element) => keyForElement(element) === key)
  }

  function pointForDomBoundary(boundary: TranscriptDomBoundary, model = textEntries()): CursorPoint | null {
    const owner = boundary.node.parentElement?.closest<HTMLElement>(NODE_SELECTOR)
    let entry = owner ? model.entries.find((candidate) => candidate.key === keyForElement(owner)) : undefined
    if (!entry) entry = model.entries.find((candidate) => candidate.element.contains(boundary.node))
    if (!entry) return null

    let offset = transcriptProjectionOffsetForBoundary(entry.element, boundary, entry.projection)
    if (offset === null) return null
    // A browser caret at the end of a text node belongs to the character on
    // its left in Normal mode. This also keeps synthetic block separators out
    // of the set of cursor destinations.
    const segment = entry.projection.segments.find(
      (candidate) =>
        candidate.node === boundary.node &&
        boundary.offset >= candidate.nodeStart &&
        boundary.offset <= candidate.nodeEnd,
    )
    if (segment && boundary.offset === segment.nodeEnd && segment.nodeEnd > segment.nodeStart) {
      offset = Math.max(segment.start, offset - 1)
    }
    return { key: entry.key, offset: clampTranscriptOffset(entry.text, offset) }
  }

  function rowIndexForRect(rows: TranscriptVisualRow[], rect: DOMRect): number {
    const centerY = (rect.top + rect.bottom) / 2
    let bestIndex = -1
    let bestDistance = Number.POSITIVE_INFINITY
    for (let index = 0; index < rows.length; index += 1) {
      const row = rows[index]
      if (!row) continue
      const overlap = Math.min(row.bottom, rect.bottom) - Math.max(row.top, rect.top)
      const distance = overlap > 0 ? 0 : Math.abs(row.centerY - centerY)
      if (distance < bestDistance) {
        bestDistance = distance
        bestIndex = index
      }
    }
    return bestIndex
  }

  function rectIsOnVisualRow(row: TranscriptVisualRow, rect: DOMRect): boolean {
    const overlap = Math.min(row.bottom, rect.bottom) - Math.max(row.top, rect.top)
    return overlap >= Math.min(row.height, rect.height) * 0.4
  }

  function pointForVisualRow(row: TranscriptVisualRow, preferredX: number): CursorPoint | null {
    const model = textEntries()
    const boundary = transcriptBoundaryOnVisualRow(row, preferredX)
    return boundary ? pointForDomBoundary(boundary, model) : null
  }

  function visualRowsForMove(direction: 'up' | 'down', count: number): TranscriptVisualRow[] {
    const elements = cursorElements()
    const point = cursorPoint()
    const index = point ? elements.findIndex((element) => keyForElement(element) === point.key) : -1
    if (index < 0) return []
    const reach = Math.max(2, count + 1)
    const start = direction === 'up' ? Math.max(0, index - reach) : Math.max(0, index - 1)
    const end =
      direction === 'down' ? Math.min(elements.length, index + reach + 1) : Math.min(elements.length, index + 2)
    return transcriptVisualRows(elements.slice(start, end))
  }

  function takeCount(defaultValue = 1): number {
    const parsed = Number.parseInt(countPrefix.value, 10)
    countPrefix.value = ''
    return Number.isFinite(parsed) && parsed > 0 ? parsed : defaultValue
  }

  function moveHorizontal(forward: boolean, count = takeCount()) {
    const entry = activeTextEntry()
    if (!entry) return
    setCursorPoint({ key: entry.key, offset: moveTranscriptGrapheme(entry.text, cursorOffset.value, forward, count) })
  }

  function moveVisualLines(direction: 'up' | 'down', count = takeCount()) {
    const point = cursorPoint()
    const rect = point ? cursorRectForPoint(point) : null
    const rows = visualRowsForMove(direction, count)
    if (!point || !rect || !rows.length) return
    const currentIndex = rowIndexForRect(rows, rect)
    if (currentIndex < 0) return
    const delta = (direction === 'down' ? 1 : -1) * Math.max(1, count)
    const targetIndex = Math.max(0, Math.min(rows.length - 1, currentIndex + delta))
    const targetX = preferredViewportX.value ?? rect.left
    const target = rows[targetIndex] ? pointForVisualRow(rows[targetIndex], targetX) : null
    if (!target) return
    setCursorPoint(target, { preserveX: true })
    preferredViewportX.value = targetX
  }

  function moveWord(action: Extract<TranscriptVimAction, { type: 'word' }>, count = takeCount()) {
    const model = textEntries()
    const point = cursorPoint()
    if (!point) return
    const global = globalOffsetForPoint(point, model)
    if (global === null) return
    const target = moveTranscriptWord(model.text, global, {
      forward: action.direction === 'forward',
      toEnd: action.edge === 'end',
      bigWord: action.big,
      count,
    })
    setCursorPoint(pointForGlobalOffset(target, action.direction, model))
  }

  function moveToLineEdge(edge: 'start' | 'first-non-blank' | 'end') {
    const point = cursorPoint()
    const rect = point ? cursorRectForPoint(point) : null
    const element = point ? elementForKey(point.key) : null
    const rows = element ? transcriptVisualRows([element]) : []
    if (!point || !rect || !rows.length) return
    const row = rows[rowIndexForRect(rows, rect)]
    if (!row) return
    const targetX = edge === 'end' ? row.right : row.left
    let target = pointForVisualRow(row, targetX)
    if (!target) return

    if (edge === 'first-non-blank') {
      const entry = textEntries().entries.find((candidate) => candidate.key === target?.key)
      if (entry) {
        const graphemes = transcriptGraphemes(entry.text)
        let index = graphemes.findIndex((grapheme) => target && grapheme.start === target.offset)
        while (index >= 0 && /^\s$/u.test(graphemes[index]?.text || '')) {
          const next = graphemes[index + 1]
          if (!next) break
          const candidate = { key: entry.key, offset: next.start }
          const candidateRect = cursorRectForPoint(candidate)
          if (!candidateRect || !rectIsOnVisualRow(row, candidateRect)) break
          target = candidate
          index += 1
        }
      }
    }
    setCursorPoint(target)
  }

  function moveMessage(delta: number, count = takeCount()) {
    const messages = messageElements()
    if (!messages.length) return
    const activeMessageId = activeElement()?.dataset.messageId || ''
    let index = messages.findIndex((element) => element.dataset.messageId === activeMessageId)
    if (index < 0) index = delta > 0 ? -1 : messages.length
    const next = Math.max(0, Math.min(messages.length - 1, index + delta * count))
    pushJumpMark()
    const message = messages[next]
    const messageId = message?.dataset.messageId || ''
    const candidates = textEntries().entries.filter((entry) => entry.element.dataset.messageId === messageId)
    const target = delta > 0 ? candidates[0] : candidates.at(-1)
    if (target)
      setCursorPoint({
        key: target.key,
        offset: delta > 0 ? 0 : transcriptLineRange(target.text, target.text.length).start,
      })
    else selectElement(message)
  }

  function partForKey(key: string): TranscriptDisplayPart | null {
    for (const block of opts.renderBlocks.value) {
      if (block.kind !== 'message') continue
      const part = block.displayParts.find((candidate) => candidate.key === key)
      if (part) return part
    }
    return null
  }

  function toggleCurrentPart() {
    const part = partForKey(activeNodeKey.value)
    if (part?.toggleable) {
      opts.togglePart(part, !opts.isPartExpanded(part))
      return
    }
    activeElement()?.querySelector<HTMLButtonElement>('[data-transcript-toggle="true"]')?.click()
  }

  function visibleVisualRows(): TranscriptVisualRow[] {
    const scroll = opts.scrollEl.value
    if (!scroll) return []
    const bounds = scroll.getBoundingClientRect()
    const visibleElements = cursorElements().filter((element) => {
      const rect = element.getBoundingClientRect()
      return rect.bottom > bounds.top && rect.top < bounds.bottom
    })
    return transcriptVisualRows(visibleElements).filter((row) => row.bottom > bounds.top && row.top < bounds.bottom)
  }

  function pointAtViewportPosition(x: number, y: number): CursorPoint | null {
    const scroll = opts.scrollEl.value
    if (!scroll) return null
    const bounds = scroll.getBoundingClientRect()
    const model = textEntries()
    const directBoundary = transcriptCaretBoundaryAtPoint(x, y)
    const direct = directBoundary ? pointForDomBoundary(directBoundary, model) : null
    const directRect = direct ? cursorRectForPoint(direct) : null
    if (
      direct &&
      directRect &&
      rectIntersects(directRect, bounds) &&
      Math.abs((directRect.top + directRect.bottom) / 2 - y) <= Math.max(24, directRect.height * 1.5)
    ) {
      return direct
    }
    const rows = visibleVisualRows()
    if (!rows.length) return null
    const targetY = Math.max(bounds.top + 1, Math.min(bounds.bottom - 1, y))
    const row = rows.reduce((best, candidate) => {
      const distance =
        targetY < candidate.top ? candidate.top - targetY : targetY > candidate.bottom ? targetY - candidate.bottom : 0
      const bestDistance = targetY < best.top ? best.top - targetY : targetY > best.bottom ? targetY - best.bottom : 0
      return distance < bestDistance ? candidate : best
    })
    const targetX = Math.max(bounds.left + 1, Math.min(bounds.right - 1, x))
    return pointForVisualRow(row, targetX)
  }

  function pointAtViewportRatio(ratio: number): CursorPoint | null {
    const scroll = opts.scrollEl.value
    if (!scroll) return null
    const bounds = scroll.getBoundingClientRect()
    const x = preferredViewportX.value ?? bounds.left + Math.min(32, bounds.width / 4)
    return pointAtViewportPosition(x, bounds.top + bounds.height * ratio)
  }

  function movePage(direction: 'up' | 'down', half: boolean) {
    const scroll = opts.scrollEl.value
    if (!scroll) return
    const target = resolveTranscriptPageTarget({
      scrollTop: scroll.scrollTop,
      clientHeight: scroll.clientHeight,
      scrollHeight: scroll.scrollHeight,
      direction,
      half,
      count: takeCount(),
    })
    suppressScrollFollow()
    scroll.scrollTo({ top: target.top, behavior: 'auto' })
    window.requestAnimationFrame(() => {
      if (target.boundary) {
        const entries = textEntries().entries
        const entry = target.boundary === 'end' ? entries.at(-1) : entries[0]
        if (!entry) return
        setCursorPoint({
          key: entry.key,
          offset: target.boundary === 'end' ? transcriptLineRange(entry.text, entry.text.length).start : 0,
        })
        return
      }
      const preferredX = preferredViewportX.value
      setCursorPoint(pointAtViewportRatio(direction === 'down' ? 0.75 : 0.25), { preserveX: true })
      preferredViewportX.value = preferredX
    })
  }

  function scrollLine(direction: 'up' | 'down') {
    const count = takeCount()
    if (!cursorScreenAnchor) syncNativeSelection()
    const scroll = opts.scrollEl.value
    const point = cursorPoint()
    if (!scroll || !point) return
    suppressScrollFollow()
    scroll.scrollBy({ top: (direction === 'down' ? 1 : -1) * count * 24, behavior: 'auto' })
    window.requestAnimationFrame(() => {
      const rect = cursorRectForPoint(point)
      const bounds = scroll.getBoundingClientRect()
      if (rect && rectIntersects(rect, bounds) && cursorRectIsVisible(point, rect)) {
        syncNativeSelection()
        return
      }
      const x = preferredViewportX.value ?? cursorScreenAnchor?.x ?? bounds.left + 1
      const y = direction === 'down' ? bounds.top + 5 : bounds.bottom - 5
      const target = pointAtViewportPosition(x, y)
      if (target) installCursorWithoutReveal(target, { updatePreferredX: false })
    })
  }

  function moveView(row: 'top' | 'middle' | 'bottom', place = false) {
    if (place) {
      const scroll = opts.scrollEl.value
      const point = cursorPoint()
      const rect = point ? cursorRectForPoint(point) : null
      if (!scroll || !point || !rect) return
      const bounds = contentViewport(scroll)
      const targetY =
        row === 'top' ? bounds.top + 4 : row === 'middle' ? (bounds.top + bounds.bottom) / 2 : bounds.bottom - 4
      const sourceY = row === 'top' ? rect.top : row === 'middle' ? (rect.top + rect.bottom) / 2 : rect.bottom
      suppressScrollFollow()
      scroll.scrollTop += sourceY - targetY
      syncNativeSelection()
      return
    }

    const ratio = row === 'top' ? 0.08 : row === 'middle' ? 0.5 : 0.92
    const preferredX = preferredViewportX.value
    const point = pointAtViewportRatio(ratio)
    if (!point) return
    setCursorPoint(point, { preserveX: true })
    preferredViewportX.value = preferredX
  }

  function gotoTopOrLine() {
    const model = textEntries()
    if (!model.entries.length) return
    const count = takeCount(0)
    pushJumpMark()
    if (count > 0) {
      setCursorPoint(pointForGlobalOffset(transcriptOffsetAtLineColumn(model.text, count - 1, 0), 'forward', model), {
        center: true,
      })
    } else {
      setCursorPoint({ key: model.entries[0]!.key, offset: 0 }, { center: true })
    }
  }

  function gotoBottomOrLine() {
    const model = textEntries()
    if (!model.entries.length) return
    const count = takeCount(0)
    pushJumpMark()
    if (count > 0) {
      setCursorPoint(pointForGlobalOffset(transcriptOffsetAtLineColumn(model.text, count - 1, 0), 'forward', model), {
        center: true,
      })
    } else {
      const last = model.entries.at(-1)!
      setCursorPoint({ key: last.key, offset: transcriptLineRange(last.text, last.text.length).start })
    }
  }

  function visibleMessageText(element: HTMLElement): string {
    const messageId = String(element.dataset.messageId || '').trim()
    const parts = textEntries()
      .entries.filter((entry) => entry.element.dataset.messageId === messageId)
      .map((entry) => entry.text)
      .filter(Boolean)
    return parts.length ? parts.join('\n\n') : transcriptTextProjection(element).text
  }

  let cursorOverlay: HTMLDivElement | null = null
  let ownsNativeSelection = false

  function removeCursorOverlay() {
    cursorOverlay?.remove()
    cursorOverlay = null
  }

  function domBoundary(
    point: CursorPoint,
    direction: 'forward' | 'backward' = 'forward',
    model = textEntries(),
  ): TranscriptDomBoundary | null {
    const entry = model.entries.find((candidate) => candidate.key === point.key)
    if (!entry) return null
    return transcriptDomBoundaryForOffset(entry.element, point.offset, direction, entry.projection)
  }

  function rangeBetweenPoints(anchor: CursorPoint, head: CursorPoint, includeHead: boolean): Range | null {
    const model = textEntries()
    const anchorGlobal = globalOffsetForPoint(anchor, model)
    const headGlobal = globalOffsetForPoint(head, model)
    if (anchorGlobal === null || headGlobal === null) return null
    const startPoint = anchorGlobal <= headGlobal ? anchor : head
    const endPoint = anchorGlobal <= headGlobal ? head : anchor
    const start = domBoundary(startPoint, 'forward', model)
    const end = domBoundary(endPoint, 'forward', model)
    if (!start || !end) return null
    const range = document.createRange()
    range.setStart(start.node, Math.min(start.node.data.length, start.offset))
    let endOffset = Math.min(end.node.data.length, end.offset)
    if (includeHead) {
      const tail = transcriptGraphemes(end.node.data).find((item) => endOffset >= item.start && endOffset < item.end)
      endOffset = tail?.end ?? endOffset
    }
    range.setEnd(end.node, endOffset)
    return range
  }

  function cursorRectForPoint(point: CursorPoint): DOMRect | null {
    const range = rangeBetweenPoints(point, point, true)
    return range?.getClientRects()[0] || range?.getBoundingClientRect() || null
  }

  function scrollableOverflow(value: string): boolean {
    return value === 'auto' || value === 'scroll' || value === 'overlay'
  }

  function clippingOverflow(value: string): boolean {
    return scrollableOverflow(value) || value === 'hidden' || value === 'clip'
  }

  function contentViewport(element: HTMLElement): { left: number; right: number; top: number; bottom: number } {
    const rect = element.getBoundingClientRect()
    const left = rect.left + element.clientLeft
    const top = rect.top + element.clientTop
    return {
      left,
      right: left + element.clientWidth,
      top,
      bottom: top + element.clientHeight,
    }
  }

  function revealCursorPoint(point: CursorPoint, center: boolean) {
    const outer = opts.scrollEl.value
    const range = rangeBetweenPoints(point, point, true)
    let current = range?.startContainer.parentElement || null
    if (!outer || !range || !current) return

    while (current) {
      const style = window.getComputedStyle(current)
      const scrollsX =
        (current === outer || scrollableOverflow(style.overflowX)) && current.scrollWidth > current.clientWidth
      const scrollsY =
        (current === outer || scrollableOverflow(style.overflowY)) && current.scrollHeight > current.clientHeight
      if (scrollsX || scrollsY) {
        const viewport = contentViewport(current)
        const rect = range.getClientRects()[0] || range.getBoundingClientRect()
        if (scrollsX) {
          if (rect.left < viewport.left + 4) current.scrollLeft += rect.left - viewport.left - 4
          else if (rect.right > viewport.right - 4) current.scrollLeft += rect.right - viewport.right + 4
        }
        if (scrollsY) {
          if (center && current === outer) {
            current.scrollTop += (rect.top + rect.bottom - viewport.top - viewport.bottom) / 2
          } else if (rect.top < viewport.top + 4) {
            current.scrollTop += rect.top - viewport.top - 4
          } else if (rect.bottom > viewport.bottom - 4) {
            current.scrollTop += rect.bottom - viewport.bottom + 4
          }
        }
      }
      if (current === outer) break
      current = current.parentElement
    }
  }

  function cursorRectIsVisible(point: CursorPoint, rect: DOMRect): boolean {
    const outer = opts.scrollEl.value
    const boundary = domBoundary(point)
    let current = boundary?.node.parentElement || null
    if (!outer || !current) return false
    while (current) {
      const style = window.getComputedStyle(current)
      const viewport = contentViewport(current)
      if (clippingOverflow(style.overflowX) && (rect.right <= viewport.left || rect.left >= viewport.right))
        return false
      if (clippingOverflow(style.overflowY) && (rect.bottom <= viewport.top || rect.top >= viewport.bottom))
        return false
      if (current === outer) return true
      current = current.parentElement
    }
    return false
  }

  function visualBlockSelection(anchor: CursorPoint, head: CursorPoint): { ranges: Range[]; text: string } | null {
    const anchorRect = cursorRectForPoint(anchor)
    const headRect = cursorRectForPoint(head)
    const elements = cursorElements()
    const anchorElement = elements.findIndex((element) => keyForElement(element) === anchor.key)
    const headElement = elements.findIndex((element) => keyForElement(element) === head.key)
    if (!anchorRect || !headRect || anchorElement < 0 || headElement < 0) return null

    const rows = transcriptVisualRows(
      elements.slice(Math.min(anchorElement, headElement), Math.max(anchorElement, headElement) + 1),
    )
    const anchorRow = rowIndexForRect(rows, anchorRect)
    const headRow = rowIndexForRect(rows, headRect)
    if (anchorRow < 0 || headRow < 0) return null

    const left = Math.min(anchorRect.left, headRect.left)
    const right = Math.max(anchorRect.left, headRect.left)
    const model = textEntries()
    const ranges: Range[] = []
    const lines: string[] = []
    for (const row of rows.slice(Math.min(anchorRow, headRow), Math.max(anchorRow, headRow) + 1)) {
      const startPoint = pointForVisualRow(row, left)
      const endPoint = pointForVisualRow(row, right)
      if (!startPoint || !endPoint) continue
      const range = rangeBetweenPoints(startPoint, endPoint, true)
      const start = globalOffsetForPoint(startPoint, model)
      const end = globalOffsetForPoint(endPoint, model)
      if (!range || start === null || end === null) continue
      ranges.push(range)
      lines.push(transcriptSelectionText(model.text, start, end))
    }
    return ranges.length ? { ranges, text: lines.join('\n') } : null
  }

  function visualBlockHighlightSupport(): {
    registry: CssHighlightRegistry
    Constructor: HighlightConstructor
  } | null {
    if (typeof CSS === 'undefined') return null
    const registry = (CSS as typeof CSS & { highlights?: CssHighlightRegistry }).highlights
    const Constructor = (globalThis as typeof globalThis & { Highlight?: HighlightConstructor }).Highlight
    return registry && Constructor ? { registry, Constructor } : null
  }

  function clearVisualBlockHighlight() {
    visualBlockHighlightSupport()?.registry.delete(VISUAL_BLOCK_HIGHLIGHT)
  }

  function installVisualBlockHighlight(ranges: Range[]): boolean {
    const support = visualBlockHighlightSupport()
    if (!support) return false
    support.registry.set(VISUAL_BLOCK_HIGHLIGHT, new support.Constructor(...ranges))
    return true
  }

  function rectIntersects(left: DOMRect, right: DOMRect): boolean {
    return left.right > right.left && left.left < right.right && left.bottom > right.top && left.top < right.bottom
  }

  function syncNativeSelection(options?: { updatePreferredX?: boolean; preserveScreenAnchor?: boolean }) {
    if (typeof document === 'undefined' || mode.value === 'INSERT' || mode.value === 'SEARCH') {
      clearVisualBlockHighlight()
      removeCursorOverlay()
      return
    }
    const head = cursorPoint()
    if (!head) {
      removeCursorOverlay()
      return
    }

    const headRect = cursorRectForPoint(head)
    const viewport = opts.scrollEl.value?.getBoundingClientRect() || null
    const headIsVisible = Boolean(
      headRect && viewport && rectIntersects(headRect, viewport) && cursorRectIsVisible(head, headRect),
    )
    if (headRect && headIsVisible && !options?.preserveScreenAnchor) {
      cursorScreenAnchor = { x: headRect.left, y: (headRect.top + headRect.bottom) / 2 }
    }
    if (headRect && headIsVisible && options?.updatePreferredX) preferredViewportX.value = headRect.left

    if (mode.value.startsWith('VISUAL') && visualAnchorKey.value) {
      removeCursorOverlay()
      const anchorPoint = { key: visualAnchorKey.value, offset: visualAnchorOffset.value }
      if (mode.value === 'VISUAL BLOCK') {
        const block = visualBlockSelection(anchorPoint, head)
        if (block && installVisualBlockHighlight(block.ranges)) {
          window.getSelection()?.removeAllRanges()
          ownsNativeSelection = false
          return
        }
      } else {
        clearVisualBlockHighlight()
      }
      let range: Range | null
      if (mode.value === 'VISUAL LINE') {
        const model = textEntries()
        const anchorGlobal = globalOffsetForPoint(anchorPoint, model)
        const headGlobal = globalOffsetForPoint(head, model)
        if (anchorGlobal === null || headGlobal === null) {
          range = null
        } else {
          const lineRange = transcriptVisualLineRange(model.text, anchorGlobal, headGlobal)
          const start = pointForGlobalOffset(lineRange.start, 'forward', model)
          const finalGrapheme = transcriptGraphemes(model.text.slice(0, lineRange.end)).at(-1)
          const end = finalGrapheme
            ? pointForGlobalOffset(finalGrapheme.start, 'backward', model)
            : pointForGlobalOffset(lineRange.end, 'backward', model)
          range = start && end ? rangeBetweenPoints(start, end, true) : null
        }
      } else {
        range = rangeBetweenPoints(anchorPoint, head, true)
      }
      const selection = window.getSelection()
      if (range && selection) {
        selection.removeAllRanges()
        selection.addRange(range)
        ownsNativeSelection = true
      }
      return
    }

    clearVisualBlockHighlight()

    if (ownsNativeSelection && !mouseSelection) {
      window.getSelection()?.removeAllRanges()
      ownsNativeSelection = false
    }
    const range = rangeBetweenPoints(head, head, true)
    const rect = range?.getClientRects()[0] || range?.getBoundingClientRect()
    if (
      !rect ||
      (!rect.width && !rect.height) ||
      !viewport ||
      !rectIntersects(rect, viewport) ||
      !cursorRectIsVisible(head, rect)
    ) {
      removeCursorOverlay()
      return
    }
    if (!cursorOverlay) {
      cursorOverlay = document.createElement('div')
      cursorOverlay.dataset.agenaVimCursor = 'true'
      cursorOverlay.style.cssText =
        'position:fixed;pointer-events:none;z-index:69;background:oklch(var(--primary) / 0.3);border-bottom:2px solid oklch(var(--primary));will-change:left,top;'
      document.body.append(cursorOverlay)
    }
    cursorOverlay.dataset.transcriptKey = head.key
    cursorOverlay.dataset.transcriptOffset = String(head.offset)
    const left = Math.max(rect.left, viewport.left)
    const top = Math.max(rect.top, viewport.top)
    const right = Math.min(rect.right, viewport.right)
    const bottom = Math.min(rect.bottom, viewport.bottom)
    cursorOverlay.style.left = `${left}px`
    cursorOverlay.style.top = `${top}px`
    cursorOverlay.style.width = `${Math.max(2, right - left)}px`
    cursorOverlay.style.height = `${Math.max(2, bottom - top)}px`

    // Mouse drag selections persist independently of the vim cursor; re-apply
    // the highlight so cursor updates and scrolls don't drop it.
    if (mouseSelection && !mode.value.startsWith('VISUAL')) {
      applyMouseSelectionHighlight()
    }
  }

  function installCursorWithoutReveal(
    point: CursorPoint,
    options?: { preserveScreenAnchor?: boolean; updatePreferredX?: boolean },
  ) {
    const entry = textEntries().entries.find((candidate) => candidate.key === point.key)
    if (!entry) return
    activeNodeKey.value = entry.key
    cursorOffset.value = clampTranscriptOffset(entry.text, point.offset)
    if (mode.value.startsWith('VISUAL') && !visualAnchorKey.value) {
      visualAnchorKey.value = entry.key
      visualAnchorOffset.value = cursorOffset.value
    }
    syncNativeSelection(options)
  }

  function followCursorAfterScroll() {
    scrollFollowFrame = 0
    if (mouseSelecting) return
    if (scrollFollowSuppressed) {
      scheduleCursorPlacement()
      return
    }
    if (mode.value === 'INSERT' || mode.value === 'SEARCH') {
      removeCursorOverlay()
      return
    }
    const scroll = opts.scrollEl.value
    if (!scroll) {
      syncNativeSelection()
      return
    }

    // The cursor follows a fixed screen anchor while the viewport scrolls, so
    // content that never passes through the anchor (the very last/first line
    // when the anchor sits above/below it) would be unreachable by wheel.
    // Match the TUI's move_cursor_by_wheel clamping: once the viewport reaches
    // the top/bottom of the scrollable range, land the cursor on the boundary
    // line instead of leaving it glued to the stale anchor row.
    const boundary = transcriptScrollBoundary({
      scrollTop: scroll.scrollTop,
      clientHeight: scroll.clientHeight,
      scrollHeight: scroll.scrollHeight,
    })
    if (boundary) {
      const model = textEntries()
      const entry = boundary === 'bottom' ? model.entries.at(-1) : model.entries[0]
      if (entry) {
        const offset = boundary === 'bottom' ? transcriptLineRange(entry.text, entry.text.length).start : 0
        installCursorWithoutReveal({ key: entry.key, offset }, { updatePreferredX: false })
        return
      }
    }

    if (!cursorScreenAnchor) {
      syncNativeSelection()
      return
    }
    const target = pointAtViewportPosition(cursorScreenAnchor.x, cursorScreenAnchor.y)
    if (!target) {
      syncNativeSelection({ preserveScreenAnchor: true })
      return
    }
    installCursorWithoutReveal(target, { preserveScreenAnchor: true })
  }

  function onTranscriptScroll() {
    if (scrollFollowFrame) window.cancelAnimationFrame(scrollFollowFrame)
    scrollFollowFrame = window.requestAnimationFrame(followCursorAfterScroll)
  }

  function samePoint(left: CursorPoint, right: CursorPoint): boolean {
    return left.key === right.key && left.offset === right.offset
  }

  function mouseSelectionRange(): Range | null {
    const selection = mouseSelection
    if (!selection) return null
    return rangeBetweenPoints(selection.anchor, selection.head, true)
  }

  function mouseSelectionText(): string {
    const selection = mouseSelection
    if (!selection) return ''
    const model = textEntries()
    const anchor = globalOffsetForPoint(selection.anchor, model)
    const head = globalOffsetForPoint(selection.head, model)
    if (anchor === null || head === null) return ''
    return transcriptSelectionText(model.text, anchor, head)
  }

  function applyMouseSelectionHighlight() {
    if (!mouseSelection) return
    const range = mouseSelectionRange()
    const selection = window.getSelection()
    if (!range || !selection) return
    selection.removeAllRanges()
    selection.addRange(range)
    ownsNativeSelection = true
  }

  function clearMouseSelection() {
    mouseSelecting = false
    if (mouseSelection) {
      mouseSelection = null
      if (ownsNativeSelection) {
        window.getSelection()?.removeAllRanges()
        ownsNativeSelection = false
      }
    }
  }

  function onTranscriptPointerMove(event: PointerEvent) {
    if (!mouseSelecting || !mouseSelection) return
    event.preventDefault()
    const scroll = opts.scrollEl.value
    if (!scroll) return
    const bounds = scroll.getBoundingClientRect()
    if (event.clientY < bounds.top || event.clientY > bounds.bottom) {
      const delta = event.clientY < bounds.top ? event.clientY - bounds.top : event.clientY - bounds.bottom
      scroll.scrollTop = Math.max(0, Math.min(scroll.scrollHeight - scroll.clientHeight, scroll.scrollTop + delta))
    }
    const model = textEntries()
    const boundary = transcriptCaretBoundaryAtPoint(event.clientX, event.clientY)
    const point =
      (boundary ? pointForDomBoundary(boundary, model) : null) || pointAtViewportPosition(event.clientX, event.clientY)
    if (!point) return
    mouseSelection = { anchor: mouseSelection.anchor, head: point }
    installCursorWithoutReveal(point, { updatePreferredX: false, preserveScreenAnchor: true })
  }

  function onTranscriptPointerUp(event: PointerEvent) {
    if (!mouseSelecting) return
    mouseSelecting = false
    window.removeEventListener('pointermove', onTranscriptPointerMove, true)
    window.removeEventListener('pointerup', onTranscriptPointerUp, true)
    window.removeEventListener('pointercancel', onTranscriptPointerCancel, true)
    event.preventDefault()
    if (!mouseSelection) return
    if (samePoint(mouseSelection.anchor, mouseSelection.head)) {
      clearMouseSelection()
      scheduleCursorPlacement()
      return
    }
    applyMouseSelectionHighlight()
    // A browser still dispatches `click` after a drag; suppress it so a drag
    // ending on a link or toggle does not trigger navigation/activation.
    suppressMouseClickUntil = Date.now() + 350
  }

  function onTranscriptPointerCancel() {
    if (!mouseSelecting) return
    mouseSelecting = false
    window.removeEventListener('pointermove', onTranscriptPointerMove, true)
    window.removeEventListener('pointerup', onTranscriptPointerUp, true)
    window.removeEventListener('pointercancel', onTranscriptPointerCancel, true)
    clearMouseSelection()
    scheduleCursorPlacement()
  }

  function onTranscriptPointerDown(event: PointerEvent) {
    if (event.button !== 0 || !(event.target instanceof Element)) return
    const scroll = opts.scrollEl.value
    if (!scroll) return
    if (event.target.closest('[data-transcript-chrome="true"]')) return
    const transcriptNode = event.target.closest<HTMLElement>(NODE_SELECTOR)
    const insideTranscript =
      Boolean(transcriptNode && opts.pageRef.value?.contains(transcriptNode)) || scroll.contains(event.target)
    if (!insideTranscript) return

    const model = textEntries()
    const boundary = transcriptCaretBoundaryAtPoint(event.clientX, event.clientY)
    const point =
      (boundary ? pointForDomBoundary(boundary, model) : null) || pointAtViewportPosition(event.clientX, event.clientY)
    if (!point) return
    // Mouse interaction always positions the cursor (keyboard navigation stays
    // gated by the focused pane), matching the TUI. A drag becomes the mouse
    // selection, so any active vim visual selection is released first.
    if (mode.value.startsWith('VISUAL')) cancelVisual()
    if (mode.value === 'INSERT') mode.value = 'NAVIGATE'
    installCursorWithoutReveal(point, { updatePreferredX: true })
    clearMouseSelection()
    mouseSelection = { anchor: point, head: point }
    mouseSelecting = true
    // Own the selection: suppress the browser's native selection/drag so the
    // transcript selection model stays consistent.
    event.preventDefault()
    window.addEventListener('pointermove', onTranscriptPointerMove, true)
    window.addEventListener('pointerup', onTranscriptPointerUp, true)
    window.addEventListener('pointercancel', onTranscriptPointerCancel, true)
    scheduleCursorPlacement()
  }

  function onTranscriptCopy(event: ClipboardEvent) {
    if (!mouseSelection) return
    const text = mouseSelectionText()
    if (!text) return
    event.preventDefault()
    clearMouseSelection()
    scheduleCursorPlacement()
    void copyRawText(text)
  }

  function onTranscriptClickCapture(event: MouseEvent) {
    if (Date.now() >= suppressMouseClickUntil) return
    const target = event.target
    if (!(target instanceof Element)) return
    if (!opts.scrollEl.value?.contains(target)) return
    event.preventDefault()
    event.stopImmediatePropagation()
  }

  const selectedNodeKeys = computed(() => {
    if (!mode.value.startsWith('VISUAL') || !visualAnchorKey.value || !activeNodeKey.value) return new Set<string>()
    const nodes = cursorElements()
    const anchor = nodeIndex(visualAnchorKey.value, nodes)
    const head = nodeIndex(activeNodeKey.value, nodes)
    if (anchor < 0 || head < 0) return new Set<string>()
    const start = Math.min(anchor, head)
    const end = Math.max(anchor, head)
    return new Set(
      nodes
        .slice(start, end + 1)
        .map(keyForElement)
        .filter(Boolean),
    )
  })

  function visualSelectionText(): string {
    const model = textEntries()
    const anchor = globalOffsetForPoint({ key: visualAnchorKey.value, offset: visualAnchorOffset.value }, model)
    const head = globalOffsetForPoint({ key: activeNodeKey.value, offset: cursorOffset.value }, model)
    if (anchor === null || head === null) return ''

    if (mode.value === 'VISUAL LINE') {
      const range = transcriptVisualLineRange(model.text, anchor, head)
      return model.text.slice(range.start, range.end)
    }

    if (mode.value === 'VISUAL BLOCK') {
      if (visualBlockHighlightSupport()) {
        const block = visualBlockSelection(
          { key: visualAnchorKey.value, offset: visualAnchorOffset.value },
          { key: activeNodeKey.value, offset: cursorOffset.value },
        )
        if (block) return block.text
      }
    }

    return transcriptSelectionText(model.text, anchor, head)
  }

  async function copyRawText(value: string) {
    if (!value) return
    const ok = await copyTextToClipboard(value)
    opts.toasts.push(
      ok ? 'success' : 'error',
      ok ? 'Copied transcript selection' : 'Failed to copy transcript selection',
    )
  }

  function isNodeSelected(key: string): boolean {
    return selectedNodeKeys.value.has(key)
  }

  function startVisual(nextMode: 'character' | 'line' | 'block') {
    ensureActive()
    const mapped: TranscriptVimMode =
      nextMode === 'line' ? 'VISUAL LINE' : nextMode === 'block' ? 'VISUAL BLOCK' : 'VISUAL'
    if (mode.value === mapped) {
      mode.value = 'NAVIGATE'
      visualAnchorKey.value = ''
      visualAnchorOffset.value = 0
      syncNativeSelection()
      return
    }
    mode.value = mapped
    visualAnchorKey.value = activeNodeKey.value
    visualAnchorOffset.value = cursorOffset.value
    pendingCommand.value = ''
    syncNativeSelection()
  }

  function cancelVisual() {
    if (mode.value.startsWith('VISUAL') && visualAnchorKey.value && activeNodeKey.value) {
      lastVisualRange.value = {
        anchor: { key: visualAnchorKey.value, offset: visualAnchorOffset.value },
        head: { key: activeNodeKey.value, offset: cursorOffset.value },
        mode: mode.value,
      }
    }
    mode.value = 'NAVIGATE'
    visualAnchorKey.value = ''
    visualAnchorOffset.value = 0
    syncNativeSelection()
  }

  function swapVisualEndpoint() {
    if (!mode.value.startsWith('VISUAL') || !visualAnchorKey.value) return
    const head = activeNodeKey.value
    const headOffset = cursorOffset.value
    activeNodeKey.value = visualAnchorKey.value
    cursorOffset.value = visualAnchorOffset.value
    visualAnchorKey.value = head
    visualAnchorOffset.value = headOffset
    selectElement(elementForKey(activeNodeKey.value), { offset: cursorOffset.value })
  }

  async function yankSelectionOrStartOperator() {
    if (mode.value.startsWith('VISUAL')) {
      await copyRawText(visualSelectionText())
      cancelVisual()
      return
    }
    pendingCommand.value = 'y'
    commandEcho.value = 'y'
  }

  async function yankCurrentLines() {
    const model = textEntries()
    const point = cursorPoint()
    if (!point) return
    const global = globalOffsetForPoint(point, model)
    if (global === null) return
    const count = takeCount()
    const start = transcriptLineRange(model.text, global).start
    const position = transcriptLinePosition(model.text, global)
    const finalLine = position.line + Math.max(1, count) - 1
    const finalOffset = transcriptOffsetAtLineColumn(model.text, finalLine, 0)
    const end = transcriptLineRange(model.text, finalOffset).end
    await copyRawText(model.text.slice(start, end))
    pendingCommand.value = ''
  }

  function textObjectRange(
    scope: 'markdown' | 'message' | 'paragraph' | 'word',
    around: boolean,
  ): {
    text: string
    range: { start: number; end: number }
    entry?: TextEntry
  } | null {
    const active = ensureActive()
    const entry = activeTextEntry()
    if (!active || !entry) return null
    if (scope === 'message') {
      const messageId = active.dataset.messageId || ''
      const message = messageElements().find((element) => element.dataset.messageId === messageId)
      const text = message ? visibleMessageText(message) : entry.text
      return { text, range: { start: 0, end: text.length } }
    }
    if (scope === 'markdown') return { text: entry.text, range: { start: 0, end: entry.text.length }, entry }
    const range =
      scope === 'word'
        ? transcriptWordRange(entry.text, cursorOffset.value, around)
        : transcriptParagraphRange(entry.text, cursorOffset.value, around)
    return { text: entry.text, range, entry }
  }

  async function yankTextObject(scope: 'markdown' | 'message' | 'paragraph' | 'word', around: boolean) {
    const object = textObjectRange(scope, around)
    if (!object) return
    await copyRawText(object.text.slice(object.range.start, object.range.end))
    pendingCommand.value = ''
    commandEcho.value = ''
  }

  function selectTextObject(scope: 'markdown' | 'message' | 'paragraph' | 'word', around: boolean) {
    if (scope === 'message') {
      const messageId = activeElement()?.dataset.messageId || activeTextEntry()?.element.dataset.messageId || ''
      const entries = textEntries().entries.filter((entry) => entry.element.dataset.messageId === messageId)
      const first = entries[0]
      const last = entries.at(-1)
      if (!first || !last) return
      visualAnchorKey.value = first.key
      visualAnchorOffset.value = 0
      const graphemes = transcriptGraphemes(last.text)
      setCursorPoint({ key: last.key, offset: graphemes.at(-1)?.start ?? 0 })
      return
    }
    const object = textObjectRange(scope, around)
    if (!object) return
    if (!object.entry) return
    visualAnchorKey.value = object.entry.key
    visualAnchorOffset.value = object.range.start
    const graphemes = transcriptGraphemes(object.text.slice(object.range.start, object.range.end))
    const headOffset = object.range.start + (graphemes.at(-1)?.start ?? 0)
    setCursorPoint({ key: object.entry.key, offset: headOffset })
  }

  let searchHighlightQueued = false
  let applyingSearchHighlight = false
  let searchHighlightObserver: MutationObserver | null = null

  function removeSearchHighlights(root: HTMLElement) {
    for (const mark of Array.from(root.querySelectorAll<HTMLElement>('mark[data-agena-search-match]'))) {
      const parent = mark.parentNode
      if (!parent) continue
      parent.replaceChild(document.createTextNode(mark.textContent || ''), mark)
      parent.normalize()
    }
  }

  function wrapSearchRange(startNode: Text, startOffset: number, endNode: Text, endOffset: number, active: boolean) {
    const range = document.createRange()
    range.setStart(startNode, Math.max(0, Math.min(startNode.data.length, startOffset)))
    range.setEnd(endNode, Math.max(0, Math.min(endNode.data.length, endOffset)))
    if (range.collapsed) return
    const mark = document.createElement('mark')
    mark.dataset.agenaSearchMatch = 'true'
    if (active) mark.dataset.agenaSearchActive = 'true'
    mark.append(range.extractContents())
    range.insertNode(mark)
  }

  function applySearchHighlightNow() {
    const root = opts.pageRef.value
    if (!root) return
    applyingSearchHighlight = true
    try {
      removeSearchHighlights(root)
      invalidateTextModel()
      const query = searchQuery.value.trim()
      if (!query || !searchMatches.value.length) return
      const activeMatch = searchMatchIndex.value >= 0 ? searchMatches.value[searchMatchIndex.value] : null
      const activeKey = activeMatch?.key || ''
      for (const element of cursorElements()) {
        const key = keyForElement(element)
        const projection = transcriptTextProjection(element)
        if (!projection.segments.length) continue
        const ranges = transcriptSearchRanges(projection.text, query)
        if (!ranges.length) continue
        const keyMatches = searchMatches.value.filter((match) => match.key === key)
        const activeOrdinal = activeKey === key ? keyMatches.indexOf(activeMatch as TranscriptSearchMatch) : -1
        for (let rangeIndex = ranges.length - 1; rangeIndex >= 0; rangeIndex -= 1) {
          const range = ranges[rangeIndex]
          const segments = projection.segments.filter(
            (segment) => segment.end > range.start && segment.start < range.end,
          )
          for (let segmentIndex = segments.length - 1; segmentIndex >= 0; segmentIndex -= 1) {
            const segment = segments[segmentIndex]
            if (!segment) continue
            const start = segment.nodeStart + Math.max(range.start, segment.start) - segment.start
            const end = segment.nodeStart + Math.min(range.end, segment.end) - segment.start
            wrapSearchRange(segment.node, start, segment.node, end, rangeIndex === activeOrdinal)
          }
        }
      }
    } finally {
      invalidateTextModel()
      scheduleCursorPlacement()
      queueMicrotask(() => {
        applyingSearchHighlight = false
      })
    }
  }

  function scheduleSearchHighlight() {
    if (searchHighlightQueued) return
    searchHighlightQueued = true
    void nextTick(() => {
      searchHighlightQueued = false
      applySearchHighlightNow()
    })
  }

  function refreshSearchMatches() {
    const query = searchQuery.value.trim()
    if (!query) {
      searchMatches.value = []
      searchMatchIndex.value = -1
      scheduleSearchHighlight()
      return
    }
    searchMatches.value = collectTranscriptSearchMatches(textEntries().entries, query)
    searchMatchIndex.value = -1
    scheduleSearchHighlight()
  }

  function isNodeSearchMatch(key: string): boolean {
    return searchMatches.value.some((match) => match.key === key)
  }

  function openSearch(forward: boolean) {
    searchForward.value = forward
    searchOpen.value = true
    mode.value = 'SEARCH'
    searchMatchIndex.value = -1
    removeCursorOverlay()
    commandEcho.value = forward ? '/' : '?'
    refreshSearchMatches()
    nextTick(() => {
      opts.searchInputRef.value?.focus()
      opts.searchInputRef.value?.select()
    })
  }

  function setSearchQuery(query: string) {
    searchQuery.value = query
    refreshSearchMatches()
  }

  function closeSearch(clear = false) {
    searchOpen.value = false
    mode.value = 'NAVIGATE'
    if (clear) {
      searchQuery.value = ''
      searchMatches.value = []
      searchMatchIndex.value = -1
      scheduleSearchHighlight()
    }
    ensureActive()?.focus({ preventScroll: true })
    syncNativeSelection()
  }

  function selectSearchMatch(match: TranscriptSearchMatch, recordJump: boolean) {
    const element = elementForKey(match.key)
    if (!element) return
    searchMatchIndex.value = searchMatches.value.indexOf(match)
    selectElement(element, { center: true, recordJump, offset: match.textStart })
    scheduleSearchHighlight()
  }

  function jumpSearch(reverse: boolean) {
    if (!searchMatches.value.length) refreshSearchMatches()
    if (!searchMatches.value.length) return
    const forward = searchForward.value !== reverse
    const point = cursorPoint()
    const activeMatch = searchMatchIndex.value >= 0 ? searchMatches.value[searchMatchIndex.value] : null
    const cursorGlobal = point ? globalOffsetForPoint(point) : 0
    let anchor = cursorGlobal ?? 0
    if (
      activeMatch &&
      cursorGlobal !== null &&
      cursorGlobal >= activeMatch.globalStart &&
      cursorGlobal < activeMatch.globalEnd
    ) {
      anchor = forward ? activeMatch.globalEnd : activeMatch.globalStart
    }
    const next = nextTranscriptSearchMatchIndex(searchMatches.value, anchor, forward)
    const match = searchMatches.value[next]
    if (!match) return
    selectSearchMatch(match, true)
    if (searchOpen.value) {
      void nextTick(() => {
        opts.searchInputRef.value?.focus()
        opts.searchInputRef.value?.setSelectionRange(searchQuery.value.length, searchQuery.value.length)
      })
    }
  }

  function handleSearchKeydown(event: KeyboardEvent) {
    event.stopPropagation()
    if (event.key === 'Escape') {
      event.preventDefault()
      closeSearch(false)
      return
    }
    if (event.key === 'Enter') {
      event.preventDefault()
      jumpSearch(false)
      closeSearch(false)
      return
    }
    if (event.key === 'ArrowDown') {
      event.preventDefault()
      jumpSearch(false)
      return
    }
    if (event.key === 'ArrowUp') {
      event.preventDefault()
      jumpSearch(true)
    }
  }

  function findTarget(target: string, find: PendingFind) {
    const entry = activeTextEntry()
    if (!entry) return
    const offset = findTranscriptCharacter(entry.text, cursorOffset.value, target, {
      forward: find.direction === 'forward',
      till: find.till,
      count: find.count,
    })
    setCursorPoint({ key: entry.key, offset })
    lastFind.value = { ...find, target }
  }

  function repeatFind(reverse: boolean) {
    const previous = lastFind.value
    if (!previous) return
    findTarget(previous.target, {
      ...previous,
      direction: reverse ? (previous.direction === 'forward' ? 'backward' : 'forward') : previous.direction,
      count: takeCount(),
    })
  }

  function jumpHistoryMove(direction: 'backward' | 'forward') {
    if (!jumpHistory.value.length) return
    if (direction === 'backward') jumpHistoryIndex.value = Math.max(0, jumpHistoryIndex.value - 1)
    else jumpHistoryIndex.value = Math.min(jumpHistory.value.length, jumpHistoryIndex.value + 1)
    const point = jumpHistory.value[jumpHistoryIndex.value]
    if (point) setCursorPoint(point, { center: true })
  }

  function enterInsertMode() {
    mode.value = 'INSERT'
    pendingCommand.value = ''
    countPrefix.value = ''
    removeCursorOverlay()
    if (ownsNativeSelection) {
      window.getSelection()?.removeAllRanges()
      ownsNativeSelection = false
    }
    nextTick(() => composerTextarea(opts.composerRef.value)?.focus())
  }

  function returnToNavigate() {
    mode.value = 'NAVIGATE'
    pendingCommand.value = ''
    pendingFind.value = null
    countPrefix.value = ''
    ensureActive()?.focus({ preventScroll: true })
    syncNativeSelection()
  }

  function clearPending() {
    pendingCommand.value = ''
    pendingFind.value = null
    countPrefix.value = ''
    commandEcho.value = ''
  }

  async function yankByMotion(action: Extract<TranscriptVimAction, { type: 'move' | 'line' | 'word' }>) {
    const origin = cursorPoint()
    if (!origin) return
    const count = takeCount()
    const linewise = action.type === 'move' && (action.direction === 'up' || action.direction === 'down')

    if (action.type === 'move') {
      if (action.direction === 'up') moveVisualLines('up', count)
      else if (action.direction === 'down') moveVisualLines('down', count)
      else moveHorizontal(action.direction === 'right', count)
    } else if (action.type === 'line') {
      moveToLineEdge(action.edge)
    } else {
      moveWord(action, count)
    }

    const destination = cursorPoint()
    const model = textEntries()
    const originGlobal = globalOffsetForPoint(origin, model)
    const destinationGlobal = destination ? globalOffsetForPoint(destination, model) : null
    if (originGlobal !== null && destinationGlobal !== null) {
      if (linewise) {
        const start = transcriptLineRange(model.text, Math.min(originGlobal, destinationGlobal)).start
        const end = transcriptLineRange(model.text, Math.max(originGlobal, destinationGlobal)).end
        await copyRawText(model.text.slice(start, end))
      } else {
        const start = Math.min(originGlobal, destinationGlobal)
        const forwardWordExclusive =
          action.type === 'word' &&
          action.direction === 'forward' &&
          action.edge === 'start' &&
          destinationGlobal > originGlobal
        const end = forwardWordExclusive
          ? destinationGlobal
          : transcriptSelectionEnd(model.text, Math.max(originGlobal, destinationGlobal))
        await copyRawText(model.text.slice(start, end))
      }
    }
    setCursorPoint(origin)
    clearPending()
  }

  function handlePendingKey(event: KeyboardEvent, action: TranscriptVimAction | null): boolean {
    if (pendingFind.value) {
      const find = pendingFind.value
      pendingFind.value = null
      if (event.key.length === 1 && !event.metaKey && !event.ctrlKey && !event.altKey) findTarget(event.key, find)
      clearPending()
      return true
    }

    if (pendingCommand.value === 'g') {
      pendingCommand.value = ''
      if (event.key === 'g') gotoTopOrLine()
      else if (event.key === 'v' && lastVisualRange.value) {
        countPrefix.value = ''
        mode.value = lastVisualRange.value.mode
        visualAnchorKey.value = lastVisualRange.value.anchor.key
        visualAnchorOffset.value = lastVisualRange.value.anchor.offset
        setCursorPoint(lastVisualRange.value.head)
      } else if (event.key === 'e' || event.key === 'E') {
        moveWord(
          {
            type: 'word',
            direction: 'backward',
            edge: 'end',
            big: event.key === 'E',
          },
          takeCount(),
        )
      } else clearPending()
      commandEcho.value = ''
      return true
    }

    if (pendingCommand.value === 'z') {
      pendingCommand.value = ''
      if (event.key === 'z') moveView('middle', true)
      else if (event.key === 't') moveView('top', true)
      else if (event.key === 'b') moveView('bottom', true)
      clearPending()
      return true
    }

    if (pendingCommand.value === 'y') {
      if (action?.type === 'count') {
        countPrefix.value += String(action.digit)
        return true
      }
      if (event.key === 'y') {
        void yankCurrentLines()
        return true
      }
      if (event.key === 'a' || event.key === 'i') {
        pendingCommand.value = `y${event.key}`
        commandEcho.value = pendingCommand.value
        return true
      }
      if (event.key === 'g') {
        pendingCommand.value = 'yg'
        commandEcho.value = pendingCommand.value
        return true
      }
      if (action?.type === 'move' || action?.type === 'line' || action?.type === 'word') {
        void yankByMotion(action)
        return true
      }
      clearPending()
      return true
    }

    if (pendingCommand.value === 'yg') {
      if (event.key === 'e' || event.key === 'E') {
        void yankByMotion({
          type: 'word',
          direction: 'backward',
          edge: 'end',
          big: event.key === 'E',
        })
      } else clearPending()
      return true
    }

    if (pendingCommand.value === 'ya' || pendingCommand.value === 'yi') {
      const around = pendingCommand.value === 'ya'
      if (event.key === 'M') void yankTextObject('message', around)
      else if (event.key === 'm') void yankTextObject('markdown', around)
      else if (event.key === 'p') void yankTextObject('paragraph', around)
      else if (event.key === 'w') void yankTextObject('word', around)
      else clearPending()
      return true
    }

    if (pendingCommand.value === 'va' || pendingCommand.value === 'vi') {
      const around = pendingCommand.value === 'va'
      if (event.key === 'M') selectTextObject('message', around)
      else if (event.key === 'm') selectTextObject('markdown', around)
      else if (event.key === 'p') selectTextObject('paragraph', around)
      else if (event.key === 'w') selectTextObject('word', around)
      else clearPending()
      pendingCommand.value = ''
      commandEcho.value = ''
      return true
    }
    return false
  }

  function executeAction(event: KeyboardEvent, action: TranscriptVimAction): boolean {
    if (action.type === 'interrupt') {
      // With an active mouse selection, Ctrl+C means copy — leave it to the
      // browser's `copy` event instead of aborting the run.
      if (mouseSelection) return false
      if (opts.draft.value.trim()) opts.clearComposer()
      else if (opts.canAbort.value) void opts.abortRun()
      return true
    }
    if (action.type === 'copy') {
      if (mouseSelection) {
        const text = mouseSelectionText()
        clearMouseSelection()
        scheduleCursorPlacement()
        if (text) void copyRawText(text)
        return true
      }
      void yankSelectionOrStartOperator()
      return true
    }
    if (action.type === 'cancel') {
      if (mouseSelection) {
        clearMouseSelection()
        scheduleCursorPlacement()
        return true
      }
      if (mode.value.startsWith('VISUAL')) cancelVisual()
      else clearPending()
      return true
    }
    // Any other action replaces the mouse selection, mirroring the TUI where a
    // pointer selection is cancelled once the cursor moves or the mode changes.
    clearMouseSelection()

    if (action.type === 'count') {
      countPrefix.value += String(action.digit)
      commandEcho.value = ''
      return true
    }
    if (action.type === 'insert') {
      if (mode.value.startsWith('VISUAL')) {
        pendingCommand.value = 'vi'
        commandEcho.value = 'vi'
        return true
      }
      enterInsertMode()
      return true
    }
    if (action.type === 'visual') {
      startVisual(action.mode)
      return true
    }
    if (action.type === 'yank-line') {
      void yankCurrentLines()
      return true
    }
    if (action.type === 'toggle') {
      toggleCurrentPart()
      clearPending()
      return true
    }
    if (action.type === 'move') {
      if (action.direction === 'up') moveVisualLines('up')
      else if (action.direction === 'down') moveVisualLines('down')
      else moveHorizontal(action.direction === 'right')
      return true
    }
    if (action.type === 'message') {
      moveMessage(action.direction === 'previous' ? -1 : 1)
      return true
    }
    if (action.type === 'line') {
      if (event.key === '0' && countPrefix.value) {
        countPrefix.value += '0'
        commandEcho.value = ''
      } else moveToLineEdge(action.edge)
      return true
    }
    if (action.type === 'word') {
      moveWord(action)
      return true
    }
    if (action.type === 'find') {
      pendingFind.value = { direction: action.direction, till: action.till, count: takeCount() }
      commandEcho.value = event.key
      return true
    }
    if (action.type === 'repeat-find') {
      repeatFind(action.reverse)
      return true
    }
    if (action.type === 'goto-prefix') {
      pendingCommand.value = 'g'
      commandEcho.value = 'g'
      return true
    }
    if (action.type === 'end') {
      gotoBottomOrLine()
      return true
    }
    if (action.type === 'viewport-prefix') {
      pendingCommand.value = 'z'
      commandEcho.value = 'z'
      return true
    }
    if (action.type === 'view') {
      moveView(action.row)
      return true
    }
    if (action.type === 'swap-endpoint') {
      swapVisualEndpoint()
      return true
    }
    if (action.type === 'text-object') {
      if (mode.value.startsWith('VISUAL') && action.scope === 'around') {
        pendingCommand.value = 'va'
        commandEcho.value = 'va'
        return true
      }
      clearPending()
      return false
    }
    if (action.type === 'page') {
      movePage(action.direction, action.half)
      return true
    }
    if (action.type === 'scroll-line') {
      scrollLine(action.direction)
      return true
    }
    if (action.type === 'jump') {
      jumpHistoryMove(action.direction)
      return true
    }
    if (action.type === 'search') {
      openSearch(action.direction === 'forward')
      return true
    }
    if (action.type === 'search-repeat') {
      jumpSearch(action.reverse)
      return true
    }
    if (action.type === 'help') {
      opts.toggleHelp()
      return true
    }
    if (action.type === 'open-plan') {
      opts.openPlan()
      return true
    }
    if (action.type === 'new-session') {
      // Session creation remains centralized in the application shortcut so
      // embedded workspace routing keeps the same behavior.
      return false
    }
    // Unknown future actions remain available to application-level shortcuts.
    return false
  }

  function onKeydown(event: KeyboardEvent) {
    if (opts.enabled && !opts.enabled.value) return
    if (!opts.pageRef.value?.isConnected || event.defaultPrevented) return
    const target = event.target
    const textarea = composerTextarea(opts.composerRef.value)

    if (target === opts.searchInputRef.value) return
    if (target === textarea) {
      mode.value = 'INSERT'
      if (event.key === 'Escape') {
        event.preventDefault()
        event.stopPropagation()
        returnToNavigate()
        return
      }
      const action = resolveTranscriptVimAction(event)
      if (action?.type === 'interrupt') {
        // An active mouse selection means Ctrl+C copies the selection; let the
        // browser's `copy` event handle it instead of clearing the composer.
        if (mouseSelection) return
        event.preventDefault()
        event.stopPropagation()
        if (opts.draft.value.trim()) opts.clearComposer()
        else if (opts.canAbort.value) void opts.abortRun()
      } else if (action?.type === 'open-plan') {
        // Composer focus does not open Plan in the TUI, but Ctrl+P must not
        // fall through to the browser's print dialog.
        event.preventDefault()
        event.stopPropagation()
      }
      return
    }

    if (isEditable(target)) return
    if (target instanceof Element && target.closest('[data-transcript-chrome="true"]')) return
    if (target instanceof Element) {
      const owningControl = target.closest('button, a, select, [role="button"], [role="menuitem"], [role="option"]')
      if (
        owningControl &&
        !owningControl.matches('[data-transcript-toggle="true"], [data-transcript-vim-toggle="true"]')
      ) {
        return
      }
      const transcriptTarget = target.closest('[data-transcript-root="true"], [data-transcript-node]')
      const passiveSurfaceTarget =
        target === opts.pageRef.value ||
        target === opts.scrollEl.value ||
        target === document.body ||
        target === document.documentElement
      if (!transcriptTarget && !passiveSurfaceTarget) return
    }

    if (mode.value === 'INSERT') mode.value = 'NAVIGATE'
    ensureActive()

    if (pendingFind.value || pendingCommand.value) {
      const action = resolveTranscriptVimAction(event)
      event.preventDefault()
      event.stopPropagation()
      handlePendingKey(event, action)
      return
    }

    const action = resolveTranscriptVimAction(event)
    if (!action) return
    const handled = executeAction(event, action)
    if (handled) {
      event.preventDefault()
      event.stopPropagation()
    }
  }

  const modeLabel = computed(() => mode.value)
  const commandLabel = computed(() => {
    if (mode.value === 'SEARCH') return `${searchForward.value ? '/' : '?'}${searchQuery.value}`
    return `${countPrefix.value}${commandEcho.value || pendingCommand.value}`
  })
  const searchSummary = computed(() => {
    if (!searchQuery.value.trim()) return ''
    const total = searchMatches.value.length
    const current = total && searchMatchIndex.value >= 0 ? searchMatchIndex.value + 1 : 0
    return `${current}/${total}`
  })

  function onTranscriptResize() {
    invalidateTextModel()
    scheduleCursorPlacement()
  }

  watch(
    () => opts.selectedSessionId.value,
    () => {
      invalidateTextModel()
      activeNodeKey.value = ''
      visualAnchorKey.value = ''
      cursorOffset.value = 0
      preferredViewportX.value = null
      cursorScreenAnchor = null
      clearMouseSelection()
      visualAnchorOffset.value = 0
      searchQuery.value = ''
      searchMatches.value = []
      searchMatchIndex.value = -1
      scheduleSearchHighlight()
      jumpHistory.value = []
      jumpHistoryIndex.value = 0
      clearPending()
      mode.value = 'NAVIGATE'
      nextTick(() => {
        ensureActive(true)
        syncNativeSelection({ updatePreferredX: true })
      })
    },
  )

  watch(
    () => opts.renderBlocks.value.map((block) => block.key).join('|'),
    () =>
      nextTick(() => {
        invalidateTextModel()
        ensureActive(true)
        syncNativeSelection({ updatePreferredX: true })
        scheduleSearchHighlight()
      }),
  )

  onMounted(() => {
    const root = opts.pageRef.value
    mountedRoot = root
    mountedScroll = opts.scrollEl.value
    if (root && typeof MutationObserver !== 'undefined') {
      searchHighlightObserver = new MutationObserver(() => {
        invalidateTextModel()
        if (applyingSearchHighlight) return
        if (!searchOpen.value && !searchQuery.value.trim()) return
        scheduleSearchHighlight()
      })
      searchHighlightObserver.observe(root, { childList: true, subtree: true, characterData: true })
    }
    if (mountedScroll && typeof ResizeObserver !== 'undefined') {
      transcriptResizeObserver = new ResizeObserver(onTranscriptResize)
      transcriptResizeObserver.observe(mountedScroll)
    }
    root?.addEventListener('pointerdown', onTranscriptPointerDown, true)
    window.addEventListener('keydown', onKeydown)
    window.addEventListener('resize', onTranscriptResize)
    document.addEventListener('copy', onTranscriptCopy)
    document.addEventListener('click', onTranscriptClickCapture, true)
    mountedScroll?.addEventListener('scroll', onTranscriptScroll, { passive: true })
  })
  onBeforeUnmount(() => {
    searchHighlightObserver?.disconnect()
    searchHighlightObserver = null
    transcriptResizeObserver?.disconnect()
    transcriptResizeObserver = null
    mountedRoot?.removeEventListener('pointerdown', onTranscriptPointerDown, true)
    window.removeEventListener('keydown', onKeydown)
    window.removeEventListener('resize', onTranscriptResize)
    document.removeEventListener('copy', onTranscriptCopy)
    document.removeEventListener('click', onTranscriptClickCapture, true)
    mountedScroll?.removeEventListener('scroll', onTranscriptScroll)
    if (placementFrame) window.cancelAnimationFrame(placementFrame)
    if (scrollFollowFrame) window.cancelAnimationFrame(scrollFollowFrame)
    if (scrollSuppressionFrame) window.cancelAnimationFrame(scrollSuppressionFrame)
    clearMouseSelection()
    clearVisualBlockHighlight()
    removeCursorOverlay()
    if (ownsNativeSelection) window.getSelection()?.removeAllRanges()
  })

  return {
    mode,
    modeLabel,
    commandLabel,
    activeNodeKey,
    selectedNodeKeys,
    searchOpen,
    searchQuery,
    searchSummary,
    searchMatches,
    searchMatchIndex,
    searchForward,
    selectNode,
    isNodeSelected,
    isNodeSearchMatch,
    setSearchQuery,
    handleSearchKeydown,
    closeSearch,
    enterInsertMode,
    returnToNavigate,
  }
}
