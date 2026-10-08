<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useResizeObserver } from '@vueuse/core'
import { RiArrowDownLine, RiEditLine } from '@remixicon/vue'
import { useI18n } from 'vue-i18n'
import AttachmentPicker from '@/components/chat/AttachmentPicker.vue'
import type { AttachedFile, PendingAttachment } from '@/pages/chat/useChatAttachments'
import type { ComposerInput, ComposerSegment } from '@/pages/chat/composerInput'
import { composerDomSelection } from '@/pages/chat/composerDomSelection'

const props = defineProps<{
  draft: string
  fullscreen: boolean
  modeLabel?: string
  attachedFiles: AttachedFile[]
  pendingAttachments: PendingAttachment[]
}>()
const emit = defineEmits<{
  (e: 'update:draft', value: string): void
  (e: 'toggleFullscreen'): void
  (e: 'drop', ev: DragEvent): void
  (e: 'paste', ev: ClipboardEvent): void
  (e: 'draftInput'): void
  (e: 'draftKeydown', ev: KeyboardEvent): void
  (e: 'filesSelected', files: FileList): void
  (e: 'removeAttachment', id: string): void
}>()
const shellEl = ref<HTMLDivElement | null>(null)
const statusEl = ref<HTMLDivElement | null>(null)
const statusHeight = ref(28)
const editorEl = ref<HTMLDivElement | null>(null)
const attachmentPickerRef = ref<InstanceType<typeof AttachmentPicker> | null>(null)
const { t } = useI18n()
// The status row straddles the shell border. Reserve its lower half outside
// the scrolling editor so it cannot paint over text, including after scroll.
useResizeObserver(statusEl, ([entry]) => {
  if (entry) statusHeight.value = entry.borderBoxSize?.[0]?.blockSize ?? entry.contentRect.height
})
const ATTACHMENT_CHARACTER = '\ufffc'
let savedStart = 0
let savedEnd = 0
let reportedText: string | undefined
let selectionCache: { startNode: Node; endNode: Node; start: number; end: number } | undefined

function readSegments(): ComposerSegment[] {
  const segments: ComposerSegment[] = []
  const pushText = (text: string) => {
    if (!text) return
    const last = segments[segments.length - 1]
    if (last?.type === 'text') last.text += text
    else segments.push({ type: 'text', text })
  }
  const walk = (node: Node) => {
    if (node.nodeType === Node.TEXT_NODE) {
      pushText(node.textContent ?? '')
      return
    }
    if (!(node instanceof HTMLElement)) return
    const id = node.dataset.attachmentId
    if (id) {
      segments.push({ type: 'attachment', id })
      return
    }
    if (node.tagName === 'BR') {
      pushText('\n')
      return
    }
    // IMEs may add spans; browsers can also wrap inserted lines in divs.
    if ((node.tagName === 'DIV' || node.tagName === 'P') && segments.length) pushText('\n')
    for (const child of node.childNodes) walk(child)
  }
  for (const child of editorEl.value?.childNodes ?? []) walk(child)
  return segments
}
// One authoritative text space: text nodes, <br> and attachment chips.
// Browsers and IMEs may wrap inserted lines in a DIV/P block, which the
// segment reader counts as an extra newline while the selection math
// treats the block as transparent. Unwrapping the block on every input
// keeps both views identical instead of letting the offsets drift.
function normalizeBlocks() {
  const root = editorEl.value
  if (!root) return
  for (const block of [...root.children]) {
    if (block.tagName !== 'DIV' && block.tagName !== 'P') continue
    const fragment = document.createDocumentFragment()
    if (block.previousSibling) fragment.append(document.createElement('br'))
    while (block.firstChild) fragment.append(block.firstChild)
    root.replaceChild(fragment, block)
  }
}

