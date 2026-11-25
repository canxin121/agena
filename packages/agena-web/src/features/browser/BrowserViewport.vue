<script setup lang="ts">
import { nextTick, ref, useId, watch } from 'vue'
import Button from '@/components/ui/Button.vue'
import Input from '@/components/ui/Input.vue'
import { handleOptionKeydown, isPlainNavigationKey } from '@/lib/keyboardNavigation'
import type { BrowserFrame, BrowserInput } from './api'
import { useBrowserCopy } from './copy'

const props = defineProps<{ frame: BrowserFrame | null; interactive: boolean }>()
const emit = defineEmits<{ input: [event: BrowserInput] }>()
const t = useBrowserCopy()
const viewport = ref<HTMLImageElement>()
const keyboard = ref<HTMLTextAreaElement>()
const dialogPanel = ref<HTMLElement>()
const dialogId = useId()
const dialogValue = ref('')
let composing = false
let pointer: number | undefined
let previousClick: { x: number; y: number; time: number; count: number } | undefined
let clickCount = 1
const pressedKeys = new Map<string, Extract<BrowserInput, { kind: 'key' }>>()

function modifiers(event: MouseEvent | KeyboardEvent) {
  return (
    Number(event.altKey) | (Number(event.ctrlKey) << 1) | (Number(event.metaKey) << 2) | (Number(event.shiftKey) << 3)
  )
}

function point(event: MouseEvent) {
  const rect = viewport.value?.getBoundingClientRect()
  if (!rect?.width || !rect.height) return
  const clamp = (value: number) => Math.min(1, Math.max(0, value))
  return {
    x: clamp((event.clientX - rect.left) / rect.width),
    y: clamp((event.clientY - rect.top) / rect.height),
    modifiers: modifiers(event),
  }
}

function focusKeyboard() {
  if (props.interactive && !props.frame?.dialog) keyboard.value?.focus({ preventScroll: true })
}

function sendPointer(event: PointerEvent, action: 'down' | 'up' | 'move') {
  if (!props.interactive || props.frame?.dialog || !event.isPrimary) return
  const position = point(event)
  if (!position) return
  if (action === 'down') {
    event.preventDefault()
    viewport.value?.setPointerCapture(event.pointerId)
    pointer = event.pointerId
    focusKeyboard()
    const time = performance.now()
    const distance = previousClick
      ? Math.hypot(event.clientX - previousClick.x, event.clientY - previousClick.y)
      : Infinity
    clickCount =
      previousClick && time - previousClick.time < 380 && distance < 5 ? Math.min(previousClick.count + 1, 3) : 1
    previousClick = { x: event.clientX, y: event.clientY, time, count: clickCount }
  }
  if (action === 'up') {
    if (pointer !== event.pointerId) return
    pointer = undefined
  }
  const button = event.button === 0 ? 'left' : event.button === 1 ? 'middle' : event.button === 2 ? 'right' : 'none'
  emit('input', {
    kind: 'pointer',
    action,
    ...position,
    button,
    buttons: event.buttons,
    clicks: action === 'move' ? 0 : clickCount,
  })
}

function wheel(event: WheelEvent) {
  if (!props.interactive || props.frame?.dialog) return
  const position = point(event)
  if (!position) return
  event.preventDefault()
  const factor = event.deltaMode === 1 ? 18 : event.deltaMode === 2 ? props.frame?.height || 600 : 1
  const delta = (raw: number) => Math.max(-4000, Math.min(4000, raw * factor))
  emit('input', { kind: 'wheel', ...position, delta_x: delta(event.deltaX), delta_y: delta(event.deltaY) })
}

