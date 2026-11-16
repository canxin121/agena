<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { Terminal } from '@xterm/xterm'
import { useI18n } from 'vue-i18n'
import { RiArrowDownLine, RiClipboardLine, RiLoader4Line } from '@remixicon/vue'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'

import {
  contentChunkBytes,
  readContent,
  terminalSnapshotText,
  type ContentBuffer,
  type ContentChunk,
  type ContentCursor,
  type ContentFormat,
  type ContentRef,
} from '@/lib/content'
import AgenaContentText from './AgenaContentText.vue'
import AgenaContentStatic from './AgenaContentStatic.vue'
import PartLoadingIndicator from './PartLoadingIndicator.vue'
import OperationBlock from './AgenaOperationBlock.vue'
import { observeContent } from '@/lib/contentSubscriptions'
import { contentDocumentText, type ContentDocument } from '@/lib/contentDocument'
import { readContentForCopy, terminalBufferPlainText } from '@/lib/contentCopy'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'

const semanticDocument = shallowRef<ContentDocument | null>(null)

const props = defineProps<{ resource: ContentRef; sessionId: string; format?: ContentFormat }>()
const { t } = useI18n()
const pane = useWorkspacePaneContext()
const isVisible = () => !pane || pane.isVisible.value
const surface = ref<HTMLDivElement>()
const terminalHost = ref<HTMLDivElement>()
const outputRoot = ref<HTMLDivElement>()
const status = ref('active')
const error = ref('')
const loading = ref(true)
const paused = ref(false)
const selectionActive = ref(false)
const unseen = ref(0)
const hasOutput = ref(false)
const hasStderr = ref(false)
const gap = ref(false)
const windowed = ref(false)
const terminalHeight = ref<number>()
const staticOutput = shallowRef<{ text: string; cursor: ContentCursor; followTail: boolean } | null>(null)
let buffer: ContentBuffer | undefined
const copying = ref(false)
let terminal: Terminal | undefined
let fit: FitAddon | undefined
let resize: ResizeObserver | undefined
let theme: MutationObserver | undefined
let stop: (() => void) | undefined
let stopped = false
let writing = false
let needsReplay = false
let pending: ContentChunk[] = []
let pendingBytes = 0
let fitting = false
let mounted = false
let resizeFrame = 0
let lastTerminalCols = 240

function receivedFinalContent() {
  const resource = buffer?.resource
  const cursor = buffer?.cursor
  return Boolean(
    resource &&
    cursor &&
    resource.state !== 'active' &&
    cursor.epoch === resource.cursor.epoch &&
    cursor.sequence >= resource.cursor.sequence,
  )
}

function canKeepStaticOutput() {
  const snapshot = staticOutput.value
  if (!snapshot) return false
  const resource = buffer?.resource
  // A fresh subscription may still be paging towards a result we already have.
  // Its final target, rather than its current read cursor, identifies that result.
  return (
    !resource ||
    (resource.state !== 'active' &&
      resource.cursor.epoch === snapshot.cursor.epoch &&
      resource.cursor.sequence === snapshot.cursor.sequence)
  )
}

function disposeTerminal() {
  resize?.disconnect()
  resize = undefined
  theme?.disconnect()
  theme = undefined
  if (resizeFrame) cancelAnimationFrame(resizeFrame)
  resizeFrame = 0
  const current = terminal
  terminal = undefined
  fit = undefined
  current?.dispose()
}

function settleOutput() {
  const cursor = buffer?.cursor
  if (!terminal || !cursor || !receivedFinalContent() || writing || fitting || pending.length || needsReplay) return
  if (paused.value || selectionActive.value || terminal.getSelection()) return
  const screen = terminal.buffer.active
  lastTerminalCols = terminal.cols
  staticOutput.value = {
    text: terminalBufferPlainText(screen, props.resource.kind === 'terminal' ? screen.baseY : 0),
    cursor: { ...cursor },
    followTail: Boolean(terminal.element),
  }
  loading.value = false
  unseen.value = 0
  document.removeEventListener('selectionchange', updateSelection)
  disposeTerminal()
}