function editorValue() {
  return readSegments()
    .map((part) => (part.type === 'text' ? part.text : ATTACHMENT_CHARACTER))
    .join('')
}
function plainText() {
  return readSegments()
    .filter((part): part is { type: 'text'; text: string } => part.type === 'text')
    .map((part) => part.text)
    .join('')
}
function selectionOffsets(): [number, number] {
  const root = editorEl.value
  const selection = window.getSelection()
  if (
    !root ||
    !selection ||
    !selection.rangeCount ||
    !root.contains(selection.anchorNode) ||
    !root.contains(selection.focusNode)
  ) {
    return [savedStart, savedEnd]
  }
  const range = selection.getRangeAt(0)
  if (
    selectionCache?.startNode === range.startContainer &&
    selectionCache.endNode === range.endContainer &&
    selectionCache.start === range.startOffset &&
    selectionCache.end === range.endOffset
  ) {
    return [savedStart, savedEnd]
  }
  ;[savedStart, savedEnd] = composerDomSelection(root, range)
  selectionCache = {
    startNode: range.startContainer,
    endNode: range.endContainer,
    start: range.startOffset,
    end: range.endOffset,
  }
  return [savedStart, savedEnd]
}
function boundaryAt(position: number): { node: Node; offset: number } {
  const root = editorEl.value!
  let remaining = Math.max(0, position)
  const visit = (parent: Node): { node: Node; offset: number } | null => {
    for (let index = 0; index < parent.childNodes.length; index++) {
      const child = parent.childNodes[index]!
      if (child.nodeType === Node.TEXT_NODE) {
        const length = child.textContent?.length ?? 0
        if (remaining <= length) return { node: child, offset: remaining }
        remaining -= length
      } else if (child instanceof HTMLElement && (child.dataset.attachmentId || child.tagName === 'BR')) {
        if (remaining === 0) return { node: parent, offset: index }
        remaining -= 1
        if (remaining === 0) return { node: parent, offset: index + 1 }
      } else {
        const found = visit(child)
        if (found) return found
      }
    }
    return null
  }
  return visit(root) ?? { node: root, offset: root.childNodes.length }
}
function setSelectionRange(start: number, end: number) {
  selectionCache = undefined
  const root = editorEl.value
  const length = editorValue().length
  savedStart = Math.max(0, Math.min(start, length))
  savedEnd = Math.max(savedStart, Math.min(end, length))
  if (!root || document.activeElement !== root) return
  const from = boundaryAt(savedStart)
  const to = boundaryAt(savedEnd)
  const range = document.createRange()
  range.setStart(from.node, from.offset)
  range.setEnd(to.node, to.offset)
  const selection = window.getSelection()
  selection?.removeAllRanges()
  selection?.addRange(range)
}
function notifyInput(notifyRemoved = true) {
  selectionCache = undefined
  const segments = readSegments()
  const known = new Set([...props.attachedFiles, ...props.pendingAttachments].map((file) => file.id))
  const present = new Set(
    segments
      .filter((part): part is { type: 'attachment'; id: string } => part.type === 'attachment')
      .map((part) => part.id),
  )
  reportedText = segments
    .filter((part) => part.type === 'text')
    .map((part) => part.text)
    .join('')
  emit('update:draft', reportedText)
  const reported = reportedText
  void nextTick(() => {
    if (reportedText === reported) reportedText = undefined
  })
  if (notifyRemoved) for (const id of known) if (!present.has(id)) emit('removeAttachment', id)
  emit('draftInput')
}
function replaceRange(
  start: number,
  end: number,
  replacement: string,
  selectionMode: 'select' | 'start' | 'end' | 'preserve' = 'end',
) {
  const root = editorEl.value
  if (!root) return
  const from = boundaryAt(start)
  const to = boundaryAt(end)
  const range = document.createRange()
  range.setStart(from.node, from.offset)
  range.setEnd(to.node, to.offset)
  range.deleteContents()
  if (replacement) range.insertNode(document.createTextNode(replacement))
  const cursor = selectionMode === 'start' ? start : start + replacement.length
  setSelectionRange(cursor, selectionMode === 'select' ? start + replacement.length : cursor)
  notifyInput()
}
const textareaEl: ComposerInput = {
  get element() {
    return editorEl.value
  },
  get value() {
    return editorValue()
  },
  get selectionStart() {
    return selectionOffsets()[0]
  },
  get selectionEnd() {
    return selectionOffsets()[1]
  },
  focus() {
    editorEl.value?.focus()
    setSelectionRange(savedStart, savedEnd)
  },
  blur() {
    editorEl.value?.blur()
  },
  contains(node) {
    return !!node && !!editorEl.value?.contains(node)
  },
  setSelectionRange,
  setRangeText: (replacement, start, end, selectionMode) => replaceRange(start, end, replacement, selectionMode),
}
function updateChip(chip: HTMLSpanElement) {
  const id = chip.dataset.attachmentId
  const file =
    props.attachedFiles.find((item) => item.id === id) ?? props.pendingAttachments.find((item) => item.id === id)
  const label = file?.pastePreview ?? file?.filename ?? String(t('chat.attachments.title'))
  chip.textContent = label
  chip.title = label
  chip.setAttribute('aria-label', `${t('chat.attachments.title')}: ${label}`)
  chip.dataset.pending =
    props.pendingAttachments.some((item) => item.id === id) && !props.attachedFiles.some((item) => item.id === id)
      ? 'true'
      : 'false'
}
function createChip(id: string): HTMLSpanElement {
  const chip = document.createElement('span')
  chip.dataset.attachmentId = id
  chip.contentEditable = 'false'
  chip.className = 'composer-attachment'
  chip.setAttribute('role', 'img')
  updateChip(chip)
  return chip
}
function attachmentOffset(id: string): number {
  let offset = 0
  for (const part of readSegments()) {
    if (part.type === 'attachment' && part.id === id) return offset
    offset += part.type === 'text' ? part.text.length : 1
  }
  return -1
}
function removeChipIds(ids: Set<string>) {
  selectionOffsets()
  const positions = [...ids]
    .map(attachmentOffset)
    .filter((position) => position >= 0)
    .sort((a, b) => b - a)
  for (const chip of editorEl.value?.querySelectorAll<HTMLSpanElement>('[data-attachment-id]') ?? []) {
    if (ids.has(chip.dataset.attachmentId ?? '')) chip.remove()
  }
  for (const position of positions) {
    if (savedStart > position) savedStart -= 1
    if (savedEnd > position) savedEnd -= 1
  }
  setSelectionRange(savedStart, savedEnd)
}
function insertChip(id: string) {
  if (!editorEl.value) return
  // Insert at a collapsed caret. A selection (possibly a stale one kept
  // while the editor was unfocused) must never be deleted by staging an
  // attachment: that silently removed the text the user had selected.
  const [, end] = selectionOffsets()
  const at = boundaryAt(end)
  const range = document.createRange()
  range.setStart(at.node, at.offset)
  range.setEnd(at.node, at.offset)
  range.insertNode(createChip(id))
  setSelectionRange(end + 1, end + 1)
  emit('update:draft', plainText())
}
function syncAttachments() {
  selectionCache = undefined
  const root = editorEl.value
  if (!root) return
  const known = new Set([...props.attachedFiles, ...props.pendingAttachments].map((file) => file.id))
  const removed = new Set<string>()
  for (const chip of root.querySelectorAll<HTMLSpanElement>('[data-attachment-id]')) {
    if (!known.has(chip.dataset.attachmentId ?? '')) removed.add(chip.dataset.attachmentId ?? '')
    else updateChip(chip)
  }
  if (removed.size) removeChipIds(removed)
  const present = new Set(
    [...root.querySelectorAll<HTMLSpanElement>('[data-attachment-id]')].map((chip) => chip.dataset.attachmentId),
  )
  for (const file of [...props.attachedFiles, ...props.pendingAttachments]) {
    if (present.has(file.id)) continue
    insertChip(file.id)
    present.add(file.id)
  }
}
function replaceAttachmentWithText(id: string, text: string) {
  const chip = [...(editorEl.value?.querySelectorAll<HTMLSpanElement>('[data-attachment-id]') ?? [])].find(
    (item) => item.dataset.attachmentId === id,
  )
  if (!chip) return
  selectionOffsets()
  const position = attachmentOffset(id)
  chip.replaceWith(document.createTextNode(text))
  if (position >= 0) {
    if (savedStart > position) savedStart += text.length - 1
    if (savedEnd > position) savedEnd += text.length - 1
    setSelectionRange(savedStart, savedEnd)
  }
  notifyInput(false)
}
function removeAttachmentNodes(ids: string[]) {
  removeChipIds(new Set(ids))
}
function restoreSegments(segments: ComposerSegment[]) {
  const root = editorEl.value
  if (!root) return
  root.replaceChildren()
  for (const segment of segments)
    root.append(segment.type === 'text' ? document.createTextNode(segment.text) : createChip(segment.id))
  setSelectionRange(editorValue().length, editorValue().length)
  emit('update:draft', plainText())
}
function insertText(text: string) {
  const [start, end] = selectionOffsets()
  replaceRange(start, end, text)
}
function handleBeforeInput(event: InputEvent) {
  if (event.isComposing) return
  if (event.inputType === 'insertParagraph' || event.inputType === 'insertLineBreak') {
    event.preventDefault()
    insertText('\n')
    return
  }
  if (!event.inputType.startsWith('delete')) return
  const [start, end] = selectionOffsets()
  const value = editorValue()
  if (start !== end && value.slice(start, end).includes(ATTACHMENT_CHARACTER)) {
    event.preventDefault()
    replaceRange(start, end, '')
  } else if (
    start === end &&
    event.inputType === 'deleteContentBackward' &&
    value[start - 1] === ATTACHMENT_CHARACTER
  ) {
    event.preventDefault()
    replaceRange(start - 1, start, '')
  } else if (start === end && event.inputType === 'deleteContentForward' && value[start] === ATTACHMENT_CHARACTER) {
    event.preventDefault()
    replaceRange(start, start + 1, '')
  }
}
function handleInput() {
  selectionCache = undefined
  normalizeBlocks()
  if (editorEl.value?.childNodes.length === 1 && editorEl.value.firstChild instanceof HTMLBRElement)
    editorEl.value.replaceChildren()
  selectionOffsets()
  notifyInput()
}
function rememberSelection() {
  if (document.activeElement === editorEl.value) selectionOffsets()
}
watch(
  () => props.draft,
  (text) => {
    if (!editorEl.value || reportedText === text || plainText() === text) return
    selectionCache = undefined
    reportedText = undefined
    editorEl.value.replaceChildren(...(text ? [document.createTextNode(text)] : []))
    savedStart = savedEnd = text.length
    syncAttachments()
  },
  { flush: 'sync' },
)
watch(() => [props.attachedFiles, props.pendingAttachments], syncAttachments, { flush: 'sync' })
onMounted(() => {
  editorEl.value?.replaceChildren(...(props.draft ? [document.createTextNode(props.draft)] : []))
  savedStart = savedEnd = props.draft.length
  syncAttachments()
  document.addEventListener('selectionchange', rememberSelection)
})
onBeforeUnmount(() => document.removeEventListener('selectionchange', rememberSelection))
function openFilePicker() {
  attachmentPickerRef.value?.openFilePicker()
}
defineExpose({
  shellEl,
  textareaEl,
  openFilePicker,
  getSegments: readSegments,
  replaceAttachmentWithText,
  removeAttachmentNodes,
  restoreSegments,
  insertText,
})
</script>