function keydown(event: KeyboardEvent) {
  if (
    !props.interactive ||
    props.frame?.dialog ||
    composing ||
    event.isComposing ||
    event.key === 'Process' ||
    event.key === 'Unidentified'
  )
    return
  event.stopPropagation()
  const paste = (event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'v'
  if (paste || (Array.from(event.key).length === 1 && !(event.altKey || event.ctrlKey || event.metaKey))) return
  event.preventDefault()
  if (event.key === 'Escape') {
    keyboard.value?.blur()
    return
  }
  const input: Extract<BrowserInput, { kind: 'key' }> = {
    kind: 'key',
    down: true,
    key: event.key,
    code: event.code,
    key_code: Math.min(event.keyCode, 255),
    modifiers: modifiers(event),
    repeat: event.repeat,
  }
  pressedKeys.set(event.code, input)
  emit('input', input)
}

function keyup(event: KeyboardEvent) {
  event.stopPropagation()
  const down = pressedKeys.get(event.code)
  if (down) {
    pressedKeys.delete(event.code)
    if (props.interactive) emit('input', { ...down, down: false, repeat: false, modifiers: modifiers(event) })
  }
}

function releaseKeyboard() {
  if (props.interactive) {
    for (const key of pressedKeys.values()) emit('input', { ...key, down: false, repeat: false, modifiers: 0 })
  }
  pressedKeys.clear()
  composing = false
  if (keyboard.value) keyboard.value.value = ''
}

function text(value: string) {
  if (!props.interactive || props.frame?.dialog || !value) return
  // Bound each message without splitting a Unicode code point.
  const characters = Array.from(value).slice(0, 16384)
  for (let i = 0; i < characters.length; i += 4096) {
    emit('input', { kind: 'text', text: characters.slice(i, i + 4096).join('') })
  }
}

function input(event: Event) {
  const change = event as InputEvent
  const field = event.target as HTMLTextAreaElement
  if (composing || change.isComposing) return
  text(change.data || field.value)
  if (!change.data && !field.value && change.inputType === 'deleteContentBackward') {
    const key = { kind: 'key' as const, key: 'Backspace', code: 'Backspace', key_code: 8, modifiers: 0, repeat: false }
    emit('input', { ...key, down: true })
    emit('input', { ...key, down: false })
  }
  field.value = ''
}

function compositionEnd(event: CompositionEvent) {
  composing = false
  text(event.data)
  if (keyboard.value) keyboard.value.value = ''
}

function paste(event: ClipboardEvent) {
  event.preventDefault()
  text(event.clipboardData?.getData('text/plain') || '')
}

function resolveDialog(accept: boolean) {
  emit('input', {
    kind: 'dialog',
    accept,
    text: props.frame?.dialog?.kind === 'prompt' ? dialogValue.value : undefined,
  })
  dialogValue.value = ''
}

function onDialogKeydown(event: KeyboardEvent) {
  if (!props.interactive || !isPlainNavigationKey(event) || event.shiftKey) return
  const textInput = event.target instanceof HTMLInputElement ? event.target : null
  if (handleOptionKeydown(event, dialogPanel.value ?? null, textInput)) return
  const cancel = event.key === 'Escape'
  if (!cancel && (event.key !== 'Enter' || !textInput)) return
  event.preventDefault()
  event.stopPropagation()
  if (!event.repeat) resolveDialog(!cancel)
}

watch(
  () => props.interactive && Boolean(props.frame?.dialog),
  (open) => {
    const active = document.activeElement
    if (open && active === keyboard.value) {
      void nextTick(() => {
        if (document.activeElement === active) {
          dialogPanel.value?.querySelector<HTMLElement>('[data-browser-dialog-initial]')?.focus({ preventScroll: true })
        }
      })
    } else if (!open && active && dialogPanel.value?.contains(active)) {
      void nextTick(() => {
        if (document.activeElement === document.body) focusKeyboard()
      })
    }
  },
)

watch(
  () => props.interactive,
  (interactive) => {
    if (!interactive) {
      keyboard.value?.blur()
      releaseKeyboard()
      dialogValue.value = ''
    }
  },
)
</script>

<template>
  <div class="space-y-2">
    <div class="relative grid min-h-36 place-items-center overflow-hidden rounded-md border border-border bg-muted/50">
      <img
        v-if="frame?.jpeg"
        ref="viewport"
        :src="'data:image/jpeg;base64,' + frame.jpeg"
        :alt="t('preview')"
        :draggable="false"
        :class="interactive ? 'cursor-crosshair touch-none' : ''"
        class="block max-h-[55dvh] max-w-full select-none"
        data-testid="browser-viewport"
        @pointerdown="sendPointer($event, 'down')"
        @pointerup="sendPointer($event, 'up')"
        @pointermove="sendPointer($event, 'move')"
        @pointercancel="sendPointer($event, 'up')"
        @wheel="wheel"
        @contextmenu.prevent
      />
      <p v-else-if="!frame?.dialog" role="status" class="px-4 py-8 text-sm text-muted-foreground">{{ t('loading') }}</p>
      <div v-if="frame?.dialog" class="absolute inset-0 grid place-items-center bg-background/95 p-3">
        <div
          ref="dialogPanel"
          role="dialog"
          :aria-labelledby="dialogId"
          :aria-describedby="`${dialogId}-message`"
          :data-keyboard-overlay="interactive ? 'true' : undefined"
          class="oc-overlay-panel max-h-full w-full max-w-sm space-y-2 overflow-y-auto overscroll-contain p-3"
          @keydown="onDialogKeydown"
        >
          <p :id="dialogId" class="text-sm font-semibold">{{ t('dialog') }}</p>
          <p :id="`${dialogId}-message`" class="whitespace-pre-wrap break-words text-xs text-muted-foreground">
            {{ frame.dialog.message }}
          </p>
          <Input
            v-if="interactive && frame.dialog.kind === 'prompt'"
            v-model="dialogValue"
            data-browser-dialog-initial
            :aria-label="t('dialog')"
            autocomplete="off"
            class="h-8 text-xs"
          />
          <div v-if="interactive" class="flex flex-wrap justify-end gap-2">
            <Button
              data-keyboard-option
              :data-browser-dialog-initial="frame.dialog.kind !== 'prompt' ? '' : undefined"
              size="sm"
              variant="outline"
              @click="resolveDialog(false)"
              >{{ t('cancel') }}</Button
            >
            <Button data-keyboard-option size="sm" @click="resolveDialog(true)">{{ t('confirm') }}</Button>
          </div>
        </div>
      </div>
      <textarea
        v-if="interactive"
        ref="keyboard"
        :aria-label="t('keyboard')"
        autocomplete="off"
        autocapitalize="off"
        :spellcheck="false"
        class="absolute left-0 top-0 h-px w-px resize-none opacity-0"
        tabindex="-1"
        @keydown="keydown"
        @keyup="keyup"
        @blur="releaseKeyboard"
        @input="input"
        @compositionstart="composing = true"
        @compositionend="compositionEnd"
        @paste="paste"
      />
    </div>
    <div v-if="interactive && !frame?.dialog" class="flex flex-wrap items-center gap-2">
      <Button size="xs" variant="outline" @click="focusKeyboard">{{ t('keyboard') }}</Button>
      <p class="text-xs text-muted-foreground">{{ t('keyboardHint') }}</p>
    </div>
  </div>
</template>