function updateTerminalLayout() {
  // A replay resets the buffer before xterm asynchronously parses its text.
  // Keep its existing viewport until that write completes, rather than fitting
  // the transient empty buffer and disturbing the user's follow position.
  if (writing || !isVisible() || !terminal || !terminalHost.value?.clientWidth) return
  if (props.resource.kind === 'log' && (paused.value || selectionActive.value)) return
  // Xterm defers renderer resizes while offscreen, even though terminal.rows
  // changes immediately. A rendered row's own CSS height stays valid; dividing
  // the stale screen height by the new row count creates a sizing feedback loop.
  const row = terminal.element?.querySelector<HTMLElement>('.xterm-rows > div')
  const cellHeight = row ? Number.parseFloat(getComputedStyle(row).height) : 0
  if (!Number.isFinite(cellHeight) || cellHeight <= 0) return

  fitting = true
  try {
    if (props.resource.kind === 'log') {
      const dimensions = fit?.proposeDimensions()
      if (dimensions && Number.isFinite(dimensions.cols) && terminal.cols !== dimensions.cols)
        terminal.resize(dimensions.cols, terminal.rows)
      // Reflow after a width change can alter the occupied rows. Keep the
      // trailing cursor row, but never derive log rows from the host height.
      const rows = Math.min(12, Math.max(1, terminal.buffer.active.baseY + terminal.buffer.active.cursorY + 1))
      if (terminal.rows !== rows) terminal.resize(terminal.cols, rows)
    }
    terminalHeight.value = Math.min(384, Math.ceil(cellHeight * terminal.rows) + 8)
    // Even a no-op scrollToBottom requests another xterm render. Avoid an
    // onRender/layout loop once the viewport has already reached the tail.
    if (props.resource.kind === 'log' && terminal.buffer.active.viewportY !== terminal.buffer.active.baseY)
      terminal.scrollToBottom()
  } finally {
    fitting = false
  }
}

function scheduleTerminalLayout() {
  if (stopped || !isVisible() || resizeFrame) return
  resizeFrame = requestAnimationFrame(() => {
    resizeFrame = 0
    if (!stopped) updateTerminalLayout()
  })
}

function applyTheme() {
  if (!isVisible() || !surface.value || !terminal) return
  const style = getComputedStyle(surface.value)
  // CSS themes use OKLCH. Xterm accepts RGB/hex, so resolve through the
  // browser's color engine rather than letting unsupported colors become black.
  const context = document.createElement('canvas').getContext('2d', { willReadFrequently: true })
  if (!context) return
  const rgb = (color: string) => {
    context.clearRect(0, 0, 1, 1)
    context.fillStyle = color
    context.fillRect(0, 0, 1, 1)
    return context.getImageData(0, 0, 1, 1).data
  }
  const background = rgb(style.backgroundColor)
  const foreground = rgb(style.color)
  const hex = (channels: Uint8ClampedArray) =>
    `#${Array.from(channels)
      .slice(0, 3)
      .map((value) => value.toString(16).padStart(2, '0'))
      .join('')}`
  const dark = background[0]! * 0.2126 + background[1]! * 0.7152 + background[2]! * 0.0722 < 128
  const palette = dark
    ? [
        '#292524',
        '#f28b82',
        '#a8cc8c',
        '#e7c484',
        '#91b3df',
        '#cca8dd',
        '#8dcac7',
        '#d6d3d1',
        '#a8a29e',
        '#ffb4ab',
        '#c4e5a9',
        '#f2d69b',
        '#adc8ed',
        '#dfbef0',
        '#abe0dc',
        '#faf5ee',
      ]
    : [
        '#292524',
        '#b42318',
        '#276a3d',
        '#8c5d0b',
        '#255f99',
        '#8251a4',
        '#247a78',
        '#6b625a',
        '#786b5f',
        '#b42318',
        '#236936',
        '#805409',
        '#215e9f',
        '#7d4b9e',
        '#1d7472',
        '#49413b',
      ]
  const [
    black,
    red,
    green,
    yellow,
    blue,
    magenta,
    cyan,
    white,
    brightBlack,
    brightRed,
    brightGreen,
    brightYellow,
    brightBlue,
    brightMagenta,
    brightCyan,
    brightWhite,
  ] = palette
  terminal.options.theme = {
    background: hex(background),
    foreground: hex(foreground),
    cursor: hex(foreground),
    black,
    red,
    green,
    yellow,
    blue,
    magenta,
    cyan,
    white,
    brightBlack,
    brightRed,
    brightGreen,
    brightYellow,
    brightBlue,
    brightMagenta,
    brightCyan,
    brightWhite,
  }
}

function follow() {
  paused.value = false
  unseen.value = 0
  needsReplay = true
  semanticDocument.value = buffer?.document || null
  terminal?.clearSelection()
  const selection = document.getSelection()
  if (selection?.anchorNode && outputRoot.value?.contains(selection.anchorNode)) selection.removeAllRanges()
  selectionActive.value = false
  flushOutput()
  terminal?.scrollToBottom()
}