<template>
  <div
    ref="shellEl"
    class="composer-shell relative flex flex-col overflow-visible rounded-xl border border-input bg-background/85 shadow-sm"
    :class="fullscreen ? 'composer-fullscreen rounded-none' : ''"
    :style="{ paddingTop: $slots.status || $slots.topRight ? `${Math.ceil(statusHeight / 2)}px` : undefined }"
    data-oc-keyboard-tap="keep"
    @dragover.prevent
    @drop.prevent="$emit('drop', $event)"
  >
    <div
      v-if="$slots.status || $slots.topRight"
      ref="statusEl"
      class="pointer-events-none absolute inset-x-2 top-0 z-10 flex min-w-0 -translate-y-1/2 items-center gap-2 pr-8 font-mono text-[11px]"
    >
      <div
        v-if="$slots.status"
        class="pointer-events-auto flex min-w-0 max-w-full items-center overflow-x-auto whitespace-nowrap bg-background px-1 [scrollbar-width:none]"
      >
        <slot name="status" />
      </div>
      <div v-if="$slots.topRight" class="min-w-0 shrink-0 bg-background px-1">
        <slot name="topRight" />
      </div>
    </div>
    <div
      v-if="$slots.bottomLeft || $slots.bottomRight"
      class="pointer-events-none absolute inset-x-2 bottom-0 z-10 flex min-w-0 translate-y-1/2 items-center gap-2 font-mono text-[11px]"
    >
      <div v-if="$slots.bottomLeft" class="flex min-w-0 items-center gap-2 bg-background px-1">
        <slot name="bottomLeft" />
      </div>
      <div v-if="$slots.bottomRight" class="flex min-w-0 items-center gap-2 bg-background px-1">
        <slot name="bottomRight" />
      </div>
    </div>
    <slot name="overlay" />
    <div class="absolute top-1 right-1 z-10 flex items-center gap-1">
      <button
        type="button"
        :data-oc-keyboard-tap="fullscreen ? 'blur' : 'keep'"
        class="flex h-6 w-6 items-center justify-center rounded-md text-muted-foreground/80 hover:bg-secondary/60 hover:text-foreground"
        :title="fullscreen ? t('chat.composer.editor.collapse') : t('chat.composer.editor.open')"
        :aria-label="fullscreen ? t('chat.composer.editor.collapse') : t('chat.composer.editor.open')"
        @pointerdown.prevent
        @click="$emit('toggleFullscreen')"
      >
        <component :is="fullscreen ? RiArrowDownLine : RiEditLine" class="h-4 w-4" />
      </button>
    </div>
    <div
      ref="editorEl"
      contenteditable="true"
      role="textbox"
      aria-multiline="true"
      :aria-label="t('chat.composer.input.placeholder')"
      :data-placeholder="t('chat.composer.input.placeholder')"
      data-chat-input="true"
      class="composer-editor w-full min-h-8 flex-1 overflow-y-auto border-0 bg-transparent px-3 pb-1.5 pt-1 pr-9 text-sm shadow-none focus-visible:outline-none"
      :class="fullscreen ? 'composer-textarea-full' : 'max-h-none'"
      spellcheck="false"
      @beforeinput="handleBeforeInput"
      @input="handleInput"
      @compositionend="handleInput"
      @click="$emit('draftInput')"
      @keyup="$emit('draftInput')"
      @paste="$emit('paste', $event)"
      @keydown="$emit('draftKeydown', $event)"
    />
    <AttachmentPicker ref="attachmentPickerRef" @filesSelected="$emit('filesSelected', $event)" />
    <slot name="controls" />
  </div>
