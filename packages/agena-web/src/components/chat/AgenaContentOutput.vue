<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { Terminal } from '@xterm/xterm'
import { useI18n } from 'vue-i18n'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'

import {
  contentChunkBytes,
  readContent,
  terminalSnapshotText,
  type ContentBuffer,
  type ContentChunk,
  type ContentFormat,
  type ContentRef,
} from '@/lib/content'
import AgenaContentText from './AgenaContentText.vue'
import OperationBlock from './AgenaOperationBlock.vue'
import { observeContent } from '@/lib/contentSubscriptions'
import { contentDocumentText, type ContentDocument } from '@/lib/contentDocument'
import { readContentForCopy, terminalBufferPlainText } from '@/lib/contentCopy'

const semanticDocument = shallowRef<ContentDocument | null>(null)

const props = defineProps<{ resource: ContentRef; sessionId: string; format?: ContentFormat }>()
const { t } = useI18n()
const surface = ref<HTMLDivElement>()
const outputRoot = ref<HTMLDivElement>()
const status = ref('active')
const error = ref('')
const paused = ref(false)
const selectionActive = ref(false)
const unseen = ref(0)
const hasOutput = ref(false)
const hasStderr = ref(false)
const gap = ref(false)
const windowed = ref(false)
const terminalHeight = ref<number>()
let buffer: ContentBuffer | undefined
const copying = ref(false)
let terminal: Terminal | undefined
let resize: ResizeObserver | undefined
let theme: MutationObserver | undefined
let stop: (() => void) | undefined
let stopped = false
let writing = false
let needsReplay = false
let pending: ContentChunk[] = []
let pendingBytes = 0