async function copy() {
  if (copying.value) return
  copying.value = true
  try {
    const selection = document.getSelection()
    const selected =
      terminal?.getSelection() ||
      (selection && !selection.isCollapsed && selection.anchorNode && outputRoot.value?.contains(selection.anchorNode)
        ? selection.toString()
        : '')
    if (selected) {
      await navigator.clipboard.writeText(selected)
      return
    }
    const cursor = staticOutput.value?.cursor || buffer?.cursor
    if (!cursor) return
    const targetCursor = { ...cursor }
    const resultDocument = staticOutput.value ? semanticDocument.value : buffer?.document || semanticDocument.value
    const documentText = resultDocument ? contentDocumentText(resultDocument) : ''
    const text: string[] = []
    if (props.resource.kind === 'terminal') {
      text.push(
        staticOutput.value?.text ??
          (terminal ? terminalBufferPlainText(terminal.buffer.active, terminal.buffer.active.baseY) : ''),
      )
      if (gap.value) text.push(`\n[${t('content.gap')}]\n`)
    } else if (props.resource.kind === 'text') {
      await readContentForCopy(
        targetCursor,
        (cursor) => readContent(props.sessionId, props.resource.resource_id, cursor),
        t('content.gap'),
        t('content.copyLimit'),
        async (body) => {
          text.push(body)
        },
      )
    } else {
      // An unopened emulator provides the same VT semantics as the widget,
      // without copying escape sequences or depending on its visible window.
      const capture = new Terminal({
        cols: terminal?.cols || lastTerminalCols,
        rows: 16,
        scrollback: 100000,
        convertEol: true,
        disableStdin: true,
      })
      try {
        await readContentForCopy(
          targetCursor,
          (cursor) => {
            if (stopped) throw new Error(t('content.recovering'))
            return readContent(props.sessionId, props.resource.resource_id, cursor)
          },
          t('content.gap'),
          t('content.copyLimit'),
          (body) =>
            new Promise<void>((resolve, reject) => {
              capture.write(body, () => {
                if (capture.buffer.active.baseY >= 100000) reject(new Error(t('content.copyLimit')))
                else resolve()
              })
            }),
        )
        text.push(terminalBufferPlainText(capture.buffer.active))
      } finally {
        capture.dispose()
      }
    }
    if (documentText) text.push(`${text.length ? '\n\n' : ''}${documentText}`)
    await navigator.clipboard.writeText(text.join(''))
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    copying.value = false
  }
}

function flushOutput() {
  if (stopped || writing || !isVisible()) return
  if (props.resource.kind !== 'log' && props.resource.kind !== 'terminal') {
    pending = []
    pendingBytes = 0
    return
  }
  if (staticOutput.value) {
    if (canKeepStaticOutput()) {
      pending = []
      pendingBytes = 0
      needsReplay = false
      return
    }
    staticOutput.value = null
  }
  ensureTerminal()
  if (!terminal) return
  if (paused.value || selectionActive.value || terminal.getSelection()) {
    needsReplay = true
    pending = []
    pendingBytes = 0
    return
  }
  // reset/resize/selection callbacks can synchronously request another flush.
  // Hold the write guard before any emulator mutation to prevent reentrant
  // replay from enqueuing the same retained output repeatedly.
  writing = true
  if (needsReplay) {
    needsReplay = false
    pending = buffer?.replayChunks() || []
    pendingBytes = 0
    terminal.reset()
  }
  if (!pending.length) {
    writing = false
    settleOutput()
    return
  }
  const pieces: string[] = []
  for (const chunk of pending) {
    const payload = chunk.payload.type === 'document_event' ? chunk.payload.input : chunk.payload
    if (payload.type === 'text' || payload.type === 'log') pieces.push(payload.text)
    else if (payload.type === 'terminal') {
      terminal.resize(payload.screen.cols, payload.screen.rows)
      pieces.push(terminalSnapshotText(payload.screen))
    } else if (payload.type === 'terminal_patch')
      pieces.push(terminalSnapshotText(payload.screen, payload.rows_changed))
  }
  pending = []
  pendingBytes = 0
  terminal.write(pieces.join(''), () => {
    writing = false
    if (stopped || !isVisible()) return
    // Size the committed buffer before starting the next queued write, even
    // when a busy stream leaves no animation frame between write callbacks.
    updateTerminalLayout()
    if (!paused.value && !terminal?.getSelection()) terminal?.scrollToBottom()
    flushOutput()
  })
}

