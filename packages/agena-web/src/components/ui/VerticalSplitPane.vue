<script setup lang="ts">
import { ref, onBeforeUnmount } from 'vue'

const props = defineProps<{
  modelValue: number // Height of the bottom pane in pixels
  minHeight?: number
  maxHeight?: number
  disabled?: boolean
  // When true, force the top pane to stay collapsed (0px) so bottom pane is pinned to the top.
  // Useful for fullscreen-like UIs where transient viewport metric changes (mobile keyboard/toolbars)
  // would otherwise briefly reveal the top pane and make the bottom pane "jump" downward.
  collapseTop?: boolean
}>()

const emit = defineEmits<{
  (e: 'update:modelValue', value: number): void
  (e: 'dragStart'): void
  (e: 'dragEnd'): void
  (e: 'dblclick'): void
}>()

const isDragging = ref(false)
const startY = ref(0)
const startHeight = ref(0)
const KEYBOARD_RESIZE_STEP_PX = 16

function clampHeight(raw: number): number {
  let next = raw
  if (props.minHeight !== undefined) next = Math.max(props.minHeight, next)
  if (props.maxHeight !== undefined) next = Math.min(props.maxHeight, next)
  return next
}

function handlePointerDown(e: PointerEvent) {
  if (props.disabled) return

  // Only left click
  if (e.button !== 0) return

  const target = e.currentTarget as HTMLElement
  target.setPointerCapture(e.pointerId)

  isDragging.value = true
  startY.value = e.clientY
  startHeight.value = props.modelValue

  emit('dragStart')

  window.addEventListener('pointermove', handlePointerMove)
  window.addEventListener('pointerup', handlePointerUp)
  window.addEventListener('pointercancel', handlePointerUp)

  document.body.style.cursor = 'row-resize'
  document.body.style.userSelect = 'none'
}

function handlePointerMove(e: PointerEvent) {
  if (!isDragging.value) return
  e.preventDefault()

  const deltaY = startY.value - e.clientY // Dragging up increases height
  emit('update:modelValue', clampHeight(startHeight.value + deltaY))
}

function handleKeydown(event: KeyboardEvent) {
  if (props.disabled) return

  let next: number | null = null
  if (event.key === 'ArrowUp') next = props.modelValue + KEYBOARD_RESIZE_STEP_PX
  if (event.key === 'ArrowDown') next = props.modelValue - KEYBOARD_RESIZE_STEP_PX
  if (event.key === 'Home' && props.minHeight !== undefined) next = props.minHeight
  if (event.key === 'End' && props.maxHeight !== undefined) next = props.maxHeight
  if (next === null) return

  event.preventDefault()
  emit('update:modelValue', clampHeight(next))
}

function handlePointerUp(e: PointerEvent) {
  if (!isDragging.value) return

  isDragging.value = false
  emit('dragEnd')

  window.removeEventListener('pointermove', handlePointerMove)
  window.removeEventListener('pointerup', handlePointerUp)
  window.removeEventListener('pointercancel', handlePointerUp)

  document.body.style.cursor = ''
  document.body.style.userSelect = ''

  // Release capture if possible
  try {
    const target = e.target as HTMLElement
    if (target && target.hasPointerCapture(e.pointerId)) {
      target.releasePointerCapture(e.pointerId)
    }
  } catch (err) {
    // ignore
  }
}

onBeforeUnmount(() => {
  if (isDragging.value) {
    window.removeEventListener('pointermove', handlePointerMove)
    window.removeEventListener('pointerup', handlePointerUp)
    window.removeEventListener('pointercancel', handlePointerUp)
    document.body.style.cursor = ''
    document.body.style.userSelect = ''
  }
})
</script>

<template>
  <div class="flex flex-col h-full min-h-0 overflow-hidden">
    <!-- Top Pane (Flexible) -->
    <div
      class="min-h-0 overflow-hidden relative flex flex-col"
      :class="collapseTop ? 'h-0 flex-none pointer-events-none' : 'flex-1'"
      :aria-hidden="collapseTop ? 'true' : undefined"
    >
      <slot name="top" />
    </div>

    <!-- Drag Handle -->
    <div
      v-if="!collapseTop"
      role="separator"
      aria-orientation="horizontal"
      :aria-valuenow="Math.round(modelValue)"
      :aria-valuemin="minHeight !== undefined ? Math.round(minHeight) : undefined"
      :aria-valuemax="maxHeight !== undefined ? Math.round(maxHeight) : undefined"
      :aria-disabled="disabled ? 'true' : undefined"
      :tabindex="disabled ? -1 : 0"
      class="relative z-20 flex items-center justify-center shrink-0 h-3 -my-1.5 cursor-row-resize select-none touch-none group outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
      :class="{ 'pointer-events-none opacity-50': disabled }"
      @pointerdown="handlePointerDown"
      @keydown="handleKeydown"
      @dblclick="$emit('dblclick')"
    >
      <!-- Hit area and visible line -->
      <div class="w-full h-px bg-border/40 group-hover:bg-primary/50 transition-colors" />
    </div>

    <!-- Bottom Pane (Fixed/Resizable Height) -->
    <div
      class="shrink-0 relative flex flex-col min-h-0"
      :class="{ 'transition-[height] duration-200 ease-out': !isDragging }"
      :style="{ height: `${modelValue}px` }"
    >
      <slot name="bottom" />
    </div>
  </div>
</template>