</template>

<style scoped>
.composer-shell.composer-fullscreen {
  background-color: oklch(var(--background));
  height: 100%;
  flex: 1;
  min-height: 0;
}
.composer-textarea-full {
  flex: 1;
  min-height: 0;
  max-height: none;
}
.composer-editor {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  caret-color: currentColor;
}
@media (pointer: coarse) {
  .composer-editor:not(.composer-textarea-full) {
    min-height: 44px;
  }
}
.composer-editor:empty::before {
  content: attr(data-placeholder);
  color: oklch(var(--muted-foreground));
  pointer-events: none;
}
.composer-editor :deep(.composer-attachment) {
  display: inline-block;
  vertical-align: baseline;
  max-width: min(24rem, 75%);
  margin: 0 0.15rem;
  padding: 0.08rem 0.45rem;
  border: 1px solid oklch(var(--border));
  border-radius: 0.35rem;
  background: oklch(var(--secondary));
  color: oklch(var(--foreground));
  font-size: 0.85em;
  line-height: 1.45;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  user-select: all;
}
.composer-editor :deep(.composer-attachment)::before {
  content: '▣';
  margin-right: 0.35rem;
  color: oklch(var(--muted-foreground));
}
.composer-editor :deep(.composer-attachment[data-pending='true']) {
  border-style: dashed;
  opacity: 0.7;
}
</style>