function connect() {
  stop?.()
  stop = undefined
  if (!isVisible() || stopped) return
  stop = observeContent(props.sessionId, props.resource, (frame) => {
    buffer = frame.buffer
    // A completed widget can outlive the shared subscription's idle cache.
    // Keep its snapshot and metadata while a new subscription catches up.
    if (canKeepStaticOutput()) {
      if (buffer.resource) status.value = buffer.resource.state
      error.value = frame.error
      return
    }
    if (!paused.value && !selectionActive.value) semanticDocument.value = buffer.document
    status.value = buffer.resource?.state || (staticOutput.value ? status.value : 'active')
    gap.value = buffer.gap
    windowed.value = buffer.windowed
    error.value = frame.error
    loading.value =
      !staticOutput.value &&
      ((!buffer.resource && !buffer.chunks.length && !buffer.document && !frame.error) ||
        ((props.resource.kind === 'log' || props.resource.kind === 'terminal') &&
          Boolean(buffer.resource) &&
          status.value !== 'active' &&
          !terminal?.element))
    needsReplay ||= frame.reset
    for (const { payload } of frame.appended) {
      const input = payload.type === 'document_event' ? payload.input : payload
      if (input.type === 'log') hasStderr.value ||= input.stream === 'stderr'
      hasOutput.value ||= !('text' in input) || input.text.length > 0
    }
    if (paused.value || selectionActive.value) unseen.value += frame.appended.length
    pending.push(...frame.appended)
    pendingBytes += frame.appended.reduce((bytes, chunk) => bytes + contentChunkBytes(chunk), 0)
    if (pendingBytes > 512 * 1024) {
      pending = []
      pendingBytes = 0
      needsReplay = true
    }
    flushOutput()
  })
}

function updateSelection() {
  if (!isVisible()) return
  const selection = document.getSelection()
  selectionActive.value =
    Boolean(terminal?.getSelection()) ||
    Boolean(
      selection && !selection.isCollapsed && selection.anchorNode && outputRoot.value?.contains(selection.anchorNode),
    )
  if (!selectionActive.value && !paused.value) {
    semanticDocument.value = buffer?.document || null
    flushOutput()
  }
}

function ensureTerminal() {
  if (!buffer?.resource) return
  terminal ||= new Terminal({
    cols: buffer.resource.state === 'active' ? 80 : 240,
    rows: props.resource.kind === 'log' ? (buffer.resource.state === 'active' ? 1 : 16) : 24,
    disableStdin: true,
    convertEol: props.resource.kind !== 'terminal',
    scrollback: 5000,
    fontFamily: '"IBM Plex Mono", ui-monospace, monospace',
    fontSize: 12,
    lineHeight: 1.2,
    cursorBlink: false,
    cursorStyle: 'bar',
    allowProposedApi: false,
  })
  // Completed history still needs VT parsing, but never needs a live DOM
  // terminal. Drain its remaining pages before publishing a static result.
  if (terminal.element || buffer.resource.state !== 'active' || !terminalHost.value) return
  fit = new FitAddon()
  terminal.loadAddon(fit)
  terminal.open(terminalHost.value)
  applyTheme()
  updateTerminalLayout()
  terminal.onScroll((line) => {
    if (fitting) return
    const wasPaused = paused.value
    paused.value = line < (terminal?.buffer.active.baseY || 0)
    if (wasPaused && !paused.value) queueMicrotask(flushOutput)
  })
  terminal.onSelectionChange(updateSelection)
  document.addEventListener('selectionchange', updateSelection)
  terminal.onRender(scheduleTerminalLayout)
  resize = new ResizeObserver(scheduleTerminalLayout)
  resize.observe(terminalHost.value)
  theme = new MutationObserver(applyTheme)
  theme.observe(document.documentElement, { attributes: true, attributeFilter: ['class', 'style'] })
}

onMounted(async () => {
  await nextTick()
  if (stopped) return
  mounted = true
  document.addEventListener('selectionchange', updateSelection)
  connect()
})

if (pane)
  watch(
    pane.isVisible,
    (visible) => {
      if (!mounted) return
      if (!visible) {
        stop?.()
        stop = undefined
        pending = []
        pendingBytes = 0
        resize?.disconnect()
        if (resizeFrame) cancelAnimationFrame(resizeFrame)
        resizeFrame = 0
        return
      }
      if (terminalHost.value) resize?.observe(terminalHost.value)
      applyTheme()
      updateSelection()
      connect()
      scheduleTerminalLayout()
    },
    { flush: 'post' },
  )