function applyTheme() {
  if (!surface.value || !terminal) return
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
    const selected = terminal?.getSelection()
    if (selected) {
      await navigator.clipboard.writeText(selected)
      return
    }
    if (!buffer?.cursor) return
    const targetCursor = { ...buffer.cursor }
    const documentText = buffer.document ? contentDocumentText(buffer.document) : ''
    const text: string[] = []
    if (props.resource.kind === 'terminal' && terminal) {
      text.push(terminalBufferPlainText(terminal.buffer.active))
      if (buffer.gap) text.push(`\n[${t('content.gap')}]\n`)
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
        cols: terminal?.cols || 240,
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
  if (stopped || writing) return
  if (!terminal) {
    pending = []
    pendingBytes = 0
    return
  }
  if (paused.value || terminal.getSelection()) {
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
    return
  }
  const pieces: string[] = []
  for (const { payload } of pending) {
    if (payload.type === 'text' || payload.type === 'log') pieces.push(payload.text)
    else if (payload.type === 'terminal') {
      terminal.resize(payload.screen.cols, payload.screen.rows)
      const screen = terminal.element?.querySelector('.xterm-screen')
      if (screen) terminalHeight.value = screen.getBoundingClientRect().height + 16
      pieces.push(terminalSnapshotText(payload.screen))
    } else if (payload.type === 'terminal_patch')
      pieces.push(terminalSnapshotText(payload.screen, payload.rows_changed))
  }
  pending = []
  pendingBytes = 0
  terminal.write(pieces.join(''), () => {
    writing = false
    if (stopped) return
    if (!paused.value && !terminal?.getSelection()) terminal?.scrollToBottom()
    flushOutput()
  })
}

function connect() {
  stop?.()
  stop = observeContent(props.sessionId, props.resource, (frame) => {
    buffer = frame.buffer
    if (!paused.value && !selectionActive.value) semanticDocument.value = buffer.document
    status.value = buffer.resource?.state || 'active'
    gap.value = buffer.gap
    windowed.value = buffer.windowed
    error.value = frame.error
    needsReplay ||= frame.reset
    for (const { payload } of frame.appended) {
      if (payload.type === 'log') hasStderr.value ||= payload.stream === 'stderr'
      hasOutput.value ||= !('text' in payload) || payload.text.length > 0
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

onMounted(async () => {
  await nextTick()
  document.addEventListener('selectionchange', updateSelection)
  if (!surface.value) {
    connect()
    return
  }
  const fit = new FitAddon()
  terminal = new Terminal({
    disableStdin: true,
    convertEol: props.resource.kind !== 'terminal',
    scrollback: 5000,
    fontFamily: '"IBM Plex Mono", ui-monospace, monospace',
    fontSize: 12,
    lineHeight: 1.35,
    cursorBlink: false,
    cursorStyle: 'bar',
    allowProposedApi: false,
  })
  terminal.loadAddon(fit)
  terminal.open(surface.value)
  applyTheme()
  fit.fit()
  terminal.onScroll((line) => {
    paused.value = line < (terminal?.buffer.active.baseY || 0)
  })
  terminal.onSelectionChange(updateSelection)
  resize = new ResizeObserver(() => {
    if (props.resource.kind !== 'terminal') fit.fit()
  })
  resize.observe(surface.value)
  theme = new MutationObserver(applyTheme)
  theme.observe(document.documentElement, { attributes: true, attributeFilter: ['class', 'style'] })
  connect()
})

onBeforeUnmount(() => {
  stopped = true
  document.removeEventListener('selectionchange', updateSelection)
  stop?.()
  resize?.disconnect()
  theme?.disconnect()
  terminal?.dispose()
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
    connect()
  },
)
</script>

<template>
  <div
    ref="outputRoot"
    class="my-2 overflow-hidden rounded-md border border-border/60 bg-background"
    data-content-output
  >
    <div
      class="flex items-center justify-between border-b border-border/40 px-3 py-1.5 text-[11px] text-muted-foreground"
    >
      <div class="flex items-center gap-2">
        <span
          class="h-1.5 w-1.5 rounded-full"
          :class="status === 'active' ? 'animate-pulse bg-primary' : 'bg-muted-foreground/50'"
        />
        <span>{{
          t(status === 'active' ? 'content.live' : status === 'complete' ? 'content.complete' : 'content.interrupted')
        }}</span>
        <span v-if="hasStderr" class="rounded bg-amber-500/10 px-1.5 text-amber-600">stderr</span>
      </div>
      <div class="flex items-center gap-3">
        <button v-if="paused || selectionActive" class="text-primary hover:underline" type="button" @click="follow">
          {{ t('content.follow') }}<span v-if="unseen" class="ml-1">+{{ unseen }}</span> ↓
        </button>
        <button type="button" class="hover:text-foreground disabled:opacity-50" :disabled="copying" @click="copy">
          {{ t('content.copy') }}
        </button>
      </div>
    </div>
    <div v-if="gap" class="border-b border-border/40 px-3 py-1 text-[11px] text-amber-600">{{ t('content.gap') }}</div>
    <div v-if="windowed" class="border-b border-border/40 px-3 py-1 text-[11px] text-muted-foreground">
      {{ t('content.window') }}
    </div>
    <div v-if="semanticDocument" class="space-y-3 p-3" data-content-document>
      <OperationBlock v-for="block in semanticDocument.blocks" :key="block.id" :block="block" :session-id="sessionId" />
    </div>
    <AgenaContentText
      v-if="resource.kind === 'text'"
      :resource="resource"
      :session-id="sessionId"
      :format="format || 'plain'"
      class="p-3"
    />
    <div
      v-else-if="resource.kind !== 'structured'"
      ref="surface"
      class="content-terminal bg-background p-2 text-foreground"
      :class="resource.kind === 'terminal' ? 'max-h-[32rem] overflow-auto' : 'h-64'"
      :style="terminalHeight ? { height: `${terminalHeight}px` } : undefined"
      :aria-label="t('content.label')"
    />
    <div
      v-if="!hasOutput && status === 'active'"
      class="border-t border-border/40 px-3 py-1.5 text-xs text-muted-foreground"
    >
      {{ t('content.waiting') }}
    </div>
    <div v-if="error" class="border-t border-border/40 px-3 py-1.5 text-xs text-amber-600">{{ error }}</div>
  </div>
</template>

<style scoped>
.content-terminal :deep(.xterm) {
  height: 100%;
}
.content-terminal :deep(.xterm-viewport) {
  scrollbar-width: thin;
}
</style>
