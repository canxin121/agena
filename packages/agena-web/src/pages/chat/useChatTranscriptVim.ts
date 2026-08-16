import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch, type ComputedRef, type Ref } from 'vue'

import type { RenderBlock, TranscriptDisplayPart } from '@/components/chat/messageList.types'
import { copyTextToClipboard } from '@/lib/clipboard'
import { resolveTranscriptPageTarget } from './transcriptNavigation'
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
  transcriptWordRange,
} from './transcriptTextCursor'

type ComposerExpose = {
  textareaEl?: HTMLTextAreaElement | { value: HTMLTextAreaElement | null } | null
}

type ToastsLike = { push: (kind: 'success' | 'error' | 'info', message: string, duration?: number) => void }

type PendingFind = { direction: 'forward' | 'backward'; till: boolean; count: number }
type LastFind = PendingFind & { target: string }
type CursorPoint = { key: string; offset: number }
type TextEntry = CursorPoint & { element: HTMLElement; text: string; start: number; end: number }

const NODE_SELECTOR = '[data-transcript-node][data-transcript-key]'
const MESSAGE_SELECTOR = '[data-transcript-node="message"][data-transcript-key]'

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
  const preferredColumn = ref(0)
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

  function textEntries(): { entries: TextEntry[]; text: string } {
    const entries: TextEntry[] = []
    let combined = ''
    for (const element of cursorElements()) {
      const key = keyForElement(element)
      const value = cleanNodeCopyText(element)
      if (!key || !value) continue
      if (combined) combined += '\n'
      const start = combined.length
      combined += value
      entries.push({ key, offset: 0, element, text: value, start, end: combined.length })
    }
    return { entries, text: combined }
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

  function setCursorPoint(point: CursorPoint | null, options?: { center?: boolean; preserveColumn?: boolean }) {
    if (!point) return
    const entry = textEntries().entries.find((candidate) => candidate.key === point.key)
    if (!entry) return
    activeNodeKey.value = point.key
    cursorOffset.value = clampTranscriptOffset(entry.text, point.offset)
    if (!options?.preserveColumn) preferredColumn.value = transcriptLinePosition(entry.text, cursorOffset.value).column
    entry.element.focus({ preventScroll: true })
    entry.element.scrollIntoView({ behavior: 'auto', block: options?.center ? 'center' : 'nearest', inline: 'nearest' })
    syncNativeSelection()
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
    const text = cleanNodeCopyText(selected)
    cursorOffset.value = preferTail && text ? transcriptLineRange(text, text.length).start : 0
    preferredColumn.value = 0
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
    options?: { center?: boolean; recordJump?: boolean; offset?: number; preserveColumn?: boolean },
  ) {
    if (!element) return
    const key = keyForElement(element)
    if (!key) return
    if (options?.recordJump) pushJumpMark()
    const changed = activeNodeKey.value !== key
    activeNodeKey.value = key
    const text = cleanNodeCopyText(element)
    if (typeof options?.offset === 'number') cursorOffset.value = clampTranscriptOffset(text, options.offset)
    else if (changed) cursorOffset.value = 0
    if (!options?.preserveColumn) preferredColumn.value = transcriptLinePosition(text, cursorOffset.value).column
    element.focus({ preventScroll: true })
    element.scrollIntoView({ behavior: 'auto', block: options?.center ? 'center' : 'nearest', inline: 'nearest' })
    if (mode.value.startsWith('VISUAL') && !visualAnchorKey.value) visualAnchorKey.value = key
    syncNativeSelection()
  }

  function selectNode(key: string) {
    if (!key) return
    const changed = activeNodeKey.value !== key
    activeNodeKey.value = key
    if (changed) {
      cursorOffset.value = 0
      preferredColumn.value = 0
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
    const model = textEntries()
    let entryIndex = model.entries.findIndex((entry) => entry.key === activeTextEntry()?.key)
    if (entryIndex < 0) return
    let entry = model.entries[entryIndex]
    if (!entry) return
    let position = transcriptLinePosition(entry.text, cursorOffset.value)
    const targetColumn = preferredColumn.value

    for (let step = 0; step < Math.max(1, count); step += 1) {
      const lines = entry.text.split('\n')
      const nextLine = position.line + (direction === 'down' ? 1 : -1)
      if (nextLine >= 0 && nextLine < lines.length) {
        position = { line: nextLine, column: targetColumn }
        continue
      }
      const nextEntryIndex = entryIndex + (direction === 'down' ? 1 : -1)
      const nextEntry = model.entries[nextEntryIndex]
      if (!nextEntry) break
      entryIndex = nextEntryIndex
      entry = nextEntry
      position = {
        line: direction === 'down' ? 0 : Math.max(0, entry.text.split('\n').length - 1),
        column: targetColumn,
      }
    }

    setCursorPoint(
      {
        key: entry.key,
        offset: transcriptOffsetAtLineColumn(entry.text, position.line, targetColumn),
      },
      { preserveColumn: true },
    )
    preferredColumn.value = targetColumn
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
    const entry = activeTextEntry()
    if (!entry) return
    const range = transcriptLineRange(entry.text, cursorOffset.value)
    let offset = range.start
    if (edge === 'end') {
      const graphemes = transcriptGraphemes(entry.text.slice(range.start, range.end))
      offset = range.start + (graphemes.at(-1)?.start ?? 0)
    } else if (edge === 'first-non-blank') {
      const match = entry.text.slice(range.start, range.end).search(/\S/u)
      offset = range.start + Math.max(0, match)
    }
    setCursorPoint({ key: entry.key, offset })
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

  function nearestNodeAtViewportRatio(ratio: number): HTMLElement | null {
    const scroll = opts.scrollEl.value
    const nodes = cursorElements()
    if (!scroll || !nodes.length) return null
    const bounds = scroll.getBoundingClientRect()
    const target = bounds.top + bounds.height * ratio
    return nodes.reduce<HTMLElement | null>((best, element) => {
      const center = element.getBoundingClientRect().top + element.getBoundingClientRect().height / 2
      if (!best) return element
      const bestRect = best.getBoundingClientRect()
      const bestCenter = bestRect.top + bestRect.height / 2
      return Math.abs(center - target) < Math.abs(bestCenter - target) ? element : best
    }, null)
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
      selectElement(nearestNodeAtViewportRatio(direction === 'down' ? 0.75 : 0.25))
    })
  }

  function scrollLine(direction: 'up' | 'down') {
    const count = takeCount()
    opts.scrollEl.value?.scrollBy({ top: (direction === 'down' ? 1 : -1) * count * 24, behavior: 'auto' })
  }

  function moveView(row: 'top' | 'middle' | 'bottom', place = false) {
    const ratio = row === 'top' ? 0.08 : row === 'middle' ? 0.5 : 0.92
    const node = nearestNodeAtViewportRatio(ratio)
    if (!node) return
    selectElement(node)
    if (place) node.scrollIntoView({ block: row === 'middle' ? 'center' : row === 'top' ? 'start' : 'end' })
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

  function cleanNodeCopyText(element: HTMLElement): string {
    if (element.dataset.transcriptNode === 'part') return String(element.dataset.copyText || '').trim()
    const role = String(element.dataset.role || '').trim()
    const parts = Array.from(element.querySelectorAll<HTMLElement>('[data-transcript-node="part"][data-copy-text]'))
      .map((part) => String(part.dataset.copyText || '').trim())
      .filter(Boolean)
    return [role, parts.join('\n\n')].filter(Boolean).join('\n')
  }

  let cursorOverlay: HTMLDivElement | null = null
  let ownsNativeSelection = false

  function removeCursorOverlay() {
    cursorOverlay?.remove()
    cursorOverlay = null
  }

  function selectableTextNodes(element: HTMLElement): Text[] {
    const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT)
    const nodes: Text[] = []
    let current = walker.nextNode()
    while (current) {
      const textNode = current as Text
      const parent = textNode.parentElement
      if (
        textNode.data &&
        parent &&
        !parent.closest('[data-transcript-chrome="true"], [aria-hidden="true"], input, textarea, select, option')
      ) {
        nodes.push(textNode)
      }
      current = walker.nextNode()
    }
    return nodes
  }

  function domBoundary(point: CursorPoint): { node: Text; offset: number } | null {
    const element = elementForKey(point.key)
    if (!element) return null
    const nodes = selectableTextNodes(element)
    if (!nodes.length) return null
    let remaining = Math.max(0, point.offset)
    for (const node of nodes) {
      if (remaining <= node.data.length) return { node, offset: remaining }
      remaining -= node.data.length
    }
    const last = nodes.at(-1)!
    return { node: last, offset: last.data.length }
  }

  function rangeBetweenPoints(anchor: CursorPoint, head: CursorPoint, includeHead: boolean): Range | null {
    const model = textEntries()
    const anchorGlobal = globalOffsetForPoint(anchor, model)
    const headGlobal = globalOffsetForPoint(head, model)
    if (anchorGlobal === null || headGlobal === null) return null
    const startPoint = anchorGlobal <= headGlobal ? anchor : head
    const endPoint = anchorGlobal <= headGlobal ? head : anchor
    const start = domBoundary(startPoint)
    const end = domBoundary(endPoint)
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

  function syncNativeSelection() {
    if (typeof document === 'undefined' || mode.value === 'INSERT' || mode.value === 'SEARCH') {
      removeCursorOverlay()
      return
    }
    const head = cursorPoint()
    if (!head) {
      removeCursorOverlay()
      return
    }

    if (mode.value.startsWith('VISUAL') && visualAnchorKey.value) {
      removeCursorOverlay()
      const range = rangeBetweenPoints({ key: visualAnchorKey.value, offset: visualAnchorOffset.value }, head, true)
      const selection = window.getSelection()
      if (range && selection) {
        selection.removeAllRanges()
        selection.addRange(range)
        ownsNativeSelection = true
      }
      return
    }

    if (ownsNativeSelection) {
      window.getSelection()?.removeAllRanges()
      ownsNativeSelection = false
    }
    const range = rangeBetweenPoints(head, head, true)
    const rect = range?.getClientRects()[0] || range?.getBoundingClientRect()
    if (!rect || (!rect.width && !rect.height)) {
      removeCursorOverlay()
      return
    }
    if (!cursorOverlay) {
      cursorOverlay = document.createElement('div')
      cursorOverlay.dataset.agenaVimCursor = 'true'
      cursorOverlay.style.cssText =
        'position:fixed;pointer-events:none;z-index:69;background:oklch(var(--primary) / 0.3);border-bottom:2px solid oklch(var(--primary));'
      document.body.append(cursorOverlay)
    }
    cursorOverlay.style.left = `${rect.left}px`
    cursorOverlay.style.top = `${rect.top}px`
    cursorOverlay.style.width = `${Math.max(2, rect.width)}px`
    cursorOverlay.style.height = `${Math.max(2, rect.height)}px`
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

  function inclusiveGraphemeEnd(text: string, offset: number): number {
    const grapheme = transcriptGraphemes(text).find((item) => offset >= item.start && offset < item.end)
    return grapheme?.end ?? Math.max(0, Math.min(text.length, offset))
  }

  function visualSelectionText(): string {
    const model = textEntries()
    const anchor = globalOffsetForPoint({ key: visualAnchorKey.value, offset: visualAnchorOffset.value }, model)
    const head = globalOffsetForPoint({ key: activeNodeKey.value, offset: cursorOffset.value }, model)
    if (anchor === null || head === null) return ''

    if (mode.value === 'VISUAL LINE') {
      const first = transcriptLineRange(model.text, Math.min(anchor, head))
      const last = transcriptLineRange(model.text, Math.max(anchor, head))
      return model.text.slice(first.start, last.end)
    }

    if (mode.value === 'VISUAL BLOCK') {
      const anchorPosition = transcriptLinePosition(model.text, anchor)
      const headPosition = transcriptLinePosition(model.text, head)
      const startLine = Math.min(anchorPosition.line, headPosition.line)
      const endLine = Math.max(anchorPosition.line, headPosition.line)
      const startColumn = Math.min(anchorPosition.column, headPosition.column)
      const endColumn = Math.max(anchorPosition.column, headPosition.column)
      return model.text
        .split('\n')
        .slice(startLine, endLine + 1)
        .map((line) =>
          transcriptGraphemes(line)
            .slice(startColumn, endColumn + 1)
            .map((item) => item.text)
            .join(''),
        )
        .join('\n')
    }

    const start = Math.min(anchor, head)
    const end = inclusiveGraphemeEnd(model.text, Math.max(anchor, head))
    return model.text.slice(start, end)
  }

  async function copyRawText(value: string) {
    const copy = value.trimEnd()
    if (!copy) return
    const ok = await copyTextToClipboard(copy)
    opts.toasts.push(
      ok ? 'success' : 'error',
      ok ? 'Copied transcript selection' : 'Failed to copy transcript selection',
    )
  }

  function isNodeActive(key: string): boolean {
    return activeNodeKey.value === key
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
      const text = message ? cleanNodeCopyText(message) : entry.text
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
      const query = searchQuery.value.trim()
      if (!query || !searchMatches.value.length) return
      const activeMatch = searchMatchIndex.value >= 0 ? searchMatches.value[searchMatchIndex.value] : null
      const activeKey = activeMatch?.key || ''
      for (const element of cursorElements()) {
        const key = keyForElement(element)
        const nodes = selectableTextNodes(element)
        if (!nodes.length) continue
        let combined = ''
        const mapping: { node: Text; start: number; end: number }[] = []
        for (const node of nodes) {
          mapping.push({ node, start: combined.length, end: combined.length + node.data.length })
          combined += node.data
        }
        const ranges = transcriptSearchRanges(combined, query)
        if (!ranges.length) continue
        const keyMatches = searchMatches.value.filter((match) => match.key === key)
        const activeOrdinal = activeKey === key ? keyMatches.indexOf(activeMatch as TranscriptSearchMatch) : -1
        for (let rangeIndex = ranges.length - 1; rangeIndex >= 0; rangeIndex -= 1) {
          const range = ranges[rangeIndex]
          const startMapping = mapping.find((item) => range.start >= item.start && range.start <= item.end)
          const endMapping = mapping.find((item) => range.end >= item.start && range.end <= item.end)
          if (!startMapping || !endMapping) continue
          wrapSearchRange(
            startMapping.node,
            range.start - startMapping.start,
            endMapping.node,
            range.end - endMapping.start,
            rangeIndex === activeOrdinal,
          )
        }
      }
    } finally {
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
          : inclusiveGraphemeEnd(model.text, Math.max(originGlobal, destinationGlobal))
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
    if (action.type === 'cancel') {
      if (mode.value.startsWith('VISUAL')) cancelVisual()
      else clearPending()
      return true
    }
    if (action.type === 'copy') {
      void yankSelectionOrStartOperator()
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
    if (action.type === 'interrupt') {
      if (opts.draft.value.trim()) opts.clearComposer()
      else if (opts.canAbort.value) void opts.abortRun()
      return true
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

  watch(
    () => opts.selectedSessionId.value,
    () => {
      activeNodeKey.value = ''
      visualAnchorKey.value = ''
      cursorOffset.value = 0
      preferredColumn.value = 0
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
        syncNativeSelection()
      })
    },
  )

  watch(
    () => opts.renderBlocks.value.map((block) => block.key).join('|'),
    () =>
      nextTick(() => {
        ensureActive(true)
        syncNativeSelection()
        scheduleSearchHighlight()
      }),
  )

  onMounted(() => {
    const root = opts.pageRef.value
    if (root && typeof MutationObserver !== 'undefined') {
      searchHighlightObserver = new MutationObserver(() => {
        if (applyingSearchHighlight) return
        if (!searchOpen.value && !searchQuery.value.trim()) return
        scheduleSearchHighlight()
      })
      searchHighlightObserver.observe(root, { childList: true, subtree: true, characterData: true })
    }
    window.addEventListener('keydown', onKeydown)
    window.addEventListener('resize', scheduleCursorPlacement)
    opts.scrollEl.value?.addEventListener('scroll', scheduleCursorPlacement, { passive: true })
  })
  onBeforeUnmount(() => {
    searchHighlightObserver?.disconnect()
    searchHighlightObserver = null
    window.removeEventListener('keydown', onKeydown)
    window.removeEventListener('resize', scheduleCursorPlacement)
    opts.scrollEl.value?.removeEventListener('scroll', scheduleCursorPlacement)
    if (placementFrame) window.cancelAnimationFrame(placementFrame)
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
    isNodeActive,
    isNodeSelected,
    isNodeSearchMatch,
    setSearchQuery,
    handleSearchKeydown,
    closeSearch,
    enterInsertMode,
    returnToNavigate,
  }
}