onBeforeUnmount(() => {
  stopped = true
  document.removeEventListener('selectionchange', updateSelection)
  stop?.()
  disposeTerminal()
})

watch(
  () => `${props.sessionId}/${props.resource.resource_id}`,
  () => {
    // The parent keys this component by stable resource identity.
    stop?.()
    pending = []
    pendingBytes = 0
    needsReplay = true
    paused.value = selectionActive.value = false
    unseen.value = 0
    hasOutput.value = hasStderr.value = false
    semanticDocument.value = null
    staticOutput.value = null
    lastTerminalCols = 240
    connect()
  },
)
</script>

<template>
  <div ref="outputRoot" class="my-1 min-w-0" data-content-output :data-content-state="status" :aria-busy="loading">
    <div class="part-toolbar text-[11px] text-muted-foreground">
      <div class="flex min-w-0 flex-wrap items-center gap-2">
        <PartLoadingIndicator v-if="loading && resource.kind !== 'text'" />
        <template v-else>
          <span
            class="h-1.5 w-1.5 rounded-full"
            :class="status === 'active' ? 'animate-pulse bg-primary' : 'bg-muted-foreground/50'"
          />
          <span>{{
            t(status === 'active' ? 'content.live' : status === 'complete' ? 'content.complete' : 'content.interrupted')
          }}</span>
        </template>
        <span v-if="hasStderr" class="rounded bg-amber-500/10 px-1.5 text-amber-600">stderr</span>
      </div>
      <div class="part-actionbar">
        <button
          tabindex="0"
          v-if="!staticOutput && (paused || selectionActive)"
          class="part-action"
          type="button"
          @click="follow"
        >
          <RiArrowDownLine aria-hidden="true" />{{ t('content.follow')
          }}<span v-if="unseen" class="rounded bg-muted px-1 text-[10px]">+{{ unseen }}</span>
        </button>
        <button tabindex="0" type="button" class="part-action" :disabled="copying" :aria-busy="copying" @click="copy">
          <RiLoader4Line v-if="copying" class="animate-spin" aria-hidden="true" />
          <RiClipboardLine v-else aria-hidden="true" />
          {{ t('content.copy') }}
        </button>
      </div>
    </div>
    <div v-if="gap" class="border-b border-border/40 px-3 py-1 text-[11px] text-amber-600">{{ t('content.gap') }}</div>
    <div v-if="windowed" class="border-b border-border/40 px-3 py-1 text-[11px] text-muted-foreground">
      {{ t('content.window') }}
    </div>
    <div v-if="semanticDocument" class="space-y-1 p-2" data-content-document>
      <OperationBlock v-for="block in semanticDocument.blocks" :key="block.id" :block="block" :session-id="sessionId" />
    </div>
    <AgenaContentText
      v-if="resource.kind === 'text'"
      :resource="resource"
      :session-id="sessionId"
      :format="format || 'plain'"
      class="px-2 py-1"
    />
    <AgenaContentStatic
      v-if="staticOutput"
      :text="staticOutput.text"
      :format="format || 'plain'"
      :wrap="resource.kind !== 'terminal'"
      :follow-tail="staticOutput.followTail"
    />
    <div
      v-if="resource.kind === 'log' || resource.kind === 'terminal'"
      v-show="!staticOutput"
      ref="surface"
      class="content-terminal max-h-[24rem] bg-background p-1 text-foreground"
      :class="resource.kind === 'terminal' ? 'overflow-auto' : 'overflow-hidden'"
      :style="{ height: `${terminalHeight ?? 32}px` }"
      :aria-label="t('content.label')"
    >
      <div ref="terminalHost" class="h-full min-w-0" />
    </div>
    <div
      v-if="!loading && !hasOutput && status === 'active'"
      class="border-t border-border/40 px-2 py-0.5 text-xs text-muted-foreground"
    >
      {{ t('content.waiting') }}
    </div>
    <div v-if="error" class="border-t border-border/40 px-2 py-0.5 text-xs text-amber-600">{{ error }}</div>
  </div>
</template>

<style scoped>
.content-terminal :deep(.xterm) {
  height: 100%;
}
.content-terminal :deep(.xterm-screen) {
  /* Xterm's selection/helper layers can retain their old height after a
     snapshot shrinks. Keep them within the current screen's scroll extent. */
  overflow: hidden;
}
.content-terminal :deep(.xterm-viewport) {
  scrollbar-width: thin;
}
</style>
